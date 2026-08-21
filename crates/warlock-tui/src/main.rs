//! Terminal front end for warlock.
//!
//! This binary is the thin, impure shell around the pure parts in
//! `warlock_tui`: it owns the terminal's lifecycle, the working directory it
//! was invoked from, and the event loop, and nothing else. It asks the engine
//! to load the tree for that directory and knows nothing about how one is
//! built; what a frame looks like is [`warlock_tui::draw`]'s business and how
//! the selection moves is [`App`]'s.
//!
//! The one rule this file exists to keep is that the terminal is restored on
//! every way out: a normal quit, an error returned up to `main`, and a panic.
//! Raw mode left switched on after exit means a shell that no longer echoes
//! what the user types, and that is not something they should have to know how
//! to fix. A pact runs on a thread of its own now, so a panic *there* is one of
//! those ways out too — and it is covered by the same process-wide hook, which
//! is why the hook is installed before anything else happens.
//!
//! The one long keystroke is the pact key, and it is the reason the loop below
//! is shaped the way it is. A subtree pact is minutes of model passes, so
//! pressing the key spawns a worker thread ([`spawn_pact`]) and hands back a
//! [`Receiver`] of what that worker has to say: which directory it is on, and,
//! once, how the whole thing went. The loop polls for a keystroke with a short
//! timeout instead of blocking on one, drains that channel every frame, and
//! draws — so the tree still scrolls, the footer's progress line still advances,
//! and the run lands on screen the moment the worker is done with it. Nothing
//! here waits on the worker: the manifest, the tree and the message are updated
//! from the events it sends, and the thread is never joined.
//!
//! What a run leaves behind is on disk rather than in the rows, and that is the
//! other thing the shape of the loop is for. A pact writes a `WARLOCK.md` beside
//! every directory it descends through, and those are rows the tree on screen
//! has never had, so the moment a run ends the view is one load out of date. One
//! rule covers it: [`apply_progress`] does its own arm's work first — the
//! outcome applied, the manifest saved — and then [`reload_tree`] re-reads the
//! tree from disk and re-seats the view on top of it, carrying the selection,
//! the collapsed directories, the filters and the window across by path. The
//! same single call ends all four ways a run can finish and an un-pact besides,
//! it runs here on the loop's thread and never on the worker's, and a load that
//! fails this late keeps the tree already drawn instead of ending the loop.
//! Nothing else asks for a reload: a file written by something other than
//! warlock still waits for a relaunch.
//!
//! A run that takes minutes has to be stoppable, and there are two ways to stop
//! one, which this file keeps apart on purpose. Esc *cancels*: the descent ends
//! between directories, the `claude` running right now is killed, and the worker
//! still finishes — it hashes and grants what it did write and saves the
//! manifest, so the record on disk is what actually completed. `q` and Ctrl-C
//! *quit*: the same handle kills the same child, but nothing waits for the
//! worker to tidy up, so the manifest is simply never rewritten. Either way the
//! documents already written are whole, because each of them is written beside
//! its directory and renamed over (WAR-21.01), and the manifest is written once
//! by a rename too — so there is no half-state for an abandoned worker to leave.
//! Both roads run through one [`Cancel`], which the run's [`Running`] owns and
//! drops through, which is why every way out of the loop — a quit, an error, a
//! `?` in the middle of a frame — takes the child with it.

use std::io::{self, Stdout};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::time::{Duration, Instant};
use std::{env, fmt, panic, thread};

use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::crossterm::cursor::Show;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::crossterm::execute;
use ratatui::crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use warlock_engine::{
    Agent, LoadError, LoadProblem, Loaded, Manifest, ManifestError, NodeState, PactFailure,
    PactObserver, PactProblem, PactedSubtree, Pacting, load_tree, pact_subtree, repository_root,
    unpact_subtree,
};
use warlock_tui::{
    Activities, Activity, App, Cancel, ClaudeAgent, PactToggle, draw, panel_height, reseat_on,
    tree_height,
};

/// How long the loop waits for a keystroke before going round again.
///
/// The number is a compromise between two things that are both cheap. A pact in
/// flight says where it has got to over a channel, and nothing but the top of
/// the loop reads that channel, so this is also how long a progress line can be
/// out of date: a tenth of a second is under the threshold at which a person
/// reads a screen as lagging. Ten wakeups a second with nothing to do is a
/// rounding error next to a terminal that repaints on every keystroke, and the
/// draw either side of it writes nothing when nothing has changed, because
/// ratatui diffs each frame against the last.
///
/// Polled rather than blocked on even when no pact is running, so there is one
/// loop rather than two: a second, blocking path taken only while idle would be
/// a second place the frame is drawn and the keys are handled, for a saving of a
/// few timer wakeups.
const POLL_INTERVAL: Duration = Duration::from_millis(100);

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

/// What the footer says when the reload after a run could not read the tree,
/// ahead of the load's own reason for it.
///
/// It says what the reader lost, which is the refresh and nothing else: the run
/// is over, its documents are on disk and its manifest is saved, and the rows
/// under this line are the ones that were there before — true, only older than
/// disk. Worded as a fact about the view rather than as a failure of the run,
/// because the run did not fail.
const NOT_REFRESHED: &str = "the view could not be refreshed and is the tree as it was";

fn main() -> ExitCode {
    // Before anything touches the terminal: a panic during setup has to leave
    // the terminal usable too.
    install_panic_hook();

    match run() {
        Ok(()) => ExitCode::SUCCESS,
        // `run` has returned, so the guard inside it has already dropped and
        // the terminal is back to normal; only now is it worth printing
        // anything, because on the alternate screen nobody would ever see it.
        Err(error) => {
            eprintln!("warlock: {error}");
            ExitCode::FAILURE
        }
    }
}

