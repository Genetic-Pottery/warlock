//! `/pull`, from the command word to the project it would cut, read back off
//! the board.
//!
//! Everything a reader would be asked about a pull — the project's name, the
//! status it is sitting in, how many slices it has — is on the wire.
//! `.warlock/filed.toml` keeps an id and a URL and nothing else, so there is no
//! question to put up until a request has been answered: the fetch comes first
//! and what it has to say is a line on the thread.
//!
//! [`Pulls`] is [`crate::pushing::Pushes`]'s shape — the [`Opens`] seam held for
//! the life of the process, the home resolved once, an [`Option`] that is its
//! own say-no to a second run, a channel drained at the bottom of the loop —
//! with one difference: the board is resolved on the worker rather than on the
//! event loop's thread. A push resolves it first because a dialog is about to
//! name the team it found; a pull has nothing to put up yet, so the sequence
//! from the sigils to the slice count is one thing that fails in one place, and
//! the manifest crosses with the rest of the work as it does for a pact.
//!
//! The key is read on exactly one line in this file — the one that builds the
//! client — and what the worker holds either side of it is a manifest, two
//! paths and a spelling. No target outlives that line, so there is nowhere in
//! [`Pulls`], in [`Work`], in the channel's message or in a `Debug` rendering
//! for a key value to be.
//!
//! Nothing here writes. No mutation is sent, no status is moved and no record
//! is appended, which is what makes every refusal below cost nothing but the
//! line it is said in — and every one of those lines is the sentence
//! [`crate::error::Error`] already words for `warlock pull`, so a reader who
//! has met one at a shell meets the same words in the panel.

use std::mem;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread;
use std::time::Instant;

use warlock_engine::{Manifest, filed_path, resolve_filing};
use warlock_tui::{Answer, App, Cancel, PullConfirm, fetch_project, scope_block_in};

use crate::error::{Error, one_line};
use crate::pacting::CancelGuard;
use crate::pull::{counted, is_planned};
use crate::push::records;
use crate::pushing::{Linear, Opens};
use crate::standing::Standing;

// Said to a `/pull` typed with one already running, and it is the whole of that
// refusal: no board is resolved, no record is read and no request is made. Two
// pulls at once would be two sets of drafting sessions spending on one
// conversation's behalf, which is why the one-at-a-time rule is worth a line of
// its own rather than a queue.
pub(crate) const ALREADY_PULLING: &str = "a pull is already running; this one read nothing";

// The worker sends on every path it takes, so a channel that closes with
// nothing on it is a panic. The hook has already printed it; what is left to
// say is that nothing was read — which is the honest answer for a sequence that
// only reads, and the reason this sentence promises more than a lost push's
// does.
const PULL_LOST: &str = "the pull stopped without saying how it went; nothing was read or changed";

/// What a pull has to say for itself once the board has answered: one line, or
/// the facts a line is worded from.
///
/// A `String` for the failure, because the failure is worded on the worker
/// where it happens — out of `error.rs`, flattened — and a line is what the
/// thread takes.
type Landing = Result<Fetched, String>;

/// The project a `/pull` found, gated and parsed: what is on the board, what
/// state it is in, and how much of it is left to cut.
///
/// Held as the facts rather than as the line they are said in because they are
/// what a reader is asked to confirm next, and none of them can be read again
/// without a second request.
///
/// The board is in here twice over — the team the project sits on and the
/// *name* the key that read it is held under — because the dialog names both
/// and the worker is the only thing that resolved them. The key value is not
/// among them and cannot be: it lives on one line of this module (see the
/// module's note), and there is nowhere in this value, in the channel's message
/// or in a `Debug` rendering for its bytes to sit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Fetched {
    project: String,
    // The board's own spelling and not `Planned`, because that is what somebody
    // sent to look will find written on the project: the gate folds case and
    // trims, so the two need not match.
    status: String,
    slices: usize,
    uncut: usize,
    team: String,
    key: String,
}

impl Fetched {
    fn line(&self) -> String {
        format!(
            "`{}` is `{}` — {}, {} still to cut",
            self.project,
            self.status,
            counted(self.slices),
            self.uncut
        )
    }

