//! `/push`, from the command word to the question the reader answers to the
//! project on the board.
//!
//! Everything up to that answer happens on the event loop's own thread between
//! two frames and opens no socket at all: [`prepare`] is a manifest, a sigil
//! file, a key store and the brief, all of them local. The
//! answer is the one thing that leaves the machine, and it leaves on a worker
//! thread — see [`Pushes`]: a channel per run, drained at the bottom of the
//! loop, and the `Option` holding it is itself the say-no to a second one.
//!
//! What this file adds to [`mod@crate::push`] is the asking: the dialog over
//! what [`prepare`] resolved, and the lines on the thread. The boundary rule is
//! nowhere in this file, and every refusal is `push.rs`'s own sentence put on the
//! thread.
//!
//! One question comes before that one. [`prepare`] is also what resolves the
//! key, so a scope this checkout binds to nothing is a push it can only refuse —
//! and a reader who has just cloned the repository would be sent to a shell to
//! find `warlock key use`. So the binding is asked for first, between the home
//! and [`prepare`]: the key window goes up over the names this machine holds,
//! the `/push` waits in [`Parked`], and that answer decides whether there is a
//! push to prepare at all. The window is the one the `s` key puts up, held by
//! the session — nothing here keeps a second copy of it — and nothing on the
//! road reads a key: the question is about names, and the binding is written by
//! [`mod@crate::scoping`].
//!
//! A confirmed dialog files the [`Prepared`] it was drawn from, with nothing
//! resolved again: what the reader said yes to is what is sent.

use std::io;
use std::mem;
use std::path::{Path, PathBuf};
use std::time::Instant;

use warlock_engine::{Manifest, from_manifest_path, load_key_binding, to_manifest_path};

use crate::app::App;
use crate::confirm::{PushAnswered, PushConfirm};
use crate::error::{Error, one_line};
use crate::inflight::{Lost, Once, Workers, settled};
use crate::linear::{Opener as LinearOpener, Opens};
use crate::prompt::KeyPrompt;
use crate::push::{Prepared, file, prepare};
use crate::standing::Standing;

// Said to a `/push` typed with one already in flight, and it is the whole of
// that refusal: nothing is resolved, nothing is read and no window comes up. A
// brief filed twice is two projects on the board and nothing on this side can
// take one back, which is why the one-at-a-time rule is worth a line of its own
// rather than a queue.
pub(crate) const ALREADY_FILING: &str =
    "a brief is already on its way to the board; this one was not sent";

// Said of a lost push: nobody here knows how far it got, which is the honest
// answer for a mutation that is not idempotent.
const PUSH_LOST: &str =
    "the push stopped without saying how it went; look at the board before filing it again";

// `ready` is what the dialog was drawn from and is `Some` exactly while it is
// up, so a Yes files the push the reader was shown.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Pushing {
    pub(crate) confirm: PushConfirm,
    ready: Option<Prepared>,
}

impl Pushing {
    pub(crate) const fn closed() -> Self {
        Self {
            confirm: PushConfirm::Closed,
            ready: None,
        }
    }

    fn confirming(ready: Prepared) -> Self {
        Self {
            confirm: PushConfirm::open(ready.brief().name(), ready.destination().clone()),
            ready: Some(ready),
        }
    }
}

/// What a push has to say for itself, said exactly once: the address of the
/// project it created, or one line about why there is none.
///
/// A `String` rather than the error, because the error is worded on the worker
/// where it happens — out of `push.rs`'s own [`Error`], flattened — and a line is
/// what the thread takes.
type Landing = Result<String, String>;

/// The push a session is doing, if it is doing one, the window asking about the
/// next one, and where its client comes from.
///
/// The window is held here rather than beside the session's other windows for
/// [`crate::cutting::Cutter`]'s reason: it is a state of the push, and every
/// answer to it is this value's to act on.
///
/// The seam is held for the life of the process, the run is an [`Option`] that is its own one-at-a-time guard, and
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
    window: Pushing,
    /// The `/push` waiting on a binding, if one is. Never more than one: a key
    /// window takes every keystroke while it is up, so no second `/push` can be
    /// typed behind this one.
    parked: Option<Parked>,
    sending: Option<Sending>,
    workers: Workers,
}

