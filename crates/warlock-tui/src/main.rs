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
//! pact on a worker thread and applying what it says is [`pacting`]'s, the whole
//! conversation — the draft at the foot of the panel, the register it is in, the
//! turns it is made of and the window a `/write` opens — is [`chatting`]'s,
//! asking
//! for a scope and writing it is [`scoping`]'s, reading a file into the panel is
//! [`viewing`]'s, handing one to `$EDITOR` and taking the terminal back
//! afterwards is [`editing`]'s, what a brief written out of the conversation
//! would be called and the bytes of it going to disk is [`writing`]'s, where
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
//! selected one, and it does so through the same [`Pact`] — the same worker,
//! the same channel, the same account and the same say-when — which is what
//! makes one run at a time a fact rather than a rule. The two keys refuse each
//! other by that alone, and neither has to be told: [`Pact::press`] reads the
//! one run it holds before it decides anything, and says so on the line the
//! reader is already watching.
//!
//! Those events are also what fills the panel. The press that really starts a
//! run opens an [`warlock_tui::Account`] on the app — one pact, one account, so
//! the next run clears the last one — each directory the worker names opens a
//! section of it, and everything a pass is seen doing lands under the section it
//! belongs to. Both halves are [`Pact::keep_up`], which is handed the instant
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
//! rule covers it: [`Pact::keep_up`] does its own arm's work first — the
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
//! Both roads run through one [`Cancel`], which the run inside [`Pact`] owns and
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
//! The composer is the third place a keystroke can land, and the newest. It is
//! not on this stack and it is not on the app: it lives inside the [`Chat`],
//! with the turn that mutes it and the register it types commands into — and
//! being off the app is still load-bearing for its original reason, since a
//! draft on the app would be a draft the copy put back after a run had never
//! heard of. It is offered to [`press_for`] exactly when [`App::focus`] is on
//! it, read through [`Chat::composer`], which can only ever lend it out: this
//! file's `draw` lives in the library and has no way even to name a `Chat`.
//! While it holds the keyboard every key but Ctrl-C and
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
//! attached to the terminal: the arguments are read. The parser is [`Cli`], a
//! clap derive type, so the whole command line is a data structure rather than a
//! chain of string comparisons and every case below is testable with no
//! terminal, no repository and no process to spawn. No subcommand is the whole
//! of warlock as it was — the panic hook, the loop, the alternate screen — and
//! anything else is answered here and exits. `init` writes a `CLAUDE.md` at the
//! repository root and says which file it wrote; `config` prints the sigils this
//! machine holds for this repository and reads a line replacing them
//! ([`config`]); `stale` and `fresh` print the pacted directories at or below a
//! path that are in that state, a path a line or, with `--json`, one object
//! ([`query`]); `check` says which scope covers a path, what this machine holds
//! and whether the two meet, as prose or as one object ([`check`]); `unpact`
//! drops the pact on a directory and every pact below it, and `scope add` and
//! `scope remove` write and clear the boundary on one directory's pact — all
//! three only if this machine holds the boundary covering the path
//! ([`mod@edits`]); `pact` and `refresh` descend a subtree, spending a model
//! pass per directory and saying on stdout where they have got to, behind that
//! same boundary ([`mod@running`]); `-h` and
//! `--help` print clap's help; and every other word, and
//! every argument warlock has no place for, is clap's error and usage on stderr.
//! Refusing is the point of the last of those: warlock used to open the tree for
//! `warlock status`, which reads as the typed command having run.
//!
//! Every subcommand shares one rule, and it is why they are dispatched here
//! rather than anywhere inside [`run`]: none of them goes near the terminal. No
//! alternate screen, no raw mode and no panic hook — the hook exists to restore
//! a terminal these paths never take, and `config` reads its line in cooked
//! mode, which is also what makes Ctrl-C at its prompt an ordinary SIGINT that
//! ends the process before anything is written. The two listings and the check
//! add a second reason of their own: their answer is read by a script through a
//! pipe, and a program that had taken the alternate screen to print one would
//! have piped its answer into a repaint.
//!
//! The three questions — `stale`, `fresh` and `check` — share a second rule,
//! and it is the one that makes them safe to put in a script, a CI job or an
//! agent's hands: they only read. None of them writes to `.warlock/pacts.toml`
//! — no grant, no scope, no entry — none of them spawns a process, and none of
//! them runs a model pass, so asking costs no tokens, no minutes and no risk of
//! a manifest left in a state nobody asked for. What they read is what the loop
//! itself reads: the tree through [`load_tree`](warlock_engine::load_tree) with
//! its states already decided, coverage through the engine's
//! [`scope_covering`](warlock_engine::scope_covering) — never a second
//! staleness rule or a second walk written on this side of the edge. What they
//! leave behind is one answer on stdout and an exit status, and that status is
//! the whole of the contract a script reads: 0 means the question was answered,
//! whatever the answer turned out to be — nothing stale, nothing covering the
//! path, a scope closed to this machine — 1 means warlock could not answer it,
//! and 2 is clap's, for a command line that was never a question. Which is why
//! `warlock check <path> --json | jq -e '.opens'` is the recipe: the verdict is
//! `jq`'s non-zero status, and warlock spends none of its own on saying no.
//!
//! `unpact`, `scope add` and `scope remove` are the subcommands that do not only
//! read, and they are the reason the rule above is stated as being the three
//! questions' rather than
//! every subcommand's. They still take no terminal, spawn no process and run no
//! model pass — all three are `.warlock/pacts.toml` rewritten and nothing else,
//! with every `WARLOCK.md` left where it was — but they are writes, so they are
//! the ones asked who is asking. The boundary covering the path is held against
//! the sigils `warlock config` wrote for this machine before the command looks
//! at anything else at all — before the path is even checked for an entry, so
//! `warlock scope add` inside a closed boundary answers with the scope refusal
//! and never with what the manifest holds — and a boundary this machine does not
//! hold is one line on stderr and a 3, with the manifest untouched. That check is
//! [`mod@edits`]'s, it is the engine's own
//! [`scope_covering`](warlock_engine::scope_covering) and
//! [`scope_opens_to`](warlock_engine::scope_opens_to) rather than a second
//! reading of them, and there is no flag past it: `warlock config` is the only
//! road. So the writes have one status the questions never spend: 0 for a write
//! that happened, 3 for one refused at the boundary with nothing spent, 1 for
//! one warlock could not finish, 2 for a command line that was never a request.
//! The whole vocabulary, and why the refusal is worth a number of its own, is in
//! [`status_for`].
//!
//! `pact` and `refresh` are the writes that are not only a rewritten
//! `.warlock/pacts.toml`, and they are the reason the boundary is worth a check
//! that costs a file read before anything else happens. A run is minutes long,
//! it hands one `claude --print` per directory to a model and it writes a
//! `WARLOCK.md` beside each one, so a boundary asked any later than first would
//! be asked after somebody's tokens were spent and somebody else's prose was
//! overwritten. It is the same gate the three cheap writes pass through, in the
//! same words and with the same 3 ([`mod@running`]), and past it the only thing
//! that reaches the terminal is one line per directory entered and one per
//! directory documented, on stdout, because a run is watched through a pipe.
//! Nothing is drawn.
//!
//! They are also the two subcommands that can half-work, and that is where they
//! spend a status nothing else does. Each directory fails on its own — a pass
//! refused, a document that would not write — without ending the run, so a run
//! that had failures still saves the manifest the rest of the subtree earned,
//! names every failing directory on stderr a line at a time, ends with one line
//! saying how many of how many failed, and exits **4**. The split between the
//! two streams is what makes that readable: progress on stdout, every failure
//! and the count on stderr, so `warlock pact . > run.log` puts the descent in
//! the file and leaves what went wrong on the terminal.
//!
//! And they are the two subcommands with a key: minutes of somebody's tokens
//! with no panel to press Esc in leaves Ctrl-C as the only say-when, so a
//! headless run listens for it ([`mod@running`]). The first press is the
//! panel's Esc — the `claude` in flight is killed, the descent ends at the next
//! directory rather than part way through one, and what finished is hashed,
//! granted and saved before the process leaves with **130**. The second is the
//! panel's `q`: it exits at once, saving nothing and printing nothing. The same
//! [`Cancel`] does both halves of the first press, which is why a stop takes
//! milliseconds rather than the rest of a five-minute pass.
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

use clap::{Parser, Subcommand};
use ratatui::crossterm::event::{self, Event, KeyEvent, MouseEvent};
use ratatui::layout::Size;
use warlock_engine::{Agent, Manifest, Written, repository_root, write_claude_md};
use warlock_tui::{
    App, Composer, Converses, Focus, QuitConfirm, Run, ScopePrompt, Wired, composer_on_screen,
    draw, panel_height, panel_width, tree_height,
};

mod boundary;
mod chatting;
mod check;
mod config;
mod editing;
mod edits;
mod error;
mod input;
mod pacting;
mod query;
mod running;
mod scoping;
mod session;
/// In-memory stand-ins for the two model seams, so a test that is not about the
/// model does not have to spawn one. Test-only, and private on purpose: warlock
/// itself talks to `claude`.
#[cfg(test)]
mod stubs;
mod terminal;
mod viewing;
mod writing;

use chatting::Chat;
use check::check;
use config::configure;
use editing::edit_press;
use edits::{scope_add, scope_remove, unpact};
use error::Error;
use input::{Action, MouseAction, Pressed, mouse_action, press_for};
use pacting::{Pact, Reloaded};
use query::{Listing, list};
use running::{pact, refresh};
use scoping::{scope_edit, scope_press};
use session::{Scope, Watched, load_app, load_manifest, start_watching};
use terminal::{Screen, TerminalGuard, install_panic_hook};
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

/// What warlock was asked to do, as read off the command line and nothing else.
///
/// `name` is spelled out rather than taken from the package, because the package
/// is `warlock-tui` and the executable it ships is `warlock`; the help and the
/// usage lines have to say the word a reader typed.
///
/// `about` is written here and `long_about` is switched off, on this type and on
/// every subcommand, because clap's derive otherwise lifts the doc comment above
/// it into `--help`. This file's comments are essays for whoever maintains
/// warlock, and a paragraph on why the panic hook is installed where it is would
/// be a strange answer to `warlock --help`. Short help is the only help.
///
/// No `version` is declared, and that is a decision rather than an omission:
/// `--version` and `-V` are unrecognized arguments, refused with everything else
/// warlock does not have. Warlock is not installed from a registry and nobody is
/// diagnosing a version skew in it yet, so the honest answer today is that the
/// flag does not exist. Adding it later is `version` in the attribute below —
/// not a reason to carry a half-answer in the meantime.
///
/// The subcommand is an `Option`, never `arg_required_else_help`: bare `warlock`
/// is the tree, which is the thing warlock mostly is, not a mistake to be
/// answered with help.
///
/// Not [`Copy`] any more, and it is the path arguments below that took it away:
/// a [`PathBuf`] owns its bytes. Nothing here misses it — the whole of what
/// `main` does with this value is move it into one `match`.
#[derive(Debug, Clone, PartialEq, Eq, Parser)]
#[command(
    name = "warlock",
    about = "A freshness ledger for a repository's documentation.",
    long_about = None
)]
struct Cli {
    /// Which of warlock's operations to run; none of them opens the tree.
    #[command(subcommand)]
    command: Option<Command>,
}