    // The question these facts are asked as. The count it names is the one the
    // line above names first — how many slices the project has — because that
    // is what the reader is being asked about the project, and how much of it
    // is left is the line's to say.
    fn asking(&self) -> PullConfirm {
        PullConfirm::open(
            &self.project,
            &self.status,
            self.slices,
            &self.team,
            &self.key,
        )
    }
}

/// The pull a session is doing, if it is doing one, and where its client comes
/// from.
///
/// The home is here for [`crate::pushing::Pushes`]'s reason: it cannot move
/// under a running warlock, and a second reading per keystroke would be a second
/// answer. It is also what keeps every test in this crate off the developer's
/// own — a `Pulls` is built with the home it is to use, so nothing below this
/// line asks the environment.
#[derive(Debug)]
pub(crate) struct Pulls<O: Opens> {
    open: O,
    home: Option<PathBuf>,
    fetching: Option<Fetching>,
    // The question between the fetch and the run, held here rather than beside
    // the session's other windows because it is a state of the pull and not of
    // the app: what it is asked about came off the wire on the round it went
    // up, and nothing else in the panel can answer it.
    confirm: PullConfirm,
}

// The channel, and the handle that is the whole of how a pull is stopped.
//
// Dropping the session drops this, which cancels: no exit path has to remember
// to stop a pull, exactly as none has to remember to stop a pact. What a cancel
// can reach is either side of the request rather than the request itself —
// there is one request, one timeout and no retry (see `linear.rs`), and nothing
// on this side can interrupt a socket that is already waiting.
#[derive(Debug)]
struct Fetching {
    events: Receiver<Landing>,
    // Never read, and that is the whole of what it does: the guard's `Drop` is
    // the cancel, so the field being here is the session's exit path. A handle
    // somebody had to remember to call would be one somebody could forget to.
    #[expect(
        dead_code,
        reason = "held for its drop, which is what cancels the fetch in flight"
    )]
    cancel: CancelGuard,
}

impl Pulls<Linear> {
    pub(crate) fn new() -> Self {
        Self::with_client(Linear, Standing::home().ok())
    }
}

impl<O: Opens> Pulls<O> {
    // The seam a test drives the real value over a stand-in client through,
    // rather than assembling the pieces underneath and proving something about
    // an arrangement the event loop never has.
    pub(crate) const fn with_client(open: O, home: Option<PathBuf>) -> Self {
        Self {
            open,
            home,
            fetching: None,
            confirm: PullConfirm::Closed,
        }
    }

    // Read once a round by the loop, to draw the window and to decide which
    // window a keystroke belongs to.
    pub(crate) const fn confirm(&self) -> &PullConfirm {
        &self.confirm
    }

    // Read once a round by the loop and once per `/pull` by this value itself,
    // off the one run it keeps: a flag beside it would be a second record of
    // whether a project is being read back.
    pub(crate) const fn fetching(&self) -> bool {
        self.fetching.is_some()
    }

    /// `/pull` typed into the composer, with the brief it is about already
    /// spelled the manifest's way.
    ///
    /// The two refusals are asked in the order they have to be: a pull already
    /// running is answered before anything is read, because a machine with no
    /// home is a fact about a request that is not going to be made anyway.
    /// Everything past them is the worker's, so this returns having read
    /// nothing but the clock.
    pub(crate) fn press(
        &mut self,
        app: &mut App,
        manifest: &Manifest,
        repo_root: &Path,
        brief: &str,
        now: Instant,
    ) {
        if self.fetching() {
            app.panel_mut().note(ALREADY_PULLING, now);
            return;
        }
        // `Standing::home`'s own sentence, asked of the error that words it
        // rather than written again here: the session holds the answer to that
        // reading and not the failure, so this is where the failure is said.
        let Some(home) = self.home.clone() else {
            app.panel_mut()
                .note(one_line(&Error::NoHome.to_string()), now);
            return;
        };

        let work = Work {
            manifest: manifest.clone(),
            root: repo_root.to_path_buf(),
            home,
            brief: brief.to_owned(),
        };
        // Before the worker starts, so the thread says which document is being
        // read back from the instant it is: the answer is a request away and a
        // reader who has just typed the command is looking at the conversation.
        app.panel_mut().note(reading_line(brief), now);
        let cancel = CancelGuard::new();
        self.fetching = Some(Fetching {
            events: spawn_fetch(self.open.clone(), work, cancel.handle()),
            cancel,
        });
    }

