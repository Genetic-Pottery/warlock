//! `/push`, from the command word to the question the reader answers to the
//! project on the board.
//!
//! Everything up to that answer happens on the event loop's own thread between
//! two frames and opens no socket at all: [`prepare`] is a manifest, a sigil
//! file, a key store and the brief, all of them local. The
//! answer is the one thing that leaves the machine, and it leaves on a worker
//! thread — see [`Pushes`], which is [`crate::pacting::Pact`]'s shape for a
//! request instead of a pass: a channel per run, drained at the bottom of the
//! loop, and the `Option` holding it is itself the say-no to a second one.
//!
//! What this file adds to [`mod@crate::push`] is the asking: the dialog over
//! what [`prepare`] resolved, and the lines on the thread. The boundary rule is
//! nowhere in this file, and every refusal is `push.rs`'s own sentence put on the
//! thread.
//!
//! A confirmed dialog files the [`Prepared`] it was drawn from, with nothing
//! resolved again: what the reader said yes to is what is sent.

use std::io;
use std::mem;
use std::path::{Path, PathBuf};
use std::time::Instant;

use warlock_engine::{Manifest, from_manifest_path};

use crate::app::App;
use crate::confirm::{PushAnswered, PushConfirm};
use crate::error::{Error, one_line};
use crate::inflight::{Lost, Once, Workers, settled};
use crate::linear::{Opener as LinearOpener, Opens};
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
    window: Pushing,
    sending: Option<Sending>,
    workers: Workers,
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

    // `/push <SCOPE> <PATH>` typed into the composer, with the path already
    // spelled the manifest's way. The two refusals are asked in the order they
    // have to be: a push already in flight is answered before anything is read,
    // because resolving a board for a request that cannot be sent would read
    // three files to no purpose.
    pub(crate) fn press(
        &mut self,
        app: &mut App,
        manifest: &Manifest,
        repo_root: &Path,
        scope: &str,
        brief: &str,
        now: Instant,
    ) {
        self.window = self.pressed(app, manifest, repo_root, scope, brief, now);
    }

    fn pressed(
        &self,
        app: &mut App,
        manifest: &Manifest,
        repo_root: &Path,
        scope: &str,
        brief: &str,
        now: Instant,
    ) -> Pushing {
        if self.sending() {
            return saying(app, ALREADY_FILING, now);
        }
        let Some(home) = self.home.as_deref() else {
            return no_home(app, now);
        };

        let path = from_manifest_path(repo_root, brief);
        match prepare(manifest, repo_root, home, scope, &path) {
            Ok(ready) => Pushing::confirming(ready),
            Err(error) => saying(app, one_line(&error.to_string()), now),
        }
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
        if self.sending() {
            saying(app, ALREADY_FILING, now);
            return;
        }

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
    /// Drained rather than received, for [`crate::pacting`]'s reason: nothing
    /// here blocks, so frames keep being drawn, the tree keeps scrolling and the
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
    workers.once(move || {
        file(&ready, &open, &mut io::sink()).map_err(|error| one_line(&error.to_string()))
    })
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

// Every test drives a temporary repository and a temporary home, through the
// half of this module that takes both as parameters: nothing in the suite can
// read the sigils, the binding or the key store of the machine it runs on.
#[cfg(test)]
#[path = "tests/pushing.rs"]
mod tests;