/// Load the tree, set the terminal up, run the event loop, and put the
/// terminal back.
///
/// The load happens *before* the guard is entered, on purpose. Both orders
/// restore the terminal correctly — the guard's `Drop` covers every `?` after
/// it, and it does not exist before it — so the choice is about what the user
/// sees on the failing path: loading first means a repository that will not
/// load never switches the screen at all, instead of flashing the alternate
/// screen up and tearing it down again around a message that is printed after
/// it is gone. It also keeps every filesystem error out of raw mode.
///
/// After the guard is entered, every `?` returns through its `Drop`, which is
/// the whole reason the guard exists: there is no error path out of this
/// function that skips restoration. The pact key is the one keystroke that
/// writes to disk, and it is deliberately not one of those paths — a pact that
/// goes wrong is news for the footer, not a reason to tear the screen down
/// (see [`apply_toggle`]).
///
/// The loop draws, waits [`POLL_INTERVAL`] for a keystroke, and then applies
/// whatever a pact in flight has said since the last time round. That order is
/// the one that matters: a progress event drained at the bottom of the loop is
/// on screen at the top of the next one, a few milliseconds later, and a
/// keystroke pressed during a pact is handled by exactly the same match as a
/// keystroke pressed with nothing running. Quitting while a pact runs returns
/// from here without joining the worker — see [`spawn_pact`] for why that is
/// safe — and the guard restores the terminal on the way out as it always did.
fn run() -> Result<(), Error> {
    let (mut app, scope) = load_app()?;
    // Loaded before the terminal is touched, for the same reason the tree is:
    // a manifest that will not parse should say so on the normal screen. This
    // is a second read of the file the loader already parsed, which is cheap
    // and keeps the front end from reaching into the loader's internals for a
    // value it needs to keep and edit.
    let mut manifest = load_manifest(&scope.repo_root)?;
    // The one thing in this binary that runs a model, built once because it is
    // a command line and a timeout rather than a connection: nothing is spawned
    // until a pact actually asks for a pass.
    let agent = ClaudeAgent::new();
    let mut guard = TerminalGuard::enter()?;
    // The pact running somewhere else, when one is: everything this thread
    // needs to keep about a run it is not performing. `None` is the ordinary
    // state, and it is what the pact key checks before starting anything.
    let mut pact: Option<Running> = None;

    loop {
        // Told before it is drawn, and every frame rather than on resize: the
        // scroll offset is only right if it was computed against the height
        // this frame gives the tree, and `tree_height` is the same layout the
        // frame is cut by. A terminal resized between frames is handled by that
        // alone — the next frame measures again, and the next frame is at most
        // one `POLL_INTERVAL` away.
        let size = guard.terminal.size()?;
        app.set_viewport_height(tree_height(size));
        // The panel's window is measured the same way and for the same reason,
        // off the same size: `panel_height` and `tree_height` are two answers
        // from the one layout, so both panes are scrolled by the height this
        // frame is about to give them.
        app.set_panel_height(panel_height(size));
        // The instant this frame is being drawn at, read once and handed to the
        // renderer: the panel's newest clock counts up against it, so a frame
        // drawn with no event waiting still shows a run that is moving. See
        // `draw`.
        guard
            .terminal
            .draw(|frame| draw(frame, &app, Instant::now()))?;

        // Waited on rather than blocked on. Nothing is drawn while this thread
        // sits here, so the wait has to end whether or not anybody presses
        // anything: a pact reports its progress over a channel that only the
        // bottom of this loop reads, and a progress line that waits for a
        // keystroke to appear is worse than none at all.
        if event::poll(POLL_INTERVAL)?
            && let Event::Key(key) = event::read()?
        {
            // The mode is passed in rather than read out of the app because
            // there is exactly one key it changes the meaning of — Esc, which
            // cancels a run when there is one and quits when there is not. See
            // [`action_for`].
            match action_for(key, pact.is_some()) {
                // Returning is the whole of quitting, and it is enough even
                // with a pact in flight. `pact` drops on the way out, which
                // cancels the run and kills the `claude` it was waiting on
                // (see [`Running`]); the guard drops after it and puts the
                // terminal back. Nothing joins the worker: it is left to be
                // ended by the process, having written whole documents or none,
                // and the manifest it never got to rewrite still says what it
                // said before.
                Some(Action::Quit) => return Ok(()),
                // Esc with a run in flight. The handle does both halves at once
                // — it latches, so the descent stops at the next directory
                // instead of starting a pass for it, and it kills the `claude`
                // running right now, so that stop happens in milliseconds
                // rather than at the end of a five-minute pass.
                //
                // The pact is deliberately *not* taken down here. The worker is
                // still going to hash what it wrote, save the manifest and
                // report, and all of that arrives at the bottom of this loop
                // like any other outcome; forgetting about it now would leave
                // the footer's progress line up for a run nobody was listening
                // to any more.
                Some(Action::CancelPact) => {
                    if let Some(running) = pact.as_ref() {
                        running.cancel.cancel();
                    }
                }
                // Nothing but a bit of view state moves here, and deliberately
                // so: focus decides which border the next frame lights and which
                // pane a movement key is about, and both of those questions are
                // answered where they are asked — by the renderer reading
                // `App::focus`, and by the app's own movement methods, which
                // move the tree's selection or scroll the panel's window
                // depending on the pane being driven (WAR-26.02). There is
                // nothing for this arm to gate a second time, and no message: a
                // key that changes what the *next* key means has nothing to
                // report.
                Some(Action::ToggleFocus) => app.toggle_focus(),
                Some(Action::SelectPrevious) => app.select_previous(),
                Some(Action::SelectNext) => app.select_next(),
                // No height is passed: the app was told the viewport's height
                // at the top of this loop, so a page is whatever the frame just
                // drawn could show.
                Some(Action::SelectPageUp) => app.select_page_up(),
                Some(Action::SelectPageDown) => app.select_page_down(),
                Some(Action::SelectFirst) => app.select_first(),
                Some(Action::SelectLast) => app.select_last(),
                // Nothing else happens here on purpose. What is collapsed is
                // the front end's view of the tree and never touches disk (§8),
                // so there is no manifest to write; the tree has not changed,
                // so there is nothing to re-read. The app moves the selection
                // and the scroll offset back into range itself, and the next
                // frame — the top of this same loop — draws the shorter or
                // longer list.
                Some(Action::ToggleCollapsed) => app.toggle_collapsed(),
                // Nothing else happens here either, and for the same reasons as
                // collapsing: which rows are worth looking at is the front end's
                // view of the tree and is never written down (§5), so there is
                // no manifest to save, and the tree itself has not changed, so
                // there is nothing to re-read. The app re-flows its rows and
                // puts the selection and the scroll offset back in range; the
                // next frame draws whatever is left.
                Some(Action::TogglePactedOnly) => app.toggle_pacted_only(),
                // Nothing else here either, for the third time and for the same
                // reasons as the two arms above: whether the files inside a
                // module are on screen is the front end's view of the tree and
                // is never written down (§5), so there is no manifest to save,
                // and the files were read by the load that built these rows, so
                // there is nothing to re-read. The app re-flows its rows and
                // keeps the selection and the scroll offset in range; the next
                // frame draws the longer or shorter list.
                Some(Action::ToggleFiles) => app.toggle_files(),
                // The one keystroke that writes anything, and the one that
                // takes longer than a frame — so it is the one that is not done
                // here. The subtree pact goes to a worker thread and this arm
                // keeps the receipt: what the app looked like before the toggle
                // painted it, and which subtree was painted. Everything the run
                // produces arrives at the bottom of this loop, one directory at
                // a time and finally as an outcome, and until it does the loop
                // goes round as usual — drawing, scrolling, filtering.
                //
                // `None` needs nothing done about it, and it now covers two
                // cases that are alike in exactly this way. A refused toggle has
                // already put its own sentence in `App::message`, which the next
                // frame draws; a press while a pact is in flight has changed
                // nothing at all, because a second pact over a tree the first
                // one is still writing to would be two runs racing for the same
                // documents and the same manifest. Neither is this arm's to
                // explain — see `pact_press`.
                Some(Action::TogglePact) => {
                    // Copied before the toggle paints anything, because the
                    // toggle is no longer its own undo: it puts a whole subtree
                    // into one state, and the states it painted over were not
                    // all the same one. The copy is a list of rows and a tally,
                    // and it is taken once per press of one key.
                    let before = app.clone();
                    if let Some(toggle) = pact_press(&mut app, pact.is_some()) {
                        // One handle per run, and never reused: a cancel is
                        // final, so the run after a cancelled one has to start
                        // with a handle nobody has said stop to.
                        let cancel = CancelGuard::new();
                        pact = Some(Running {
                            events: spawn_pact(
                                &manifest,
                                &scope.repo_root,
                                &toggle,
                                &agent,
                                cancel.handle(),
                            ),
                            cancel,
                            path: toggle.path,
                            before,
                        });
                    }
                }
                None => {}
            }
        }

        // Every frame, whether or not a key was pressed: this is the only place
        // anything the worker says reaches the screen, and it has to keep up
        // with a thread that is not waiting for it. It is also the only place
        // the tree is re-read after startup, which is why the scope the app was
        // loaded at is handed to it — see [`reload_tree`].
        apply_progress(&mut pact, &mut app, &mut manifest, &scope);
    }
}

/// A pact being run by a worker thread, from the point of view of the thread
/// drawing the screen.
///
/// Four things, and no handle to join: what the worker has to say, how to tell
/// it to stop, which subtree it was started on, and what the tree looked like
/// before the keystroke painted that subtree yellow.
///
/// The path is kept here rather than read back off the app because it is the
/// subtree the *run* covers, which is a fact about the run and not about
/// whatever is selected by the time it ends — the reader is free to move the
/// selection anywhere while a pact works, and does.
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
struct Running {
    /// Progress and, once, the outcome. Closed by the worker dropping its end,
    /// which is how a panicked worker is noticed.
    events: Receiver<PactEvent>,
    /// Say-when for the worker: the flag its observer reads between directories
    /// and the kill switch for the `claude` it is waiting on.
    cancel: CancelGuard,
    /// The directory the pact key was pressed on, whose subtree the run covers.
    path: PathBuf,
    /// The app as it stood before the toggle painted the subtree.
    before: App,
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
struct CancelGuard {
    /// The handle every clone of which speaks for this run.
    cancel: Cancel,
}

impl CancelGuard {
    /// A handle nobody has said stop to yet.
    fn new() -> Self {
        Self {
            cancel: Cancel::new(),
        }
    }

    /// A clone for the worker to give its agent and its observer.
    fn handle(&self) -> Cancel {
        self.cancel.clone()
    }

