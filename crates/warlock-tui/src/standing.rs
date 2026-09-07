//! `here` and `home` are where a subcommand turns the process environment into
//! a repository root, a working directory and a home directory; everything
//! below takes all three as parameters. `at` is the way past them, and it is
//! not only for tests — `session::load_app` has already walked to a root and
//! asking the environment a second time could answer differently.
//!
//! A question stands here and nothing more, because none of `stale`, `fresh`,
//! `check`, `config` or `init` writes a pact. A write stands and is let
//! through: `edits::Opened` is built on top of this one and cannot be
//! constructed without `boundary::verdict` having said the covering scope is
//! open to this machine, so the gate is not a step a write can forget.

use std::env;
use std::path::{Path, PathBuf};

use warlock_engine::{Manifest, manifest, repository_root, to_manifest_path};

use crate::error::Error;

const HOME: &str = "HOME";

const USERPROFILE: &str = "USERPROFILE";

// The nine tails of `Error::NoRepository`'s sentence — "warlock could not find
// a repository to *…*" — together because they are one vocabulary. They used to
// sit beside their own subcommands, and two were re-typed as literals inside
// `error.rs`'s tests, where rewording the original failed nothing.
pub(crate) const FOR_CLAUDE_MD: &str = "write `CLAUDE.md` at";

pub(crate) const FOR_SIGILS: &str = "hold sigils for";

pub(crate) const FOR_CHECK: &str = "answer about the boundary over";

pub(crate) const FOR_LISTING: &str = "list the directories under";

pub(crate) const FOR_UNPACT: &str = "un-pact anything under";

pub(crate) const FOR_SCOPE_ADD: &str = "write a scope in";

pub(crate) const FOR_SCOPE_REMOVE: &str = "clear a scope in";

pub(crate) const FOR_PACT: &str = "pact anything under";

pub(crate) const FOR_REFRESH: &str = "refresh anything under";

// The working directory is kept as well as the root because the two answer
// different questions: the root is what stored paths are spelled against, and
// the working directory is what a relative argument is joined onto. A subcommand
// that used only the root would resolve `warlock check src` against the wrong
// place from a subdirectory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Standing {
    working_dir: PathBuf,
    repo_root: PathBuf,
}

impl Standing {
    // `wanted` is one of the `FOR_*` tails above, so a reader outside a
    // repository is told which of their commands could not be answered rather
    // than a bare "no repository".
    //
    // The root is asked for directly rather than taken from a tree load: every
    // caller here edits or answers about one path, and walking the tree to find
    // its root would read every directory in the repository to answer a question
    // about ancestors.
    pub(crate) fn here(wanted: &'static str) -> Result<Self, Error> {
        let working_dir =
            env::current_dir().map_err(|source| Error::WorkingDirectory { source })?;
        let repo_root = repository_root(&working_dir).ok_or(Error::NoRepository {
            start: working_dir.clone(),
            wanted,
        })?;
        Ok(Self {
            working_dir,
            repo_root,
        })
    }

    // For tests, and for the one caller that has already done the walk:
    // `session::load_app` loads a tree first and gets the root out of it, so
    // asking the environment again would be a second answer to a settled
    // question.
    pub(crate) const fn at(working_dir: PathBuf, repo_root: PathBuf) -> Self {
        Self {
            working_dir,
            repo_root,
        }
    }

    pub(crate) fn repo_root(&self) -> &Path {
        &self.repo_root
    }

    // Never normalised beyond the join, so a `..` that climbs out of the
    // repository is refused later rather than resolved back inside it.
    pub(crate) fn target(&self, path: impl AsRef<Path>) -> PathBuf {
        self.working_dir.join(path)
    }

    // A missing manifest is an empty one, the same reading the loader takes:
    // nothing on disk and nothing pacted are the same thing to answer about, and
    // pressing `p` in a repository with no `.warlock/` is how the first manifest
    // gets written. A file that exists and will not parse is a failure rather
    // than an empty answer — that is a repository whose pacts are unknown, not
    // one with none.
    pub(crate) fn manifest(&self) -> Result<Manifest, Error> {
        match Manifest::load(&self.repo_root) {
            Err(manifest::Error::NotFound { .. }) => Ok(Manifest::new()),
            other => other.map_err(|source| Error::Manifest { source }),
        }
    }