/// The operations warlock will do without opening the tree.
///
/// An enum rather than a parsed word, which is what makes the two listings
/// below cost a field each rather than an argument parser of their own: the
/// path they take and the `--json` they answer in are declared here, beside
/// what they dispatch to, with nothing above this to change.
///
/// The two of them are one shape twice over rather than one variant carrying a
/// state, and deliberately: what a reader types is the whole of the difference
/// between them, and a `Command::List { state }` would be a variant nobody can
/// find by grepping for the word they typed. What they *do* is one function
/// ([`query::list`]) taking a [`Listing`], so the sameness is where the work is
/// and the difference is where the words are.
#[derive(Debug, Clone, PartialEq, Eq, Subcommand)]
enum Command {
    /// `warlock init`.
    #[command(
        about = "Write warlock's section of CLAUDE.md at the repository root.",
        long_about = None
    )]
    Init,
    /// `warlock config`.
    #[command(
        about = "Set the sigils this machine holds for this repository.",
        long_about = None
    )]
    Config,
    /// `warlock stale [path]`.
    #[command(
        about = "List the pacted directories that are stale.",
        long_about = None
    )]
    Stale {
        /// Where to answer about; the repository root when it is left off.
        #[arg(value_name = "PATH")]
        path: Option<PathBuf>,
        /// Answer as one JSON object instead of one path per line.
        #[arg(long)]
        json: bool,
    },
    /// `warlock fresh [path]`.
    #[command(
        about = "List the pacted directories that are fresh.",
        long_about = None
    )]
    Fresh {
        /// Where to answer about; the repository root when it is left off.
        #[arg(value_name = "PATH")]
        path: Option<PathBuf>,
        /// Answer as one JSON object instead of one path per line.
        #[arg(long)]
        json: bool,
    },
    /// `warlock check <path>`.
    #[command(
        about = "Say which scope covers a path and whether this machine's sigils open it.",
        long_about = None
    )]
    Check {
        // Required, unlike the two listings' optional path, and the doc comment
        // below is one line for the reason every other one here is: clap lifts
        // it into `--help`. A check is a walk up from one place, so there is no
        // whole-repository answer for an omitted path to mean — leaving it off
        // is a malformed invocation and clap's own exit status of 2.
        /// Which path to answer about.
        #[arg(value_name = "PATH")]
        path: PathBuf,
        /// Answer as one JSON object instead of three lines of prose.
        #[arg(long)]
        json: bool,
    },
    /// `warlock unpact <path>`.
    #[command(
        about = "Drop the pact on a directory and every pact below it.",
        long_about = None
    )]
    Unpact {
        // Required, like the check's and for the same reason: the whole-manifest
        // answer is `warlock unpact .`, which somebody has to have typed. An
        // omitted path defaulting to the repository root would make the largest
        // edit warlock can make the one that is easiest to make by accident.
        /// Which directory to un-pact, with everything below it.
        #[arg(value_name = "PATH")]
        path: PathBuf,
    },
    /// `warlock pact <path>`.
    #[command(
        about = "Describe a directory and everything below it, writing a WARLOCK.md for each.",
        long_about = None
    )]
    Pact {
        // Required, like the un-pact's and for the same reason turned the other
        // way up: a run is minutes of model passes over whatever it is pointed
        // at, and the largest one warlock can start must not be the one an
        // omitted argument starts by itself. `warlock pact .` is somebody
        // saying so.
        /// Which directory to describe, with everything below it.
        #[arg(value_name = "PATH")]
        path: PathBuf,
    },
    /// `warlock refresh <path>`.
    #[command(
        about = "Describe the stale directories at or below one, leaving the fresh ones alone.",
        long_about = None
    )]
    Refresh {
        /// Which directory to refresh, with everything below it.
        #[arg(value_name = "PATH")]
        path: PathBuf,
    },
    /// `warlock scope <add|remove>`.
    ///
    /// The one subcommand with subcommands of its own, and the nesting is the
    /// vocabulary rather than decoration: a scope is a noun with two things a
    /// person does to it, and `warlock scope-add` would be two commands that
    /// merely start with the same letters. It carries no work of its own — every
    /// arm below dispatches into [`mod@edits`] exactly as `unpact` does, through
    /// the same boundary and the same manifest save.
    #[command(
        about = "Set or clear the scope on a pacted directory.",
        long_about = None
    )]
    Scope {
        /// Which of the two writes; there is no bare `warlock scope`.
        #[command(subcommand)]
        command: ScopeCommand,
    },
}

/// The two things `warlock scope` will do to a directory's boundary.
///
/// Add and remove and nothing else — no `list`, because that is `warlock check`
/// and a second spelling of a question is how a vocabulary rots, and no `set`
/// beside `add`, because one write with one name is what keeps the shell and the
/// `s` key describable as one rule.
///
/// Clearing is [`ScopeCommand::Remove`] and only that. The TUI's field clears a
/// scope by being empty, which is the right answer for a window somebody is
/// typing in; at a shell the empty string is far more often an argument that
/// went missing than a clear somebody meant, so `warlock scope add <path> ''` is
/// the engine's `Empty` rule and the reader is told which command clears.
#[derive(Debug, Clone, PartialEq, Eq, Subcommand)]
enum ScopeCommand {
    /// `warlock scope add <path> <scope>`.
    #[command(
        about = "Write a scope onto a pacted directory.",
        long_about = None
    )]
    Add {
        /// Which directory to scope.
        #[arg(value_name = "PATH")]
        path: PathBuf,
        // A `String` and not a validated type: what a scope may be is the
        // engine's to say, and a parser that judged it here would be a second
        // opinion about it in the one place clap's exit status of 2 would be
        // spent on a rule warlock words itself.
        /// The scope to write, lower-cased before it is judged.
        #[arg(value_name = "SCOPE")]
        scope: String,
    },
    /// `warlock scope remove <path>`.
    #[command(
        about = "Clear the scope on a pacted directory.",
        long_about = None
    )]
    Remove {
        /// Which directory to clear the scope on.
        #[arg(value_name = "PATH")]
        path: PathBuf,
    },
}

fn main() -> ExitCode {
    // Read before anything else happens and, deliberately, before anything
    // touches the terminal: `init`, help and a refusal all print on the ordinary
    // screen, and a program that entered the alternate screen to write one line
    // would tear it down around a message nobody saw. `parse` exits the process
    // itself on a parse error or on `--help`, which is only safe because of that
    // ordering: nothing is attached to the terminal yet, so there is nothing
    // left un-restored by an exit from inside here.
    //
    // Two registers of failure, kept deliberately distinct. A command line clap
    // could not parse is clap's: its wording, its usage line, its exit status of
    // 2. Anything warlock itself could not do is warlock's: the `warlock: `
    // prefix below and exit status 1. The split is load-bearing rather than
    // cosmetic, and the listings are what it is for: `warlock stale` refuses a
    // path with no repository-relative spelling with a 1, and a script has to
    // be able to tell that refusal from a typo by the exit status alone,
    // without reading a word of either message. A 0 means the question was
    // answered whatever the answer was — including an empty answer, which is
    // "nothing is stale" and not the absence of one.
    //
    // The writes below add a third register, which is warlock's too and is not
    // a failure at all: a boundary this machine's sigils do not open is refused
    // with nothing spent, and that is a 3. See `status_for`.
    let cli = Cli::parse();

    let outcome = match cli.command {
        None => {
            // Before anything touches the terminal: a panic during setup has to
            // leave the terminal usable too.
            install_panic_hook();
            run()
        }
        Some(Command::Init) => init(),
        // The second subcommand, dispatched here for the first one's reasons:
        // it prints on the ordinary screen and reads a line from stdin in cooked
        // mode, so nothing about it may touch the terminal — including the panic
        // hook, which exists to restore a terminal this path never takes.
        Some(Command::Config) => configure(),
        // The two questions, dispatched here for the same reason and with one
        // more of their own: they print their answer on the ordinary screen and
        // a script reads it, so a program that had entered the alternate screen
        // would have piped its answer into a repaint. Neither writes anything,
        // neither spawns anything, and neither installs the panic hook — there
        // is no terminal for it to restore.
        Some(Command::Stale { path, json }) => list(Listing::Stale, path, json),
        Some(Command::Fresh { path, json }) => list(Listing::Fresh, path, json),
        // The third question, beside the two listings and for their reasons. It
        // is the one that can answer "no" — a scope this machine's sigils do not
        // open — and it still exits 0 for it: a closed boundary is the answer,
        // not a failure to reach one, which is what leaves `jq -e '.opens'` to
        // spend the non-zero status on the verdict. See [`check`].
        Some(Command::Check { path, json }) => check(path, json),
        // The first subcommand that writes, dispatched here for every reason
        // the questions are — it prints one line on the ordinary screen and
        // takes no terminal — and with none of a run's machinery: no worker
        // thread, no subprocess, no model pass. What keeps it honest is the
        // boundary, asked before anything else it does; see [`mod@edits`].
        Some(Command::Unpact { path }) => unpact(&path),
        // The two runs, and the first subcommands that spend anything: minutes
        // of model passes, one `claude --print` per directory, a `WARLOCK.md`
        // beside each of them and one manifest save at the end. Dispatched here
        // with every other subcommand and for the same reasons — their progress
        // is lines on the ordinary screen that a script reads through a pipe, so
        // no alternate screen, no raw mode and no panic hook — and gated at the
        // same boundary the cheap writes are, asked before a single directory is
        // walked. See [`mod@running`].
        Some(Command::Pact { path }) => pact(&path),
        Some(Command::Refresh { path }) => refresh(&path),
        // The other two writes, dispatched beside it and through the same gate:
        // the boundary is asked before either of them looks at whether the path
        // has an entry at all, so a closed scope answers with the scope refusal
        // rather than with what the manifest does or does not hold. The nesting
        // is clap's and stops here — each arm is one call into [`mod@edits`],
        // with no work done in this match.
        Some(Command::Scope { command }) => match command {
            ScopeCommand::Add { path, scope } => scope_add(&path, &scope),
            ScopeCommand::Remove { path } => scope_remove(&path),
        },
    };

    // `run` has returned, so the guard inside it has already dropped and the
    // terminal is back to normal; only now is it worth printing anything,
    // because on the alternate screen nobody would ever see it. `init` and the
    // three questions never went near the terminal, and print through the same
    // line so that a failure looks the same however warlock was invoked.
    if let Err(error) = &outcome {
        eprintln!("warlock: {error}");
    }
    ExitCode::from(status_for(&outcome))
}