/// A `/push` waiting on a binding: the scope it named and the brief it was
/// about, as that command resolved them.
///
/// Parked rather than resolved again when the window is answered, for the reason
/// the dialog holds its [`Prepared`]: a second reading of the command could name
/// another scope, or another brief, from the one the question was asked about.
/// And the scope only — no key, no name and nothing off the store — because the
/// window the reader is looking at is what holds the choice.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Parked {
    scope: String,
    brief: PathBuf,
}

/// What a `/push` comes to between two frames.
///
/// The window is the ordinary answer, and every refusal is that same arm with
/// the line already on the thread and nothing up. The other is the question
/// asked ahead of [`prepare`]: the binding, with the push to pick up once it is
/// written.
// Unboxed for [`RecordEdited`](crate::prompt::RecordEdited)'s reason: this is
// built by the one `/push` that made it, read by the caller that asked for it
// and dropped, between two frames of one keystroke. A `Box` would buy an
// allocation and a pointer chase on the keystroke's own thread.
#[allow(clippy::large_enum_variant)]
enum Next {
    Window(Pushing),
    Binding(Parked, KeyPrompt),
}

// The channel and the board its answer is about. The team rides along because
// the worker sends back an address and nothing about where it went, and the line
// that says it landed names the board the line that started it named.
#[derive(Debug)]
struct Sending {
    events: Once<Landing>,
    team: String,
}

