//! `warlock resume <TICKET>` and the panel's `/resume <TICKET>`: the operator
//! saying they have looked at a halt, so the sub-tasks it stopped are runnable
//! again and the next pull finds work rather than halting a second time.
//!
//! Both roads are [`release`] and differ only in where the report goes — a
//! writer, or the thread and the composer. A release worded twice would be two
//! accounts of one write, and the shell's is the one a reader has already met.
//!
//! The one write is the machine-local run record, and that is why nothing here
//! asks the boundary: no socket is opened, no `git`, `gh` or `claude` is run and
//! no ticket moves, so there is nothing for a scope to gate. A ticket whose scope
//! this machine does not hold resumes, and a dirty tree resumes — the pull that
//! picks the run up is where both questions are asked, and it asks them of the
//! checkout it is about to work in. Neither refusal below is the boundary's **3**.
//!
//! No `--json`, matching every other subcommand that writes: what a script reads
//! afterwards is the record, which is a file rather than a stream to be caught.

use std::io::{self, Write};
use std::path::Path;
use std::time::Instant;

use warlock_engine::{PullRun, Reset, ResetMode, pulls, run_dir};
use warlock_tui::{App, Converses};

use crate::chatting::Chat;
use crate::error::{Error, one_line};
use crate::standing::{FOR_RESUME, Standing};

pub(crate) fn resume(ticket: &str, failed_only: bool) -> Result<(), Error> {
    let standing = Standing::here(FOR_RESUME)?;
    // The error rather than `check`'s `.ok()`, for [`mod@crate::push`]'s reason
    // with nothing bought by a softer reading: the run records sit under the
    // home, so a machine with no home holds no run to release rather than an
    // answer of "no run for this ticket".
    let home = Standing::home()?;

    resumed(
        &home,
        standing.repo_root(),
        ticket,
        failed_only,
        &mut io::stdout(),
    )
}

// Split from `resume` the way `pushed` is split from `push`: the two directories
// and the writer are parameters rather than reads, so the whole command runs
// against a temporary home with no terminal, and no test in this crate can
// release a run under the developer's own home by standing in the wrong
// directory.
fn resumed<W: Write>(
    home: &Path,
    root: &Path,
    ticket: &str,
    failed_only: bool,
    out: &mut W,
) -> Result<(), Error> {
    let release = release(home, root, ticket, mode(failed_only))?;

    for line in &release.lines {
        say(out, line);
    }
    // The hand-off a shell needs, which is the one line the panel does not print:
    // there, the command lands in the composer instead. See [`resume_press`].
    say(
        out,
        &format!(
            "`warlock pull {} --ticket {}` works the ticket again",
            release.scope, release.ticket
        ),
    );

    Ok(())
}

/// `/resume <TICKET>` on the panel: the same read and the same save, the same
/// changes on the thread, and the command that works the ticket again left in the
/// composer as an ordinary draft.
///
/// The work is [`release`]'s and is not done a second way here, so a halt released
/// from the panel and one released at a shell are the same write reported in the
/// same words — including both refusals, in [`mod@crate::error`]'s sentences.
/// `--failed-only` has no spelling in the panel, so this is always the whole halt.
///
/// The home is handed in rather than read, for [`crate::puller::Puller`]'s reason:
/// it cannot move under a running warlock, and a second reading per keystroke
/// would be a second answer to where the run records are.
///
/// The conversation is taken whole rather than the draft handed back for the loop
/// to offer — which is how a cut's proposed answer reaches the field. The two are
/// not alike: a cut's question arrives at the bottom of a later round out of a
/// worker, while this is one file read and one file written on the round the
/// command was typed, and a return value would be a hand-off with one caller and
/// nowhere to be held in between.
#[allow(
    dead_code,
    reason = "the loop reaches this in WAR-143.05; its tests reach it now"
)]
pub(crate) fn resume_press<C: Converses>(
    app: &mut App,
    chat: &mut Chat<C>,
    home: Option<&Path>,
    root: &Path,
    ticket: &str,
    in_flight: Option<&str>,
    now: Instant,
) {
    // Asked before anything is read, because a run in flight is a session editing
    // this working tree: the sub-tasks this would put back to `pending` are the
    // ones that run is deciding the fate of.
    if let Some(line) = in_flight {
        app.panel_mut().note(refused(line), now);
        return;
    }
    // `Standing::home`'s own sentence, asked of the error that words it rather
    // than written again here.
    let Some(home) = home else {
        app.panel_mut()
            .note(one_line(&Error::NoHome.to_string()), now);
        return;
    };

    match release(home, root, ticket, ResetMode::Everything) {
        Ok(release) => {
            for line in release.lines {
                app.panel_mut().note(line, now);
            }
            // Nothing is sent: the field holds it, the cursor is at the end of
            // it, and every editing key works on it — see [`Chat::offer`]. What
            // a reader does about a halt they have just released is theirs, and
            // a `/pull` warlock sent for them would start a run off a keystroke
            // that asked for a record to be written.
            chat.offer(&format!("/pull {} {}", release.scope, release.ticket));
        }
        // The ticket this checkout never pulled and the run with nothing left to
        // put back, flattened as the thread takes a line. Neither wrote anything.
        Err(error) => app.panel_mut().note(one_line(&error.to_string()), now),
    }
}