    // Read as an `OsString`, since a home directory is a path rather than text
    // and need not be UTF-8, and an empty value is treated as unset — an
    // exported but empty `HOME` would otherwise resolve the config to
    // `/.warlock/...`.
    //
    // Callers that can do without one say `.ok()`: a home that will not resolve
    // means nothing is held, which is a state of the answer rather than a
    // failure to reach one. `warlock config` takes the error, because a home is
    // the thing it was asked to write under.
    pub(crate) fn home() -> Result<PathBuf, Error> {
        [HOME, USERPROFILE]
            .into_iter()
            .find_map(|variable| env::var_os(variable).filter(|value| !value.is_empty()))
            .map(PathBuf::from)
            .ok_or(Error::NoHome)
    }

    // One line, in one place, so a root check and every row of an answer cannot
    // disagree about what a path is called or about what it means for one to
    // have no name.
    pub(crate) fn spelled(&self, path: &Path) -> Result<String, Error> {
        to_manifest_path(&self.repo_root, path).map_err(|source| Error::Unspellable { source })
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::{FOR_CHECK, Standing};

    fn standing(root: &Path) -> Standing {
        Standing::at(root.to_path_buf(), root.to_path_buf())
    }

    #[test]
    fn a_relative_argument_is_joined_onto_the_working_directory_and_not_the_root() {
        let standing = Standing::at(PathBuf::from("/repo/crates"), PathBuf::from("/repo"));

        assert_eq!(
            standing.target("engine"),
            PathBuf::from("/repo/crates/engine"),
            "a relative path is the reader's, spelled from where they are standing"
        );
    }

    #[test]
    fn an_absolute_argument_is_taken_as_it_was_typed() {
        let standing = Standing::at(PathBuf::from("/repo/crates"), PathBuf::from("/repo"));

        assert_eq!(standing.target("/elsewhere"), PathBuf::from("/elsewhere"));
    }

    #[test]
    fn a_repository_that_never_pacted_anything_stands_on_an_empty_manifest() {
        let dir = tempfile::tempdir().expect("a temporary directory");

        let manifest = standing(dir.path())
            .manifest()
            .expect("a missing manifest reads as an empty one");

        assert!(
            manifest.entries().is_empty(),
            "nothing on disk and nothing pacted are the same thing to answer about"
        );
    }

    #[test]
    fn a_manifest_that_will_not_parse_is_an_error_rather_than_an_empty_start() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let warlock = dir.path().join(".warlock");
        std::fs::create_dir_all(&warlock).expect("the bookkeeping directory");
        std::fs::write(warlock.join("pacts.toml"), "this is not toml {{{").expect("a broken file");

        standing(dir.path())
            .manifest()
            .expect_err("a manifest that exists and will not parse is a failure");
    }

    #[test]
    fn the_root_itself_spells_as_a_dot() {
        let standing = Standing::at(PathBuf::from("/repo"), PathBuf::from("/repo"));

        assert_eq!(
            standing
                .spelled(Path::new("/repo"))
                .expect("the root spells"),
            ".",
            "the manifest's own spelling of the root"
        );
    }

    #[test]
    fn a_path_outside_the_repository_has_no_spelling() {
        let standing = Standing::at(PathBuf::from("/repo"), PathBuf::from("/repo"));

        standing
            .spelled(Path::new("/elsewhere"))
            .expect_err("nothing outside the repository has a manifest form");
    }

    #[test]
    fn a_missing_repository_names_what_the_subcommand_wanted_one_for() {
        // `here` reads the process's own directory, so what is pinned here is
        // the sentence rather than the walk: every tail is one of the nine
        // above, and `NoRepository` carries the one it was given.
        let error = Error::NoRepository {
            start: PathBuf::from("/tmp"),
            wanted: FOR_CHECK,
        };

        assert!(
            error.to_string().contains(FOR_CHECK),
            "the tail is what tells a reader which command could not be answered: {error}"
        );
    }

    use crate::error::Error;
}