    /// Stop the run: latch the flag the descent reads, and kill the `claude`
    /// in flight.
    fn cancel(&self) {
        self.cancel.cancel();
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
/// Three things, and the order of two of them is fixed: one
/// [`PactEvent::Starting`] per directory as the run reaches it, any number of
/// [`PactEvent::Doing`] from the pass that directory is running, and then
/// exactly one [`PactEvent::Finished`]. Nothing else is sent, and nothing is
/// sent after the outcome — the worker drops its end of the channel and stops.
///
/// Activities ride this channel rather than one of their own because there is
/// nothing to gain from a second: they come from the same worker, they are read
/// by the same thread, and a second receiver would be a second thing the event
/// loop has to poll and a second way for the two streams to arrive out of the
/// order the run produced them in.
#[derive(Debug)]
enum PactEvent {
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
}

/// Run `toggle` on a thread of its own, and hand back the channel it reports on.
///
/// The worker owns everything it touches — its own manifest, its own root, its
/// own toggle, its own [`ClaudeAgent`], which is a command line and a timeout
/// and so is cheap to clone — so nothing is shared with the event loop but the
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
fn spawn_pact(
    manifest: &Manifest,
    repo_root: &Path,
    toggle: &PactToggle,
    agent: &ClaudeAgent,
    cancel: Cancel,
) -> Receiver<PactEvent> {
    let (events, received) = mpsc::channel();
    let (manifest, repo_root, toggle) = (manifest.clone(), repo_root.to_path_buf(), toggle.clone());
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
    thread::spawn(move || run_pact(&manifest, &repo_root, &toggle, &agent, &cancel, &events));
    received
}

/// The worker thread's whole body: carry the toggle out, saying where it has got
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
    toggle: &PactToggle,
    agent: &dyn Agent,
    cancel: &Cancel,
    events: &Sender<PactEvent>,
) {
    let outcome = apply_toggle(
        manifest,
        repo_root,
        toggle,
        agent,
        &mut Reporting { events, cancel },
    );
    // Only a pact is cancellable. An un-pact is manifest arithmetic that is over
    // before a key can be read, so a handle latched while one ran would be
    // describing something else entirely.
    let outcome = match outcome {
        Ok(toggled) if toggle.pacted && cancel.is_cancelled() => Ok(cancelled(toggled)),
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
/// Two refusals, both silent here and for different reasons. A press while a
/// pact is in flight does nothing whatsoever — no toggle, no colour, no message
/// — because the run in flight is already writing to the tree the second one
/// would write to, and because the footer is saying what that run is doing,
/// which is the answer to "why did nothing happen?". A press the app itself
/// turns down — a file row — has already had its say in
/// [`App::message`](warlock_tui::App::message).
///
/// A function rather than a guard in the match arm so that "a second press
/// changes nothing" is a property a test can hold the app up against, rather
/// than something only an event loop with a terminal attached could show.
fn pact_press(app: &mut App, in_flight: bool) -> Option<PactToggle> {
    if in_flight {
        return None;
    }
    app.toggle_pact()
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
/// However the run ended, the tree is then re-read from disk and the view put
/// back on top of it ([`reload_tree`]). One reload, at the bottom, for all four
/// endings and for an un-pact as much as for a pact: a run writes `WARLOCK.md`
/// files that no amount of recolouring rows in place can conjure into the tree,
/// and the only honest source for what is now on disk is disk. It runs *after*
/// each arm has done its own work — after the manifest is saved and after the
/// two restoring arms have put `running.before` back — so what the reload reads
/// lands on top of the arm's result rather than under it, and a reload that
/// fails leaves that result standing.
fn apply_progress(
    pact: &mut Option<Running>,
    app: &mut App,
    manifest: &mut Manifest,
    scope: &Scope,
) {
    let Some(running) = pact.as_ref() else {
        return;
    };

    let outcome = loop {
        match running.events.try_recv() {
            Ok(PactEvent::Starting {
                directory,
                position,
                total,
            }) => app.set_pact_in_flight(directory, position, total),
            // Taken off the channel and dropped, which is the whole of what this
            // slice owes them: they have to be read or they would sit in the
            // channel until the run ends, and there is nowhere for them to go
            // yet. What a reader sees of them is a later slice's business;
            // nothing is kept here, because keeping something is the first line
            // of a panel and this is not the change that draws one. Bound rather
            // than matched with `_` so that the activity a `PactEvent` carries
            // counts as read on the way past — the alternative is an `allow` on
            // the variant that would outlive its reason.
            Ok(PactEvent::Doing(activity)) => drop(activity),
            Ok(PactEvent::Finished(outcome)) => break Some(outcome),
            // Still running, and nothing new to say.
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => break None,
        }
    };

    let running = pact
        .take()
        .expect("the pact drained just above is still here");
    app.clear_pact_in_flight();
    match outcome {
        Some(Ok(Toggled {
            manifest: next,
            granted,
            message,
        })) => {
            *manifest = next;
            // The app painted the subtree stale when the key was pressed,
            // because stale is all it could know. A pact that came back with
            // nothing wrong wrote, hashed and granted every directory in it, so
            // the subtree is fresh and only this line knows it.
            //
            // A pact with a failure in it leaves the whole subtree yellow,
            // branches that did earn grants included: yellow is "pacted, not
            // proven fresh", which is true of every directory in it until the
            // next load, and colouring the rest green from here would be this
            // file second-guessing per node a manifest it did not compute.
            if granted {
                app.set_subtree_state(&running.path, NodeState::PactedFresh);
            }
            if let Some(message) = message {
                app.set_message(message);
            }
        }
        // Nothing was recorded: either the subtree could not be listed, or the
        // manifest would not save. Documents may well be on disk in the second
        // case, but the manifest is the record of what is pacted and it still
        // says what it said before, so the rows go back to matching it and the
        // reason goes on the app's line — the same one a refused toggle uses —
        // rather than out of the loop, which would take the screen with it.
        Some(Err(message)) => {
            *app = running.before;
            app.set_message(message);
        }
        // The worker died with the manifest in this thread's hand untouched, so
        // the rows go back to matching it exactly as they do for a run that
        // recorded nothing, and the footer says the run is over.
        None => {
            *app = running.before;
            app.set_message(PACT_LOST);
        }
    }

    // The run is over and everything it recorded is on disk, so the rows on
    // screen are one load out of date whichever arm above ran.
    reload_tree(app, scope);
}

/// Re-read the tree at `scope` from disk and put the view back on top of it.
///
/// Two calls and no judgement of its own: [`load_tree`] for what is on disk now,
/// and [`reseat_on`] to carry the reader's selection, collapsed set, filters,
/// window and footer across to it. The header is re-derived the way
/// [`load_app`] derives it, from the repository root and the root the new tree
/// came back rooted at, so the line at the top names the tree the engine just
/// walked rather than the one it walked at startup.
///
/// Called on the event loop's thread and on no other. A worker thread must never
/// reach in here: it would be reading a tree while the thread that draws it is
/// drawing one, for a result only the drawing thread can use.
///
/// A load that fails is not an error out of the event loop, and this is the
/// deliberate difference from [`load_app`], where the same failure is fatal.
/// Warlock is up, the documents the run wrote are whole on disk and the manifest
/// that records them is saved; quitting here would throw away a run that cost
/// minutes and money, over nothing worse than a stale screen. So the tree
/// already drawn is kept and the reader carries on with it — the one thing they
/// lose is the refresh. Problems that did not stop the load are a different
/// matter: the engine has already coloured each affected node conservatively, so
/// a tree that has the new documents in it beats the stale one it would replace,
/// and it is taken.
///
/// Either way there is a line to write — [`NOT_REFRESHED`] and the load's reason
/// for one that failed, the problems' own wording and their count for one that
/// did not — and it goes on the footer only when the run left the footer empty.
/// The pact's message wins because it is the news: what a run made of the
/// subtree the reader asked for is worth more than how the redraw after it went,
/// and the footer is one line. Precedence, not merging: two sentences joined by
/// a semicolon would be a line nobody reads to the end of.
fn reload_tree(app: &mut App, scope: &Scope) {
    let note = match load_tree(&scope.root) {
        Ok(Loaded { tree, problems }) => {
            *app = reseat_on(app, &tree).with_scope(&scope.repo_root, tree.root_path());
            // The same count, in the same words, as the startup load that
            // refuses to draw a tree with problems in it: one problem quoted
            // and the rest counted. A node the engine could not hash is
            // already stale on screen, so this line is the only place the
            // number of them appears.
            Error::from_problems(&problems).map(|counted| counted.to_string())
        }
        // Flattened for the same reason `Error` flattens it: a manifest that
        // will not parse arrives as the TOML parser's several lines, and the
        // footer is one.
        Err(source) => Some(format!(
            "{NOT_REFRESHED}: {}",
            one_line(&source.to_string())
        )),
    };

    if let Some(note) = note
        && app.message().is_none()
    {
        app.set_message(note);
    }
}

/// Where the tree on screen came from: the directory it is rooted at, and the
/// repository root above that directory.
///
/// Both are resolved once, by [`load_app`], and kept for as long as warlock
/// runs. The root is where a re-read starts, and the repository root is what the
/// manifest is written under and what the header spells the root relative to —
/// which is why they travel together rather than being guessed at again from a
/// working directory that has since had a pact run over it.
struct Scope {
    /// The directory the tree is rooted at, as the load that built it came back
    /// rooted — not the working directory as typed.
    root: PathBuf,
    /// The repository root above `root`: the nearest ancestor with a `.git/`.
    repo_root: PathBuf,
}

/// What one press of the pact key came to, once the manifest it produced is on
/// disk.
#[derive(Debug)]
struct Toggled {
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
}

/// Carry `toggle` out — pact the subtree, or take it out of the manifest — and
/// write the result to disk.
///
/// Both halves are the engine's ([`pact_subtree`], [`unpact_subtree`]); what
/// this function owns is the order the front end needs them in and the single
/// [`Manifest::save`] at the end of it. Once, at the end, is the whole point:
/// a save per directory would leave `.warlock/pacts.toml` recording a pact that
/// was still running, and one that died half way through would be indexed as
/// finished.
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
    toggle: &PactToggle,
    agent: &dyn Agent,
    observer: &mut dyn PactObserver,
) -> Result<Toggled, String> {
    let (next, granted, message) = if toggle.pacted {
        let PactedSubtree {
            manifest,
            failures,
            problems,
        } = pact_subtree(&toggle.path, repo_root, manifest, agent, observer)
            .map_err(|source| one_line(&source.to_string()))?;
        // Failures alone decide freshness, and the byte caps' problems do not:
        // a request that left a lockfile out still produced a document, a hash
        // and a grant. They are still worth a line, which is why the two travel
        // separately from here on.
        let granted = failures.is_empty();
        (manifest, granted, pact_message(&failures, &problems))
    } else {
        // Un-pacting is pure manifest editing — no walk, no pass, no hash, and
        // every `WARLOCK.md` left where it is — so the only thing it can refuse
        // is a path the manifest has no spelling for. The app has already said
        // what un-pacting leaves behind, and nothing here talks over it.
        let next = unpact_subtree(&toggle.path, repo_root, manifest)
            .map_err(|source| Error::Manifest { source }.to_string())?;
        (next, false, None)
    };

    next.save(repo_root)
        .map_err(|source| Error::Manifest { source }.to_string())?;
    Ok(Toggled {
        manifest: next,
        granted,
        message,
    })
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

/// The repository's manifest, or an empty one if it has never pacted anything.
///
/// The same reading of a missing file the loader takes: nothing on disk and
/// nothing pacted are the same thing to draw, and the difference only matters
/// to code that would refuse to create the file, which this is not — pressing
/// `p` in a repository with no `.warlock/` is how the first manifest gets
/// written.
fn load_manifest(repo_root: &Path) -> Result<Manifest, Error> {
    match Manifest::load(repo_root) {
        Err(ManifestError::NotFound { .. }) => Ok(Manifest::new()),
        other => other.map_err(|source| Error::Manifest { source }),
    }
}

/// The app state for the directory warlock was invoked from, and the
/// [`Scope`] it was loaded at.
///
/// The two paths are handed back rather than dropped once the header is built.
/// The repository root is where the manifest lives: every pact written during
/// the run is written relative to it, and finding it is a walk up the
/// filesystem that should happen once. The tree's own root is where a re-read
/// starts, and it is kept for the same reason — it is the path the engine came
/// back rooted at, which is the one thing a later load must be given rather than
/// guess.
///
/// This is the whole of the front end's knowledge about scope, and it is
/// section 12's modular invocation rule spelled out: the tree is rooted at the
/// working directory, and the manifest that colours it comes from the single
/// repository root above that directory — which is why the header needs both.
/// The root is taken from the loaded tree rather than from the working
/// directory as typed, so the header names the same path the engine walked.
///
/// A load that reported problems is refused rather than drawn. The problems
/// are files Warlock could not read, so the nodes above them are coloured
/// stale on no evidence; showing that as an ordinary tree would put a colour
/// on screen that nothing on disk backs up. That is a startup rule and stays
/// one: mid-session, with a tree already on screen and a run's documents
/// already on disk, the same problems are taken rather than fatal — see
/// [`reload_tree`].
fn load_app() -> Result<(App, Scope), Error> {
    let working_dir = env::current_dir().map_err(|source| Error::WorkingDirectory { source })?;
    let Loaded { tree, problems } =
        load_tree(&working_dir).map_err(|source| Error::Load { source })?;
    if let Some(error) = Error::from_problems(&problems) {
        return Err(error);
    }

    // The load succeeded, so a repository root was found; asking again is a
    // walk up a path, not a second load. The fallback is unreachable, and
    // labelling the root as itself is the closest thing to true if it ever is
    // reached.
    let repo_root = repository_root(tree.root_path()).unwrap_or(working_dir);

    let app = App::from_tree(&tree).with_scope(&repo_root, tree.root_path());
    let scope = Scope {
        root: tree.root_path().to_path_buf(),
        repo_root,
    };
    Ok((app, scope))
}

/// Everything that can stop warlock showing a tree.
///
/// Richer than the `io::Error` this used to return, because loading brings
/// failures that are not I/O — a directory outside any repository, a manifest
/// that will not parse — and each of them is a different sentence. Every one
/// of those sentences is a single line: `main` prints exactly one, after the
/// terminal is back, and a message wrapping onto a second line in a restored
/// shell is a message that looks like a crash.
#[derive(Debug)]
enum Error {
    /// The working directory could not be read, so there is nothing to scope
    /// the run to.
    WorkingDirectory {
        /// What the operating system said.
        source: io::Error,
    },
    /// The engine refused to load a tree for the working directory.
    Load {
        /// Which of the load's fatal cases it was.
        source: LoadError,
    },
    /// The load finished, but could not colour every node it was asked to.
    ///
    /// Kept as finished text rather than as the [`LoadProblem`]s themselves:
    /// what this variant needs to carry is one line, and formatting it at the
    /// point the problems are still in hand keeps [`fmt::Display`] free of the
    /// question of how many of them to mention.
    Problems {
        /// The first problem, as it worded itself: node, file and reason.
        first: String,
        /// How many further problems went unnamed.
        rest: usize,
    },
    /// The pact manifest could not be read at startup, or written when a pact
    /// was toggled.
    ///
    /// The write is the interesting half: a read-only `.warlock/`, a full
    /// disk. It reaches `main` the same way every other failure does, which is
    /// the point — the terminal is restored by the guard on the way out, one
    /// line goes to stderr, and the exit status says it did not work.
    Manifest {
        /// Which of the manifest's cases it was, with the path it names.
        source: ManifestError,
    },
    /// The terminal could not be set up, drawn to, or read from.
    Terminal {
        /// What the terminal said.
        source: io::Error,
    },
}

impl Error {
    /// The error for a load's non-fatal `problems`, or `None` when it had
    /// none.
    ///
    /// Only the first problem is quoted. One unreadable file usually means a
    /// whole directory of them, and a message per file would scroll the useful
    /// one off the screen; the count says how much was left out, and the named
    /// file is enough to go and look at.
    fn from_problems(problems: &[LoadProblem]) -> Option<Self> {
        let first = problems.first()?;
        Some(Self::Problems {
            first: one_line(&first.to_string()),
            rest: problems.len() - 1,
        })
    }
}

/// `message` as a single line: what it says first, and, when it ran to several
/// lines, why it says so last.
///
/// A parser's diagnostic is laid out for a compiler's output — the location on
/// the first line, then the offending source with a caret under it, then the
/// explanation — and those middle lines mean nothing once they are not in a
/// fixed-width block under each other. What survives is the two lines that are
/// sentences: `TOML parse error at line 1, column 5` and the explanation under
/// the caret, which between them say where to look and what is wrong.
///
/// Rejoining rather than truncating, because dropping the last line would
/// throw away the only part that says *why*. Single-line messages — every I/O
/// error, and every error this workspace writes itself — come back untouched.
fn one_line(message: &str) -> String {
    let mut lines = message
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty());
    let Some(first) = lines.next() else {
        return String::new();
    };
    match lines.next_back() {
        Some(last) => format!("{first}: {last}"),
        None => first.to_owned(),
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::WorkingDirectory { source } => {
                write!(f, "could not read the working directory: {source}")
            }
            // The engine's own wording, which is already the sentence to show
            // a user — flattened, because a manifest that will not parse
            // carries the TOML parser's multi-line diagnostic inside it.
            Self::Load { source } => write!(f, "{}", one_line(&source.to_string())),
            // Flattened for the same reason as a load: the manifest's own
            // errors carry the TOML parser's multi-line diagnostic.
            Self::Manifest { source } => write!(f, "{}", one_line(&source.to_string())),
            Self::Problems { first, rest: 0 } => write!(f, "{first}"),
            Self::Problems { first, rest } => {
                write!(f, "{first} (and {rest} more like it)")
            }
            Self::Terminal { source } => write!(f, "{source}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::WorkingDirectory { source } | Self::Terminal { source } => Some(source),
            Self::Load { source } => Some(source),
            Self::Manifest { source } => Some(source),
            Self::Problems { .. } => None,
        }
    }
}

impl From<io::Error> for Error {
    /// Everything reached by `?` once the terminal is up is the terminal:
    /// entering raw mode, drawing a frame, reading an event. The load path
    /// names its own errors and never comes through here.
    fn from(source: io::Error) -> Self {
        Self::Terminal { source }
    }
}

/// What a keystroke asks the app to do.
///
/// Naming the intent separately from the key that produced it keeps
/// [`action_for`] a pure function of a key event, testable with no terminal
/// attached, and leaves the loop above reading as a list of consequences.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Action {
    /// Leave the app.
    Quit,
    /// Stop the pact that is running, and stay.
    CancelPact,
    /// Move the keys from one pane to the other: the tree column and the panel
    /// beside it swap which of them is lit and which of them the movement keys
    /// drive.
    ///
    /// One action rather than a focus-the-tree and a focus-the-panel, because
    /// there is one key and two panes: with only two places focus can be, "go to
    /// the other one" is the whole of what a reader can mean by pressing it, and
    /// a pair of actions would be two names for the same keystroke read twice.
    ToggleFocus,
    /// Move the selection one row up.
    SelectPrevious,
    /// Move the selection one row down.
    SelectNext,
    /// Move the selection one screenful up.
    SelectPageUp,
    /// Move the selection one screenful down.
    SelectPageDown,
    /// Select the first row of the tree.
    SelectFirst,
    /// Select the last row of the tree.
    SelectLast,
    /// Hide the selected directory's descendants, or show them again if they
    /// are hidden already.
    ToggleCollapsed,
    /// Draw only the pacted nodes and the ancestors that reach them, or the
    /// whole tree again if that is what is on screen already.
    TogglePactedOnly,
    /// Draw the files inside each directory as well as the directories, or go
    /// back to directories alone if the files are on screen already.
    ToggleFiles,
    /// Pact the selected node, or unpact it if it is pacted already.
    TogglePact,
}

/// The action `key` asks for with a pact `in_flight` or without one, or `None`
/// for a key that means nothing here.
///
/// One key reads two ways, and it is Esc. With nothing running it quits, which
/// is what it has always done and what the footer has always said. With a pact
/// running it cancels *that* — because the run is the thing in front of the
/// reader, because stopping it is the only thing they can want from a key that
/// means "not this", and because quitting outright on the key nearest to hand
/// would be the one keystroke that costs minutes of somebody else's model time
/// by mistake. Quitting during a run is still one keystroke away, spelled `q` or
/// Ctrl-C, which say what they mean and are not what a hand reaches for to stop
/// something.
///
/// The mode is a parameter rather than something looked up, so this stays a pure
/// function of a key and a situation and both readings are one assertion each.
/// Nothing else in here consults it: every other key means exactly what it meant
/// before, mid-pact included, which is what keeps the tree usable while a run
/// works.
///
/// Only presses count. Crossterm reports key releases and auto-repeats on some
/// platforms (Windows, and on terminals that speak the Kitty keyboard
/// protocol) and not on others, so acting on anything but a press would move
/// the selection twice per keystroke on those platforms and once on the rest —
/// and, since `p` writes the manifest, would toggle a pact straight back off
/// again on the release of the key that turned it on.
///
/// Ctrl-C is a key event, not a signal: raw mode is exactly the mode in which
/// the terminal stops turning it into `SIGINT`, so if this function does not
/// handle it, nothing does — including during a pact, where it is one of the two
/// ways out that also has to take the running `claude` with it.
fn action_for(key: KeyEvent, in_flight: bool) -> Option<Action> {
    if key.kind != KeyEventKind::Press {
        return None;
    }

    match key.code {
        // `contains` rather than equality: shift or caps lock can ride along
        // (some terminals report the upper-case letter with it), and Ctrl-C is
        // still Ctrl-C.
        KeyCode::Char('c' | 'C') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            Some(Action::Quit)
        }
        // Before the quit arm below, and the only thing in here the mode
        // touches: `q` and Ctrl-C keep meaning quit while a pact runs, and Esc
        // stops being a way out for as long as there is a run to stop.
        KeyCode::Esc if in_flight => Some(Action::CancelPact),
        KeyCode::Char('q') | KeyCode::Esc => Some(Action::Quit),
        // Tab is the key every two-pane program moves focus with, and it is the
        // only new binding here: it takes no argument and asks no question, so
        // it means the same thing whether or not a pact is in flight, exactly
        // like every key below it. Shift-Tab is a different keystroke and is not
        // bound — with two panes there is no "backwards" for it to mean.
        KeyCode::Tab => Some(Action::ToggleFocus),
        KeyCode::Up | KeyCode::Char('k') => Some(Action::SelectPrevious),
        KeyCode::Down | KeyCode::Char('j') => Some(Action::SelectNext),
        KeyCode::PageUp => Some(Action::SelectPageUp),
        KeyCode::PageDown => Some(Action::SelectPageDown),
        // `g` and `G` are the pair every pager and vi-like editor has trained
        // hands for, and they are told apart by case alone: matching on the
        // character rather than on `SHIFT` keeps a terminal that reports the
        // upper-case letter without the modifier — or with it, or with caps
        // lock instead — landing on the same action, exactly as Ctrl-C above
        // does not care which of those it is handed.
        KeyCode::Char('g') => Some(Action::SelectFirst),
        KeyCode::Char('G') => Some(Action::SelectLast),
        // Space is the file-tree key everywhere, and crossterm spells it as an
        // ordinary character: there is no `KeyCode::Space`, so `Char(' ')` is
        // the whole of it. Nothing rides along that needs matching — a modifier
        // held with space is a different keystroke, not this one badly spelled.
        KeyCode::Char(' ') => Some(Action::ToggleCollapsed),
        // Lower case only, like `p` below: the upper-case letter is a
        // different keystroke and means nothing here, and a filter that also
        // answered to `O` would take a key that a later binding may want. The
        // mnemonic is "only": what stays on screen is the pacted nodes only.
        KeyCode::Char('o') => Some(Action::TogglePactedOnly),
        // Lower case only, like `o` above and `p` below. The mnemonic is
        // "files": what the key adds to the screen is the files inside each
        // module. It writes nothing and reads nothing — the files came with the
        // tree — so, unlike `p`, there is nothing here that a stray press could
        // cost anybody.
        KeyCode::Char('f') => Some(Action::ToggleFiles),
        // Lower case only, and with no confirmation: the mnemonic is the
        // product's own word (pact, §15), and the action is its own undo —
        // pressing it again removes what it wrote.
        KeyCode::Char('p') => Some(Action::TogglePact),
        _ => None,
    }
}

/// A terminal in raw mode on the alternate screen, restored when dropped.
///
/// Setup and teardown are paired by ownership rather than by remembering to
/// call a teardown function on each of the several ways out of the event loop.
/// Dropping it is the only way to restore the terminal on the normal and error
/// paths; the panic path is covered separately by [`install_panic_hook`],
/// because a panic unwinding through here would run the hook first and print
/// its message onto the alternate screen, which then disappears with it.
struct TerminalGuard {
    terminal: Terminal<CrosstermBackend<Stdout>>,
}

impl TerminalGuard {
    /// Enter raw mode and the alternate screen.
    ///
    /// On failure part-way through, the guard never exists and so never drops,
    /// which is why this undoes its own work before returning the error.
    fn enter() -> io::Result<Self> {
        enable_raw_mode()?;
        if let Err(error) = execute!(io::stdout(), EnterAlternateScreen) {
            restore_terminal();
            return Err(error);
        }
        match Terminal::new(CrosstermBackend::new(io::stdout())) {
            Ok(terminal) => Ok(Self { terminal }),
            Err(error) => {
                restore_terminal();
                Err(error)
            }
        }
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        restore_terminal();
    }
}

/// Put the terminal back the way it was found, best effort.
///
/// Every step is attempted even if an earlier one fails, and none of them
/// report anything: this runs while unwinding from a panic and while returning
/// an error, and in both cases there is a more interesting message on its way
/// to the user that a complaint about a terminal escape sequence would only
/// bury.
fn restore_terminal() {
    let _ = disable_raw_mode();
    // Drawing a frame hides the cursor, so leaving without showing it again
    // hands back a shell with an invisible caret.
    let _ = execute!(io::stdout(), LeaveAlternateScreen, Show);
}

/// Install a panic hook that restores the terminal and then chains to whatever
/// hook was installed before.
///
/// Order matters twice over. It must run before raw mode is entered, so a
/// panic during setup is covered as well. And it must restore *before*
/// delegating, so the panic message lands on the normal screen where it can be
/// read and scrolled back to, instead of on the alternate screen that is about
/// to vanish. Chaining rather than replacing keeps the default hook's message
/// and backtrace, which are the entire point of a panic.
fn install_panic_hook() {
    let previous = panic::take_hook();
    panic::set_hook(Box::new(move |info| {
        restore_terminal();
        previous(info);
    }));
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
    use warlock_engine::ManifestError;

    use super::{Action, Error, action_for, one_line};

    /// A root no test touches on disk: every path below is made relative to it
    /// by string surgery, so the tests using it need no repository, no
    /// temporary directory and no filesystem at all.
    const ROOT: &str = "/repo";

    /// A plain press of `code`, as crossterm reports one with no modifiers.
    fn press(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    /// The problem text the engine hands over, standing in for a
    /// [`LoadProblem`](warlock_engine::LoadProblem) — which cannot be built
    /// outside the engine, because the hash error inside it is
    /// `#[non_exhaustive]`. What is under test here is the wrapping, not the
    /// engine's wording.
    const PROBLEM: &str = "`/repo/crates/engine` could not be hashed and is stale: \
                           could not read `/repo/crates/engine/src/lib.rs`, so the \
                           subtree has no hash: permission denied";

    #[test]
    fn a_message_that_is_already_one_line_is_left_alone() {
        assert_eq!(one_line("permission denied"), "permission denied");
        assert_eq!(one_line("  padded  \n"), "padded");
        assert_eq!(one_line(""), "");
    }

    #[test]
    fn a_parser_diagnostic_keeps_where_and_why_and_drops_the_snippet() {
        // Exactly the shape `toml` produces for a manifest that will not
        // parse, wrapped in the engine's own wording.
        let message = "could not read the pact manifest: malformed pact manifest: \
                       TOML parse error at line 1, column 5\n  |\n1 | not a manifest\n  \
                       |     ^\nkey with no value, expected `=`\n";

        assert_eq!(
            one_line(message),
            "could not read the pact manifest: malformed pact manifest: TOML parse \
             error at line 1, column 5: key with no value, expected `=`",
        );
    }

    #[test]
    fn a_load_with_no_problems_is_not_an_error() {
        assert!(Error::from_problems(&[]).is_none());
    }

    #[test]
    fn one_problem_is_reported_as_it_worded_itself() {
        let error = Error::Problems {
            first: PROBLEM.to_owned(),
            rest: 0,
        };

        assert_eq!(error.to_string(), PROBLEM);
    }

    #[test]
    fn further_problems_are_counted_rather_than_listed() {
        let error = Error::Problems {
            first: PROBLEM.to_owned(),
            rest: 3,
        };

        assert_eq!(error.to_string(), format!("{PROBLEM} (and 3 more like it)"));
    }

    #[test]
    fn every_message_is_one_line_so_it_prints_as_one() {
        let errors = [
            Error::WorkingDirectory {
                source: std::io::Error::other("boom"),
            },
            Error::Terminal {
                source: std::io::Error::other("boom"),
            },
            Error::Problems {
                first: PROBLEM.to_owned(),
                rest: 2,
            },
            Error::Manifest {
                source: ManifestError::Io {
                    path: PathBuf::from("/repo/.warlock/pacts.toml"),
                    source: std::io::Error::other("boom"),
                },
            },
        ];

        for error in errors {
            let message = error.to_string();
            assert!(!message.contains('\n'), "{error:?} wrapped: {message}");
            assert!(!message.is_empty(), "{error:?} said nothing");
        }
    }

    #[test]
    fn a_manifest_that_cannot_be_saved_says_so_in_the_engines_words() {
        let error = Error::Manifest {
            source: ManifestError::Io {
                path: PathBuf::from("/repo/.warlock/pacts.toml"),
                source: std::io::Error::other("permission denied"),
            },
        };

        assert_eq!(
            error.to_string(),
            "could not read or write `/repo/.warlock/pacts.toml`: permission denied"
        );
    }

    #[test]
    fn the_binary_is_named_warlock() {
        assert_eq!(env!("CARGO_BIN_NAME"), "warlock");
    }

    #[test]
    fn q_and_esc_quit_with_no_pact_running() {
        assert_eq!(
            action_for(press(KeyCode::Char('q')), false),
            Some(Action::Quit)
        );
        assert_eq!(action_for(press(KeyCode::Esc), false), Some(Action::Quit));
    }

    #[test]
    fn esc_cancels_the_pact_in_flight_while_q_and_ctrl_c_still_quit() {
        let ctrl_c = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);

        assert_eq!(
            action_for(press(KeyCode::Esc), true),
            Some(Action::CancelPact),
            "Esc during a pact stops the pact, not warlock"
        );
        assert_eq!(
            action_for(press(KeyCode::Char('q')), true),
            Some(Action::Quit),
            "and the ways out are still the ways out"
        );
        assert_eq!(action_for(ctrl_c, true), Some(Action::Quit));
    }

    #[test]
    fn esc_is_the_only_key_a_pact_in_flight_changes_the_meaning_of() {
        // Everything else the tree answers to keeps working while a run works,
        // which is the point of running it on a thread at all.
        let codes = [
            KeyCode::Char('q'),
            KeyCode::Up,
            KeyCode::Down,
            KeyCode::Char('k'),
            KeyCode::Char('j'),
            KeyCode::PageUp,
            KeyCode::PageDown,
            KeyCode::Char('g'),
            KeyCode::Char('G'),
            KeyCode::Char(' '),
            KeyCode::Char('o'),
            KeyCode::Char('f'),
            KeyCode::Char('p'),
            KeyCode::Tab,
            KeyCode::Char('x'),
        ];

        for code in codes {
            assert_eq!(
                action_for(press(code), true),
                action_for(press(code), false),
                "{code:?} means something different mid-pact"
            );
        }
    }

    #[test]
    fn ctrl_c_quits_but_a_bare_c_does_not() {
        let ctrl_c = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);

        assert_eq!(action_for(ctrl_c, false), Some(Action::Quit));
        assert_eq!(action_for(press(KeyCode::Char('c')), false), None);
    }

