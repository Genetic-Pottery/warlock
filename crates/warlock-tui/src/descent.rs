//! One descent, one save, and one reading of what the engine says along the
//! way. The panel's `p` and `r` spawn a worker that sends each [`RunEvent`] over
//! a channel; `warlock pact` and `warlock refresh` descend on the loop's own
//! thread and print them. What each door *says* about an event is its own; what
//! must not differ is which engine entry point a gesture means, how many times
//! the manifest is written, and when a cancel is honoured, because a door that
//! differed on any of those would record a different repository from the same
//! keystroke.
//!
//! Both doors take the events with an exhaustive `match`, so a hook the engine
//! gains is a compile error in each until it has decided what to say, rather
//! than a default method that quietly says nothing.
//!
//! `Descent::Unpact` is here and is not a run: no walk, no pass, no hash, and
//! every `WARLOCK.md` left where it is. It is in this module anyway so that the
//! save below it is the same line rather than a special case. Only the panel's
//! `p` reaches it — the shell's `warlock unpact` is `edits`' road on purpose,
//! since it spends no model pass and needs neither an agent nor an observer.

use std::path::{Path, PathBuf};

use warlock_engine::{
    Agent, Manifest, PactedSubtree, Pacting, document, pact, pact_subtree, refresh_subtree,
    unpact_subtree,
};
use warlock_tui::Cancel;

use crate::error::Error;
use crate::standing::{FOR_PACT, FOR_REFRESH};

// A carried tag rather than three copies of `descend`, because the two doors
// must not be able to drift into calling a different entry point, or saving a
// different number of times, for the same gesture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Descent {
    Pact,
    Refresh,
    Unpact,
}

impl Descent {
    // Only the two the shell has subcommands for are ever asked: the un-pact
    // the shell does is `edits`' and carries its own tail, and the un-pact this
    // module does is a keystroke inside a warlock that already found its
    // repository. The pact's tail is the closest thing to true if it is reached.
    pub(crate) const fn wanted(self) -> &'static str {
        match self {
            Self::Pact | Self::Unpact => FOR_PACT,
            Self::Refresh => FOR_REFRESH,
        }
    }
}

// The manifest is saved exactly once, here, after the descent and never during
// it. The engine writes every `WARLOCK.md` and hands back a manifest as a
// *value*, so nothing under `.warlock/` moves until the line at the bottom of
// this function — which is what makes a partly-failed run still worth recording
// and a cancelled run keep what it finished, and a rule that would stop being
// one the moment there were two places saving.
//
// `manifest` is the one in hand rather than one read here: both callers already
// hold it, and reading it again would be a second answer to a settled question,
// one that could disagree with the boundary already judged against it.
//
// The error is typed rather than flattened to a line. The panel wants a footer
// sentence and the shell wants an exit status, and an `Error` is the one value
// that can still become either; flattening here would leave the shell parsing
// prose to find out whether it should exit 1.
pub(crate) fn descend(
    descent: Descent,
    target: &Path,
    repo_root: &Path,
    manifest: &Manifest,
    agent: &dyn Agent,
    cancel: &Cancel,
    sink: &mut dyn FnMut(RunEvent),
) -> Result<PactedSubtree, Error> {
    let observer = &mut Relay { cancel, sink };
    let subtree = match descent {
        // Every directory in the subtree, whatever state it was in.
        Descent::Pact => pact_subtree(target, repo_root, manifest, agent, observer)
            .map_err(|source| Error::Pact { source })?,
        // Only the stale ones, and which those are is the engine's judgement
        // from the same manifest handed in here: it keeps the grant of
        // everything it skipped, so a fresh directory costs no pass and loses
        // nothing.
        Descent::Refresh => refresh_subtree(target, repo_root, manifest, agent, observer)
            .map_err(|source| Error::Pact { source })?,
        // Pure manifest editing: the only thing it can refuse is a path the
        // manifest has no spelling for, and it reaches neither the agent nor the
        // observer. Shaped like the other two so the save below is one line.
        Descent::Unpact => PactedSubtree {
            manifest: unpact_subtree(target, repo_root, manifest)
                .map_err(|source| Error::Manifest { source })?,
            failures: Vec::new(),
            problems: Vec::new(),
            repairs: Vec::new(),
        },
    };

    subtree
        .manifest
        .save(repo_root)
        .map_err(|source| Error::Manifest { source })?;
    Ok(subtree)
}

