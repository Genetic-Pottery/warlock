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
    Error, Interactive, POLL_INTERVAL, TypedRecord, branch_clear, branch_show, branch_use, brief,
    close_scopes, key_add, key_forget, key_list, key_show, key_use, open_list, open_scopes, pull,
    push, resume, scope_add, scope_remove, status_for,
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
    about = "Scopes over a repository: where AI may work, and which board its work goes to.",
    long_about = None
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

/// Everything warlock does without opening the tree.
#[derive(Debug, Clone, PartialEq, Eq, Subcommand)]
enum Command {
    #[command(
        about = "Let AI work in these scopes of this repository on this machine, or name the ones it can.",
        long_about = None
    )]
    Open {
        // Optional, so a bare `warlock open` names the open scopes, the way a
        // bare `/open` does in the chat bar.
        /// Scope names, or * for every scope.
        #[arg(value_name = "SCOPE")]
        scopes: Vec<String>,
    },
    #[command(
        about = "Stop AI working in these scopes of this repository on this machine.",
        long_about = None
    )]
    Close {
        /// Scopes open on this machine.
        #[arg(value_name = "SCOPE", required = true)]
        scopes: Vec<String>,
    },
    #[command(
        about = "Say which scope covers a path and whether AI can work in it on this machine.",
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
        about = "Set or clear the scope on a directory.",
        long_about = None
    )]
    Scope {
        #[command(subcommand)]
        command: ScopeCommand,
    },
    #[command(
        about = "Name the key a scope is bound to, or store, list, bind and forget the named board keys this machine holds.",
        long_about = None,
        args_conflicts_with_subcommands = true,
        arg_required_else_help = true
    )]
    Key {
        // A positional beside the subcommands rather than a `show` word, so
        // `warlock key <SCOPE>` reads as `/key <SCOPE>` does in the chat bar.
        // A subcommand word wins over a scope spelled the same, so a scope
        // named `list` is answered by `key list` here; `/key list` does the
        // same.
        /// The scope whose bound key name to print.
        #[arg(value_name = "SCOPE")]
        scope: Option<String>,
        #[command(subcommand)]
        command: Option<KeyCommand>,
    },
    #[command(
        about = "Show or set the branch this checkout's pulls start from and open pull requests against.",
        long_about = None
    )]
    Branch {
        // Optional, so a bare `warlock branch` names the base, the way a bare
        // `/branch` does in the chat bar.
        #[command(subcommand)]
        command: Option<BranchCommand>,
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
        // Both required, and the scope never inferred from the open scopes: a
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
        // scope to mean — a machine with two scopes open would have to guess which
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
        about = "Write a scope onto a directory.",
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
        // The tracker and the five record values are all optional to clap and
        // all required by warlock in the cases that need them, because which
        // cases those are depends on what the manifest already records and on
        // which tracker was named: a `required = true` on any of them would
        // refuse the two-argument run that points a directory at a scope
        // something already records, and clap has read neither
        // `.warlock/scopes.toml` nor the word beside it. A `value_parser` on
        // the tracker would be the same mistake one step smaller — clap's exit
        // status of 2 spent on a rule warlock words itself.
        //
        // All six are judged in [`mod@edits`], past the boundary, where what
        // the manifest holds may be looked at, and stored exactly as typed:
        // what a team key, a site, a project key, a review state or a label may
        // be belongs to somebody else's tracker.
        /// Which board a new scope's record files to: linear or jira.
        #[arg(value_name = "TRACKER")]
        tracker: Option<String>,
        /// The Linear team key a new scope's issues are filed to.
        #[arg(long, value_name = "TEAM_KEY")]
        team_key: Option<String>,
        /// The Jira site a new scope's issues are filed to.
        #[arg(long, value_name = "BASE_URL")]
        base_url: Option<String>,
        /// The Jira project key a new scope's issues are filed under.
        #[arg(long, value_name = "PROJECT_KEY")]
        project_key: Option<String>,
        /// The review state a new scope's issues are routed to.
        #[arg(long, value_name = "REVIEW_STATE")]
        review_state: Option<String>,
        /// The label a new scope's issues carry.
        #[arg(long, value_name = "LABEL")]
        label: Option<String>,
    },
    #[command(
        about = "Clear the scope on a directory.",
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
/// The store is machine-wide and a binding is per scope: `add` and `list` are
/// about the box and stand in no repository, while `use` and `forget` act on
/// this checkout's config. A scope binds one name with `use` and there is no
/// default for one that has bound none, and no fallback to another scope's key
/// — an unbound scope is unbound, and says so, rather than quietly filing into
/// whichever Linear workspace some other scope happens to answer to.
#[derive(Debug, Clone, PartialEq, Eq, Subcommand)]
enum KeyCommand {
    #[command(
        about = "Read a board key on stdin and store it under a name.",
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
        about = "Bind one of the stored keys to a scope of this checkout.",
        long_about = None
    )]
    Use {
        // Scope first and key second, as in `push <scope> <path>`, `draft
        // <scope>` and `pull <scope>`: the scope is what the whole family leads
        // with. Neither is optional and neither is a flag — a one-argument
        // spelling would have to guess which scope was meant, and the guess is
        // work filed into the wrong Linear workspace.
        /// The scope to bind, as `scopes.toml` names it.
        #[arg(value_name = "SCOPE")]
        scope: String,
        /// The name of a key this machine already holds.
        #[arg(value_name = "NAME")]
        name: String,
    },
    #[command(
        about = "Remove a stored key from this machine by name.",
        long_about = None
    )]
    Forget {
        /// The name to remove, whatever scopes are bound to it.
        #[arg(value_name = "NAME")]
        name: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Subcommand)]
