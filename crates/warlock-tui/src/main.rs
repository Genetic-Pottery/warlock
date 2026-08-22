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
//! Those events are also what fills the panel. The press that really starts a
//! run opens an [`warlock_tui::Account`] on the app — one pact, one account, so
//! the next run clears the last one — each directory the worker names opens a
//! section of it, and everything a pass is seen doing lands under the section it
//! belongs to. Both halves are [`apply_progress`], which is handed the instant
//! it is called at rather than reading a clock, and so is the draw above it: the
//! newest line of the live section counts up against that instant, which is what
//! makes a pass that thinks for a minute look like something is happening. The
//! loop's existing hundred-millisecond round is the whole of the tick — there is
//! no timer, no second thread and no redraw on a schedule of its own.
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
//!
//! The third and last reason to reload is that the disk moved without anybody
//! here pressing a key, and it is what [`Watched`] is for. A watcher started
//! beside the first load reports every path that changes under the tree's root
//! and at `.warlock/pacts.toml`; the loop drains it once a round, holds each
//! path against the directories the last successful load produced — the walk is
//! the whole filter, so a `cargo build` writing into a directory no walk
//! produced costs one comparison a path and nothing else — and asks a
//! [`WatchPolicy`] whether the tree is owed a reload yet. When it is, the same
//! [`reload_tree`] runs, on this thread, and the tree it hands back becomes the
//! filter the next round holds paths against. So a file saved in another window
//! turns its directory yellow with no keystroke instead of waiting for a
//! relaunch. Two things this reason gives way to: a pact in flight, whose
//! documents set off events the run's own end-of-run reload already answers, so
//! the trigger is remembered and nothing is read twice; and warlock itself,
//! since a watcher that will not start is one line on the footer rather than a
//! way out of [`run`] — warlock with no live updates is warlock as it was.
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
use std::{env, fmt, fs, panic, thread};

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
    PactObserver, PactProblem, PactedSubtree, Pacting, Tree, load_tree, manifest_path,
    pact_subtree, repository_root, to_manifest_path, unpact_subtree,
};
use warlock_tui::{
    Activities, Activity, App, Cancel, ClaudeAgent, Outcome, PactToggle, Section, Watch,
    WatchPolicy, Watching, draw, panel_height, reseat_on, tree_height,
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

/// What the footer says when no watcher could be started, ahead of the
/// operating system's own reason for it.
///
/// It says what the reader lost, which is the noticing and nothing else: every
/// key still works, a pact still re-reads the tree when its run ends, and the
/// rows under this line are the ones the load produced — true, and true for as
/// long as nothing else writes to the repository. What will not happen is a row
/// appearing because somebody saved a file in another window.
///
/// Worded as a fact about this session rather than as a failure, because
/// nothing the reader asked for failed, and said once when the watcher is asked
/// for rather than on every frame: a line that is re-set ten times a second is a
/// line that talks over everything else the footer has to say.
const NOT_WATCHING: &str = "live updates are off; the tree is the one loaded at startup";

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
///
/// The round has a third thing in it now: what the disk did while the loop was
/// waiting on a keystroke. [`Watched`] is drained and asked once a round, after
/// a run that ended has had its own reload, and a watcher that could not be
/// started is a line put on the footer here — once, before the loop — and never
/// an error out of this function.
fn run() -> Result<(), Error> {
    let (mut app, scope, tree) = load_app()?;
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
    // Asked for once, over the tree the load just produced, and kept for as
    // long as warlock runs — dropping it stops the watch. Whether it was
    // granted is a fact for the footer and nothing more, which is why this is
    // not a `?`: warlock with no live updates is warlock as it was.
    let mut watched = Watched::start(&scope, &tree);
    // Said here rather than in the loop, so it is one line said once and not a
    // line re-set ten times a second. It gives way to anything the app already
    // has to say, exactly as the reload's own line does — see [`note`].
    if let Some(line) = watched.off_note() {
        note(&mut app, line);
    }
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
                // `None` needs nothing done about it, and it covers two cases
                // that are alike in exactly this way: both have already said
                // their piece on the app, and the next frame draws it. A refused
                // toggle put its sentence in `App::message`; a press while a
                // pact is in flight started nothing — a second pact over a tree
                // the first one is still writing to would be two runs racing for
                // the same documents and the same manifest — and said so by
                // setting the flag that words `App::pact_line` as already
                // running. Neither is this arm's to explain — see `pact_press`.
                Some(Action::TogglePact) => {
                    // Copied before the toggle paints anything, because the
                    // toggle is no longer its own undo: it puts a whole subtree
                    // into one state, and the states it painted over were not
                    // all the same one. The copy is a list of rows and a tally,
                    // and it is taken once per press of one key.
                    let before = app.clone();
                    // The instant the key was pressed is the instant the run
                    // starts, and the account counts its clocks from it: a run
                    // is as old as the keystroke that asked for it, not as old
                    // as the first thing the model got round to saying.
                    if let Some(toggle) = pact_press(&mut app, pact.is_some(), Instant::now()) {
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

        // The clock is read here rather than inside the two calls below for the
        // same reason it is read before the draw: this file owns the clock and
        // everything under it is a function of the instant it is handed. What
        // that instant means is when the events being drained now landed on
        // screen, which is within one `POLL_INTERVAL` of when they were sent.
        let now = Instant::now();

        // Read before the drain, because the drain is what ends a run: a pact
        // that is `Some` here and `None` below finished in this round, and its
        // own reload has already read the tree.
        let running = pact.is_some();
        // Every frame, whether or not a key was pressed: this is the only place
        // anything the worker says reaches the screen, and it has to keep up
        // with a thread that is not waiting for it. The scope the app was
        // loaded at is handed to it because the end of a run re-reads the tree
        // — see [`reload_tree`].
        let reloaded = apply_progress(&mut pact, &mut app, &mut manifest, &scope, now);
        if running && pact.is_none() {
            // The run's documents are exactly the sort of thing the watcher
            // reports, so the events it set off are already sitting in the
            // policy. They are answered by the reload that just happened rather
            // than by one of their own, and the tree it read becomes the filter.
            watched.caught_up(reloaded.as_ref(), now);
        }
        // And then what everything else did to the disk. Nothing is read again
        // while a pact is in flight — the trigger keeps until the run's own
        // reload above — and nothing at all is read when the disk has been
        // still, which is almost every round.
        watched.round(&mut app, &scope, pact.is_some(), now);
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
/// file row — has no run over it and so has its say the ordinary way, in
/// [`App::message`](warlock_tui::App::message), which `App::toggle_pact` has
/// already written by the time this returns.
///
/// A function rather than a guard in the match arm so that "a second press
/// changes nothing" is a property a test can hold the app up against, rather
/// than something only an event loop with a terminal attached could show.
///
/// A press that really does start a run is also where the panel's account
/// begins, at `at`: one pact, one account, so a new run clears whatever the last
/// one left rather than appending to it. It happens here rather than in the
/// event loop so that "the account a press starts is empty" is a property of the
/// same function, and it happens on this path only — a press turned down for a
/// file row, and a press while a run is in flight, leave the last run's account
/// on screen because neither of them started anything.
///
/// An un-pact does not start one either, and that is the one case where the
/// keystroke does something and the panel does not move. Un-pacting is manifest
/// arithmetic that is over before the next frame: it runs no pass, reports no
/// activity and has nothing to account for, so wiping the record of the run that
/// wrote those documents would be spending the panel on a keystroke with nothing
/// to say.
fn pact_press(app: &mut App, in_flight: bool, at: Instant) -> Option<PactToggle> {
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
fn apply_progress(
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
                app.set_pact_in_flight(directory, position, total);
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
fn restore(app: &mut App, mut before: App, message: impl Into<String>) {
    before.take_account_from(app);
    *app = before;
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
///
/// The tree that was read comes back with it, and `None` when none was. It is
/// what the watcher's filter is rebuilt from — the directories of the walk that
/// produced what is now on screen — and it is handed back rather than looked up
/// again because this function is the only thing that has it. Nothing else about
/// the view is in it: the re-seating above is the whole of the restoration, here
/// as it is after a pact.
fn reload_tree(app: &mut App, scope: &Scope) -> Option<Tree> {
    match load_tree(&scope.root) {
        Ok(Loaded { tree, problems }) => {
            *app = reseat_on(app, &tree).with_scope(&scope.repo_root, tree.root_path());
            // The same count, in the same words, as the startup load that
            // refuses to draw a tree with problems in it: one problem quoted
            // and the rest counted. A node the engine could not hash is
            // already stale on screen, so this line is the only place the
            // number of them appears.
            if let Some(counted) = Error::from_problems(&problems) {
                note(app, counted.to_string());
            }
            Some(tree)
        }
        // Flattened for the same reason `Error` flattens it: a manifest that
        // will not parse arrives as the TOML parser's several lines, and the
        // footer is one.
        Err(source) => {
            note(
                app,
                format!("{NOT_REFRESHED}: {}", one_line(&source.to_string())),
            );
            None
        }
    }
}

/// Say `line` on the footer, unless something is already saying something
/// there.
///
/// The footer's precedence in one place, because there are now two lines that
/// have it: how the redraw went ([`reload_tree`]) and whether the disk is being
/// watched at all ([`NOT_WATCHING`]). Both give way to whatever else is on the
/// line, which in practice means a pact's own message: what a run made of the
/// subtree the reader asked for is the news, and how warlock is keeping itself
/// up to date is housekeeping. Precedence, not merging — two sentences joined by
/// a semicolon would be a line nobody reads to the end of.
fn note(app: &mut App, line: impl Into<String>) {
    if app.message().is_none() {
        app.set_message(line);
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

/// Everything the loop keeps about the disk moving under it.
///
/// Three things, and no thread of its own: a [`Watching`] — which is a watcher
/// or the reason there is none — the [`WatchPolicy`] that decides what the paths
/// it reports are worth, and the manifest's path, which is the one path that
/// counts without any walk ever having produced it.
///
/// Nothing here decides *which* paths matter or *when* to act on them; both of
/// those are the library's questions — [`WatchPolicy`] answers them as values,
/// with no clock and no disk. What this type adds is the one thing that has to
/// happen on
/// this thread: reading the tree again, through the same [`reload_tree`] a pact
/// ends with, and handing the tree it read back to the policy so the next
/// round's filter is the walk that produced what is on screen.
struct Watched {
    /// The watcher, or the reason the operating system would not start one.
    watching: Watching,
    /// The filter and the timing rules, over the last successful load's walk.
    policy: WatchPolicy,
    /// `.warlock/pacts.toml`, resolved once. It is hidden, so no walk produces
    /// it and [`NodeSet`](warlock_tui::NodeSet) rejects it — and yet a pact
    /// granted or dropped in another window changes the colour of every row on
    /// screen while nothing inside the tree has moved. So it is compared for by
    /// name here, which is the only rule of this file's own about a path, and it
    /// is a path rather than a pattern.
    manifest: PathBuf,
}

impl Watched {
    /// Start watching `scope`, filtering against the walk `tree` came from.
    ///
    /// The watcher is over [`Scope::root`] — the path the load came back rooted
    /// at, so warlock run in a subdirectory hears about that subdirectory and
    /// not about a build in a sibling crate — and over the manifest under
    /// [`Scope::repo_root`]. Whether it started is not asked here: the answer is
    /// kept as a value and said once, on the footer, by whoever is drawing.
    fn start(scope: &Scope, tree: &Tree) -> Self {
        Self {
            watching: Watch::start(&scope.root, &scope.repo_root),
            policy: WatchPolicy::new(tree),
            manifest: manifest_path(&scope.repo_root),
        }
    }

    /// The one line the footer owes when nothing is being watched, or `None`
    /// when something is.
    ///
    /// Asked once, before the loop starts. It is a question about how warlock
    /// was started rather than about anything happening now, so asking it every
    /// frame would be re-answering a fact that cannot change and re-writing a
    /// line the reader has already read past.
    fn off_note(&self) -> Option<String> {
        match &self.watching {
            Watching::Live(_) => None,
            // Flattened for the same reason the reload's line is: what the
            // operating system says can run to several lines, and the footer is
            // one.
            Watching::Off(reason) => Some(format!("{NOT_WATCHING}: {}", one_line(reason))),
        }
    }

    /// One round of the event loop's watching: hear what the disk did, and read
    /// the tree again if that is now owed. Answers whether it read it.
    ///
    /// Every path drained is offered to the policy, which rejects the ones no
    /// walk produced — the whole of the cost of a `cargo build` under a watched
    /// root, thousands of paths compared against a set and dropped. The manifest
    /// is the exception it cannot see for itself, and it is the only one.
    ///
    /// `in_flight` is a pact running somewhere else, and it does not stop the
    /// draining, only the reloading: the events a run's own documents set off
    /// are remembered by the policy and answered by the reload at the end of the
    /// run ([`caught_up`](Watched::caught_up)), so a run whose documents moved
    /// the disk reads the tree once, at the end, rather than twice. Reloading
    /// under a run would also be reloading a tree the run is still writing to.
    fn round(&mut self, app: &mut App, scope: &Scope, in_flight: bool, now: Instant) -> bool {
        if let Watching::Live(watch) = &mut self.watching {
            for path in watch.drain() {
                if !self.policy.saw(&path, now) && path == self.manifest {
                    self.policy.accepted(now);
                }
            }
        }

        if in_flight || !self.policy.due(now) {
            return false;
        }
        let tree = reload_tree(app, scope);
        // The clock is read again, and this is the one place in the loop that
        // does: everything else in a round is over in microseconds, while the
        // load between these two lines is a walk and a hash per pacted subtree.
        // What the policy measures from here is the quiet period owed to
        // anything that moved *during* that load, and measuring it from before
        // the load would be starting the wait before the events it is waiting
        // for could arrive.
        self.caught_up(tree.as_ref(), Instant::now());
        true
    }

    /// Somebody else read the tree, at `at`: the reload at the end of a run.
    ///
    /// Two things, and the second is why this exists at all. The policy is told
    /// that a reload happened, which discharges whatever it was owed — an event
    /// from a pact's own documents is answered by the reload that came after it
    /// — and it is re-seated on the tree that reload produced, so the next
    /// round's filter is the walk behind what is on screen rather than the walk
    /// behind what used to be. A load that failed hands over `None`: the burst
    /// is still discharged, because the tree was read and this is as fresh as
    /// the view is going to get, and the filter stays on the last walk that
    /// worked, which is the one the rows still come from.
    fn caught_up(&mut self, tree: Option<&Tree>, at: Instant) {
        self.policy.reload_started();
        if let Some(tree) = tree {
            self.policy.follow(tree);
        }
        self.policy.reload_finished(at);
    }
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
    let (next, granted, message, refusals) = if toggle.pacted {
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
        // And the same failures a second time, per directory: the footer takes
        // one of them and the panel takes all of them, because the panel has a
        // section for each and can say which is which.
        let refusals = failures.iter().map(Refusal::from).collect();
        (
            manifest,
            granted,
            pact_message(&failures, &problems),
            refusals,
        )
    } else {
        // Un-pacting is pure manifest editing — no walk, no pass, no hash, and
        // every `WARLOCK.md` left where it is — so the only thing it can refuse
        // is a path the manifest has no spelling for. The app has already said
        // what un-pacting leaves behind, and nothing here talks over it.
        let next = unpact_subtree(&toggle.path, repo_root, manifest)
            .map_err(|source| Error::Manifest { source }.to_string())?;
        (next, false, None, Vec::new())
    };

    next.save(repo_root)
        .map_err(|source| Error::Manifest { source }.to_string())?;
    Ok(Toggled {
        manifest: next,
        granted,
        message,
        refusals,
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

/// The app state for the directory warlock was invoked from, the [`Scope`] it
/// was loaded at, and the tree it was built from.
///
/// The tree comes back as well as the app because the app is not a tree: its
/// rows are filtered by what the reader has toggled, and the directories one
/// walk produced are what the watcher's filter is ([`Watched`]). Handing it over
/// here is what keeps that filter and the rows on screen born of the same walk.
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
fn load_app() -> Result<(App, Scope, Tree), Error> {
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
    Ok((app, scope, tree))
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
        use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
        use std::sync::mpsc::{self, Sender};
        use std::time::{Duration, Instant};
        use std::{env, fs, process};

        use warlock_engine::{
            Agent, AgentError, AgentRequest, AgentResponse, Loaded, Manifest, Node, NodeState,
            PactEntry, Tree, Unwatched, decide_state, load_tree, repository_root, subtree_hash,
        };
        use warlock_tui::{
            Account, Activities, Activity, App, ClaudeAgent, Line, PactToggle, Section,
        };

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
                    ..
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
        fn a_run_in_flight(
            base: Instant,
        ) -> (App, App, Manifest, mpsc::Sender<PactEvent>, Running) {
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
                path: PathBuf::from("/repo/crates"),
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
                panel_text(&app, base),
                ["engine"],
                "the section is open and empty"
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
                path: PathBuf::from("/repo/crates"),
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
                path: PathBuf::from("/repo/crates"),
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
            let (events, received) = mpsc::channel();
            let agent = agent(&events);
            run_pact(
                &Manifest::new(),
                &scratch.root,
                &toggle(scratch, relative, true),
                &agent,
                cancel,
                &events,
            );
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
            let (events, received) = mpsc::channel();
            let mut pact = Some(Running {
                events: received,
                cancel,
                path: scope.root.clone(),
                before: app.clone(),
            });
            for (frame, event) in said.into_iter().enumerate() {
                let frame = u64::try_from(frame).expect("a run of fewer than 2^64 events");
                events.send(event).expect("the loop is still listening");
                apply_progress(&mut pact, app, manifest, scope, at(base, frame * FRAME));
            }
            assert!(pact.is_none(), "the run reported its outcome and is over");
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
                    "0:40 thinking".to_owned(),
                    format!(
                        "0:40 wrote crates/engine/src/WARLOCK.md — {} bytes, $0.25",
                        document_bytes(&scratch, "crates/engine/src")
                    ),
                    "crates/engine".to_owned(),
                    "0:20 Read crates/engine".to_owned(),
                    "0:40 thinking".to_owned(),
                    format!(
                        "0:40 wrote crates/engine/WARLOCK.md — {} bytes, $0.25",
                        document_bytes(&scratch, "crates/engine")
                    ),
                    // Nine frames of ten seconds: the run started with the first
                    // and ended with the ninth. The total is the two passes
                    // added up, and there is no `incomplete` on it because both
                    // of them said what they cost.
                    "pact finished — 2 directories, 1:20, $0.50".to_owned(),
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
                    "0:40 thinking".to_owned(),
                    "0:40 cancelled — $0.25 spent".to_owned(),
                    // Four directories and not the five the subtree holds: the
                    // descent stopped, and the account counts what it reached.
                    "pact finished — 4 directories, 2:40, $1.00".to_owned(),
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
                        "0:40 wrote {directory}/WARLOCK.md — {} bytes, $0.25",
                        document_bytes(&scratch, directory)
                    ),
                    "a section above the cancel keeps the ending it earned"
                );
            }
            // And the footer says what it has always said about a stopped run.
            assert_eq!(app.message(), Some(PACT_CANCELLED));
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
                    "0:30 thinking".to_owned(),
                    format!(
                        "0:30 wrote crates/engine/src/WARLOCK.md — {} bytes, no cost reported",
                        document_bytes(&scratch, "crates/engine/src")
                    ),
                    "crates/engine".to_owned(),
                    "0:20 Read crates/engine".to_owned(),
                    "0:40 thinking".to_owned(),
                    format!(
                        "0:40 wrote crates/engine/WARLOCK.md — {} bytes, $0.25",
                        document_bytes(&scratch, "crates/engine")
                    ),
                    "pact finished — 2 directories, 1:10, \
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
                path: PathBuf::from("/repo/crates"),
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
                    path: PathBuf::from("/repo/crates"),
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
                path: scratch.path("crates/engine"),
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

            use super::super::super::{NOT_WATCHING, POLL_INTERVAL, Watched, note};
            use super::{
                CancelGuard, Canned, Loaded, Manifest, NodeState, PactEvent, Running, Scope,
                Toggled, Unwatched, apply_progress, apply_toggle, load, load_tree, mpsc,
                one_crate_to_load, state_of, toggle,
            };

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
                let header = app.header().to_owned();
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
                assert_eq!(app.header(), header, "the header was rewritten");
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
                    path: scratch.path("crates/engine"),
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
}
