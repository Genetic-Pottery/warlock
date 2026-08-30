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
//! pact on a worker thread and applying what it says is [`pacting`]'s, asking
//! for a scope and writing it is [`scoping`]'s, reading a file into the panel is
//! [`viewing`]'s, handing one to `$EDITOR` and taking the terminal back
//! afterwards is [`editing`]'s, where
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
//! The third reason to reload is that the disk moved without anybody
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
//! The fourth and last is the disk having moved because this loop asked somebody
//! else to move it: `e` hands a file to `$EDITOR`, and whatever was saved in
//! there is on disk before the terminal comes back. So the same [`reload_tree`]
//! runs on the way in, inside [`editing::edit_press`] — a `WARLOCK.md` that was
//! edited restales its own directory, and a row that only went yellow at the
//! reader's next keystroke would be warlock knowing something and not saying it.
//! The panel is read again there too, and only in one case: when the document
//! card is holding the very file that was edited, which is what `document` below
//! is kept for. Which card is showing never moves for it.
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
//! state, the scope prompt's, the composer's and whether a run is in flight go
//! in, and what the loop is to do comes out — so the whole gate is testable with
//! nothing attached to stdout, and the arms below are the five things that can
//! come of a keystroke: leave, move the question, type into the scope prompt,
//! type into the composer, or hand the key to the app. While either window is up
//! the app hears nothing, the pointer included: mouse events are read and
//! dropped, so a click cannot select a row behind a window that is about to
//! close.
//!
//! The composer is the third place a keystroke can land, and the newest. It is a
//! [`Composer`] on this stack — beside the two questions and for their reason,
//! since a draft on the app would be a draft the copy put back after a run had
//! never heard of — and it is offered to [`press_for`] exactly when
//! [`App::focus`] is on it. While it holds the keyboard every key but Ctrl-C and
//! Tab goes to [`warlock_tui::compose_for`] and never to [`input::action_for`],
//! which is the whole point of the field: `p` is the letter p rather than a pact
//! over whatever row happens to be selected. Ctrl-C still leaves, Tab still moves
//! the keyboard on, Esc hands it back with the draft intact, and Enter offers the
//! draft up to nobody — this slice has no consumer for a submission, and the arm
//! below is inert on purpose.
//!
//! The second window is the scope prompt, and it is the one keystroke that
//! writes to disk without being a run. `s` opens it over the selected directory
//! holding the scope that directory carries now, read out of the manifest this
//! loop already holds; Enter writes the manifest here, on this thread, between
//! two frames. There is no worker, no channel, no say-when, no account and no
//! reload — a scope is one string in one entry of a file already in hand, and it
//! changes no row's state or colour, so re-reading the tree afterwards would walk
//! the repository to arrive at the tree already on screen. Both halves of it live
//! in [`scoping`], the way the pact key's live in [`pacting`], and the deliberate
//! consequence is recorded there: a successful write says nothing at all, because
//! the fact it produces is a label on the row that a sibling slice draws.
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
//! anything else is answered here and exits. `init` writes a `CLAUDE.md` at the
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

use std::path::PathBuf;
use std::process::ExitCode;
use std::time::{Duration, Instant};
use std::{env, io};

use ratatui::crossterm::event::{self, DisableMouseCapture, EnableMouseCapture, Event, MouseEvent};
use ratatui::crossterm::execute;
use ratatui::layout::Size;
use warlock_engine::{Manifest, Written, repository_root, write_claude_md};
use warlock_tui::{
    App, ClaudeAgent, Composed, Composer, Focus, QuitConfirm, ScopePrompt, composer_on_screen,
    draw, panel_height, panel_width, tree_height,
};

mod config;
mod editing;
mod error;
mod input;
mod pacting;
mod scoping;
mod session;
mod terminal;
mod viewing;

use config::configure;
use editing::edit_press;
use error::Error;
use input::{Action, MouseAction, Pressed, mouse_action, press_for};
use pacting::{Running, Work, apply_progress, pact_press, refresh_press, start_run};
use scoping::{scope_edit, scope_press};
use session::{Scope, Watched, load_app, load_manifest, start_watching};
use terminal::{TerminalGuard, install_panic_hook};
use viewing::view_press;

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
                     `init` writes CLAUDE.md at the repository root; `config` sets \
                     the sigils this machine holds for it";

/// What `warlock init` wants the repository root for, as the tail of
/// [`Error::NoRepository`]'s sentence. `config`'s own tail is spelled beside
/// `config`, in its module.
const FOR_CLAUDE_MD: &str = "write `CLAUDE.md` at";