impl Pushes<LinearOpener> {
    // The seam and the home, both settled here: building the seam costs nothing
    // — it is a unit value, and no socket exists until an answered dialog asks
    // for one — and a home that will not resolve is a `None` that every `/push`
    // is refused with in `Standing::home`'s own words.
    pub(crate) fn new() -> Self {
        Self::with_client(LinearOpener, Standing::home().ok())
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
            window: Pushing::closed(),
            parked: None,
            sending: None,
            workers: Workers::Threaded,
        }
    }

    #[cfg(test)]
    pub(crate) fn inline(self) -> Self {
        Self {
            workers: Workers::Inline,
            ..self
        }
    }

    // Read once a round by the loop and once per `/push` by this value itself,
    // off the one run it keeps: a flag beside it would be a second record of
    // whether a brief is in the air.
    pub(crate) const fn sending(&self) -> bool {
        self.sending.is_some()
    }

    pub(crate) const fn window(&self) -> &Pushing {
        &self.window
    }

    // `/push <SCOPE> <PATH>` typed into the composer, the path as typed. The
    // refusals are asked in the order they have to be: a push already in flight
    // is answered before anything is read, because resolving a board for a
    // request that cannot be sent would read three files to no purpose. The
    // path is spelled the manifest's way next, so one that climbs out of the
    // repository is refused in the engine's own words; the thread names files
    // against the repository root, so a typed path is read against it too.
    //
    // What comes back is the key window the session is to put up, which is
    // [`KeyPrompt::Closed`] on every road but the one unbound scope takes: the
    // session holds that window, and this value holds the push waiting behind
    // it.
    pub(crate) fn press(
        &mut self,
        app: &mut App,
        manifest: &Manifest,
        repo_root: &Path,
        scope: &str,
        typed: &str,
        now: Instant,
    ) -> KeyPrompt {
        // Whatever an earlier `/push` parked goes here: this command says which
        // scope and which brief the next question is about. Nothing can be
        // parked in practice — a key window up takes every keystroke, so no
        // `/push` reaches the composer behind one — and it is dropped anyway,
        // because a push that outlived its own window would be prepared by an
        // Enter meant for another one.
        self.parked = None;
        match self.pressed(app, manifest, repo_root, scope, typed, now) {
            Next::Window(window) => {
                self.window = window;
                KeyPrompt::Closed
            }
            // No dialog on this road and nothing resolved: there is no board,
            // no brief read and nothing to say yes to until the binding is
            // written.
            Next::Binding(parked, prompt) => {
                self.window = Pushing::closed();
                self.parked = Some(parked);
                prompt
            }
        }
    }

    fn pressed(
        &self,
        app: &mut App,
        manifest: &Manifest,
        repo_root: &Path,
        scope: &str,
        typed: &str,
        now: Instant,
    ) -> Next {
        if self.sending() {
            return Next::Window(saying(app, ALREADY_FILING, now));
        }
        let brief = match to_manifest_path(repo_root, typed) {
            Ok(brief) => brief,
            Err(source) => return Next::Window(saying(app, one_line(source), now)),
        };
        let Some(home) = self.home.as_deref() else {
            return Next::Window(no_home(app, now));
        };

        let path = from_manifest_path(repo_root, &brief);
        // After the home, which is where the config sits, and before `prepare`,
        // which is what resolves the key: a scope bound to nothing gets the
        // question rather than the refusal.
        match load_key_binding(home, repo_root, scope) {
            // Bound, which is every `/push` that worked before this question
            // existed: on to `prepare`, and the dialog over what it resolved.
            // The name is not kept — what the dialog names comes back from
            // `prepare` with the board, and this read is only whether there is
            // one.
            Ok(Some(_)) => {}
            // Bound to nothing, so nothing on this machine could file this
            // scope: the window, and the push parked behind it.
            Ok(None) => return binding(app, home, scope, &path, now),
            // A checkout nobody has configured, and a config that will not
            // read. The engine's own sentence on the thread, as every other
            // `/push` refusal is, and no window: the answer would be written
            // into the very file that could not be read.
            Err(source) => {
                return Next::Window(saying(app, one_line(Error::Sigils { source }), now));
            }
        }

        Next::Window(match prepare(manifest, repo_root, home, scope, &path) {
            Ok(ready) => Pushing::confirming(ready),
            Err(error) => saying(app, one_line(error), now),
        })
    }

    /// The parked push picked up over the binding the key window has just
    /// written: straight to `prepare`, which is the road a `/push` with a key
    /// bound takes on the round it is typed.
    ///
    /// Taken rather than read and then cleared, so one question leads to one
    /// dialog and no later Enter at the key window — from the `s` key's road, or
    /// from a session with no home — can pick this push up a second time. Does
    /// nothing at all when no `/push` is waiting, which is every other way that
    /// window is answered.
    ///
    /// A binding the submit could not write is not asked about again: `prepare`
    /// refuses an unbound scope in the engine's own words, and that line on the
    /// thread is the answer rather than a second window.
    pub(crate) fn bound(
        &mut self,
        app: &mut App,
        manifest: &Manifest,
        repo_root: &Path,
        now: Instant,
    ) {
        let Some(parked) = self.parked.take() else {
            return;
        };
        // The home this value was built with, as the press that parked the push
        // read it: a home cannot move under a running warlock, and this is the
        // one it was resolved against.
        let Some(home) = self.home.as_deref() else {
            self.window = no_home(app, now);
            return;
        };

        self.window = match prepare(manifest, repo_root, home, &parked.scope, &parked.brief) {
            Ok(ready) => Pushing::confirming(ready),
            Err(error) => saying(app, one_line(error), now),
        };
    }

    /// The key window abandoned, and the push that was waiting on it with it:
    /// nothing is prepared, no dialog opens and nothing is sent.
    ///
    /// The binding is the window's own to not write — Esc never opens the config
    /// — so this is the whole of what an Esc costs on this side.
    pub(crate) fn put_down(&mut self) {
        self.parked = None;
    }

    /// The dialog, moved or answered. An arrow re-lights the question that is
    /// up and either answer takes it down; a Yes also starts the worker.
    ///
    /// The window is taken rather than read and then closed, so what the
    /// question was asked about is what the request is made from and there is
    /// no round on which both a dialog and its own push are up.
    pub(crate) fn answer(&mut self, app: &mut App, answered: PushAnswered, now: Instant) {
        let window = &mut self.window;
        match answered {
            PushAnswered::Open(answer) => window.confirm = window.confirm.lit(answer),
            PushAnswered::Cancel => window.confirm = PushConfirm::Closed,
            PushAnswered::Send => {
                if let Some(ready) = mem::take(window).ready {
                    self.send(app, ready, now);
                }
            }
        }
    }

    fn send(&mut self, app: &mut App, ready: Prepared, now: Instant) {
        // `push` refused before the dialog that leads here was opened, and the
        // dialog holds every key until it is answered.
        debug_assert!(!self.sending(), "a second push reached send");

        let team = ready.destination().team_key().to_owned();
        // Before the worker starts, so the thread says which board is being
        // filed to from the instant it is: the answer may be seconds away and a
        // reader who has just said yes is looking at the conversation.
        app.panel_mut().note(filing_line(&team), now);
        self.sending = Some(Sending {
            team,
            events: spawn_push(self.workers, self.open.clone(), ready),
        });
    }

    /// What the push has said since the last round, which is one thing at most
    /// and ends the run either way.
    ///
    /// Drained rather than received: nothing here blocks, so frames keep being drawn, the tree keeps scrolling and the
    /// clocks keep ticking while the request is in flight.
    pub(crate) fn keep_up(&mut self, app: &mut App, now: Instant) {
        let Some((sending, landing)) =
            settled(&mut self.sending, |sending| sending.events.landed())
        else {
            return;
        };
        let line = match landing.unwrap_or_else(|Lost| Err(PUSH_LOST.to_owned())) {
            Ok(url) => filed_line(&sending.team, &url),
            // The worker's own sentence, which is `push.rs`'s wording of
            // whatever went wrong: a transport failure, Linear's refusal, a
            // project of the same name already in the team.
            Err(line) => line,
        };
        app.panel_mut().note(line, now);
    }
}