    /// What the pull has said since the last round, which is one thing at most
    /// and ends the fetch either way.
    ///
    /// Drained rather than received, for [`crate::pacting`]'s reason: nothing
    /// here blocks, so frames keep being drawn, the tree keeps scrolling and the
    /// clocks keep ticking while the request is in flight.
    pub(crate) fn keep_up(&mut self, app: &mut App, now: Instant) {
        let Some(fetching) = self.fetching.as_ref() else {
            return;
        };

        let landing = match fetching.events.try_recv() {
            Ok(landing) => landing,
            // Still in flight, and nothing new to say.
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => Err(PULL_LOST.to_owned()),
        };
        let line = match &landing {
            Ok(fetched) => fetched.line(),
            // The worker's own sentence, which is `error.rs`'s wording of
            // whatever stopped it: a machine that cannot say which board it
            // stands at, a brief nothing filed, a project Linear no longer has,
            // one that is not planned, content that is not a scope, or a
            // project with nothing left to cut.
            Err(line) => line.clone(),
        };
        // Taken before the line is put on the thread, so the round that reports
        // a pull is a round on which the next `/pull` is already allowed.
        self.fetching = None;
        app.panel_mut().note(line, now);
        // The question goes up on the round the answer landed, over the line
        // that reports it: what a reader is being asked to confirm is what they
        // have just read. A fetch that failed put its own sentence on the
        // thread and there is nothing to ask about, so nothing opens.
        if let Ok(fetched) = landing {
            self.confirm = fetched.asking();
        }
    }

    /// The same question with the other answer lit. A closed dialog stays
    /// closed, which is [`PullConfirm::lit`]'s rule and not a second one here.
    pub(crate) fn lit(&mut self, answer: Answer) {
        self.confirm = self.confirm.lit(answer);
    }

    /// The question taken down with nothing started: no request is made, no
    /// record is read and the session is left exactly where it was, which is
    /// the whole of what a No costs.
    pub(crate) fn cancelled(&mut self) {
        self.confirm = PullConfirm::Closed;
    }

    /// The confirmed question: the window down and the run begun.
    ///
    /// Taken rather than read and then closed, so there is no round on which
    /// both the question and its own run are up.
    pub(crate) fn cut(&mut self, app: &mut App, now: Instant) {
        let confirm = mem::replace(&mut self.confirm, PullConfirm::Closed);
        let Some(cutting) = confirm.cutting() else {
            return;
        };
        app.panel_mut().note(cutting_line(cutting.project()), now);
    }
}

// Everything the worker owns, and the whole of what crosses the thread
// boundary beside the seam: the manifest is cloned as a pact's is, because the
// board is resolved over there and a `Target` borrows the manifest it was found
// in.
#[derive(Debug, Clone)]
struct Work {
    manifest: Manifest,
    root: PathBuf,
    home: PathBuf,
    // The manifest's own spelling, which is what a filed record is keyed by:
    // the composer's `/pull docs/a-brief.md` and the path `/write` remembered
    // are both spelled before they get here, so nothing below resolves a path
    // against a working directory.
    brief: String,
}

// The `JoinHandle` is dropped on purpose, as a turn's and a push's are: joining
// is waiting, and this thread exists precisely so nobody waits for it. The
// guard the caller keeps is what stops it.
fn spawn_fetch<O: Opens>(open: O, work: Work, cancel: Cancel) -> Receiver<Landing> {
    let (events, received) = mpsc::channel();
    thread::spawn(move || {
        let landing = fetched(&open, &work, &cancel).map_err(|error| one_line(&error.to_string()));
        // Ignored for the reason every other worker's send is: a receiver that
        // has gone away is an application that is quitting, which is also the
        // one thing that cancels a pull.
        let _ = events.send(landing);
    });

    received
}