/// The status warlock leaves behind for `outcome`, as the number a shell sees.
///
/// A few lines, given a name and pulled out of `main`, because they are the
/// whole of the exit contract every headless subcommand promises and `main`
/// itself is the one function here that no test can call: a test that ran it
/// would run the event loop, or would have to spawn a process to avoid doing so.
/// As a function of the outcome it is ordinary code, and the modules that
/// produce those outcomes ([`query`], [`check`], [`mod@edits`]) pin their own end
/// of the contract against it — an empty listing and a scope closed to this
/// machine are both `Ok(())` from a question, and both are a 0.
///
/// The vocabulary the project agreed, in full. **0** completed: the question
/// was answered or the write happened, whatever the answer turned out to be.
/// **1** warlock could not do it: the repository will not resolve, the manifest
/// will not parse or will not save, the path has no repository-relative
/// spelling — the line on stderr is the thing to go and read. **2** a malformed
/// invocation, which is clap's and is never produced here at all: `Cli::parse`
/// has exited the process with it long before this is reached. **3** refused,
/// with nothing spent: this machine's sigils do not open the scope covering the
/// path a write was aimed at, so no byte of `.warlock/pacts.toml` moved,
/// retrying changes nothing, and the road out is `warlock config` rather than
/// anything in the message. **4** completed with failures: a `warlock pact` or
/// `warlock refresh` descended the subtree, wrote the documents it could and
/// saved the manifest, and some of its directories did not come out of it — the
/// directories are named on stderr, one line each, and the line this status goes
/// with says how many of how many ([`mod@running`]). **130** cancelled: somebody
/// pressed Ctrl-C during one of those two runs, the descent stopped at the next
/// directory and the pass in flight was killed, and what had finished by then is
/// hashed, granted and saved before the status is reached — 128 plus SIGINT, so
/// a shell, `make` and CI read it as interrupted without being told to.
///
/// 4 is not 1 for the reason 3 is not: they want different things done about
/// them. A 1 is warlock unable to do the thing and nothing having happened
/// — including a run whose manifest would not save, which stays a 1 however many
/// directories failed inside it, because a run whose record never reached the
/// disk is the bigger news and the one worth retrying. A 4 is the work done and
/// partly not taken: the grants that were earned are on disk, so the thing to do
/// is read the lines above the count and re-run over what failed.
///
/// 130 is not 4 for the same kind of reason, one step further along: a 4 is
/// warlock's news about a run, and a 130 is the reader's own news back. Nothing
/// went wrong in a cancelled run — the passes that finished are on disk and
/// granted, the rest were never asked for — so a script that retries a 4 over
/// the directories that failed must not retry a 130 at all, because the thing
/// that stopped it was somebody deciding to stop it. The number is not warlock's
/// invention: 128 plus the signal is what a shell reports for a killed process,
/// SIGINT is 2, and every wrapper that already special-cases 130 gets this run
/// right without being told anything about warlock.
///
/// 3 is here so that a script can act without reading English: the two non-zero
/// results a write can have want opposite things done about them, and telling
/// them apart by their wording is telling them apart by parsing prose. It is
/// spent on a write refused and never on a question — `warlock check` over a
/// boundary this machine does not hold still exits 0, because there a closed
/// scope is the answer rather than a failure to reach one. So no verdict a
/// question reached — nothing stale, nothing covering a path, a scope closed to
/// this machine — ever spends a non-zero status, which is what leaves that
/// status free for `jq -e '.opens'`.
///
/// [`Error::ClosedScopeBelow`] — the un-pact refused because the subtree it
/// would drop carries a scope this machine does not hold — stays a 1, and that
/// is a decision rather than an oversight. A 3 says the reader is *outside*:
/// the path they aimed at is not theirs to touch, they were told nothing about
/// what the manifest holds past it, and one road leads out. The descendant
/// refusal says the reverse about the same reader — the boundary over that path
/// already said yes and they may work there — and what it refuses is the blast
/// radius, which is why its own sentence offers a second road that needs no
/// sigil at all: un-pact the parts you hold. A script reading a 3 as "this
/// checkout is locked out of that path, stop" would be wrong about it, and it
/// is that wrongness which decides this rather than the tidiness of one number
/// per rule. It is the weaker half of the fit — nothing was spent there either
/// — so if the pact/refresh slice finds callers wanting the two together,
/// moving it is one arm of the match below.
const fn status_for(outcome: &Result<(), Error>) -> u8 {
    match outcome {
        Ok(()) => 0,
        // The boundary, and only the upward one: see the decision above.
        Err(Error::ClosedScope { .. }) => 3,
        // A run that finished with some of its directories failed. Above the
        // catch-all rather than folded into it, because it is the one non-zero
        // status that comes with the work having been done: the documents that
        // could be written are written and the manifest is saved, and the line
        // printed for it is a count under a list already on stderr.
        Err(Error::Failures { .. }) => 4,
        // A run somebody stopped, and the one status here that is not warlock's
        // verdict on anything: the work up to the Ctrl-C is saved, so this sits
        // beside the 4 rather than under the catch-all, and it is the number a
        // shell already spells an interrupted process with.
        Err(Error::Cancelled) => CANCELLED,
        Err(_) => 1,
    }
}

/// What an interrupted run leaves behind: 128 plus SIGINT, the number every
/// shell already reports for a process that took a Ctrl-C.
///
/// Named once and read twice, which is the whole reason it is a constant: the
/// first Ctrl-C ends with [`status_for`] mapping [`Error::Cancelled`] to it, and
/// the second leaves through the handler in [`mod@running`] without any outcome
/// to map — and the two must not be able to drift into telling one shell two
/// different stories about the same keypress.
const CANCELLED: u8 = 130;

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
    let (app, scope, tree) = load_app()?;
    // Loaded before the terminal is touched, for the same reason the tree is:
    // a manifest that will not parse should say so on the normal screen. This
    // is a second read of the file the loader already parsed, which is cheap
    // and keeps the front end from reaching into the loader's internals for a
    // value it needs to keep and edit.
    let manifest = load_manifest(&scope.repo_root)?;
    // Asked for once, over the tree the load just produced, and kept for as
    // long as warlock runs — dropping it stops the watch. Whether it was
    // granted is a fact for the footer and nothing more, which is why this is
    // not a `?` and why the line about it is put up in there rather than here:
    // warlock with no live updates is warlock as it was. See [`start_watching`].
    let mut app = app;
    let watched = start_watching(&mut app, &scope, &tree);

    // The terminal, taken last, so everything above that can fail says so on
    // the ordinary screen. From here to the end of this function the alternate
    // screen is up, and the guard is what puts it back — on the return below,
    // on a `?`, and on a panic through the hook installed in `main`.
    // The conversation's root, taken before `scope` moves into the session.
    let root = scope.repo_root.clone();
    let mut session = Session {
        app,
        screen: TerminalGuard::enter()?,
        scope,
        manifest,
        // Built once, and cheap to build: an agent is a command line and a
        // timeout, so no `claude` exists until a key asks for a pass or a turn.
        pact: Pact::new(),
        chat: Chat::new(root),
        confirm: QuitConfirm::default(),
        prompt: ScopePrompt::default(),
        document: None,
        // The terminal has just been asked to report its pointer, and `m` is
        // the one thing that changes the answer.
        mouse_captured: true,
        watched,
    };

    loop {
        // Measured once a round, because the round needs it twice: it is what
        // the frame is cut by and what a click landing on that frame is
        // hit-tested against, and those two have to be the one answer.
        let size = session.size()?;
        session.draw(size)?;

        // Waited on rather than blocked on. Nothing is drawn while this thread
        // sits here, so the wait has to end whether or not anybody presses
        // anything: a pact reports its progress over a channel that only the
        // bottom of this loop reads, and a progress line that waits for a
        // keystroke to appear is worse than none at all.
        if event::poll(POLL_INTERVAL)? {
            // The instant the event arrived, read once and here: it is the
            // instant anything this key starts is as old as, so a turn and a
            // pass are both clocked from the keystroke that asked for them
            // rather than from the first thing the model got round to saying.
            let now = Instant::now();
            match event::read()? {
                // `false` is the one thing a press can say that is answered
                // here, and it says the session is over. Returning is the whole
                // of quitting and it happens on this stack, so the session
                // drops on the way out — cancelling any run and killing the
                // `claude` it was waiting on — and its screen drops with it and
                // puts the terminal back.
                Event::Key(key) => {
                    if !session.press(key, now)? {
                        return Ok(());
                    }
                }
                // The pointer, at the size the frame above was drawn at. None of
                // what it does reads the terminal and none of it draws: the
                // round is the redraw, which is why a pointer swept across the
                // screen costs nothing.
                Event::Mouse(mouse) => session.point(mouse, size),
                _ => {}
            }
        }

        // And then everything that happened off this thread: what a run has
        // said since the last round, what a turn has, and what the disk did
        // while this one was waiting on a keystroke.
        session.keep_up();
    }
}

/// One warlock session: everything on screen, everything in flight, and
/// everything a keystroke is allowed to move.
///
/// The eleven values [`run`] used to keep as locals, owned in one place. They
/// were locals because they *are* one thing — a session — and the loop proved it
/// by threading overlapping subsets of them through four different argument
/// lists, thirty-one parameters in all, plus a struct of borrows built afresh
/// every round to carry the widest of those subsets. Adding one fact about a
/// session meant editing four signatures and the struct.
///
/// So the four are methods now, and a round is [`Session::draw`],
/// [`Session::press`] or [`Session::point`], and [`Session::keep_up`]. What a
/// method needs it reaches for; nothing is handed to it that it already had.
///
/// The two prompts are fields here and deliberately *not* fields on the
/// [`App`]: a window that lived on the app would be a window a restored copy of
/// the app could put back up, and an Esc could have changed something with. The
/// app is copied and put back by a run that recorded nothing; a session is not.
///
/// It is generic over its three seams and over nothing else. Warlock runs it on
/// a [`TerminalGuard`], a [`ClaudeAgent`] and a [`ChatAgent`]; a test runs it on
/// three values that answer out of memory, which is the whole reason a round can
/// be driven at all.
struct Session<S: Screen, P: Wired + Agent, C: Converses> {
    /// What is on screen: the tree, the panel, the register and the footer.
    app: App,
    /// The screen this session is drawn on, given away for `e` and taken back.
    screen: S,
    /// The repository root and what warlock was pointed at. Settled by the load
    /// and never moved afterwards.
    scope: Scope,
    /// The manifest this thread holds, which the scope prompt reads and writes.
    manifest: Manifest,
    /// The pact key's business: the agent runs are made with, and the run
    /// happening somewhere else when there is one. Never the conversation's
    /// agent, which is inside `chat`.
    pact: Pact<P>,
    /// The conversation, and the turn being answered when there is one.
    chat: Chat<C>,
    /// The gate on the way out.
    confirm: QuitConfirm,
    /// The scope prompt: the window `s` opens.
    prompt: ScopePrompt,
    /// Which file the panel's document card is holding.
    document: Option<PathBuf>,
    /// Whether the terminal is reporting its mouse.
    mouse_captured: bool,
    /// The filesystem watcher, and what it has been told about the tree.
    watched: Watched,
}

impl<S: Screen, P: Wired + Agent, C: Converses> Session<S, P, C> {
    /// How big the screen is right now.
    ///
    /// Measured once a round and handed to the three methods that cut a layout
    /// by it, so a frame, a click and the app's own idea of its viewport cannot
    /// disagree about the size they were computed at.
    ///
    /// # Errors
    ///
    /// Whatever the screen says when it cannot be measured.
    fn size(&self) -> io::Result<Size> {
        self.screen.size()
    }