/// What `warlock init` says when there was no `CLAUDE.md` and now there is one.
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

/// `warlock init`: write the `CLAUDE.md` at the repository root and say which
/// file was written.
///
/// Three steps and no policy of its own. The working directory says where to
/// start, [`repository_root`] walks up to the nearest ancestor with a `.git/`
/// — so running this from any subdirectory writes the one file in the right
/// place — and the engine does the writing, because the splice, the delimiters
/// and the text are all its business (see
/// [`write_claude_md`](warlock_engine::write_claude_md)).
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
        wanted: FOR_CLAUDE_MD,
    })?;

    let written = write_claude_md(&root).map_err(|source| Error::ClaudeMd { source })?;
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
    // not a `?` and why the line about it is put up in there rather than here:
    // warlock with no live updates is warlock as it was. See [`start_watching`].
    let mut watched = start_watching(&mut app, &scope, &tree);
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
    // `prompt` and not `scope`: the `scope` this function already holds is the
    // session's — the repo root and what warlock was pointed at — and two things
    // by that name in one loop is one of them being read as the other.
    let mut prompt = ScopePrompt::default();
    // What has been typed into the composer, and the only copy of it: empty as
    // every session starts. It lives here, beside the two questions above,
    // rather than on the app — and that is load-bearing rather than tidy. A
    // pact that ends with nothing recorded puts the copy of the app taken
    // before it back over the live one and keeps only the panel (see
    // `App::restore_from`), so a draft stored on the app would be a draft a run
    // could swallow half a sentence into. Here, nothing a run does can reach it:
    // the keystrokes are the only thing that ever writes to it.
    let mut composer = Composer::default();
    // Which file the panel's document card is holding, and the only record of
    // it: `None` until the first `v` of the session that read something. It
    // lives here rather than on the app for the reason `mouse_captured` does —
    // the app is handed lines and never a path, because a path on it would be a
    // path something later had to open (see `App::show_document`) — and it is
    // read by exactly one keystroke, `e`, which re-reads the card only when the
    // file it handed to an editor is the file on it.
    let mut document: Option<PathBuf> = None;

    loop {
        // Measured once a round, and here rather than inside `draw_frame`,
        // because the round needs it twice: it is what the app is told to
        // scroll against and what a click landing on this frame is hit-tested
        // against, and those two have to be the one answer.
        let size = guard.terminal.size()?;
        // Told what the terminal is doing with the pointer, here rather than in
        // there because the flag is this thread's and not the frame's: the
        // footer names the `m` key by what the next press of it will do, and the
        // app cannot see a terminal. Every frame rather than at the keystroke,
        // so a view restored from the copy taken before a pact — which is a copy
        // of a flag that may have been toggled since — is put right before it is
        // drawn.
        app.set_mouse_captured(mouse_captured);
        // The draft goes in beside the two questions: it is a pane cut off the
        // bottom of the panel's column, so the panel is drawn and scrolled a few
        // rows shorter for as long as there is a field on screen. Whether there
        // is one is `composer_on_screen`'s answer and not this loop's — the
        // document card takes the whole column back — and it is asked in there,
        // and again below for the pointer, so the frame and the hit test are one
        // rule read twice rather than two opinions about the same rows.
        draw_frame(
            &mut app, &mut guard, &scope, size, confirm, &prompt, &composer,
        )?;

        // Waited on rather than blocked on. Nothing is drawn while this thread
        // sits here, so the wait has to end whether or not anybody presses
        // anything: a pact reports its progress over a channel that only the
        // bottom of this loop reads, and a progress line that waits for a
        // keystroke to appear is worse than none at all.
        if event::poll(POLL_INTERVAL)? {
            // Whether there is a field for a keystroke to land in, worked out
            // before the event is read because it is a fact about the frame that
            // was just drawn: the composer is offered to `press_for` on exactly
            // the condition that lights its border, which is the keyboard being
            // pointed at it. Offered rather than looked up, because `press_for`
            // has never heard of an `App` — the same way the two questions above
            // are handed in — and with the keyboard anywhere else this is `None`,
            // there is no draft to type into, and every letter is the command it
            // has always been.
            let typing = (app.focus() == Focus::Composer).then_some(&composer);
            match event::read()? {
                // Three situations are passed in rather than read out of the
                // app, and each answers a set of keys. Whether a run is in
                // flight is what Esc reads two ways — it cancels a run when
                // there is one and asks about quitting when there is not — the
                // question on screen is what every key reads differently while
                // it is up, and the composer is what turns the letters into
                // text. See [`press_for`], which owns all three readings.
                Event::Key(key) => match press_for(key, confirm, &prompt, typing, pact.is_some()) {
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
                    // The one key that reads a file and the only one that shows
                    // anything a model wrote. It is done here, on this thread,
                    // between two frames: a read capped at a few kilobytes is
                    // over inside a frame, so there is no worker, no channel and
                    // no account, and nothing to reload afterwards because
                    // reading a file changes nothing about the tree.
                    //
                    // It needs no answer of its own, for the reason the scope
                    // key's arm needs none: everything this press can refuse it
                    // refuses inside `view_press` — a directory row through
                    // `App::message` — and every way the read itself can fail
                    // ends as one line on that same footer with the panel left
                    // as it was. So the loop goes round again after a failure
                    // exactly as it does after a success. Unlike `p`, `r` and
                    // `s` it is not handed the run: a read races nothing, so
                    // there is nothing for a run in flight to refuse. See
                    // `viewing::view_press`.
                    //
                    // What comes back is the file that is now on the document
                    // card, and it is kept here because the app is never told:
                    // `App::show_document` takes lines and never a path, so
                    // "which file the panel is holding" is this loop's to know.
                    // The one thing that asks for it is the edit key, which
                    // re-reads the card only when the file it just handed to an
                    // editor is the file on it. A press that read nothing —
                    // refused, or a read that failed — leaves the card holding
                    // what it held, which is why what was remembered before is
                    // what a `None` falls back to rather than being cleared.
                    Pressed::Act(Action::ViewFile) => document = view_press(&mut app).or(document),
                    // The one key that gives the screen away, and the only one
                    // whose answer is measured in minutes of somebody typing
                    // rather than in frames. The loop stops here for the whole
                    // of it: the terminal is put back the way warlock found it,
                    // `$EDITOR` is run on the selected file as a foreground
                    // child, the child is waited on, and the terminal is taken
                    // again — every one of those through the guard this loop
                    // already holds, so there is one spelling of teardown and
                    // one of setup (see `TerminalGuard::suspended`). Nothing is
                    // drawn behind the editor and no progress event is drained
                    // while it runs, which is the honest shape of handing the
                    // terminal to somebody else.
                    //
                    // `mouse_captured` is handed over because it is this
                    // thread's only record of what `m` last did: resuming
                    // without it would switch reporting back on behind a reader
                    // who turned it off.
                    //
                    // It needs no answer of its own, for the reason `v`'s and
                    // `s`'s arms need none: everything this press can refuse it
                    // refuses inside `edit_press` — a directory row in the very
                    // words `v` uses, a run in flight on the progress line, an
                    // `$EDITOR` naming nothing on the footer — and both ways the
                    // child itself can go wrong end as one line on that footer
                    // with the loop going round again. The `?` is the terminal
                    // and only the terminal: a screen that could not be taken
                    // back is not news for a footer nobody could read, so it
                    // leaves through the guard like every other terminal
                    // failure. See `editing::edit_press`.
                    //
                    // Two things are read again on the way back, and both are
                    // inside `edit_press`: the tree, so a directory whose file
                    // changed goes yellow without a further keystroke, and the
                    // document card — but only when the file just edited is the
                    // one on it, which is what `document` is kept for. Which
                    // card is showing does not move for either.
                    Pressed::Act(Action::EditFile) => {
                        edit_press(
                            &mut app,
                            &mut guard,
                            &scope,
                            document.as_deref(),
                            mouse_captured,
                            pact.is_some(),
                        )?;
                    }
                    // The panel's other card, and nothing else: the account if
                    // the document is up, the document if the account is. It is
                    // done here, on this thread, without reading anything —
                    // both cards are already in the app, so a swap is one field
                    // moved and the next frame drawing the other one.
                    //
                    // It needs no answer of its own, for `v`'s reason: the one
                    // thing this press can refuse — a session with no document
                    // read yet — it refuses inside `App::swap_card`, which
                    // leaves the panel on the account and puts a line on the
                    // footer naming the key that would make a second card. A
                    // swap that worked says nothing, because the reader can see
                    // it. Unlike `p`, `r` and `s` it is not handed the run: a
                    // swap races nothing, and a run that changed which card is
                    // showing would take a document out of the reader's hands.
                    Pressed::Act(Action::SwapCard) => app.swap_card(),
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
                    // The third key that writes to disk, and the one that is
                    // not a run: it opens a window holding the scope the
                    // selected directory carries now, read out of the manifest
                    // this loop is already holding. Everything it can refuse it
                    // refuses inside `scope_press` — a file row and an unpacted
                    // one through `App::message`, a press during a run through
                    // the progress line — and every one of those comes back as
                    // a prompt that is still closed, so this arm needs no
                    // `None` case of its own. See `scoping::scope_press`.
                    Pressed::Act(Action::OpenScope) => {
                        prompt = scope_press(&mut app, &manifest, &scope.repo_root, pact.is_some());
                    }
                    // Somebody typing into that window: a character more or
                    // less in the field, the window abandoned, or — on Enter —
                    // the manifest written. The whole of that last one happens
                    // here, on this thread, between two frames: no worker, no
                    // channel, no account and no reload, because a scope is one
                    // string written into one entry of a file already in this
                    // thread's hand (see `mod@scoping`). What comes back is the
                    // prompt from here on — down for a submit that was
                    // answered, still up over the text for one the engine
                    // refused. See `scoping::scope_edit`.
                    Pressed::Scope(edited) => {
                        prompt =
                            scope_edit(&mut app, &mut manifest, &scope.repo_root, &prompt, edited);
                    }
                    // Somebody typing at the foot of the panel's column: a
                    // character more or less in the draft, the keyboard handed
                    // back, or a draft offered up. What each of those comes to
                    // is [`apply_compose`], which is handed the local above
                    // rather than reaching for anything on the app — what is in
                    // the draft is not a fact about the tree.
                    Pressed::Compose(outcome) => apply_compose(&mut app, &mut composer, outcome),
                    // A key nothing is bound to, or one whose press has already
                    // been answered where it was decided.
                    Pressed::Nothing => {}
                },
                // The pointer, answered in the same shape and for the same
                // reasons, and in one line because both halves of it live in
                // [`apply_mouse`]: the event, the size this round measured at
                // the top — the one the hit test has to agree with, because it
                // is the size the frame above was drawn at — both windows, and
                // the draft under the panel, whose rows the hit test has to know
                // are not the panel's. None of what it does reads the terminal
                // and none of it draws: the round is the redraw, which is why a
                // pointer swept across the screen costs nothing.
                Event::Mouse(mouse) => {
                    apply_mouse(&mut app, mouse, size, confirm, &prompt, &composer);
                }
                // Resizes, focus changes and pasted text: read and dropped. The
                // frame is measured again at the top of every round, so a
                // resize needs nothing done about it here, and the other two
                // mean nothing to a program with nowhere to paste into.
                _ => {}
            }
        }

        // And then everything that happened off this thread: what a run has
        // said since the last round, and what the disk did while this one was
        // waiting on a keystroke. See `keep_up`, which reads the clock the
        // round is measured against — the instant the events being drained now
        // land on screen, within one `POLL_INTERVAL` of when they were sent.
        keep_up(&mut pact, &mut app, &mut manifest, &scope, &mut watched);
    }
}

