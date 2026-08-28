//! Terminal front end for warlock.
//!
//! This binary is the thin, impure shell around the pure parts in
//! `warlock_tui`: it owns the terminal's lifecycle, the working directory it
//! was invoked from, and the event loop, and nothing else. It asks the engine
//! to load the tree for that directory and knows nothing about how one is
//! built; what a frame looks like is [`warlock_tui::draw`]'s business and how
//! the selection moves is [`App`]'s.
//!
//! This file is the loop itself; each of the loop's concerns lives in a
//! sibling module. What a keystroke or a click means is [`input`]'s, running a
//! pact on a worker thread and applying what it says is [`pacting`]'s, where
//! the tree came from and when it is re-read is [`session`]'s, the terminal's
//! setup and restoration is [`terminal`]'s, and the one-line errors `main`
//! prints are [`error`]'s. The paragraphs below describe how the loop drives
//! all of them, and each module's own doc says why it is shaped as it is.
//!
//! The one rule this shell exists to keep is that the terminal is restored on
//! every way out: a normal quit, an error returned up to `main`, and a panic.
//! Raw mode left switched on after exit means a shell that no longer echoes
//! what the user types, and that is not something they should have to know how
//! to fix. A pact runs on a thread of its own now, so a panic *there* is one of
//! those ways out too — and it is covered by the same process-wide hook, which
//! is why the hook is installed before anything else happens.
//!
//! The one long keystroke is the pact key, and it is the reason the loop below
//! is shaped the way it is. A subtree pact is minutes of model passes, so
//! pressing the key spawns a worker thread ([`pacting::spawn_pact`]) and hands back a
//! [`Receiver`] of what that worker has to say: which directory it is on, and,
//! once, how the whole thing went. The loop polls for a keystroke with a short
//! timeout instead of blocking on one, drains that channel every frame, and
//! draws — so the tree still scrolls, the footer's progress line still advances,
//! and the run lands on screen the moment the worker is done with it. Nothing
//! here waits on the worker: the manifest, the tree and the message are updated
//! from the events it sends, and the thread is never joined.
//!
//! The refresh key is the second long keystroke and is not a second anything
//! else: `r` asks the engine to describe only the stale directories under the
//! selected one, and it does so through the same [`start_run`], the same
//! channel, the same account and the same [`Running`] — which is what makes one
//! run at a time a fact rather than a rule. The two keys refuse each other by
//! that alone: whichever run is in flight, the other key's press finds a `Some`
//! here and says so on the line the reader is already watching.
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
//!
//! The loop answers a pointer as well as a keyboard, and it is the same
//! arrangement twice over. [`TerminalGuard`] asks the terminal to report its
//! mouse in the same breath as it takes the alternate screen, so the reporting
//! is switched off by the same [`restore_terminal`] every way out already runs
//! through; and an event that arrives is turned into an intention by
//! [`mouse_action`], which is [`input::action_for`] for the pointer — a function
//! of the event, the size this round measured, the app and the gate on the way
//! out, with no terminal in it. The wheel drives whichever pane the pointer is
//! over rather than whichever pane has the keys, a left click selects a row and
//! takes the keys with it, and everything else a mouse can send is read and
//! dropped, so a pointer swept across the screen changes nothing and costs no
//! more than the round it arrived in.
//!
//! There is one thing between a keystroke and the end of the session now, and
//! it is a question. Esc and `q` no longer return from [`run`]: with nothing
//! running they open the quit confirmation ([`QuitConfirm`]), which is drawn
//! over the frame and answered from the keyboard, and only a Yes returns. The
//! decision is [`press_for`]'s and not this file's — a key, the question's
//! state, the scope prompt's and whether a run is in flight go in, and what the
//! loop is to do comes out — so the whole gate is testable with nothing attached
//! to stdout, and the arms below are the four things that can come of a
//! keystroke: leave, move the question, type into the scope prompt — which this
//! loop does not open yet, so that road lands in the arm that does nothing — or
//! hand the key to the app. While either window is up the app hears nothing, the
//! pointer included: mouse events are read and dropped, so a click cannot select
//! a row behind a window that is about to close.
//!
//! Two keystrokes are deliberately outside the gate. Ctrl-C is answered before
//! the question is consulted, because in raw mode it is a key event rather than
//! a signal: routed through the dialog it would be an ordinary `c` with a
//! modifier riding along — one of the keys that change nothing — and the last
//! resort of a reader who wants out would be the one keystroke the dialog
//! swallowed. And a run in flight suppresses the gate entirely: Esc means cancel
//! for as long as there is something to cancel, so the reflex press this gate
//! exists for cannot reach the way out anyway, while `q` and Ctrl-C during a run
//! are what a reader reaches for having already decided — a question in front of
//! them would be a question asked of somebody who has answered it.
//!
//! One thing now happens before any of that, and it happens with nothing
//! attached to the terminal: the arguments are read. Zero of them is the whole
//! of warlock as it was — the panic hook, the loop, the alternate screen — and
//! anything else is answered here and exits. `init` writes an `AGENTS.md` at the
//! repository root and says which file it wrote; `config` prints the sigils this
//! machine holds for this repository and reads a line replacing them
//! ([`config`]); `-h` and `--help` print the one usage line; and every other
//! word, and every second argument, prints that same line on stderr and fails.
//! Refusing is the point of the last of those: warlock used to open the tree for
//! `warlock status`, which reads as the typed command having run. The decision
//! is [`intention_for`], a function of the arguments alone, so all of it is
//! testable with no terminal, no repository and no process to spawn.
//!
//! Both subcommands share one rule, and it is why they are dispatched here
//! rather than anywhere inside [`run`]: neither goes near the terminal. No
//! alternate screen, no raw mode and no panic hook — the hook exists to restore
//! a terminal these paths never take, and `config` reads its line in cooked
//! mode, which is also what makes Ctrl-C at its prompt an ordinary SIGINT that
//! ends the process before anything is written.
//!
//! The reader can hand the pointer back. `m` turns the terminal's reporting off
//! and on for the rest of the session ([`Action::ToggleMouseCapture`]); with it
//! off the terminal keeps its own text selection and no `Event::Mouse` arrives
//! at all, so nothing here needs a second gate. Whether it is on is this
//! thread's to know — the app is handed a copy of it every round so the footer
//! can name the key by what the next press does — and every way out still goes
//! through [`restore_terminal`], which turns reporting off whichever state the
//! toggle was left in.