    #[test]
    fn ctrl_c_quits_with_caps_lock_or_shift_held() {
        // Some terminals report Ctrl-C as an upper-case `C` when shift or caps
        // lock is in play; it is still the key everyone reaches for to get out.
        let ctrl_shift_c = KeyEvent::new(
            KeyCode::Char('C'),
            KeyModifiers::CONTROL | KeyModifiers::SHIFT,
        );

        assert_eq!(action_for(ctrl_shift_c, false), Some(Action::Quit));
    }

    #[test]
    fn tab_moves_the_keys_to_the_other_pane() {
        assert_eq!(
            action_for(press(KeyCode::Tab), false),
            Some(Action::ToggleFocus)
        );
    }

    #[test]
    fn tab_means_the_same_thing_during_a_pact() {
        // Esc is the one key a run in flight re-reads, and focus is nothing to
        // do with a run: the tree stays drivable while a pact works
        // (WAR-21.05), so the key that says which pane is being driven has to
        // work then too.
        assert_eq!(
            action_for(press(KeyCode::Tab), true),
            Some(Action::ToggleFocus)
        );
    }

    #[test]
    fn releases_and_repeats_of_tab_move_no_focus() {
        // The same rule as every other key, and with the same consequence: a
        // release acted on would put focus straight back where the press took
        // it from, so one keystroke would look like none at all.
        for kind in [KeyEventKind::Release, KeyEventKind::Repeat] {
            let event = KeyEvent::new_with_kind_and_state(
                KeyCode::Tab,
                KeyModifiers::NONE,
                kind,
                KeyEventState::NONE,
            );

            assert_eq!(
                action_for(event, false),
                None,
                "{kind:?} of Tab should not move focus"
            );
        }
    }