/// Tell the app how big the frame it is about to be drawn in is, and draw it.
///
/// Told before it is drawn, and every frame rather than on resize: the scroll
/// offset is only right if it was computed against the height this frame gives
/// the tree, and [`tree_height`] is the same layout the frame is cut by. A
/// terminal resized between frames is handled by that alone — the next frame
/// measures again, and the next frame is at most one [`POLL_INTERVAL`] away.
/// The panel's window is measured the same way and for the same reason, off the
/// same size: [`panel_height`] and [`tree_height`] are two answers from the one
/// layout, so both panes are scrolled by the height this frame is about to give
/// them.
///
/// `size` is the caller's rather than measured here, because the round needs it
/// again: a click is hit-tested against the size of the frame it landed on, and
/// measuring twice would be two answers where the hit test needs one.
///
/// What the terminal is doing with the pointer is *not* here: it is a flag the
/// loop keeps about a terminal rather than anything this frame measures, so the
/// app is told it beside the size, one line above the call.
///
/// The instant the frame is drawn at is read here and handed to the renderer:
/// the panel's newest clock counts up against it, so a frame drawn with no event
/// waiting still shows a run that is moving. See [`draw`].
///
/// The gate on the way out is handed in beside the app because the app has never
/// heard of it (see [`QuitConfirm`]): closed, it changes nothing about the frame;
/// open, it is a small window drawn over the middle of it with everything behind
/// it cleared. The scope prompt goes in beside it for the same reason and is
/// drawn the same way — by reference, since it carries the text somebody is
/// typing.
///
/// The composer comes in the same way and is the reason the panel's height is
/// worth a second look: it is a pane cut off the bottom of the panel's column,
/// so every row it takes is a row the account no longer has, and the height the
/// app is told to scroll by has to be the reduced one — a panel told the whole
/// column would scroll by a window that is partly the field's. So the one
/// measurement goes to [`panel_height`] and to [`draw`] both, and whether there
/// is a field on this frame at all is [`composer_on_screen`]'s answer, asked
/// here: the document card takes the column back, and the panel is measured and
/// drawn as it was before there was a composer to pay for.
///
/// The width is not measured against it, because the field takes rows and never
/// columns: a document is wrapped at the width the panel had and the composer is
/// drawn at that very width. See [`panel_width`].
fn draw_frame(
    app: &mut App,
    guard: &mut TerminalGuard,
    scope: &Scope,
    size: Size,
    confirm: QuitConfirm,
    prompt: &ScopePrompt,
    composer: &Composer,
) -> io::Result<()> {
    let field = composer_on_screen(app, composer);
    app.set_viewport_height(tree_height(size));
    app.set_panel_height(panel_height(size, field));
    app.set_panel_width(panel_width(size));
    guard.terminal.draw(|frame| {
        draw(
            frame,
            app,
            &scope.chrome,
            Instant::now(),
            confirm,
            prompt,
            field,
        );
    })?;
    Ok(())
}

