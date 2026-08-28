//! A pact run on a worker thread, from the keystroke to the saved manifest.
//!
//! The pact key is the one keystroke that writes to disk and the one that
//! takes longer than a frame, so it is the one that is not done on the event
//! loop's thread. [`pact_press`] decides what a press comes to, [`spawn_pact`]
//! starts the worker and hands back the channel it reports on, and
//! [`apply_progress`] is the loop's other half: everything the worker says —
//! which directory it is on, what its pass is doing, and finally how it went —
//! lands on the app there, and the run ends with the one reload that puts the
//! documents it wrote on screen.
//!
//! The refresh key is the same machinery over a shorter list. [`refresh_press`]
//! is [`pact_press`]'s twin, [`Work`] is the one value that says which of the
//! two a run is, and everything below that — the worker, the channel, the
//! account, the say-when, the single save, the reload — is shared rather than
//! written twice: the difference between the two runs is one engine call in
//! [`apply_toggle`] and one verb on the footer.
//!
//! Stopping a run has two spellings kept deliberately apart: Esc *cancels*
//! through [`CancelGuard`], and the worker still hashes, saves and reports
//! what it finished; quitting drops the guard, which kills the `claude` in
//! flight and leaves the manifest as it was. Either way no half-state exists
//! for an abandoned worker to leave — see [`spawn_pact`] for the bargain.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::time::Instant;
use std::{fs, io, thread};

use warlock_engine::{
    Agent, Manifest, NodeState, PactFailure, PactObserver, PactProblem, PactedSubtree, Pacting,
    Tree, pact_subtree, refresh_subtree, to_manifest_path, unpact_subtree,
};
use warlock_tui::{
    Activities, Activity, App, Cancel, ClaudeAgent, Outcome, PactToggle, Run, Section,
};

use crate::error::{Error, one_line};
use crate::session::{Scope, reload_tree};

/// What the footer says when the worker thread stopped without reporting
/// anything — which, since it reports on every path it takes itself, means it
/// panicked.
///
/// The panic hook has already printed the panic where it can be read, so this
/// exists to say that the run is over and that Warlock's own record of what is
/// pacted did not move. Documents the worker had already written are still on
/// disk, and the next load will find them.
const PACT_LOST: &str = "the pact stopped without saying how it went; nothing new was recorded";

/// What the footer says about a run the reader stopped with Esc.
///
/// It replaces whatever [`pact_message`] would have said, and that is the point
/// of it. A cancel kills the pass in flight, so the directory it was working
/// comes back as a failure, and every directory after it was never offered a
/// pass at all; reporting any of that as something that *went wrong* would put a
/// directory's name on the footer as if the reader had to go and look at it,
/// when the only thing that happened is that they pressed Esc. What did get
/// written is on disk and recorded, and the tree says which parts those are the
/// next time it is loaded.
const PACT_CANCELLED: &str = "the pact was cancelled; what it finished first is recorded";

/// A pact being run by a worker thread, from the point of view of the thread
/// drawing the screen.
///
/// Four things, and no handle to join: what the worker has to say, how to tell
/// it to stop, what it was asked to do, and what the tree looked like before the
/// keystroke painted that subtree yellow.
///
/// The work is kept here rather than read back off the app because the subtree
/// it names is the one the *run* covers, which is a fact about the run and not
/// about whatever is selected by the time it ends — the reader is free to move
/// the selection anywhere while a run works, and does. It is also what says
/// whether the footer's line reads as pacting or as refreshing, for the length
/// of the run and not just at the press.
///
/// The copy of the app is the undo for a run that comes back with nothing
/// recorded. It is taken before the toggle paints, so it is also older than any
/// scrolling done during the run; putting it back costs the reader their place
/// in the tree on a path that only a failed walk or an unwritable manifest
/// reaches. That is the same restoration the blocking version did, kept as one
/// code path rather than split into "the rows, but not the selection".
///
/// The say-when is a [`CancelGuard`] rather than a bare handle, so that every
/// way out of the event loop takes the running `claude` with it whether or not
/// it remembered to; see that type for the bargain.
pub(crate) struct Running {
    /// Progress and, once, the outcome. Closed by the worker dropping its end,
    /// which is how a panicked worker is noticed.
    pub(crate) events: Receiver<PactEvent>,
    /// Say-when for the worker: the flag its observer reads between directories
    /// and the kill switch for the `claude` it is waiting on.
    pub(crate) cancel: CancelGuard,
    /// What the key that started this run asked for, which names the subtree it
    /// covers and says which of the two runs it is.
    pub(crate) work: Work,
    /// The app as it stood before the toggle painted the subtree.
    pub(crate) before: App,
}

/// What a worker thread was asked to do: carry a pact toggle out, or refresh a
/// subtree.
///
/// One value rather than two of everything, because below the keystroke there
/// is nothing to tell the two runs apart. A refresh is the same worker on the
/// same channel reporting through the same observer into the same account,
/// stoppable by the same handle and ending in the same single save and the same
/// reload; the whole of the difference is which engine entry point
/// [`apply_toggle`] calls — [`pact_subtree`] describes every directory of the
/// subtree, [`refresh_subtree`] only the stale ones — and which verb the
/// footer's progress line is worded with.
///
/// A refresh carries a bare directory where a pact carries a [`PactToggle`],
/// and that is the whole of the shape difference: a pact goes two ways and has
/// to say which, while there is no such thing as an un-refresh.
#[derive(Debug, Clone)]
pub(crate) enum Work {
    /// The pact key's press: describe the whole subtree, or take it out of the
    /// manifest, as [`PactToggle::pacted`] says.
    Pact(PactToggle),
    /// The refresh key's press: describe the stale directories under this one
    /// and leave the fresh ones exactly as they are.
    Refresh(PathBuf),
}

impl Work {
    /// The root of the subtree the run covers, whichever key asked for it.
    pub(crate) fn path(&self) -> &Path {
        match self {
            Self::Pact(toggle) => &toggle.path,
            Self::Refresh(directory) => directory,
        }
    }

    /// Which of the two runs this is, for the one line the app words about a
    /// run in flight: see [`Run`] and
    /// [`App::set_run_in_flight`](warlock_tui::App::set_run_in_flight).
    pub(crate) const fn kind(&self) -> Run {
        match self {
            Self::Pact(_) => Run::Pact,
            Self::Refresh(_) => Run::Refresh,
        }
    }

    /// Whether Esc can stop this run part way through.
    ///
    /// Anything that runs model passes can: a pact and a refresh are both a
    /// descent of minutes, and both hash and save whatever they finished before
    /// the key landed. An un-pact cannot — it is manifest arithmetic that is
    /// over before a key can be read, so a handle latched while one ran would be
    /// describing something else entirely.
    const fn is_cancellable(&self) -> bool {
        match self {
            Self::Pact(toggle) => toggle.pacted,
            Self::Refresh(_) => true,
        }
    }
}

/// A [`Cancel`] that is spent when it goes out of scope.
///
/// The same bargain [`TerminalGuard`] strikes with the terminal, for the same
/// reason: a quit, an error bubbling up through a `?` in the middle of a frame
/// and a panic unwinding past here are all ways out of the event loop, and each
/// of them would otherwise have to remember to stop the run — which is to say,
/// one of them eventually would not, and would leave a `claude` burning the
/// user's subscription with nobody left to read what it says.
///
/// A type of its own rather than a `Drop` on [`Running`], so that the outcome
/// path can still move the app it kept out of it: a struct that implements
/// `Drop` cannot be taken apart, and this one has nothing anybody wants to take.
pub(crate) struct CancelGuard {
    /// The handle every clone of which speaks for this run.
    cancel: Cancel,
}

impl CancelGuard {
    /// A handle nobody has said stop to yet.
    pub(crate) fn new() -> Self {
        Self {
            cancel: Cancel::new(),
        }
    }

    /// A clone for the worker to give its agent and its observer.
    pub(crate) fn handle(&self) -> Cancel {
        self.cancel.clone()
    }

    /// Stop the run: latch the flag the descent reads, and kill the `claude`
    /// in flight.
    pub(crate) fn cancel(&self) {
        self.cancel.cancel();
    }

    /// Whether somebody said stop to this run.
    ///
    /// Asked once, when the run's outcome comes back, and only so that the
    /// panel's last section can say the run was stopped rather than that its
    /// pass went wrong. The footer already has this fact by another road — the
    /// worker rewords a cancelled outcome before sending it ([`cancelled`]) —
    /// but that wording is one line about the whole run, and what the panel
    /// needs is which directory it happened in, which only the thread holding
    /// the sections knows.
    fn is_cancelled(&self) -> bool {
        self.cancel.is_cancelled()
    }
}

impl Drop for CancelGuard {
    /// Whatever ended the run, no child outlives it.
    ///
    /// Idempotent, and on the ordinary path a no-op: a run that reported its
    /// outcome has no pass left to kill, and the latched flag dies here with the
    /// last handle holding it.
    fn drop(&mut self) {
        self.cancel();
    }
}

/// What a worker thread has to say for itself.
///
/// Five things, and the order of two of them is fixed: one
/// [`PactEvent::Starting`] per directory as the run reaches it, any number of
/// [`PactEvent::Doing`] from the pass that directory is running and of
/// [`PactEvent::Summarising`] from the passes over the big files inside it, at
/// most one [`PactEvent::Documented`] per directory as its pass delivers, and
/// then exactly one [`PactEvent::Finished`]. Nothing else is sent, and nothing
/// is sent after the outcome — the worker drops its end of the channel and
/// stops.
///
/// Activities ride this channel rather than one of their own because there is
/// nothing to gain from a second: they come from the same worker, they are read
/// by the same thread, and a second receiver would be a second thing the event
/// loop has to poll and a second way for the two streams to arrive out of the
/// order the run produced them in.
#[derive(Debug)]
pub(crate) enum PactEvent {
    /// The pass for `directory` is about to run: directory `position` of
    /// `total`, counting from one, in the order the engine works them.
    Starting {
        /// The directory being worked, as an absolute path; the footer spells
        /// it relative to the tree on screen.
        directory: PathBuf,
        /// Which directory of the run this is, counting from one.
        position: usize,
        /// How many directories the whole run covers.
        total: usize,
    },
    /// The pass running now was seen doing something: a tool call, a stretch of
    /// thinking, or what it cost.
    ///
    /// Carries no directory, because it needs none to be delivered — the
    /// [`Starting`](PactEvent::Starting) before it says which directory is being
    /// worked, and anything more is the business of whoever draws these rather
    /// than of the channel that carries them.
    Doing(Activity),
    /// A summarising pass over one over-cap file inside the directory being
    /// worked is about to run: pass `part` of `parts`, counting from one.
    ///
    /// The reason a directory holding a two-megabyte lockfile is minutes long,
    /// said while it is being paid for rather than afterwards. Like
    /// [`Doing`](PactEvent::Doing) it carries no directory, because the
    /// [`Starting`](PactEvent::Starting) before it already named the one whose
    /// pass is running; unlike it, it names the file, because the file is the
    /// whole of what it has to say.
    ///
    /// `parts` counts *passes*, not chunks — a file read in three chunks is
    /// announced four times, the last of them the reduce over them — because
    /// that is the fraction of the wait a reader can do something with. See
    /// [`PactObserver::summarising`], whose numbers these are, unaltered.
    Summarising {
        /// The file being summarised, as an absolute path; the footer spells it
        /// relative to the tree on screen, as it does a directory.
        file: PathBuf,
        /// Which pass over that file this is, counting from one.
        part: usize,
        /// How many passes the file costs in all.
        parts: usize,
    },
    /// `directory` and everything under it is documented: its pass delivered,
    /// and no pass below it failed. The engine's own word, sent the moment it
    /// becomes true — see [`PactObserver::documented`] — and the one event that
    /// recolours rows while a run is still going: a finished directory turns
    /// green there and then instead of staying yellow until the whole batch is
    /// over.
    Documented {
        /// The directory, as an absolute path.
        directory: PathBuf,
    },
    /// The run is over, however it went: exactly what [`apply_toggle`] returned.
    Finished(Result<Toggled, String>),
}

/// An activity port that forwards to `events`, for the agent of one run.
///
/// The other half of the shape [`spawn_pact`] already gives [`Cancel`]: a
/// handle made per run, attached to that run's own copy of the agent, and
/// spent when the run ends. That is not decoration. The event loop's long-lived
/// [`ClaudeAgent`] keeps the port it was built with, which is one nobody
/// listens to, so a pass that outlives the run that started it — a `claude`
/// still writing to a pipe while the worker is being torn down — has no way to
/// report into the run after it.
///
/// The closure is called on the worker's thread, from inside the pass, while
/// the pass is still going, so it does the least it can: one send and back.
/// A send that fails is ignored for the same reason [`Reporting`]'s are — a
/// receiver that has gone away is an application that is quitting — and here
/// there is the additional reason that this one is called from inside a model
/// pass, where the only thing an error could do is fail work that is otherwise
/// going fine for the sake of a screen nobody is looking at.
///
/// The port holds a clone of the sender, so this run's agent is one of the
/// things keeping the channel open. That costs nothing on the real path — the
/// agent lives in the worker's closure and dies when the worker's body returns,
/// which is after the outcome has been sent — but it is why the loop's own
/// long-lived agent must never be given one of these: a port on it would hold
/// the channel of whichever run made it open for as long as warlock runs.
fn activity_port(events: &Sender<PactEvent>) -> Activities {
    let events = events.clone();
    Activities::new(move |activity| {
        let _ = events.send(PactEvent::Doing(activity));
    })
}

/// The engine's progress port, wired to a channel and to the reader's Esc.
///
/// The one adapter between an operation that knows which directory it is on and
/// a thread that can draw. Both directions of the port go through it: the
/// directory about to be worked goes out over the channel, and the answer comes
/// back off the cancel handle the event loop kept a clone of, which is the only
/// place a stop can come from — nobody but a person at a keyboard decides that a
/// pact has gone on long enough.
///
/// The summarising passes inside a directory go out the same way and ask
/// nothing: they are an announcement of what is being paid for while it is
/// being paid for, and the port answers them with a send and nothing else.
///
/// The handle is read before anything is sent, so a cancelled run neither
/// announces a directory it will not work nor works it. The engine's rule that
/// the answer is asked for between directories is what bounds how long a cancel
/// takes; killing the pass in flight, which [`Cancel`] does in the same breath
/// as latching, is what makes that bound milliseconds rather than a pass.
///
/// A send that fails means the event loop has dropped its receiver, which means
/// warlock is on its way out. It is deliberately ignored rather than turned into
/// a stop: the process is ending, the work in flight is about to end with it,
/// and there is nothing here worth reporting to a screen that is already gone.
struct Reporting<'a> {
    /// The event loop's end of the channel.
    events: &'a Sender<PactEvent>,
    /// The reader's say-when, cloned from the one the event loop holds.
    cancel: &'a Cancel,
}

impl PactObserver for Reporting<'_> {
    fn starting(&mut self, directory: &Path, position: usize, total: usize) -> Pacting {
        if self.cancel.is_cancelled() {
            return Pacting::Stop;
        }
        let _ = self.events.send(PactEvent::Starting {
            directory: directory.to_path_buf(),
            position,
            total,
        });
        Pacting::Continue
    }

    /// Pass the announcement on and get out of the way.
    ///
    /// One send, exactly as `starting` does it and with the same shrug at a
    /// failure: a receiver that has gone away is an application that is
    /// quitting. No cancellation check of its own, deliberately — this is
    /// called from inside a directory's pass, where the engine asks nothing and
    /// could act on no answer, and the one place a pact stops is still the
    /// question asked between directories above.
    fn summarising(&mut self, file: &Path, part: usize, parts: usize) {
        let _ = self.events.send(PactEvent::Summarising {
            file: file.to_path_buf(),
            part,
            parts,
        });
    }

    /// Pass the announcement on, exactly as `summarising` is passed on and
    /// with the same shrug at a send that fails.
    fn documented(&mut self, directory: &Path) {
        let _ = self.events.send(PactEvent::Documented {
            directory: directory.to_path_buf(),
        });
    }
}

/// Run `work` on a thread of its own, and hand back the channel it reports on.
///
/// A pact and a refresh come through here alike, because from here down they
/// are the same run: what tells them apart is inside [`Work`] and is spent in
/// one `match` at the bottom of [`apply_toggle`].
///
/// The worker owns everything it touches — its own manifest, its own root, its
/// own copy of the work, its own [`ClaudeAgent`], which is a command line and a
/// timeout and so is cheap to clone — so nothing is shared with the event loop but the
/// channel and `cancel`. The handle is the exception that proves the rule: it is
/// a flag and a slot for one child, written by whoever says stop and read
/// between directories, and it is what the agent given to this run answers to,
/// so the pass in flight is killed by the same call that ends the descent.
///
/// That ownership is what makes the thread safe to abandon: the loop can return
/// and the process exit at any moment without waiting for it, because there is
/// no state the two of them are half way through agreeing on. Each `WARLOCK.md`
/// is written beside and renamed over (WAR-21.01), so an abandoned worker leaves
/// whole documents or none, never half of one, and the manifest is written once
/// at the very end or not at all.
///
/// The [`JoinHandle`](std::thread::JoinHandle) is dropped on purpose: joining is
/// waiting, and this thread is started precisely so that nobody waits for it.
/// The worker reports on every path it takes itself, so the only way the channel
/// closes without an outcome is a panic in the worker — which the caller reads
/// as the run being over ([`apply_progress`]) rather than as a reason to hang.
pub(crate) fn spawn_pact(
    manifest: &Manifest,
    repo_root: &Path,
    work: &Work,
    agent: &ClaudeAgent,
    cancel: Cancel,
) -> Receiver<PactEvent> {
    let (events, received) = mpsc::channel();
    let (manifest, repo_root, work) = (manifest.clone(), repo_root.to_path_buf(), work.clone());
    // This run's copy of the agent, and the only one that answers to this run's
    // handle: the agent the event loop keeps has a handle of its own that nobody
    // else holds, so cancelling one run can never reach into the next.
    //
    // The activity port is attached the same way and for the same reason, in the
    // same breath — see `activity_port`. Both are one-per-run, and both die with
    // the copy of the agent this thread owns.
    let agent = agent
        .clone()
        .with_cancel(cancel.clone())
        .with_activities(activity_port(&events));
    thread::spawn(move || run_pact(&manifest, &repo_root, &work, &agent, &cancel, &events));
    received
}

/// Everything a press that really starts a run comes to, once the press itself
/// has decided there is one.
///
/// The three things the event loop has to keep about work it is not doing,
/// gathered where they belong together: the worker and the channel it reports
/// on ([`spawn_pact`]), the say-when the reader's Esc latches, and the copy of
/// the app taken before the keystroke painted anything.
///
/// One function for both keys, and that is the point of it rather than a saving
/// of lines: a pact and a refresh are the same run, so they are started by the
/// same code and the loop holds the same one value for either. The handle is
/// made here and never reused — a cancel is final, so the run after a cancelled
/// one has to start with a handle nobody has said stop to.
pub(crate) fn start_run(
    work: Work,
    before: App,
    manifest: &Manifest,
    repo_root: &Path,
    agent: &ClaudeAgent,
) -> Running {
    let cancel = CancelGuard::new();
    Running {
        events: spawn_pact(manifest, repo_root, &work, agent, cancel.handle()),
        cancel,
        work,
        before,
    }
}

/// The worker thread's whole body: carry the work out, saying where it has got
/// to, and report how it went.
///
/// Written as a function of its channel rather than inside the closure so that
/// it can be driven straight from a test — with a fake agent over a scratch
/// repository, on the test's own thread — and the sequence of events a real run
/// produces asserted without a terminal, a thread or a `claude`.
///
/// Exactly one [`PactEvent::Finished`] is sent, always, and it is the last thing
/// this function does. A failure is an outcome like any other: the footer's line
/// about a manifest that would not save travels down the same channel as the
/// news that everything worked. So is a cancel — a run the reader stopped still
/// hashes what it wrote, still saves, and still reports, which is what puts the
/// completed part of the subtree in `.warlock/pacts.toml` instead of losing it.
/// The one thing a cancel changes is how that outcome reads: see [`cancelled`].
fn run_pact(
    manifest: &Manifest,
    repo_root: &Path,
    work: &Work,
    agent: &dyn Agent,
    cancel: &Cancel,
    events: &Sender<PactEvent>,
) {
    let outcome = apply_toggle(
        manifest,
        repo_root,
        work,
        agent,
        &mut Reporting { events, cancel },
    );
    // Every run that spends minutes on model passes is stoppable, and both of
    // them are: a refresh is reworded here exactly as a pact is. The one run
    // that is not is an un-pact — see [`Work::is_cancellable`].
    let outcome = match outcome {
        Ok(toggled) if work.is_cancellable() && cancel.is_cancelled() => Ok(cancelled(toggled)),
        outcome => outcome,
    };
    // Ignored for the same reason `Reporting`'s sends are: a receiver that has
    // gone away is an application that is quitting.
    let _ = events.send(PactEvent::Finished(outcome));
}

/// `toggled` as the outcome of a run the reader stopped.
///
/// Two changes, and the manifest is not one of them: what the run recorded is
/// what it finished, and it is already on disk.
///
/// Nothing is granted, whatever the failures say. A cancel usually lands between
/// directories, where no pass has failed at all, and the subtree is still full of
/// directories that were never offered one — so the run's own "nothing went
/// wrong" would otherwise paint a subtree green that is mostly undocumented. The
/// manifest is the honest account: the parts that finished have grants and draw
/// green from the next load, the rest do not.
///
/// And the line is [`PACT_CANCELLED`] rather than the first thing that went
/// wrong, because after a cancel there is nothing here that went *wrong* in a
/// sense the reader can act on — the killed pass and the directories after it are
/// all the same fact, which is that they pressed Esc.
fn cancelled(toggled: Toggled) -> Toggled {
    Toggled {
        granted: false,
        message: Some(PACT_CANCELLED.to_owned()),
        ..toggled
    }
}