// Owned, so the panel can send it to another thread; the engine's `Defect` and
// `Mend` are flattened to their own sentences, because what a door says about
// one is that sentence and the types that say why belong to the engine.
#[derive(Debug)]
pub(crate) enum RunEvent {
    Starting {
        directory: PathBuf,
        position: usize,
        total: usize,
    },
    Describing {
        position: usize,
        total: usize,
        bytes: u64,
    },
    Requesting {
        files: usize,
        bytes: u64,
    },
    Rejected {
        defects: Vec<String>,
        attempt: usize,
        attempts: usize,
    },
    Repaired {
        directory: PathBuf,
        mend: String,
    },
    Documented {
        directory: PathBuf,
    },
    Unchanged {
        directory: PathBuf,
    },
    Skipped {
        directory: PathBuf,
        below: PathBuf,
    },
}

// The crate's one `pact::Observer`.
struct Relay<'a> {
    cancel: &'a Cancel,
    sink: &'a mut dyn FnMut(RunEvent),
}

impl pact::Observer for Relay<'_> {
    // The one place a run is asked to stop between passes. Asked *before*
    // anything is reported, so a cancelled run neither announces a directory it
    // will not describe nor describes it; a pass that has already started is the
    // agent's own to give up on.
    fn starting(&mut self, directory: &Path, position: usize, total: usize) -> Pacting {
        if carry_on(self.cancel) == Pacting::Stop {
            return Pacting::Stop;
        }
        (self.sink)(RunEvent::Starting {
            directory: directory.to_path_buf(),
            position,
            total,
        });
        Pacting::Continue
    }

    // The directory and the file's name are dropped: the section a file's
    // progress belongs under is the one the `Starting` before it opened.
    fn describing(
        &mut self,
        _directory: &Path,
        _name: &str,
        bytes: u64,
        position: usize,
        total: usize,
    ) {
        (self.sink)(RunEvent::Describing {
            position,
            total,
            bytes,
        });
    }

    fn requesting(&mut self, files: usize, bytes: u64) {
        (self.sink)(RunEvent::Requesting { files, bytes });
    }

    fn rejected(
        &mut self,
        _directory: &Path,
        defects: &[document::Defect],
        attempt: usize,
        attempts: usize,
    ) {
        (self.sink)(RunEvent::Rejected {
            defects: defects.iter().map(ToString::to_string).collect(),
            attempt,
            attempts,
        });
    }

    fn repaired(&mut self, directory: &Path, mend: &document::Mend) {
        (self.sink)(RunEvent::Repaired {
            directory: directory.to_path_buf(),
            mend: mend.to_string(),
        });
    }

    fn documented(&mut self, directory: &Path) {
        (self.sink)(RunEvent::Documented {
            directory: directory.to_path_buf(),
        });
    }

    fn unchanged(&mut self, directory: &Path) {
        (self.sink)(RunEvent::Unchanged {
            directory: directory.to_path_buf(),
        });
    }

    fn skipped(&mut self, directory: &Path, below: &Path) {
        (self.sink)(RunEvent::Skipped {
            directory: directory.to_path_buf(),
            below: below.to_path_buf(),
        });
    }
}

fn carry_on(cancel: &Cancel) -> Pacting {
    if cancel.is_cancelled() {
        Pacting::Stop
    } else {
        Pacting::Continue
    }
}

#[cfg(test)]
#[path = "tests/descent.rs"]
mod tests;
