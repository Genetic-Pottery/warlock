//! Where warlock is standing when a subcommand runs: the directory it was
//! invoked in, the repository above that, the manifest in it, and this
//! machine's home.
//!
//! Every headless subcommand opens the same way. It asks the process where it
//! is, walks up for a `.git/`, and — depending on which subcommand it is —
//! reads the manifest, resolves a home directory, and joins the path the reader
//! typed onto the working directory. Six entry points used to spell that out
//! for themselves, each carrying its own tail constant for the sentence
//! [`Error::NoRepository`] ends with, and each reaching into a different module
//! for the next piece: `home_directory` from `config`, `load_manifest` from
//! `session`, `spelled` from `query`. [`check`](crate::check) imported all three
//! to assemble one prologue.
//!
//! So the prologue lives here now, as [`Standing`], and the tails live with it.
//!
//! # The two reaches
//!
//! **A question stands.** [`Standing::here`] is the whole of what
//! `warlock stale`, `warlock fresh`, `warlock check`, `warlock config` and
//! `warlock init` need: where am I, and what repository is that. Nothing about
//! it is gated, because none of them writes a pact.
//!
//! **A write stands and is let through.** [`Opened`](crate::edits::Opened) is
//! built on top of this one and adds the boundary: it cannot be constructed
//! without [`verdict`](crate::boundary::verdict) having said the scope covering
//! the target is open to this machine. That is why the gate is not a step a
//! write can forget — there is no way to hold the value without having passed
//! it.
//!
//! # The environment becomes parameters exactly here
//!
//! [`Standing::here`] and [`Standing::home`] are the only places in the front
//! end that read `std::env`. Everything downstream takes what they produce as a
//! parameter, which is what lets the tests run against a temporary repository
//! and a temporary home rather than the developer's own — the engine already
//! takes a home that way, all the way down. [`Standing::at`] is the other
//! constructor, and it is how a test stands somewhere without a process
//! environment to arrange.

use std::env;
use std::path::{Path, PathBuf};

use warlock_engine::{Manifest, manifest, repository_root, to_manifest_path};

use crate::error::Error;

/// The variable a home directory is read from.
const HOME: &str = "HOME";

/// The Windows spelling of [`HOME`], consulted only when `HOME` says nothing.
const USERPROFILE: &str = "USERPROFILE";

/// What `warlock init` wants the repository root for.
///
/// These nine are the tails of [`Error::NoRepository`]'s sentence — "warlock
/// could not find a repository to *…*" — and they live together because they
/// are one vocabulary: nine ways of finishing one sentence, in one wording, in
/// one place. They used to sit beside their own subcommands, each with a doc
/// comment pointing at where the others were, and two of them were re-typed as
/// literals inside `error.rs`'s tests, where rewording the original would not
/// have failed anything.
pub(crate) const FOR_CLAUDE_MD: &str = "write `CLAUDE.md` at";

/// What `warlock config` wants one for.
pub(crate) const FOR_SIGILS: &str = "hold sigils for";

/// What `warlock check` wants one for.
pub(crate) const FOR_CHECK: &str = "answer about the boundary over";

/// What `warlock stale` and `warlock fresh` want one for.
pub(crate) const FOR_LISTING: &str = "list the directories under";

/// What `warlock unpact` wants one for.
pub(crate) const FOR_UNPACT: &str = "un-pact anything under";

/// What `warlock scope add` wants one for.
pub(crate) const FOR_SCOPE_ADD: &str = "write a scope in";

/// What `warlock scope remove` wants one for: the same fact about `.git` with
/// the other consequence on the end of it.
pub(crate) const FOR_SCOPE_REMOVE: &str = "clear a scope in";

/// What `warlock pact` wants one for.
pub(crate) const FOR_PACT: &str = "pact anything under";

/// What `warlock refresh` wants one for.
pub(crate) const FOR_REFRESH: &str = "refresh anything under";

/// Where warlock is, and what repository that is inside.
///
/// Two paths and nothing else. Everything a subcommand goes on to need — the
/// manifest, the home directory, the path the reader typed — is asked of this
/// value rather than of the process, so a subcommand body has no environment in
/// it and a test can put one anywhere.
///
/// The working directory is kept as well as the root because the two answer
/// different questions: the root is what stored paths are spelled against, and
/// the working directory is what a relative argument is joined onto. A
/// subcommand that used only the root would resolve `warlock check src` against
/// the wrong place from a subdirectory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Standing {
    /// The directory the process was invoked in.
    working_dir: PathBuf,
    /// The nearest ancestor holding a `.git/`, the working directory included.
    repo_root: PathBuf,
}