/// What one press of the pact key comes to, given whether a pact is running
/// already.
///
/// Two refusals, neither of them silent, and each said in its own place. A press
/// while a pact is in flight *starts* nothing — no toggle, no colour, no run,
/// and the panel keeps the account already on it — because the run in flight is
/// already writing to the tree the second one would write to; what it does do is
/// say so, by setting the flag that adds `— already running` to the end of the
/// line the reader is already watching (see
/// [`App::set_pact_refused`](warlock_tui::App::set_pact_refused) and
/// [`App::pact_line`](warlock_tui::App::pact_line)). Deliberately not a message:
/// the message line is the one a pact in flight has taken, so a sentence left
/// there would be the one sentence the reader could not see, and it would turn
/// up minutes later when the run ended. A press the app itself turns down — a
/// file row, or a directory the repository's `.warlockignore` keeps out — has no
/// run over it and so has its say the ordinary way, in
/// [`App::message`](warlock_tui::App::message), which `App::toggle_pact` has
/// already written by the time this returns. Every such refusal comes back from
/// `App::toggle_pact` as `None` and leaves through the `?` below, so nothing
/// here starts a run or opens an account for one: there is one place that
/// decides what a press means, and this is not it.
///
/// A function rather than a guard in the match arm so that "a second press
/// changes nothing" is a property a test can hold the app up against, rather
/// than something only an event loop with a terminal attached could show.
///
/// A press that really does start a run is also where the panel's account
/// begins, at `at`: one pact, one account, so a new run clears whatever the last
/// one left rather than appending to it. It happens here rather than in the
/// event loop so that "the account a press starts is empty" is a property of the
/// same function, and it happens on this path only — a press the app turned
/// down, and a press while a run is in flight, leave the last run's account on
/// screen because neither of them started anything.
///
/// An un-pact does not start one either, and that is the one case where the
/// keystroke does something and the panel does not move. Un-pacting is manifest
/// arithmetic that is over before the next frame: it runs no pass, reports no
/// activity and has nothing to account for, so wiping the record of the run that
/// wrote those documents would be spending the panel on a keystroke with nothing
/// to say.
pub(crate) fn pact_press(app: &mut App, in_flight: bool, at: Instant) -> Option<PactToggle> {
    if in_flight {
        // The whole of the refusal: a bit of wording on a line that is already
        // on screen. Setting it again says the same thing, so a reader leaning
        // on the key changes nothing after the first press.
        app.set_pact_refused();
        return None;
    }
    let toggle = app.toggle_pact()?;
    if toggle.pacted {
        app.start_account(at);
    }
    Some(toggle)
}

/// What one press of the refresh key comes to, given whether a run is going
/// already.
///
/// [`pact_press`] for the other key, and deliberately the same two refusals in
/// the same two places. A press while *any* run is in flight — a pact or
/// another refresh — starts nothing and says so on the progress line the reader
/// is already watching, leaving the account that run opened where it is; that
/// is the one in-flight check both keys go through, so a pact refused during a
/// refresh and a refresh refused during a pact read alike. A press the app
/// itself turns down — a file row, a fresh directory, an unpacted one — has no
/// run over it and has its say the ordinary way, in
/// [`App::message`](warlock_tui::App::message), which
/// [`App::refresh`](warlock_tui::App::refresh) has already written by the time
/// this returns.
///
/// What comes back is the directory to refresh: the run covers it and
/// everything under it, and which of the directories in there are stale enough
/// to be worth a pass is the engine's judgement rather than this file's.
///
/// Every press that starts a run starts the panel's account, at `at`, for
/// [`pact_press`]'s reason — one run, one account — and there is no second case
/// here as there is there: a refresh always runs passes when it runs at all, so
/// there is nothing that changes the manifest without having anything to
/// account for.
pub(crate) fn refresh_press(app: &mut App, in_flight: bool, at: Instant) -> Option<PathBuf> {
    if in_flight {
        // The whole of the refusal, and the very one a second pact press gets:
        // a bit of wording on a line that is already on screen.
        app.set_pact_refused();
        return None;
    }
    let directory = app.refresh()?;
    app.start_account(at);
    Some(directory)
}

/// Apply everything the worker has said since the last frame, and take the pact
/// down once it has said how it went.
///
/// Drained rather than received: the worker is not waiting for this thread, so
/// several directories can go by between two frames, and only the last of them
/// is worth drawing. The loop ends the moment there is nothing left to read,
/// which is the ordinary case — a pass is seconds, a frame is a tenth of one.
///
/// The outcome is applied exactly as the blocking version applied it, because it
/// is the same value computed by the same function: a granted subtree goes
/// green, a partial one keeps the yellow the keystroke painted and puts a line
/// on the footer, and a run that recorded nothing puts the app back as it was.
/// The progress line goes in every case — it describes work happening now, and
/// there is none.
///
/// A channel that closes without an outcome is a worker that panicked. It is
/// treated as the pact having ended, because it has: the panic hook has already
/// restored the terminal and printed what happened, this loop is still drawing,
/// and the alternative — waiting for a message from a thread that no longer
/// exists — would hang warlock on the one path where it can least afford to.
///
/// However the run ended, the panel's account is then closed off
/// ([`close_account`]): every section gets the ending belonging to its own
/// directory and the run gets its one summary line. Outside the arms rather than
/// inside them, because an account that is never closed is a finished run whose
/// newest line goes on counting up for as long as warlock is open.
///
/// The tree is then re-read from disk and the view put
/// back on top of it ([`reload_tree`]). One reload, at the bottom, for all four
/// endings and for an un-pact as much as for a pact: a run writes `WARLOCK.md`
/// files that no amount of recolouring rows in place can conjure into the tree,
/// and the only honest source for what is now on disk is disk. It runs *after*
/// each arm has done its own work — after the manifest is saved and after the
/// two restoring arms have put `running.before` back — so what the reload reads
/// lands on top of the arm's result rather than under it, and a reload that
/// fails leaves that result standing.
///
/// What comes back is the tree that reload read, when a run ended and the load
/// succeeded, and `None` otherwise: it is the whole of what the caller needs to
/// keep the watcher's filter on the tree that is now on screen, since this is
/// one of the two places the tree is ever re-read. See [`Watched::caught_up`].
///
/// `now` is the caller's clock, and this function reads none of its own: every
/// event drained here is filed under it, so a run is drivable from a base
/// instant in a test exactly as the account below it is. All the events of one
/// frame share it, which is the truth to a tenth of a second — the loop is the
/// only thing that hears the worker, so an event's arrival is when the loop got
/// round to it.
pub(crate) fn apply_progress(
    pact: &mut Option<Running>,
    app: &mut App,
    manifest: &mut Manifest,
    scope: &Scope,
    now: Instant,
) -> Option<Tree> {
    // No run, nothing drained, nothing reloaded — which is what almost every
    // frame of warlock's life does here.
    let running = pact.as_ref()?;

    let outcome = loop {
        match running.events.try_recv() {
            Ok(PactEvent::Starting {
                directory,
                position,
                total,
            }) => {
                // Two places, one fact, and they are two because they are read
                // at two different speeds: the footer says which directory of
                // how many is being worked *now* and replaces itself every time,
                // while the panel keeps a section per directory for as long as
                // the run lasts. The section is opened first so that the
                // activities drained after it — which may be in this same batch
                // — land under the directory they belong to.
                if let Some(account) = app.account_mut() {
                    account.open_section(section_label(&scope.root, &directory), now);
                }
                // The fraction is the observer's own, whichever run is
                // reporting: a refresh of a subtree of forty directories with
                // seven stale ones counts to seven, because seven is what the
                // engine planned to visit and said so. Nothing here counts
                // anything. The kind rides along so the line reads as
                // refreshing rather than pacting — see `Work::kind`.
                app.set_run_in_flight(running.work.kind(), directory, position, total);
            }
            // Filed under whichever directory is open, which is the one the
            // `Starting` before it named: an activity carries no directory
            // because it needs none, and the account's live section is the
            // answer. An account is always there during a run — the press that
            // started it made one — so `None` is a run nobody started this way,
            // which is a test driving the events directly; dropping the line is
            // the honest thing to do with it either way.
            //
            // What each activity comes to is the account's business and not this
            // file's: a tool is its name and its one detail, thinking is the
            // word `thinking`, and a cost is added to the section's spend rather
            // than drawn as a line of its own. See `Account::record`.
            Ok(PactEvent::Doing(activity)) => {
                if let Some(account) = app.account_mut() {
                    account.record(&activity, now);
                }
            }
            // The footer only, and on purpose: the panel's account reports what
            // went wrong and what was written, and a file that is being
            // summarised has done neither yet. The state it sets is cleared by
            // the next directory's `Starting` and by the end of the run, both
            // of which `App` does for itself, so no chunk wording is ever
            // attributed to a directory the run has moved past.
            Ok(PactEvent::Summarising { file, part, parts }) => {
                app.set_pact_summarising(file, part, parts);
            }
            // The one recolouring a run does before it is over. The engine
            // only says this of a directory whose whole subtree delivered —
            // the very condition its grant is decided on — so painting the
            // subtree green here repeats the engine's judgement rather than
            // second-guessing it, and a directory that finished third of five
            // is green while the fourth is still being paid for. The reload at
            // the end repaints from the manifest either way, which is what
            // catches the one thing this preview cannot know: a hash that
            // fails in phase two.
            Ok(PactEvent::Documented { directory }) => {
                app.set_subtree_state(&directory, NodeState::PactedFresh);
                // The document the pass just wrote, put on screen where it was
                // written: beside the directory, in the colour the paint above
                // has this moment given it. The paint comes first so the row is
                // born green rather than repainted into it, though the order is
                // not load-bearing — `set_subtree_state` paints a directory's
                // files along with the directory, so an insertion above it
                // would end up the same colour by the other road.
                //
                // And no reload, here or anywhere else mid-run. The manifest on
                // disk is still the pre-pact one until the single save at the
                // end of the run, so re-reading the tree now would re-derive
                // every row's state from that stale record and wipe the green
                // the run has been painting a directory at a time. That is why
                // the one reload stays at the bottom of this function, after the
                // outcome has landed and the manifest is written: by then disk
                // is the honest account, and this preview is the thing it
                // corrects rather than the thing it contradicts.
                app.insert_file_row(directory.join(DOCUMENT_FILE));
            }
            Ok(PactEvent::Finished(outcome)) => break Some(outcome),
            // Still running, and nothing new to say.
            Err(TryRecvError::Empty) => return None,
            Err(TryRecvError::Disconnected) => break None,
        }
    };

    let running = pact
        .take()
        .expect("the pact drained just above is still here");
    app.clear_pact_in_flight();
    // Read before the outcome is taken apart, because one arm below moves the
    // rest of the run out from under it: whether the reader stopped this run is
    // what decides how the panel's last section ends.
    let cancelled = running.cancel.is_cancelled();
    let refusals = match outcome {
        Some(Ok(Toggled {
            manifest: next,
            granted,
            message,
            refusals,
        })) => {
            *manifest = next;
            // The app painted the subtree stale when the key was pressed, and
            // the run's `Documented` announcements have been turning finished
            // branches green one at a time since. A pact that came back with
            // nothing wrong wrote, hashed and granted every directory in it,
            // so the one paint left is the whole subtree at once — the same
            // fact the announcements said piecemeal, said once at the end.
            //
            // A pact with a failure in it paints nothing here: the finished
            // branches are already green by the engine's own word, and the
            // failure's ancestors stay yellow — "pacted, not proven fresh" —
            // which is what the manifest is about to say of them. Colouring
            // anything else from this arm would be this file second-guessing
            // per node a manifest it did not compute.
            //
            // A refresh reads the same way, and it is the engine that makes
            // that true rather than anything here: a refresh keeps the grant of
            // every directory it skipped (WAR-39), so a run with no failure in
            // it leaves the whole subtree granted even though it described a
            // handful of directories. The paint is of the subtree the run
            // covered either way.
            if granted {
                app.set_subtree_state(running.work.path(), NodeState::PactedFresh);
            }
            if let Some(message) = message {
                app.set_message(message);
            }
            // Kept for the panel, which is the only place they can be said one
            // directory at a time.
            refusals
        }
        // Nothing was recorded: either the subtree could not be listed, or the
        // manifest would not save. Documents may well be on disk in the second
        // case, but the manifest is the record of what is pacted and it still
        // says what it said before, so the rows go back to matching it and the
        // reason goes on the app's line — the same one a refused toggle uses —
        // rather than out of the loop, which would take the screen with it.
        Some(Err(message)) => {
            restore(app, running.before, message);
            Vec::new()
        }
        // The worker died with the manifest in this thread's hand untouched, so
        // the rows go back to matching it exactly as they do for a run that
        // recorded nothing, and the footer says the run is over.
        None => {
            restore(app, running.before, PACT_LOST);
            Vec::new()
        }
    };

    // Every section gets its ending and the run gets its summary, whichever arm
    // above ran and whatever the run had to say for itself. After the arms
    // rather than inside them, because two of them put a different app on
    // screen and the account belongs to whichever one survives — and because a
    // run that ends without its clocks stopped is a finished run whose newest
    // line goes on counting up for as long as warlock is open.
    close_account(app, scope, &refusals, cancelled, now);

    // The run is over and everything it recorded is on disk, so the rows on
    // screen are one load out of date whichever arm above ran.
    reload_tree(app, scope)
}

/// The file each pacted directory's document is written to, as the engine writes
/// it (WAR-21.01): beside the directory, and renamed over.
///
/// Spelled out here because the engine's observer port says only which directory
/// is starting — nothing on the way back names the document — so the one thing
/// the panel can do is look for it where it is always written. Reading it is a
/// `stat` on the event loop's own thread, which is a filesystem call this file
/// already makes several of.
const DOCUMENT_FILE: &str = "WARLOCK.md";

/// Close every section of the panel's account and end it with the run's summary.
///
/// Where the panel stops describing work and starts describing what came of it,
/// and it happens here, once, when the run's outcome lands — because that is
/// when the run says how each directory went. The engine's failures name their
/// own directories, so each section is closed with the reason belonging to *its*
/// directory rather than with the one line the footer took.
///
/// A section no failure mentions wrote a document, and the two things worth
/// saying about that document — what it is and how big it is — are read off disk
/// here, on this thread, at `<directory>/WARLOCK.md`. There is nowhere else to
/// read them from: the port the run reports over says only which directory is
/// starting.
///
/// `cancelled` closes the section the run was stopped in before anything else is
/// worded, so it says the reader stopped it rather than that its pass failed —
/// which is what a killed `claude` otherwise comes back as. The sections above
/// it are worded exactly as they would have been, because they finished.
///
/// Does nothing when there is no account, which is a run nobody started through
/// the pact key: a test driving the events straight down the channel.
fn close_account(
    app: &mut App,
    scope: &Scope,
    refusals: &[Refusal],
    cancelled: bool,
    now: Instant,
) {
    let Some(account) = app.account_mut() else {
        return;
    };
    if cancelled {
        account.close_section(&Outcome::Cancelled, now);
    }
    account.close_open_sections(now, |section| {
        section_outcome(section, refusals, &scope.root)
    });
    account.finish(now);
}

/// How the section for one directory ends: the document it wrote, or the reason
/// it has none.
///
/// The refusal is looked for by name, and the name it is looked for under is the
/// one [`section_label`] gave the heading — the same function on both sides, so a
/// failure and a section agree about which directory they are about by
/// construction rather than by two spellings happening to match.
///
/// Failing that, the document is `stat`ed where the engine writes it. It is named
/// relative to the tree on screen, like the heading above it, because a panel
/// column is narrow and the absolute prefix is the part the reader already knows.
///
/// A directory with neither a failure nor a document is the odd case, and it is
/// said rather than smoothed over: a run whose worker died half way through a
/// pass leaves exactly this, and `wrote … — 0 bytes` would be a claim about a
/// file that is not there.
fn section_outcome(section: &Section, refusals: &[Refusal], root: &Path) -> Outcome {
    let refused = refusals
        .iter()
        .find(|refusal| Path::new(&section_label(root, &refusal.directory)) == section.directory());
    if let Some(refusal) = refused {
        return Outcome::Refused {
            reason: refusal.reason.clone(),
        };
    }

    // The heading's own spelling with the document's name under it, and the
    // absolute path to go and look at it: the label is relative to the root, so
    // joining it back on is what the run walked.
    let document = section.directory().join(DOCUMENT_FILE);
    match fs::metadata(root.join(&document)) {
        Ok(written) => Outcome::Wrote {
            document,
            bytes: written.len(),
        },
        Err(error) if error.kind() == io::ErrorKind::NotFound => Outcome::Refused {
            reason: "no document was written".to_owned(),
        },
        Err(error) => Outcome::Refused {
            reason: format!("its document could not be read: {error}"),
        },
    }
}

/// Put the view back to `before` and say `message`, keeping the account of the
/// run that is ending.
///
/// The copy taken when the key was pressed is the tree as it stood before the
/// toggle painted it, and that is all it is good for: it was taken before the
/// run started, so it has none of what the run then did. The account does not go
/// back with the rows, because it is not a claim about the tree — it is the
/// record of a run that really happened, and a reader whose pact died half way
/// through wants to see where it got to more than a reader of any other run
/// does. So the rows, the colours and the selection go back to what the manifest
/// on disk still says, and the panel keeps its account.
fn restore(app: &mut App, before: App, message: impl Into<String>) {
    app.restore_from(before);
    app.set_message(message);
}

/// How the panel names `directory`: relative to `root`, which is the root of the
/// tree on screen.
///
/// The same spelling the footer's progress line uses, for the same reason. A
/// heading that begins with the part of the path every row on screen shares
/// spends a narrow panel on what the reader already knows, and the truncation
/// that follows then eats the part they do not — the panel cuts a long line at
/// its right-hand end. A directory that cannot be spelled relative to the root —
/// including the root itself, whose relative spelling is `"."` — is named as it
/// stands, because a heading that says something odd beats one that says
/// nothing.
fn section_label(root: &Path, directory: &Path) -> String {
    match to_manifest_path(root, directory) {
        Ok(relative) if relative != "." => relative,
        _ => directory.display().to_string(),
    }
}

/// What one press of the pact key came to, once the manifest it produced is on
/// disk.
#[derive(Debug)]
pub(crate) struct Toggled {
    /// The manifest as it now stands in `.warlock/pacts.toml`, to keep as the
    /// one the *next* keystroke edits.
    manifest: Manifest,
    /// Whether every directory in the subtree came out documented, hashed and
    /// granted — the only case in which the subtree on screen is fresh rather
    /// than merely pacted. Always `false` for an un-pact, which grants nothing.
    granted: bool,
    /// One line about what went wrong on the way, or `None` when nothing did.
    /// Never a reason to throw the manifest away: everything it can say is
    /// about part of a subtree, and the rest of it earned what it got.
    message: Option<String>,
    /// The directories that came back with no document, and why, one entry per
    /// directory.
    ///
    /// The same failures `message` says one of, kept apart from it because they
    /// are read at two different widths: the footer is one line about the run
    /// and the panel is a section per directory, and a section can only say why
    /// *its* pass was refused if the reason arrives attached to a directory.
    /// A run where nothing was refused carries none.
    refusals: Vec<Refusal>,
}

/// Why one directory's pass produced no document.
///
/// A [`PactFailure`] flattened to the two things the panel needs: which
/// directory it is about, which the engine's failures already name, and the
/// sentence the failure worded itself as. Flattened at the point the failures
/// are in hand rather than carried whole, so that what travels back to the event
/// loop is a plain pair of a path and a line — and so that the panel's wording
/// is the engine's own rather than a second opinion about what went wrong.
#[derive(Debug)]
struct Refusal {
    /// The directory that got no document, as the engine named it: absolute,
    /// and spelled relative to the tree on screen by whoever draws it.
    directory: PathBuf,
    /// The failure's own sentence, on one line.
    reason: String,
}

impl From<&PactFailure> for Refusal {
    fn from(failure: &PactFailure) -> Self {
        Self {
            directory: failure.directory().to_path_buf(),
            // Flattened for the same reason every other line here is: a failure
            // can carry a manifest error, which carries the TOML parser's
            // several lines, and a panel row is one row.
            reason: one_line(&failure.to_string()),
        }
    }
}

