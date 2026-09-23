//! `warlock pact <path>` and `warlock refresh <path>`: the two subcommands that
//! spend anything, hence their own file rather than two more arms of
//! [`mod@crate::edits`].
//!
//! [`descended`] takes an [`Opened`], so the boundary is asked before the walk. Asked
//! afterwards it would have listed somebody else's directories before refusing;
//! asked after the first directory it would have spent a pass and overwritten a
//! `WARLOCK.md` that no exit status puts back. Which directories a refresh
//! describes stays the engine's judgement, because a second opinion about
//! staleness here would disagree with the colour the tree is drawn in.
//!
//! Failures are named on stderr one line each, unlike the footer's one-line
//! shape: the reader here is often a script or a log read tomorrow, and "and 99
//! more" throws away the only list of what to go and look at. A cancelled run
//! names none of its own, because nothing tells the killed pass apart from a
//! real failure. The environment is read in [`started`] alone; everything under
//! it takes the repository, agent, writers and say-when as parameters, which is
//! the seam the tests run through.

use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process;
use std::sync::atomic::{AtomicBool, Ordering};

use warlock_engine::{Agent, PactedSubtree, pact, to_manifest_path};
use warlock_tui::{Cancel, ClaudeAgent};

use crate::CANCELLED;
use crate::descent::{Descent, RunEvent, descend};
use crate::edits::{Opened, opened};
use crate::error::{Error, one_line};

// The headless counterpart of the panel's channel, and much smaller: no thread
// and no screen, so where the panel draws every event, this writes the few a
// person watching a pipe can act on.
//
// Generic over the writer rather than reaching for `io::stdout` itself, for the
// reason every other seam in this crate is a parameter: the tests assert on the
// exact lines a run produces, in order, and a function that printed could only
// be tested by spawning a process to read the output of.
//
// A write that fails is ignored — the same shrug the panel gives a send into a
// closed channel, for a stronger reason. A closed stdout is
// `warlock pact . | head -1`, and failing a run of model passes because the
// thing reading its progress went away would be spending minutes of somebody's
// tokens on the state of a pipe.
struct Progress<W: Write> {
    root: PathBuf,
    out: W,
    // Kept because the failure report needs it and nothing else has it: the
    // `PactedSubtree` carries the failures but not the size of the run they
    // happened in, and "3 failed" without "of 100" is the illegible half of the
    // report. The engine's own denominator, unaltered, and it does not move for
    // the length of a run — `starting` is handed `directories.len()` every time
    // — so reading it after the descent reads the number every progress line was
    // counting against.
    //
    // A field rather than a count of the lines written, because the run's size
    // is a thing the engine states and this port's arithmetic about it would be
    // a second opinion waiting to disagree.
    total: usize,
}

impl<W: Write> Progress<W> {
    const fn new(root: PathBuf, out: W) -> Self {
        Self {
            root,
            out,
            total: 0,
        }
    }

    const fn total(&self) -> usize {
        self.total
    }

    fn say(&mut self, fact: &str) {
        // Ignored on purpose; see the type's doc.
        let _ = writeln!(self.out, "warlock: {fact}");
    }

    fn on(&mut self, event: RunEvent) {
        match event {
            // The fraction is the engine's own, unaltered and one-based, and its
            // denominator does not move for the length of the run — so `[3/12]`
            // is a thing a reader can watch rather than a running total that
            // redefines itself. It is on this line and not the completion line
            // because this is where it means something: it counts the
            // directories offered.
            RunEvent::Starting {
                directory,
                position,
                total,
            } => {
                self.total = total;
                self.say(&format!(
                    "[{position}/{total}] documenting {}",
                    named(&self.root, &directory)
                ));
            }
            // On stdout with the progress and not on stderr with the report,
            // because a mended slot is a document that was written, not a
            // directory that was missed: the failure report is the list of
            // things to go and look at, and a repair belongs to the story of the
            // run. A log read tomorrow should still be able to tell a repaired
            // entry from a written one, so it says which slot and what was done
            // to it.
            RunEvent::Repaired { directory, mend } => {
                self.say(&format!("{} — {mend}", named(&self.root, &directory)));
            }
            RunEvent::Documented { directory } => {
                self.say(&format!("documented {}", named(&self.root, &directory)));
            }
            RunEvent::Unchanged { directory } => {
                self.say(&format!("unchanged {}", named(&self.root, &directory)));
            }
            // Both names on the line. A headless run is read in a log after the
            // fact, often by whoever has to explain why a directory is still
            // yellow, and `skipped crates/tui` on its own is the half of the
            // answer that does not help.
            RunEvent::Skipped { directory, below } => {
                self.say(&format!(
                    "skipped {} — {} below it was not documented",
                    named(&self.root, &directory),
                    named(&self.root, &below)
                ));
            }
            // A pipe is read a line per directory; the panel's per-file
            // fractions and retries would bury those lines.
            RunEvent::Describing { .. }
            | RunEvent::Requesting { .. }
            | RunEvent::Rejected { .. } => {}
        }
    }
}