    #[test]
    fn tab_is_the_only_key_that_moves_focus() {
        // Its neighbours on the keyboard and the keys it sits between in the
        // match arms above, plus the back-tab a terminal sends for Shift-Tab,
        // which is a keystroke of its own and is not bound.
        for code in [
            KeyCode::BackTab,
            KeyCode::Esc,
            KeyCode::Char('q'),
            KeyCode::Char(' '),
            KeyCode::Enter,
            KeyCode::Up,
            KeyCode::Char('p'),
        ] {
            assert_ne!(
                action_for(press(code), false),
                Some(Action::ToggleFocus),
                "{code:?} should not move focus"
            );
            assert_ne!(
                action_for(press(code), true),
                Some(Action::ToggleFocus),
                "{code:?} should not move focus mid-pact"
            );
        }
    }

    #[test]
    fn up_and_k_move_the_selection_up() {
        assert_eq!(
            action_for(press(KeyCode::Up), false),
            Some(Action::SelectPrevious)
        );
        assert_eq!(
            action_for(press(KeyCode::Char('k')), false),
            Some(Action::SelectPrevious)
        );
    }

    #[test]
    fn down_and_j_move_the_selection_down() {
        assert_eq!(
            action_for(press(KeyCode::Down), false),
            Some(Action::SelectNext)
        );
        assert_eq!(
            action_for(press(KeyCode::Char('j')), false),
            Some(Action::SelectNext)
        );
    }