/// Carry `work` out — pact the subtree, refresh its stale parts, or take it out
/// of the manifest — and write the result to disk.
///
/// Every half is the engine's ([`pact_subtree`], [`refresh_subtree`],
/// [`unpact_subtree`]); what this function owns is the order the front end needs
/// them in and the single [`Manifest::save`] at the end of it. Once, at the end,
/// is the whole point: a save per directory would leave `.warlock/pacts.toml`
/// recording a pact that was still running, and one that died half way through
/// would be indexed as finished. A refresh saves nothing of its own for exactly
/// that reason, and so belongs under the same one write as everything else here.
///
/// This is the one place the two runs part company, and they part in a single
/// `match` arm: [`refresh_subtree`] takes [`pact_subtree`]'s arguments and hands
/// back [`pact_subtree`]'s [`PactedSubtree`], so what the front end makes of it
/// is the same in both cases — see [`described`].
///
/// The agent is passed in as the engine's port rather than reached for here, so
/// that the tests of this file drive it with a fake and never run `claude`.
/// `observer` is the engine's other port and is passed in for the same reason:
/// this runs on a worker thread and says where it has got to over a channel
/// ([`Reporting`]), and a test says it over a `Vec`.
///
/// Called from the worker thread and from nowhere else, which is why it can take
/// as long as it takes. Nothing about it is thread-aware: it is the same
/// sequence of engine calls it always was, and the thread is
/// [`spawn_pact`]'s idea.
///
/// # Errors
///
/// A line for the footer, not an error type: the only two things that stop this
/// getting as far as a saved manifest are a subtree that cannot be walked and a
/// manifest that cannot be written, and the single thing the caller does with
/// either is show it. Anything richer would be a vocabulary invented for one
/// `match` arm that puts a string on the screen. Both cases leave the previous
/// `.warlock/pacts.toml` exactly as it was.
fn apply_toggle(
    manifest: &Manifest,
    repo_root: &Path,
    work: &Work,
    agent: &dyn Agent,
    observer: &mut dyn PactObserver,
) -> Result<Toggled, String> {
    let toggled = match work {
        // Un-pacting is pure manifest editing — no walk, no pass, no hash, and
        // every `WARLOCK.md` left where it is — so the only thing it can refuse
        // is a path the manifest has no spelling for. The app has already said
        // what un-pacting leaves behind, and nothing here talks over it.
        Work::Pact(toggle) if !toggle.pacted => Toggled {
            manifest: unpact_subtree(&toggle.path, repo_root, manifest)
                .map_err(|source| Error::Manifest { source }.to_string())?,
            granted: false,
            message: None,
            refusals: Vec::new(),
        },
        // Every directory in the subtree, whatever state it was in.
        Work::Pact(toggle) => described(
            pact_subtree(&toggle.path, repo_root, manifest, agent, observer)
                .map_err(|source| one_line(&source.to_string()))?,
        ),
        // Only the stale ones, and which those are is the engine's judgement:
        // it decides staleness from the same manifest handed in here, keeps the
        // grant of everything it skipped, and — like the pact above — saves
        // nothing, leaving the one write below.
        Work::Refresh(directory) => described(
            refresh_subtree(directory, repo_root, manifest, agent, observer)
                .map_err(|source| one_line(&source.to_string()))?,
        ),
    };

    toggled
        .manifest
        .save(repo_root)
        .map_err(|source| Error::Manifest { source }.to_string())?;
    Ok(toggled)
}

/// What a run that described directories came to, whichever run described them.
///
/// The half a pact and a refresh share, which is all of it below the engine
/// call: the two entry points hand back the same [`PactedSubtree`], and reading
/// one is the same job twice over. The manifest inside is the one the caller is
/// about to save.
fn described(subtree: PactedSubtree) -> Toggled {
    let PactedSubtree {
        manifest,
        failures,
        problems,
    } = subtree;
    // Failures alone decide freshness, and the byte caps' problems do not:
    // a request that left a lockfile out still produced a document, a hash
    // and a grant. They are still worth a line, which is why the two travel
    // separately from here on.
    let granted = failures.is_empty();
    // And the same failures a second time, per directory: the footer takes
    // one of them and the panel takes all of them, because the panel has a
    // section for each and can say which is which.
    let refusals = failures.iter().map(Refusal::from).collect();
    Toggled {
        manifest,
        granted,
        message: pact_message(&failures, &problems),
        refusals,
    }
}

/// The footer's one line about a pact that did not go perfectly, or `None` for
/// one that did.
///
/// A pact is N directories and each of them can go wrong on its own, so this is
/// the same shape as [`Error::from_problems`] and for the same reason: one
/// failure quoted in full, because it is the one worth acting on, and a count
/// of everything else, because a line per directory would push the useful one
/// off a footer that is one line tall. Failures come first — a directory with
/// no document is worse news than a file left out of a request that worked —
/// and the count covers both piles, which is why it does not claim the rest are
/// "like it".
fn pact_message(failures: &[PactFailure], problems: &[PactProblem]) -> Option<String> {
    let (first, rest) = match (failures.split_first(), problems.split_first()) {
        (Some((first, others)), _) => (first.to_string(), others.len() + problems.len()),
        (None, Some((first, others))) => (first.to_string(), others.len()),
        (None, None) => return None,
    };

    let first = one_line(&first);
    Some(match rest {
        0 => first,
        rest => format!("{first} (and {rest} more)"),
    })
}

/// What one press of the pact key actually does: the manifest that ends up
/// on disk, how many times it is written, what the footer is told, whether
/// the subtree comes out fresh, and what the worker thread says on its way
/// through all of that.
///
/// Every test here drives the real engine operations over a repository of
/// its own under the temporary directory, with a hand-written fake in place
/// of the model. No `claude`, no network, no terminal, no mocking
/// framework — the agent seam is what makes that possible, and this is what
/// it was for. The worker's body is driven directly, on the test's own
/// thread, so what is asserted is the sequence of events a real run sends
/// rather than the timing of one.
#[cfg(test)]
mod tests {
    /// A root no test touches on disk: every path below is made relative to it
    /// by string surgery, so the tests using it need no repository, no
    /// temporary directory and no filesystem at all.
    const ROOT: &str = "/repo";

    use std::cell::RefCell;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::mpsc::{self, Sender};
    use std::time::{Duration, Instant};
    use std::{env, fs, process};

    use warlock_engine::{
        Agent, AgentError, AgentRequest, AgentResponse, Loaded, Manifest, Node, NodeState,
        PER_FILE_BYTE_CAP, PactEntry, Tree, Unwatched, decide_state, load_tree, repository_root,
        subtree_hash,
    };
    use warlock_tui::{
        Account, Activities, Activity, App, Chrome, ClaudeAgent, Line, PactToggle, Run, Section,
    };

    use warlock_tui::Cancel;

    use super::{
        CancelGuard, PACT_CANCELLED, PACT_LOST, PactEvent, Running, Toggled, Work, activity_port,
        apply_progress, apply_toggle, pact_press, refresh_press, run_pact, spawn_pact,
    };
    use crate::session::{NOT_REFRESHED, Scope};

    /// The file every pacted directory is documented in, as the engine
    /// writes it. Spelled out here so a test can go and look for it.
    const DOCUMENT_FILE: &str = "WARLOCK.md";

    /// A model pass that never happens: it answers with the same markdown
    /// every time, turns down whatever it was told to turn down, and notes
    /// what the manifest looked like when each request arrived.
    struct Canned {
        /// The repository root, so a request can be recorded by its
        /// directory's relative path and the manifest can be looked for.
        root: PathBuf,
        /// Directories, relative to the root, whose pass comes back with an
        /// answer too short for the engine to accept.
        refused: Vec<&'static str>,
        /// The directory whose pass the reader presses Esc during, and the
        /// handle they press it with, or `None` for a run nobody stops.
        cancel_at: Option<(&'static str, Cancel)>,
        /// Where each pass says what it is doing. A handle nobody listens
        /// to unless a test attached one, exactly as a real
        /// [`ClaudeAgent`]'s is.
        activities: Activities,
        /// One entry per request in call order: which directory it was for,
        /// and whether `.warlock/pacts.toml` existed at that moment.
        seen: RefCell<Vec<(PathBuf, bool)>>,
    }

    impl Canned {
        /// A fake over `scratch` that refuses the directories in `refused`
        /// and answers everything else.
        fn new(scratch: &Scratch, refused: impl IntoIterator<Item = &'static str>) -> Self {
            Self {
                root: scratch.root.clone(),
                refused: refused.into_iter().collect(),
                cancel_at: None,
                activities: Activities::none(),
                seen: RefCell::new(Vec::new()),
            }
        }

        /// The same fake, saying what each pass is doing to `activities`.
        ///
        /// What it says is canned, like everything else here, and it is the
        /// three kinds of thing a real pass reports: a tool call with a
        /// detail, a stretch of thinking, and what the pass cost. The detail
        /// is the directory being worked, so a test can tell one pass's
        /// activities from the next one's.
        fn reporting(mut self, activities: Activities) -> Self {
            self.activities = activities;
            self
        }

        /// The same fake, with somebody pressing Esc while `directory` is
        /// being worked.
        ///
        /// The pass still answers, which is the ordinary way a cancel lands:
        /// the reader's key beats the engine to the *next* directory rather
        /// than to this one's answer. The pass that is killed under a real
        /// cancel comes back as a failure instead, and a failure is what
        /// `refused` already produces — see the test that uses both.
        fn cancelling_at(mut self, directory: &'static str, cancel: Cancel) -> Self {
            self.cancel_at = Some((directory, cancel));
            self
        }

        /// The directories a pass ran for, in call order.
        fn directories(&self) -> Vec<PathBuf> {
            self.seen
                .borrow()
                .iter()
                .map(|(directory, _)| directory.clone())
                .collect()
        }

        /// Whether a manifest was on disk while the passes were running.
        fn saw_a_manifest(&self) -> bool {
            self.seen.borrow().iter().any(|(_, saved)| *saved)
        }
    }

    impl Agent for Canned {
        fn run(&self, request: &AgentRequest) -> Result<AgentResponse, AgentError> {
            let directory = request.directory();
            let relative = directory
                .strip_prefix(&self.root)
                .unwrap_or(directory)
                .to_path_buf();
            self.seen
                .borrow_mut()
                .push((relative.clone(), saved(&self.root).is_some()));

            // Reported before anything is answered, because that is when a
            // real pass reports: while it is still running.
            self.activities.report(Activity::Tool {
                name: "Read".to_owned(),
                detail: Some(relative.display().to_string()),
            });
            self.activities.report(Activity::Thinking);
            self.activities.report(Activity::Cost { usd: 0.25 });

            if let Some((at, cancel)) = &self.cancel_at
                && Path::new(at) == relative
            {
                cancel.cancel();
            }
            if self.refused.iter().any(|name| Path::new(name) == relative) {
                // Short enough that the engine turns it down: the cheapest
                // way to fail one directory of a pact for real, rather than
                // by reaching into the engine's error types, which are
                // `#[non_exhaustive]` and cannot be built from here.
                return Ok(AgentResponse::new("no."));
            }
            Ok(AgentResponse::new(document()))
        }
    }

    /// A document long enough for the engine to accept. The rule is a byte
    /// count and nothing here reads what it says, so this is filler.
    fn document() -> String {
        format!("# module\n\n{}\n", "What it does, at length. ".repeat(20))
    }

    /// The manifest as it sits on disk under `root`, or `None` when there
    /// is none.
    fn saved(root: &Path) -> Option<Manifest> {
        Manifest::load(root).ok()
    }

    /// A repository of this test's own under the temporary directory,
    /// removed when the test that made it ends.
    ///
    /// Hand-rolled the way `claude.rs`'s tests do it: this crate's manifest
    /// gains nothing for a `mkdir` and an `rm -r`.
    struct Scratch {
        /// The root every path below is built from, and the root the
        /// manifest is saved under.
        root: PathBuf,
    }

    impl Scratch {
        /// An empty repository, named after the test using it so a leftover
        /// says where it came from.
        fn new(name: &str) -> Self {
            static NEXT: AtomicUsize = AtomicUsize::new(0);

            let unique = NEXT.fetch_add(1, Ordering::Relaxed);
            let root =
                env::temp_dir().join(format!("warlock-pact-{}-{name}-{unique}", process::id()));
            fs::create_dir_all(&root).expect("a scratch repository under the temp directory");
            Self { root }
        }

        /// Write `contents` at `relative`, creating every directory above
        /// it.
        fn write(&self, relative: &str, contents: &str) {
            let path = self.root.join(relative);
            fs::create_dir_all(path.parent().expect("a file has a parent"))
                .expect("creates the directories above a file");
            fs::write(&path, contents).expect("writes a file");
        }

        /// The path at `relative`, as the app would name it.
        fn path(&self, relative: &str) -> PathBuf {
            self.root.join(relative)
        }
    }

    impl Drop for Scratch {
        /// Best effort: a leftover under the temporary directory is untidy,
        /// not a test failure.
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    /// The work a press of the pact key over the directory at `relative` hands
    /// to a worker.
    fn toggle(scratch: &Scratch, relative: &str, pacted: bool) -> Work {
        Work::Pact(PactToggle {
            path: scratch.path(relative),
            pacted,
        })
    }

    /// The work a press of the refresh key over the directory at `relative`
    /// hands to a worker: the same run, over whichever directories under it the
    /// engine finds stale.
    fn refreshing(scratch: &Scratch, relative: &str) -> Work {
        Work::Refresh(scratch.path(relative))
    }

    /// The same for a pact of the subtree at `path`, for the tests that build a
    /// [`Running`] straight rather than through a scratch repository.
    fn pact_of(path: impl Into<PathBuf>) -> Work {
        Work::Pact(PactToggle {
            path: path.into(),
            pacted: true,
        })
    }

    /// A repository with one crate of two directories in it.
    fn one_crate(name: &str) -> Scratch {
        let scratch = Scratch::new(name);
        scratch.write("crates/engine/src/lib.rs", "//! Core engine.\n");
        scratch
    }

    /// The same repository, with the `.git/` that makes the loader agree it
    /// is one.
    ///
    /// [`one_crate`] is enough for the engine operations, which are given a
    /// root; a *load* walks up looking for `.git/` and refuses without one.
    /// The file inside it is neither read nor walked — hidden directories
    /// are skipped, `.git/` among them — and is written only because
    /// [`Scratch`] makes directories by writing files into them.
    fn one_crate_to_load(name: &str) -> Scratch {
        let scratch = one_crate(name);
        scratch.write(".git/HEAD", "ref: refs/heads/main\n");
        scratch
    }

    /// The app and the [`Scope`] the event loop would hold for `scratch`,
    /// built the way `load_app` builds them.
    fn load(scratch: &Scratch) -> (App, Scope) {
        let Loaded { tree, .. } =
            load_tree(&scratch.root).expect("a scratch repository with a `.git/` loads");
        let repo_root =
            repository_root(tree.root_path()).expect("the load found a repository root");
        let app = App::from_tree(&tree);
        let scope = Scope {
            chrome: Chrome::of(&repo_root, tree.root_path()),
            root: tree.root_path().to_path_buf(),
            repo_root,
        };
        (app, scope)
    }

    /// `base` plus `seconds`, so a run's whole timeline is one instant and
    /// some arithmetic rather than a sleep.
    fn at(base: Instant, seconds: u64) -> Instant {
        base + Duration::from_secs(seconds)
    }

    /// What the panel would draw for `app` at `now`, as plain strings: a
    /// heading is its path, a clocked line is its clock and its text, and
    /// the summary is its own line.
    ///
    /// The whole account rather than the window onto it, because what these
    /// tests are about is what the run put into the panel and not how much
    /// of it fits.
    fn panel_text(app: &App, now: Instant) -> Vec<String> {
        app.account()
            .map(|account| account.lines(now))
            .unwrap_or_default()
            .iter()
            .map(|line| match line {
                Line::Directory { path } => path.display().to_string(),
                Line::Clocked { clock, text } => format!("{clock} {text}"),
                Line::Summary { text } => text.clone(),
            })
            .collect()
    }

    /// A scope for a tree that is not on disk anywhere.
    ///
    /// What the tests built around synthetic paths hand to
    /// [`apply_progress`]: a load from here fails, which is the case where
    /// the tree already on screen is kept.
    fn nowhere() -> Scope {
        Scope {
            root: PathBuf::from("/repo/crates"),
            repo_root: PathBuf::from("/repo"),
            chrome: Chrome::of("/repo", "/repo/crates"),
        }
    }

    /// The state the app is showing for the node at `path`, or `None` when
    /// no row stands for it.
    fn state_of(app: &App, path: &Path) -> Option<NodeState> {
        app.rows()
            .iter()
            .find(|row| row.path == path)
            .map(|row| row.state)
    }

    /// Every document row on screen, as paths relative to `scratch`.
    fn documents(app: &App, scratch: &Scratch) -> Vec<PathBuf> {
        app.rows()
            .iter()
            .filter(|row| {
                row.path
                    .file_name()
                    .is_some_and(|name| name == DOCUMENT_FILE)
            })
            .map(|row| {
                row.path
                    .strip_prefix(&scratch.root)
                    .unwrap_or(&row.path)
                    .to_path_buf()
            })
            .collect()
    }

    /// Run the worker's body for `toggle` over `scratch` on this thread, and
    /// let the event loop take everything it said off the channel.
    ///
    /// The two halves of a real run, joined by the real channel and with
    /// nothing faked but the model: [`run_pact`] is what the worker thread
    /// runs, and [`apply_progress`] is what the frame after it does.
    fn run_and_apply(
        scratch: &Scratch,
        app: &mut App,
        manifest: &mut Manifest,
        scope: &Scope,
        work: &Work,
        agent: &dyn Agent,
    ) {
        let before = app.clone();
        let (events, received) = mpsc::channel();
        run_pact(
            manifest,
            &scratch.root,
            work,
            agent,
            &Cancel::new(),
            &events,
        );

        let mut pact = Some(Running {
            events: received,
            cancel: CancelGuard::new(),
            work: work.clone(),
            before,
        });
        apply_progress(&mut pact, app, manifest, scope, Instant::now());
        assert!(pact.is_none(), "the run reported its outcome and is over");
    }

    #[test]
    fn a_pact_documents_every_directory_in_the_subtree_and_grants_it() {
        let scratch = one_crate("grants");
        let agent = Canned::new(&scratch, []);

        let Toggled {
            manifest,
            granted,
            message,
            ..
        } = apply_toggle(
            &Manifest::new(),
            &scratch.root,
            &toggle(&scratch, "crates/engine", true),
            &agent,
            &mut Unwatched,
        )
        .expect("a subtree that walks and a manifest that writes");

        assert!(granted, "nothing went wrong, so the subtree is fresh");
        assert_eq!(message, None, "and there is nothing to report");

        let mut modules: Vec<&str> = manifest.entries().iter().map(PactEntry::module).collect();
        modules.sort_unstable();
        assert_eq!(modules, ["crates/engine", "crates/engine/src"]);

        for entry in manifest.entries() {
            let directory = entry.module_path(&scratch.root);
            let module = entry.module();
            assert_eq!(entry.document(), format!("{module}/{DOCUMENT_FILE}"));
            assert!(
                directory.join(DOCUMENT_FILE).is_file(),
                "{module} has no document"
            );
            // The point of the two phases: every document was written
            // before any hash was taken, so a parent is as fresh as its
            // children.
            let hash = subtree_hash(&directory).expect("a directory just written hashes");
            assert_eq!(
                decide_state(Some(entry), &hash),
                NodeState::PactedFresh,
                "{module} is not fresh"
            );
        }
    }

    #[test]
    fn the_manifest_is_written_once_and_only_when_the_pact_is_over() {
        let scratch = one_crate("once");
        let agent = Canned::new(&scratch, []);

        let Toggled { manifest, .. } = apply_toggle(
            &Manifest::new(),
            &scratch.root,
            &toggle(&scratch, "crates/engine", true),
            &agent,
            &mut Unwatched,
        )
        .expect("a subtree that walks and a manifest that writes");

        // Two passes ran, and neither of them found a manifest: a save per
        // directory would have left one on disk for the second to see.
        assert_eq!(
            agent.directories(),
            [
                PathBuf::from("crates/engine/src"),
                PathBuf::from("crates/engine")
            ],
            "children before parents",
        );
        assert!(
            !agent.saw_a_manifest(),
            "the manifest was written while the pact was still running"
        );
        // And it is there afterwards, saying exactly what came back.
        assert_eq!(
            saved(&scratch.root).expect("the manifest was written"),
            manifest
        );
    }

    #[test]
    fn entries_outside_the_pacted_subtree_are_kept_exactly_as_they_were() {
        let scratch = one_crate("outside");
        scratch.write("crates/tui/src/main.rs", "fn main() {}\n");
        let outside = PactEntry::new(
            &scratch.root,
            scratch.path("crates/tui"),
            scratch.path("crates/tui").join(DOCUMENT_FILE),
        )
        .expect("a path under the root can be stored")
        .with_grant("earned-earlier", "2026-01-01T00:00:00Z");
        let agent = Canned::new(&scratch, []);

        let Toggled { manifest, .. } = apply_toggle(
            &Manifest::with_entries([outside.clone()]),
            &scratch.root,
            &toggle(&scratch, "crates/engine", true),
            &agent,
            &mut Unwatched,
        )
        .expect("a subtree that walks and a manifest that writes");

        // Same entry, same grant: a pact of one subtree is no judgement of
        // any other.
        assert_eq!(manifest.entry("crates/tui"), Some(&outside));
        assert!(
            !scratch.path("crates/tui").join(DOCUMENT_FILE).exists(),
            "a directory outside the pact was written to"
        );
        assert_eq!(agent.directories().len(), 2, "and no pass ran for it");
    }

    #[test]
    fn a_directory_the_pass_refuses_is_summarised_on_the_footers_one_line() {
        let scratch = one_crate("refused");
        let agent = Canned::new(&scratch, ["crates/engine/src"]);

        let Toggled {
            manifest,
            granted,
            message,
            ..
        } = apply_toggle(
            &Manifest::new(),
            &scratch.root,
            &toggle(&scratch, "crates/engine", true),
            &agent,
            &mut Unwatched,
        )
        .expect("half a pact is still a manifest worth writing");

        assert!(!granted, "a subtree with a failure in it is not fresh");
        let message = message.expect("the failure is reported");
        assert!(!message.contains('\n'), "the footer is one line: {message}");
        assert!(
            message.contains("crates/engine/src"),
            "the failing directory is named: {message}"
        );
        assert!(
            !message.contains("(and"),
            "one failure has nothing to count: {message}"
        );

        // No document, no entry; the ancestor inside the pact is recorded
        // with nothing granted, which is what draws it yellow.
        assert_eq!(manifest.entry("crates/engine/src"), None);
        let entry = manifest
            .entry("crates/engine")
            .expect("the ancestor is still pacted");
        assert_eq!(entry.granted_hash(), None);
        assert_eq!(
            saved(&scratch.root).expect("the manifest was written"),
            manifest
        );
    }

