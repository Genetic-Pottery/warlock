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

use std::io::{self, Stdout};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::time::Duration;
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
use warlock_tui::{App, ClaudeAgent, PactToggle, draw, tree_height};

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
    let (mut app, repo_root) = load_app()?;
    // Loaded before the terminal is touched, for the same reason the tree is:
    // a manifest that will not parse should say so on the normal screen. This
    // is a second read of the file the loader already parsed, which is cheap
    // and keeps the front end from reaching into the loader's internals for a
    // value it needs to keep and edit.
    let mut manifest = load_manifest(&repo_root)?;
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
        app.set_viewport_height(tree_height(guard.terminal.size()?));
        guard.terminal.draw(|frame| draw(frame, &app))?;

        // Waited on rather than blocked on. Nothing is drawn while this thread
        // sits here, so the wait has to end whether or not anybody presses
        // anything: a pact reports its progress over a channel that only the
        // bottom of this loop reads, and a progress line that waits for a
        // keystroke to appear is worse than none at all.
        if event::poll(POLL_INTERVAL)?
            && let Event::Key(key) = event::read()?
        {
            match action_for(key) {
                Some(Action::Quit) => return Ok(()),
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
                        pact = Some(Running {
                            events: spawn_pact(&manifest, &repo_root, &toggle, &agent),
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
        // with a thread that is not waiting for it.
        apply_progress(&mut pact, &mut app, &mut manifest);
    }
}

/// A pact being run by a worker thread, from the point of view of the thread
/// drawing the screen.
///
/// Three things, and no handle to join: what the worker has to say, which
/// subtree it was started on, and what the tree looked like before the keystroke
/// painted that subtree yellow.
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
struct Running {
    /// Progress and, once, the outcome. Closed by the worker dropping its end,
    /// which is how a panicked worker is noticed.
    events: Receiver<PactEvent>,
    /// The directory the pact key was pressed on, whose subtree the run covers.
    path: PathBuf,
    /// The app as it stood before the toggle painted the subtree.
    before: App,
}

/// What a worker thread has to say for itself.
///
/// Two things, in this order: one [`PactEvent::Starting`] per directory as the
/// run reaches it, and then exactly one [`PactEvent::Finished`]. Nothing else is
/// sent, and nothing is sent after the outcome — the worker drops its end of the
/// channel and stops.
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
    /// The run is over, however it went: exactly what [`apply_toggle`] returned.
    Finished(Result<Toggled, String>),
}

/// The engine's progress port, wired to a channel.
///
/// The one adapter between an operation that knows which directory it is on and
/// a thread that can draw. It answers [`Pacting::Continue`] always: stopping a
/// pact is a decision made by a person at a keyboard, and there is nobody at
/// this end of the channel to make it.
///
/// A send that fails means the event loop has dropped its receiver, which means
/// warlock is on its way out. It is deliberately ignored rather than turned into
/// a stop: the process is ending, the work in flight is about to end with it,
/// and there is nothing here worth reporting to a screen that is already gone.
struct Reporting<'a> {
    /// The event loop's end of the channel.
    events: &'a Sender<PactEvent>,
}

