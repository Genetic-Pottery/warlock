//! The binary: argv, the terminal's lifecycle and the event loop, and nothing
//! else. Everything they hand values to is in `warlock_tui`.
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

// `Error` belongs to the library, and the lint measures an enum from another
// crate whole where it measures a local one by its largest variant: it fires on
// `run` here and on none of the library's functions returning the same type.
#![allow(clippy::result_large_err)]

use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Instant;

use clap::{Parser, Subcommand};
use ratatui::crossterm::event::{self, Event};
use warlock_tui::check::{self, check};
use warlock_tui::planned;
use warlock_tui::{
    Error, Interactive, Listing, POLL_INTERVAL, RecordFields, brief, configure, key_add,
    key_forget, key_list, key_use, list, pact, pull, push, refresh, resume, scope_add,
    scope_remove, status_for, unpact,
};

mod terminal;

use terminal::{TerminalGuard, install_panic_hook};

/// `about` is spelled out on every command below, with `long_about = None`, so
/// that the doc comments in this file are free to say why rather than being
/// lifted into `--help`. The `#[arg]` fields are the exception and keep theirs.
#[derive(Debug, Clone, PartialEq, Eq, Parser)]
#[command(
    name = "warlock",
    version,
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
        // Optional to clap and required in practice, which is `--gate`'s doing
        // rather than a softening: `required_unless_present` keeps a plain check
        // with no path the malformed invocation it has always been — a check is a
        // walk up from one place, so there is no whole-repository answer for an
        // omitted path to mean, and leaving it off is clap's wording and clap's
        // own exit status of 2 — while leaving `--gate` free to be asked with no
        // path at all, which is the hook form reading one on stdin. The doc
        // comments here are one line each for the reason every other one in this
        // file is: clap lifts them into `--help`.
        /// Which path to answer about.
        #[arg(value_name = "PATH", required_unless_present = "gate")]
        path: Option<PathBuf>,
        /// Answer as one JSON object instead of three lines of prose.
        #[arg(long)]
        json: bool,
        // A clap conflict with `--json` rather than a flag quietly ignored
        // beside it: a gate prints no envelope at all, so `--gate --json` is
        // somebody about to pipe an empty stdout into `jq`, and being told so at
        // the command line costs them one read where the silence would cost them
        // a debugging session.
        /// Refuse instead of answering: exit 3 when PATH's scope is closed here.
        #[arg(long, conflicts_with = "json")]
        gate: bool,
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
        about = "Describe a directory and everything below it, writing a .warlock.md for each.",
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
    #[command(
        about = "Store, list, bind and forget the named Linear keys this machine holds.",
        long_about = None
    )]
    Key {
        #[command(subcommand)]
        command: KeyCommand,
    },
    #[command(
        about = "Argue a brief at the shell, sending a turn on a blank line.",
        long_about = None
    )]
    // No argument, and there is nothing for one to name: the conversation
    // decides what the document is about, and where it would be written is
    // `briefs.toml`'s. No flag either — `warlock brief` cannot leave brief mode,
    // because it is brief mode.
    Brief,
    #[command(
        about = "File a brief as a project on the board a scope files to.",
        long_about = None
    )]
    Push {
        // Both required, and the scope never inferred from the sigils held: a
        // project filed to the wrong board is one nothing here can take back.
        // A `String` and not a validated type, for the reason a scope is one on
        // `scope add`.
        /// Which scope's board to file to.
        #[arg(value_name = "SCOPE")]
        scope: String,
        /// Which brief to file.
        #[arg(value_name = "PATH")]
        path: PathBuf,
        /// Print what would be sent and open no socket.
        #[arg(long)]
        dry_run: bool,
    },
    #[command(
        name = "draft",
        about = "List a scope's planned projects, or draft tickets from one project's scope block.",
        long_about = None
    )]
    Cut {
        /// Which scope's board to read from.
        #[arg(value_name = "SCOPE")]
        scope: String,
        // Optional, and the only way to name a project: without it the planned
        // projects are listed, one slug and name a line, and whoever is at the
        // shell picks one. Nothing here matches a name, so a slug is taken
        // exactly or refused.
        /// The project's slug, the hex at the end of its URL; omit it to list them.
        #[arg(value_name = "SLUG")]
        project: Option<String>,
        /// Print the project, its status and its slices, and draft nothing.
        #[arg(long)]
        dry_run: bool,
    },
    #[command(
        about = "Work the next ready ticket in a scope's queue to an open pull request.",
        long_about = None
    )]
    Pull {
        // Required, like the push's path and for its reason: a pull is about one
        // scope's queue, and there is no whole-repository answer for an omitted
        // scope to mean — a machine holding two sigils would have to guess which
        // board's work to start. A `String` and not a validated type, as a scope
        // is everywhere else: what a scope may be is the engine's to say.
        /// Which scope's queue to take a ticket from.
        #[arg(value_name = "SCOPE")]
        scope: String,
        // Optional to clap and the only way to say which ticket: what it picks
        // among is the operator's own queue, held to the same rules the chooser
        // holds it to. There is no `--any` — taking somebody else's ticket is a
        // reassignment a human makes on the board.
        /// Work this ticket instead of choosing, if the queue's rules allow it.
        #[arg(long, value_name = "TICKET")]
        ticket: Option<String>,
        /// Print the ticket that would be taken and every one passed over; write nothing.
        #[arg(long)]
        dry_run: bool,
    },
    #[command(
        about = "Release a halted run, so the next pull of that ticket finds work it can run.",
        long_about = None
    )]
    Resume {
        // Required, and a ticket rather than a scope: a halt is one run, the run
        // records are keyed by ticket, and there is no whole-queue answer for an
        // omitted one to mean. A `String` and not a validated type, as every
        // ticket identifier here is — what an identifier may be is the board's.
        /// Which ticket's halted run to release.
        #[arg(value_name = "TICKET")]
        ticket: String,
        // The only flag, and there is not going to be a second: everything else a
        // resume could be asked is a fact about the run, which is a file the
        // operator can read. No `--dry-run` either — a resume that changed nothing
        // is a refusal with the record untouched, so the dry run is the run.
        /// Put only the `failed` sub-tasks back, leaving `blocked` and `crossed` ones as they are.
        #[arg(long)]
        failed_only: bool,
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
        // Optional to clap and required by warlock, because which it is
        // depends on whether the manifest already records the name: a
        // `required = true` here would refuse the flagless run that writes an
        // already-recorded scope, and clap has not read `.warlock/pacts.toml`.
        // Judged in [`mod@edits`], past the boundary, where what the manifest
        // holds may be looked at. Values are stored exactly as typed — what a
        // team key, a review state or a label may be belongs to somebody
        // else's tracker.
        /// The Linear team key a new scope's issues are filed to.
        #[arg(long, value_name = "TEAM_KEY")]
        team_key: Option<String>,
        /// The review state a new scope's issues are routed to.
        #[arg(long, value_name = "REVIEW_STATE")]
        review_state: Option<String>,
        /// The label a new scope's issues carry.
        #[arg(long, value_name = "LABEL")]
        label: Option<String>,
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

/// The machine's key store, by name.
///
/// There is no argument for the secret and there is not going to be one: argv
/// is readable by every process on the box while the command runs and is
/// written into a shell history afterwards, so the key comes in on stdin.
/// Whether it is echoed there depends on what stdin is — a terminal is read
/// with it off, a pipe exactly as before; see [`mod@key`].
///
/// `add`, `use` and `forget` take no `--json`, matching the other writing
/// subcommands: an envelope is for an answer a script parses, and the only
/// thing that could go in one of these is the name the command was already
/// given.
///
/// A checkout binds one name with `use` and there is no default for one that
/// has bound none — an unbound checkout is unbound, and says so, rather than
/// quietly reaching for whichever key happens to be first in the store.
#[derive(Debug, Clone, PartialEq, Eq, Subcommand)]
enum KeyCommand {
    #[command(
        about = "Read a Linear key on stdin and store it under a name.",
        long_about = None
    )]
    Add {
        // A `String` and not a validated type, for the reason a scope is one:
        // what a name may be is the engine's to say, and a parser judging it
        // here would spend clap's exit status of 2 on a rule warlock words
        // itself.
        /// The name to store the key under.
        #[arg(value_name = "NAME")]
        name: String,
    },
    #[command(
        about = "List the names this machine holds keys for, and never a key.",
        long_about = None
    )]
    List {
        /// Answer as one JSON object instead of one name per line.
        #[arg(long)]
        json: bool,
    },
    #[command(
        about = "Bind one of the stored keys to this checkout by name.",
        long_about = None
    )]
    Use {
        /// The name of a key this machine already holds.
        #[arg(value_name = "NAME")]
        name: String,
    },
    #[command(
        about = "Remove a stored key from this machine by name.",
        long_about = None
    )]
    Forget {
        /// The name to remove, whatever is bound to it.
        #[arg(value_name = "NAME")]
        name: String,
    },
}