use std::process::ExitCode;
use std::time::{Duration, Instant};
use std::{env, io};

use ratatui::crossterm::event::{self, DisableMouseCapture, EnableMouseCapture, Event};
use ratatui::crossterm::execute;
use warlock_engine::{Written, repository_root, write_agents_md};
use warlock_tui::{ClaudeAgent, Focus, QuitConfirm, ScopePrompt, draw, panel_height, tree_height};

mod config;
mod error;
mod input;
mod pacting;
mod session;
mod terminal;

use config::configure;
use error::Error;
use input::{Action, MouseAction, Pressed, mouse_action, press_for};
use pacting::{Running, Work, apply_progress, pact_press, refresh_press, start_run};
use session::{Watched, load_app, load_manifest, note};
use terminal::{TerminalGuard, install_panic_hook};

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

/// The whole of warlock's command line, in the one line it is printed as.
///
/// Printed on stdout when it was asked for and on stderr when it was not, and
/// the same string either way: a reader who typed something warlock does not
/// have should be shown exactly what a reader who asked for help is shown.
///
/// One line, because that is all there is to say — warlock is a terminal
/// interface with two subcommands, not a CLI with a manual — and because the
/// refusal path shares it, where several lines of help in answer to a typo would
/// bury the fact that nothing ran.
const USAGE: &str = "usage: warlock [init|config] — no arguments opens the tree; \
                     `init` writes AGENTS.md at the repository root; `config` sets \
                     the sigils this machine holds for it";

/// What `warlock init` wants the repository root for, as the tail of
/// [`Error::NoRepository`]'s sentence. `config`'s own tail is spelled beside
/// `config`, in its module.
const FOR_AGENTS_MD: &str = "write `AGENTS.md` at";

/// What `warlock init` says when there was no `AGENTS.md` and now there is one.
const CREATED: &str = "created";

/// What it says when there was one already and warlock's section in it is now
/// current — which includes the case where the file did not change, since
/// "updated" is true of the section either way and a reader running `init`
/// twice is not owed a third word for it.
const UPDATED: &str = "updated";

