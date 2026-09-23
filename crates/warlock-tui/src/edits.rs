//! The headless writes — `warlock unpact`, `warlock scope add` and `warlock
//! scope remove` — and the boundary all three are asked over.
//!
//! [`Opened`] cannot be built without that question having been asked, which is
//! how the ordering is kept, and first means first: before the spelling, before
//! the existence check, before any look at what the manifest holds. A closed
//! boundary must not be able to answer "there is no entry for that directory",
//! because that is a fact about the inside of a manifest a reader has just been
//! told they may not work in. [`closed_scope`](crate::session::closed_scope) is
//! the same rule's other door and is not written in terms of this one, since it
//! is about a selected row on an `App` and there is no app here.
//!
//! A boundary this machine does not open is one line on stderr and exit status
//! **3** rather than 1, because the two want opposite things done about them: a
//! 1 is warlock unable to do the thing, a 3 is warlock declining to, and
//! re-running will never work. There is no `--force`.

use std::path::{Path, PathBuf};

use warlock_engine::{Manifest, PactEntry, unpact_subtree};

use crate::boundary::{Operation, Verdict, permits};
use crate::error::Error;
use crate::query::spelled;
use crate::rescope::{RecordFields, rescope};
use crate::session::sigils_under;
use crate::standing::{FOR_SCOPE_ADD, FOR_SCOPE_REMOVE, FOR_UNPACT, Standing};