fn main() -> ExitCode {
    // Read before anything else happens and, deliberately, before anything
    // touches the terminal: help and a refusal all print on the ordinary
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
        //
        // `--gate` is that same question with the verdict spent on the exit
        // status instead of printed: nothing on stdout, and a closed scope is the
        // boundary's **3** through `Error::ClosedScope`, which is the register
        // the writes already refuse in. It is the one caller that wants the
        // non-zero status, so it takes the one `jq` would otherwise have spent.
        //
        // The pathless `None` is the `PreToolUse` hook form, which clap only lets
        // through for `--gate`'s sake: the path arrives in a payload on stdin
        // instead of in argv, and the refusal is a deny object on stdout instead
        // of a status, because **2** is the only status Claude Code honours from
        // a hook and it does not mean the boundary. Exit 0 either way; see
        // [`check::hook`].
        Some(Command::Check { path, json, gate }) => match (path, gate) {
            (Some(path), false) => check(path, json),
            (Some(path), true) => check::gate(path),
            (None, _) => check::hook(),
        },
        // The first subcommand that writes, dispatched here for every reason
        // the questions are — it prints one line on the ordinary screen and
        // takes no terminal — and with none of a run's machinery: no worker
        // thread, no subprocess, no model pass. What keeps it honest is the
        // boundary, asked before anything else it does; see [`mod@edits`].
        Some(Command::Unpact { path }) => unpact(&path),
        // The two runs, and the first subcommands that spend anything: minutes
        // of model passes, one `claude --print` per directory, a `.warlock.md`
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
            ScopeCommand::Add {
                path,
                scope,
                team_key,
                review_state,
                label,
            } => scope_add(
                &path,
                &scope,
                RecordFields {
                    team_key: team_key.as_deref(),
                    review_state: review_state.as_deref(),
                    label: label.as_deref(),
                },
            ),
            ScopeCommand::Remove { path } => scope_remove(&path),
        },
        // The key store, dispatched here for `config`'s reasons and with one
        // more of its own: `add` takes the terminal itself for the length of
        // one read, so a program that had already entered the alternate screen
        // would be prompting for a secret underneath a frame. What it takes it
        // puts back — see [`mod@key`] — and it arms no panic hook, because the
        // hook exists to restore a session this path never starts. The first two verbs stand in
        // no repository — a key is a fact about the machine — and the last two
        // stand in one because a binding is a fact about a checkout; each
        // resolves what it needs itself. The nesting is clap's and stops here,
        // each arm one call into [`mod@key`].
        Some(Command::Key { command }) => match command {
            KeyCommand::Add { name } => key_add(&name),
            KeyCommand::List { json } => key_list(json),
            KeyCommand::Use { name } => key_use(&name),
            KeyCommand::Forget { name } => key_forget(&name),
        },
        // The first step of the workflow, dispatched here for `config`'s reasons:
        // it prints on the ordinary screen and reads cooked lines off stdin for
        // as long as somebody keeps typing, so nothing about it may touch the
        // terminal — no alternate screen, no raw mode and no panic hook, because
        // the hook exists to restore a session this path never starts. What it
        // spends is one conversation with `claude`, and its only refusals — a
        // brief template or a `briefs.toml` that will not read — are made before
        // the first turn. Gated by nothing here: it writes no file and opens no
        // board, so none of it is the boundary's **3**. See [`mod@briefing`].
        Some(Command::Brief) => brief(),
        // The one subcommand that sends anything anywhere, dispatched here for
        // every reason the writes are — it prints its lines on the ordinary
        // screen and takes no terminal — and gated by nothing here: the scope
        // picks the board rather than opening a directory, so none of what it
        // refuses is the boundary's **3**. Every refusal but a project of the
        // same name already on the board is reached before the socket is
        // opened; see [`mod@push`].
        Some(Command::Push {
            scope,
            path,
            dry_run,
        }) => push(&scope, &path, dry_run),
        // The other half of that one, dispatched beside it and gated by nothing
        // here for the same reason: a cut picks its board by the sigil rather
        // than by opening a directory, so not one of its refusals — a slug the
        // board does not know, a status that is not `Planned`, a scope block
        // that will not parse, nothing left to cut — is the boundary's **3**.
        // They are all ordinary **1**s through `status_for`'s catch-all. What it
        // spends past the read is one drafting session per slice and the issues
        // those file; see [`mod@planned`].
        Some(Command::Cut {
            scope,
            project,
            dry_run,
        }) => planned::cut(&scope, project.as_deref(), dry_run),
        // The third step of the workflow and the one that spends the most:
        // a splitting pass, one session per sub-task, a commit each, a push and a
        // pull request. Dispatched here with the rest and for the same reasons —
        // its progress is lines on the ordinary screen that a script reads through
        // a pipe, so no alternate screen, no raw mode and no panic hook — and
        // gated by the sigil rather than by opening a directory. Two of its
        // refusals are the boundary's **3** all the same: a scope this machine
        // does not hold, and a session that wrote past one. See [`mod@pull`].
        Some(Command::Pull {
            scope,
            ticket,
            dry_run,
        }) => pull(&scope, ticket.as_deref(), dry_run),
        // The one command that turns a halt back into runnable work, dispatched
        // here with the rest and for their reasons — it prints its lines on the
        // ordinary screen and takes no terminal — and the cheapest thing in this
        // match: one file read and one file written, under the home. No board, no
        // `git`, no session and no ticket moved, so there is nothing for a scope to
        // gate and neither of its refusals is the boundary's **3**; both are
        // ordinary **1**s through `status_for`'s catch-all. The pull it hands off
        // to is where the boundary is asked. See [`mod@resume`].
        Some(Command::Resume {
            ticket,
            failed_only,
        }) => resume(&ticket, failed_only),
    };

    // `run` has returned, so the guard inside it has already dropped and the
    // terminal is back to normal; only now is it worth printing anything,
    // because on the alternate screen nobody would ever see it. The
    // subcommands never went near the terminal, and print through the same
    // line so that a failure looks the same however warlock was invoked.
    if let Err(error) = &outcome {
        eprintln!("warlock: {error}");
    }
    ExitCode::from(status_for(&outcome))
}

/// The interactive session: load, take the terminal, then loop. The first two
/// are [`Interactive::open`]'s, in the order that keeps a failure on the
/// ordinary screen.
///
/// Returning is the whole of quitting: the session drops on this stack, which
/// puts the terminal back, cancels any run and kills the `claude` it was
/// waiting on. Nothing joins the worker.
fn run() -> Result<(), Error> {
    let mut session = Interactive::open(TerminalGuard::enter)?;

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

#[cfg(test)]
#[path = "tests/main.rs"]
mod tests;