    #[test]
    fn page_up_and_page_down_move_the_selection_by_a_screenful() {
        assert_eq!(
            action_for(press(KeyCode::PageUp), false),
            Some(Action::SelectPageUp)
        );
        assert_eq!(
            action_for(press(KeyCode::PageDown), false),
            Some(Action::SelectPageDown)
        );
    }

    #[test]
    fn lower_g_jumps_to_the_first_row_and_upper_g_to_the_last() {
        assert_eq!(
            action_for(press(KeyCode::Char('g')), false),
            Some(Action::SelectFirst)
        );
        assert_eq!(
            action_for(press(KeyCode::Char('G')), false),
            Some(Action::SelectLast)
        );
    }

    #[test]
    fn upper_g_still_jumps_to_the_last_row_with_shift_reported() {
        // Terminals disagree about whether the modifier rides along with the
        // upper-case letter; both spellings are the same keystroke.
        let shift_g = KeyEvent::new(KeyCode::Char('G'), KeyModifiers::SHIFT);

        assert_eq!(action_for(shift_g, false), Some(Action::SelectLast));
    }

    #[test]
    fn releases_and_repeats_of_the_new_movement_keys_move_nothing() {
        let codes = [
            KeyCode::PageUp,
            KeyCode::PageDown,
            KeyCode::Char('g'),
            KeyCode::Char('G'),
        ];

        for code in codes {
            for kind in [KeyEventKind::Release, KeyEventKind::Repeat] {
                let event = KeyEvent::new_with_kind_and_state(
                    code,
                    KeyModifiers::NONE,
                    kind,
                    KeyEventState::NONE,
                );

                assert_eq!(
                    action_for(event, false),
                    None,
                    "{kind:?} of {code:?} should not move anything"
                );
            }
        }
    }