fn main() -> ExitCode {
    // Read before anything else happens and, deliberately, before anything
    // touches the terminal: `init`, help and a refusal all print on the ordinary
    // screen, and a program that entered the alternate screen to write one line
    // would tear it down around a message nobody saw.
    let outcome = match intention_for(env::args().skip(1)) {
        Intention::Tui => {
            // Before anything touches the terminal: a panic during setup has to
            // leave the terminal usable too.
            install_panic_hook();
            run()
        }
        Intention::Init => init(),
        // The second subcommand, dispatched here for the first one's reasons:
        // it prints on the ordinary screen and reads a line from stdin in cooked
        // mode, so nothing about it may touch the terminal — including the panic
        // hook, which exists to restore a terminal this path never takes.
        Intention::Config => configure(),
        // Asked for, so it goes to stdout and the exit status says it worked.
        Intention::Help => {
            println!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        // Not asked for, so the same line goes to stderr and the exit status
        // says nothing ran. This is the arm that stops `warlock status` opening
        // the tree as if the word had meant something.
        Intention::Refuse => {
            eprintln!("{USAGE}");
            return ExitCode::FAILURE;
        }
    };

    match outcome {
        Ok(()) => ExitCode::SUCCESS,
        // `run` has returned, so the guard inside it has already dropped and
        // the terminal is back to normal; only now is it worth printing
        // anything, because on the alternate screen nobody would ever see it.
        // `init` never went near the terminal, and prints through the same line
        // so that a failure looks the same however warlock was invoked.
        Err(error) => {
            eprintln!("warlock: {error}");
            ExitCode::FAILURE
        }
    }
}

/// What warlock was asked to do, as decided by its arguments and nothing else.
///
/// Five things, which is all a program with two subcommands has: open the tree,
/// write the file, set the sigils, say how it is invoked, or say that and fail.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Intention {
    /// No arguments: warlock as it has always been.
    Tui,
    /// `warlock init`.
    Init,
    /// `warlock config`.
    Config,
    /// `-h` or `--help`: [`USAGE`] on stdout, exit success.
    Help,
    /// Anything else at all: [`USAGE`] on stderr, exit failure.
    Refuse,
}

/// What `args` — the arguments after the program's own name — asks warlock to
/// do.
///
/// A handful of string comparisons rather than an argument parser, and
/// deliberately so: warlock's command line is one optional word, and a
/// dependency to compare one word against four is a dependency to keep up to
/// date forever. A second subcommand is a second arm below, which is the whole
/// of what it costs. Pure, so every arm is a test rather than a process to
/// spawn.
///
/// More than one argument is refused before the first is even looked at, which
/// is the rule that matters most here: `warlock init extra` typed by somebody
/// who meant something by `extra` must not run an `init` that silently ignored
/// it. `warlock config` takes no argument at all — the sigils are typed at its
/// prompt, where the line that clears them can be explained first — so
/// `warlock config data-plane` lands in the same refusal.
fn intention_for(args: impl IntoIterator<Item = impl AsRef<str>>) -> Intention {
    let mut args = args.into_iter();
    let Some(first) = args.next() else {
        return Intention::Tui;
    };
    if args.next().is_some() {
        return Intention::Refuse;
    }

    match first.as_ref() {
        "init" => Intention::Init,
        "config" => Intention::Config,
        "-h" | "--help" => Intention::Help,
        // `--version` was considered and is deliberately left out: it lands
        // here, in the refusal, along with everything else warlock does not
        // have. Warlock is not installed from a registry and nobody is
        // diagnosing a version skew in it yet, so the honest answer today is
        // that the flag does not exist. Adding it later is one arm above this
        // one and a `println!` of `env!("CARGO_PKG_VERSION")` — not a reason to
        // carry a half-answer in the meantime.
        _ => Intention::Refuse,
    }
}