    /// Tell the app how big the frame it is about to be drawn in is, and draw
    /// it.
    ///
    /// Told before it is drawn, and every frame rather than on resize: the
    /// scroll offset is only right if it was computed against the height this
    /// frame gives the tree, and [`tree_height`] is the same layout the frame is
    /// cut by. A terminal resized between frames is handled by that alone — the
    /// next frame measures again, and the next frame is at most one
    /// [`POLL_INTERVAL`] away. The panel's window is measured the same way and
    /// for the same reason, off the same size: [`panel_height`] and
    /// [`tree_height`] are two answers from the one layout.
    ///
    /// The composer and the run's header are the two things that take rows off
    /// the panel, and both are read *here*, once, and handed to
    /// [`panel_height`] and to [`draw`] — so the measurement and the drawing
    /// cannot disagree about how many rows the account has.
    ///
    /// `size` is the caller's rather than measured here, because the round needs
    /// it again: a click is hit-tested against the size of the frame it landed
    /// on, and measuring twice would be two answers where the hit test needs
    /// one.
    ///
    /// What the terminal is doing with the pointer is told to the app here as
    /// well. Every frame rather than at the keystroke, so a view restored from
    /// the copy taken before a pact — a copy of a flag that may have been
    /// toggled since — is put right before it is drawn.
    ///
    /// The instant the frame is drawn at is read here and handed to the
    /// renderer: the panel's newest clock counts up against it, so a frame drawn
    /// with no event waiting still shows a run that is moving. See [`draw`].
    ///
    /// # Errors
    ///
    /// Whatever the screen says when the frame cannot be written.
    fn draw(&mut self, size: Size) -> io::Result<()> {
        self.app.set_mouse_captured(self.mouse_captured);
        let field = composer_on_screen(&self.app, self.chat.composer());
        let header = self.app.run_header();
        self.app.set_viewport_height(tree_height(size));
        self.app
            .set_panel_height(panel_height(size, field, header.as_ref()));
        self.app.set_panel_width(panel_width(size));

        let (app, chrome, confirm, prompt) =
            (&self.app, &self.scope.chrome, self.confirm, &self.prompt);
        let write = self.chat.write_prompt();
        self.screen.draw(|frame| {
            draw(
                frame,
                app,
                chrome,
                Instant::now(),
                confirm,
                prompt,
                write,
                field,
            );
        })
    }

    /// What a pointer event at `size` comes to.
    ///
    /// A thin arm onto [`apply_mouse`], which stays a function over an app and
    /// a layout: what a click *means* is decidable with nothing owned, and a
    /// session is not needed to assert it.
    fn point(&mut self, mouse: MouseEvent, size: Size) {
        apply_mouse(
            &mut self.app,
            mouse,
            size,
            self.confirm,
            &self.prompt,
            self.chat.write_prompt(),
            self.chat.composer(),
        );
    }

    /// What one keystroke comes to, with everything it can move at `pressing` and
    /// the instant it arrived at `now`: `true` when the session goes on and `false`
    /// when it is over.
    ///
    /// The loop's list of consequences, and it is a list rather than a decision:
    /// what a key *means* is [`press_for`]'s, and this is what warlock does about
    /// the answer. Splitting it out of [`run`] changes nothing about either — the
    /// arms are the arms, in the order they were in — and it keeps the round above
    /// short enough to read: draw, wait, press, drain.
    ///
    /// The four situations a key is read against are taken here, once each, and
    /// immediately: whether there is a draft for it to land in, which is the
    /// keyboard being pointed at the composer and is offered to [`press_for`]
    /// rather than looked up there; whether a run is in flight; whether a turn is
    /// being answered; and the instant above. One reading apiece, before anything
    /// is done, so the arms below cannot disagree with the gate about what was
    /// going on when the key was pressed.
    ///
    /// The only way out is `false`, and only [`Pressed::Leave`] and the app's old
    /// quit produce it. Leaving is deliberately not done from in here: the run's
    /// handle and the terminal guard are [`run`]'s to drop, in the order it has
    /// always dropped them.
    fn press(&mut self, key: KeyEvent, now: Instant) -> Result<bool, Error> {
        // The composer is offered on exactly the condition that lights its border,
        // which is the keyboard being pointed at it: with the keys anywhere else
        // this is `None`, there is no draft to type into, and every letter is the
        // command it has always been.
        let typing = (self.app.focus() == Focus::Composer).then(|| self.chat.composer());
        let running = self.pact.running();
        let asked = self.chat.answering();
        let pressed = press_for(
            key,
            self.confirm,
            &self.prompt,
            self.chat.write_prompt(),
            typing,
            running,
            asked,
        );

        match pressed {
            // Saying the session is over is the whole of quitting, and it is
            // enough even with a pact in flight: [`run`] returns on this answer and
            // returning is what does the rest. `pact` drops on the way out, which
            // cancels the run and kills the `claude` it was waiting on (see
            // [`Running`]); the guard drops after it and puts the terminal back.
            // Nothing joins the worker: it is left to be ended by the process,
            // having written whole documents or none, and the manifest it never got
            // to rewrite still says what it said before.
            //
            // Every way out arrives here: a Yes to the question, Ctrl-C, and `q`
            // during a run. The second spelling is the app's old quit, which
            // [`press_for`] no longer produces — naming it beside the first keeps
            // one road out of this loop rather than two that have to be kept doing
            // the same thing.
            Pressed::Leave | Pressed::Act(Action::Quit) => return Ok(false),
            // The question, opened, moved, or taken down again. Nothing else
            // happens and nothing else needs to: the app was never touched, so a
            // No has nothing to put back, and the top of this loop draws whatever
            // the question now is.
            Pressed::Confirm(next) => self.confirm = next,
            // Esc with a run in flight. The handle does both halves at once — it
            // latches, so the descent stops at the next directory instead of
            // starting a pass for it, and it kills the `claude` running right now,
            // so that stop happens in milliseconds rather than at the end of a
            // five-minute pass.
            //
            // The pact is deliberately *not* taken down here. The worker is still
            // going to hash what it wrote, save the manifest and report, and all
            // of that arrives at the bottom of this loop like any other outcome;
            // forgetting about it now would leave the footer's progress line up
            // for a run nobody was listening to any more.
            Pressed::Act(Action::CancelPact) => self.pact.stop(),
            // Ctrl-C with a turn being answered, and the one keystroke in warlock
            // that stops something without leaving. The handle is the turn's own —
            // the same `Cancel` a run is stopped through — so it kills the
            // `claude` this turn is waiting on and the worker comes back within
            // milliseconds.
            //
            // The turn is deliberately *not* taken down here, exactly as a
            // cancelled pact is not: the worker still has one thing to say, and it
            // says it at the bottom of this loop like any other ending. That is
            // what puts the cancelled line under whatever work had already arrived
            // and gives the field the keyboard back — a turn forgotten here would
            // leave the composer muted for the rest of the session.
            Pressed::CancelTurn => self.chat.stop(),
            // Nothing but a bit of view state moves here, and deliberately so:
            // focus decides which border the next frame lights and which pane a
            // movement key is about, and both of those questions are answered
            // where they are asked — by the renderer reading `App::focus`, and by
            // the app's own movement methods, which move the tree's selection or
            // scroll the panel's window depending on the pane being driven
            // (WAR-26.02). There is nothing for this arm to gate a second time,
            // and no message: a key that changes what the *next* key means has
            // nothing to report.
            Pressed::Act(Action::ToggleFocus) => self.app.toggle_focus(),
            Pressed::Act(Action::SelectPrevious) => self.app.select_previous(),
            Pressed::Act(Action::SelectNext) => self.app.select_next(),
            // No height is passed: the app was told the viewport's height at the
            // top of this loop, so a page is whatever the frame just drawn could
            // show.
            Pressed::Act(Action::SelectPageUp) => self.app.select_page_up(),
            Pressed::Act(Action::SelectPageDown) => self.app.select_page_down(),
            Pressed::Act(Action::SelectFirst) => self.app.select_first(),
            Pressed::Act(Action::SelectLast) => self.app.select_last(),
            // Nothing else happens here on purpose. What is collapsed is the front
            // end's view of the tree and never touches disk (§8), so there is no
            // manifest to write; the tree has not changed, so there is nothing to
            // re-read. The app moves the selection and the scroll offset back into
            // range itself, and the next frame — the top of this same loop — draws
            // the shorter or longer list.
            Pressed::Act(Action::ToggleCollapsed) => self.app.toggle_collapsed(),
            // Nothing else happens here either, and for the same reasons as
            // collapsing: which rows are worth looking at is the front end's view
            // of the tree and is never written down (§5), so there is no manifest
            // to save, and the tree itself has not changed, so there is nothing to
            // re-read. The app re-flows its rows and puts the selection and the
            // scroll offset back in range; the next frame draws whatever is left.
            Pressed::Act(Action::TogglePactedOnly) => self.app.toggle_pacted_only(),
            // Nothing else here either, for the third time and for the same
            // reasons as the two arms above: whether the files inside a module are
            // on screen is the front end's view of the tree and is never written
            // down (§5), so there is no manifest to save, and the files were read
            // by the load that built these rows, so there is nothing to re-read.
            // The app re-flows its rows and keeps the selection and the scroll
            // offset in range; the next frame draws the longer or shorter list.
            Pressed::Act(Action::ToggleFiles) => self.app.toggle_files(),
            // The two keystrokes that write anything, and the two that take longer
            // than a frame — so they are the ones that are not done here. Both go
            // to a worker thread and both fill the one run `Pact` keeps, which is
            // what makes them refuse each other; everything they produce arrives at
            // the bottom of this loop, one directory at a time and finally as an
            // outcome, and until it does the loop goes round as usual — drawing,
            // scrolling, filtering.
            //
            // Two arms rather than one, because the list of keys is what this
            // function is for and a key dispatching on a value computed elsewhere
            // would be a key a reader could not find here. The kind is the app's own
            // [`Run`], the only thing the two runs differ by all the way down;
            // everything either side of it, and everything they refuse, is
            // [`Pact::press`]'s.
            Pressed::Act(Action::TogglePact) => {
                self.pact
                    .press(Run::Pact, &mut self.app, &self.manifest, &self.scope, now);
            }
            Pressed::Act(Action::Refresh) => {
                self.pact.press(
                    Run::Refresh,
                    &mut self.app,
                    &self.manifest,
                    &self.scope,
                    now,
                );
            }
            // The one key that reads a file and the only one that shows anything a
            // model wrote. It is done here, on this thread, between two frames: a
            // read capped at a few kilobytes is over inside a frame, so there is
            // no worker, no channel and no account, and nothing to reload
            // afterwards because reading a file changes nothing about the tree.
            //
            // It needs no answer of its own, for the reason the scope key's arm
            // needs none: everything this press can refuse it refuses inside
            // `view_press` — a directory row through `App::message` — and every
            // way the read itself can fail ends as one line on that same footer
            // with the panel left as it was. So the loop goes round again after a
            // failure exactly as it does after a success. Unlike `p`, `r` and `s`
            // it is not handed the run: a read races nothing, so there is nothing
            // for a run in flight to refuse. See `viewing::view_press`.
            //
            // What comes back is the file that is now on the document card, and it
            // is kept here because the app is never told: `App::show_document`
            // takes lines and never a path, so "which file the panel is holding"
            // is this loop's to know. The one thing that asks for it is the edit
            // key, which re-reads the card only when the file it just handed to an
            // editor is the file on it. A press that read nothing — refused, or a
            // read that failed — leaves the card holding what it held, which is
            // why what was remembered before is what a `None` falls back to rather
            // than being cleared.
            Pressed::Act(Action::ViewFile) => {
                if let Some(read) = view_press(&mut self.app) {
                    self.document = Some(read);
                }
            }
            // The one key that gives the screen away, and the only one whose
            // answer is measured in minutes of somebody typing rather than in
            // frames. The loop stops here for the whole of it: the terminal is put
            // back the way warlock found it, `$EDITOR` is run on the selected file
            // as a foreground child, the child is waited on, and the terminal is
            // taken again — every one of those through the guard this loop already
            // holds, so there is one spelling of teardown and one of setup (see
            // `Screen::suspended`). Nothing is drawn behind the editor and
            // no progress event is drained while it runs, which is the honest
            // shape of handing the terminal to somebody else.
            //
            // `mouse_captured` is handed over because it is this thread's only
            // record of what `m` last did: resuming without it would switch
            // reporting back on behind a reader who turned it off.
            //
            // It needs no answer of its own, for the reason `v`'s and `s`'s arms
            // need none: everything this press can refuse it refuses inside
            // `edit_press` — a directory row in the very words `v` uses, a run in
            // flight on the progress line, an `$EDITOR` naming nothing on the
            // footer — and both ways the child itself can go wrong end as one line
            // on that footer with the loop going round again. The `?` is the
            // terminal and only the terminal: a screen that could not be taken
            // back is not news for a footer nobody could read, so it leaves
            // through the guard like every other terminal failure. See
            // `editing::edit_press`.
            //
            // Two things are read again on the way back, and both are inside
            // `edit_press`: the tree, so a directory whose file changed goes
            // yellow without a further keystroke, and the document card — but only
            // when the file just edited is the one on it, which is what `document`
            // is kept for. Which card is showing does not move for either.
            Pressed::Act(Action::EditFile) => {
                edit_press(
                    &mut self.app,
                    &mut self.screen,
                    &self.scope,
                    self.document.as_deref(),
                    self.mouse_captured,
                    running,
                )?;
            }
            // The panel's other card, and nothing else: the account if the
            // document is up, the document if the account is. It is done here, on
            // this thread, without reading anything — both cards are already in
            // the app, so a swap is one field moved and the next frame drawing the
            // other one.
            //
            // It needs no answer of its own, for `v`'s reason: the one thing this
            // press can refuse — a session with no document read yet — it refuses
            // inside `App::swap_card`, which leaves the panel on the account and
            // puts a line on the footer naming the key that would make a second
            // card. A swap that worked says nothing, because the reader can see
            // it. Unlike `p`, `r` and `s` it is not handed the run: a swap races
            // nothing, and a run that changed which card is showing would take a
            // document out of the reader's hands.
            Pressed::Act(Action::SwapCard) => self.app.swap_card(),
            // The one key that answers to the terminal rather than to the app. The
            // sequence is written first and the flag moved only if it went out, so
            // what this thread believes about the terminal is what it last
            // successfully told it; a write that fails takes the whole loop down
            // through the guard, which turns capture off on the way past whatever
            // state it was left in.
            //
            // Nothing else happens: no focus moves, no row is selected, nothing is
            // redrawn here — the top of the loop draws every round, and it is
            // where the footer picks the new wording up. With capture off the
            // terminal keeps the pointer to itself, so `Event::Mouse` simply stops
            // arriving and the mouse handler needs no gate of its own.
            Pressed::Act(Action::ToggleMouseCapture) => {
                self.screen.report_mouse(!self.mouse_captured)?;
                self.mouse_captured = !self.mouse_captured;
            }
            // The third key that writes to disk, and the one that is not a run: it
            // opens a window holding the scope the selected directory carries now,
            // read out of the manifest this loop is already holding. Everything it
            // can refuse it refuses inside `scope_press` — a file row and an
            // unpacted one through `App::message`, a press during a run through
            // the progress line — and every one of those comes back as a prompt
            // that is still closed, so this arm needs no `None` case of its own.
            // See `scoping::scope_press`.
            Pressed::Act(Action::OpenScope) => {
                self.prompt = scope_press(
                    &mut self.app,
                    &self.manifest,
                    &self.scope.repo_root,
                    self.scope.chrome.sigils(),
                    running,
                );
            }
            // Somebody typing into that window: a character more or less in the
            // field, the window abandoned, or — on Enter — the manifest written.
            // The whole of that last one happens here, on this thread, between two
            // frames: no worker, no channel, no account and no reload, because a
            // scope is one string written into one entry of a file already in this
            // thread's hand (see `mod@scoping`). What comes back is the prompt
            // from here on — down for a submit that was answered, still up over
            // the text for one the engine refused. See `scoping::scope_edit`.
            Pressed::Scope(edited) => {
                self.prompt = scope_edit(
                    &mut self.app,
                    &mut self.manifest,
                    &self.scope.repo_root,
                    &self.prompt,
                    edited,
                );
            }
            // Somebody typing into the other window: a character more or less in
            // the path, the window abandoned, or — on Enter — the document
            // written. The whole of that last one happens here too, on this
            // thread, between two frames: the bytes are the answer already on the
            // card and the destination is the line on screen, so there is nothing
            // to spawn and nothing to wait for (see `mod@writing`). What comes
            // back is the prompt from here on — down for a write that happened and
            // for an Esc that wrote nothing, still up over the typed path for one
            // that was refused. An Esc changes nothing else at all: the reply
            // stays on the card and the register stays what it was, because the
            // app was never told the question was asked. See
            // `writing::write_edit`.
            Pressed::Write(edited) => {
                self.chat.write(&mut self.app, &self.manifest, edited, now);
            }
            // Somebody typing at the foot of the panel's column: a character more
            // or less in the draft, the keyboard handed back, or a draft offered
            // up. What each of those comes to is [`apply_compose`], which is
            // handed the local above rather than reaching for anything on the app
            // — what is in the draft is not a fact about the tree.
            //
            // The output directory goes in for the same reason and in the same
            // shape: it is a local of the loop, `/brief` is the one thing that
            // settles it, and this is where `/brief` is answered — so it goes in
            // borrowed rather than being fetched from somewhere in there.
            //
            // The last of the three is now a worker thread, so the agent and the
            // turn go in with it, and the instant the key was pressed goes in as
            // well for the pact key's reason: a turn is as old as the question
            // that asked it, not as old as the first thing the model got round to
            // saying.
            Pressed::Compose(outcome) => self.chat.compose(&mut self.app, outcome, now),
            // A key nothing is bound to, or one whose press has already been
            // answered where it was decided.
            Pressed::Nothing => {}
        }

        Ok(true)
    }