// The type is the gate: the fields are private to this module and the only
// constructor asks the boundary, so possessing an `Opened` is proof that this
// machine's sigils open the scope covering the path inside it. A later
// subcommand that wants to edit the manifest from a shell asks for one and
// inherits the check rather than remembering to repeat it — `running.rs` is the
// first, and the one it matters most for, because `warlock pact` spends model
// passes and rewrites documents, so a boundary asked after the descent had
// started would already have cost somebody's tokens and somebody else's prose.
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
    // A path the manifest cannot spell is passed through as open, which is
    // `closed_scope`'s own reading of that case: coverage has nothing to say
    // about a path that is not in this repository, and the command's own refusal
    // — one line, naming the root it is not inside — is the better sentence than
    // a boundary refusal on a technicality. It is refused a moment later, in the
    // write, so nothing is written either way.
    pub(crate) fn new(
        repo_root: PathBuf,
        home: Option<&Path>,
        manifest: Manifest,
        target: PathBuf,
        operation: Operation,
    ) -> Result<Self, Error> {
        // Resolved here rather than by the caller, so that the one thing this
        // check reads from disk is read inside the check. A home that cannot be
        // resolved is nothing held rather than a config that would not read:
        // there is no file in that case, so there is nothing broken to report.
        // Flattened here, at the door: what the shell needs is the two-valued
        // fact the gate takes, and the third state is a line only the header
        // and `warlock check` ever print. See `boundary::permits`.
        let held: Vec<String> = home.map_or_else(Vec::new, |home| {
            sigils_under(home, &repo_root).as_slice().to_vec()
        });
        let verdict = permits(operation, &target, &repo_root, &manifest, &held);
        // Refused paths are spellable by construction: a path with no manifest
        // form has no coverage and nothing below it, so it did not reach here.
        // The `?` keeps the fallible call honest rather than being a second
        // refusal.
        match verdict {
            Verdict::Open => {}
            Verdict::Closed { scope } => {
                return Err(Error::ClosedScope {
                    path: spelled(&repo_root, &target)?,
                    scope,
                });
            }
            Verdict::ClosedBelow { scopes } => {
                return Err(Error::ClosedScopeBelow {
                    path: spelled(&repo_root, &target)?,
                    scopes,
                });
            }
        }

        Ok(Self {
            repo_root,
            manifest,
            target,
        })
    }

    // Getters rather than public fields because the fields being private is the
    // gate: a caller may look at what an open boundary gave it and may not
    // assemble one of these out of parts it found lying around.
    pub(crate) fn repo_root(&self) -> &Path {
        &self.repo_root
    }

    pub(crate) const fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    pub(crate) fn target(&self) -> &Path {
        &self.target
    }

    // Three engine calls and nothing else: no walk, no hash, no pass, and not a
    // single `WARLOCK.md` touched. Un-pacting is warlock forgetting it ever
    // promised to keep a document current; the documents are the repository's,
    // and deleting somebody's prose because they stopped tracking its freshness
    // is not a thing warlock gets to do.
    //
    // Reached only through an `Opened` built for `Operation::Unpact`, which is
    // what asked about the boundaries below the path as well as the one over
    // it: this call drops every entry underneath, and an entry is the only home
    // a scope has.
    //
    // The dropped entries are worked out by difference rather than by asking the
    // engine twice: whatever `unpact_subtree` kept is what remains. That keeps
    // the count and the names honest against the engine's rule about what "below"
    // means — including that `crates/engine` does not swallow
    // `crates/engine-tools` — without this file holding an opinion about it.
    //
    // The line comes back rather than being printed here, so the sentence a
    // reader sees is a value a test can assert about.
    fn unpacted(&self) -> Result<String, Error> {
        // Spelled before the edit, because it is the name the answer is about
        // and because it is this command's refusal of a path from outside the
        // repository. `unpact_subtree` refuses the same path on the same grounds
        // a line later; asking here means the refusal happens before a manifest
        // is rebuilt rather than after.
        let path = spelled(&self.repo_root, &self.target)?;
        let remaining = unpact_subtree(&self.target, &self.repo_root, &self.manifest)
            // The engine's own case, rewrapped as the spelling refusal it is:
            // this agrees with the line above by construction, since both are
            // `to_manifest_path`.
            .map_err(|source| Error::Unspellable { source })?;

        let dropped: Vec<&PactEntry> = self
            .manifest
            .entries()
            .iter()
            .filter(|entry| remaining.entry(entry.module()).is_none())
            .collect();

        remaining
            .save(&self.repo_root)
            .map_err(|source| Error::Manifest { source })?;

        Ok(unpacted_line(&path, &dropped))
    }

    // The path is spelled first because the ordering rule stops at the boundary
    // and not at the write: a path with no manifest form is this command's own
    // refusal, and asking first means a run with two things wrong with it answers
    // about where it was pointed. Every rule after that is [`rescope`]'s, which
    // the `s` key's two windows ask as well.
    fn scoped(&self, scope: &str, record: RecordFields<'_>) -> Result<String, Error> {
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
// each subcommand says what *it* could not do. Shared with `running.rs` rather
// than copied there: a second function doing this would be a second order for
// these steps to be in.
//
// A sigil config that will not read is deliberately not a failure here: it is a
// state of the answer — nothing held, so nothing scoped is open.
pub(crate) fn opened(
    wanted: &'static str,
    operation: Operation,
    path: &Path,
) -> Result<Opened, Error> {
    let standing = Standing::here(wanted)?;
    let manifest = standing.manifest()?;
    let home = Standing::home().ok();
    let target = standing.target(path);

    Opened::new(
        standing.repo_root().to_path_buf(),
        home.as_deref(),
        manifest,
        target,
        operation,
    )
}

// Two lines, because the halves are elsewhere on purpose: `opened` is the
// boundary and the resolution, `Opened::unpacted` is the edit and the sentence,
// and this is the subcommand. `main` prints a refusal on stderr and takes the
// status from the error — a 3 for a closed boundary, a 1 for everything else.
pub(crate) fn unpact(path: &Path) -> Result<(), Error> {
    println!(
        "warlock: {}",
        opened(FOR_UNPACT, Operation::Unpact, path)?.unpacted()?
    );
    Ok(())
}

pub(crate) fn scope_add(path: &Path, scope: &str, flags: RecordFields<'_>) -> Result<(), Error> {
    println!(
        "warlock: {}",
        opened(FOR_SCOPE_ADD, Operation::Scope, path)?.scoped(scope, flags)?
    );
    Ok(())
}

pub(crate) fn scope_remove(path: &Path) -> Result<(), Error> {
    println!(
        "warlock: {}",
        opened(FOR_SCOPE_REMOVE, Operation::Scope, path)?.unscoped()?
    );
    Ok(())
}

// The success line is the mitigation for the blast radius, not decoration. In
// the panel you navigate to a visible row with the subtree under the cursor;
// from a shell this is one line in a script that scrolls past. So the count
// comes first — `warlock unpact .` in a repository of forty pacted directories
// says `40 entries dropped`, which is a number a reader notices — and then the
// scopes, and only the scopes. Naming all forty paths would be a paragraph on a
// line that has to stay one line; naming the scoped ones is the half that
// matters, because a scope is somebody else's boundary. Nothing scoped drops the
// clause rather than reporting `0 scoped`: a clause that is almost always "and
// none" trains a reader to stop reading the line.
//
// A dropped entry's scope is named exactly as it is written down, including one
// `validate_scope` would refuse. Coverage ignores such a scope, so it never
// closed this boundary — but it is a word somebody wrote in the file, and
// omitting it would be warlock deciding on a reader's behalf that what they
// wrote did not count.
fn unpacted_line(path: &str, dropped: &[&PactEntry]) -> String {
    let scoped: Vec<String> = dropped
        .iter()
        .filter_map(|entry| {
            entry
                .scope()
                .map(|scope| format!("{}: {scope}", entry.module()))
        })
        .collect();
    let taken = if scoped.is_empty() {
        String::new()
    } else {
        format!(", {} scoped ({})", scoped.len(), scoped.join(", "))
    };
    // Singular for one, because a line a person reads should not say "1
    // entries"; the count itself is what a script would parse either way.
    let entries = if dropped.len() == 1 {
        "entry"
    } else {
        "entries"
    };

    format!(
        "unpacted {path} — {} {entries} dropped{taken}",
        dropped.len()
    )
}

// The scope that was there before is named only when there was a different one:
// moving somebody's boundary is worth saying out loud, for `unpacted_line`'s
// reason, while re-writing the scope a directory already carried is a no-op
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