impl Standing {
    /// Where this process is, and the repository above it.
    ///
    /// `wanted` is one of the `FOR_*` tails above: what the subcommand wanted a
    /// repository *for*, so that a reader outside one is told which of their
    /// commands could not be answered rather than a bare "no repository".
    ///
    /// The root is asked for directly rather than taken from a tree load. Every
    /// caller here edits or answers about one path, and walking the tree to find
    /// its root would be reading every directory in the repository to answer a
    /// question about ancestors.
    ///
    /// # Errors
    ///
    /// [`Error::WorkingDirectory`] if the process cannot say where it is, and
    /// [`Error::NoRepository`] — carrying `wanted` — if nothing at or above it
    /// holds a `.git/`.
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

    /// Standing somewhere named rather than somewhere found.
    ///
    /// For tests, and for the one caller that has already done the walk:
    /// [`load_app`](crate::session::load_app) loads a tree first and gets the
    /// root out of it, so asking the environment a second time would be a second
    /// answer to a settled question.
    pub(crate) const fn at(working_dir: PathBuf, repo_root: PathBuf) -> Self {
        Self {
            working_dir,
            repo_root,
        }
    }

    /// Where `.warlock/pacts.toml` lives, and what every stored path is spelled
    /// against.
    pub(crate) fn repo_root(&self) -> &Path {
        &self.repo_root
    }

    /// The path the reader named, joined onto the working directory.
    ///
    /// Absolute when they typed an absolute one, and never normalised beyond
    /// that, so a `..` that climbs out of the repository is refused later rather
    /// than resolved back inside it.
    pub(crate) fn target(&self, path: impl AsRef<Path>) -> PathBuf {
        self.working_dir.join(path)
    }

    /// The manifest as it stands. **A missing one is an empty one.**
    ///
    /// The same reading of a missing file the loader takes: nothing on disk and
    /// nothing pacted are the same thing to answer about, and the difference
    /// only matters to code that would refuse to create the file, which none of
    /// this is — pressing `p` in a repository with no `.warlock/` is how the
    /// first manifest gets written.
    ///
    /// # Errors
    ///
    /// [`Error::Manifest`] when a file is there and will not parse. That is a
    /// failure and not an empty answer: a manifest that exists and cannot be
    /// read is a repository whose pacts are unknown, not one with none.
    pub(crate) fn manifest(&self) -> Result<Manifest, Error> {
        match Manifest::load(&self.repo_root) {
            Err(manifest::Error::NotFound { .. }) => Ok(Manifest::new()),
            other => other.map_err(|source| Error::Manifest { source }),
        }
    }

    /// This machine's home directory, from [`HOME`] or, failing that,
    /// [`USERPROFILE`].
    ///
    /// The only place in warlock that reads either variable. The engine takes a
    /// home as a parameter all the way through, which is what keeps its tests
    /// off the developer's real one, so this is the single point where a
    /// process's environment becomes a path.
    ///
    /// Read as an `OsString`, since a home directory is a path rather than text
    /// and need not be UTF-8, and an empty value is treated as unset — an
    /// exported but empty `HOME` would otherwise resolve the config to
    /// `/.warlock/...`.
    ///
    /// Callers that can do without one say `.ok()`: a home that will not resolve
    /// means nothing is held, which is a state of the answer rather than a
    /// failure to reach one. `warlock config` is the exception and takes the
    /// error, because a home is the thing it was asked to write under.
    ///
    /// An associated function rather than a method, because a home has nothing
    /// to do with where warlock is standing — it is the same path from any
    /// directory in any repository. It lives here anyway, beside
    /// [`Standing::here`], because the two are the whole of what the front end
    /// reads from the environment and they are worth finding in one place.
    ///
    /// # Errors
    ///
    /// [`Error::NoHome`] if neither variable is set to anything.
    pub(crate) fn home() -> Result<PathBuf, Error> {
        [HOME, USERPROFILE]
            .into_iter()
            .find_map(|variable| env::var_os(variable).filter(|value| !value.is_empty()))
            .map(PathBuf::from)
            .ok_or(Error::NoHome)
    }

    /// `path` as the manifest spells one: relative to the root, forward slashes,
    /// and `.` for the root itself.
    ///
    /// One line, in one place, so that a root check and every row of an answer
    /// cannot disagree about what a path is called or about what it means for
    /// one to have no name.
    ///
    /// # Errors
    ///
    /// [`Error::Unspellable`] when `path` has no repository-relative form —
    /// which is what a path outside the repository is.
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