// No cancel handle — one request per operation, no retry and no backoff (see
// `linear.rs`), and a mutation that may already have run is not something a
// keystroke can take back.
//
// `io::sink` is where the line `file` prints would have gone. The panel words its
// own — a conversation is not a terminal — and the address it needs comes back
// from the call rather than out of the bytes.
fn spawn_push<O: Opens>(workers: Workers, open: O, ready: Prepared) -> Once<Landing> {
    workers.once(move || file(&ready, &open, &mut io::sink()).map_err(one_line))
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

// The key window over this machine's key names, with the push parked behind it.
//
// Names and no values, which is [`KeyPrompt::open`]'s whole shape: it lists the
// store through `load_key_names`, exactly as the `s` key's own road does, and
// there is no field on it a value could sit in.
//
// A free function because nothing about it is the push's: the home and the scope
// are handed in, and what comes back is a window the session puts up.
fn binding(app: &mut App, home: &Path, scope: &str, brief: &Path, now: Instant) -> Next {
    match KeyPrompt::open(home, scope) {
        Ok(prompt) => Next::Binding(
            Parked {
                scope: scope.to_owned(),
                brief: brief.to_path_buf(),
            },
            prompt,
        ),
        // A store that will not read, in the engine's own sentence on the
        // thread: there is nothing to pick from, so no window goes up and no
        // push waits for one.
        Err(source) => Next::Window(saying(app, one_line(Error::Keys { source }), now)),
    }
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
    saying(app, one_line(Error::NoHome), now)
}

// Every test drives a temporary repository and a temporary home, through the
// half of this module that takes both as parameters: nothing in the suite can
// read the sigils, the binding or the key store of the machine it runs on.
#[cfg(test)]
#[path = "tests/pushing.rs"]
mod tests;