fn named(root: &Path, directory: &Path) -> String {
    to_manifest_path(root, directory).unwrap_or_else(|_| directory.display().to_string())
}

// A value rather than a printing function, for the reason `Opened::unpacted`
// hands its success line back instead of printing it: the report is the
// interesting half of this module, and a test that could only read it by
// spawning a process would be a test of the shell.
//
// Only ever built for a run that had failures — [`report`] answers `None` for a
// clean run — so there is no empty report to be printed as a row of nothing and
// no `0 of 12 directories failed` line.
#[derive(Debug)]
struct Report {
    lines: Vec<String>,
    failed: usize,
    total: usize,
}

impl Report {
    fn onto<W: Write>(&self, err: &mut W) {
        for line in &self.lines {
            // Ignored on purpose; see above.
            let _ = writeln!(err, "warlock: {line}");
        }
    }

    fn status(&self) -> Error {
        Error::Failures {
            failed: self.failed,
            total: self.total,
        }
    }
}

// One line per failing directory and no line for any other. Directories are
// named against `root` the way [`Progress`] names them, so the failure line for
// a directory and the progress line that entered it say the same word; the
// engine's sentence is flattened because a report is read a line at a time and a
// TOML diagnostic wrapping over four of them would be four directories' worth of
// screen for one.
//
// A directory is named once however many ways it went wrong. Today it can only
// go wrong once, so this is a promise about the report rather than a filter that
// fires: `failed` counts directories, `total` counts directories, and the two
// have to be countable against each other.
fn report(root: &Path, failures: &[pact::Failure], total: usize) -> Option<Report> {
    let mut named_already: Vec<String> = Vec::new();
    let mut lines: Vec<String> = Vec::new();
    for failure in failures {
        let directory = named(root, failure.directory());
        if named_already.contains(&directory) {
            continue;
        }
        lines.push(format!("{directory} — {}", one_line(&failure.to_string())));
        named_already.push(directory);
    }

    if lines.is_empty() {
        return None;
    }
    Some(Report {
        failed: lines.len(),
        lines,
        total,
    })
}

// The whole of the signal handling, and it is a flag and two lines. Two presses
// rather than one because they are two different questions. The first is "stop
// when you can", and what it buys is the manifest: the directories already
// documented are hashed, granted and saved by the save at the end of the
// descent, which cannot happen if the process dies here. The second is "stop
// now", honoured literally with nothing saved and nothing printed — nothing is
// corrupted by taking it, because every file is written beside and renamed over
// and the descent's thread is simply abandoned.
//
// No `unsafe`, and none is needed: `ctrlc::set_handler` does the registration
// and calls this closure on an ordinary thread of its own, where latching an
// `AtomicBool`, taking a mutex and killing a child are ordinary rather than the
// undefined behaviour they would be in signal context. That is the whole reason
// the dependency is in the manifest.
//
// `pressed` is this function's own flag and not `Cancel::is_cancelled`, because
// the two say different things: a handle can be latched by the run itself, and
// "has anybody pressed Ctrl-C" is a question about the keyboard.
//
// Called once per process, after the boundary is asked and before the first
// pass, so a run refused here has spent nothing and written nothing.
fn listening() -> Result<Cancel, Error> {
    let cancel = Cancel::new();
    let latch = cancel.clone();
    let pressed = AtomicBool::new(false);
    ctrlc::set_handler(move || {
        if pressed.swap(true, Ordering::SeqCst) {
            // The second press, honoured as asked: no save, no report, and the
            // status the run was going to leave with anyway.
            process::exit(i32::from(CANCELLED));
        }
        latch.cancel();
    })
    .map_err(|source| Error::Signal { source })?;
    Ok(cancel)
}