impl PactObserver for Reporting<'_> {
    fn starting(&mut self, directory: &Path, position: usize, total: usize) -> Pacting {
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
/// channel. That is what makes the thread safe to abandon: the loop can return
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
) -> Receiver<PactEvent> {
    let (events, received) = mpsc::channel();
    let (manifest, repo_root, toggle, agent) = (
        manifest.clone(),
        repo_root.to_path_buf(),
        toggle.clone(),
        agent.clone(),
    );
    thread::spawn(move || run_pact(&manifest, &repo_root, &toggle, &agent, &events));
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
/// news that everything worked.
fn run_pact(
    manifest: &Manifest,
    repo_root: &Path,
    toggle: &PactToggle,
    agent: &dyn Agent,
    events: &Sender<PactEvent>,
) {
    let outcome = apply_toggle(
        manifest,
        repo_root,
        toggle,
        agent,
        &mut Reporting { events },
    );
    // Ignored for the same reason `Reporting`'s sends are: a receiver that has
    // gone away is an application that is quitting.
    let _ = events.send(PactEvent::Finished(outcome));
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
fn apply_progress(pact: &mut Option<Running>, app: &mut App, manifest: &mut Manifest) {
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
/// repository root it sits in.
///
/// The root is handed back rather than dropped once the header is built,
/// because it is also where the manifest lives: every pact written during the
/// run is written relative to this path, and finding it is a walk up the
/// filesystem that should happen once.
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
/// on screen that nothing on disk backs up.
fn load_app() -> Result<(App, PathBuf), Error> {
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
    Ok((app, repo_root))
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

/// The action `key` asks for, or `None` for a key that means nothing here.
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
/// handle it, nothing does.
fn action_for(key: KeyEvent) -> Option<Action> {
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
        KeyCode::Char('q') | KeyCode::Esc => Some(Action::Quit),
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
    fn q_and_esc_quit() {
        assert_eq!(action_for(press(KeyCode::Char('q'))), Some(Action::Quit));
        assert_eq!(action_for(press(KeyCode::Esc)), Some(Action::Quit));
    }

    #[test]
    fn ctrl_c_quits_but_a_bare_c_does_not() {
        let ctrl_c = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);

        assert_eq!(action_for(ctrl_c), Some(Action::Quit));
        assert_eq!(action_for(press(KeyCode::Char('c'))), None);
    }

    #[test]
    fn ctrl_c_quits_with_caps_lock_or_shift_held() {
        // Some terminals report Ctrl-C as an upper-case `C` when shift or caps
        // lock is in play; it is still the key everyone reaches for to get out.
        let ctrl_shift_c = KeyEvent::new(
            KeyCode::Char('C'),
            KeyModifiers::CONTROL | KeyModifiers::SHIFT,
        );

        assert_eq!(action_for(ctrl_shift_c), Some(Action::Quit));
    }

    #[test]
    fn up_and_k_move_the_selection_up() {
        assert_eq!(action_for(press(KeyCode::Up)), Some(Action::SelectPrevious));
        assert_eq!(
            action_for(press(KeyCode::Char('k'))),
            Some(Action::SelectPrevious)
        );
    }

    #[test]
    fn down_and_j_move_the_selection_down() {
        assert_eq!(action_for(press(KeyCode::Down)), Some(Action::SelectNext));
        assert_eq!(
            action_for(press(KeyCode::Char('j'))),
            Some(Action::SelectNext)
        );
    }

    #[test]
    fn page_up_and_page_down_move_the_selection_by_a_screenful() {
        assert_eq!(
            action_for(press(KeyCode::PageUp)),
            Some(Action::SelectPageUp)
        );
        assert_eq!(
            action_for(press(KeyCode::PageDown)),
            Some(Action::SelectPageDown)
        );
    }

    #[test]
    fn lower_g_jumps_to_the_first_row_and_upper_g_to_the_last() {
        assert_eq!(
            action_for(press(KeyCode::Char('g'))),
            Some(Action::SelectFirst)
        );
        assert_eq!(
            action_for(press(KeyCode::Char('G'))),
            Some(Action::SelectLast)
        );
    }

    #[test]
    fn upper_g_still_jumps_to_the_last_row_with_shift_reported() {
        // Terminals disagree about whether the modifier rides along with the
        // upper-case letter; both spellings are the same keystroke.
        let shift_g = KeyEvent::new(KeyCode::Char('G'), KeyModifiers::SHIFT);

        assert_eq!(action_for(shift_g), Some(Action::SelectLast));
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
                    action_for(event),
                    None,
                    "{kind:?} of {code:?} should not move anything"
                );
            }
        }
    }

    #[test]
    fn space_toggles_the_collapse_of_the_selected_directory() {
        assert_eq!(
            action_for(press(KeyCode::Char(' '))),
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
                action_for(event),
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
                action_for(press(code)),
                Some(Action::ToggleCollapsed),
                "{code:?} should not collapse anything"
            );
        }
    }

    #[test]
    fn o_toggles_the_pacted_only_filter() {
        assert_eq!(
            action_for(press(KeyCode::Char('o'))),
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
                action_for(event),
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
                action_for(press(code)),
                Some(Action::TogglePactedOnly),
                "{code:?} should not filter anything"
            );
        }
    }

    #[test]
    fn f_toggles_the_files_inside_each_directory() {
        assert_eq!(
            action_for(press(KeyCode::Char('f'))),
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
                action_for(event),
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
                action_for(press(code)),
                Some(Action::ToggleFiles),
                "{code:?} should not show any files"
            );
        }
    }

    #[test]
    fn p_toggles_the_pact_on_the_selected_node() {
        assert_eq!(
            action_for(press(KeyCode::Char('p'))),
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
                action_for(event),
                None,
                "{kind:?} should not write anything"
            );
        }
    }

    #[test]
    fn keys_with_no_meaning_here_are_ignored() {
        assert_eq!(action_for(press(KeyCode::Char('x'))), None);
        assert_eq!(action_for(press(KeyCode::Enter)), None);
        assert_eq!(action_for(press(KeyCode::Left)), None);
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

            assert_eq!(action_for(event), None, "{kind:?} should not move anything");
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
            Agent, AgentError, AgentRequest, AgentResponse, Manifest, Node, NodeState, PactEntry,
            Tree, Unwatched, decide_state, subtree_hash,
        };
        use warlock_tui::{App, ClaudeAgent, PactToggle};

        use super::super::{PactEvent, Toggled, apply_toggle, pact_press, run_pact};
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
                    seen: RefCell::new(Vec::new()),
                }
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
        /// `scratch` with `agent`, in the order the event loop would have
        /// drained it.
        ///
        /// The worker's body runs here, on the test's own thread, and its end of
        /// the channel is dropped before anything is read: what comes back is
        /// the whole sequence a real run sends, with none of the timing of one.
        fn events_of(scratch: &Scratch, toggle: &PactToggle, agent: &dyn Agent) -> Vec<PactEvent> {
            let (events, received) = mpsc::channel();
            run_pact(&Manifest::new(), &scratch.root, toggle, agent, &events);
            drop(events);
            received.into_iter().collect()
        }

        #[test]
        fn the_worker_names_each_directory_in_turn_and_then_says_how_the_run_went() {
            let scratch = one_crate("progress");
            let agent = Canned::new(&scratch, []);

            let events = events_of(&scratch, &toggle(&scratch, "crates/engine", true), &agent);

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
        fn a_run_that_reaches_no_model_comes_back_naming_the_program_it_looked_for() {
            let scratch = one_crate("no-agent");
            // A name nothing on any `PATH` answers to, standing in for a machine
            // with no `claude` installed. The spawn fails for every directory,
            // and the run still comes back as an outcome with a line for the
            // footer rather than as a crash or as silence.
            let missing = "warlock-claude-that-is-not-installed";
            let agent = ClaudeAgent::new().with_program(missing);

            let events = events_of(&scratch, &toggle(&scratch, "crates/engine", true), &agent);

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
    }
}