    #[test]
    fn several_things_going_wrong_are_one_line_with_the_rest_counted() {
        let scratch = one_crate("counted");
        let agent = Canned::new(&scratch, ["crates/engine/src", "crates/engine"]);

        let Toggled {
            manifest, message, ..
        } = apply_toggle(
            &Manifest::new(),
            &scratch.root,
            &toggle(&scratch, "crates/engine", true),
            &agent,
            &mut Unwatched,
        )
        .expect("a pact that documented nothing still saves");

        let message = message.expect("the failures are reported");
        assert!(!message.contains('\n'), "the footer is one line: {message}");
        // The first failure in full — children first, so it is the deeper
        // directory — and the other counted rather than quoted.
        assert!(
            message.contains("crates/engine/src"),
            "the first failure is quoted: {message}"
        );
        assert!(message.ends_with("(and 1 more)"), "{message}");
        assert!(
            manifest.entries().is_empty(),
            "nothing was documented, so nothing is recorded: {manifest:?}"
        );
    }

    #[test]
    fn un_pacting_saves_a_manifest_without_the_subtree_and_keeps_the_documents() {
        let scratch = one_crate("unpact");
        let agent = Canned::new(&scratch, []);
        let pacted = apply_toggle(
            &Manifest::new(),
            &scratch.root,
            &toggle(&scratch, "crates/engine", true),
            &agent,
            &mut Unwatched,
        )
        .expect("a subtree that walks and a manifest that writes")
        .manifest;

        let Toggled {
            manifest,
            granted,
            message,
            ..
        } = apply_toggle(
            &pacted,
            &scratch.root,
            &toggle(&scratch, "crates/engine", false),
            &agent,
            &mut Unwatched,
        )
        .expect("dropping entries needs nothing but the manifest");

        assert!(!granted, "un-pacting grants nothing");
        assert_eq!(
            message, None,
            "the app has already said what un-pacting leaves behind"
        );
        assert!(manifest.entries().is_empty());
        assert_eq!(
            saved(&scratch.root).expect("the manifest was written"),
            manifest
        );
        // The writing survives the claim being taken back.
        for module in ["crates/engine", "crates/engine/src"] {
            assert!(
                scratch.path(module).join(DOCUMENT_FILE).is_file(),
                "{module}'s document was deleted"
            );
        }
        assert_eq!(
            agent.directories().len(),
            2,
            "un-pacting runs no model passes"
        );
    }

    #[test]
    fn a_directory_outside_the_repository_root_is_refused_rather_than_stored() {
        // No filesystem: un-pacting is path arithmetic, and this path has
        // no manifest spelling to do it with.
        let outside = Work::Pact(PactToggle {
            path: PathBuf::from("/elsewhere/crates/engine"),
            pacted: false,
        });
        let scratch = Scratch::new("elsewhere");

        let message = apply_toggle(
            &Manifest::new(),
            Path::new(ROOT),
            &outside,
            &Canned::new(&scratch, []),
            &mut Unwatched,
        )
        .expect_err("a path outside the root has no manifest spelling");

        assert!(!message.contains('\n'), "the footer is one line: {message}");
        assert!(
            message.contains("/elsewhere/crates/engine"),
            "the refused path is named: {message}"
        );
        assert!(
            saved(Path::new(ROOT)).is_none(),
            "nothing was written anywhere"
        );
    }

    /// The one failure that needs a directory nobody can write to, and
    /// making one is Unix-only: `chmod` has no portable stand-in, and the
    /// alternative — a second binary, or a dependency — costs more than the
    /// coverage.
    #[cfg(unix)]
    #[test]
    fn a_manifest_that_cannot_be_saved_leaves_the_previous_one_alone() {
        use std::os::unix::fs::PermissionsExt as _;

        let scratch = one_crate("readonly");
        let agent = Canned::new(&scratch, []);
        // Something for the failed save to spare: a manifest from an
        // earlier run, and the bytes it is expected to still hold.
        let previous = Manifest::with_entries([PactEntry::new(
            &scratch.root,
            scratch.path("crates/tui"),
            scratch.path("crates/tui").join(DOCUMENT_FILE),
        )
        .expect("a path under the root can be stored")]);
        previous.save(&scratch.root).expect("the first save works");
        let manifest_dir = scratch.path(".warlock");
        let before = fs::read(manifest_dir.join("pacts.toml")).expect("reads what was saved");
        fs::set_permissions(&manifest_dir, fs::Permissions::from_mode(0o555))
            .expect("chmods the manifest directory read-only");

        let message = apply_toggle(
            &previous,
            &scratch.root,
            &toggle(&scratch, "crates/engine", true),
            &agent,
            &mut Unwatched,
        )
        .expect_err("a manifest directory nobody can write to");

        // Back to writable before anything can fail, so the scratch
        // repository can still be removed.
        fs::set_permissions(&manifest_dir, fs::Permissions::from_mode(0o755))
            .expect("chmods it back");

        assert!(!message.contains('\n'), "the footer is one line: {message}");
        assert!(
            message.starts_with("could not read or write "),
            "the engine's own wording: {message}"
        );
        assert_eq!(
            fs::read(manifest_dir.join("pacts.toml")).expect("reads it again"),
            before,
            "the previous manifest was not touched"
        );
    }

    /// Everything the worker had to say about a run of `toggle` over
    /// `scratch` with `agent`, answering to `cancel`, in the order the event
    /// loop would have drained it.
    ///
    /// The worker's body runs here, on the test's own thread, and its end of
    /// the channel is dropped before anything is read: what comes back is
    /// the whole sequence a real run sends, with none of the timing of one.
    /// The handle stands in for the one the event loop keeps: a fresh one
    /// nobody touches is a run nobody stops, and a fake that latches it
    /// half way through is somebody pressing Esc.
    fn events_of(
        scratch: &Scratch,
        work: &Work,
        agent: &dyn Agent,
        cancel: &Cancel,
    ) -> Vec<PactEvent> {
        events_from(scratch, &Manifest::new(), work, agent, cancel)
    }

    /// The same, starting from `manifest` rather than from nothing.
    ///
    /// What a refresh needs and a first pact does not: which directories are
    /// stale is decided against the manifest the run is handed, so a refresh
    /// over an empty one would find everything stale and prove nothing.
    fn events_from(
        scratch: &Scratch,
        manifest: &Manifest,
        work: &Work,
        agent: &dyn Agent,
        cancel: &Cancel,
    ) -> Vec<PactEvent> {
        let (events, received) = mpsc::channel();
        run_pact(manifest, &scratch.root, work, agent, cancel, &events);
        drop(events);
        received.into_iter().collect()
    }

    /// What each directory a run announced said it was: its position and the
    /// total the observer reported alongside it.
    fn fractions(events: &[PactEvent]) -> Vec<(usize, usize)> {
        events
            .iter()
            .filter_map(|event| match event {
                PactEvent::Starting {
                    position, total, ..
                } => Some((*position, *total)),
                PactEvent::Doing(_)
                | PactEvent::Summarising { .. }
                | PactEvent::Documented { .. }
                | PactEvent::Finished(_) => None,
            })
            .collect()
    }

    /// The directories a run announced, in the order it announced them,
    /// spelled relative to `scratch`'s root.
    fn announced(events: &[PactEvent], scratch: &Scratch) -> Vec<PathBuf> {
        events
            .iter()
            .filter_map(|event| match event {
                PactEvent::Starting { directory, .. } => Some(
                    directory
                        .strip_prefix(&scratch.root)
                        .unwrap_or(directory)
                        .to_path_buf(),
                ),
                PactEvent::Doing(_)
                | PactEvent::Summarising { .. }
                | PactEvent::Documented { .. }
                | PactEvent::Finished(_) => None,
            })
            .collect()
    }

    /// The one outcome a run ends with.
    fn outcome_of(events: &[PactEvent]) -> &Result<Toggled, String> {
        match events.last() {
            Some(PactEvent::Finished(outcome)) => outcome,
            _ => panic!("the worker said: {events:?}"),
        }
    }

    /// A repository with two crates in it, so a cancel can land with one
    /// subtree finished and the other not.
    fn two_crates(name: &str) -> Scratch {
        let scratch = Scratch::new(name);
        scratch.write("crates/alpha/src/lib.rs", "//! Alpha.\n");
        scratch.write("crates/beta/src/lib.rs", "//! Beta.\n");
        scratch
    }

    /// Pact the subtree at `relative` for real, and hand back the manifest it
    /// earned.
    ///
    /// Where every refresh test starts, because a refresh is only a question
    /// about a subtree that was pacted once: the grants this leaves are what
    /// staleness is later decided against, and the documents it writes are what
    /// a skipped directory keeps. Saved to disk as a real pact saves it, so the
    /// refresh that follows runs against the repository a reader would have.
    fn pacted(scratch: &Scratch, relative: &str) -> Manifest {
        let Toggled {
            manifest, granted, ..
        } = apply_toggle(
            &Manifest::new(),
            &scratch.root,
            &toggle(scratch, relative, true),
            &Canned::new(scratch, []),
            &mut Unwatched,
        )
        .expect("a subtree that walks and a manifest that writes");
        assert!(granted, "the subtree a refresh test starts from is fresh");
        manifest
    }

    #[test]
    fn the_worker_names_each_directory_in_turn_and_then_says_how_the_run_went() {
        let scratch = one_crate("progress");
        let agent = Canned::new(&scratch, []);

        let events = events_of(
            &scratch,
            &toggle(&scratch, "crates/engine", true),
            &agent,
            &Cancel::new(),
        );

        // One announcement per directory, in the order the passes run —
        // children before parents — counted from one against a total that
        // does not move, each answered by the word that its pass delivered,
        // and then exactly one outcome and nothing after it.
        let [
            PactEvent::Starting {
                directory: first,
                position: 1,
                total: 2,
            },
            PactEvent::Documented {
                directory: first_done,
            },
            PactEvent::Starting {
                directory: second,
                position: 2,
                total: 2,
            },
            PactEvent::Documented {
                directory: second_done,
            },
            PactEvent::Finished(Ok(Toggled {
                manifest,
                granted: true,
                message: None,
                ..
            })),
        ] = events.as_slice()
        else {
            panic!("the worker said: {events:?}");
        };

        assert_eq!(first, &scratch.path("crates/engine/src"));
        assert_eq!(second, &scratch.path("crates/engine"));
        assert_eq!(
            first_done, first,
            "documented names the pass that delivered"
        );
        assert_eq!(second_done, second);
        assert_eq!(agent.directories().len(), 2, "and it ran both passes");
        // The outcome that reaches the loop is the one that reached disk:
        // saved once, at the end, by the worker itself.
        assert_eq!(
            &saved(&scratch.root).expect("the manifest was written"),
            manifest
        );
    }

    #[test]
    fn what_each_pass_is_doing_arrives_between_its_directory_and_the_next() {
        let scratch = one_crate("activities");
        // The worker's own channel, made here so the fake can be given the
        // very port `spawn_pact` gives this run's agent: one function, one
        // route, and no second channel anywhere in the picture.
        let (events, received) = mpsc::channel();
        let agent = Canned::new(&scratch, []).reporting(activity_port(&events));

        run_pact(
            &Manifest::new(),
            &scratch.root,
            &toggle(&scratch, "crates/engine", true),
            &agent,
            &Cancel::new(),
            &events,
        );
        // Both ends the worker would have held: its own, and the one inside
        // the port attached to its agent. The channel closes when the last
        // of them goes, which on the real path is the worker's thread ending
        // and here is these two lines.
        drop(events);
        drop(agent);
        let events: Vec<PactEvent> = received.into_iter().collect();

        // Both streams in one sequence, in the order the run produced them:
        // a directory is announced, then what its pass did, then the next
        // directory. The three kinds of activity arrive whole and unaltered
        // — this channel carries them, it does not interpret them.
        let [
            PactEvent::Starting {
                directory: first,
                position: 1,
                total: 2,
            },
            PactEvent::Doing(Activity::Tool {
                name: first_tool,
                detail: Some(first_detail),
            }),
            PactEvent::Doing(Activity::Thinking),
            PactEvent::Doing(Activity::Cost { usd: first_cost }),
            PactEvent::Documented { .. },
            PactEvent::Starting {
                directory: second,
                position: 2,
                total: 2,
            },
            PactEvent::Doing(Activity::Tool {
                detail: Some(second_detail),
                ..
            }),
            PactEvent::Doing(Activity::Thinking),
            PactEvent::Doing(Activity::Cost { .. }),
            PactEvent::Documented { .. },
            PactEvent::Finished(Ok(Toggled { granted: true, .. })),
        ] = events.as_slice()
        else {
            panic!("the worker said: {events:?}");
        };

        assert_eq!(first, &scratch.path("crates/engine/src"));
        assert_eq!(second, &scratch.path("crates/engine"));
        assert_eq!(first_tool, "Read");
        // Carried, not computed with: what is asserted is that the number
        // the fake reported is the number that came out the other end.
        assert!(
            (*first_cost - 0.25).abs() < f64::EPSILON,
            "the cost arrives as the pass said it: {first_cost}"
        );
        assert_eq!(first_detail, "crates/engine/src");
        assert_eq!(second_detail, "crates/engine");

        // The rule activities do not get to break: one outcome, and it is
        // the last thing on the channel. Everything the worker ever sent is
        // in this vector — its end was dropped before a single event was
        // read — so a stray activity after the outcome would be here.
        let outcomes = events
            .iter()
            .filter(|event| matches!(event, PactEvent::Finished(_)))
            .count();
        assert_eq!(outcomes, 1, "exactly one outcome: {events:?}");
        assert!(
            matches!(events.last(), Some(PactEvent::Finished(_))),
            "and nothing follows it: {events:?}"
        );
        // And the announcements are what they were before activities shared
        // the channel with them.
        assert_eq!(
            announced(&events, &scratch),
            [
                PathBuf::from("crates/engine/src"),
                PathBuf::from("crates/engine")
            ]
        );
    }

    /// A file too big for one request and too big for one chunk of one:
    /// comfortably over [`PER_FILE_BYTE_CAP`], so the engine summarises it
    /// rather than sending it, and over twice the chunk size under that cap, so
    /// summarising it is several map passes and a reduce over them.
    ///
    /// Lines, because the chunker cuts just after a newline, and lockfile-ish
    /// filler because that is what an over-cap file in a repository actually is.
    fn over_the_cap() -> String {
        let line = "checksum = \"0123456789abcdef0123456789abcdef\"\n";
        let cap = usize::try_from(PER_FILE_BYTE_CAP).expect("the cap is a few kilobytes");
        let lines = 2 * cap / line.len() + 1;
        line.repeat(lines)
    }

    #[test]
    fn the_passes_over_a_big_file_are_announced_inside_the_directory_holding_it() {
        let scratch = one_crate("summarising");
        scratch.write("crates/engine/src/deps.lock", &over_the_cap());
        // The same fake as everywhere else: its answer is long enough to clear
        // `MINIMUM_DOCUMENT_BYTES` and so more than long enough to be kept as
        // an account of a chunk, which is all a summarising pass asks of it.
        let agent = Canned::new(&scratch, []);

        let events = events_of(
            &scratch,
            &toggle(&scratch, "crates/engine", true),
            &agent,
            &Cancel::new(),
        );

        // The two directories' announcements, by where they landed in the one
        // sequence: everything between them is work done inside the first of
        // them, which is the whole reason these ride the same channel rather
        // than a second one that could arrive out of order.
        let starting: Vec<usize> = events
            .iter()
            .enumerate()
            .filter(|(_, event)| matches!(event, PactEvent::Starting { .. }))
            .map(|(index, _)| index)
            .collect();
        let [first, second] = starting.as_slice() else {
            panic!("one announcement per directory: {events:?}");
        };

        let passes: Vec<(&PathBuf, usize, usize)> = events
            .iter()
            .enumerate()
            .filter_map(|(index, event)| match event {
                PactEvent::Summarising { file, part, parts } => {
                    assert!(
                        index > *first && index < *second,
                        "a pass over a file in crates/engine/src arrived outside it: {events:?}"
                    );
                    Some((file, *part, *parts))
                }
                _ => None,
            })
            .collect();

        // And the stretch they had to land in is the right one: children before
        // parents, so the first directory announced is the deeper one, which is
        // where the big file is.
        assert_eq!(
            announced(&events, &scratch)[0],
            PathBuf::from("crates/engine/src")
        );

        let parts = passes
            .first()
            .expect("an over-cap file costs at least one pass")
            .2;
        assert!(
            parts > 1,
            "several chunks and the reduce over them: {passes:?}"
        );
        assert_eq!(
            passes.len(),
            parts,
            "every pass the file costs is announced: {passes:?}"
        );
        // Carried, not computed with: the file the engine named, the count it
        // gave, and the parts running 1..=parts in order.
        for (index, (file, part, of)) in passes.iter().enumerate() {
            assert_eq!(*file, &scratch.path("crates/engine/src/deps.lock"));
            assert_eq!(*part, index + 1);
            assert_eq!(*of, parts, "the count does not move: {passes:?}");
        }

        // And none of it changed how the run went: the passes are an
        // announcement, not a vote.
        assert!(
            matches!(outcome_of(&events), Ok(Toggled { granted: true, .. })),
            "the run still granted the subtree: {events:?}"
        );
    }

    /// A `claude` that prints one tool use and then a result line carrying
    /// `document`, and exits.
    ///
    /// Quoting is single quotes around JSON that contains none, as in
    /// `claude.rs`'s stand-ins, and `printf '%s\n' a b` is one process and
    /// no loop, so every line arrives whole.
    #[cfg(unix)]
    fn stand_in(document: &str) -> String {
        let tool = concat!(
            r#"{"type":"assistant","message":{"role":"assistant","content":"#,
            r#"[{"type":"tool_use","id":"toolu_1","name":"Read","#,
            r#""input":{"file_path":"src/lib.rs"}}]}}"#,
        );
        let result = format!(
            r#"{{"type":"result","subtype":"success","result":"{document}","total_cost_usd":0.5}}"#
        );
        format!("printf '%s\\n' '{tool}' '{result}'")
    }