    /// Keep up with what is happening off this thread: the run's progress, the
    /// turn's, and the disk moving under the tree.
    ///
    /// The bottom of every round, whether or not a key was pressed. [`Pact::keep_up`]
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
    /// The turn is drained last and by exactly the same rules: [`apply_turn`] takes
    /// whatever the worker has said since the last frame and returns rather than
    /// waiting, so the frame goes on redrawing while a question is being answered
    /// and a burst of tool calls that arrived between two frames lands in the order
    /// it happened. Last rather than first only because the two are independent —
    /// a turn writes no file, changes no row and reloads nothing, so there is no
    /// order between it and the run for the screen to disagree about. It is where
    /// the muting ends: the drain is what takes `turn` down, however the turn
    /// finished, and the top of the next round hands the field back on the strength
    /// of that alone.
    ///
    /// The clock is read once, here, and handed to everything under this: the round
    /// is measured against one instant, which is when the events being drained now
    /// land on screen — within one [`POLL_INTERVAL`] of when they were sent. It is
    /// read at the top rather than per call for the reason the frame reads its own:
    /// two readings a round would be two answers to one question.
    fn keep_up(&mut self) {
        let now = Instant::now();
        // The one round in a run's life the watcher has to hear about, and it says
        // so itself: a `Reloaded` comes back on the round the run ended and on no
        // other, carrying the tree that reload read. This loop used to work that
        // edge out by reading `is_some()` either side of the drain and comparing —
        // a detector kept by hand over a fact the run already knew. See
        // [`Reloaded`].
        if let Some(Reloaded(tree)) =
            self.pact
                .keep_up(&mut self.app, &mut self.manifest, &self.scope, now)
        {
            self.watched.caught_up(tree.as_ref(), now);
        }
        self.watched
            .round(&mut self.app, &self.scope, self.pact.running(), now);
        // And the conversation's own bottom end. Nothing comes back: a `/write`
        // turn's answer opens the window that goes over it, and that window is the
        // conversation's, so it is opened in there rather than here out of two
        // values this loop would otherwise have to be handed. See [`Chat::keep_up`].
        self.chat.keep_up(&mut self.app, now);
    }
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
    path_prompt: &ScopePrompt,
    composer: &Composer,
) {
    let field = composer_on_screen(app, composer);
    match mouse_action(mouse, size, app, confirm, prompt, path_prompt, field) {
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

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};
    use std::time::{Duration, Instant};
    use std::{fs, io};

    use clap::error::ErrorKind;
    use clap::{CommandFactory, Parser};
    use ratatui::backend::TestBackend;
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use ratatui::layout::Size;
    use ratatui::{Frame, Terminal};
    use warlock_engine::{Loaded, Manifest, Node, NodeState, Tree, load_tree, repository_root};
    use warlock_tui::{App, Chrome, QuitConfirm, Row, ScopePrompt, tree_height};

    use super::{Cli, Command, Error, FOR_CLAUDE_MD, ScopeCommand, Session, status_for};
    use crate::chatting::Chat;
    use crate::pacting::Pact;
    use crate::query::spelled;
    use crate::session::{Scope, Watched};
    use crate::stubs::{Passing, Saying};
    use crate::terminal::Screen;

    /// What warlock would do, given `args` after the program's own name.
    ///
    /// `try_parse_from` wants argv as the process gets it, program name and all,
    /// so the name is put back here and every test below reads as the words a
    /// person types.
    fn parse(args: &[&str]) -> Result<Cli, clap::Error> {
        let typed = std::iter::once("warlock").chain(args.iter().copied());
        Cli::try_parse_from(typed)
    }

    /// The parser clap built for the subcommand spelled by `names`, walked one
    /// word at a time so that a nested pair — `["scope", "add"]` — is reached the
    /// way a reader types it.
    ///
    /// A clone rather than a borrow, because [`clap::Command::render_long_help`]
    /// wants the command by mutable reference and the tests below want several of
    /// them from one parser.
    fn subcommand(names: &[&str]) -> clap::Command {
        let mut command = Cli::command();
        for name in names {
            let found = command
                .find_subcommand(name)
                .unwrap_or_else(|| panic!("warlock has a `{name}` subcommand"))
                .clone();
            command = found;
        }

        command
    }

    #[test]
    fn the_binary_is_named_warlock() {
        assert_eq!(env!("CARGO_BIN_NAME"), "warlock");
    }

    #[test]
    fn the_parser_itself_is_well_formed() {
        // Every misuse of the derive that clap can catch — a duplicate long
        // flag, a subcommand named twice — is a panic here rather than in
        // somebody's terminal.
        Cli::command().debug_assert();
    }

    #[test]
    fn no_arguments_opens_the_tree() {
        assert_eq!(parse(&[]).unwrap().command, None);
    }

    #[test]
    fn init_and_config_are_the_subcommands() {
        assert_eq!(parse(&["init"]).unwrap().command, Some(Command::Init));
        assert_eq!(parse(&["config"]).unwrap().command, Some(Command::Config));
    }

    #[test]
    fn a_listing_with_no_path_asks_about_the_whole_repository() {
        // `None` and not the working directory: what an omitted path means is
        // `query::list`'s to decide, and it decides on the repository root, so
        // the parser hands over the absence rather than filling it in.
        assert_eq!(
            parse(&["stale"]).unwrap().command,
            Some(Command::Stale {
                path: None,
                json: false
            })
        );
        assert_eq!(
            parse(&["fresh"]).unwrap().command,
            Some(Command::Fresh {
                path: None,
                json: false
            })
        );
    }

    #[test]
    fn a_listing_takes_a_path_and_a_json_flag_in_either_order() {
        assert_eq!(
            parse(&["stale", "crates"]).unwrap().command,
            Some(Command::Stale {
                path: Some(PathBuf::from("crates")),
                json: false
            })
        );
        for args in [["stale", "crates", "--json"], ["stale", "--json", "crates"]] {
            assert_eq!(
                parse(&args).unwrap().command,
                Some(Command::Stale {
                    path: Some(PathBuf::from("crates")),
                    json: true
                }),
                "{args:?}"
            );
        }
        assert_eq!(
            parse(&["fresh", "--json"]).unwrap().command,
            Some(Command::Fresh {
                path: None,
                json: true
            })
        );
    }

    #[test]
    fn a_listing_answers_about_one_path_rather_than_a_list_of_them() {
        // `warlock stale a b` is somebody expecting one of two things warlock
        // does not do — several paths, or a second flag spelled as a word — and
        // either way an answer about `a` alone would be an answer to a question
        // nobody asked. Clap's refusal, so it is a 2 and not a 1.
        for args in [
            ["stale", "a", "b"].as_slice(),
            ["fresh", "a", "b"].as_slice(),
            ["stale", "--jsonn"].as_slice(),
        ] {
            let error = parse(args).unwrap_err();
            assert!(error.use_stderr(), "{args:?}");
            assert_eq!(error.exit_code(), 2, "{args:?}");
        }
    }

    #[test]
    fn a_check_takes_the_path_it_is_a_check_of_and_a_json_flag_in_either_order() {
        assert_eq!(
            parse(&["check", "crates/engine"]).unwrap().command,
            Some(Command::Check {
                path: PathBuf::from("crates/engine"),
                json: false
            })
        );
        for args in [
            ["check", "crates/engine", "--json"],
            ["check", "--json", "crates/engine"],
        ] {
            assert_eq!(
                parse(&args).unwrap().command,
                Some(Command::Check {
                    path: PathBuf::from("crates/engine"),
                    json: true
                }),
                "{args:?}"
            );
        }
    }

    #[test]
    fn a_check_with_no_path_is_a_malformed_invocation_rather_than_a_whole_repository_answer() {
        // Unlike the two listings, whose omitted path means the repository
        // root: a check is a walk up from one place, so there is no
        // whole-repository answer for an absence to mean. Clap's refusal, so it
        // is a 2 and not warlock answering about something nobody named.
        for args in [
            ["check"].as_slice(),
            ["check", "--json"].as_slice(),
            // And one path, as everywhere else here.
            ["check", "a", "b"].as_slice(),
        ] {
            let error = parse(args).unwrap_err();
            assert!(error.use_stderr(), "{args:?}");
            assert_eq!(error.exit_code(), 2, "{args:?}");
        }
    }

    #[test]
    fn an_unpact_takes_the_one_directory_it_un_pacts_and_takes_it_from_the_reader() {
        assert_eq!(
            parse(&["unpact", "crates/engine"]).unwrap().command,
            Some(Command::Unpact {
                path: PathBuf::from("crates/engine")
            })
        );
        // The whole-manifest edit is a path somebody typed, and it is spelled
        // like any other: the parser has no default standing behind it, so the
        // largest edit warlock can make is never the one a missing argument
        // makes by itself.
        assert_eq!(
            parse(&["unpact", "."]).unwrap().command,
            Some(Command::Unpact {
                path: PathBuf::from(".")
            })
        );
    }

    #[test]
    fn an_unpact_with_no_path_or_with_two_is_a_malformed_invocation() {
        // An omitted path is not the repository root here, unlike the two
        // listings: it is clap's 2, for the reason above.
        let malformed: [&[&str]; 3] = [
            &["unpact"],
            &["unpact", "a", "b"],
            &["unpact", "--nonsense"],
        ];

        for args in malformed {
            let error = parse(args).unwrap_err();
            assert!(error.use_stderr(), "{args:?}");
            assert_eq!(error.exit_code(), 2, "{args:?}");
        }
    }

    #[test]
    fn the_two_runs_take_the_one_subtree_they_descend_and_take_it_from_the_reader() {
        assert_eq!(
            parse(&["pact", "crates/engine"]).unwrap().command,
            Some(Command::Pact {
                path: PathBuf::from("crates/engine")
            })
        );
        assert_eq!(
            parse(&["refresh", "crates/engine"]).unwrap().command,
            Some(Command::Refresh {
                path: PathBuf::from("crates/engine")
            })
        );
        // The whole repository, spelled by somebody who meant it. The largest
        // run warlock can start — minutes of passes over every directory there
        // is — is never the one an omitted argument starts by itself.
        assert_eq!(
            parse(&["pact", "."]).unwrap().command,
            Some(Command::Pact {
                path: PathBuf::from(".")
            })
        );
    }

    #[test]
    fn a_run_with_no_path_or_with_two_is_a_malformed_invocation() {
        // Clap's 2, for the un-pact's reason with the money on it: a run that
        // guessed at what to descend would have spent the tokens before anybody
        // could say it guessed wrong.
        let malformed: [&[&str]; 6] = [
            &["pact"],
            &["pact", "a", "b"],
            &["pact", "--nonsense"],
            &["refresh"],
            &["refresh", "a", "b"],
            &["refresh", "--nonsense"],
        ];

        for args in malformed {
            let error = parse(args).unwrap_err();
            assert!(error.use_stderr(), "{args:?}");
            assert_eq!(error.exit_code(), 2, "{args:?}");
        }
    }

    #[test]
    fn no_flag_on_a_write_or_a_run_gets_past_the_boundary_or_asks_for_an_object() {
        // Two absences, pinned where they are decided. There is no `--force`,
        // `--yes` or any other word that skips the scope check — `warlock
        // config` is the one road past a boundary, and a flag that existed
        // would be a second. And there is no `--json`: the three questions
        // answer in objects because something reads their answers, while these
        // five say what they did as they do it and spend the status on whether
        // it happened. The two runs are here for the stronger form of the first
        // reason: a flag past their boundary would spend somebody's tokens
        // rewriting somebody else's documents.
        let refused: [&[&str]; 14] = [
            &["pact", "crates", "--force"],
            &["pact", "--force", "crates"],
            &["pact", "crates", "--json"],
            &["refresh", "crates", "--force"],
            &["refresh", "crates", "--json"],
            &["unpact", "crates", "--force"],
            &["unpact", "--force", "crates"],
            &["unpact", "crates", "--json"],
            &["scope", "add", "crates", "web", "--force"],
            &["scope", "add", "crates", "web", "--yes"],
            &["scope", "add", "crates", "web", "--json"],
            &["scope", "remove", "crates", "--force"],
            &["scope", "remove", "crates", "--json"],
            &["scope", "--force", "remove", "crates"],
        ];

        for args in refused {
            let error = parse(args).unwrap_err();
            assert!(error.use_stderr(), "{args:?}");
            assert_eq!(error.exit_code(), 2, "{args:?}");
        }
    }

    #[test]
    fn the_two_scope_writes_are_a_noun_and_a_verb_rather_than_two_words_run_together() {
        assert_eq!(
            parse(&["scope", "add", "crates/engine", "data-plane"])
                .unwrap()
                .command,
            Some(Command::Scope {
                command: ScopeCommand::Add {
                    path: PathBuf::from("crates/engine"),
                    scope: "data-plane".to_owned(),
                }
            })
        );
        assert_eq!(
            parse(&["scope", "remove", "crates/engine"])
                .unwrap()
                .command,
            Some(Command::Scope {
                command: ScopeCommand::Remove {
                    path: PathBuf::from("crates/engine"),
                }
            })
        );
    }

    #[test]
    fn a_scope_is_taken_as_it_was_typed_and_judged_by_the_engine_rather_than_by_clap() {
        // Both of these are refusals — one is not a scope, the other is the
        // `Empty` rule — and both are warlock's to word and to spend a 1 on.
        // Clap's job is to hand over the string, so that what a reader typed is
        // what the engine's sentence is about.
        for typed in ["Data Plane", ""] {
            assert_eq!(
                parse(&["scope", "add", "crates", typed]).unwrap().command,
                Some(Command::Scope {
                    command: ScopeCommand::Add {
                        path: PathBuf::from("crates"),
                        scope: typed.to_owned(),
                    }
                }),
                "{typed:?}"
            );
        }
    }

    #[test]
    fn a_scope_write_with_a_piece_missing_is_a_malformed_invocation() {
        // A bare `warlock scope` is a noun with nothing done to it, an `add`
        // with one argument is a scope that went missing rather than a clear —
        // clearing is `scope remove` — and a third argument is somebody
        // expecting something warlock does not do. All three are clap's 2.
        let malformed: [&[&str]; 6] = [
            &["scope"],
            &["scope", "add"],
            &["scope", "add", "crates"],
            &["scope", "add", "crates", "web", "extra"],
            &["scope", "remove"],
            &["scope", "remove", "crates", "web"],
        ];

        for args in malformed {
            let error = parse(args).unwrap_err();
            assert!(error.use_stderr(), "{args:?}");
            assert_eq!(error.exit_code(), 2, "{args:?}");
        }
    }

    #[test]
    fn both_spellings_of_help_are_a_help_exit_that_succeeded() {
        // Not an error in the sense that matters: help was asked for, so it
        // goes to stdout and the process exits zero.
        for spelling in ["-h", "--help"] {
            let error = parse(&[spelling]).unwrap_err();
            assert_eq!(error.kind(), ErrorKind::DisplayHelp, "{spelling}");
            assert_eq!(error.exit_code(), 0, "{spelling}");
            assert!(!error.use_stderr(), "{spelling}");
        }
    }

    #[test]
    fn per_subcommand_help_is_a_help_exit_too() {
        for args in [
            ["init", "--help"].as_slice(),
            ["config", "--help"].as_slice(),
            ["stale", "--help"].as_slice(),
            ["fresh", "--help"].as_slice(),
            ["check", "--help"].as_slice(),
            ["unpact", "--help"].as_slice(),
            // The two runs: the help for a command that spends minutes and
            // tokens is the one a reader is likeliest to ask for before typing
            // it for real.
            ["pact", "--help"].as_slice(),
            ["refresh", "--help"].as_slice(),
            // The nested pair, asked for at both depths: `warlock scope --help`
            // is the noun's two verbs, and each verb has a help of its own.
            ["scope", "--help"].as_slice(),
            ["scope", "add", "--help"].as_slice(),
            ["scope", "remove", "--help"].as_slice(),
        ] {
            let error = parse(args).unwrap_err();
            assert_eq!(error.kind(), ErrorKind::DisplayHelp, "{args:?}");
            assert_eq!(error.exit_code(), 0, "{args:?}");
        }
    }

    #[test]
    fn each_subcommands_help_says_what_that_subcommand_does() {
        // The same `about` and `long_about = None` pair as on `Cli`, pinned one
        // subcommand at a time: without the `about` clap falls back to the doc
        // comment, and `warlock init --help` answers "`warlock init`." — the
        // name back, which is not what a reader asked for.
        for (name, said) in [
            ("init", "CLAUDE.md"),
            ("config", "sigils"),
            ("stale", "stale"),
            ("fresh", "fresh"),
            ("check", "scope"),
            ("unpact", "pact"),
            // The two runs say what they leave behind and which directories
            // they spend a pass on, because that is the difference somebody
            // typing one of them is choosing between.
            ("pact", "WARLOCK.md"),
            ("refresh", "stale"),
            ("scope", "scope"),
        ] {
            let mut command = Cli::command();
            let help = command
                .find_subcommand_mut(name)
                .unwrap_or_else(|| panic!("no `{name}` subcommand"))
                .render_long_help()
                .to_string();
            assert!(help.contains(said), "{name}: {help}");
            assert!(!help.contains("essays"), "{name}: {help}");
            assert!(help.lines().count() < 20, "{name}: {help}");
            // The doc comment above each variant is the command in backticks
            // and nothing an `about` here writes is, so a backtick in the help
            // is a doc comment clap lifted — which for `stale` would be the
            // name back rather than what it does.
            assert!(!help.contains('`'), "{name}: {help}");
        }
    }

    #[test]
    fn a_word_warlock_does_not_have_is_refused_rather_than_opening_the_tree() {
        // The whole reason the dispatch exists: `warlock status` used to open
        // the tree, which reads as the typed command having run.
        for word in ["status", "nonsense", ""] {
            let error = parse(&[word]).unwrap_err();
            assert!(error.use_stderr(), "{word}");
            assert_eq!(error.exit_code(), 2, "{word}");
        }
    }

    #[test]
    fn version_is_refused_because_warlock_does_not_have_one_yet() {
        // Deliberate, and recorded on `Cli`: no version is declared, so the
        // flag is an unrecognized argument like any other word warlock does not
        // have rather than a half-truth about which warlock this is.
        for spelling in ["--version", "-V"] {
            let error = parse(&[spelling]).unwrap_err();
            assert_ne!(error.kind(), ErrorKind::DisplayVersion, "{spelling}");
            assert_eq!(error.exit_code(), 2, "{spelling}");
        }
    }

    #[test]
    fn a_trailing_argument_is_refused_and_never_quietly_dropped() {
        // `warlock init extra` typed by somebody who meant something by `extra`
        // must not run an `init` that silently ignored it.
        let refused: [&[&str]; 4] = [
            &["init", "extra"],
            &["init", "init", "init"],
            &["config", "extra"],
            // The one somebody will try: the sigils are typed at `config`'s
            // prompt, where the answer that clears them can be explained before
            // it is given, and never as an argument.
            &["config", "data-plane"],
        ];
        for args in refused {
            let error = parse(args).unwrap_err();
            assert!(error.use_stderr(), "{args:?}");
            assert_eq!(error.exit_code(), 2, "{args:?}");
        }
    }

    #[test]
    fn a_parse_failure_and_a_warlock_failure_do_not_share_an_exit_status() {
        // The split `main` records: clap's refusals are 2, and warlock's own
        // failures are the 1 that `ExitCode::FAILURE` is. Later slices' scope
        // refusals have to be tellable from a typo by the status alone.
        assert_eq!(parse(&["status"]).unwrap_err().exit_code(), 2);
    }

    #[test]
    fn help_prints_a_few_lines_rather_than_this_file() {
        // `long_about = None` is what stands between `warlock --help` and the
        // essays above; without it clap lifts the doc comments wholesale.
        let help = Cli::command().render_long_help().to_string();
        for subcommand in [
            "init", "config", "stale", "fresh", "check", "unpact", "pact", "refresh", "scope",
        ] {
            assert!(help.contains(subcommand), "{subcommand}: {help}");
        }
        assert!(!help.contains("panic hook"), "{help}");
        assert!(help.lines().count() < 20, "{help}");
        // Every doc comment on `Cli` and its variants spells the command in
        // backticks, and no `about` above does, so a backtick reaching the help
        // is a doc comment that got lifted into it.
        assert!(!help.contains('`'), "{help}");
    }

    #[test]
    fn an_answered_question_is_a_zero_whatever_the_answer_was() {
        // The half of the exit contract that carries the verdicts: warlock ran
        // the query and the answer is in the output, so the status says the
        // question was answered and nothing more. The two answers that read
        // most like failures and are not — an empty listing and a scope closed
        // to this machine — are `Ok(())` where they are produced, pinned in
        // `query::tests` and `check::tests` against this same function.
        assert_eq!(status_for(&Ok(())), 0);
    }

    #[test]
    fn a_question_warlock_could_not_answer_is_a_one_and_never_a_two() {
        // The other half: no repository above the working directory, a load
        // that could not colour what it was asked about, and a path with no
        // repository-relative spelling — the last of them built by the very
        // function the three subcommands spell their paths through, so this is
        // the refusal a reader would actually get.
        let refusals = [
            Error::NoRepository {
                start: PathBuf::from("/nowhere"),
                wanted: FOR_CLAUDE_MD,
            },
            Error::Problems {
                first: "`/repo/docs`: `WARLOCK.md` could not be read".to_owned(),
                rest: 2,
            },
            spelled(Path::new("/repo"), Path::new("/elsewhere"))
                .expect_err("a path outside the repository has no manifest form"),
        ];

        for refusal in refusals {
            let said = refusal.to_string();
            assert_eq!(status_for(&Err(refusal)), 1, "{said}");
            // One line, because `main` prints it as one line with a `warlock: `
            // in front of it.
            assert!(!said.contains('\n'), "{said}");
        }
    }

    #[test]
    fn a_boundary_this_machine_does_not_hold_is_a_three_and_nothing_else_is() {
        // The write half of the contract, and the only verdict of warlock's own
        // that is neither a 0 nor a 1: nothing was spent, so it is not a
        // failure, and re-running it will never work, so it is not something to
        // read on stderr and try again. Pinned here beside the other statuses;
        // the three write commands pin their own ends in `edits::tests`.
        let refusal = Error::ClosedScope {
            path: "crates/engine".to_owned(),
            scope: "data-plane".to_owned(),
        };
        assert_eq!(status_for(&Err(refusal)), 3);

        // And it is the refusal's alone. The descendant refusal keeps a 1 by
        // the argument on `status_for`, and so does everything warlock could
        // not do.
        assert_eq!(
            status_for(&Err(Error::ClosedScopeBelow {
                path: ".".to_owned(),
                scopes: vec!["platform".to_owned()],
            })),
            1
        );
        assert_eq!(
            status_for(&Err(Error::NoRepository {
                start: PathBuf::from("/nowhere"),
                wanted: FOR_CLAUDE_MD,
            })),
            1
        );
    }

    #[test]
    fn the_six_statuses_a_write_can_leave_are_all_different_numbers() {
        // The vocabulary, held together in one place so that a script reading
        // only the status can tell them apart: a write that happened, one
        // warlock could not finish, one refused at the boundary with nothing
        // spent, a command line that was never a request, a run that descended a
        // subtree and came back with some of its directories failed, and a run
        // somebody stopped with Ctrl-C. Each is taken from the thing that really
        // produces it — `status_for` for warlock's own five, clap for the sixth
        // — rather than written down as a number.
        let completed = i32::from(status_for(&Ok(())));
        let could_not = i32::from(status_for(&Err(Error::NoRepository {
            start: PathBuf::from("/nowhere"),
            wanted: FOR_CLAUDE_MD,
        })));
        let refused = i32::from(status_for(&Err(Error::ClosedScope {
            path: "crates/engine".to_owned(),
            scope: "data-plane".to_owned(),
        })));
        // The half-worked run: the manifest is saved and the documents that
        // could be written are written, so this is neither the 0 of a run with
        // nothing wrong with it nor the 1 of a warlock that could not do the
        // thing.
        let with_failures = i32::from(status_for(&Err(Error::Failures {
            failed: 3,
            total: 12,
        })));
        // The run somebody stopped: neither warlock's inability nor its verdict
        // on anything, and the one number here a shell already has a meaning
        // for — 128 plus SIGINT.
        let cancelled = i32::from(status_for(&Err(Error::Cancelled)));
        // Clap's, from a write invocation rather than a question's, because it
        // is a write's statuses that are being told apart.
        let malformed = parse(&["scope", "add", "crates"])
            .expect_err("a scope write with the scope missing is clap's")
            .exit_code();

        let vocabulary = [
            completed,
            could_not,
            refused,
            malformed,
            with_failures,
            cancelled,
        ];
        assert_eq!(vocabulary, [0, 1, 3, 2, 4, 130]);
        for (first, one) in vocabulary.iter().enumerate() {
            for (second, other) in vocabulary.iter().enumerate() {
                assert!(
                    first == second || one != other,
                    "two of the outcomes share a status: {vocabulary:?}"
                );
            }
        }
    }

    #[test]
    fn no_argument_the_parser_accepts_gets_a_write_past_the_boundary() {
        // The absence stated over the parser itself rather than over a list of
        // spellings somebody thought of: `--force` is refused in the test above,
        // and this says there is no word at all — however spelled — that a write
        // takes besides its path, its scope and clap's own `--help`. The one
        // road past a boundary is `warlock config`, and an option here would be
        // a second one.
        for names in [
            vec!["unpact"],
            vec!["scope"],
            vec!["scope", "add"],
            vec!["scope", "remove"],
        ] {
            let mut command = subcommand(&names);
            // The positionals are the path, and the scope on an `add`; every
            // other argument a write accepts has to be clap's own help.
            for argument in command.get_arguments().filter(|a| !a.is_positional()) {
                assert_eq!(
                    argument.get_long(),
                    Some("help"),
                    "{names:?} takes an option other than clap's help"
                );
            }

            // And the help a reader is shown offers none of the words an
            // override would be spelled with, in case one arrives later as a
            // subcommand rather than as a flag.
            let help = command.render_long_help().to_string().to_lowercase();
            for word in ["force", "override", "skip", "anyway", "ignore", "sudo"] {
                assert!(!help.contains(word), "{names:?}: {help}");
            }
        }
    }

    #[test]
    fn a_malformed_invocation_is_clap_s_two_across_all_three_questions() {
        // The third status, and the one warlock never produces itself: clap
        // exits the process with it before `main` has anything to map. Held
        // here over each of the three so that a script can tell a typo from a
        // refusal by the status alone, without reading a word of either.
        let malformed: [&[&str]; 6] = [
            &["stale", "--nonsense"],
            &["stale", "here", "there"],
            &["fresh", "--json=yes"],
            &["check"],
            &["check", "here", "there"],
            &["check", "--nonsense", "here"],
        ];

        for args in malformed {
            let error = parse(args).unwrap_err();
            assert_eq!(error.exit_code(), 2, "{args:?}");
            assert!(error.use_stderr(), "{args:?}");
            // And distinct from the status warlock spends on its own failures,
            // which is the whole reason the split is worth keeping.
            assert_ne!(
                error.exit_code(),
                i32::from(status_for(&Ok(()))),
                "{args:?}"
            );
            assert_ne!(
                error.exit_code(),
                i32::from(status_for(&Err(Error::NoRepository {
                    start: PathBuf::from("/nowhere"),
                    wanted: FOR_CLAUDE_MD,
                }))),
                "{args:?}"
            );
        }
    }

    /// A screen that draws into memory and is never given away.
    ///
    /// The second adapter at the [`Screen`] seam, and the whole reason a
    /// [`Session`] is reachable from a test. Warlock's own is a terminal in raw
    /// mode on the alternate screen, which a test runner has not got; this one
    /// is ratatui's `TestBackend`, which every `ui.rs` test already draws
    /// against, plus a record of the two things a session asks of a terminal
    /// that are not drawing.
    #[derive(Debug)]
    struct FakeScreen {
        /// Where frames go.
        terminal: Terminal<TestBackend>,
        /// The `mouse` of every suspension asked for, in the order it was asked.
        suspensions: Vec<bool>,
        /// Every mouse-reporting change asked for, in the order it was asked.
        reported: Vec<bool>,
    }

    impl FakeScreen {
        /// A screen `width` by `height`, with nothing asked of it yet.
        fn of(width: u16, height: u16) -> Self {
            Self {
                terminal: Terminal::new(TestBackend::new(width, height))
                    .expect("a test backend never fails to start"),
                suspensions: Vec::new(),
                reported: Vec::new(),
            }
        }
    }

    impl Screen for FakeScreen {
        fn size(&self) -> io::Result<Size> {
            // A `TestBackend` cannot fail, so its error type is `Infallible`
            // and there is nothing here for warlock to handle.
            Ok(self.terminal.size().expect("a test backend never fails"))
        }

        fn draw<F: FnOnce(&mut Frame<'_>)>(&mut self, render: F) -> io::Result<()> {
            self.terminal
                .draw(render)
                .expect("a test backend never fails");
            Ok(())
        }

        fn suspended<T, F: FnOnce() -> T>(&mut self, mouse: bool, body: F) -> io::Result<T> {
            self.suspensions.push(mouse);
            Ok(body())
        }

        fn report_mouse(&mut self, on: bool) -> io::Result<()> {
            self.reported.push(on);
            Ok(())
        }
    }

    /// The session a test drives: an in-memory screen, and both models answering
    /// out of memory.
    type Driven = Session<FakeScreen, Passing, Saying>;

    /// A session over `rows`, with nothing pacted, nothing in flight and no
    /// child process anywhere.
    ///
    /// The very value [`run`] builds, with its three seams filled by stand-ins
    /// instead of a terminal and two `claude`s — which is the point: what these
    /// tests press is the real key handler over the real session, rather than a
    /// retyped copy of either.
    fn session(rows: Vec<Row>) -> Driven {
        let root = PathBuf::from("/warlock/no/such/repository");
        let scope = Scope {
            chrome: Chrome::of(&root, &root),
            root: root.clone(),
            repo_root: root.clone(),
        };
        // A one-node tree for the watcher to be started over. Nothing is there,
        // so no watcher is granted and `Watching` says why — which is exactly
        // the state a session runs in when the platform refuses one, and costs
        // these tests nothing.
        let tree = Tree::new(Node::new(&root, None::<PathBuf>, NodeState::Unpacted));
        let watched = Watched::start(&scope, &tree);
        Session {
            app: App::from_rows(rows),
            screen: FakeScreen::of(80, 24),
            scope,
            manifest: Manifest::new(),
            pact: Pact::with_agent(Passing::answering(DOCUMENT)),
            chat: Chat::with_agent(root, Saying::answering(ANSWER)),
            confirm: QuitConfirm::default(),
            prompt: ScopePrompt::default(),
            document: None,
            mouse_captured: true,
            watched,
        }
    }

    /// Press `key` on `driven`, and say whether the session goes on.
    ///
    /// The instant is read here because none of the tests below are about a
    /// clock: what a key *does* is the same at every instant, and the two things
    /// that are timed — a run's account and a turn's — are driven from their own
    /// modules against instants those tests hand in.
    fn pressed(driven: &mut Driven, key: KeyEvent) -> bool {
        driven
            .press(key, Instant::now())
            .expect("no key pressed here writes to a terminal")
    }

    /// What the stand-in model answers a pass with: long enough that the engine
    /// keeps it rather than dropping it under `MINIMUM_DOCUMENT_BYTES`.
    const DOCUMENT: &str = "# crates\n\nWhat this directory is for, said at about the length a \
                            real document says it at, so that the engine keeps what comes back \
                            instead of refusing it as too short to be a document. The floor is \
                            `MINIMUM_DOCUMENT_BYTES`, which is two hundred bytes, and a stand-in \
                            that answers with less than that is a stand-in whose passes all \
                            quietly fail.";

    /// What the stand-in model answers a turn with.
    const ANSWER: &str = "The tree, the manifest and the pact.";

    /// One ordinary keystroke, with no modifier held.
    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    /// One directory row, which is what every refusal below is aimed at.
    fn directory(path: &str) -> Row {
        Row::new(0, path, None, NodeState::Unpacted)
    }

    /// How long a round-driving test waits for a run that is never going to
    /// end. Only ever reached when something is already wrong, and every wait
    /// ends the moment the run does.
    const AT_MOST: Duration = Duration::from_secs(5);

    /// A repository on disk: one crate of two directories, and the `.git/` that
    /// makes the loader agree it is a repository.
    ///
    /// The file inside `.git/` is neither read nor walked — hidden directories
    /// are skipped — and is written only because a directory is made here by
    /// writing a file into it.
    fn a_repository() -> tempfile::TempDir {
        let scratch = tempfile::tempdir().expect("a temporary directory");
        for (path, text) in [
            (".git/HEAD", "ref: refs/heads/main\n"),
            ("crates/engine/src/lib.rs", "//! Core engine.\n"),
        ] {
            let at = scratch.path().join(path);
            fs::create_dir_all(at.parent().expect("every path here has a parent"))
                .expect("a scratch directory is writable");
            fs::write(&at, text).expect("a scratch file is writable");
        }
        scratch
    }

    /// A session over a repository that is really on disk, built the way
    /// [`run`] builds one — and, like it, with the tree read first.
    fn session_over(root: &Path) -> Driven {
        let Loaded { tree, .. } = load_tree(root).expect("a scratch repository loads");
        let repo_root = repository_root(tree.root_path()).expect("the load found a repository");
        let scope = Scope {
            chrome: Chrome::of(&repo_root, tree.root_path()),
            root: tree.root_path().to_path_buf(),
            repo_root: repo_root.clone(),
        };
        let watched = Watched::start(&scope, &tree);
        Session {
            app: App::from_tree(&tree),
            screen: FakeScreen::of(80, 24),
            scope,
            manifest: Manifest::new(),
            pact: Pact::with_agent(Passing::answering(DOCUMENT)),
            chat: Chat::with_agent(repo_root, Saying::answering(ANSWER)),
            confirm: QuitConfirm::default(),
            prompt: ScopePrompt::default(),
            document: None,
            mouse_captured: true,
            watched,
        }
    }

    /// Round after round, until nothing is in flight any more.
    fn rounds_until_settled(driven: &mut Driven) {
        let waited = Instant::now();
        while driven.pact.running() && waited.elapsed() < AT_MOST {
            let size = driven.size().expect("the fake screen has a size");
            driven.draw(size).expect("the fake screen draws");
            driven.keep_up();
        }
        assert!(!driven.pact.running(), "the run never finished");
    }

    #[test]
    fn a_round_tells_the_app_the_size_the_frame_is_being_cut_at() {
        let mut driven = session(vec![directory("/repo/crates")]);
        let size = driven.size().expect("the fake screen has a size");

        driven.draw(size).expect("the fake screen draws");

        assert_eq!(
            driven.app.viewport_height(),
            usize::from(tree_height(size)),
            "the app was told the height this frame gives the tree"
        );
    }

    #[test]
    fn pressing_the_pact_key_descends_the_subtree_and_lands_its_documents() {
        let repo = a_repository();
        let mut driven = session_over(repo.path());

        assert!(pressed(&mut driven, key(KeyCode::Char('p'))));
        assert!(driven.pact.running(), "the press started a run");

        rounds_until_settled(&mut driven);

        assert!(
            repo.path().join("WARLOCK.md").is_file(),
            "the root was never documented"
        );
        assert!(
            repo.path().join("crates/engine/src/WARLOCK.md").is_file(),
            "the descent stopped short of the deepest directory"
        );
        assert_eq!(
            driven.manifest.entries().len(),
            4,
            "every directory the walk produced should have been granted"
        );
    }

    #[test]
    fn a_second_press_of_the_pact_key_takes_the_whole_subtree_back_out() {
        let repo = a_repository();
        let mut driven = session_over(repo.path());
        pressed(&mut driven, key(KeyCode::Char('p')));
        rounds_until_settled(&mut driven);

        pressed(&mut driven, key(KeyCode::Char('p')));
        rounds_until_settled(&mut driven);

        assert_eq!(
            driven.manifest.entries().len(),
            0,
            "un-pacting left entries behind"
        );
        assert!(
            repo.path().join("WARLOCK.md").is_file(),
            "un-pacting deleted a document, which it has never done"
        );
    }

    #[test]
    fn the_quit_key_opens_the_question_rather_than_leaving() {
        let mut driven = session(vec![directory("/repo/crates")]);

        assert!(
            pressed(&mut driven, key(KeyCode::Char('q'))),
            "the session goes on"
        );
        assert_eq!(
            driven.confirm,
            QuitConfirm::open(),
            "and the question is up with No lit"
        );
    }

    #[test]
    fn ctrl_c_leaves_without_asking() {
        let mut driven = session(vec![directory("/repo/crates")]);

        let key = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert!(!pressed(&mut driven, key), "the session is over");
        assert_eq!(
            driven.confirm,
            QuitConfirm::Closed,
            "and no question was ever asked"
        );
    }

    #[test]
    fn answering_no_puts_the_question_down_and_stays() {
        let mut driven = session(vec![directory("/repo/crates")]);
        pressed(&mut driven, key(KeyCode::Char('q')));

        assert!(
            pressed(&mut driven, key(KeyCode::Enter)),
            "the session goes on"
        );
        assert_eq!(
            driven.confirm,
            QuitConfirm::Closed,
            "and the question is down"
        );
    }

    #[test]
    fn the_focus_key_moves_the_keyboard_on() {
        let mut driven = session(vec![directory("/repo/crates")]);
        let before = driven.app.focus();

        assert!(pressed(&mut driven, key(KeyCode::Tab)));
        assert_ne!(driven.app.focus(), before, "the focus moved");
    }

    #[test]
    fn the_mouse_key_flips_what_the_loop_is_holding() {
        let mut driven = session(vec![directory("/repo/crates")]);

        assert!(pressed(&mut driven, key(KeyCode::Char('m'))));
        assert!(!driven.mouse_captured, "reporting was turned off");

        assert!(pressed(&mut driven, key(KeyCode::Char('m'))));
        assert!(driven.mouse_captured, "and back on again");

        assert_eq!(
            driven.screen.reported,
            [false, true],
            "and the terminal was told each time, through the screen rather \
             than past it"
        );
    }

    #[test]
    fn an_edit_over_a_directory_never_asks_for_the_screen() {
        let mut driven = session(vec![directory("/repo/crates")]);

        assert!(pressed(&mut driven, key(KeyCode::Char('e'))));
        assert!(
            driven.screen.suspensions.is_empty(),
            "a row that is not a file is refused before any child is run"
        );
        assert!(
            driven.app.message().is_some(),
            "and the refusal is said rather than swallowed"
        );
    }

    #[test]
    fn the_scope_prompt_swallows_the_pact_key() {
        let mut driven = session(vec![directory("/repo/crates")]);
        driven.prompt = ScopePrompt::open("crates", "");

        assert!(pressed(&mut driven, key(KeyCode::Char('p'))));
        let field = driven
            .prompt
            .field()
            .expect("the window is still up over the directory it opened on");
        assert_eq!(field.text(), "p", "the key was typed, not pressed");
    }

    #[test]
    fn a_key_the_window_does_not_want_puts_it_down_and_starts_nothing() {
        let mut driven = session(vec![directory("/repo/crates")]);
        driven.prompt = ScopePrompt::open("crates", "");

        assert!(pressed(&mut driven, key(KeyCode::Esc)));
        assert_eq!(
            driven.prompt,
            ScopePrompt::Closed,
            "Esc closes the window rather than quitting warlock"
        );
    }
}