enum BranchCommand {
    #[command(
        about = "Pull from and open pull requests against this branch.",
        long_about = None
    )]
    Use {
        /// A branch the remote has.
        #[arg(value_name = "BRANCH")]
        name: String,
    },
    #[command(
        about = "Go back to the remote's default branch.",
        long_about = None
    )]
    Clear,
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
    // cosmetic: `warlock check` refuses a path with no repository-relative
    // spelling with a 1, and a script has to be able to tell that refusal from
    // a typo by the exit status alone, without reading a word of either
    // message. A 0 means the question was answered whatever the answer was.
    //
    // The writes below add a third register, which is warlock's too and is not
    // a failure at all: a boundary closed on this machine is refused
    // with nothing spent, and that is a 3. See `status_for`.
    let cli = Cli::parse();

    let outcome = match cli.command {
        None => {
            // Before anything touches the terminal: a panic during setup has to
            // leave the terminal usable too.
            install_panic_hook();
            run()
        }
        Some(Command::Open { scopes }) if scopes.is_empty() => open_list(),
        Some(Command::Open { scopes }) => open_scopes(&scopes),
        Some(Command::Close { scopes }) => close_scopes(&scopes),
        // A plain check exits 0 for a closed scope: the closed boundary is its
        // answer, not a failure to reach one, which leaves `jq -e '.opens'` to
        // spend the non-zero status on the verdict. `--gate` spends it instead:
        // nothing on stdout, and a closed or unscoped path is the boundary's
        // **3** through `Error::ClosedScope` or `Error::Unscoped`, the register
        // the writes already refuse in.
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
        Some(Command::Scope { command }) => match command {
            ScopeCommand::Add {
                path,
                scope,
                tracker,
                team_key,
                base_url,
                project_key,
                review_state,
                label,
            } => scope_add(
                &path,
                &scope,
                // Handed over flat and whole, the tracker word among the rest:
                // which of these a record is made of, and whether this run
                // needed any of them, is settled past the boundary and not here.
                TypedRecord {
                    tracker: tracker.as_deref(),
                    team_key: team_key.as_deref(),
                    base_url: base_url.as_deref(),
                    project_key: project_key.as_deref(),
                    review_state: review_state.as_deref(),
                    label: label.as_deref(),
                },
            ),
            ScopeCommand::Remove { path } => scope_remove(&path),
        },
        Some(Command::Key { scope, command }) => match (scope, command) {
            (_, Some(KeyCommand::Add { name })) => key_add(&name),
            (_, Some(KeyCommand::List { json })) => key_list(json),
            (_, Some(KeyCommand::Use { scope, name })) => key_use(&scope, &name),
            (_, Some(KeyCommand::Forget { name })) => key_forget(&name),
            (Some(scope), None) => key_show(&scope),
            // `arg_required_else_help` prints the help before this is reached.
            (None, None) => unreachable!("clap requires a scope or a subcommand"),
        },
        Some(Command::Branch { command }) => match command {
            None => branch_show(),
            Some(BranchCommand::Use { name }) => branch_use(&name),
            Some(BranchCommand::Clear) => branch_clear(),
        },
        Some(Command::Brief) => brief(),
        Some(Command::Push {
            scope,
            path,
            dry_run,
        }) => push(&scope, &path, dry_run),
        Some(Command::Cut {
            scope,
            project,
            dry_run,
        }) => planned::cut(&scope, project.as_deref(), dry_run),
        Some(Command::Pull {
            scope,
            ticket,
            dry_run,
        }) => pull(&scope, ticket.as_deref(), dry_run),
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
        // anything: a pull or a turn reports its progress over a channel that
        // only the bottom of this loop reads, and a progress line that waits for
        // a keystroke to appear is worse than none at all.
        if event::poll(POLL_INTERVAL)? {
            // The instant the event arrived, read once and here: it is the
            // instant anything this key starts is as old as, so a turn is
            // clocked from the keystroke that asked for it rather than from the
            // first thing the model got round to saying.
            let now = Instant::now();
            match event::read()? {
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
                // bracketed paste is on (see `TerminalGuard::enter`). Nothing is
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