// The one line every keystroke that races a pull is refused with, with the pull
// named by the value holding it: what this adds is what this did not do.
fn refused(in_flight: &str) -> String {
    format!("{in_flight}; this `/resume` changed no run record")
}

/// What one release did, for whoever is reporting it.
///
/// The scope and the ticket come off the record rather than from the caller: a run
/// knows which queue took it, and the command that works it again is spelled from
/// the record on both roads out of here.
struct Release {
    lines: Vec<String>,
    scope: String,
    ticket: String,
}

// Saved before a word of it is worded, which is the opposite of what a push does
// and for the opposite reason: a push's project exists whatever the record does
// next, while nothing here has happened until `state.json` lands. Lines handed
// back first would tell a reader their halt was released by a resume that then
// failed to write it, and the next pull would halt again on the same sub-tasks.
fn release(home: &Path, root: &Path, ticket: &str, mode: ResetMode) -> Result<Release, Error> {
    let mut run = loaded(home, root, ticket)?;
    // Read before the reset, which moves the run to `resumed`: what the refusal
    // below names is the status the record was holding when it was read.
    let status = run.status();

    let changed = run.resume(mode);
    if changed.is_empty() {
        return Err(Error::NothingToResume {
            ticket: ticket.to_owned(),
            status,
            failed_only: mode == ResetMode::FailedOnly,
        });
    }

    run.save(home, root)
        .map_err(|source| Error::Runs { source })?;

    Ok(Release {
        lines: changed.iter().map(released).collect(),
        scope: run.scope().to_owned(),
        ticket: run.ticket().to_owned(),
    })
}

// A record that is absent and one that will not read are two answers here and
// never one, which is the reading `PullRun::load` insists on: the first is a
// ticket this machine never pulled, and the second is a record broken by a hand
// edit, describing a branch that may be holding somebody's uncommitted work.
fn loaded(home: &Path, root: &Path, ticket: &str) -> Result<PullRun, Error> {
    PullRun::load(home, root, ticket).map_err(|source| match source {
        pulls::Error::NotFound { .. } => Error::NoRun {
            ticket: ticket.to_owned(),
            directory: run_dir(home, root, ticket),
        },
        source => Error::Runs { source },
    })
}

const fn mode(failed_only: bool) -> ResetMode {
    if failed_only {
        ResetMode::FailedOnly
    } else {
        ResetMode::Everything
    }
}

// The status the sub-task had, then the status it has, then why it stopped: the
// reason is the only surviving copy — the reset dropped it from the record — and
// it is flattened for `error.rs`'s reason, because a session's account of a
// failure is prose and may arrive with newlines in it.
fn released(reset: &Reset) -> String {
    let line = format!(
        "`{}` was `{}` and is `pending` again",
        reset.id(),
        reset.was().as_str()
    );
    match reset.was().reason() {
        Some(reason) => format!("{line} — {}", one_line(reason)),
        None => line,
    }
}

fn say<W: Write>(out: &mut W, fact: &str) {
    drop(writeln!(out, "warlock: {fact}"));
}

// Every test drives the whole subcommand against a temporary home, so none of
// them opens a socket, runs a subprocess or reads a record that is not its own.
#[cfg(test)]
#[path = "tests/resume.rs"]
mod tests;