    #[test]
    fn space_toggles_the_collapse_of_the_selected_directory() {
        assert_eq!(
            action_for(press(KeyCode::Char(' ')), false),
            Some(Action::ToggleCollapsed)
        );
    }

    #[test]
    fn releases_and_repeats_of_space_collapse_nothing() {
        // The same rule as every other key: a release acted on would expand
        // again what the press had just collapsed, so one keystroke would look
        // like none at all.
        for kind in [KeyEventKind::Release, KeyEventKind::Repeat] {
            let event = KeyEvent::new_with_kind_and_state(
                KeyCode::Char(' '),
                KeyModifiers::NONE,
                kind,
                KeyEventState::NONE,
            );

            assert_eq!(
                action_for(event, false),
                None,
                "{kind:?} of space should not collapse anything"
            );
        }
    }

    #[test]
    fn space_is_the_only_key_that_collapses() {
        // Neighbours on the keyboard and in the match arms above, in case a
        // space ever gets typed into the wrong pattern.
        for code in [
            KeyCode::Enter,
            KeyCode::Tab,
            KeyCode::Char('s'),
            KeyCode::Char('p'),
            KeyCode::Char('g'),
        ] {
            assert_ne!(
                action_for(press(code), false),
                Some(Action::ToggleCollapsed),
                "{code:?} should not collapse anything"
            );
        }
    }

    #[test]
    fn o_toggles_the_pacted_only_filter() {
        assert_eq!(
            action_for(press(KeyCode::Char('o')), false),
            Some(Action::TogglePactedOnly)
        );
    }

    #[test]
    fn releases_and_repeats_of_o_filter_nothing() {
        // The same rule as space: a release acted on would restore the whole
        // tree the press had just narrowed, so one keystroke would look like
        // none at all.
        for kind in [KeyEventKind::Release, KeyEventKind::Repeat] {
            let event = KeyEvent::new_with_kind_and_state(
                KeyCode::Char('o'),
                KeyModifiers::NONE,
                kind,
                KeyEventState::NONE,
            );

            assert_eq!(
                action_for(event, false),
                None,
                "{kind:?} of o should not filter anything"
            );
        }
    }

    #[test]
    fn o_is_the_only_key_that_filters() {
        // Its neighbours on the keyboard, the key it sits next to in the match
        // arms above, and its upper-case self, which this binding does not
        // answer to.
        for code in [
            KeyCode::Char('i'),
            KeyCode::Char('p'),
            KeyCode::Char('k'),
            KeyCode::Char('l'),
            KeyCode::Char('O'),
            KeyCode::Char(' '),
        ] {
            assert_ne!(
                action_for(press(code), false),
                Some(Action::TogglePactedOnly),
                "{code:?} should not filter anything"
            );
        }
    }

    #[test]
    fn f_toggles_the_files_inside_each_directory() {
        assert_eq!(
            action_for(press(KeyCode::Char('f')), false),
            Some(Action::ToggleFiles)
        );
    }

    #[test]
    fn releases_and_repeats_of_f_show_nothing() {
        // The same rule as space and `o`: a release acted on would hide again
        // the files the press had just shown, so one keystroke would look like
        // none at all.
        for kind in [KeyEventKind::Release, KeyEventKind::Repeat] {
            let event = KeyEvent::new_with_kind_and_state(
                KeyCode::Char('f'),
                KeyModifiers::NONE,
                kind,
                KeyEventState::NONE,
            );

            assert_eq!(
                action_for(event, false),
                None,
                "{kind:?} of f should not show anything"
            );
        }
    }

    #[test]
    fn f_is_the_only_key_that_shows_files() {
        // Its neighbours on the keyboard, the keys it sits between in the match
        // arms above, and its upper-case self, which this binding does not
        // answer to.
        for code in [
            KeyCode::Char('d'),
            KeyCode::Char('g'),
            KeyCode::Char('r'),
            KeyCode::Char('o'),
            KeyCode::Char('p'),
            KeyCode::Char('F'),
            KeyCode::Char(' '),
        ] {
            assert_ne!(
                action_for(press(code), false),
                Some(Action::ToggleFiles),
                "{code:?} should not show any files"
            );
        }
    }

    #[test]
    fn p_toggles_the_pact_on_the_selected_node() {
        assert_eq!(
            action_for(press(KeyCode::Char('p')), false),
            Some(Action::TogglePact)
        );
    }

    #[test]
    fn releases_and_repeats_of_p_write_nothing() {
        // The same rule as for movement, and it matters more here: a release
        // acted on would undo the pact the press had just written, and a held
        // key would rewrite the manifest as fast as the terminal repeats.
        for kind in [KeyEventKind::Release, KeyEventKind::Repeat] {
            let event = KeyEvent::new_with_kind_and_state(
                KeyCode::Char('p'),
                KeyModifiers::NONE,
                kind,
                KeyEventState::NONE,
            );

            assert_eq!(
                action_for(event, false),
                None,
                "{kind:?} should not write anything"
            );
        }
    }

