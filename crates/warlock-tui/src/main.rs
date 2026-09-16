//! The terminal front end: the impure shell around the pure parts in
//! `warlock_tui`. It owns the terminal's lifecycle, the directory warlock was
//! invoked from, and the event loop, and nothing else.
//!
//! Every subcommand is dispatched *before* anything touches the terminal, and
//! none of them installs the panic hook: they print on the ordinary screen for
//! a script reading through a pipe, and `Cli::parse` exits the process itself
//! on `--help`, which is only safe while there is nothing attached to the
//! terminal to leave un-restored. The terminal is then restored on every way
//! out, including a panic on any thread, which is why [`install_panic_hook`]
//! runs before [`TerminalGuard::enter`] and why the guard lives inside [`run`].
//!
//! The loop draws and then *polls* for [`POLL_INTERVAL`] rather than blocking
//! on a key, because the long keystrokes run on worker threads and report over
//! channels that only the bottom of the loop drains. Doc comments on the
//! `#[arg]` fields below are clap's `--help` text, not prose: deleting one
//! changes what `warlock --help` prints.

use std::io;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use clap::{Parser, Subcommand};
use ratatui::crossterm::event::{self, Event, KeyEvent, MouseEvent};
use ratatui::layout::Size;
use warlock_engine::{Agent, Manifest, Written, write_claude_md};
use warlock_tui::{
    App, Cell, Converses, Focus, Position, QuitConfirm, Reach, Run, ScopePrompt, Wired,
    composer_on_screen, copied_text, draw, panel_height, panel_width, paste_for, position_at,
    tree_height,
};

mod boundary;
mod chatting;
mod check;
mod clipboard;
mod config;
mod descent;
mod editing;
mod edits;
mod error;
mod input;
mod pacting;
mod query;
mod running;
mod scoping;
mod session;
mod standing;
#[cfg(test)]
mod stubs;
mod terminal;
mod viewing;
mod writing;

use chatting::Chat;
use check::check;
use clipboard::{Clip, Clipboard};
use config::configure;
use editing::edit_press;
use edits::{scope_add, scope_remove, unpact};
use error::Error;
use input::{Action, Drag, MouseAction, Pressed, drag_after, mouse_action, press_for};
use pacting::{Pact, Reloaded};
use query::{Listing, list};
use running::{pact, refresh};
use scoping::{scope_edit, scope_press};
use session::{Scope, Watched, load_app, start_watching};
use standing::{FOR_CLAUDE_MD, Standing};
use terminal::{Screen, TerminalGuard, install_panic_hook};
use viewing::view_press;

/// How long the loop waits for a keystroke before going round anyway.
///
/// The ceiling on how stale the footer's clock and a run's progress line can
/// get, and the reason an idle warlock redraws ten times a second rather than
/// sleeping: the worker threads have no way to wake this one.
const POLL_INTERVAL: Duration = Duration::from_millis(100);

/// How long the footer keeps what it was told.
///
/// The footer says what the last thing that happened was, and nothing on it says
/// when: `copied 104 characters` an hour after the copy reads as a copy that
/// just happened. So a message is dropped once it is older than this, and the
/// footer says nothing rather than something out of its time. The highlight a
/// copy leaves on the conversation is what stays.
const MESSAGE_LIFETIME: Duration = Duration::from_secs(10);

/// The two words [`init`] reports with. Everything the engine can return that
/// is not a brand-new file is an update, which is why this is a `matches!` on
/// one variant rather than a match over an `#[non_exhaustive]` enum.
const CREATED: &str = "created";

const UPDATED: &str = "updated";

/// `about` is spelled out on every command below, with `long_about = None`, so
/// that the doc comments in this file are free to say why rather than being
/// lifted into `--help`. The `#[arg]` fields are the exception and keep theirs.
#[derive(Debug, Clone, PartialEq, Eq, Parser)]
#[command(
    name = "warlock",
    about = "A freshness ledger for a repository's documentation.",
    long_about = None
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

/// Everything warlock does without opening the tree.
///
/// `Stale` and `Fresh` are one shape twice over rather than one variant
/// carrying a state: what a reader types is the whole difference between them,
/// and a `List { state }` would be a variant nobody finds by grepping for the
/// word they typed. What they *do* is one function taking a [`Listing`].
#[derive(Debug, Clone, PartialEq, Eq, Subcommand)]
enum Command {
    #[command(
        about = "Write warlock's section of CLAUDE.md at the repository root.",
        long_about = None
    )]
    Init,
    #[command(
        about = "Set the sigils this machine holds for this repository.",
        long_about = None
    )]
    Config,
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
    #[command(
        about = "Describe the stale directories at or below one, leaving the fresh ones alone.",
        long_about = None
    )]
    Refresh {
        /// Which directory to refresh, with everything below it.
        #[arg(value_name = "PATH")]
        path: PathBuf,
    },
    #[command(
        about = "Set or clear the scope on a pacted directory.",
        long_about = None
    )]
    Scope {
        #[command(subcommand)]
        command: ScopeCommand,
    },
}

/// Add and remove, and no third. No `list`, because that is `warlock check`;
/// no `set` beside `add`, because one write with one name is what keeps the
/// shell and the `s` key describable as one rule.
///
/// Clearing is [`ScopeCommand::Remove`] only. The TUI's field clears by being
/// empty, which is right for a window somebody is typing in; at a shell an
/// empty string is far more often an argument that went missing.
#[derive(Debug, Clone, PartialEq, Eq, Subcommand)]
enum ScopeCommand {
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

/// The process's exit status.
///
/// Four non-zero registers, kept distinct so a script can tell them apart
/// without reading a word of the message: **2** is clap's, for a command line
/// it could not parse; **1** is warlock could not do it; **3** is a boundary
/// this machine's sigils do not open, refused with nothing spent; **4** is a
/// run that finished with some directories failed, which is the one non-zero
/// status that comes with the work having been done and saved; and
/// [`CANCELLED`] is a run somebody stopped, likewise saved.
///
/// **0** means the question was answered whatever the answer was — an empty
/// listing is "nothing is stale", and a closed scope is `check`'s answer rather
/// than a failure to reach one.
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

/// The status a shell already spells an interrupted process with, so warlock
/// does not invent a second one.
const CANCELLED: u8 = 130;

/// `warlock init`: write warlock's section of `CLAUDE.md` at the repository
/// root and say which file changed. Touches no terminal and spends nothing.
fn init() -> Result<(), Error> {
    let standing = Standing::here(FOR_CLAUDE_MD)?;

    let written =
        write_claude_md(standing.repo_root()).map_err(|source| Error::ClaudeMd { source })?;
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

/// The interactive session: load, take the terminal, then loop.
///
/// The order of the first half is the point. The manifest, the tree and the
/// watch are all set up before [`TerminalGuard::enter`], so anything that can
/// fail says so on the ordinary screen; from the guard onwards the alternate
/// screen is up and the only way back is dropping it.
///
/// Returning is the whole of quitting: the session drops on this stack, which
/// cancels any run and kills the `claude` it was waiting on, and the guard
/// drops after it. Nothing joins the worker.
fn run() -> Result<(), Error> {
    let (mut app, scope, tree, manifest) = load_app()?;
    // Asked for once, over the tree the load just produced, and kept for as
    // long as warlock runs — dropping it stops the watch. Whether it was
    // granted is a fact for the footer and nothing more, which is why this is
    // not a `?` and why the line about it is put up in there rather than here:
    // warlock with no live updates is warlock as it was. See [`start_watching`].
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
        // Opened here and nowhere else, and dropped when this function returns:
        // a handle that does not outlive the copies made through it loses the
        // text it put on an X11 selection. See `mod@clipboard`.
        clipboard: Clipboard::open(),
        confirm: QuitConfirm::default(),
        prompt: ScopePrompt::default(),
        drag: None,
        document: None,
        said: None,
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
                Event::Mouse(mouse) => session.point(mouse, size, now),
                // A block of text the terminal handed over whole, because
                // bracketed paste is on (see `take_terminal`). Nothing is
                // returned and nothing can be: a paste never ends the session
                // and never starts a turn, whatever newlines are in it, so
                // there is no answer for this arm to act on. On a terminal
                // without bracketed paste this never arrives and the same
                // bytes come through the arm above, one key at a time.
                Event::Paste(text) => session.paste(&text),
                _ => {}
            }
        }