    #[cfg(unix)]
    #[test]
    fn a_spawned_run_reports_what_its_passes_do_over_the_channel_it_hands_back() {
        // The one thing `spawn_pact` does that driving `run_pact` cannot
        // show: attaching the port. It goes on a `ClaudeAgent` and on
        // nothing else, so this run has a real one, over a shell stand-in
        // printing a stream a pass would print. Handing the fake a port the
        // test made itself would prove only that the test can call
        // `activity_port`.
        let scratch = one_crate("spawned-activities");
        // Long enough that the engine keeps what comes back:
        // `MINIMUM_DOCUMENT_BYTES` is 200. `\n` inside the JSON string is
        // the two characters JSON wants, not a newline in the shell's way.
        let prose =
            "What this directory is for, said at about the length a real document says it at. ";
        let document = format!("# engine\\n\\n{prose}{prose}{prose}");
        let script = stand_in(&document);
        let agent = ClaudeAgent::new()
            .with_program("/bin/sh")
            .with_args(["-c", script.as_str()]);

        let received = spawn_pact(
            &Manifest::new(),
            &scratch.root,
            &toggle(&scratch, "crates/engine", true),
            &agent,
            Cancel::new(),
        );
        // Blocks until every sender is gone, which is the worker's own end
        // and the one inside the port on the agent it owns. That it returns
        // at all is half the assertion: a port that outlived its run would
        // hang this line rather than fail it.
        let events: Vec<PactEvent> = received.into_iter().collect();

        let activities: Vec<&Activity> = events
            .iter()
            .filter_map(|event| match event {
                PactEvent::Doing(activity) => Some(activity),
                _ => None,
            })
            .collect();
        // Two directories, and each pass says the same two things: the tool
        // it used, with its one whitelisted detail, and what it cost.
        assert_eq!(
            activities.len(),
            4,
            "both passes reported through the port `spawn_pact` attached: {events:?}"
        );
        assert!(
            activities.iter().all(|activity| matches!(
                activity,
                Activity::Tool { name, detail: Some(detail) }
                    if name == "Read" && detail == "src/lib.rs"
            ) || matches!(activity, Activity::Cost { .. })),
            "and said what the stream said: {activities:?}"
        );

        // The rule a real agent does not get to break either.
        assert!(
            matches!(events.last(), Some(PactEvent::Finished(Ok(_)))),
            "one outcome, and it is the last thing on the channel: {events:?}"
        );
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event, PactEvent::Finished(_)))
                .count(),
            1,
            "exactly one outcome: {events:?}"
        );
    }

    #[test]
    fn a_run_that_reaches_no_model_comes_back_naming_the_program_it_looked_for() {
        let scratch = one_crate("no-agent");
        // A name nothing on any `PATH` answers to, standing in for a machine
        // with no `claude` installed. The spawn fails for every directory,
        // and the run still comes back as an outcome with a line for the
        // footer rather than as a crash or as silence.
        let missing = "warlock-claude-that-is-not-installed";
        let agent = ClaudeAgent::new().with_program(missing);

        let events = events_of(
            &scratch,
            &toggle(&scratch, "crates/engine", true),
            &agent,
            &Cancel::new(),
        );

        let Some(PactEvent::Finished(Ok(Toggled {
            granted, message, ..
        }))) = events.last()
        else {
            panic!("the worker said: {events:?}");
        };

        assert!(!*granted, "nothing was documented, so nothing is fresh");
        let message = message.as_deref().expect("the failure is reported");
        assert!(!message.contains('\n'), "the footer is one line: {message}");
        assert!(
            message.contains(missing),
            "the binary that could not be run is named: {message}"
        );
    }

    #[test]
    fn the_pact_key_starts_nothing_while_a_pact_is_in_flight() {
        // No filesystem: which subtree a press asks for is app state, and
        // the tree it is asked of is written out here.
        let tree = Tree::new(Node::new(
            "/repo/crates",
            None::<PathBuf>,
            NodeState::Unpacted,
        ));
        let mut app = App::from_tree(&tree);
        // The run the press is going to bounce off, as the event loop would
        // have told the app about it, and the sentence the last keystroke
        // left, which is not this press's to spend.
        app.set_pact_in_flight("/repo/crates/engine", 3, 12);
        app.set_message("something the last key said");
        let before = app.clone();

        assert_eq!(
            pact_press(&mut app, true, Instant::now()),
            None,
            "no second pact"
        );
        // Saying so is the whole of it: the same app with the flag set and
        // nothing else moved — no colour, no selection, no account started
        // and the rows exactly as they were.
        let refused = {
            let mut refused = before.clone();
            refused.set_pact_refused();
            refused
        };
        assert_eq!(app, refused, "the press did more than say so");
        assert_eq!(
            app.pact_line().as_deref(),
            Some("pacting engine (3/12) — already running"),
            "the refusal is worded onto the line the reader is watching"
        );
        assert_eq!(
            app.message(),
            Some("something the last key said"),
            "the refusal did not go through the message"
        );

        // The same press, with nothing running, is the press that starts a
        // pact: same key, same app, different answer. The run being over is
        // the loop's news, so the app hears it the way it always does.
        app.clear_pact_in_flight();
        let toggle =
            pact_press(&mut app, false, Instant::now()).expect("a directory can be pacted");
        assert_eq!(toggle.path, PathBuf::from("/repo/crates"));
        assert!(toggle.pacted);
        assert_ne!(app, before, "the subtree it covers is painted");
    }

    #[test]
    fn leaning_on_the_pact_key_while_a_pact_is_in_flight_changes_nothing_further() {
        let tree = Tree::new(Node::new(
            "/repo/crates",
            None::<PathBuf>,
            NodeState::Unpacted,
        ));
        let mut app = App::from_tree(&tree);
        app.set_pact_in_flight("/repo/crates/engine", 3, 12);

        assert_eq!(pact_press(&mut app, true, Instant::now()), None);
        let after_one = app.clone();

        // A reader who presses again is asking the same question, and the
        // answer already on screen is the same one. There is nothing to
        // count and nothing to say twice.
        assert_eq!(pact_press(&mut app, true, Instant::now()), None);
        assert_eq!(pact_press(&mut app, true, Instant::now()), None);

        assert_eq!(app, after_one, "pressing again changed something");
        assert_eq!(
            app.pact_line().as_deref(),
            Some("pacting engine (3/12) — already running")
        );
    }

    #[test]
    fn a_press_on_a_file_row_with_nothing_running_still_refuses_through_the_message() {
        // A directory with a file under it, and the files shown, so the
        // selection can sit on the one row a pact is never about.
        let tree = Tree::new(
            Node::new("/repo/crates", None::<PathBuf>, NodeState::Unpacted)
                .with_files([PathBuf::from("/repo/crates/Cargo.toml")]),
        );
        let mut app = App::from_tree(&tree);
        app.toggle_files();
        app.select_next();
        assert_eq!(
            app.selected_row().map(|row| row.path.clone()),
            Some(PathBuf::from("/repo/crates/Cargo.toml")),
            "the selection is on the file"
        );

        assert_eq!(
            pact_press(&mut app, false, Instant::now()),
            None,
            "a file is not a thing to pact"
        );

        // Its own refusal, in its own place: no run has the message line, so
        // the sentence the app wrote is the one that is drawn. And no flag,
        // because no run turned this press down.
        assert!(
            app.message().is_some_and(|message| !message.is_empty()),
            "the file row said nothing"
        );
        assert_eq!(app.pact_line(), None, "nothing is running to be refused by");
    }

    #[test]
    fn a_press_on_a_row_the_ignore_file_keeps_out_starts_nothing() {
        // A directory the repository's `.warlockignore` excludes, said the way
        // the load says it: a flag on the node, so no filesystem is needed to
        // answer the key.
        let tree =
            Tree::new(
                Node::new("/repo", None::<PathBuf>, NodeState::Unpacted).with_children([
                    Node::new("/repo/notes", None::<PathBuf>, NodeState::Unpacted)
                        .with_ignored(true),
                ]),
            );
        let mut app = App::from_tree(&tree);
        app.select_next();
        assert_eq!(
            app.selected_row().map(|row| row.path.clone()),
            Some(PathBuf::from("/repo/notes")),
            "the selection is on the excluded directory"
        );
        let before = app.clone();

        assert_eq!(
            pact_press(&mut app, false, Instant::now()),
            None,
            "there is nothing in there to pact"
        );

        // The refusal is the app's, so it goes through the message, and this
        // function adds nothing to it: no account is opened for a run that never
        // started, and no flag is set, because no run turned the press down.
        assert!(
            app.message()
                .is_some_and(|message| message.contains(".warlockignore")),
            "the excluded row said nothing about the rules that keep it out"
        );
        assert!(!app.has_account(), "an account was opened for no run");
        assert_eq!(app.pact_line(), None, "nothing is running to be refused by");
        let refused = {
            let mut refused = before.clone();
            refused.set_message(app.message().expect("the app said why"));
            refused
        };
        assert_eq!(app, refused, "the press did more than say so");
    }

    #[test]
    fn the_press_that_starts_a_run_starts_an_account_and_an_un_pact_leaves_it_alone() {
        // One directory and three presses of the same key: a pact, the
        // un-pact that undoes it, and a second pact. No filesystem, because
        // what is under test is what a press does to the app.
        let tree = Tree::new(Node::new(
            "/repo/crates",
            None::<PathBuf>,
            NodeState::Unpacted,
        ));
        let mut app = App::from_tree(&tree);
        let base = Instant::now();

        assert!(!app.has_account(), "no pact has run this session");

        // The pact, and a line of the run it started.
        let toggle = pact_press(&mut app, false, base).expect("a directory can be pacted");
        assert!(toggle.pacted);
        let account = app.account_mut().expect("the press started an account");
        account.open_section("crates", base);
        account.record(&Activity::Thinking, at(base, 1));
        assert_eq!(app.account().map(Account::line_count), Some(2));

        // The un-pact. It runs no pass and reports nothing, so wiping the
        // record of the run that wrote the documents it is removing would
        // cost the reader the only account they have for no news at all.
        let toggle = pact_press(&mut app, false, at(base, 2)).expect("it is pacted now");
        assert!(!toggle.pacted, "the second press takes the pact off");
        assert_eq!(
            app.account().map(Account::line_count),
            Some(2),
            "the last run's account is still on screen"
        );

        // The second pact. One pact, one account: this one starts empty
        // rather than under the last one.
        let toggle = pact_press(&mut app, false, at(base, 3)).expect("it can be pacted again");
        assert!(toggle.pacted);
        assert_eq!(
            app.account().map(Account::line_count),
            Some(0),
            "a new run starts from nothing"
        );
    }

    #[test]
    fn either_key_starts_nothing_while_a_run_is_in_flight() {
        // No filesystem: what a press comes to while something is running is
        // app state and a bit of wording. The run in flight is a refresh, and
        // the account it opened has a section in it, so that a refusal that
        // cleared the panel would show.
        let tree = Tree::new(Node::new(
            "/repo/crates",
            None::<PathBuf>,
            NodeState::PactedStale,
        ));
        let mut app = App::from_tree(&tree);
        let base = Instant::now();
        app.start_account(base);
        app.account_mut()
            .expect("the press that started the run opened one")
            .open_section("engine", base);
        app.set_run_in_flight(Run::Refresh, "/repo/crates/engine", 2, 7);
        app.set_message("something the last key said");
        let before = app.clone();

        // The refresh key during a refresh, and the pact key during the same
        // refresh: one in-flight check, so one answer.
        assert_eq!(
            refresh_press(&mut app, true, at(base, 1)),
            None,
            "no second run"
        );
        assert_eq!(
            pact_press(&mut app, true, at(base, 2)),
            None,
            "and not by the other key either"
        );

        let refused = {
            let mut refused = before.clone();
            refused.set_pact_refused();
            refused
        };
        assert_eq!(app, refused, "the presses did more than say so");
        assert_eq!(
            app.pact_line().as_deref(),
            Some("refreshing engine (2/7) — already running"),
            "the refusal is worded onto the line the reader is watching"
        );
        assert_eq!(
            app.message(),
            Some("something the last key said"),
            "the refusal did not go through the message"
        );
        assert_eq!(
            app.account().map(Account::line_count),
            before.account().map(Account::line_count),
            "the running run's account was cleared"
        );

        // And the other direction, which is the same sentence about the other
        // run: a refresh asked for during a pact.
        app.set_pact_in_flight("/repo/crates/engine", 3, 12);
        assert_eq!(refresh_press(&mut app, true, at(base, 3)), None);
        assert_eq!(
            app.pact_line().as_deref(),
            Some("pacting engine (3/12) — already running")
        );
    }

    #[test]
    fn a_refresh_of_a_directory_with_nothing_stale_under_it_starts_no_run() {
        // The good outcome: the reader asked whether anything needed
        // describing again and the answer is no, which costs a sentence rather
        // than a run.
        let fresh = Tree::new(Node::new(
            "/repo/crates",
            None::<PathBuf>,
            NodeState::PactedFresh,
        ));
        let mut app = App::from_tree(&fresh);
        let base = Instant::now();

        assert_eq!(
            refresh_press(&mut app, false, base),
            None,
            "there is nothing under it to describe again"
        );
        assert!(!app.has_account(), "a run nobody started has no account");
        assert!(
            app.message()
                .is_some_and(|message| message.contains("already fresh")),
            "the fresh row said nothing: {:?}",
            app.message()
        );
        assert_eq!(app.pact_line(), None, "nothing is running to be refused by");

        // The same key on the same directory gone stale is the press that
        // starts a run, and it starts the panel's account at the moment of the
        // press — which is what the clocks under it are counted from.
        let stale = Tree::new(Node::new(
            "/repo/crates",
            None::<PathBuf>,
            NodeState::PactedStale,
        ));
        let mut app = App::from_tree(&stale);
        assert_eq!(
            refresh_press(&mut app, false, base),
            Some(PathBuf::from("/repo/crates")),
            "a stale subtree is a thing to refresh"
        );
        let account = app.account_mut().expect("the press started an account");
        account.open_section("crates", at(base, 5));
        account.finish(at(base, 5));
        assert_eq!(
            panel_text(&app, at(base, 5)),
            [
                "crates".to_owned(),
                "0:00 waiting".to_owned(),
                // Five seconds of run, and nothing in it had happened yet:
                // the account was opened by the press.
                "pact finished — 1 directory, 0:05, $0.00 (incomplete: 1 pass reported no cost)"
                    .to_owned(),
            ],
            "the run is counted from the press that started it"
        );
    }

    #[test]
    fn a_cancel_stops_the_descent_and_records_only_what_finished() {
        let scratch = two_crates("cancelled");
        let cancel = Cancel::new();
        // One directory that goes wrong on its own, and one that the reader
        // stops the run during: `alpha/src` is refused the way a pass that
        // fails always is, and Esc is pressed while `alpha` is being worked.
        // The engine walks in reverse path order, so `beta` comes before
        // `alpha` and the pact's own root comes last of all.
        let agent = Canned::new(&scratch, ["crates/alpha/src"])
            .cancelling_at("crates/alpha", cancel.clone());

        let events = events_of(&scratch, &toggle(&scratch, "crates", true), &agent, &cancel);

        // Four directories offered a pass out of five, and the fifth — the
        // root of the pact, worked last because parents come after their
        // children — never announced and never run: the descent stopped
        // between directories rather than part way through one.
        let worked = [
            PathBuf::from("crates/beta/src"),
            PathBuf::from("crates/beta"),
            PathBuf::from("crates/alpha/src"),
            PathBuf::from("crates/alpha"),
        ];
        assert_eq!(announced(&events, &scratch), worked);
        assert_eq!(agent.directories(), worked);

        let Ok(Toggled {
            manifest,
            granted,
            message,
            ..
        }) = outcome_of(&events)
        else {
            panic!("a cancelled pact still saves what it finished: {events:?}");
        };
        assert!(!granted, "a run that was stopped has proved nothing fresh");
        // The refusal above is in this run's failures, and the footer says
        // none of it: what happened is that somebody pressed Esc.
        assert_eq!(message.as_deref(), Some(PACT_CANCELLED));

        // What is on disk is what finished, and nothing else. `beta` was
        // documented with nothing missing under it, so it earned a grant and
        // draws green; `alpha` was documented over a child that failed, so it
        // has an entry with no grant and draws yellow; everything the run
        // never reached has no entry at all.
        assert_eq!(
            &saved(&scratch.root).expect("the manifest was written"),
            manifest
        );
        for module in ["crates/beta", "crates/beta/src"] {
            let hash = subtree_hash(scratch.path(module)).expect("it hashes");
            assert_eq!(
                decide_state(manifest.entry(module), &hash),
                NodeState::PactedFresh,
                "a subtree that finished is green: {manifest:?}"
            );
        }
        let hash = subtree_hash(scratch.path("crates/alpha")).expect("it hashes");
        assert_eq!(
            decide_state(manifest.entry("crates/alpha"), &hash),
            NodeState::PactedStale,
            "a directory with an unfinished descendant is yellow: {manifest:?}"
        );
        for module in ["crates/alpha/src", "crates"] {
            assert_eq!(
                manifest.entry(module),
                None,
                "`{module}` was never documented, so it is not recorded"
            );
        }

        // And every document written before the cancel is still there.
        for module in ["crates/alpha", "crates/beta", "crates/beta/src"] {
            assert!(
                scratch.path(module).join(DOCUMENT_FILE).is_file(),
                "`{module}`'s document did not survive the cancel"
            );
        }
    }

    #[test]
    fn a_cancel_grants_nothing_even_when_no_pass_went_wrong() {
        let scratch = two_crates("cancelled-clean");
        let cancel = Cancel::new();
        // Nothing refused this time: the run is stopped with every pass it
        // ran having worked, which is what a cancel usually looks like. Esc
        // lands on the last directory before the pact's own root, which the
        // engine works last of all.
        let agent = Canned::new(&scratch, []).cancelling_at("crates/alpha", cancel.clone());

        let events = events_of(&scratch, &toggle(&scratch, "crates", true), &agent, &cancel);

        let Ok(Toggled {
            manifest,
            granted,
            message,
            ..
        }) = outcome_of(&events)
        else {
            panic!("a cancelled pact still saves what it finished: {events:?}");
        };
        // Not a failure in sight, and still not fresh: the subtree's own
        // root was never documented, so painting the whole of it green —
        // which is all `granted` is for — would be a claim about
        // directories this run never opened.
        assert!(
            !granted,
            "a stopped run grants nothing, whatever it did not trip over"
        );
        assert_eq!(message.as_deref(), Some(PACT_CANCELLED));
        assert_eq!(manifest.entry("crates"), None);
        assert!(
            !scratch.path("crates").join(DOCUMENT_FILE).exists(),
            "the root of the pact was never worked"
        );
        // The crates that did finish kept everything they earned.
        for module in [
            "crates/alpha",
            "crates/alpha/src",
            "crates/beta",
            "crates/beta/src",
        ] {
            let hash = subtree_hash(scratch.path(module)).expect("it hashes");
            assert_eq!(
                decide_state(manifest.entry(module), &hash),
                NodeState::PactedFresh,
                "`{module}` finished before the cancel: {manifest:?}"
            );
        }
    }

    #[test]
    fn a_refresh_describes_the_stale_directories_and_leaves_the_fresh_ones_as_they_were() {
        // A pact of everything, then one file moved under `beta`, then the
        // refresh key on the root of it all. Three of the five directories are
        // stale — the one holding the file, its parent and the root above them
        // — and the run is over the three rather than the five.
        let scratch = two_crates("refreshed");
        let before = pacted(&scratch, "crates");
        let kept: Vec<PactEntry> = ["crates/alpha", "crates/alpha/src"]
            .into_iter()
            .map(|module| {
                before
                    .entry(module)
                    .expect("the pact recorded every directory")
                    .clone()
            })
            .collect();
        scratch.write("crates/beta/src/lib.rs", "//! Beta, and something new.\n");

        let agent = Canned::new(&scratch, []);
        let events = events_from(
            &scratch,
            &before,
            &refreshing(&scratch, "crates"),
            &agent,
            &Cancel::new(),
        );

        // Announced and worked in the same reverse order a pact walks, and
        // `alpha` in neither list: a pass is what a refresh is trying not to
        // spend.
        let stale = [
            PathBuf::from("crates/beta/src"),
            PathBuf::from("crates/beta"),
            PathBuf::from("crates"),
        ];
        assert_eq!(announced(&events, &scratch), stale);
        assert_eq!(agent.directories(), stale);
        // And the fraction counts the run rather than the subtree: three of
        // three, from the engine's observer, with five directories under the
        // key that was pressed.
        assert_eq!(fractions(&events), [(1, 3), (2, 3), (3, 3)]);

        let Ok(Toggled {
            manifest,
            granted,
            message,
            ..
        }) = outcome_of(&events)
        else {
            panic!("a refresh that walks and saves: {events:?}");
        };
        assert!(granted, "no pass went wrong, so the subtree is fresh");
        assert_eq!(*message, None, "and there is nothing to report");

        // The skipped directories are the entries they were, grant and all —
        // the engine's rule (WAR-39), which is what lets the whole subtree be
        // painted fresh off `granted` alone.
        for entry in &kept {
            assert_eq!(
                manifest.entry(entry.module()),
                Some(entry),
                "a skipped directory was rewritten"
            );
        }
        assert_eq!(
            &saved(&scratch.root).expect("the manifest was written"),
            manifest,
            "the one save at the end is this run's"
        );
        for module in [
            "crates",
            "crates/alpha",
            "crates/alpha/src",
            "crates/beta",
            "crates/beta/src",
        ] {
            let hash = subtree_hash(scratch.path(module)).expect("it hashes");
            assert_eq!(
                decide_state(manifest.entry(module), &hash),
                NodeState::PactedFresh,
                "`{module}` is not fresh after the refresh: {manifest:?}"
            );
        }
    }

    #[test]
    fn a_cancelled_refresh_saves_what_it_finished_and_says_it_was_stopped() {
        // Both crates move, so every directory under `crates` is stale and the
        // refresh has all five to work; Esc lands while `alpha` is being
        // described, which is the fourth of them.
        let scratch = two_crates("refresh-cancelled");
        scratch.write(".git/HEAD", "ref: refs/heads/main\n");
        let before = pacted(&scratch, "crates");
        scratch.write("crates/alpha/src/lib.rs", "//! Alpha, edited.\n");
        scratch.write("crates/beta/src/lib.rs", "//! Beta, edited.\n");

        let (mut app, scope) = load(&scratch);
        let mut manifest = before.clone();
        let base = Instant::now();
        app.start_account(base);

        // The event loop's own handle and the clone the run answers to: the
        // pair `spawn_pact` makes, so what the panel reads at the end is the
        // flag the reader's key latched.
        let guard = CancelGuard::new();
        let cancel = guard.handle();
        let work = refreshing(&scratch, "crates");
        let said = recorded_from(&scratch, &before, &work, &cancel, |events| {
            Canned::new(&scratch, [])
                .reporting(activity_port(events))
                .cancelling_at("crates/alpha", cancel.clone())
        });

        // The descent stopped between directories, exactly as a cancelled pact
        // stops: four of the five offered a pass, and the root of the run —
        // worked last, after its children — was never announced and never run.
        let worked = [
            PathBuf::from("crates/beta/src"),
            PathBuf::from("crates/beta"),
            PathBuf::from("crates/alpha/src"),
            PathBuf::from("crates/alpha"),
        ];
        assert_eq!(announced(&said, &scratch), worked);
        assert_eq!(fractions(&said), [(1, 5), (2, 5), (3, 5), (4, 5)]);

        let Ok(Toggled {
            granted, message, ..
        }) = outcome_of(&said)
        else {
            panic!("a cancelled refresh still saves what it finished: {said:?}");
        };
        // Reworded as the stopped run it is, which is the whole of what a
        // refresh needed of `run_pact`: a pact says this and an un-pact cannot.
        assert_eq!(message.as_deref(), Some(PACT_CANCELLED));
        assert!(!granted, "a run that was stopped has proved nothing fresh");

        let progress = replay_work(&mut app, &mut manifest, &scope, guard, &work, said, base);
        assert_eq!(progress.len(), 4, "one line per directory: {progress:?}");

        // What the run finished is on disk. The four directories that did are
        // green again, and the root the cancel came before keeps the entry its
        // pact gave it and stays yellow: a refresh drops nothing it could not
        // re-describe.
        assert_eq!(
            saved(&scratch.root).expect("the manifest was written"),
            manifest,
            "what the run finished is on disk"
        );
        for module in [
            "crates/alpha",
            "crates/alpha/src",
            "crates/beta",
            "crates/beta/src",
        ] {
            let hash = subtree_hash(scratch.path(module)).expect("it hashes");
            assert_eq!(
                decide_state(manifest.entry(module), &hash),
                NodeState::PactedFresh,
                "`{module}` finished before the cancel: {manifest:?}"
            );
        }
        let hash = subtree_hash(scratch.path("crates")).expect("it hashes");
        assert_eq!(
            decide_state(manifest.entry("crates"), &hash),
            NodeState::PactedStale,
            "the directory the cancel came before is still pacted and still stale"
        );
        assert_eq!(
            state_of(&app, &scratch.path("crates")),
            Some(NodeState::PactedStale),
            "a stopped run painted the subtree green"
        );

        // And the panel says it was stopped, in the section it was stopped in
        // and with what that pass had spent by then — the same ending a
        // cancelled pact leaves, because it is the same code closing it.
        let lines = panel_text(&app, at(base, 10_000));
        assert_eq!(
            &lines[lines.len() - 5..],
            [
                "crates/alpha".to_owned(),
                "0:20 Read crates/alpha".to_owned(),
                "0:50 thinking".to_owned(),
                "0:50 cancelled — $0.25 spent".to_owned(),
                "pact finished — 4 directories, 3:20, $1.00".to_owned(),
            ],
            "the cancel is recorded in the section it happened in"
        );
        assert_eq!(app.message(), Some(PACT_CANCELLED));
    }

    #[test]
    fn a_run_that_goes_out_of_scope_takes_its_claude_with_it() {
        // What quitting mid-pact comes to: `q` and Ctrl-C return from the
        // event loop, the run goes out of scope with them, and the child it
        // was waiting on is killed on the way — no join, and no `claude`
        // left running against a terminal nobody is looking at any more.
        let tree = Tree::new(Node::new(
            "/repo/crates",
            None::<PathBuf>,
            NodeState::Unpacted,
        ));
        let cancel = CancelGuard::new();
        let watching = cancel.handle();
        let (_events, received) = mpsc::channel();
        let running = Running {
            events: received,
            cancel,
            work: pact_of("/repo/crates"),
            before: App::from_tree(&tree),
        };

        assert!(!watching.is_cancelled(), "a run in flight is not cancelled");

        drop(running);

        assert!(
            watching.is_cancelled(),
            "the run outlived the loop that started it"
        );
    }

    /// Everything the event loop holds one moment after the pact key
    /// started a run over `/repo/crates` at `base`: the app with the account
    /// that press opened, the copy of it taken before the toggle painted,
    /// the manifest on disk, the end of the channel a worker would talk
    /// down, and the run itself.
    ///
    /// The copy comes back because the footer's wording is asserted against
    /// it: what the progress line says during a run is what it has always
    /// said, and the way to show that is to say it on an app this slice
    /// never touched.
    fn a_run_in_flight(base: Instant) -> (App, App, Manifest, mpsc::Sender<PactEvent>, Running) {
        let tree = Tree::new(Node::new(
            "/repo/crates",
            None::<PathBuf>,
            NodeState::Unpacted,
        ));
        let before = App::from_tree(&tree);
        let mut app = before.clone();
        app.start_account(base);
        let (events, received) = mpsc::channel();
        let running = Running {
            events: received,
            cancel: CancelGuard::new(),
            work: pact_of("/repo/crates"),
            before: before.clone(),
        };
        (app, before, Manifest::new(), events, running)
    }

    #[test]
    fn a_pass_fills_the_panel_under_the_directory_it_is_working_on() {
        // The directory the worker names opens a section, everything the
        // pass is then seen doing lands under it one line at a time, a cost
        // is money rather than a line, and the footer goes on saying exactly
        // what it always said.
        let base = Instant::now();
        let (mut app, before, mut manifest, events, running) = a_run_in_flight(base);
        let mut pact = Some(running);

        // The first directory, on its own: the run has reached it and the
        // pass has not said anything yet.
        events
            .send(PactEvent::Starting {
                directory: PathBuf::from("/repo/crates/engine"),
                position: 1,
                total: 2,
            })
            .expect("the loop is still listening");
        apply_progress(&mut pact, &mut app, &mut manifest, &nowhere(), base);

        assert!(pact.is_some(), "a run that is talking is still running");
        assert_eq!(
            panel_text(&app, at(base, 2)),
            ["engine", "0:02 waiting"],
            "the section is open, and its clock ticks before the pass says anything"
        );
        let mut in_flight = before.clone();
        in_flight.set_pact_in_flight("/repo/crates/engine", 1, 2);
        assert_eq!(
            app.pact_line(),
            in_flight.pact_line(),
            "and the footer's progress line says what it always said"
        );
        assert_eq!(
            app.message(),
            in_flight.message(),
            "with no message of its own"
        );

        // Then what the pass is doing, four seconds in.
        for event in [
            PactEvent::Doing(Activity::Tool {
                name: "Bash".to_owned(),
                detail: Some("cargo test".to_owned()),
            }),
            PactEvent::Doing(Activity::Thinking),
        ] {
            events.send(event).expect("the loop is still listening");
        }
        apply_progress(&mut pact, &mut app, &mut manifest, &nowhere(), at(base, 4));

        assert_eq!(
            panel_text(&app, at(base, 4)),
            ["engine", "0:04 Bash cargo test", "0:04 thinking"],
            "one line per activity, under the directory that reported it"
        );

        // A cost is not a thing the pass did, so it draws no line — it is
        // added to what this directory spent.
        events
            .send(PactEvent::Doing(Activity::Cost { usd: 0.21 }))
            .expect("the loop is still listening");
        apply_progress(&mut pact, &mut app, &mut manifest, &nowhere(), at(base, 6));

        assert_eq!(
            panel_text(&app, at(base, 6)).len(),
            3,
            "the cost added no line: {:?}",
            panel_text(&app, at(base, 6))
        );
        assert_eq!(
            app.account()
                .expect("a run is under way")
                .sections()
                .first()
                .and_then(Section::cost),
            Some(0.21),
            "it was counted instead",
        );

        // The newest line counts up on its own, with nothing arriving: the
        // same app, a later instant, a moving clock. This is what the loop's
        // hundred-millisecond round does for a pass that thinks for a
        // minute.
        assert_eq!(
            panel_text(&app, at(base, 65)),
            ["engine", "0:04 Bash cargo test", "1:05 thinking"],
            "the line beneath the newest one is frozen and the newest is not"
        );
    }

    #[test]
    fn a_subtree_pact_reads_as_a_section_per_directory_in_walk_order() {
        // A pass each for two directories: the second heading opens under
        // the first section rather than over it, its clock starts again at
        // nothing, and the account is still whole once the run is over.
        let base = Instant::now();
        let (mut app, before, mut manifest, events, running) = a_run_in_flight(base);
        let mut pact = Some(running);

        events
            .send(PactEvent::Starting {
                directory: PathBuf::from("/repo/crates/engine"),
                position: 1,
                total: 2,
            })
            .expect("the loop is still listening");
        apply_progress(&mut pact, &mut app, &mut manifest, &nowhere(), base);
        events
            .send(PactEvent::Doing(Activity::Tool {
                name: "Bash".to_owned(),
                detail: Some("cargo test".to_owned()),
            }))
            .expect("the loop is still listening");
        apply_progress(&mut pact, &mut app, &mut manifest, &nowhere(), at(base, 4));

        // The next directory. Its clock starts again at nothing, and the
        // section above it stops where the run left it — seventy seconds in,
        // which is where its last line stays however long the run goes on.
        events
            .send(PactEvent::Starting {
                directory: PathBuf::from("/repo/crates/tui"),
                position: 2,
                total: 2,
            })
            .expect("the loop is still listening");
        apply_progress(&mut pact, &mut app, &mut manifest, &nowhere(), at(base, 70));
        events
            .send(PactEvent::Doing(Activity::Thinking))
            .expect("the loop is still listening");
        apply_progress(&mut pact, &mut app, &mut manifest, &nowhere(), at(base, 71));

        assert_eq!(
            panel_text(&app, at(base, 100)),
            ["engine", "1:10 Bash cargo test", "tui", "0:30 thinking"],
            "the sections are in walk order and each clock counts from its own start"
        );
        let mut in_flight = before.clone();
        in_flight.set_pact_in_flight("/repo/crates/tui", 2, 2);
        assert_eq!(
            app.pact_line(),
            in_flight.pact_line(),
            "and the footer is still the footer"
        );

        // And the outcome behind them lands exactly as it always did.
        events
            .send(PactEvent::Finished(Ok(Toggled {
                manifest: Manifest::new(),
                granted: true,
                message: None,
                refusals: Vec::new(),
            })))
            .expect("the loop is still listening");
        apply_progress(
            &mut pact,
            &mut app,
            &mut manifest,
            &nowhere(),
            at(base, 120),
        );

        assert!(pact.is_none(), "the run is over");
        assert!(app.pact_line().is_none(), "so nothing is being pacted now");
        // Nothing the run did is on the footer, because nothing went wrong
        // in it. The line that is there belongs to the reload at the bottom
        // of the call, which has no `/repo/crates` on disk to read.
        assert!(
            app.message()
                .is_some_and(|line| line.starts_with(NOT_REFRESHED)),
            "the run reported something of its own: {:?}",
            app.message()
        );
        // The footer stops describing a run that is over; the panel does
        // not. Everything the run said is still there to be read, with each
        // section ended and the run summed up under them. Nothing is on
        // disk at `/repo/crates`, so neither directory has a document to
        // point at and both sections say so rather than claim one.
        assert_eq!(
            panel_text(&app, at(base, 200)),
            [
                "engine",
                "1:10 Bash cargo test",
                "1:10 refused — no document was written",
                "tui",
                "0:50 thinking",
                "0:50 refused — no document was written",
                "pact finished — 2 directories, 2:00, $0.00 \
                     (incomplete: 2 passes reported no cost)",
            ],
            "the account of a finished run stays whole and is closed off"
        );
    }

    #[test]
    fn a_documented_directory_turns_green_the_moment_the_engine_says_so() {
        // The engine's word that a directory and everything under it
        // delivered recolours that subtree there and then, mid-run: a run
        // three directories long shows one green, one flashing and one
        // still yellow, rather than a wall of yellow until the batch is
        // over.
        let stale = NodeState::PactedStale;
        let tree = Tree::new(
            Node::new("/repo/crates", None::<PathBuf>, stale).with_children([
                Node::new("/repo/crates/engine", None::<PathBuf>, stale)
                    .with_children([Node::new("/repo/crates/engine/src", None::<PathBuf>, stale)]),
                Node::new("/repo/crates/tui", None::<PathBuf>, stale),
            ]),
        );
        let mut app = App::from_tree(&tree);
        let base = Instant::now();
        app.start_account(base);
        let mut manifest = Manifest::new();
        let (events, received) = mpsc::channel();
        let mut pact = Some(Running {
            events: received,
            cancel: CancelGuard::new(),
            work: pact_of("/repo/crates"),
            before: app.clone(),
        });

        // The deepest directory's pass delivers, and the run moves on to the
        // one above it without ending.
        for event in [
            PactEvent::Starting {
                directory: PathBuf::from("/repo/crates/engine/src"),
                position: 1,
                total: 3,
            },
            PactEvent::Documented {
                directory: PathBuf::from("/repo/crates/engine/src"),
            },
            PactEvent::Starting {
                directory: PathBuf::from("/repo/crates/engine"),
                position: 2,
                total: 3,
            },
        ] {
            events.send(event).expect("the loop is still listening");
        }
        apply_progress(&mut pact, &mut app, &mut manifest, &nowhere(), base);

        assert!(pact.is_some(), "the run is still going");
        assert_eq!(
            state_of(&app, Path::new("/repo/crates/engine/src")),
            Some(NodeState::PactedFresh),
            "the finished directory is green while the run pays for the rest"
        );
        for still in ["/repo/crates", "/repo/crates/engine", "/repo/crates/tui"] {
            assert_eq!(
                state_of(&app, Path::new(still)),
                Some(stale),
                "{still} has not delivered, so it keeps the keypress's yellow"
            );
        }
    }

    /// A subtree whose middle directory has a listing, for the tests about the
    /// document row a run writes into the tree as it goes.
    ///
    /// The listing is what makes the position assertable: `WARLOCK.md` sorts
    /// between `Cargo.toml` and `build.rs` — paths compare component by
    /// component, and a capital sorts before a lowercase — so a row spliced in
    /// where a fresh load would put it lands *between* the two rather than at
    /// either end of the run.
    fn one_directory_with_files(state: NodeState) -> Tree {
        Tree::new(
            Node::new("/repo/crates", None::<PathBuf>, state).with_children([
                Node::new("/repo/crates/engine", None::<PathBuf>, state)
                    .with_files(
                        [
                            "/repo/crates/engine/Cargo.toml",
                            "/repo/crates/engine/build.rs",
                        ]
                        .map(PathBuf::from),
                    )
                    .with_children([Node::new("/repo/crates/engine/src", None::<PathBuf>, state)]),
                Node::new("/repo/crates/tui", None::<PathBuf>, state),
            ]),
        )
    }

    /// A run of `work` over `app` that nobody has said anything to yet, and the
    /// end of the channel a test sends its events down.
    fn running_over(app: &App, work: Work) -> (Sender<PactEvent>, Option<Running>) {
        let (events, received) = mpsc::channel();
        let running = Running {
            events: received,
            cancel: CancelGuard::new(),
            work,
            before: app.clone(),
        };
        (events, Some(running))
    }

    /// Every row the app is drawing, as a path and the state it is drawn in.
    fn drawn(app: &App) -> Vec<(PathBuf, NodeState)> {
        app.rows()
            .iter()
            .map(|row| (row.path.clone(), row.state))
            .collect()
    }

    #[test]
    fn the_document_a_pass_wrote_appears_under_its_directory_as_the_run_goes() {
        // The engine's word that a directory delivered is also the news that
        // there is a `WARLOCK.md` beside it now. The row for it goes in there
        // and then, in the colour the same event just gave the directory,
        // where a fresh load would have put it.
        let stale = NodeState::PactedStale;
        let mut app = App::from_tree(&one_directory_with_files(stale));
        // The tally the recolouring moves between states but nothing moves the
        // size of: a document row stands for no node and is counted nowhere.
        let nodes = app.counts().total();
        let mut manifest = Manifest::new();
        let (events, mut pact) = running_over(&app, pact_of("/repo/crates"));
        let on_screen: Vec<_> = app.rows().iter().map(|row| row.path.clone()).collect();

        events
            .send(PactEvent::Documented {
                directory: PathBuf::from("/repo/crates/engine"),
            })
            .expect("the loop is still listening");
        apply_progress(
            &mut pact,
            &mut app,
            &mut manifest,
            &nowhere(),
            Instant::now(),
        );

        assert!(pact.is_some(), "the run is still going");
        // Files are hidden, so the reader's tree has not moved a row — the
        // insertion went into the walk and waits there for the toggle.
        assert_eq!(
            app.rows()
                .iter()
                .map(|row| row.path.clone())
                .collect::<Vec<_>>(),
            on_screen,
            "the same rows in the same order: nothing new is drawn"
        );
        assert_eq!(
            app.counts().total(),
            nodes,
            "a file row is counted nowhere, so the tally cannot have grown"
        );

        app.toggle_files();
        assert_eq!(
            drawn(&app),
            [
                ("/repo/crates", stale),
                ("/repo/crates/engine", NodeState::PactedFresh),
                ("/repo/crates/engine/Cargo.toml", NodeState::PactedFresh),
                ("/repo/crates/engine/WARLOCK.md", NodeState::PactedFresh),
                ("/repo/crates/engine/build.rs", NodeState::PactedFresh),
                ("/repo/crates/engine/src", NodeState::PactedFresh),
                ("/repo/crates/tui", stale),
            ]
            .map(|(path, state)| (PathBuf::from(path), state)),
            "the document is among its directory's files, in path order and \
             before the subdirectory, in the green the run just painted"
        );
    }

    #[test]
    fn a_second_document_for_the_same_directory_inserts_nothing() {
        // A re-pact of an already documented directory says `Documented`
        // about a `WARLOCK.md` that is already a row. One row per path, so
        // the second announcement changes nothing at all.
        let mut app = App::from_tree(&one_directory_with_files(NodeState::PactedStale));
        app.toggle_files();
        let mut manifest = Manifest::new();
        let (events, mut pact) = running_over(&app, pact_of("/repo/crates"));

        for _ in 0..2 {
            events
                .send(PactEvent::Documented {
                    directory: PathBuf::from("/repo/crates/engine"),
                })
                .expect("the loop is still listening");
        }
        apply_progress(
            &mut pact,
            &mut app,
            &mut manifest,
            &nowhere(),
            Instant::now(),
        );

        assert!(pact.is_some(), "the run is still going");
        assert_eq!(
            app.rows()
                .iter()
                .filter(|row| row.path == Path::new("/repo/crates/engine/WARLOCK.md"))
                .count(),
            1,
            "the document has exactly one row however often it is announced"
        );
    }

    #[test]
    fn a_documented_directory_with_no_row_changes_nothing() {
        // The event names a directory the tree on screen knows nothing about
        // — a run over a subtree the reader has since loaded away from. There
        // is nothing to hang a document row on, and nothing happens.
        let stale = NodeState::PactedStale;
        let mut app = App::from_tree(&one_directory_with_files(stale));
        app.toggle_files();
        let before = drawn(&app);
        let counts = app.counts();
        let mut manifest = Manifest::new();
        let (events, mut pact) = running_over(&app, pact_of("/repo/crates"));

        events
            .send(PactEvent::Documented {
                directory: PathBuf::from("/repo/docs/adr"),
            })
            .expect("the loop is still listening");
        apply_progress(
            &mut pact,
            &mut app,
            &mut manifest,
            &nowhere(),
            Instant::now(),
        );

        assert!(pact.is_some(), "the run is still going");
        assert_eq!(
            drawn(&app),
            before,
            "no row stands for that directory, so none is added under it"
        );
        assert_eq!(app.counts(), counts, "and the tally is where it was");
    }

    #[test]
    fn the_document_rows_a_run_writes_cost_no_reload() {
        // Every directory of the run delivers, one event at a time, and the
        // rows for their documents pile up in a tree that is nowhere on disk.
        // A reload from `nowhere()` would fail and say so on the footer, and a
        // reload that somehow worked would have thrown these rows away — so a
        // tree still holding them, with nothing on the message line and the
        // run still in flight, is the whole of "nothing was re-read".
        let stale = NodeState::PactedStale;
        let mut app = App::from_tree(&one_directory_with_files(stale));
        app.toggle_files();
        let mut manifest = Manifest::new();
        let (events, mut pact) = running_over(&app, pact_of("/repo/crates"));

        for directory in ["/repo/crates/engine/src", "/repo/crates/engine"] {
            events
                .send(PactEvent::Documented {
                    directory: PathBuf::from(directory),
                })
                .expect("the loop is still listening");
        }
        apply_progress(
            &mut pact,
            &mut app,
            &mut manifest,
            &nowhere(),
            Instant::now(),
        );

        assert!(pact.is_some(), "the run is still going");
        assert!(
            app.message().is_none(),
            "no reload was attempted, so no reload failed: {:?}",
            app.message()
        );
        assert_eq!(
            drawn(&app),
            [
                ("/repo/crates", stale),
                ("/repo/crates/engine", NodeState::PactedFresh),
                ("/repo/crates/engine/Cargo.toml", NodeState::PactedFresh),
                ("/repo/crates/engine/WARLOCK.md", NodeState::PactedFresh),
                ("/repo/crates/engine/build.rs", NodeState::PactedFresh),
                ("/repo/crates/engine/src", NodeState::PactedFresh),
                ("/repo/crates/engine/src/WARLOCK.md", NodeState::PactedFresh),
                ("/repo/crates/tui", stale),
            ]
            .map(|(path, state)| (PathBuf::from(path), state)),
            "both documents are on screen, each under its own directory"
        );
    }

    #[test]
    fn the_reader_keeps_the_tree_while_the_panel_fills_up() {
        // The run writes to the panel and to nothing else. The reader moves,
        // collapses and filters throughout, and the selection stays where
        // they left it rather than chasing the directory being pacted.
        // Everything is pacted, so the pacted-only filter keeps every row
        // and what it does to the selection is nothing at all.
        let pacted = NodeState::PactedFresh;
        let tree = Tree::new(Node::new("/repo", None::<PathBuf>, pacted).with_children([
            Node::new("/repo/crates", None::<PathBuf>, pacted).with_children([
                Node::new("/repo/crates/engine", None::<PathBuf>, pacted),
                Node::new("/repo/crates/tui", None::<PathBuf>, pacted),
            ]),
            Node::new("/repo/docs", None::<PathBuf>, pacted).with_children([Node::new(
                "/repo/docs/adr",
                None::<PathBuf>,
                pacted,
            )]),
        ]));
        let mut app = App::from_tree(&tree);
        let base = Instant::now();
        app.set_viewport_height(10);
        app.start_account(base);
        let mut manifest = Manifest::new();
        let (events, received) = mpsc::channel();
        let mut pact = Some(Running {
            events: received,
            cancel: CancelGuard::new(),
            work: pact_of("/repo/crates"),
            before: app.clone(),
        });

        // The reader parks on `docs`, which is nowhere near the subtree
        // being pacted.
        app.select_last();
        app.select_previous();
        let parked = app
            .selected_row()
            .map(|row| row.path.clone())
            .expect("a row is selected");
        assert_eq!(parked, PathBuf::from("/repo/docs"));

        // The first directory, and then the three keys that shape the tree.
        let mut round = |now: Instant, directory: &str, position: usize, app: &mut App| {
            events
                .send(PactEvent::Starting {
                    directory: PathBuf::from(directory),
                    position,
                    total: 2,
                })
                .expect("the loop is still listening");
            events
                .send(PactEvent::Doing(Activity::Thinking))
                .expect("the loop is still listening");
            apply_progress(&mut pact, app, &mut manifest, &nowhere(), now);
        };

        round(base, "/repo/crates/engine", 1, &mut app);
        app.toggle_collapsed();
        app.toggle_pacted_only();
        app.toggle_files();

        // The second, and then the movement keys, which end where they
        // started because the reader put them back.
        round(at(base, 10), "/repo/crates/tui", 2, &mut app);
        app.select_previous();
        app.select_next();

        assert!(pact.is_some(), "the run is still going");
        assert_eq!(
            panel_text(&app, at(base, 10)),
            ["engine", "0:10 thinking", "tui", "0:00 thinking"],
            "the run filled the panel and nothing else"
        );
        assert_eq!(
            app.selected_row().map(|row| row.path.clone()),
            Some(parked),
            "the selection followed the run"
        );
        assert!(app.pacted_only(), "the filter key did nothing");
        assert!(app.show_files(), "the file key did nothing");
        assert!(
            app.is_collapsed("/repo/docs"),
            "the collapse key did nothing"
        );
    }

    #[test]
    fn a_run_that_dies_leaves_the_account_of_what_it_managed_on_screen() {
        // Putting the tree back where it was is the undo for a run that
        // recorded nothing, and it is taken from a copy older than the run
        // itself. The account is not part of that undo: the lines are the
        // record of a pass that really did happen, and this is the run whose
        // reader most wants to see where it got to.
        let tree = Tree::new(Node::new(
            "/repo/crates",
            None::<PathBuf>,
            NodeState::Unpacted,
        ));
        let before = App::from_tree(&tree);
        let mut app = before.clone();
        let base = Instant::now();
        app.start_account(base);
        let mut manifest = Manifest::new();
        let (events, received) = mpsc::channel();
        let mut pact = Some(Running {
            events: received,
            cancel: CancelGuard::new(),
            work: pact_of("/repo/crates"),
            before: before.clone(),
        });

        events
            .send(PactEvent::Starting {
                directory: PathBuf::from("/repo/crates/engine"),
                position: 1,
                total: 2,
            })
            .expect("the loop is still listening");
        events
            .send(PactEvent::Doing(Activity::Thinking))
            .expect("the loop is still listening");
        apply_progress(&mut pact, &mut app, &mut manifest, &nowhere(), base);

        // The worker goes away without an outcome behind it.
        drop(events);
        apply_progress(&mut pact, &mut app, &mut manifest, &nowhere(), at(base, 5));

        assert!(pact.is_none(), "the run is over, however it ended");
        assert_eq!(app.message(), Some(PACT_LOST), "and the footer says so");
        assert_eq!(app.rows(), before.rows(), "the rows match the manifest");
        assert_eq!(
            panel_text(&app, at(base, 5)),
            [
                "engine",
                "0:05 thinking",
                "0:05 refused — no document was written",
                "pact finished — 1 directory, 0:05, $0.00 \
                     (incomplete: 1 pass reported no cost)",
            ],
            "and the panel still holds what the run said before it died"
        );
        assert_eq!(
            panel_text(&app, at(base, 500)),
            panel_text(&app, at(base, 5)),
            "with its clocks stopped where the run stopped"
        );
    }

    /// How long a frame lasts in the replayed runs below, in seconds.
    ///
    /// Ten rather than one so the clocks read as something a reader of the
    /// test can check by counting frames, and so a section's lines are told
    /// apart by their clocks rather than all reading `0:00`.
    const FRAME: u64 = 10;

    /// Everything one whole run has to say, from the worker's own body run
    /// on this thread over `scratch`.
    ///
    /// The announcements, whatever the passes report, and the outcome, in
    /// the order a real run produces them: `agent` is handed the very sender
    /// the run reports on, so the port it puts on its fake is the one
    /// [`spawn_pact`] would put on a real agent, over the one channel.
    fn recorded(
        scratch: &Scratch,
        relative: &str,
        cancel: &Cancel,
        agent: impl FnOnce(&Sender<PactEvent>) -> Canned,
    ) -> Vec<PactEvent> {
        recorded_from(
            scratch,
            &Manifest::new(),
            &toggle(scratch, relative, true),
            cancel,
            agent,
        )
    }

    /// The same, for a run that starts from a manifest rather than from
    /// nothing: what a refresh needs, since staleness is decided against it.
    fn recorded_from(
        scratch: &Scratch,
        manifest: &Manifest,
        work: &Work,
        cancel: &Cancel,
        agent: impl FnOnce(&Sender<PactEvent>) -> Canned,
    ) -> Vec<PactEvent> {
        let (events, received) = mpsc::channel();
        let agent = agent(&events);
        run_pact(manifest, &scratch.root, work, &agent, cancel, &events);
        // Both ends the worker would have held: its own, and the one inside
        // the port attached to its agent.
        drop(events);
        drop(agent);
        received.into_iter().collect()
    }

    /// Play `said` back at the event loop a frame at a time, ten seconds a
    /// frame, and leave the app holding what the run put in the panel.
    ///
    /// One event per frame because a run collected in a millisecond and
    /// drained in a single call would be one instant with every clock
    /// reading `0:00`, and what each section's clock says is half of what
    /// the panel is for. The other half is the outcome under each section,
    /// which lands in the last frame of all — the one carrying
    /// [`PactEvent::Finished`].
    fn replay(
        app: &mut App,
        manifest: &mut Manifest,
        scope: &Scope,
        cancel: CancelGuard,
        said: Vec<PactEvent>,
        base: Instant,
    ) {
        replay_work(
            app,
            manifest,
            scope,
            cancel,
            &pact_of(scope.root.clone()),
            said,
            base,
        );
    }

    /// The same, for a run that is not a pact of the whole tree, keeping what
    /// the footer's progress line said as it went.
    ///
    /// The line comes back with consecutive repeats dropped, because it is
    /// re-worded every frame and says the same thing until the next directory
    /// starts: what a test of it is about is what the reader saw change.
    fn replay_work(
        app: &mut App,
        manifest: &mut Manifest,
        scope: &Scope,
        cancel: CancelGuard,
        work: &Work,
        said: Vec<PactEvent>,
        base: Instant,
    ) -> Vec<String> {
        let (events, received) = mpsc::channel();
        let mut pact = Some(Running {
            events: received,
            cancel,
            work: work.clone(),
            before: app.clone(),
        });
        let mut progress: Vec<String> = Vec::new();
        for (frame, event) in said.into_iter().enumerate() {
            let frame = u64::try_from(frame).expect("a run of fewer than 2^64 events");
            events.send(event).expect("the loop is still listening");
            apply_progress(&mut pact, app, manifest, scope, at(base, frame * FRAME));
            if let Some(line) = app.pact_line()
                && progress.last() != Some(&line)
            {
                progress.push(line);
            }
        }
        assert!(pact.is_none(), "the run reported its outcome and is over");
        progress
    }

    /// How big the document under `relative` is, read the way the panel
    /// reads it: off disk, where the engine wrote it.
    fn document_bytes(scratch: &Scratch, relative: &str) -> u64 {
        fs::metadata(scratch.path(relative).join(DOCUMENT_FILE))
            .expect("the pass wrote a document")
            .len()
    }

    #[test]
    fn each_section_ends_with_the_document_that_pass_wrote_and_what_it_cost() {
        // A whole run, from the worker's body to the panel: two directories,
        // each with a pass that reads something, thinks, and says what it
        // spent. What closes each section is a fact about disk — the
        // document at `<directory>/WARLOCK.md` and its size — and a fact
        // about the pass, which is the cost the activity port carried.
        let scratch = one_crate_to_load("outcomes");
        let (mut app, scope) = load(&scratch);
        let mut manifest = Manifest::new();
        let base = Instant::now();
        app.start_account(base);

        let said = recorded(&scratch, "crates/engine", &Cancel::new(), |events| {
            Canned::new(&scratch, []).reporting(activity_port(events))
        });
        replay(
            &mut app,
            &mut manifest,
            &scope,
            CancelGuard::new(),
            said,
            base,
        );

        // Read long after the run ended, which is also how it is shown that
        // every clock stopped: a finished account says the same thing for
        // ever.
        assert_eq!(
            panel_text(&app, at(base, 10_000)),
            [
                "crates/engine/src".to_owned(),
                "0:20 Read crates/engine/src".to_owned(),
                "0:50 thinking".to_owned(),
                format!(
                    "0:50 wrote crates/engine/src/WARLOCK.md — {} bytes, $0.25",
                    document_bytes(&scratch, "crates/engine/src")
                ),
                "crates/engine".to_owned(),
                "0:20 Read crates/engine".to_owned(),
                "0:50 thinking".to_owned(),
                format!(
                    "0:50 wrote crates/engine/WARLOCK.md — {} bytes, $0.25",
                    document_bytes(&scratch, "crates/engine")
                ),
                // Eleven frames of ten seconds: the run started with the
                // first and ended with the eleventh. The total is the two
                // passes added up, and there is no `incomplete` on it because
                // both of them said what they cost.
                "pact finished — 2 directories, 1:40, $0.50".to_owned(),
            ],
            "each section is closed with its own document and its own cost"
        );

        // And the account of a run that is over is still all there and still
        // all reachable: three lines of panel, and the reader can walk the
        // nine the run wrote a screenful at a time.
        app.toggle_focus();
        app.set_panel_height(3);
        app.select_first();
        let mut walked = Vec::new();
        loop {
            walked.extend(app.panel_lines(at(base, 10_000)));
            if app.panel_lines_below() == 0 {
                break;
            }
            app.select_page_down();
        }
        assert_eq!(
            walked,
            app.account()
                .expect("the run left one")
                .lines(at(base, 10_000)),
            "every line of the run is still reachable through a short panel"
        );
    }

    #[test]
    fn a_directory_the_run_refused_ends_with_the_reason_it_was_refused() {
        // One directory of the two comes back with an answer the engine will
        // not take. The section for that directory says why, in the words of
        // the failure that named it, and the directory that did write a
        // document is untouched by its neighbour going wrong.
        let scratch = one_crate_to_load("refused-in-the-panel");
        let (mut app, scope) = load(&scratch);
        let mut manifest = Manifest::new();
        let base = Instant::now();
        app.start_account(base);

        let said = recorded(&scratch, "crates/engine", &Cancel::new(), |events| {
            Canned::new(&scratch, ["crates/engine/src"]).reporting(activity_port(events))
        });
        replay(
            &mut app,
            &mut manifest,
            &scope,
            CancelGuard::new(),
            said,
            base,
        );

        let lines = panel_text(&app, at(base, 10_000));
        let [first, _, _, refused, second, _, _, wrote, summary] = lines.as_slice() else {
            panic!("a two-directory run reads as two sections and a summary: {lines:?}");
        };
        assert_eq!(first, "crates/engine/src");
        assert_eq!(second, "crates/engine");

        let reason = refused
            .strip_prefix("0:40 refused — ")
            .unwrap_or_else(|| panic!("the section says why it was refused: {refused}"));
        assert!(
            reason.contains("crates/engine/src"),
            "and the reason is about that directory: {reason}"
        );
        // The engine's own sentence rather than a second opinion worded
        // here: the footer quotes the same failure, and this is the way to
        // say that the two came out of one place.
        let message = app.message().expect("a partial run reports it");
        assert!(
            message.contains(reason),
            "the panel's reason is the failure's own: {message} / {reason}"
        );

        // Nothing was refused about the parent, which was documented over
        // its failing child, and its section says so in the ordinary way.
        assert_eq!(
            wrote,
            &format!(
                "0:40 wrote crates/engine/WARLOCK.md — {} bytes, $0.25",
                document_bytes(&scratch, "crates/engine")
            )
        );
        assert!(
            !scratch
                .path("crates/engine/src")
                .join(DOCUMENT_FILE)
                .exists(),
            "the refused directory really has no document"
        );
        assert_eq!(summary, "pact finished — 2 directories, 1:20, $0.50");
    }

    #[test]
    fn a_cancelled_run_says_so_in_the_section_it_was_stopped_in() {
        // Esc during the fourth pass of five. The section being worked when
        // it landed says the run was stopped and what it had spent by then —
        // whatever that pass got as far as answering, because a real cancel
        // kills the `claude` it was waiting on — and the three sections
        // above it keep the endings they earned.
        let scratch = two_crates("cancelled-in-the-panel");
        scratch.write(".git/HEAD", "ref: refs/heads/main\n");
        let (mut app, scope) = load(&scratch);
        let mut manifest = Manifest::new();
        let base = Instant::now();
        app.start_account(base);

        // The event loop's own handle, and the clone the run answers to:
        // exactly the pair `spawn_pact` makes, so what the panel reads at
        // the end is the flag the reader's key latched.
        let guard = CancelGuard::new();
        let cancel = guard.handle();
        let said = recorded(&scratch, "crates", &cancel, |events| {
            Canned::new(&scratch, [])
                .reporting(activity_port(events))
                .cancelling_at("crates/alpha", cancel.clone())
        });
        assert!(cancel.is_cancelled(), "the reader stopped this run");
        replay(&mut app, &mut manifest, &scope, guard, said, base);

        let lines = panel_text(&app, at(base, 10_000));
        assert_eq!(
            lines.len(),
            17,
            "four sections of four lines and a summary: {lines:?}"
        );
        assert_eq!(
            &lines[12..],
            [
                "crates/alpha".to_owned(),
                "0:20 Read crates/alpha".to_owned(),
                "0:50 thinking".to_owned(),
                "0:50 cancelled — $0.25 spent".to_owned(),
                // Four directories and not the five the subtree holds: the
                // descent stopped, and the account counts what it reached.
                "pact finished — 4 directories, 3:20, $1.00".to_owned(),
            ],
            "the cancel is recorded in the section it happened in"
        );
        // Everything above it is exactly what a finished pass leaves.
        for (index, directory) in ["crates/beta/src", "crates/beta", "crates/alpha/src"]
            .into_iter()
            .enumerate()
        {
            assert_eq!(&lines[index * 4], directory);
            assert_eq!(
                lines[index * 4 + 3],
                format!(
                    "0:50 wrote {directory}/WARLOCK.md — {} bytes, $0.25",
                    document_bytes(&scratch, directory)
                ),
                "a section above the cancel keeps the ending it earned"
            );
        }
        // And the footer says what it has always said about a stopped run.
        assert_eq!(app.message(), Some(PACT_CANCELLED));
    }

    /// A repository of eight crates, sixteen directories under the one they
    /// sit in.
    ///
    /// Big enough for a refresh to be visibly a run over part of it: the
    /// fraction the reader is shown has to be able to disagree with the size
    /// of the subtree for the test of it to say anything.
    fn eight_crates(name: &str) -> Scratch {
        let scratch = Scratch::new(name);
        for crate_name in CRATES {
            scratch.write(
                &format!("crates/{crate_name}/src/lib.rs"),
                "//! A crate of its own.\n",
            );
        }
        scratch.write(".git/HEAD", "ref: refs/heads/main\n");
        scratch
    }

    /// The crates [`eight_crates`] writes, named so that reverse path order —
    /// which is the order the engine descends in — is `c8` first and `c1`
    /// last.
    const CRATES: [&str; 8] = ["c1", "c2", "c3", "c4", "c5", "c6", "c7", "c8"];

    #[test]
    fn a_refresh_counts_what_it_will_visit_rather_than_the_subtree_it_was_pointed_at() {
        // Seventeen directories pacted, three files then moved, and a refresh
        // of the lot: seven directories are stale — three that hold a moved
        // file, their three parents, and the root above all of them — and
        // seven is what the reader is counted to.
        let scratch = eight_crates("refresh-fraction");
        let pacted_once = pacted(&scratch, "crates");
        assert_eq!(
            pacted_once.entries().len(),
            17,
            "the subtree the key is pressed on is much bigger than the run"
        );
        for crate_name in ["c1", "c2", "c3"] {
            scratch.write(
                &format!("crates/{crate_name}/src/lib.rs"),
                "//! A crate of its own, and something new.\n",
            );
        }

        let (mut app, scope) = load(&scratch);
        let mut manifest = pacted_once.clone();
        let base = Instant::now();
        // Everything the press does before the worker starts, which is what
        // `refresh_press` does for real: one account, opened at the press.
        app.start_account(base);

        let work = refreshing(&scratch, "crates");
        let said = recorded_from(&scratch, &pacted_once, &work, &Cancel::new(), |_| {
            Canned::new(&scratch, [])
        });
        let progress = replay_work(
            &mut app,
            &mut manifest,
            &scope,
            CancelGuard::new(),
            &work,
            said,
            base,
        );

        // The verb is the run's and the fraction is the engine observer's:
        // seven of seven, and not one of seventeen.
        assert_eq!(
            progress,
            [
                "refreshing crates/c3/src (1/7)",
                "refreshing crates/c3 (2/7)",
                "refreshing crates/c2/src (3/7)",
                "refreshing crates/c2 (4/7)",
                "refreshing crates/c1/src (5/7)",
                "refreshing crates/c1 (6/7)",
                "refreshing crates (7/7)",
            ],
            "the footer counted something other than what the run visited"
        );

        // The panel filled the same way, a section per directory the run
        // reached and none for a directory it skipped.
        let lines = panel_text(&app, at(base, 10_000));
        let stale = [
            "crates/c3/src",
            "crates/c3",
            "crates/c2/src",
            "crates/c2",
            "crates/c1/src",
            "crates/c1",
            "crates",
        ];
        let headings: Vec<&str> = lines
            .iter()
            .map(String::as_str)
            .filter(|line| stale.contains(line))
            .collect();
        assert_eq!(headings, stale, "a section per directory, in walk order");
        assert_eq!(
            lines.last().map(String::as_str),
            Some(
                "pact finished — 7 directories, 2:20, $0.00 (incomplete: 7 passes reported no cost)"
            ),
            "the summary counts the run: {lines:?}"
        );

        // And the run ended the ordinary way: the manifest it computed is on
        // disk, and every directory of the subtree — the described and the
        // skipped alike — is green again.
        assert_eq!(
            saved(&scratch.root).expect("the manifest was written"),
            manifest
        );
        for entry in manifest.entries() {
            let module = entry.module();
            assert_eq!(
                state_of(&app, &scratch.path(module)),
                Some(NodeState::PactedFresh),
                "`{module}` is not green after the refresh"
            );
        }
    }

    #[test]
    fn a_pass_that_never_said_what_it_cost_leaves_the_total_incomplete() {
        // The first pass's result line carries no cost — WAR-24 reports none
        // rather than a zero — and the port drops it on the floor exactly as
        // a result line without the field would. What the run spent is then
        // one pass's worth and the total says so, rather than adding a zero
        // that was never measured and under-reporting the run.
        let scratch = one_crate_to_load("no-cost");
        let (mut app, scope) = load(&scratch);
        let mut manifest = Manifest::new();
        let base = Instant::now();
        app.start_account(base);

        let said = recorded(&scratch, "crates/engine", &Cancel::new(), |events| {
            let events = events.clone();
            let swallowed = AtomicBool::new(false);
            let port = Activities::new(move |activity| {
                if matches!(activity, Activity::Cost { .. })
                    && !swallowed.swap(true, Ordering::Relaxed)
                {
                    return;
                }
                let _ = events.send(PactEvent::Doing(activity));
            });
            Canned::new(&scratch, []).reporting(port)
        });
        replay(
            &mut app,
            &mut manifest,
            &scope,
            CancelGuard::new(),
            said,
            base,
        );

        assert_eq!(
            panel_text(&app, at(base, 10_000)),
            [
                "crates/engine/src".to_owned(),
                "0:20 Read crates/engine/src".to_owned(),
                "0:40 thinking".to_owned(),
                format!(
                    "0:40 wrote crates/engine/src/WARLOCK.md — {} bytes, no cost reported",
                    document_bytes(&scratch, "crates/engine/src")
                ),
                "crates/engine".to_owned(),
                "0:20 Read crates/engine".to_owned(),
                "0:50 thinking".to_owned(),
                format!(
                    "0:50 wrote crates/engine/WARLOCK.md — {} bytes, $0.25",
                    document_bytes(&scratch, "crates/engine")
                ),
                "pact finished — 2 directories, 1:30, \
                     $0.25 (incomplete: 1 pass reported no cost)"
                    .to_owned(),
            ],
            "a pass that reported nothing is missing rather than free"
        );
    }

    #[test]
    fn a_worker_that_says_nothing_more_ends_the_run_rather_than_hanging() {
        // The one way the channel closes without an outcome: the worker
        // reports on every path it takes itself, so a silence is a panic in
        // it. The hook has already put the terminal back and printed what
        // happened; what is left to this thread is to stop describing a run
        // that is over, and above all not to wait for a thread that no
        // longer exists.
        let tree = Tree::new(Node::new(
            "/repo/crates",
            None::<PathBuf>,
            NodeState::Unpacted,
        ));
        let before = App::from_tree(&tree);
        let mut app = before.clone();
        let mut manifest = Manifest::new();
        let (events, received) = mpsc::channel();
        let mut pact = Some(Running {
            events: received,
            cancel: CancelGuard::new(),
            work: pact_of("/repo/crates"),
            before: before.clone(),
        });

        events
            .send(PactEvent::Starting {
                directory: PathBuf::from("/repo/crates"),
                position: 1,
                total: 1,
            })
            .expect("the loop is still listening");
        apply_progress(
            &mut pact,
            &mut app,
            &mut manifest,
            &nowhere(),
            Instant::now(),
        );

        assert!(pact.is_some(), "a run that has only started is still on");
        assert!(
            app.pact_line().is_some(),
            "and the footer says where it has got to"
        );

        // The worker's end goes away with no `Finished` behind it.
        drop(events);
        apply_progress(
            &mut pact,
            &mut app,
            &mut manifest,
            &nowhere(),
            Instant::now(),
        );

        assert!(pact.is_none(), "the run is over, however it ended");
        assert!(
            app.pact_line().is_none(),
            "so nothing is being pacted now: {:?}",
            app.pact_line()
        );
        assert_eq!(app.message(), Some(PACT_LOST), "and the footer says so");
        assert_eq!(
            manifest,
            Manifest::new(),
            "nothing new was recorded, because nothing came back to record"
        );
        // The rows are the ones the keystroke painted over, put back: the
        // same app, down to the message that is the only thing this path
        // adds to it.
        let mut restored = before;
        restored.set_message(PACT_LOST);
        assert_eq!(
            app, restored,
            "and the rows go back to matching the manifest"
        );
    }

    #[test]
    fn a_finished_pact_puts_the_documents_it_wrote_on_screen() {
        // The whole point of the reload, in one test: a run writes
        // `WARLOCK.md` into every directory of the subtree, and the app has
        // no way to know that except by reading the tree again. No key is
        // pressed here and nothing is relaunched — the frame after the
        // outcome shows them.
        let scratch = one_crate_to_load("reload-granted");
        let (mut app, scope) = load(&scratch);
        let mut manifest = Manifest::new();
        let agent = Canned::new(&scratch, []);
        let engine = scratch.path("crates/engine");

        // Files on, so a document that appears is a row that appears.
        app.toggle_files();
        assert_eq!(
            documents(&app, &scratch),
            Vec::<PathBuf>::new(),
            "nothing has been pacted yet, so there is nothing to show"
        );
        // What the pact key paints before the run starts: pacted, and not
        // yet proven fresh.
        app.set_subtree_state(&engine, NodeState::PactedStale);

        run_and_apply(
            &scratch,
            &mut app,
            &mut manifest,
            &scope,
            &toggle(&scratch, "crates/engine", true),
            &agent,
        );

        assert_eq!(
            documents(&app, &scratch),
            [
                PathBuf::from("crates/engine/WARLOCK.md"),
                PathBuf::from("crates/engine/src/WARLOCK.md"),
            ],
            "the documents the run wrote are rows in the tree"
        );
        // And they are green, because the tree they came back in was built
        // from the manifest the run had already saved.
        for relative in ["crates/engine", "crates/engine/src"] {
            assert_eq!(
                state_of(&app, &scratch.path(relative)),
                Some(NodeState::PactedFresh),
                "{relative} did not come back fresh"
            );
        }
        assert_eq!(app.message(), None, "and nothing went wrong to report");
    }

    #[test]
    fn a_reload_leaves_the_reader_exactly_where_they_were() {
        // A tree that collapses to the root and throws the selection to the
        // top every time a pact ends is worse than one that never updates,
        // so: collapse something, filter, scroll, select — then end a pact
        // that really does change the tree, and none of the five moves.
        let scratch = one_crate_to_load("reload-place");
        scratch.write("crates/engine/tests/one.rs", "#[test] fn one() {}\n");
        scratch.write("crates/tui/src/main.rs", "fn main() {}\n");
        scratch.write("docs/adr/one.md", "# One\n");

        // A pact that happened before this reader sat down, so there is
        // something for the pacted-only filter to keep.
        let agent = Canned::new(&scratch, []);
        let Toggled { mut manifest, .. } = apply_toggle(
            &Manifest::new(),
            &scratch.root,
            &toggle(&scratch, "crates", true),
            &agent,
            &mut Unwatched,
        )
        .expect("a subtree that walks and a manifest that writes");

        let (mut app, scope) = load(&scratch);
        app.set_viewport_height(4);
        app.toggle_files();
        app.toggle_pacted_only();
        app = app.with_collapsed([scratch.path("crates/tui")]);
        for _ in 0..5 {
            app.select_next();
        }

        let selected = app
            .selected_row()
            .map(|row| row.path.clone())
            .expect("a row is selected");
        let collapsed = app.collapsed().clone();
        let offset = app.scroll_offset();
        assert!(offset > 0, "the reader scrolled off the first row");

        // A pact over a subtree nothing has touched yet: the reload really
        // does bring back a different tree, with `docs` pacted and two more
        // documents in it.
        run_and_apply(
            &scratch,
            &mut app,
            &mut manifest,
            &scope,
            &toggle(&scratch, "docs", true),
            &agent,
        );

        assert_eq!(
            state_of(&app, &scratch.path("docs")),
            Some(NodeState::PactedFresh),
            "the tree that came back is the new one"
        );
        assert_eq!(
            app.selected_row().map(|row| row.path.clone()),
            Some(selected),
            "the selection moved"
        );
        assert_eq!(app.collapsed(), &collapsed, "the collapsed set moved");
        assert!(app.pacted_only(), "the filter was dropped");
        assert!(app.show_files(), "the file toggle was dropped");
        assert_eq!(app.scroll_offset(), offset, "the window moved");
    }

    #[test]
    fn an_un_pact_reloads_the_tree_by_the_same_one_rule() {
        // One rule for every ending, so an un-pact re-reads too. Nothing in
        // `apply_progress`'s arms recolours anything for an un-pact — it
        // grants nothing and has nothing to say — so a subtree that comes
        // back unpacted here came back from disk.
        let scratch = one_crate_to_load("reload-unpact");
        let agent = Canned::new(&scratch, []);
        let Toggled { mut manifest, .. } = apply_toggle(
            &Manifest::new(),
            &scratch.root,
            &toggle(&scratch, "crates/engine", true),
            &agent,
            &mut Unwatched,
        )
        .expect("a subtree that walks and a manifest that writes");

        let (mut app, scope) = load(&scratch);
        assert_eq!(
            state_of(&app, &scratch.path("crates/engine")),
            Some(NodeState::PactedFresh),
            "the reader is looking at a pacted subtree"
        );

        run_and_apply(
            &scratch,
            &mut app,
            &mut manifest,
            &scope,
            &toggle(&scratch, "crates/engine", false),
            &agent,
        );

        assert!(
            manifest.entries().is_empty(),
            "the un-pact emptied the manifest"
        );
        for relative in ["crates/engine", "crates/engine/src"] {
            assert_eq!(
                state_of(&app, &scratch.path(relative)),
                Some(NodeState::Unpacted),
                "{relative} is still coloured by a manifest that no longer says so"
            );
        }
    }

    #[test]
    fn a_reload_that_will_not_load_keeps_the_tree_already_on_screen() {
        // Mid-session, a load that fails is not fatal and never was going to
        // be: warlock is up, the documents are on disk and the manifest is
        // saved, so quitting would throw away a run that cost minutes and
        // money. The arm's own result stands and the rows do not move.
        let tree = Tree::new(Node::new(
            "/repo/crates",
            None::<PathBuf>,
            NodeState::Unpacted,
        ));
        let before = App::from_tree(&tree);
        let mut app = before.clone();
        let mut manifest = Manifest::new();
        let (events, received) = mpsc::channel();
        let mut pact = Some(Running {
            events: received,
            cancel: CancelGuard::new(),
            work: pact_of("/repo/crates"),
            before: before.clone(),
        });

        events
            .send(PactEvent::Finished(Ok(Toggled {
                manifest: Manifest::new(),
                granted: true,
                message: None,
                refusals: Vec::new(),
            })))
            .expect("the loop is still listening");
        // There is no repository at `/repo/crates`, so the reload at the
        // bottom of this call fails.
        apply_progress(
            &mut pact,
            &mut app,
            &mut manifest,
            &nowhere(),
            Instant::now(),
        );

        assert!(pact.is_none(), "the run is over");
        assert_eq!(
            app.rows().len(),
            before.rows().len(),
            "the tree on screen was thrown away"
        );
        assert_eq!(
            state_of(&app, Path::new("/repo/crates")),
            Some(NodeState::PactedFresh),
            "the outcome the run did report was undone by a load that failed"
        );
        // And the reader is told why the rows did not move, on the one line
        // the footer has, with the load's own reason after it.
        let message = app.message().expect("a refresh that failed says so");
        assert!(
            message.starts_with(NOT_REFRESHED),
            "the footer says nothing about the refresh: {message}"
        );
        assert!(
            message.len() > NOT_REFRESHED.len() + 2,
            "the footer does not say why: {message}"
        );
        assert!(
            !message.contains('\n'),
            "a footer line that wraps is a footer line that hides a row: {message}"
        );
    }

    #[test]
    fn the_pacts_own_message_wins_over_the_reloads() {
        // Both have something to say and there is one line to say it on. The
        // run is the news — it is what the reader asked for, and it cost
        // minutes — so the reload's line waits for a footer nobody else
        // wanted. Both endings are driven here against the same failing
        // reload, so the only difference between them is whether the pact
        // left a message.
        const REFUSED: &str = "the manifest would not save";

        let ending = |message: Option<String>| {
            let tree = Tree::new(Node::new(
                "/repo/crates",
                None::<PathBuf>,
                NodeState::Unpacted,
            ));
            let before = App::from_tree(&tree);
            let mut app = before.clone();
            let mut manifest = Manifest::new();
            let (events, received) = mpsc::channel();
            let mut pact = Some(Running {
                events: received,
                cancel: CancelGuard::new(),
                work: pact_of("/repo/crates"),
                before,
            });

            events
                .send(PactEvent::Finished(Ok(Toggled {
                    manifest: Manifest::new(),
                    granted: true,
                    message,
                    refusals: Vec::new(),
                })))
                .expect("the loop is still listening");
            // Nothing is on disk at `/repo/crates`, so the reload has its
            // own line to offer in both cases.
            apply_progress(
                &mut pact,
                &mut app,
                &mut manifest,
                &nowhere(),
                Instant::now(),
            );
            app.message().map(str::to_owned)
        };

        assert_eq!(
            ending(Some(REFUSED.to_owned())).as_deref(),
            Some(REFUSED),
            "the reload talked over the run"
        );
        assert!(
            ending(None).is_some_and(|line| line.starts_with(NOT_REFRESHED)),
            "the reload said nothing into a footer nobody else was using"
        );
    }

    /// Only on unix, because the only way to make a load report problems
    /// rather than fail outright is a file the process may not read, and
    /// `chmod` is how that is arranged.
    #[cfg(unix)]
    #[test]
    fn a_reload_with_problems_takes_the_new_tree_and_counts_them() {
        use std::os::unix::fs::PermissionsExt as _;

        // A `Problem` is a per-node fact the engine has already coloured
        // conservatively — the node is stale and says so — so a tree with
        // the run's documents in it beats the stale one it replaces, and it
        // is taken. What the footer adds is the count, in the words the
        // startup load already uses for it.
        let scratch = one_crate_to_load("reload-problems");
        let agent = Canned::new(&scratch, []);
        let Toggled { mut manifest, .. } = apply_toggle(
            &Manifest::new(),
            &scratch.root,
            &toggle(&scratch, "crates/engine", true),
            &agent,
            &mut Unwatched,
        )
        .expect("a subtree that walks and a manifest that writes");

        let (mut app, scope) = load(&scratch);
        assert_eq!(
            state_of(&app, &scratch.path("crates/engine")),
            Some(NodeState::PactedFresh),
            "the reader is looking at a pacted subtree"
        );

        // Something new on disk for the reload to find, and something it
        // cannot read: `crates/engine` and `crates/engine/src` are both
        // pacted, and neither can be hashed with an unreadable file under
        // it, so there are two problems and one of them is counted.
        scratch.write("docs/adr/one.md", "# One\n");
        let unreadable = scratch.path("crates/engine/src/lib.rs");
        fs::set_permissions(&unreadable, fs::Permissions::from_mode(0o000)).expect("chmods");

        let (events, received) = mpsc::channel();
        let mut pact = Some(Running {
            events: received,
            cancel: CancelGuard::new(),
            work: pact_of(scratch.path("crates/engine")),
            before: app.clone(),
        });
        events
            .send(PactEvent::Finished(Ok(Toggled {
                manifest: manifest.clone(),
                granted: true,
                message: None,
                refusals: Vec::new(),
            })))
            .expect("the loop is still listening");
        apply_progress(&mut pact, &mut app, &mut manifest, &scope, Instant::now());

        let message = app.message().expect("problems are reported").to_owned();
        fs::set_permissions(&unreadable, fs::Permissions::from_mode(0o644)).expect("chmods back");

        assert!(
            state_of(&app, &scratch.path("docs")).is_some(),
            "the tree that came back is the old one, without the problems in it"
        );
        assert_eq!(
            state_of(&app, &scratch.path("crates/engine")),
            Some(NodeState::PactedStale),
            "a node with no hash is coloured as if it had one"
        );
        assert!(
            message.contains("could not be hashed"),
            "the footer does not say what went wrong: {message}"
        );
        assert!(
            message.contains("and 1 more like it"),
            "the footer does not say how many there were: {message}"
        );
        assert!(
            !message.contains('\n'),
            "a footer line that wraps is a footer line that hides a row: {message}"
        );
    }

    /// What the loop does when the disk moves under it: the reload a
    /// watcher earns, the one it holds back while a pact is running, and
    /// the one line it says when there is no watcher at all.
    ///
    /// No real watcher in any of these, and nothing waits on one. Which
    /// paths are Warlock's business and when a burst has settled are
    /// `watch.rs`'s two questions, answered there against instants nobody
    /// had to live through; what is left for here is the loop's own half —
    /// that a reload really happens, that it leaves the reader where they
    /// were, and that a run in flight defers it. So the policy is told an
    /// event was accepted rather than being handed one by an operating
    /// system, and [`Watching::Off`] stands in for the half that talks to
    /// one.
    mod watching {
        use std::time::{Duration, Instant};

        use warlock_engine::manifest_path;
        use warlock_tui::{QUIET_PERIOD, RELOAD_CEILING, WatchPolicy, Watching};

        use super::{
            CancelGuard, Canned, Loaded, Manifest, NodeState, PactEvent, Running, Scope, Toggled,
            Unwatched, apply_progress, apply_toggle, load, load_tree, mpsc, one_crate_to_load,
            pact_of, state_of, toggle,
        };
        use crate::POLL_INTERVAL;
        use crate::session::{NOT_WATCHING, Watched, note};

        /// A [`Watched`] with no watcher behind it, filtering against the
        /// tree that is on disk at `scope`.
        ///
        /// [`Watching::Off`] rather than a real watcher, so nothing here
        /// turns on an operating system deciding when to mention a write:
        /// what a drain would have handed the policy is handed to it
        /// directly instead, which is the same call the drain makes.
        fn unwatched(scope: &Scope) -> Watched {
            let Loaded { tree, .. } =
                load_tree(&scope.root).expect("a scratch repository with a `.git/` loads");
            Watched {
                watching: Watching::Off("no watcher in a test".to_owned()),
                policy: WatchPolicy::new(&tree),
                manifest: manifest_path(&scope.repo_root),
            }
        }

        #[test]
        fn a_watcher_driven_reload_leaves_the_reader_exactly_where_they_were() {
            // The same five things a post-pact reload keeps, kept by the
            // same one reload: nobody pressed anything this time, so a
            // tree that threw the reader back to the top here would do it
            // while they were reading, which is worse than a tree that
            // never updates at all.
            let scratch = one_crate_to_load("watch-place");
            scratch.write("crates/engine/tests/one.rs", "#[test] fn one() {}\n");
            scratch.write("crates/tui/src/main.rs", "fn main() {}\n");
            scratch.write("docs/adr/one.md", "# One\n");

            // A pact from before this reader sat down, so the pacted-only
            // filter has something to keep and the reload has a colour to
            // change.
            let agent = Canned::new(&scratch, []);
            apply_toggle(
                &Manifest::new(),
                &scratch.root,
                &toggle(&scratch, "crates", true),
                &agent,
                &mut Unwatched,
            )
            .expect("a subtree that walks and a manifest that writes");

            let (mut app, scope) = load(&scratch);
            let mut watched = unwatched(&scope);
            app.set_viewport_height(4);
            app.toggle_files();
            app.toggle_pacted_only();
            app = app.with_collapsed([scratch.path("crates/tui")]);
            for _ in 0..5 {
                app.select_next();
            }

            let selected = app
                .selected_row()
                .map(|row| row.path.clone())
                .expect("a row is selected");
            let collapsed = app.collapsed().clone();
            let offset = app.scroll_offset();
            assert!(offset > 0, "the reader scrolled off the first row");
            assert_eq!(
                state_of(&app, &scratch.path("crates/engine")),
                Some(NodeState::PactedFresh),
                "the reader is looking at a subtree that was fresh when they sat down"
            );

            // Somebody else saves a file in a directory the last walk
            // produced. Nothing is pressed here and nothing is pressed
            // below: what stands in for the drain is the policy being told
            // the path was accepted.
            scratch.write("crates/engine/src/extra.rs", "//! Late arrival.\n");
            let saved_at = Instant::now();
            watched.policy.accepted(saved_at);

            assert!(
                !watched.round(&mut app, &scope, false, saved_at),
                "read the tree before the disk had gone quiet"
            );
            assert!(
                watched.round(&mut app, &scope, false, saved_at + QUIET_PERIOD),
                "the disk went quiet and the tree was never read"
            );

            assert_eq!(
                state_of(&app, &scratch.path("crates/engine")),
                Some(NodeState::PactedStale),
                "a directory that went stale under the reader is still drawn green"
            );
            assert_eq!(
                app.selected_row().map(|row| row.path.clone()),
                Some(selected),
                "the selection moved"
            );
            assert_eq!(app.collapsed(), &collapsed, "the collapsed set moved");
            assert!(app.pacted_only(), "the filter was dropped");
            assert!(app.show_files(), "the file toggle was dropped");
            assert_eq!(app.scroll_offset(), offset, "the window moved");
            // The header used to be asserted here too. It is not an `App`'s to
            // lose any more — a reload rebuilds the app and never touches the
            // `Chrome` beside it — so the claim moved to `session`, where the
            // thing that could drop it lives.
            assert_eq!(app.message(), None, "and nothing went wrong to report");
        }

        #[test]
        fn a_trigger_during_a_pact_waits_for_the_runs_own_reload() {
            // A run writes documents, the documents are events, and the
            // run ends by re-reading the tree anyway. So the trigger is
            // remembered rather than acted on: one reload at the end, not
            // one during the run and another after it — and none at all
            // over a tree the run is still writing into.

            /// How many quiet periods of rounds the run is given: enough
            /// that the last of them is past the ceiling as well, so what
            /// they prove is that neither deadline fires under a run.
            const ROUNDS: u32 = 12;

            let scratch = one_crate_to_load("watch-in-flight");
            let (mut app, scope) = load(&scratch);
            let mut manifest = Manifest::new();
            let mut watched = unwatched(&scope);

            // What the run put on disk while it was running.
            scratch.write("docs/adr/one.md", "# One\n");
            let base = Instant::now();
            watched.policy.accepted(base);

            // Round after round with the pact in flight, long past the
            // quiet period and past the ceiling too: neither deadline is a
            // reason to read a tree out from under a run.
            for round in 1..=ROUNDS {
                let at = base + QUIET_PERIOD * round;
                assert!(
                    !watched.round(&mut app, &scope, true, at),
                    "the tree was read under a run in flight, {at:?} in"
                );
            }
            assert!(
                QUIET_PERIOD * ROUNDS > RELOAD_CEILING,
                "the rounds above stopped short of the ceiling, so they proved nothing about it"
            );
            assert!(
                state_of(&app, &scratch.path("docs")).is_none(),
                "the tree moved while the run was still going"
            );
            assert!(
                watched.policy.owes_reload(),
                "the trigger was dropped rather than remembered"
            );

            // The run ends the way every run ends — one reload at the
            // bottom of `apply_progress` — and the tree it read is what
            // the loop hands back to the policy.
            let (events, received) = mpsc::channel();
            let mut pact = Some(Running {
                events: received,
                cancel: CancelGuard::new(),
                work: pact_of(scratch.path("crates/engine")),
                before: app.clone(),
            });
            events
                .send(PactEvent::Finished(Ok(Toggled {
                    manifest: manifest.clone(),
                    granted: true,
                    message: None,
                    refusals: Vec::new(),
                })))
                .expect("the loop is still listening");
            let ended = base + Duration::from_secs(3);
            let reloaded = apply_progress(&mut pact, &mut app, &mut manifest, &scope, ended);
            assert!(pact.is_none(), "the run is over");
            assert!(reloaded.is_some(), "the run's own reload read the tree");
            watched.caught_up(reloaded.as_ref(), ended);

            assert!(
                state_of(&app, &scratch.path("docs")).is_some(),
                "the run's reload did not bring back what moved during it"
            );
            assert!(
                !watched.policy.owes_reload(),
                "the trigger outlived the reload that answered it"
            );
            assert!(
                !watched.round(
                    &mut app,
                    &scope,
                    false,
                    ended + RELOAD_CEILING + QUIET_PERIOD
                ),
                "the run reloaded twice: once at its end and once for the events it caused"
            );
        }

        #[test]
        fn the_line_saying_live_updates_are_off_is_said_once_and_gives_way() {
            // A watcher that would not start costs the noticing and
            // nothing else, so it is one line, said where the watcher was
            // asked for, and it never talks over anything the reader
            // actually asked for.
            const REFUSED: &str = "the manifest would not save";

            let scratch = one_crate_to_load("watch-off");
            let (mut app, scope) = load(&scratch);
            let mut watched = unwatched(&scope);

            let line = watched
                .off_note()
                .expect("a watcher that would not start says so");
            assert!(
                line.starts_with(NOT_WATCHING),
                "the footer says nothing about live updates: {line}"
            );
            assert!(
                line.len() > NOT_WATCHING.len() + 2,
                "the footer does not say why: {line}"
            );
            assert!(
                !line.contains('\n'),
                "a footer line that wraps is a footer line that hides a row: {line}"
            );

            // Said once, before the first frame, into a footer nobody else
            // was using.
            note(&mut app, line.clone());
            assert_eq!(app.message(), Some(line.as_str()));

            // And then a pact has something to say, which is the news: the
            // rounds after it are rounds of a loop that is not watching
            // anything, and not one of them says this again.
            app.set_message(REFUSED);
            let base = Instant::now();
            for round in 0..20 {
                watched.round(&mut app, &scope, false, base + POLL_INTERVAL * round);
            }
            assert_eq!(
                app.message(),
                Some(REFUSED),
                "the line said itself again, over a run's own message"
            );
            note(&mut app, line);
            assert_eq!(
                app.message(),
                Some(REFUSED),
                "the footer's precedence is the other way round for this line"
            );

            // And a watcher that did start says nothing at all. A real one
            // this time, over the scratch repository — started and asked,
            // with nothing waited for, since when this operating system
            // reports a write is not what is being asserted.
            let Loaded { tree, .. } = load_tree(&scope.root).expect("the scratch repository");
            assert_eq!(
                Watched::start(&scope, &tree).off_note(),
                None,
                "a working watcher put a line on the footer"
            );
        }
    }
}