// The three endings a run can have, decided in one place and in this order.
// `cancelled` wins because a stopped run's failures are the stopping — the
// killed pass comes back in `PactedSubtree::failures` like any other, and
// nothing in that list says which entries the reader caused — so a cancelled run
// prints no per-directory lines at all. What finished is on stdout, where it has
// been all along.
//
// Takes the stderr to write to rather than being three lines inside [`started`],
// for the reason [`Progress`] is generic over its writer: this is the decision
// the endings of a run are made by, and a test that could only read it by
// spawning a process would be a test of the shell.
fn ending<W: Write>(cancelled: bool, report: Option<&Report>, err: &mut W) -> Result<(), Error> {
    if cancelled {
        return Err(Error::Cancelled);
    }
    let Some(report) = report else {
        return Ok(());
    };
    report.onto(err);
    Err(report.status())
}

// The one place in this module that reads the working directory, the home
// directory, stdout or the keyboard, kept to five lines so everything worth
// testing is underneath it. A closed boundary leaves through the `?` with
// nothing spawned, nothing written and no handler installed.
//
// The order is the load-bearing part: a manifest that would not save leaves
// through `descended` as `Error::Manifest` and a 1, failures or cancel or
// neither, because a run whose record never reached the disk is warlock unable
// to do the thing rather than a run that completed imperfectly.
fn started(descent: Descent, path: &Path) -> Result<(), Error> {
    let opened = opened(descent.wanted(), descent.into(), path)?;
    let cancel = listening()?;
    let agent = ClaudeAgent::new().with_cancel(cancel.clone());

    descended(
        &opened,
        descent,
        &agent,
        &cancel,
        io::stdout(),
        &mut io::stderr(),
    )?
    .outcome
}

// The progress is handed back still holding what it wrote and the denominator it
// counted against, because a caller that gave it a `Vec<u8>` wants both. On the
// real road the writer is stdout and nobody reads either again. No `Debug`,
// because [`Progress`] has none: a writer is not a value to print.
//
// The fields are the point: [`started`] reads only `outcome`, while the suite
// reads all four off the same value. Dropping the three would make the
// composition untestable again, which is the thing this type exists to fix.
#[allow(dead_code, reason = "read by the tests that drive this composition")]
struct Descended<W: Write> {
    subtree: PactedSubtree,
    progress: Progress<W>,
    report: Option<Report>,
    outcome: Result<(), Error>,
}

// The composition, and one function rather than four lines in [`started`]
// because the order is the load-bearing part. Four things have to be true
// together, and are true here:
//
// * The descent and the agent answer to one `Cancel`. The handle handed in goes
//   to `descend`, and the caller has already given the same one to the agent —
//   so a single Ctrl-C kills the pass in flight *and* stops the descent at the
//   next directory, rather than doing one and not the other.
// * The manifest is saved before anything has an opinion. That happens inside
//   `descend`, which is what makes `Error::Failures` "completed with failures"
//   rather than a failure, and a cancel a run that recorded what it finished.
// * The denominator is read after the descent. `Progress::total` is `0` until
//   the engine states the run's size, so the report has to count against it
//   afterwards; counting before would report every failure out of nothing.
// * `ending` is last, so a manifest that would not save leaves through the `?`
//   above as the bigger news.
fn descended<O: Write, E: Write>(
    opened: &Opened,
    descent: Descent,
    agent: &dyn Agent,
    cancel: &Cancel,
    out: O,
    err: &mut E,
) -> Result<Descended<O>, Error> {
    // `opened` is the proof the boundary was asked — it cannot be built any other
    // way — so this does not ask it again.
    let mut progress = Progress::new(opened.repo_root().to_path_buf(), out);
    let subtree = descend(
        descent,
        opened.target(),
        opened.repo_root(),
        opened.manifest(),
        agent,
        cancel,
        &mut |event| progress.on(event),
    )?;

    let report = report(opened.repo_root(), &subtree.failures, progress.total());
    let outcome = ending(cancel.is_cancelled(), report.as_ref(), err);

    Ok(Descended {
        subtree,
        progress,
        report,
        outcome,
    })
}

// One line, for the reason `edits::unpact` is two: the whole of the command is
// [`started`], and what tells it from the refresh below is the tag it is handed.
pub(crate) fn pact(path: &Path) -> Result<(), Error> {
    started(Descent::Pact, path)
}

pub(crate) fn refresh(path: &Path) -> Result<(), Error> {
    started(Descent::Refresh, path)
}

#[cfg(test)]
#[path = "tests/running.rs"]
mod tests;
