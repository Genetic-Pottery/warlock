//! `warlock resume <TICKET>`: the operator saying they have looked at a halt, so
//! the sub-tasks it stopped are runnable again and the next `warlock pull` finds
//! work rather than halting a second time.
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

use warlock_engine::{PullRun, Reset, ResetMode, pulls, run_dir};

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
    let mut run = loaded(home, root, ticket)?;
    // Read before the reset, which moves the run to `resumed`: what the refusal
    // below names is the status the record was holding when it was read.
    let status = run.status();

    let changed = run.resume(mode(failed_only));
    if changed.is_empty() {
        return Err(Error::NothingToResume {
            ticket: ticket.to_owned(),
            status,
            failed_only,
        });
    }

    // Saved before a word is printed, which is the opposite of what a push does
    // and for the opposite reason: a push's project exists whatever the record
    // does next, while nothing here has happened until `state.json` lands. Lines
    // printed first would tell a reader their halt was released by a run that
    // then failed to write it, and the next pull would halt again on the same
    // sub-tasks.
    run.save(home, root)
        .map_err(|source| Error::Runs { source })?;

    for reset in &changed {
        say(out, &released(reset));
    }
    // The hand-off, with the scope read out of the record rather than asked of
    // anything: a run knows which queue took it, and a resume that made the
    // reader go and look it up would be sending them to the board for a fact
    // this file is holding.
    say(
        out,
        &format!(
            "`warlock pull {} --ticket {}` works the ticket again",
            run.scope(),
            run.ticket()
        ),
    );

    Ok(())
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
