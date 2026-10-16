//! The headless writes — `warlock scope add` and `warlock scope remove` — and
//! the boundary both are asked over.
//!
//! [`Opened`] cannot be built without that question having been asked, which is
//! how the ordering is kept, and first means first: before the spelling, before
//! the existence check, before any look at what the manifest holds. A closed
//! boundary must not be able to answer "there is no entry for that directory",
//! because that is a fact about the inside of a manifest a reader has just been
//! told they may not work in.
//!
//! A boundary this machine does not open is one line on stderr and exit status
//! **3** rather than 1, because the two want opposite things done about them: a
//! 1 is warlock unable to do the thing, a 3 is warlock declining to, and
//! re-running will never work. There is no `--force`.

use std::path::{Path, PathBuf};

use warlock_engine::Manifest;

use crate::boundary::{Verdict, permits};
use crate::error::Error;
use crate::query::spelled;
use crate::rescope::{Record, RecordFields, TypedRecord, rescope};
use crate::settings::Settings;
use crate::standing::{FOR_SCOPE_ADD, FOR_SCOPE_REMOVE, Standing};

// The type is the gate: the fields are private to this module and the only
// constructor asks the boundary, so possessing an `Opened` is proof that this
// machine may redraw the boundary at the path inside it. A later subcommand
// that wants to edit the manifest from a shell asks for one and inherits the
// check rather than remembering to repeat it.
#[derive(Debug)]
pub(crate) struct Opened {
    repo_root: PathBuf,
    manifest: Manifest,
    // Joined onto the working directory and never normalised beyond that, so a
    // `..` that climbs out of the repository is refused rather than resolved
    // back inside it.
    target: PathBuf,
}

impl Opened {
    // Every input is a parameter — the manifest in hand, the home the caller
    // resolved, the path it joined — so the tests run against a temporary home
    // and a temporary repository. The environment becomes those parameters in
    // exactly one place, `opened`.
    //
    // A path the manifest cannot spell is passed through as open: coverage has
    // nothing to say about a path that is not in this repository, and the command's own refusal
    // — one line, naming the root it is not inside — is the better sentence than
    // a boundary refusal on a technicality. It is refused a moment later, in the
    // write, so nothing is written either way.
    pub(crate) fn new(
        repo_root: PathBuf,
        home: Option<&Path>,
        manifest: Manifest,
        target: PathBuf,
    ) -> Result<Self, Error> {
        // Resolved here rather than by the caller, so that the one thing this
        // check reads from disk is read inside the check. A home that cannot be
        // resolved is no scope open rather than a config that would not read:
        // there is no file in that case, so there is nothing broken to report.
        // Flattened here, at the door: what the shell needs is the two-valued
        // fact the gate takes, and the third state is a line only the panel's
        // "AI can work in" box and `warlock check` ever print. See `boundary::permits`.
        let held = Settings::at(home.map(Path::to_path_buf), &repo_root).open_or_unknown();
        let verdict = permits(&target, &repo_root, &manifest, held.as_slice());
        // Refused paths are spellable by construction: a path with no manifest
        // form has no coverage, so it did not reach here.
        // The `?` keeps the fallible call honest rather than being a second
        // refusal.
        if let Verdict::Closed { scope } = verdict {
            return Err(Error::ClosedScope {
                path: spelled(&repo_root, &target)?,
                scope,
            });
        }

        Ok(Self {
            repo_root,
            manifest,
            target,
        })
    }