/// Keep up with what is happening off this thread: the run's progress, and the
/// disk moving under the tree.
///
/// The bottom of every round, whether or not a key was pressed. [`apply_progress`]
/// is the only place anything the worker says reaches the screen, and it has to
/// keep up with a thread that is not waiting for it; the scope is handed to it
/// because the end of a run re-reads the tree.
///
/// Whether a run was in flight is read *before* the drain, because the drain is
/// what ends one: a pact that is `Some` on the way in and `None` on the way out
/// finished in this round, and its own reload has already read the tree. The
/// documents it wrote are exactly the sort of thing the watcher reports, so the
/// events they set off are already sitting in the policy — they are answered by
/// the reload that just happened rather than by one of their own, and the tree it
/// read becomes the next round's filter.
///
/// Then what everything else did to the disk. Nothing is read again while a pact
/// is in flight — the trigger keeps until the run's own reload above — and
/// nothing at all is read when the disk has been still, which is almost every
/// round.
///
/// The clock is read once, here, and handed to everything under this: the round
/// is measured against one instant, which is when the events being drained now
/// land on screen — within one [`POLL_INTERVAL`] of when they were sent. It is
/// read at the top rather than per call for the reason the frame reads its own:
/// two readings a round would be two answers to one question.
fn keep_up(
    pact: &mut Option<Running>,
    app: &mut App,
    manifest: &mut Manifest,
    scope: &Scope,
    watched: &mut Watched,
) {
    let now = Instant::now();
    let running = pact.is_some();
    let reloaded = apply_progress(pact, app, manifest, scope, now);
    if running && pact.is_none() {
        watched.caught_up(reloaded.as_ref(), now);
    }
    watched.round(app, scope, pact.is_some(), now);
}

