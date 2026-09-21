//! `/push`, from the command word to the question the reader answers to the
//! project on the board.
//!
//! Everything up to that answer happens on the event loop's own thread between
//! two frames and opens no socket at all: which board a brief files to is a
//! manifest, a sigil file and a key store, all of them local. The answer is the
//! one thing that leaves the machine, and it leaves on a worker thread — see
//! [`Pushes`], which is [`crate::pacting::Pact`]'s shape for a request instead
//! of a pass: a channel per run, drained at the bottom of the loop, and the
//! `Option` holding it is itself the say-no to a second one.
//!
//! [`resolve_filing`] is asked once per press and the boundary rule is nowhere
//! in this file — a sigil compared against a `[[scope]]` record here would be a
//! second copy of `scope_opens_to`, and it is the copy that would forget the
//! wildcard. Every refusal but the ambiguous one is the engine's own sentence,
//! put on the thread verbatim, so the key refusals stay worded in `route.rs`
//! where `warlock check` meets them first.
//!
//! No `Target` outlives a call below. It borrows the manifest and carries the
//! Linear key's value, so what the window keeps is the scope name that found
//! it, and a confirmed question resolves it again.
//!
//! The key is read on exactly one line in this file — the one that builds the
//! client — and what crosses onto the worker is that client and five owned
//! strings. There is nowhere in [`Work`], in the channel's message, in a line or
//! in a `Debug` rendering for a key value to be, which is the same arrangement
//! `push.rs` makes for the subcommand.
//!
//! What the worker then does is the subcommand's own [`sent`]: the document, the
//! four requests in their order, the wording of every failure and the record
//! appended to `.warlock/filed.toml`. Nothing about a push is composed twice.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::Instant;
use std::{io, thread};

use warlock_engine::{Manifest, Target, filing, from_manifest_path, resolve_filing};
use warlock_tui::{
    App, Edited, Filing, LinearClient, Posts, PushConfirm, ScopeField, ScopePrompt, brief_at,
};

use crate::error::{Error, one_line};
use crate::push::{Board, records, sent};
use crate::standing::Standing;

pub(crate) const FILING_HEADING: &str = "Scope to file the brief to";

const NO_SCOPE: &str = "type the name of a scope to file to, or press Esc to file nothing";

// Said to a `/push` typed with one already in flight, and it is the whole of
// that refusal: nothing is resolved, nothing is read and no window comes up. A
// brief filed twice is two projects on the board and nothing on this side can
// take one back, which is why the one-at-a-time rule is worth a line of its own
// rather than a queue.
pub(crate) const ALREADY_FILING: &str =
    "a brief is already on its way to the board; this one was not sent";

// The worker sends on every path it takes, so a channel that closes with nothing
// on it is a panic. The hook has already printed it; what is left to say is that
// nobody here knows how far it got, which is the honest answer for a mutation
// that is not idempotent.
const PUSH_LOST: &str =
    "the push stopped without saying how it went; look at the board before filing it again";

// Both windows in one value, for `scoping::Windows`'s reason: a submit of the
// field is the very act that takes it down and puts the dialog up, and two
// returns would let a caller apply half of that. They are never both open.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Pushing {
    pub(crate) confirm: PushConfirm,
    pub(crate) field: ScopePrompt,
}

impl Pushing {
    pub(crate) const fn closed() -> Self {
        Self {
            confirm: PushConfirm::Closed,
            field: ScopePrompt::Closed,
        }
    }

    fn asking(field: ScopeField) -> Self {
        Self {
            confirm: PushConfirm::Closed,
            field: ScopePrompt::Open(field),
        }
    }

    fn confirming(confirm: PushConfirm) -> Self {
        Self {
            confirm,
            field: ScopePrompt::Closed,
        }
    }
}

/// Where the client comes from, as a parameter rather than a call: the seam
/// `push.rs::pushed` takes as `open: FnOnce(&str) -> P`, written as a trait so
/// that [`Pushes`] — and so [`crate::Session`] — gains one type parameter for it
/// rather than two.
///
/// The bounds are what the worker needs: a client is built on the event loop's
/// thread, from a key borrowed for as long as the target lives, and then owned by
/// a thread that outlives the press.
pub(crate) trait Opens {
    type Client: Posts + Send + 'static;

    fn open(&self, key: &str) -> Self::Client;
}

/// The one [`Opens`] that opens a socket, and the only value in warlock that
/// does. Holds nothing: a client is built per push, from a key read on one line
/// and dropped with the worker that used it.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Linear;

impl Opens for Linear {
    type Client = LinearClient;

    fn open(&self, key: &str) -> LinearClient {
        LinearClient::new(key)
    }
}