    // The path is spelled first because the ordering rule stops at the boundary
    // and not at the write: a path with no manifest form is this command's own
    // refusal, and asking first means a run with two things wrong with it answers
    // about where it was pointed. Every rule after that is [`rescope`]'s — the
    // tracker word and which flags go with it included — which the `s` key's
    // windows ask as well.
    fn scoped<'a>(&self, scope: &str, record: impl Into<Record<'a>>) -> Result<String, Error> {
        let module = spelled(&self.repo_root, &self.target)?;
        let rescoped = rescope(&self.manifest, &module, Some(scope), record)
            .map_err(|refusal| Error::Scope { refusal })?;
        self.saved(&rescoped.manifest)?;

        Ok(scoped_line(
            &module,
            rescoped.scope.as_deref().unwrap_or_default(),
            rescoped.was.as_deref(),
        ))
    }

    // A directory carrying no scope is success and not a refusal, and the save
    // still happens: one road through this function, and what it writes is a
    // manifest identical to the one it read. A second, quieter road through a
    // write is a thing a caller then has to reason about.
    fn unscoped(&self) -> Result<String, Error> {
        let module = spelled(&self.repo_root, &self.target)?;
        let rescoped = rescope(&self.manifest, &module, None, RecordFields::default())
            .map_err(|refusal| Error::Scope { refusal })?;
        self.saved(&rescoped.manifest)?;

        Ok(unscoped_line(&module, rescoped.was.as_deref()))
    }

    fn saved(&self, next: &Manifest) -> Result<(), Error> {
        next.save(&self.repo_root)
            .map_err(|source| Error::Manifest { source })
    }
}

// The one place in the headless writes where the environment becomes a home path
// and a repository root; everything past it takes both as parameters, which is
// what keeps the tests off the developer's own home. `path` is joined onto the
// working directory, which leaves an absolute one as it stands, and nothing on
// disk has to exist for the boundary to be asked: coverage is a walk up the
// manifest's stored paths and never a walk of the filesystem.
//
// `wanted` is the tail of the sentence a missing repository is refused with, so
// each subcommand says what *it* could not do.
//
// A config that will not read is deliberately not a failure here: it is a
// state of the answer, and every scoped directory reads as closed.
pub(crate) fn opened(wanted: &'static str, path: &Path) -> Result<Opened, Error> {
    let standing = Standing::here(wanted)?;
    let manifest = standing.manifest()?;
    let home = Standing::home().ok();
    let target = standing.target(path);

    Opened::new(
        standing.repo_root().to_path_buf(),
        home.as_deref(),
        manifest,
        target,
    )
}

// Two lines, because the halves are elsewhere on purpose: `opened` is the
// boundary and the resolution, `Opened::scoped` is the edit and the sentence,
// and this is the subcommand. `main` prints a refusal on stderr and takes the
// status from the error — a 3 for a closed boundary, a 1 for everything else.
pub fn scope_add(path: &Path, scope: &str, record: TypedRecord<'_>) -> Result<(), Error> {
    println!(
        "warlock: {}",
        opened(FOR_SCOPE_ADD, path)?.scoped(scope, record)?
    );
    Ok(())
}

pub fn scope_remove(path: &Path) -> Result<(), Error> {
    println!("warlock: {}", opened(FOR_SCOPE_REMOVE, path)?.unscoped()?);
    Ok(())
}

// The scope that was there before is named only when there was a different one:
// moving somebody's boundary is worth saying out loud, because a scope is
// somebody else's boundary, while re-writing the scope a directory already carried is a no-op
// nobody needs a clause about.
fn scoped_line(module: &str, scope: &str, was: Option<&str>) -> String {
    match was {
        Some(was) if was != scope => format!("{module} is scoped `{scope}` — was `{was}`"),
        _ => format!("{module} is scoped `{scope}`"),
    }
}

// Two sentences, because there are two things that can have happened and a
// reader is owed the difference: a boundary that was there and is not any more,
// named so the run is auditable, and a directory that carried no scope to begin
// with — which is success, exits 0, and says so rather than implying a removal
// nobody performed.
fn unscoped_line(module: &str, was: Option<&str>) -> String {
    match was {
        Some(was) => format!("{module} is no longer scoped — was `{was}`"),
        None => format!("{module} carried no scope"),
    }
}

#[cfg(test)]
#[path = "tests/edits.rs"]
mod tests;