    #[test]
    fn keys_with_no_meaning_here_are_ignored() {
        assert_eq!(action_for(press(KeyCode::Char('x')), false), None);
        assert_eq!(action_for(press(KeyCode::Enter), false), None);
        assert_eq!(action_for(press(KeyCode::Left), false), None);
    }

    #[test]
    fn releases_and_repeats_are_ignored_so_one_keystroke_moves_one_row() {
        for kind in [KeyEventKind::Release, KeyEventKind::Repeat] {
            let event = KeyEvent::new_with_kind_and_state(
                KeyCode::Down,
                KeyModifiers::NONE,
                kind,
                KeyEventState::NONE,
            );

            assert_eq!(
                action_for(event, false),
                None,
                "{kind:?} should not move anything"
            );
        }
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
    mod pacting {
        use std::cell::RefCell;
        use std::path::{Path, PathBuf};
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::mpsc;
        use std::{env, fs, process};

        use warlock_engine::{
            Agent, AgentError, AgentRequest, AgentResponse, Loaded, Manifest, Node, NodeState,
            PactEntry, Tree, Unwatched, decide_state, load_tree, repository_root, subtree_hash,
        };
        use warlock_tui::{Activities, Activity, App, ClaudeAgent, PactToggle};

        use warlock_tui::Cancel;

        use super::super::{
            CancelGuard, NOT_REFRESHED, PACT_CANCELLED, PACT_LOST, PactEvent, Running, Scope,
            Toggled, activity_port, apply_progress, apply_toggle, pact_press, run_pact, spawn_pact,
        };
        use super::ROOT;

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

        /// The toggle the app hands back for the directory at `relative`.
        fn toggle(scratch: &Scratch, relative: &str, pacted: bool) -> PactToggle {
            PactToggle {
                path: scratch.path(relative),
                pacted,
            }
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
            let app = App::from_tree(&tree).with_scope(&repo_root, tree.root_path());
            let scope = Scope {
                root: tree.root_path().to_path_buf(),
                repo_root,
            };
            (app, scope)
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
            toggle: &PactToggle,
            agent: &dyn Agent,
        ) {
            let before = app.clone();
            let (events, received) = mpsc::channel();
            run_pact(
                manifest,
                &scratch.root,
                toggle,
                agent,
                &Cancel::new(),
                &events,
            );

            let mut pact = Some(Running {
                events: received,
                cancel: CancelGuard::new(),
                path: toggle.path.clone(),
                before,
            });
            apply_progress(&mut pact, app, manifest, scope);
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
            let outside = PactToggle {
                path: PathBuf::from("/elsewhere/crates/engine"),
                pacted: false,
            };
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
            toggle: &PactToggle,
            agent: &dyn Agent,
            cancel: &Cancel,
        ) -> Vec<PactEvent> {
            let (events, received) = mpsc::channel();
            run_pact(
                &Manifest::new(),
                &scratch.root,
                toggle,
                agent,
                cancel,
                &events,
            );
            drop(events);
            received.into_iter().collect()
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
                    PactEvent::Doing(_) | PactEvent::Finished(_) => None,
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
            // does not move, and then exactly one outcome and nothing after it.
            let [
                PactEvent::Starting {
                    directory: first,
                    position: 1,
                    total: 2,
                },
                PactEvent::Starting {
                    directory: second,
                    position: 2,
                    total: 2,
                },
                PactEvent::Finished(Ok(Toggled {
                    manifest,
                    granted: true,
                    message: None,
                })),
            ] = events.as_slice()
            else {
                panic!("the worker said: {events:?}");
            };

            assert_eq!(first, &scratch.path("crates/engine/src"));
            assert_eq!(second, &scratch.path("crates/engine"));
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
            let before = app.clone();

            assert_eq!(pact_press(&mut app, true), None, "no second pact");
            assert_eq!(
                app, before,
                "and no colour, no message and no selection moved"
            );

            // The same press, with nothing running, is the press that starts a
            // pact: same key, same app, different answer.
            let toggle = pact_press(&mut app, false).expect("a directory can be pacted");
            assert_eq!(toggle.path, PathBuf::from("/repo/crates"));
            assert!(toggle.pacted);
            assert_ne!(app, before, "the subtree it covers is painted");
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
                path: PathBuf::from("/repo/crates"),
                before: App::from_tree(&tree),
            };

            assert!(!watching.is_cancelled(), "a run in flight is not cancelled");

            drop(running);

            assert!(
                watching.is_cancelled(),
                "the run outlived the loop that started it"
            );
        }

        #[test]
        fn the_loop_takes_activities_off_the_channel_and_carries_on_as_before() {
            // Activities share the channel with the progress the footer draws,
            // so the loop has to read them or they would sit in it until the run
            // ends and hold up nothing but themselves. Reading them is all it
            // does with them here: the app that comes out the far side is the
            // one a run with no activities at all would have produced.
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
                path: PathBuf::from("/repo/crates"),
                before: before.clone(),
            });

            // A directory, then a pass talking about it, in the order a worker
            // sends them.
            for event in [
                PactEvent::Starting {
                    directory: PathBuf::from("/repo/crates"),
                    position: 1,
                    total: 1,
                },
                PactEvent::Doing(Activity::Tool {
                    name: "Bash".to_owned(),
                    detail: Some("cargo test".to_owned()),
                }),
                PactEvent::Doing(Activity::Thinking),
                PactEvent::Doing(Activity::Cost { usd: 1.5 }),
            ] {
                events.send(event).expect("the loop is still listening");
            }
            apply_progress(&mut pact, &mut app, &mut manifest, &nowhere());

            assert!(pact.is_some(), "a run that is talking is still running");
            let mut in_flight = before.clone();
            in_flight.set_pact_in_flight("/repo/crates", 1, 1);
            assert_eq!(
                app, in_flight,
                "the footer says what it would have said, and nothing else moved"
            );

            // And the outcome behind them lands exactly as it always did.
            events
                .send(PactEvent::Finished(Ok(Toggled {
                    manifest: Manifest::new(),
                    granted: true,
                    message: None,
                })))
                .expect("the loop is still listening");
            apply_progress(&mut pact, &mut app, &mut manifest, &nowhere());

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
                path: PathBuf::from("/repo/crates"),
                before: before.clone(),
            });

            events
                .send(PactEvent::Starting {
                    directory: PathBuf::from("/repo/crates"),
                    position: 1,
                    total: 1,
                })
                .expect("the loop is still listening");
            apply_progress(&mut pact, &mut app, &mut manifest, &nowhere());

            assert!(pact.is_some(), "a run that has only started is still on");
            assert!(
                app.pact_line().is_some(),
                "and the footer says where it has got to"
            );

            // The worker's end goes away with no `Finished` behind it.
            drop(events);
            apply_progress(&mut pact, &mut app, &mut manifest, &nowhere());

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
                path: PathBuf::from("/repo/crates"),
                before: before.clone(),
            });

            events
                .send(PactEvent::Finished(Ok(Toggled {
                    manifest: Manifest::new(),
                    granted: true,
                    message: None,
                })))
                .expect("the loop is still listening");
            // There is no repository at `/repo/crates`, so the reload at the
            // bottom of this call fails.
            apply_progress(&mut pact, &mut app, &mut manifest, &nowhere());

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
                    path: PathBuf::from("/repo/crates"),
                    before,
                });

                events
                    .send(PactEvent::Finished(Ok(Toggled {
                        manifest: Manifest::new(),
                        granted: true,
                        message,
                    })))
                    .expect("the loop is still listening");
                // Nothing is on disk at `/repo/crates`, so the reload has its
                // own line to offer in both cases.
                apply_progress(&mut pact, &mut app, &mut manifest, &nowhere());
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
                path: scratch.path("crates/engine"),
                before: app.clone(),
            });
            events
                .send(PactEvent::Finished(Ok(Toggled {
                    manifest: manifest.clone(),
                    granted: true,
                    message: None,
                })))
                .expect("the loop is still listening");
            apply_progress(&mut pact, &mut app, &mut manifest, &scope);

            let message = app.message().expect("problems are reported").to_owned();
            fs::set_permissions(&unreadable, fs::Permissions::from_mode(0o644))
                .expect("chmods back");

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
    }
}