/// `warlock init`: write the `AGENTS.md` at the repository root and say which
/// file was written.
///
/// Three steps and no policy of its own. The working directory says where to
/// start, [`repository_root`] walks up to the nearest ancestor with a `.git/`
/// — so running this from any subdirectory writes the one file in the right
/// place — and the engine does the writing, because the splice, the delimiters
/// and the text are all its business (see
/// [`write_agents_md`](warlock_engine::write_agents_md)).
///
/// Nothing here touches the terminal: what happened is one line on the ordinary
/// screen, and a failure is an [`Error`] returned to `main`, which prints it in
/// exactly the same place and shape as a tree that would not load.
fn init() -> Result<(), Error> {
    let working_dir = env::current_dir().map_err(|source| Error::WorkingDirectory { source })?;
    // Asked directly rather than through a load: `init` writes one file into a
    // repository that may never have been pacted, and walking the tree to find
    // its root would be reading every directory in it to answer a question about
    // ancestors.
    let root = repository_root(&working_dir).ok_or(Error::NoRepository {
        start: working_dir,
        wanted: FOR_AGENTS_MD,
    })?;

    let written = write_agents_md(&root).map_err(|source| Error::AgentsMd { source })?;
    // Asked as a question rather than matched arm by arm, because the engine's
    // enum is `#[non_exhaustive]`: there is one thing to distinguish here — a
    // file that did not exist before — and anything it gains later is a file
    // that did.
    let what = if matches!(written, Written::Created { .. }) {
        CREATED
    } else {
        UPDATED
    };
    println!("warlock: {what} `{}`", written.path().display());
    Ok(())
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
/// from here without joining the worker — see [`pacting::spawn_pact`] for why that is
/// safe — and the guard restores the terminal on the way out as it always did.
///
/// The round has a third thing in it now: what the disk did while the loop was
/// waiting on a keystroke. [`Watched`] is drained and asked once a round, after
/// a run that ended has had its own reload, and a watcher that could not be
/// started is a line put on the footer here — once, before the loop — and never
/// an error out of this function.
///
/// One thing now stands between a keystroke and the return: the question the
/// gate asks. It is a value on this stack rather than a field on the app, and
/// [`press_for`] is what decides what each key does to it — see the module docs
/// above for why Ctrl-C goes round it and why a run in flight suppresses it.
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
    // Whether the terminal is reporting its mouse, and the only record of it:
    // the guard has just asked it to, and `m` is the one thing that changes the
    // answer. It lives here rather than on the app because it is a fact about a
    // terminal — the app is handed a copy of it every frame for the footer's
    // sake, and copies of the app are taken and put back around a pact, which a
    // source of truth must survive.
    let mut mouse_captured = true;
    // The pact running somewhere else, when one is: everything this thread
    // needs to keep about a run it is not performing. `None` is the ordinary
    // state, and it is what the pact key checks before starting anything.
    let mut pact: Option<Running> = None;
    // The gate on the way out, closed as every session starts. It lives here
    // rather than on the app because it is state about this keystroke and the
    // next one rather than about what warlock is showing — which is also what
    // makes "answering No leaves the app exactly as it was" true by
    // construction: the app is never told the question was asked.
    let mut confirm = QuitConfirm::default();
    // The other question this loop can be carrying: the scope prompt, closed as
    // every session starts, and here for the reason the gate is — it is state
    // about this keystroke and the next one, and an `App` that has never heard
    // of it is an `App` that Esc cannot have changed.
    //
    // Not `mut` yet, and drawn without ever being opened: the key that opens it
    // is a slice of its own, and a `mut` on a binding nothing assigns to is a
    // warning this workspace treats as an error. The frame it makes while it is
    // closed is the frame warlock has always drawn.
    //
    // `prompt` and not `scope`: the `scope` this function already holds is the
    // session's — the repo root and what warlock was pointed at — and two things
    // by that name in one loop is one of them being read as the other.
    let prompt = ScopePrompt::default();

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
        // And told what the terminal is doing with the pointer, for the same
        // reason and in the same place: the footer names the `m` key by what the
        // next press of it will do, and the app cannot see a terminal. Every
        // frame rather than at the keystroke, so a view restored from the copy
        // taken before a pact — which is a copy of a flag that may have been
        // toggled since — is put right before it is drawn.
        app.set_mouse_captured(mouse_captured);
        // The instant this frame is being drawn at, read once and handed to the
        // renderer: the panel's newest clock counts up against it, so a frame
        // drawn with no event waiting still shows a run that is moving. See
        // `draw`.
        //
        // The gate on the way out is handed in beside the app because the app
        // has never heard of it (see `QuitConfirm`): closed, it changes nothing
        // about the frame; open, it is a small window drawn over the middle of
        // it with everything behind it cleared. The scope prompt goes in beside
        // it for the same reason and is drawn the same way — by reference, since
        // it carries the text somebody is typing.
        guard
            .terminal
            .draw(|frame| draw(frame, &app, Instant::now(), confirm, &prompt))?;

        // Waited on rather than blocked on. Nothing is drawn while this thread
        // sits here, so the wait has to end whether or not anybody presses
        // anything: a pact reports its progress over a channel that only the
        // bottom of this loop reads, and a progress line that waits for a
        // keystroke to appear is worse than none at all.
        if event::poll(POLL_INTERVAL)? {
            match event::read()? {
                // Two situations are passed in rather than read out of the app,
                // and each answers one key. Whether a run is in flight is what
                // Esc reads two ways — it cancels a run when there is one and
                // asks about quitting when there is not — and the question on
                // screen is what every key reads differently while it is up.
                // See [`press_for`], which owns both readings.
                Event::Key(key) => match press_for(key, confirm, &prompt, pact.is_some()) {
                    // Returning is the whole of quitting, and it is enough even
                    // with a pact in flight. `pact` drops on the way out, which
                    // cancels the run and kills the `claude` it was waiting on
                    // (see [`Running`]); the guard drops after it and puts the
                    // terminal back. Nothing joins the worker: it is left to be
                    // ended by the process, having written whole documents or none,
                    // and the manifest it never got to rewrite still says what it
                    // said before.
                    //
                    // Every way out arrives here: a Yes to the question, Ctrl-C,
                    // and `q` during a run. The second spelling is the app's old
                    // quit, which [`press_for`] no longer produces — naming it
                    // beside the first keeps one road out of this loop rather
                    // than two that have to be kept doing the same thing.
                    Pressed::Leave | Pressed::Act(Action::Quit) => return Ok(()),
                    // The question, opened, moved, or taken down again. Nothing
                    // else happens and nothing else needs to: the app was never
                    // touched, so a No has nothing to put back, and the top of
                    // this loop draws whatever the question now is.
                    Pressed::Confirm(next) => confirm = next,
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
                    Pressed::Act(Action::CancelPact) => {
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
                    Pressed::Act(Action::ToggleFocus) => app.toggle_focus(),
                    Pressed::Act(Action::SelectPrevious) => app.select_previous(),
                    Pressed::Act(Action::SelectNext) => app.select_next(),
                    // No height is passed: the app was told the viewport's height
                    // at the top of this loop, so a page is whatever the frame just
                    // drawn could show.
                    Pressed::Act(Action::SelectPageUp) => app.select_page_up(),
                    Pressed::Act(Action::SelectPageDown) => app.select_page_down(),
                    Pressed::Act(Action::SelectFirst) => app.select_first(),
                    Pressed::Act(Action::SelectLast) => app.select_last(),
                    // Nothing else happens here on purpose. What is collapsed is
                    // the front end's view of the tree and never touches disk (§8),
                    // so there is no manifest to write; the tree has not changed,
                    // so there is nothing to re-read. The app moves the selection
                    // and the scroll offset back into range itself, and the next
                    // frame — the top of this same loop — draws the shorter or
                    // longer list.
                    Pressed::Act(Action::ToggleCollapsed) => app.toggle_collapsed(),
                    // Nothing else happens here either, and for the same reasons as
                    // collapsing: which rows are worth looking at is the front end's
                    // view of the tree and is never written down (§5), so there is
                    // no manifest to save, and the tree itself has not changed, so
                    // there is nothing to re-read. The app re-flows its rows and
                    // puts the selection and the scroll offset back in range; the
                    // next frame draws whatever is left.
                    Pressed::Act(Action::TogglePactedOnly) => app.toggle_pacted_only(),
                    // Nothing else here either, for the third time and for the same
                    // reasons as the two arms above: whether the files inside a
                    // module are on screen is the front end's view of the tree and
                    // is never written down (§5), so there is no manifest to save,
                    // and the files were read by the load that built these rows, so
                    // there is nothing to re-read. The app re-flows its rows and
                    // keeps the selection and the scroll offset in range; the next
                    // frame draws the longer or shorter list.
                    Pressed::Act(Action::ToggleFiles) => app.toggle_files(),
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
                    Pressed::Act(Action::TogglePact) => {
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
                            // The worker, the channel and the say-when, in the one
                            // value this loop keeps about a run it is not doing —
                            // see [`start_run`], which is where both keys start
                            // theirs.
                            pact = Some(start_run(
                                Work::Pact(toggle),
                                before,
                                &manifest,
                                &scope.repo_root,
                                &agent,
                            ));
                        }
                    }
                    // The same arm again with one word changed, and that is the
                    // point of it: a refresh is a run like a pact — one worker,
                    // one channel, one account, one say-when — over the stale
                    // directories of the subtree rather than all of them, and
                    // which those are is the engine's judgement and not this
                    // loop's. So the copy is taken the same way, the press
                    // decides the same three things ([`refresh_press`]), and
                    // what comes back fills the very same `Option<Running>`,
                    // which is what makes the two keys refuse each other: the
                    // in-flight check both of them go through is `pact.is_some()`
                    // here, whichever run is the one in flight.
                    //
                    // `None` needs nothing done about it here either, for
                    // `TogglePact`'s two reasons: a row the app turned down has
                    // its sentence in `App::message` already, and a press while
                    // a run is going said so on that run's progress line.
                    Pressed::Act(Action::Refresh) => {
                        let before = app.clone();
                        if let Some(directory) =
                            refresh_press(&mut app, pact.is_some(), Instant::now())
                        {
                            pact = Some(start_run(
                                Work::Refresh(directory),
                                before,
                                &manifest,
                                &scope.repo_root,
                                &agent,
                            ));
                        }
                    }
                    // The one key that answers to the terminal rather than to
                    // the app. The sequence is written first and the flag moved
                    // only if it went out, so what this thread believes about
                    // the terminal is what it last successfully told it; a write
                    // that fails takes the whole loop down through the guard,
                    // which turns capture off on the way past whatever state it
                    // was left in.
                    //
                    // Nothing else happens: no focus moves, no row is selected,
                    // nothing is redrawn here — the top of the loop draws every
                    // round, and it is where the footer picks the new wording up.
                    // With capture off the terminal keeps the pointer to itself,
                    // so `Event::Mouse` simply stops arriving and the mouse
                    // handler needs no gate of its own.
                    Pressed::Act(Action::ToggleMouseCapture) => {
                        report_mouse(!mouse_captured)?;
                        mouse_captured = !mouse_captured;
                    }
                    // A key nothing is bound to, or one whose press has already
                    // been answered where it was decided — and, for now, the
                    // two roads the scope prompt is on. `s` asks for a prompt
                    // this loop does not yet open, and a key typed into one is
                    // therefore unreachable: reading the entry, putting the
                    // window up and saving the manifest on Enter is the slice
                    // after this one, and until it lands the honest answer is
                    // the same nothing an unbound key gets. Nothing is invented
                    // in the meantime — a message standing in for the missing
                    // window would be one more thing to take out again.
                    Pressed::Act(Action::OpenScope) | Pressed::Scope(_) | Pressed::Nothing => {}
                },
                // The pointer, answered in the same shape and for the same
                // reasons. [`mouse_action`] is handed the event, the size this
                // round measured at the top — the one the hit test has to
                // agree with, because it is the size the frame above was drawn
                // at — and the app, since which row a click lands on depends
                // on where the tree's window is. Nothing here reads the
                // terminal and nothing draws: the round is the redraw, which is
                // why a pointer swept across the screen costs nothing.
                //
                // The question on the way out is handed over too, and it makes
                // every event mean nothing: while it is up the pointer is read
                // and dropped, because the dialog has no clickable answers and a
                // click on the tree behind it would move a selection the reader
                // cannot see.
                Event::Mouse(mouse) => match mouse_action(mouse, size, &app, confirm, &prompt) {
                    // The wheel over the tree column, whichever pane the keys
                    // are pointed at: the selection moves and the window
                    // follows it, exactly as it does for a movement key.
                    Some(MouseAction::SelectNextBy(rows)) => app.select_next_by(rows),
                    Some(MouseAction::SelectPreviousBy(rows)) => app.select_previous_by(rows),
                    // The panel's half of the same wheel. What the follow rule
                    // makes of it is the app's business and is not restated
                    // here: a window scrolled back stops following the newest
                    // line, and one scrolled to the end starts again.
                    Some(MouseAction::ScrollPanelDown(lines)) => app.scroll_panel_down(lines),
                    Some(MouseAction::ScrollPanelUp(lines)) => app.scroll_panel_up(lines),
                    // A click names a row, and a click in a pane also says
                    // which pane the keys are about from now on: the reader has
                    // just pointed at it, and leaving the keys driving the
                    // other pane would send the next `j` somewhere they are not
                    // looking.
                    Some(MouseAction::SelectRow(index)) => {
                        app.set_focus(Focus::Tree);
                        app.select_row(index);
                    }
                    // A click on the row that is already selected, which is
                    // space by another road — so it goes through the very
                    // method space goes through, and a row with nothing under
                    // it collapses nothing here exactly as it would there.
                    Some(MouseAction::ToggleCollapsed) => {
                        app.set_focus(Focus::Tree);
                        app.toggle_collapsed();
                    }
                    // A click inside a pane with nothing under it: the tree's
                    // header, the space below its last row, a line of the
                    // panel. Taking the focus is the whole of what it does.
                    Some(MouseAction::Focus(focus)) => app.set_focus(focus),
                    None => {}
                },
                // Resizes, focus changes and pasted text: read and dropped. The
                // frame is measured again at the top of every round, so a
                // resize needs nothing done about it here, and the other two
                // mean nothing to a program with nowhere to paste into.
                _ => {}
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

/// Ask the terminal to report its mouse, or to stop reporting it.
///
/// The whole of what `m` does to a terminal, and the only thing that differs
/// between its two directions is which sequence is written — so it is one line
/// at the keystroke rather than an `if` in the middle of the loop. The caller
/// moves the flag it keeps only after this has returned, which is what keeps
/// what warlock believes about the terminal down to what it last successfully
/// told it.
fn report_mouse(on: bool) -> io::Result<()> {
    if on {
        execute!(io::stdout(), EnableMouseCapture)
    } else {
        execute!(io::stdout(), DisableMouseCapture)
    }
}

#[cfg(test)]
mod tests {
    use super::{Intention, USAGE, intention_for};

    #[test]
    fn the_binary_is_named_warlock() {
        assert_eq!(env!("CARGO_BIN_NAME"), "warlock");
    }

    #[test]
    fn no_arguments_opens_the_tree() {
        assert_eq!(intention_for(Vec::<String>::new()), Intention::Tui);
    }

    #[test]
    fn init_and_config_are_the_subcommands() {
        assert_eq!(intention_for(["init"]), Intention::Init);
        assert_eq!(intention_for(["config"]), Intention::Config);
    }

    #[test]
    fn both_spellings_of_help_are_help() {
        assert_eq!(intention_for(["-h"]), Intention::Help);
        assert_eq!(intention_for(["--help"]), Intention::Help);
    }

    #[test]
    fn a_word_warlock_does_not_have_is_refused_rather_than_opening_the_tree() {
        // The whole reason the dispatch exists: `warlock status` used to open
        // the tree, which reads as the typed command having run.
        assert_eq!(intention_for(["status"]), Intention::Refuse);
        assert_eq!(intention_for(["nonsense"]), Intention::Refuse);
        assert_eq!(intention_for([""]), Intention::Refuse);
    }

    #[test]
    fn version_is_refused_because_warlock_does_not_have_one_yet() {
        // Deliberate, and recorded on `intention_for`: the flag is not
        // implemented, so it is refused like any other word warlock does not
        // have rather than answered with a half-truth.
        assert_eq!(intention_for(["--version"]), Intention::Refuse);
        assert_eq!(intention_for(["-V"]), Intention::Refuse);
    }

    #[test]
    fn a_trailing_argument_is_refused_and_never_quietly_dropped() {
        assert_eq!(intention_for(["init", "extra"]), Intention::Refuse);
        assert_eq!(intention_for(["--help", "init"]), Intention::Refuse);
        assert_eq!(intention_for(["init", "init", "init"]), Intention::Refuse);
        assert_eq!(intention_for(["config", "extra"]), Intention::Refuse);
        // The one somebody will try: the sigils are typed at the prompt, where
        // the answer that clears them can be explained before it is given, and
        // never as an argument.
        assert_eq!(intention_for(["config", "data-plane"]), Intention::Refuse);
    }

    #[test]
    fn the_usage_line_is_one_line_and_names_both_subcommands() {
        // Printed on stdout when asked for and on stderr when not, and a usage
        // line that wraps is a usage line that reads like a crash.
        assert!(!USAGE.contains('\n'), "{USAGE}");
        assert!(USAGE.contains("init"), "{USAGE}");
        assert!(USAGE.contains("config"), "{USAGE}");
        assert!(USAGE.starts_with("usage: warlock"), "{USAGE}");
    }
}