        // The one thing that happens on the round rather than on an event, and
        // so outside the poll above: a pointer held past the conversation's edge
        // sends nothing at all while it sits there, so the scrolling it asks for
        // has to come round with the loop or not at all.
        session.drag_scroll();

        // And then everything that happened off this thread: what a run has
        // said since the last round, what a turn has, and what the disk did
        // while this one was waiting on a keystroke.
        session.keep_up();
    }
}

/// Everything one interactive session holds, and the seam the tests drive.
///
/// Generic over all four impure things — the screen, the model, the
/// conversation's model, the clipboard — so a test can press keys at a whole
/// session with no terminal attached, no `claude` installed and no display.
/// `warlock` itself only ever instantiates it one way, in [`run`].
struct Session<S: Screen, P: Wired + Agent, C: Converses, B: Clip> {
    app: App,
    screen: S,
    scope: Scope,
    manifest: Manifest,
    pact: Pact<P>,
    chat: Chat<C>,
    /// The session's one clipboard handle, opened in [`run`] and held until it
    /// returns because a copy does not outlive the handle that made it.
    clipboard: B,
    confirm: QuitConfirm,
    prompt: ScopePrompt,
    /// The left button held down over the conversation, if it is: the one piece
    /// of a gesture that outlives the event carrying it, because the rounds
    /// between one drag event and the next are what [`Session::drag_scroll`]
    /// runs on. `None` is no button held, or one held after a press that landed
    /// anywhere else.
    drag: Option<Drag>,
    /// Which file is on the document card, which the app is never told:
    /// `App::show_document` takes lines and never a path. The edit key is what
    /// asks, so that it re-reads the card only when the file it just handed to
    /// an editor is the one on it. A press that read nothing leaves this as it
    /// was rather than clearing it.
    document: Option<PathBuf>,
    /// Which saying is on the footer and the instant it landed, for
    /// [`Session::forget_stale_message`]. Held here rather than on the app
    /// because the app is told every instant it works with and keeps no clock of
    /// its own, and a message's age is the one thing about it that moves without
    /// anybody touching it.
    said: Option<(u64, Instant)>,
    mouse_captured: bool,
    watched: Watched,
}

impl<S: Screen, P: Wired + Agent, C: Converses, B: Clip> Session<S, P, C, B> {
    fn size(&self) -> io::Result<Size> {
        self.screen.size()
    }