/// Do to the app whatever the pointer just asked for.
///
/// The other half of [`mouse_action`], which is handed the event, the size the
/// round measured — the one the hit test has to agree with, because it is the
/// size the frame was drawn at — and the app, since which row a click lands on
/// depends on where the tree's window is. Nothing here reads the terminal and
/// nothing draws: the round is the redraw, which is why a pointer swept across
/// the screen costs nothing.
///
/// Nothing at all covers a click that means nothing and every event arriving
/// while a window is up: the pointer is read and dropped then, because neither
/// dialog has clickable answers and a click on the tree behind one would move a
/// selection the reader cannot see.
///
/// The decision is made here rather than in the loop's arm because it takes the
/// app as it stands and the app cannot be lent out twice: the hit test reads it,
/// and what comes of the hit test writes to it, so the two are two statements
/// with the reading finished before the writing starts. The composer is in that
/// reading — the rows it takes are rows the panel gave up, so a click on the
/// field would otherwise be answered as a line of an account that is not drawn
/// there — and whether the frame had one on it at all is
/// [`composer_on_screen`]'s answer, asked here about the app that was drawn.
fn apply_mouse(
    app: &mut App,
    mouse: MouseEvent,
    size: Size,
    confirm: QuitConfirm,
    prompt: &ScopePrompt,
    composer: &Composer,
) {
    let field = composer_on_screen(app, composer);
    match mouse_action(mouse, size, app, confirm, prompt, field) {
        // The wheel over the tree column, whichever pane the keys are pointed
        // at: the selection moves and the window follows it, exactly as it does
        // for a movement key.
        Some(MouseAction::SelectNextBy(rows)) => app.select_next_by(rows),
        Some(MouseAction::SelectPreviousBy(rows)) => app.select_previous_by(rows),
        // The panel's half of the same wheel. What the follow rule makes of it
        // is the app's business and is not restated here: a window scrolled
        // back stops following the newest line, and one scrolled to the end
        // starts again.
        Some(MouseAction::ScrollPanelDown(lines)) => app.scroll_panel_down(lines),
        Some(MouseAction::ScrollPanelUp(lines)) => app.scroll_panel_up(lines),
        // A click names a row, and a click in a pane also says which pane the
        // keys are about from now on: the reader has just pointed at it, and
        // leaving the keys driving the other pane would send the next `j`
        // somewhere they are not looking.
        Some(MouseAction::SelectRow(index)) => {
            app.set_focus(Focus::Tree);
            app.select_row(index);
        }
        // A click on the row that is already selected, which is space by
        // another road — so it goes through the very method space goes through,
        // and a row with nothing under it collapses nothing here exactly as it
        // would there.
        Some(MouseAction::ToggleCollapsed) => {
            app.set_focus(Focus::Tree);
            app.toggle_collapsed();
        }
        // A click inside a pane with nothing under it: the tree's header, the
        // space below its last row, a line of the panel. Taking the focus is
        // the whole of what it does.
        Some(MouseAction::Focus(focus)) => app.set_focus(focus),
        None => {}
    }
}