/// What a push has to say for itself, said exactly once: the address of the
/// project it created, or one line about why there is none.
///
/// A `String` rather than the error, because the error is worded on the worker
/// where it happens — out of `push.rs`'s own [`Error`], flattened — and a line is
/// what the thread takes.
type Landing = Result<String, String>;

/// The push a session is doing, if it is doing one, and where its client comes
/// from.
///
/// [`crate::pacting::Pact`]'s shape: the seam is held for the life of the
/// process, the run is an [`Option`] that is its own one-at-a-time guard, and
/// everything the run has to say arrives through [`Pushes::keep_up`] at the
/// bottom of the event loop.
///
/// The home is here for the reason the header's sigils are resolved once in
/// `session::load_app`: a home cannot move under a running warlock, and a second
/// reading per keystroke would be a second answer. It is also what keeps every
/// test in this crate off the developer's own — a `Pushes` is built with the home
/// it is to use, so nothing below this line asks the environment.
#[derive(Debug)]
pub(crate) struct Pushes<O: Opens> {
    open: O,
    home: Option<PathBuf>,
    sending: Option<Sending>,
}

// The channel and the board its answer is about. The team rides along because
// the worker sends back an address and nothing about where it went, and the line
// that says it landed names the board the line that started it named.
#[derive(Debug)]
struct Sending {
    events: Receiver<Landing>,
    team: String,
}

impl Pushes<Linear> {
    // The seam and the home, both settled here: building the seam costs nothing
    // — it is a unit value, and no socket exists until an answered dialog asks
    // for one — and a home that will not resolve is a `None` that every `/push`
    // is refused with in `Standing::home`'s own words.
    pub(crate) fn new() -> Self {
        Self::with_client(Linear, Standing::home().ok())
    }
}

impl<O: Opens> Pushes<O> {
    // The seam a test drives the real value over a stand-in client through,
    // rather than assembling the pieces underneath and proving something about an
    // arrangement the event loop never has.
    pub(crate) const fn with_client(open: O, home: Option<PathBuf>) -> Self {
        Self {
            open,
            home,
            sending: None,
        }
    }

    // Read once a round by the loop and once per `/push` by this value itself,
    // off the one run it keeps: a flag beside it would be a second record of
    // whether a brief is in the air.
    pub(crate) const fn sending(&self) -> bool {
        self.sending.is_some()
    }

    // `/push` typed into the composer. The two refusals are asked in the order
    // they have to be: a push already in flight is answered before anything is
    // read, because resolving a board for a request that cannot be sent would
    // read three files to no purpose.
    pub(crate) fn press(
        &self,
        app: &mut App,
        manifest: &Manifest,
        repo_root: &Path,
        written: &str,
        now: Instant,
    ) -> Pushing {
        if self.sending() {
            return saying(app, ALREADY_FILING, now);
        }
        let Some(home) = self.home_or_note(app, now) else {
            return Pushing::closed();
        };

        filing_to(app, manifest, repo_root, home, written, None, now)
    }