// The whole of what a `/pull` does before anybody is asked anything, in the
// order `pull.rs` runs it: the board, the brief's own record, the project, the
// gate, the scope block, and what is left to cut. Everything is a parameter —
// the manifest, the root, the home, the seam — so the whole sequence runs
// against a temporary repository and a temporary home, and no test in this
// crate can reach the developer's key store by standing in the wrong directory.
//
// The cancel is asked either side of the request and nowhere else. A socket
// already waiting cannot be interrupted, so what the guard buys is a worker
// that neither opens one after the session has gone nor reads the repository
// for an answer nobody will hear.
fn fetched<O: Opens>(open: &O, work: &Work, cancel: &Cancel) -> Result<Fetched, Error> {
    if cancel.is_cancelled() {
        return Err(Error::Cancelled);
    }

    let target = resolve_filing(&work.manifest, &work.root, &work.home, None)
        .map_err(|source| Error::Filing { source })?;
    let filed = records(&work.root)?;
    let Some(record) = filed.record(&work.brief) else {
        return Err(Error::NoRecord {
            path: work.brief.clone(),
        });
    };

    // The key is read here and on no other line in this module, on the last
    // line before anything can leave the machine — `push.rs` and `pushing.rs`
    // read it in the same one place for the same reason.
    let linear = open.open(target.value());
    let project = fetch_project(&linear, record.project_id())
        .map_err(|source| Error::Linear { source })?
        .ok_or_else(|| Error::UnknownProject {
            id: record.project_id().to_owned(),
            path: filed_path(&work.root),
        })?;

    if cancel.is_cancelled() {
        return Err(Error::Cancelled);
    }

    // The gate is `pull.rs`'s, asked of the status the board answered with, so
    // the panel and the subcommand cannot come to disagree about what `Planned`
    // spells. What comes back out of it is that same string, for the line.
    let Some(status) = project.status().filter(|status| is_planned(status)) else {
        return Err(Error::NotPlanned {
            path: work.brief.clone(),
            status: project.status().map(ToOwned::to_owned),
        });
    };
    let status = status.to_owned();

    let block = scope_block_in(project.content()).map_err(|source| Error::ScopeBlock { source })?;
    // `ordered` and not `slices`: the count reported is of the slices a run
    // would walk, in the order it would walk them, so a `depends_on` that names
    // something unreachable is refused here rather than counted.
    let slices = block.ordered();
    // Asked of the file rather than worked out here: `cut_state` matches on the
    // key a record spells and not on a fresh fold of the title beside it, which
    // is the difference between a slice somebody renamed and one warlock
    // quietly treats as already filed.
    let state = filed.cut_state(&work.brief, slices.iter().map(|slice| slice.heading()));
    if state.uncut().is_empty() {
        return Err(Error::AllCut {
            path: work.brief.clone(),
        });
    }

    Ok(Fetched {
        project: project.name().to_owned(),
        status,
        slices: slices.len(),
        uncut: state.uncut().len(),
        team: target.record().team().to_owned(),
        // The key by name. `Target::value` is read on one line above and
        // nowhere else on this path.
        key: target.key().to_owned(),
    })
}

// What a Yes has to say for itself so far. The run it starts is the next slice
// of this work; what is true already is that the question was answered and the
// project it named is the one being cut.
fn cutting_line(project: &str) -> String {
    format!("cutting `{project}` into tickets")
}

// Named by the document rather than by the project, because the project has no
// name on this side of the request: what the reader typed is a brief, and the
// record that turns it into an id is the only thing warlock holds until the
// board answers.
fn reading_line(brief: &str) -> String {
    format!("reading the project filed for `{brief}`")
}

// Every test drives a temporary repository and a temporary home, through the
// half of this module that takes both as parameters: nothing in the suite can
// read the sigils, the binding or the key store of the machine it runs on.
#[cfg(test)]
#[path = "tests/pulling.rs"]
mod tests;