/// Do to the draft and to the keyboard whatever the composer just made of a
/// key.
///
/// The other half of [`warlock_tui::compose_for`], in [`apply_mouse`]'s shape
/// and for its reason: three short arms that would otherwise be three more
/// paragraphs in the middle of the loop. Nothing here reads the terminal, draws,
/// starts a thread or writes a file — typing at the foot of the panel is the one
/// thing in warlock that costs nothing but a redraw.
///
/// The draft is a local of the loop and is handed in rather than read off the
/// app, which is the whole of why it survives a run: a pact that recorded
/// nothing puts the copy of the app taken before it back over the live one and
/// keeps only the panel (see [`App::restore_from`]), and this function is the
/// only thing in the binary that ever writes to the draft.
///
/// The three arms:
///
/// **Typing** is the draft replaced by the one [`compose_for`] just made —
/// a character more, a character less, or a new line. The app is not told,
/// because what somebody is halfway through writing is not a fact about the
/// tree, and the next frame draws whatever the local now holds.
///
/// **Leave** is Esc, the one key that means something different here to what it
/// means anywhere else: it hands the keyboard back and leaves every character
/// where it is. Nothing is thrown away — what somebody typed is worth more than
/// the keystroke that stopped typing it — and the draft is not this arm's
/// business at all, since a focus change cannot reach it. The panel rather than
/// the tree, and the same landing [`App::set_focus`] rescues a hidden composer
/// onto: the field is drawn under the panel, so the panel is what the reader is
/// looking at and what the movement keys they press next should be about. Tab
/// from there is one press back into the field.
///
/// **Submit** is a draft offered up, and deliberately offered to nobody. This
/// slice builds the field and routes the keyboard through it; what a submitted
/// draft is *for* is the next one's. So this arm starts nothing, spawns nothing,
/// writes nothing and says nothing on the footer — an Enter that announced a
/// submission nothing received would be warlock claiming to have done something.
/// The draft is left exactly as it is rather than cleared, which is the honest
/// half of that: clearing it would be the one place in here that can lose
/// somebody's typing, and it would lose it to a consumer that does not exist
/// yet. When there is one, this is where the draft is taken and the field reset
/// in the same breath. An empty or whitespace-only draft never arrives here at
/// all — [`compose_for`] answers that Enter with the draft unchanged — so a
/// submission with nothing in it is a keystroke rather than a mistake and has
/// nothing to report.
///
/// [`compose_for`]: warlock_tui::compose_for
fn apply_compose(app: &mut App, composer: &mut Composer, outcome: Composed) {
    match outcome {
        Composed::Typing(next) => *composer = next,
        Composed::Leave => app.set_focus(Focus::Panel),
        Composed::Submit => {}
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