    // Somebody typing into the scope field. No in-flight question: the field is
    // only ever up between a `/push` and the dialog, and a push in flight is a
    // dialog that was answered rounds ago.
    //
    // Typing and abandoning move nothing but the field: nothing has been
    // resolved and nothing has been sent, so an Esc has nothing to put back.
    //
    // `written` is asked of the session again rather than carried on the field,
    // because there is one record of what `/write` wrote and this is not a
    // second one. It is `None` only in a state the keys cannot reach — the field
    // is up because a brief was written, and nothing can un-write one — and that
    // reads as nothing to file rather than as a refusal to word.
    #[expect(
        clippy::too_many_arguments,
        reason = "a submit of this field is a board resolution, so it takes what \
                  one takes: the repository, the document, the typing being \
                  judged and the keystroke judging it"
    )]
    pub(crate) fn edit(
        &self,
        app: &mut App,
        manifest: &Manifest,
        repo_root: &Path,
        written: Option<&str>,
        prompt: &ScopePrompt,
        edited: Edited,
        now: Instant,
    ) -> Pushing {
        match edited {
            Edited::Open(field) => Pushing::asking(field),
            Edited::Close => Pushing::closed(),
            // An empty field is refused with a line of its own and before the
            // home is so much as looked at: the engine would answer about a
            // scope named nothing, and what is true is that the reader has not
            // typed yet.
            Edited::Submit => match (prompt.field(), written) {
                (Some(field), _) if field.text().trim().is_empty() => {
                    Pushing::asking(field.clone().refused(NO_SCOPE))
                }
                (Some(field), Some(written)) => match self.home_or_note(app, now) {
                    Some(home) => {
                        filing_to(app, manifest, repo_root, home, written, Some(field), now)
                    }
                    None => Pushing::closed(),
                },
                _ => Pushing::closed(),
            },
        }
    }

    /// The confirmed question: the board resolved a second time, the client
    /// built, and the worker started.
    ///
    /// Everything that can refuse is asked here, on this thread, and says so on
    /// the thread — a refusal costs a line and leaves the session exactly where
    /// it was. The socket is opened on the line marked below and on no earlier
    /// one.
    pub(crate) fn send(
        &mut self,
        app: &mut App,
        manifest: &Manifest,
        repo_root: &Path,
        written: Option<&str>,
        filing: &Filing,
        now: Instant,
    ) {
        if self.sending() {
            saying(app, ALREADY_FILING, now);
            return;
        }
        // Both are `None` only in a state the keys cannot reach: this dialog is
        // up because a home resolved and because a brief was written, and nothing
        // can un-write one. That reads as nothing to file rather than as a
        // refusal to word.
        let (Some(home), Some(written)) = (self.home.as_deref(), written) else {
            return;
        };

        // The scope name the dialog was opened with, asked of the engine again:
        // the target that answered the first time borrows the manifest and
        // carries the key, so it could not be parked on the window. See the
        // module's note.
        let target = match resolve_filing(manifest, repo_root, home, Some(filing.scope())) {
            Ok(target) => target,
            Err(error) => {
                refused(app, &error, now);
                return;
            }
        };
        let record = target.record();
        let work = Work {
            root: repo_root.to_path_buf(),
            // The manifest-relative spelling `/write` handed back, resolved
            // against the root rather than the working directory, exactly as the
            // read that opened the dialog resolved it.
            path: from_manifest_path(repo_root, written),
            scope: target.scope().to_owned(),
            team: record.team().to_owned(),
            label: record.label().to_owned(),
        };

        // The key is read here and on no other line in this module, on the last
        // line before anything can leave the machine — `push.rs` reads it in the
        // same one place for the same reason.
        let client = self.open.open(target.value());
        // Before the worker starts, so the thread says which board is being
        // filed to from the instant it is: the answer may be seconds away and a
        // reader who has just said yes is looking at the conversation.
        app.panel_mut().note(filing_line(&work.team), now);
        self.sending = Some(Sending {
            team: work.team.clone(),
            events: spawn_push(client, work),
        });
    }

    /// What the push has said since the last round, which is one thing at most
    /// and ends the run either way.
    ///
    /// Drained rather than received, for [`crate::pacting`]'s reason: nothing
    /// here blocks, so frames keep being drawn, the tree keeps scrolling and the
    /// clocks keep ticking while the request is in flight.
    pub(crate) fn keep_up(&mut self, app: &mut App, now: Instant) {
        let Some(sending) = self.sending.as_ref() else {
            return;
        };

        let landing = match sending.events.try_recv() {
            Ok(landing) => landing,
            // Still in flight, and nothing new to say.
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => Err(PUSH_LOST.to_owned()),
        };
        let line = match &landing {
            Ok(url) => filed_line(&sending.team, url),
            // The worker's own sentence, which is `push.rs`'s wording of
            // whatever went wrong: a transport failure, Linear's refusal, a
            // document that is no longer a brief, a record that would not save.
            Err(line) => line.clone(),
        };
        // Taken before the line is put on the thread, so the round that reports
        // a push is a round on which the next `/push` is already allowed.
        self.sending = None;
        app.panel_mut().note(line, now);
    }

    fn home_or_note(&self, app: &mut App, now: Instant) -> Option<&Path> {
        let home = self.home.as_deref();
        if home.is_none() {
            no_home(app, now);
        }
        home
    }
}

// Everything the worker owns, and the whole of what crosses the thread
// boundary beside the client: five owned values, nothing borrowing the manifest,
// the app or the target the board was read out of.
#[derive(Debug, Clone)]
struct Work {
    root: PathBuf,
    path: PathBuf,
    scope: String,
    team: String,
    label: String,
}

// The `JoinHandle` is dropped on purpose, as a turn's and a pass's are: joining
// is waiting, and this thread exists precisely so nobody waits for it. There is
// no cancel handle either — one request per operation, no retry and no backoff
// (see `linear.rs`), and a mutation that may already have run is not something a
// keystroke can take back.
fn spawn_push<P: Posts + Send + 'static>(client: P, work: Work) -> Receiver<Landing> {
    let (events, received) = mpsc::channel();
    thread::spawn(move || {
        let landing = filed(&client, &work).map_err(|error| one_line(&error.to_string()));
        // Ignored for the reason every other worker's send is: a receiver that
        // has gone away is an application that is quitting.
        let _ = events.send(landing);
    });

    received
}