    /// Everything that has to be told the frame's dimensions before anything
    /// reads them. The loop draws and *then* waits for a key, which is what
    /// lets the composer's row-wise keys and the panel's wrapping both work off
    /// a width nothing has measured yet at startup.
    fn draw(&mut self, size: Size) -> io::Result<()> {
        self.app.set_mouse_captured(self.mouse_captured);
        // One reading of the panel's inside width, told to both the things that
        // are about to use it: the app wraps the document it holds at it, and
        // the composer is drawn in the same column, so it is the width the
        // field's own rows are folded at and therefore the width its row-wise
        // keys move over. Told here, before the key that reads it, because the
        // loop draws and then waits.
        let width = panel_width(size);
        self.chat.set_composer_width(width);
        let field = composer_on_screen(&self.app, self.chat.composer());
        let header = self.app.run_header();
        self.app.set_viewport_height(tree_height(size));
        let panel = self.app.panel_mut();
        panel.set_height(panel_height(size, field, header.as_ref()));
        panel.set_width(width);

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

    /// `now` is the instant the event arrived, for the reason a keystroke's is
    /// read once by the caller: a clock is part of a row's prefix, so the cell
    /// under the pointer is worked out against the same instant the rest of this
    /// round is.
    ///
    /// The copy is done here rather than in [`apply_mouse`] because the
    /// clipboard is the session's, held open for as long as it runs — see
    /// [`Clipboard`].
    fn point(&mut self, mouse: MouseEvent, size: Size, now: Instant) {
        // Read here rather than inside `apply_mouse` because every window and
        // the draft the frame was drawn with are the session's: what the event
        // means is decided against them, and what it then does is the app's
        // alone.
        let field = composer_on_screen(&self.app, self.chat.composer());
        let action = mouse_action(
            mouse,
            size,
            &self.app,
            self.confirm,
            &self.prompt,
            self.chat.write_prompt(),
            field,
        );
        // Before the action is acted on, because this is bookkeeping about the
        // gesture rather than part of it: what the app does with a press or a
        // drag is the same whether or not the button stays down afterwards.
        self.drag = drag_after(self.drag, mouse.kind, action);
        // Derived from the drag rather than kept beside it: `drag_after` already
        // says exactly "the left button is still down after a press on a line of
        // the conversation", which is the whole of what suspends following. Said
        // on every pointer event, so the release that ends the drag — wherever
        // the pointer was when the button came up — is the one that lets the card
        // go back to the newest line, and a press that landed anywhere else never
        // holds it at all.
        self.app.panel_mut().hold_thread(self.drag.is_some());
        if let Some(text) = apply_mouse(&mut self.app, action, now) {
            self.copy(&text);
        }
    }

    /// The conversation scrolled under a drag held past its edge, once per
    /// round, with the highlight taken along.
    ///
    /// Called off the loop's tick rather than off an event because a pointer
    /// held still past the card sends nothing: the last drag event said which
    /// edge it went past and by how far, and every round after it reads that
    /// again. Does nothing at all on a round with no button held, one held after
    /// a press that landed elsewhere, or a pointer still level with the rows.
    fn drag_scroll(&mut self) {
        let Some(past) = self.drag.and_then(|drag| drag.past) else {
            return;
        };
        // A key pressed with the button still down can put another card in the
        // panel, and the drag outlives it: without this the held pointer would
        // go on scrolling, now through the account or the document, neither of
        // which a drag has ever been able to move.
        if !self.app.panel().showing_thread() {
            return;
        }
        let (rows, column, downwards) = match past {
            Reach::Above { rows, column } => (rows, column, false),
            Reach::Below { rows, column } => (rows, column, true),
            // Never recorded: a pointer level with the rows has a cell under it,
            // and `mouse_action` hands that over as the cell rather than as a
            // reach past anything.
            Reach::Inside { .. } => return,
        };

        let before = self.app.panel().scroll_offset();
        let by = rows_per_tick(rows);
        if downwards {
            self.app.scroll_panel_down(by);
        } else {
            self.app.scroll_panel_up(by);
        }

        let (scroll, height, width) = {
            let panel = self.app.panel();
            (panel.scroll_offset(), panel.height(), panel.width())
        };
        if scroll == before {
            // Either end of the thread, where the panel clamped and nothing new
            // came into view. The highlight stops growing with it: there is no
            // newly revealed row for the pointer to be pulling towards, and
            // extending anyway would drag the far end across the same edge row
            // once a tick for as long as the button is held.
            return;
        }

        // The row the scroll just uncovered — the bottom one going down, the top
        // one going up — at the column the pointer left the card by. Measured
        // from the panel's own scroll and width, as a cell under the pointer is,
        // so the endpoint maps back through the wrapping this frame was drawn
        // with.
        let cell = Cell {
            column: usize::from(column),
            row: if downwards {
                height.saturating_sub(1)
            } else {
                0
            },
            scroll,
            width,
        };
        extend(&mut self.app, cell, Instant::now());
    }

    /// One keystroke. `Ok(false)` is the session being over and the only thing
    /// that ends [`run`]'s loop.
    ///
    /// `now` is the instant the event arrived, read once by the caller: a turn
    /// and a pass are clocked from the keystroke that asked for them rather
    /// than from the first thing the model got round to saying.
    ///
    /// Every arm below is one key. Nothing here re-gates what the module it
    /// dispatches to already refuses, which is why most arms have no error case
    /// — a refusal is a line on the footer and the loop goes round again.
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
                    &mut self.manifest,
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
                self.chat.write(&mut self.app, edited, now);
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

    /// A block the terminal handed over whole. Nothing is returned and nothing
    /// can be: a paste never ends the session and never starts a turn, whatever
    /// newlines are in it.
    ///
    /// The two gates are here rather than in [`paste_for`], because a paste is
    /// a single arrival carrying however much was copied and a gate missed at a
    /// call site would land the lot.
    fn paste(&mut self, text: &str) {
        let Some(typing) = (self.app.focus() == Focus::Composer).then(|| self.chat.composer())
        else {
            return;
        };
        if typing.is_muted() {
            return;
        }

        let pasted = paste_for(text, typing);
        self.chat.paste(pasted);
    }

    /// Text onto the system clipboard, and a line on the footer either way.
    ///
    /// Reached by the release that ends a drag over the conversation, and by
    /// nothing else: what decides the text is the selection that drag built.
    fn copy(&mut self, text: &str) {
        clipboard::copy(&mut self.clipboard, &mut self.app, text);
    }

    /// Everything that happened off this thread since the last round: what a
    /// run has said, what a turn has, and what the disk did while this thread
    /// was waiting on a keystroke.
    /// Takes `now` rather than reading the clock, so a test can hand it an
    /// instant ten seconds on instead of sitting there for ten seconds.
    ///
    /// The count and not the words: the same sentence said again is a new
    /// saying, and comparing the text would leave a second copy of the same
    /// length wearing out what was left of the first one's ten seconds.
    fn forget_stale_message(&mut self, now: Instant) {
        if self.app.message().is_none() {
            self.said = None;
            return;
        }
        let said = self.app.said();
        match self.said {
            Some((seen, at)) if seen == said => {
                if now.duration_since(at) >= MESSAGE_LIFETIME {
                    self.app.forget_message();
                    self.said = None;
                }
            }
            _ => self.said = Some((said, now)),
        }
    }

    fn keep_up(&mut self) {
        let now = Instant::now();
        self.forget_stale_message(now);
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
        self.watched.round(
            &mut self.app,
            &self.scope,
            &mut self.manifest,
            self.pact.running(),
            now,
        );
        // And the conversation's own bottom end. Nothing comes back: a `/write`
        // turn's answer opens the window that goes over it, and that window is the
        // conversation's, so it is opened in there rather than here out of two
        // values this loop would otherwise have to be handed. See [`Chat::keep_up`].
        self.chat.keep_up(&mut self.app, now);
    }
}

/// What a pointer event already read off the frame comes to on the app.
///
/// Free rather than a method because none of it reads the terminal, spawns
/// anything or draws: the round is the redraw, which is why a pointer swept
/// across the screen costs nothing.
///
/// `now` is the instant the event arrived, because a clock is part of a row's
/// prefix and a row with a wider prefix breaks somewhere else — the cell under
/// the pointer has to be worked out against the same instant everything else
/// this round is.
///
/// What comes back is text a release asked for, which the caller puts on the
/// clipboard it holds: the one thing a pointer event can want that is not on
/// the app.
fn apply_mouse(app: &mut App, action: Option<MouseAction>, now: Instant) -> Option<String> {
    match action {
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
        // A press on the conversation is the focus arm above plus the anchor a
        // drag extends from. Whatever was highlighted comes down first and
        // comes down either way, cell with text under it or not: the reader has
        // just pointed somewhere else, and a highlight left standing is one the
        // next release would copy.
        Some(MouseAction::StartSelection(cell)) => {
            app.set_focus(Focus::Panel);
            app.clear_selection();
            if let Some(at) = position_under(app, cell, now) {
                app.start_selection(at);
            }
        }
        // A drag moves the far end and nothing else. A cell with no text under
        // it leaves the selection where the last one put it rather than
        // collapsing it: a pointer dragged off the end of the card is still
        // that drag.
        Some(MouseAction::ExtendSelection(cell)) => extend(app, cell, now),
        // The release ends the drag where the button came up, and what it hands
        // back is what the selection covers — nothing at all for a press nobody
        // dragged from, since the two ends are one position and `copied_text`
        // of that is empty. An empty copy is not a copy: the clipboard keeps
        // what it had and the footer is left alone, so nothing tells a reader
        // something went that did not.
        //
        // The highlight stays up. It is the only thing on screen saying what
        // went, and taking it down in the same frame as the footer says how much
        // would leave the reader to take warlock's word for it.
        Some(MouseAction::EndSelection(cell)) => {
            extend(app, cell, now);
            let text = selected_text(app);
            return (!text.is_empty()).then_some(text);
        }
        // The release that ends a drag held past the card's edge. The far end is
        // left where the last cell under the pointer put it — there is no cell
        // out here to move it to — and what the highlight covers is handed back
        // exactly as the arm above hands it back, so a button let go past the
        // edge still copies rather than dropping the gesture on the floor.
        Some(MouseAction::EndPastEdge(_)) => {
            let text = selected_text(app);
            return (!text.is_empty()).then_some(text);
        }
        // A drag held past that edge does nothing to the app *here*, which is
        // why it sits with the events that mean nothing at all: what it asks for
        // is a scroll a round at a time, and a round is not an event. The reach
        // it carries is kept on the session by [`Session::point`] and read by
        // [`Session::drag_scroll`] for as long as the button is down.
        Some(MouseAction::ExtendPastEdge(_)) | None => {}
    }

    None
}

/// How many rows one tick scrolls the card by, for a pointer `past` rows past
/// its edge.
///
/// The tick is [`POLL_INTERVAL`], ten a second, so a row per tick is ten rows a
/// second: slow enough to stop on the line you meant, which is what the first
/// row past the edge has to be. The ceiling is five, or fifty rows a second —
/// about a screenful a half-second, fast enough to cross a long answer and slow
/// enough to see what is going past. Between them a row of speed per three rows
/// of reach, so the whole range is available inside a few rows of pointer travel
/// rather than needing the reader to drag off the bottom of the terminal.
fn rows_per_tick(past: u16) -> usize {
    const FASTEST: usize = 5;
    const ROWS_PER_STEP: usize = 3;

    usize::from(past).div_ceil(ROWS_PER_STEP).clamp(1, FASTEST)
}

fn extend(app: &mut App, cell: Cell, now: Instant) {
    if let Some(to) = position_under(app, cell, now) {
        app.extend_selection(to);
    }
}

/// Where in the conversation's own text a cell of the card is, which is `None`
/// for a cell past its last row — a place a reader can put the pointer and not
/// a position in anything.
fn position_under(app: &App, cell: Cell, now: Instant) -> Option<Position> {
    position_at(app.panel().thread()?, cell, now)
}

/// What the highlight covers, as the thread's own stored text: no marker, no
/// indent and no break where the panel wrapped it (see [`copied_text`]).
fn selected_text(app: &App) -> String {
    let (Some(selection), Some(thread)) = (app.selection(), app.panel().thread()) else {
        return String::new();
    };

    copied_text(&thread.pieces(), selection)
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
    use warlock_tui::{App, Chrome, Focus, QuitConfirm, Row, ScopePrompt, tree_height};

    use super::{Cli, Command, Error, FOR_CLAUDE_MD, ScopeCommand, Session, status_for};
    use crate::chatting::Chat;
    use crate::pacting::Pact;
    use crate::query::spelled;
    use crate::session::{Scope, Watched};
    use crate::stubs::{Copying, Passing, Saying};
    use crate::terminal::Screen;

    // `try_parse_from` wants argv as the process gets it, program name and all,
    // so the name is put back here and every test below reads as the words a
    // person types.
    fn parse(args: &[&str]) -> Result<Cli, clap::Error> {
        let typed = std::iter::once("warlock").chain(args.iter().copied());
        Cli::try_parse_from(typed)
    }

    // Walked one word at a time, so a nested pair — `["scope", "add"]` — is
    // reached the way a reader types it. Cloned because `render_long_help` wants
    // the command by mutable reference and the tests want several from one
    // parser.
    fn subcommand(names: &[&str]) -> clap::Command {
        let mut command = Cli::command();
        for name in names {
            command = command
                .find_subcommand(name)
                .unwrap_or_else(|| panic!("warlock has a `{name}` subcommand"))
                .clone();
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

    #[derive(Debug)]
    struct FakeScreen {
        terminal: Terminal<TestBackend>,
        suspensions: Vec<bool>,
        reported: Vec<bool>,
    }

    impl FakeScreen {
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

    type Driven = Session<FakeScreen, Passing, Saying, Copying>;

    fn driving(app: App, scope: Scope, tree: &Tree) -> Driven {
        let watched = Watched::start(&scope, tree);
        let root = scope.repo_root.clone();
        Session {
            app,
            screen: FakeScreen::of(80, 24),
            scope,
            manifest: Manifest::new(),
            pact: Pact::with_agent(Passing::filling()),
            chat: Chat::with_agent(root, Saying::answering(ANSWER)),
            clipboard: Copying::taking(),
            confirm: QuitConfirm::default(),
            prompt: ScopePrompt::default(),
            drag: None,
            said: None,
            document: None,
            mouse_captured: true,
            watched,
        }
    }

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
        driving(App::from_rows(rows), scope, &tree)
    }

    fn pressed(driven: &mut Driven, key: KeyEvent) -> bool {
        driven
            .press(key, Instant::now())
            .expect("no key pressed here writes to a terminal")
    }

    const ANSWER: &str = "The tree, the manifest and the pact.";

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn directory(path: &str) -> Row {
        Row::new(0, path, None, NodeState::Unpacted)
    }

    const AT_MOST: Duration = Duration::from_secs(5);

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

    // `session` over a real scratch repository, built the way `run` builds one
    // and, like it, with the tree read first.
    fn session_over(root: &Path) -> Driven {
        let Loaded { tree, .. } = load_tree(root).expect("a scratch repository loads");
        let repo_root = repository_root(tree.root_path()).expect("the load found a repository");
        let scope = Scope {
            chrome: Chrome::of(&repo_root, tree.root_path()),
            root: tree.root_path().to_path_buf(),
            repo_root,
        };
        driving(App::from_tree(&tree), scope, &tree)
    }

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
    fn a_scope_written_outside_warlock_survives_the_next_run_after_a_reload() {
        // `git pull`, or `warlock scope add` in another terminal: the manifest on
        // disk moves while warlock is up. The reload that follows is what every
        // key after it has to act on, or the next save writes the old copy back
        // over the edit.
        let repo = a_repository();
        let mut driven = session_over(repo.path());
        pressed(&mut driven, key(KeyCode::Char('p')));
        rounds_until_settled(&mut driven);

        let outside = Manifest::load(repo.path()).expect("the run saved a manifest");
        let outside = Manifest::with_entries(outside.entries().iter().map(|entry| {
            if entry.module() == "crates/engine/src" {
                entry.clone().with_scope("data-plane")
            } else {
                entry.clone()
            }
        }));
        outside.save(repo.path()).expect("saves");
        fs::write(
            repo.path().join("crates/engine/src/lib.rs"),
            "//! Core engine, revised.\n",
        )
        .expect("a scratch file is writable");
        crate::session::reload(&mut driven.app, &driven.scope, &mut driven.manifest);

        pressed(&mut driven, key(KeyCode::Char('r')));
        rounds_until_settled(&mut driven);

        let after = Manifest::load(repo.path()).expect("a manifest that reads");
        assert_eq!(
            after
                .entry("crates/engine/src")
                .and_then(warlock_engine::PactEntry::scope),
            Some("data-plane"),
            "the refresh wrote warlock's stale copy back over the outside edit"
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

    mod copying {
        use super::{Copying, directory, session};

        // Not "no clipboard here": what arboard hands over is some other
        // program's complaint, and the footer has one line to say it on.
        const REFUSED: &str = "no clipboard on this session\nnothing was listening";

        #[test]
        fn a_copy_that_lands_says_how_much_went() {
            let mut driven = session(vec![directory("/repo/crates")]);

            driven.copy("crates/engine");

            assert_eq!(
                driven.clipboard.copied(),
                ["crates/engine"],
                "the text never reached the clipboard"
            );
            assert_eq!(driven.app.message(), Some("copied 13 characters"));
        }

        #[test]
        fn one_character_is_counted_in_the_singular() {
            let mut driven = session(vec![directory("/repo/crates")]);

            driven.copy("p");

            assert_eq!(driven.app.message(), Some("copied 1 character"));
        }

        #[test]
        fn characters_are_counted_rather_than_the_bytes_utf_8_spells_them_with() {
            let mut driven = session(vec![directory("/repo/crates")]);

            // Five characters and seven bytes: a count of bytes would tell a
            // reader something about UTF-8 rather than about what they copied.
            let text = "péché";
            assert_ne!(text.len(), text.chars().count(), "this text is all ASCII");
            driven.copy(text);

            assert_eq!(driven.app.message(), Some("copied 5 characters"));
        }

        #[test]
        fn a_clipboard_that_refuses_says_so_on_one_line_and_claims_nothing() {
            let mut driven = session(vec![directory("/repo/crates")]);
            driven.clipboard = Copying::refusing(REFUSED);

            driven.copy("crates/engine");

            let said = driven
                .app
                .message()
                .expect("a copy that did not happen is said rather than swallowed");
            assert!(
                !said.contains('\n'),
                "the footer has one line and this wrapped: {said}"
            );
            assert!(
                said.starts_with("nothing was copied"),
                "the footer claims something happened: {said}"
            );
            assert!(
                !said.contains("character"),
                "a failed copy counted characters onto the clipboard: {said}"
            );
            assert!(
                said.contains("no clipboard on this session"),
                "what the clipboard said was thrown away: {said}"
            );
            assert!(
                driven.clipboard.copied().is_empty(),
                "a refused copy left text on the clipboard anyway"
            );
        }
    }

    mod forgetting {
        use std::time::Instant;

        use super::super::MESSAGE_LIFETIME;
        use super::{directory, session};

        #[test]
        fn a_message_stays_up_for_its_lifetime_and_is_gone_after_it() {
            let base = Instant::now();
            let mut driven = session(vec![directory("/repo/crates")]);

            driven.copy("crates/engine");
            driven.forget_stale_message(base);

            driven.forget_stale_message(base + MESSAGE_LIFETIME / 2);
            assert_eq!(
                driven.app.message(),
                Some("copied 13 characters"),
                "the footer dropped what it was told before its time"
            );

            driven.forget_stale_message(base + MESSAGE_LIFETIME);
            assert_eq!(
                driven.app.message(),
                None,
                "the footer is still claiming a copy that has passed"
            );
        }

        #[test]
        fn the_same_sentence_said_again_gets_its_own_lifetime() {
            let base = Instant::now();
            let mut driven = session(vec![directory("/repo/crates")]);

            driven.copy("crates/engine");
            driven.forget_stale_message(base);

            // Most of the way through the first saying's life, the same text
            // again, so the footer's line is the same line to the character.
            // Nothing about the words says it is new, which is what the count
            // rather than a comparison of them is for.
            let again = base + MESSAGE_LIFETIME / 2;
            driven.copy("crates/engine");
            driven.forget_stale_message(again);

            driven.forget_stale_message(base + MESSAGE_LIFETIME);
            assert_eq!(
                driven.app.message(),
                Some("copied 13 characters"),
                "the second copy was timed from the first one's saying"
            );

            driven.forget_stale_message(again + MESSAGE_LIFETIME);
            assert_eq!(driven.app.message(), None);
        }

        #[test]
        fn a_footer_with_nothing_on_it_is_left_alone() {
            let base = Instant::now();
            let mut driven = session(vec![directory("/repo/crates")]);

            driven.forget_stale_message(base);
            driven.forget_stale_message(base + MESSAGE_LIFETIME);

            assert_eq!(driven.app.message(), None);
        }
    }

    mod dragging {
        use ratatui::crossterm::event::{
            KeyCode, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
        };
        use ratatui::layout::Size;

        use super::{ANSWER, Copying, Driven, Focus, Instant, directory, key, pressed, session};

        const PRESS: MouseEventKind = MouseEventKind::Down(MouseButton::Left);
        const DRAG: MouseEventKind = MouseEventKind::Drag(MouseButton::Left);
        const RELEASE: MouseEventKind = MouseEventKind::Up(MouseButton::Left);

        // Piece 0 of the conversation these tests drag over, so what a drag
        // across the word `does` copies is `does` — the question as it was
        // asked, not the row the panel drew it on.
        const ASKED: &str = "what does the engine do?";

        const REFUSED: &str = "no clipboard on this session\nnothing was listening";

        // A session with a turn in the conversation, drawn once: a pointer event
        // is read against the frame it landed on, and the frame is what tells
        // the panel its width and height.
        fn conversing(now: Instant) -> (Driven, Size) {
            let mut driven = session(vec![directory("/repo/crates")]);
            driven.app.panel_mut().start_turn(ASKED, now);
            driven.app.panel_mut().answer_turn(ANSWER, now);
            let size = redrawn(&mut driven);
            (driven, size)
        }

        fn redrawn(driven: &mut Driven) -> Size {
            let size = driven.size().expect("the fake screen has a size");
            driven.draw(size).expect("the fake screen draws");
            size
        }

        // Where a word the frame drew is, as screen cells: the column its first
        // character landed on and the row it landed in. Read off the drawn frame
        // rather than worked out from the layout, so these tests point at the
        // cells a reader would point at.
        fn drawn_at(driven: &Driven, word: &str) -> (u16, u16) {
            found_at(driven, word).unwrap_or_else(|| panic!("the frame never drew {word:?}"))
        }

        // The same search, handing back the absence rather than panicking on it,
        // for the tests that turn on a line *not* being on screen yet.
        fn found_at(driven: &Driven, word: &str) -> Option<(u16, u16)> {
            let buffer = driven.screen.terminal.backend().buffer();
            let area = buffer.area;
            (0..area.height).find_map(|row| {
                let line: String = (0..area.width).map(|x| buffer[(x, row)].symbol()).collect();
                let byte = line.find(word)?;
                let column = line[..byte].chars().count();
                let column = u16::try_from(column).expect("a column of the frame");
                Some((column, row))
            })
        }

        fn point(
            driven: &mut Driven,
            kind: MouseEventKind,
            at: (u16, u16),
            size: Size,
            now: Instant,
        ) {
            let (column, row) = at;
            driven.point(
                MouseEvent {
                    kind,
                    column,
                    row,
                    modifiers: KeyModifiers::NONE,
                },
                size,
                now,
            );
        }

        #[test]
        fn a_drag_across_the_conversation_copies_what_the_highlight_covers() {
            let now = Instant::now();
            let (mut driven, size) = conversing(now);
            let (column, row) = drawn_at(&driven, "does");

            point(&mut driven, PRESS, (column, row), size, now);
            point(&mut driven, DRAG, (column + 2, row), size, now);
            // The space after the `s`, which is the character the drag stopped
            // on and the end of `does`.
            point(&mut driven, DRAG, (column + 4, row), size, now);
            point(&mut driven, RELEASE, (column + 4, row), size, now);

            assert_eq!(
                driven.clipboard.copied(),
                ["does"],
                "the release copied something other than what the drag covered"
            );
            assert_eq!(driven.app.message(), Some("copied 4 characters"));
            // The one thing on screen saying what went: a footer line over a
            // card with nothing highlighted would leave the reader taking
            // warlock's word for it.
            assert_eq!(
                crate::selected_text(&driven.app),
                "does",
                "the copy took the highlight down with it"
            );
            assert_eq!(
                driven.app.focus(),
                Focus::Panel,
                "the press no longer points the keys at the pane it landed in"
            );
        }

        #[test]
        fn a_drag_whose_copy_is_refused_says_so_and_claims_nothing() {
            let now = Instant::now();
            let (mut driven, size) = conversing(now);
            driven.clipboard = Copying::refusing(REFUSED);
            let (column, row) = drawn_at(&driven, "does");

            point(&mut driven, PRESS, (column, row), size, now);
            point(&mut driven, DRAG, (column + 4, row), size, now);
            point(&mut driven, RELEASE, (column + 4, row), size, now);

            let said = driven
                .app
                .message()
                .expect("a copy that did not happen is said rather than swallowed");
            assert!(
                !said.contains('\n'),
                "the footer has one line and this wrapped: {said}"
            );
            assert!(
                !said.contains("character"),
                "the footer counted characters onto a clipboard that refused: {said}"
            );
            assert!(
                said.contains("no clipboard on this session"),
                "what the clipboard said was thrown away: {said}"
            );
            assert!(
                driven.clipboard.copied().is_empty(),
                "a refused copy left text on the clipboard anyway"
            );
            assert_eq!(
                crate::selected_text(&driven.app),
                "does",
                "the highlight came down over a copy that never happened"
            );
        }

        #[test]
        fn a_press_nobody_dragged_from_copies_nothing_and_says_nothing() {
            let now = Instant::now();
            let (mut driven, size) = conversing(now);
            let (column, row) = drawn_at(&driven, "does");

            point(&mut driven, PRESS, (column, row), size, now);
            point(&mut driven, RELEASE, (column, row), size, now);

            assert!(
                driven.clipboard.copied().is_empty(),
                "a press with no drag after it copied the character under it"
            );
            assert_eq!(
                driven.app.message(),
                None,
                "a copy that never happened was reported anyway"
            );
            assert_eq!(
                crate::selected_text(&driven.app),
                String::new(),
                "a press with no drag after it highlighted text"
            );
            assert_eq!(
                driven.app.focus(),
                Focus::Panel,
                "a press in the panel stopped taking the keys"
            );
        }

        #[test]
        fn a_drag_over_the_tree_copies_nothing_and_selects_its_row() {
            let now = Instant::now();
            let (mut driven, size) = conversing(now);
            let (column, row) = drawn_at(&driven, "crates");

            point(&mut driven, PRESS, (column, row), size, now);
            point(&mut driven, DRAG, (column + 3, row), size, now);
            point(&mut driven, RELEASE, (column + 3, row), size, now);

            assert!(
                driven.clipboard.copied().is_empty(),
                "a drag down the tree column copied something"
            );
            assert_eq!(
                driven.app.message(),
                None,
                "a drag over the tree wrote a line on the footer"
            );
            assert_eq!(
                driven.app.selection(),
                None,
                "a drag over the tree highlighted the conversation"
            );
            assert_eq!(
                driven.app.focus(),
                Focus::Tree,
                "a press on a row stopped pointing the keys at the tree"
            );
        }

        #[test]
        fn a_drag_over_another_card_copies_nothing_and_says_nothing() {
            let now = Instant::now();
            // The size this one drags at is the one the frame with the document
            // on it was drawn at, below.
            let (mut driven, _) = conversing(now);
            // The word is in the document rather than the conversation, so the
            // cells the drag covers are cells of the card that is showing.
            driven.app.show_document(["what a document does"], false);
            assert!(
                !driven.app.panel().showing_thread(),
                "the document card never took the conversation's place"
            );
            let size = redrawn(&mut driven);
            let said_before = driven.app.message().map(str::to_owned);
            let (column, row) = drawn_at(&driven, "does");

            point(&mut driven, PRESS, (column, row), size, now);
            point(&mut driven, DRAG, (column + 4, row), size, now);
            point(&mut driven, RELEASE, (column + 4, row), size, now);

            assert!(
                driven.clipboard.copied().is_empty(),
                "a drag over the document card copied a line of it"
            );
            assert_eq!(
                driven.app.message().map(str::to_owned),
                said_before,
                "a drag over the document card wrote a line on the footer"
            );
            assert_eq!(
                driven.app.selection(),
                None,
                "a drag over the document card highlighted the conversation behind it"
            );
        }

        // Typed into the composer so there is a field on screen to press in, and
        // a word to find it by.
        const DRAFT: &str = "draft";

        // A conversation several screens tall, so there is somewhere for the
        // card to scroll, drawn once at the size the drags below land on.
        fn scrollback(now: Instant) -> (Driven, Size) {
            let mut driven = session(vec![directory("/repo/crates")]);
            for turn in 0..12 {
                let asked = format!("question {turn} about the engine");
                driven.app.panel_mut().start_turn(&asked, now);
                driven.app.panel_mut().answer_turn(ANSWER, now);
            }
            let size = redrawn(&mut driven);
            assert!(
                driven.app.panel().scroll_offset() > 0,
                "the conversation fits on the card: nothing here would scroll"
            );
            (driven, size)
        }

        // The card wound back to its first line, where a drag downwards has the
        // whole conversation below it.
        fn wound_back(driven: &mut Driven) -> Size {
            driven.app.scroll_panel_up(usize::MAX);
            redrawn(driven)
        }

        // A point below every row of the card: the footer, which is inside the
        // screen and outside the panel.
        fn below_the_card(size: Size, column: u16) -> (u16, u16) {
            (column, size.height - 1)
        }

        // A draft typed into the composer, so that the field is drawn and there
        // is somewhere in it to press. Typed rather than set, because the
        // composer only takes letters with the keys pointed at it and that is
        // the state a reader presses in it from.
        fn drafting(driven: &mut Driven) {
            driven.app.set_focus(Focus::Composer);
            for letter in DRAFT.chars() {
                assert!(
                    pressed(driven, key(KeyCode::Char(letter))),
                    "typing into the composer ended the session"
                );
            }
        }

        // The tick, driven by hand: `run`'s loop calls `drag_scroll` once a
        // round whether or not an event arrived, so a round with nothing in it
        // is `drag_scroll` on its own.
        mod past_the_edge {
            use ratatui::crossterm::event::MouseEventKind;

            use super::{
                DRAFT, DRAG, Driven, Focus, Instant, PRESS, RELEASE, below_the_card, drafting,
                drawn_at, found_at, point, scrollback, wound_back,
            };
            use crate::rows_per_tick;

            // A pointer moved with nothing held down, which is what a reader
            // whose hand is off the button sends as they cross the footer.
            const MOVED: MouseEventKind = MouseEventKind::Moved;

            fn covered(driven: &Driven) -> usize {
                crate::selected_text(&driven.app).chars().count()
            }

            #[test]
            fn one_row_past_the_edge_is_a_row_a_tick_and_far_past_it_is_several() {
                assert_eq!(
                    rows_per_tick(1),
                    1,
                    "the row just past the edge is not the slow, aimable one"
                );
                assert_eq!(rows_per_tick(6), 2, "the middle of the curve moved");
                assert_eq!(
                    rows_per_tick(40),
                    5,
                    "a pointer dragged to the bottom of the terminal is not at the ceiling"
                );
            }

            #[test]
            fn a_drag_held_below_the_card_keeps_scrolling_and_takes_the_highlight_with_it() {
                let now = Instant::now();
                let (mut driven, _) = scrollback(now);
                let size = wound_back(&mut driven);
                let (column, row) = drawn_at(&driven, "question");

                point(&mut driven, PRESS, (column, row), size, now);
                point(&mut driven, DRAG, below_the_card(size, column), size, now);
                let anchored = driven.app.panel().scroll_offset();
                assert_eq!(
                    covered(&driven),
                    0,
                    "the drag past the edge highlighted text off its own event"
                );

                driven.drag_scroll();
                let after_one = driven.app.panel().scroll_offset();
                assert!(
                    after_one > anchored,
                    "the tick left the card where the drag did: {after_one}"
                );
                let after_one_covered = covered(&driven);
                assert!(
                    after_one_covered > 0,
                    "the card scrolled out from under the highlight"
                );

                // No further event: the pointer is being held still, which is
                // the whole reason this runs off the tick.
                driven.drag_scroll();
                assert!(
                    driven.app.panel().scroll_offset() > after_one,
                    "the scrolling stopped when the pointer did"
                );
                assert!(
                    covered(&driven) > after_one_covered,
                    "the highlight stopped growing while the card went on scrolling"
                );
            }

            #[test]
            fn the_copy_on_release_takes_in_what_the_ticks_scrolled_into_view() {
                // A turn far enough down the conversation that the frame the
                // press lands on has not drawn it: nothing but the ticks can
                // bring it inside the highlight, so finding it on the clipboard
                // is the scrolling and the copy proving each other.
                const LATER: &str = "question 11";

                let now = Instant::now();
                let (mut driven, _) = scrollback(now);
                let size = wound_back(&mut driven);
                let (column, row) = drawn_at(&driven, "question 0");
                let past = below_the_card(size, column);
                assert!(
                    found_at(&driven, LATER).is_none(),
                    "the whole conversation is on screen already: \
                     {LATER} needs no scrolling to reach"
                );

                point(&mut driven, PRESS, (column, row), size, now);
                point(&mut driven, DRAG, past, size, now);
                let anchored = driven.app.panel().scroll_offset();
                for _ in 0..200 {
                    driven.drag_scroll();
                }
                assert!(
                    driven.app.panel().scroll_offset() > anchored,
                    "the ticks left the card where the drag did"
                );
                assert!(covered(&driven) > 0, "the ticks grew no highlight");
                point(&mut driven, RELEASE, past, size, now);

                let [copied] = driven.clipboard.copied() else {
                    panic!(
                        "the release past the edge copied something other than once: {:?}",
                        driven.clipboard.copied()
                    )
                };
                assert!(
                    copied.starts_with("question 0"),
                    "the copy began somewhere other than where the press did: {copied:?}"
                );
                assert!(
                    copied.contains(LATER),
                    "the copy stopped at the edge the drag left rather than at \
                     the line the ticks reached: {copied:?}"
                );
            }

            #[test]
            fn the_scrolling_stops_at_the_end_of_the_thread() {
                let now = Instant::now();
                let (mut driven, _) = scrollback(now);
                let size = wound_back(&mut driven);
                let (column, row) = drawn_at(&driven, "question");

                point(&mut driven, PRESS, (column, row), size, now);
                point(&mut driven, DRAG, below_the_card(size, column), size, now);
                for _ in 0..200 {
                    driven.drag_scroll();
                }

                assert_eq!(
                    driven.app.panel().lines_below(),
                    0,
                    "the ticks left the card short of the end of the conversation"
                );
                let end = driven.app.panel().scroll_offset();
                let held = covered(&driven);

                driven.drag_scroll();

                assert_eq!(
                    driven.app.panel().scroll_offset(),
                    end,
                    "the card scrolled past the last line of the conversation"
                );
                assert_eq!(
                    covered(&driven),
                    held,
                    "the highlight went on growing over a card that had stopped"
                );
            }

            #[test]
            fn a_drag_held_above_the_card_scrolls_the_other_way_and_stops_at_the_top() {
                let now = Instant::now();
                // Left where a conversation sits: at the newest line, with
                // everything else above it.
                let (mut driven, size) = scrollback(now);
                let (column, row) = drawn_at(&driven, "engine");
                let at_the_end = driven.app.panel().scroll_offset();

                point(&mut driven, PRESS, (column, row), size, now);
                // Row zero is the top border of the panes, which is past every
                // row of the card.
                point(&mut driven, DRAG, (column, 0), size, now);
                driven.drag_scroll();

                assert!(
                    driven.app.panel().scroll_offset() < at_the_end,
                    "the tick scrolled the wrong way for a pointer above the card"
                );
                assert!(
                    covered(&driven) > 0,
                    "the highlight did not follow the card upwards"
                );

                for _ in 0..200 {
                    driven.drag_scroll();
                }
                let top = driven.app.panel().scroll_offset();
                assert_eq!(top, 0, "the ticks stopped short of the first line");

                driven.drag_scroll();

                assert_eq!(
                    driven.app.panel().scroll_offset(),
                    0,
                    "the card scrolled above its first line"
                );
            }

            #[test]
            fn the_release_that_ends_the_drag_ends_the_scrolling() {
                let now = Instant::now();
                let (mut driven, _) = scrollback(now);
                let size = wound_back(&mut driven);
                let (column, row) = drawn_at(&driven, "question");
                let past = below_the_card(size, column);

                point(&mut driven, PRESS, (column, row), size, now);
                point(&mut driven, DRAG, past, size, now);
                driven.drag_scroll();
                point(&mut driven, RELEASE, past, size, now);
                let let_go = driven.app.panel().scroll_offset();
                let held = covered(&driven);

                driven.drag_scroll();

                assert_eq!(
                    driven.app.panel().scroll_offset(),
                    let_go,
                    "the card went on scrolling after the button came up"
                );
                assert_eq!(
                    covered(&driven),
                    held,
                    "the highlight went on growing after the button came up"
                );
                assert_eq!(
                    driven.clipboard.copied().len(),
                    1,
                    "the release past the edge copied something other than once"
                );
            }

            #[test]
            fn a_round_with_no_button_held_scrolls_nothing() {
                let now = Instant::now();
                let (mut driven, _) = scrollback(now);
                let where_it_was = driven.app.panel().scroll_offset();

                driven.drag_scroll();

                assert_eq!(
                    driven.app.panel().scroll_offset(),
                    where_it_was,
                    "a card nobody is dragging over scrolled by itself"
                );
                assert_eq!(
                    driven.app.selection(),
                    None,
                    "a tick with no drag behind it highlighted something"
                );
            }

            #[test]
            fn a_press_on_the_tree_dragged_past_the_card_scrolls_nothing() {
                let now = Instant::now();
                let (mut driven, _) = scrollback(now);
                let size = wound_back(&mut driven);
                let (tree_column, tree_row) = drawn_at(&driven, "crates");
                let (column, _) = drawn_at(&driven, "question");
                let where_it_was = driven.app.panel().scroll_offset();

                point(&mut driven, PRESS, (tree_column, tree_row), size, now);
                point(&mut driven, DRAG, below_the_card(size, column), size, now);
                driven.drag_scroll();
                driven.drag_scroll();

                assert_eq!(
                    driven.app.panel().scroll_offset(),
                    where_it_was,
                    "a drag that began in the tree scrolled the conversation"
                );
                assert_eq!(
                    driven.app.selection(),
                    None,
                    "a drag that began in the tree highlighted the conversation"
                );
            }

            #[test]
            fn a_press_on_the_composer_dragged_past_the_card_scrolls_nothing() {
                let now = Instant::now();
                let (mut driven, _) = scrollback(now);
                drafting(&mut driven);
                // Pointed away again, so that the press taking the keys back is
                // this test's proof that it landed in the field rather than on a
                // line of the conversation behind it.
                driven.app.set_focus(Focus::Tree);
                let size = wound_back(&mut driven);
                let field = drawn_at(&driven, DRAFT);
                let (column, _) = drawn_at(&driven, "question");
                let where_it_was = driven.app.panel().scroll_offset();

                point(&mut driven, PRESS, field, size, now);
                assert_eq!(
                    driven.app.focus(),
                    Focus::Composer,
                    "the press landed somewhere other than the composer"
                );
                point(&mut driven, DRAG, below_the_card(size, column), size, now);
                driven.drag_scroll();
                driven.drag_scroll();

                assert_eq!(
                    driven.app.panel().scroll_offset(),
                    where_it_was,
                    "a drag that began in the composer scrolled the conversation"
                );
                assert_eq!(
                    driven.app.selection(),
                    None,
                    "a drag that began in the composer highlighted the conversation"
                );
            }

            #[test]
            fn a_pointer_past_the_edge_with_no_button_held_scrolls_nothing() {
                let now = Instant::now();
                let (mut driven, _) = scrollback(now);
                let size = wound_back(&mut driven);
                let (column, _) = drawn_at(&driven, "question");
                let where_it_was = driven.app.panel().scroll_offset();

                point(&mut driven, MOVED, below_the_card(size, column), size, now);
                driven.drag_scroll();
                driven.drag_scroll();

                assert_eq!(
                    driven.app.panel().scroll_offset(),
                    where_it_was,
                    "a pointer crossing the footer with nothing held down \
                     scrolled the conversation"
                );
                assert_eq!(
                    driven.app.selection(),
                    None,
                    "a pointer crossing the footer with nothing held down \
                     highlighted the conversation"
                );
            }

            #[test]
            fn a_card_put_up_while_the_button_is_held_is_not_scrolled_by_it() {
                let now = Instant::now();
                let (mut driven, _) = scrollback(now);
                let size = wound_back(&mut driven);
                let (column, row) = drawn_at(&driven, "question");

                point(&mut driven, PRESS, (column, row), size, now);
                point(&mut driven, DRAG, below_the_card(size, column), size, now);
                // The one way another card can take the conversation's place
                // without the button coming up first: a key pressed mid-drag.
                driven
                    .app
                    .show_document(std::iter::repeat_n("a line of the file", 200), false);
                let where_it_was = driven.app.panel().scroll_offset();

                driven.drag_scroll();
                driven.drag_scroll();

                assert_eq!(
                    driven.app.panel().scroll_offset(),
                    where_it_was,
                    "the held drag scrolled the document that replaced the conversation"
                );
            }
        }

        // Following, held off for the length of a drag. A turn is long, so
        // copying an earlier answer while a later one arrives is an ordinary
        // thing to want, and a card that went on pulling itself to the newest
        // line would take the text out from under the pointer mid-gesture.
        mod pausing {
            use warlock_tui::Activity;
            use warlock_tui::panel::Showing;

            use super::{
                DRAFT, DRAG, Driven, Instant, PRESS, RELEASE, below_the_card, drafting, drawn_at,
                found_at, point, redrawn, scrollback, wound_back,
            };

            // A word nothing in the conversation has until the appends below put
            // it there, short enough that the panel cannot wrap it: finding it on
            // the frame is the card having gone to the newest line and nothing
            // else.
            const NEWEST: &str = "ozymandias";

            // Everything a live turn puts into the conversation: the question, a
            // line of activity, one of warlock's own notes, the answer. The
            // question and the note go through `Card::accrue`, which is the one
            // path that sets following; the other two are written into the turn
            // already there and ride the flag it left.
            fn a_turn_arrives(driven: &mut Driven, now: Instant) {
                let panel = driven.app.panel_mut();
                panel.start_turn("question 12 about the engine", now);
                panel.record_turn(&Activity::Thinking, now);
                panel.note("warlock has something to say", now);
                panel.answer_turn(NEWEST, now);
            }

            // The conversation growing under a gesture that is not a drag held
            // over it, which has to move the card to the newest line exactly as
            // it does with no button down anywhere. `before` is where the
            // conversation's window was, read by the caller while the card was
            // showing: some of these gestures put another card in front of it,
            // and a turn arriving brings the conversation back.
            fn the_conversation_still_follows(driven: &mut Driven, before: usize, now: Instant) {
                a_turn_arrives(driven, now);

                let panel = driven.app.panel();
                assert!(
                    panel.showing_thread(),
                    "a turn arriving left another card in front of the conversation"
                );
                let after = panel.scroll_offset();
                assert!(
                    after > before,
                    "the conversation stayed where it was: {before} to {after}"
                );
                assert!(
                    panel.follows(),
                    "the card came out of the gesture not following"
                );
                assert_eq!(
                    panel.lines_below(),
                    0,
                    "the card stopped short of the newest line"
                );
            }

            #[test]
            fn a_held_drag_keeps_the_card_still_while_the_conversation_grows() {
                let now = Instant::now();
                let (mut driven, _) = scrollback(now);
                // An earlier part of the conversation, with everything the turn
                // is about to say far below it.
                let size = wound_back(&mut driven);
                let (column, row) = drawn_at(&driven, "question 0");

                point(&mut driven, PRESS, (column, row), size, now);
                point(&mut driven, DRAG, (column + 8, row), size, now);
                let offset = driven.app.panel().scroll_offset();
                let rows = driven.app.panel().window(now);
                let covered = crate::selected_text(&driven.app);
                assert!(
                    !covered.is_empty(),
                    "the drag covered nothing: there is no highlight here to disturb"
                );

                a_turn_arrives(&mut driven, now);

                assert_eq!(
                    driven.app.panel().scroll_offset(),
                    offset,
                    "the arriving turn pulled the held card to the newest line"
                );
                assert_eq!(
                    driven.app.panel().window(now),
                    rows,
                    "the rows under the pointer changed while the button was held"
                );
                assert_eq!(
                    crate::selected_text(&driven.app),
                    covered,
                    "what the highlight covers changed under the held drag"
                );
                redrawn(&mut driven);
                assert!(
                    found_at(&driven, NEWEST).is_none(),
                    "the frame drew the newest line over a drag held on an earlier one"
                );
            }

            #[test]
            fn the_release_hands_the_newest_line_back_mid_turn() {
                let now = Instant::now();
                let (mut driven, _) = scrollback(now);
                // A question out and unanswered, which is the state a reader
                // copies an earlier answer in. The card is following it, so the
                // drag below is over the newest screenful — an earlier part of
                // the conversation than the answer still to come, and a card
                // that was following when the button went down.
                driven
                    .app
                    .panel_mut()
                    .start_turn("question 12 about the engine", now);
                let size = redrawn(&mut driven);
                let (column, row) = drawn_at(&driven, "question 12");
                let held = driven.app.panel().scroll_offset();

                point(&mut driven, PRESS, (column, row), size, now);
                point(&mut driven, DRAG, (column + 8, row), size, now);
                // A line of its own, unlike the turn's first activity line,
                // which takes the place of the one the log draws for a turn
                // that has heard nothing yet.
                driven
                    .app
                    .panel_mut()
                    .note("warlock has something to say", now);

                assert_eq!(
                    driven.app.panel().scroll_offset(),
                    held,
                    "the note pulled the held card down"
                );

                point(&mut driven, RELEASE, (column + 8, row), size, now);

                let released = driven.app.panel().scroll_offset();
                assert!(
                    released > held,
                    "the card came out of the drag still parked where it was held"
                );

                driven.app.panel_mut().answer_turn(NEWEST, now);

                assert!(
                    driven.app.panel().scroll_offset() > released,
                    "the answer left the card where the drag had it"
                );
                assert_eq!(
                    driven.app.panel().lines_below(),
                    0,
                    "the card stopped short of the newest line"
                );
                redrawn(&mut driven);
                assert!(
                    found_at(&driven, NEWEST).is_some(),
                    "the answer that ended the turn was never drawn"
                );
            }

            #[test]
            fn a_release_past_the_card_hands_the_newest_line_back_too() {
                let now = Instant::now();
                let (mut driven, _) = scrollback(now);
                let size = wound_back(&mut driven);
                let (column, row) = drawn_at(&driven, "question 0");
                let past = below_the_card(size, column);

                let where_it_was = driven.app.panel().scroll_offset();

                point(&mut driven, PRESS, (column, row), size, now);
                point(&mut driven, DRAG, past, size, now);
                // The button let go out here, where there is no cell of the card
                // under it: a release all the same, and the end of the hold.
                point(&mut driven, RELEASE, past, size, now);

                the_conversation_still_follows(&mut driven, where_it_was, now);
            }

            #[test]
            fn a_card_put_up_mid_drag_does_not_leave_the_conversation_held() {
                let now = Instant::now();
                let (mut driven, _) = scrollback(now);
                let size = wound_back(&mut driven);
                let (column, row) = drawn_at(&driven, "question 0");

                let where_it_was = driven.app.panel().scroll_offset();

                point(&mut driven, PRESS, (column, row), size, now);
                point(&mut driven, DRAG, (column + 8, row), size, now);
                // A key pressed with the button still down, which is the one way
                // another card takes the conversation's place mid-drag.
                driven.app.show_document(["what a document does"], false);
                point(&mut driven, RELEASE, (column + 8, row), size, now);

                the_conversation_still_follows(&mut driven, where_it_was, now);
            }

            #[test]
            fn a_press_nobody_dragged_from_leaves_the_card_following() {
                let now = Instant::now();
                let (mut driven, size) = scrollback(now);
                let (column, row) = drawn_at(&driven, "question");

                let where_it_was = driven.app.panel().scroll_offset();

                point(&mut driven, PRESS, (column, row), size, now);
                point(&mut driven, RELEASE, (column, row), size, now);

                the_conversation_still_follows(&mut driven, where_it_was, now);
            }

            #[test]
            fn a_drag_that_began_in_the_tree_leaves_the_card_following() {
                let now = Instant::now();
                let (mut driven, size) = scrollback(now);
                let (column, row) = drawn_at(&driven, "crates");

                let where_it_was = driven.app.panel().scroll_offset();

                point(&mut driven, PRESS, (column, row), size, now);
                point(&mut driven, DRAG, (column + 3, row), size, now);

                the_conversation_still_follows(&mut driven, where_it_was, now);
            }

            #[test]
            fn a_drag_that_began_in_the_composer_leaves_the_card_following() {
                let now = Instant::now();
                let (mut driven, _) = scrollback(now);
                drafting(&mut driven);
                let size = redrawn(&mut driven);
                let field = drawn_at(&driven, DRAFT);
                let (column, row) = drawn_at(&driven, "question");

                let where_it_was = driven.app.panel().scroll_offset();

                point(&mut driven, PRESS, field, size, now);
                point(&mut driven, DRAG, (column, row), size, now);

                the_conversation_still_follows(&mut driven, where_it_was, now);
            }

            #[test]
            fn a_drag_that_began_on_the_footer_leaves_the_card_following() {
                let now = Instant::now();
                let (mut driven, size) = scrollback(now);
                let (column, row) = drawn_at(&driven, "question");

                let where_it_was = driven.app.panel().scroll_offset();

                point(&mut driven, PRESS, below_the_card(size, column), size, now);
                point(&mut driven, DRAG, (column, row), size, now);

                the_conversation_still_follows(&mut driven, where_it_was, now);
            }

            #[test]
            fn a_drag_over_the_document_card_leaves_the_conversation_following() {
                let now = Instant::now();
                let (mut driven, _) = scrollback(now);
                // Read while the conversation is still the card showing, which
                // is the one moment the panel answers for it.
                let where_it_was = driven.app.panel().scroll_offset();
                driven.app.show_document(["what a document does"], false);
                let size = redrawn(&mut driven);
                let (column, row) = drawn_at(&driven, "document");

                point(&mut driven, PRESS, (column, row), size, now);
                point(&mut driven, DRAG, (column + 4, row), size, now);

                the_conversation_still_follows(&mut driven, where_it_was, now);
            }

            #[test]
            fn a_drag_over_the_account_card_leaves_the_conversation_following() {
                let now = Instant::now();
                let (mut driven, _) = scrollback(now);
                let where_it_was = driven.app.panel().scroll_offset();
                driven.app.start_account(now);
                driven
                    .app
                    .panel_mut()
                    .write_run(|account| account.open_section("crates/engine", now));
                // The conversation has content, so a run does not put its own
                // account up: the swap key is what a reader would press, and this
                // is that press without the keyboard.
                driven.app.panel_mut().show(Showing::Account);
                let size = redrawn(&mut driven);
                let (column, row) = drawn_at(&driven, "crates/engine");

                point(&mut driven, PRESS, (column, row), size, now);
                point(&mut driven, DRAG, (column + 4, row), size, now);

                the_conversation_still_follows(&mut driven, where_it_was, now);
            }
        }
    }

    mod pasting {
        use super::{Focus, Instant, directory, session};
        use crate::chatting::Asked;

        #[test]
        fn a_paste_with_the_keyboard_off_the_composer_changes_nothing_anywhere() {
            let mut driven = session(vec![directory("/repo/crates"), directory("/repo/docs")]);
            // Where a session opens: the keys are commands and there is no
            // draft for anything to land in.
            assert_ne!(
                driven.app.focus(),
                Focus::Composer,
                "this test is about the keyboard being somewhere else"
            );
            let selected = driven.app.selected();
            let mode = driven.app.panel().mode();

            driven.paste("crates\ndocs\n");

            assert_eq!(
                driven.chat.composer().draft(),
                "",
                "a paste aimed at nothing was typed into the field anyway"
            );
            assert_eq!(
                driven.app.selected(),
                selected,
                "the pasted lines moved the tree's selection"
            );
            assert_eq!(
                driven.app.panel().mode(),
                mode,
                "the pasted lines changed register"
            );
            assert!(
                driven.app.message().is_none(),
                "a paste nobody can act on said something on the footer"
            );
            assert!(!driven.chat.answering(), "a paste started a turn");
        }

        #[test]
        fn a_paste_at_a_muted_field_leaves_the_draft_byte_for_byte() {
            let mut driven = session(vec![directory("/repo/crates")]);
            driven.app.set_focus(Focus::Composer);
            driven.paste("half a question");

            // A question put out without going past the field, which is what
            // leaves a draft standing under the muting: a submit would have
            // emptied it on the way through, and `Chat::settle_field` — still
            // the one thing that sets the flag — is what `say` calls.
            driven.chat.say(
                &mut driven.app,
                "what is a pact?",
                "what is a pact?",
                Asked::Answer,
                Instant::now(),
            );
            assert!(
                driven.chat.composer().is_muted(),
                "a question is out and the field still types"
            );

            driven.paste("\nand the rest of it");

            assert_eq!(
                driven.chat.composer().draft(),
                "half a question",
                "the muted field took a paste"
            );
            assert!(
                driven.chat.composer().is_muted(),
                "a paste handed the keyboard back mid-turn"
            );
        }

        #[test]
        fn a_multi_line_paste_lands_whole_and_asks_nothing() {
            let mut driven = session(vec![directory("/repo/crates")]);
            driven.app.set_focus(Focus::Composer);

            // The block that used to send line one and lose the other two.
            driven.paste("what is a pact?\nand what is a scope?\nand a sigil?");

            assert_eq!(
                driven.chat.composer().draft(),
                "what is a pact?\nand what is a scope?\nand a sigil?",
                "the pasted lines did not all reach the draft"
            );
            assert!(
                !driven.chat.answering(),
                "a newline in a paste started a turn"
            );
            assert!(
                driven.app.panel().thread().is_none(),
                "a paste sent something: there is a conversation and nobody asked for one"
            );
            assert!(
                !driven.chat.composer().is_muted(),
                "a paste muted the field"
            );
        }
    }
}