// The document, the four requests and the record, every one of them the
// subcommand's: [`sent`] sequences them, words their failures and decides where
// the label is resolved, and this is the same call `warlock push` makes.
//
// `io::sink` is where the line `sent` prints would have gone. The panel words its
// own — a conversation is not a terminal — and the address it needs comes back
// from the call rather than out of the bytes.
//
// The brief is read here rather than on the event loop's thread, so a document
// edited or deleted between the dialog and the answer is reported by the same
// worker as everything else: one line, from one place, however the push went.
fn filed(client: &impl Posts, work: &Work) -> Result<String, Error> {
    let brief = brief_at(&work.root, &work.path).map_err(|source| Error::Brief { source })?;
    let records = records(&work.root)?;

    sent(
        client,
        &work.root,
        Board {
            scope: &work.scope,
            team: &work.team,
            label: &work.label,
        },
        &brief,
        &work.path,
        records,
        &mut io::sink(),
    )
}

fn filing_line(team: &str) -> String {
    format!("filing to {team}")
}

// The address, which is the one thing about a push that must not be lost, with
// the board it went to: the same two facts the line above it said it was about
// to do.
fn filed_line(team: &str, url: &str) -> String {
    format!("filed to {team}: {url}")
}

// `asked` is the field this answer came out of, and `None` is the `/push` that
// asked nobody anything. It is both the name the engine is handed and the field
// a refusal reopens over, so the text a reader is looking at and the text that
// was judged cannot come apart.
fn filing_to(
    app: &mut App,
    manifest: &Manifest,
    repo_root: &Path,
    home: &Path,
    written: &str,
    asked: Option<&ScopeField>,
    now: Instant,
) -> Pushing {
    let name = asked.map(|field| field.text().trim());
    match resolve_filing(manifest, repo_root, home, name) {
        Ok(target) => confirming(app, repo_root, written, &target, now),
        // The one sentence of the engine's this does not repeat: its own names
        // `--scope`, which is a flag on the subcommand and nothing a panel has,
        // and here the field that is about to open is the instruction.
        Err(filing::Error::Several { candidates }) => {
            Pushing::asking(ScopeField::new(FILING_HEADING, "").refused(pick_one(&candidates)))
        }
        // Back to the field with the candidates under it and the typing where
        // it was, one character from being right. The other arm cannot happen —
        // there is no unknown name without a name — and answers it the way
        // every other refusal is answered rather than by inventing a window.
        Err(error @ filing::Error::Unknown { .. }) => match asked {
            Some(field) => Pushing::asking(field.clone().refused(one_line(&error.to_string()))),
            None => refused(app, &error, now),
        },
        Err(error) => refused(app, &error, now),
    }
}

// The name is read off the brief here rather than from the path, because it is
// the name the project would be filed under: the file the reader is looking at
// is the document, and a title guessed from its filename would be a second
// opinion about what it is called.
fn confirming(
    app: &mut App,
    repo_root: &Path,
    written: &str,
    target: &Target<'_>,
    now: Instant,
) -> Pushing {
    let path = from_manifest_path(repo_root, written);
    match brief_at(repo_root, &path) {
        Ok(brief) => Pushing::confirming(PushConfirm::open(
            brief.name(),
            target.scope(),
            target.record().team(),
            // The key by name. `Target::value` is not read on this path at all.
            target.key(),
        )),
        Err(error) => saying(app, one_line(&error.to_string()), now),
    }
}

fn refused(app: &mut App, error: &filing::Error, now: Instant) -> Pushing {
    saying(app, one_line(&error.to_string()), now)
}

// On the thread and not the footer, where `/push`'s other refusal already
// goes: a command typed into the conversation is answered in the conversation.
fn saying(app: &mut App, line: impl Into<String>, now: Instant) -> Pushing {
    app.panel_mut().note(line, now);
    Pushing::closed()
}

// A machine with no home is a line and no window, rather than an error out of
// the event loop: warlock is running, the brief is still on disk, and what is
// missing is the directory the sigils and the key store sit under.
//
// `Standing::home`'s own sentence, asked of the one thing that reading can say
// rather than worded again here — the session holds the answer to it (see
// [`Pushes`]) and not the failure.
fn no_home(app: &mut App, now: Instant) -> Pushing {
    saying(app, one_line(&Error::NoHome.to_string()), now)
}

fn pick_one(candidates: &[String]) -> String {
    let named: Vec<String> = candidates
        .iter()
        .map(|candidate| format!("`{candidate}`"))
        .collect();
    format!("this machine can file to {}: type one", named.join(", "))
}

// Every test drives a temporary repository and a temporary home, through the
// half of this module that takes both as parameters: nothing in the suite can
// read the sigils, the binding or the key store of the machine it runs on.
#[cfg(test)]
#[path = "tests/pushing.rs"]
mod tests;
