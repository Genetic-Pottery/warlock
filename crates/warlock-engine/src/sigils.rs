//! Machine-local sigils: `<home>/.warlock/<project>/config.toml`.
//!
//! A scope is a fact about a directory, committed in `.warlock/pacts.toml` and
//! read by everybody who clones the repository. A **sigil** is the other half
//! of that vocabulary and it goes the other way: it is what one person on one
//! machine holds, and it is never written inside a repository. Reversing either
//! home — a scope on a machine, a sigil in a checkout — is the mistake this
//! design is arranged to prevent, so the two live in two modules that share no
//! file, no directory and no writer.
//!
//! This module is the file. It derives the per-repository directory the file
//! sits in ([`project_directory`]), says where the file is
//! ([`sigils_path`]), reads it ([`load_sigils`]) and writes it
//! ([`save_sigils`]). Three properties are worth stating up front:
//!
//! * **The home directory is a parameter, never an environment variable.**
//!   Nothing here reads `HOME`, `USERPROFILE` or anything else: the caller that
//!   knows what a home is resolves one and hands it over. That is what lets
//!   every test in this crate point at a temporary directory, so no test can
//!   read or write the developer's real home, and it keeps the engine's promise
//!   that it does what it was asked and nothing beside.
//! * **Nothing here validates a sigil.** What strings qualify is
//!   [`validate_sigil`](crate::validate_sigil)'s question, asked by whoever
//!   takes the string from a person. This is the store, not the gate: what was
//!   written is read back exactly as written, the way a hand-edited manifest is.
//! * **Absent is not empty.** A missing file is [`Error::NotFound`], following
//!   [`Manifest::load`](crate::Manifest::load)'s precedent, rather than an empty
//!   set invented here — see [`load_sigils`] for why the two are different
//!   facts. A file that exists but cannot be read or parsed is a named error
//!   again, so "broken" is never indistinguishable from "holds nothing".
//!
//! # Two checkouts hold sigils separately
//!
//! `<project>` is derived from the repository root's *path*, so the same
//! repository cloned twice into two directories gets two config files and two
//! sets of sigils. That is the intended behaviour rather than a defect: a sigil
//! is a fact about this checkout on this machine, and recognising "the same
//! repository" across clones would need git remotes, which this crate neither
//! reads nor wants to.

use std::ffi::OsStr;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::manifest::{temp_file_name, write_and_sync};

/// The directory warlock keeps its machine-local bookkeeping in, directly under
/// the home directory. The same name as the repository-side `.warlock/`, which
/// is deliberate: one name for warlock's own files wherever they sit.
const SIGIL_DIR: &str = ".warlock";

/// The file name inside `<home>/.warlock/<project>/`.
const SIGIL_FILE: &str = "config.toml";

/// Domain separation for the project digest, via blake3's key derivation, for
/// the same two reasons [`HASH_CONTEXT`](crate::hash) has one: this digest can
/// never collide with a plain `blake3` of the same bytes computed for something
/// else, and the `v1` is where a change to what goes into it announces itself.
///
/// The rule that governs it: **the version moves when a repository that moved
/// nowhere would derive a different directory**, because that is the day
/// everybody's sigils appear to vanish. There is no migration for it and there
/// is not meant to be one — moving it is re-entering the sigils.
const PROJECT_CONTEXT: &str = "warlock project directory v1 2026-08-28";

/// How many hex characters of the digest go into the directory name.
///
/// Sixteen is 64 bits, which is far more than enough to keep one person's
/// checkouts apart, and short enough that the name still reads as a name in a
/// directory listing rather than as a hash with a word in front of it.
const DIGEST_CHARACTERS: usize = 16;

/// The most characters of the repository's own directory name to keep.
///
/// The name is only there so a person can recognise the directory; the digest
/// is what makes it unique. Capping it keeps the whole segment comfortably
/// inside every filesystem's name limit, which a 255-character repository name
/// plus a digest would not be.
const MAXIMUM_NAME_CHARACTERS: usize = 32;

/// The name used for a root with no file name of its own — `/` and nothing
/// else, in practice. The digest still separates it from anything else.
const UNNAMED_ROOT: &str = "root";

/// The directory name holding one repository's machine-local config: the
/// repository root's own name, a `-`, and a short digest of its canonical
/// absolute path.
///
/// Deterministic: the same root derives the same name on every run, which is
/// what makes the sigils entered on Monday the sigils read on Tuesday. The name
/// alone would collide across two checkouts of the same repository, and the
/// digest alone would be unreadable in a directory listing, so it is both.
///
/// ```
/// use warlock_engine::project_directory;
///
/// let root = tempfile::tempdir()?;
/// assert_eq!(project_directory(root.path()), project_directory(root.path()));
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
///
/// # What goes into the digest
///
/// The canonical absolute path, as text. Canonical, so `/repo`, `/repo/.` and
/// a path reached through a symlink are one checkout rather than three.
/// **Canonicalisation can fail** — the root was deleted while warlock was
/// running, or a component cannot be resolved — and rather than panic or
/// invent, the path as handed over is digested instead. That keeps the answer
/// deterministic for a given input, at the cost of a root that later becomes
/// canonicalisable deriving a different directory than it did while it was
/// gone; the failure that matters, reading somebody else's sigils, cannot
/// happen either way.
///
/// The text is the path's lossy UTF-8 form rather than its platform bytes,
/// because this string becomes a directory name on disk and has to mean the
/// same thing after a toolchain upgrade. The trade is that two paths differing
/// only in bytes that are not valid UTF-8 derive the same directory, which is
/// a pair of checkouts sharing sigils in a case that does not arise in
/// practice — and never a pair that read each other's.
#[must_use]
pub fn project_directory(root: impl AsRef<Path>) -> String {
    let root = root.as_ref();
    let canonical = fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());

    let mut hasher = blake3::Hasher::new_derive_key(PROJECT_CONTEXT);
    hasher.update(canonical.to_string_lossy().as_bytes());
    let digest = hasher.finalize().to_hex();

    format!(
        "{}-{}",
        readable_name(&canonical),
        &digest.as_str()[..DIGEST_CHARACTERS]
    )
}

/// Where one repository's machine-local config lives under `home`:
/// `<home>/.warlock/<project>/config.toml`.
///
/// `home` is the home directory the caller resolved, and `root` the repository
/// root. There is no search and no environment lookup: this is a join and a
/// derivation, and the caller is the one that knows both ends.
///
/// ```
/// use warlock_engine::{project_directory, sigils_path};
///
/// let (home, root) = (tempfile::tempdir()?, tempfile::tempdir()?);
/// assert_eq!(
///     sigils_path(home.path(), root.path()),
///     home.path()
///         .join(".warlock")
///         .join(project_directory(root.path()))
///         .join("config.toml"),
/// );
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[must_use]
pub fn sigils_path(home: impl AsRef<Path>, root: impl AsRef<Path>) -> PathBuf {
    project_dir(home.as_ref(), root.as_ref()).join(SIGIL_FILE)
}

/// The directory the config file sits in: `<home>/.warlock/<project>`.
///
/// Spelled once, here, so [`sigils_path`] and [`save_sigils`] cannot disagree
/// about where the file goes — and so the saver has the directory to create
/// without taking the parent of a path it just built and having to say what it
/// would do if there were none.
fn project_dir(home: &Path, root: &Path) -> PathBuf {
    home.join(SIGIL_DIR).join(project_directory(root))
}

/// Read the sigils held for `root` on this machine, from under `home`.
///
/// The strings come back in file order and exactly as stored: this neither
/// validates them, lower-cases them nor de-duplicates them, so a hand-edited
/// config reads back as what it says. A caller that needs well-formed sigils
/// asks [`validate_sigil`](crate::validate_sigil) about what it gets.
///
/// **A missing file is [`Error::NotFound`], not an empty set.** The precedent
/// is [`Manifest::load`](crate::Manifest::load) and so is the reasoning: "this
/// machine has never been configured for this checkout" and "this machine holds
/// no sigils here" are different facts, and only the caller knows which of them
/// it is acting on. Both read as *no sigils held* for the two callers there are
/// today, and both are one line to write:
///
/// ```
/// use warlock_engine::{SigilError, load_sigils};
///
/// let (home, root) = (tempfile::tempdir()?, tempfile::tempdir()?);
/// let held = match load_sigils(home.path(), root.path()) {
///     Err(SigilError::NotFound { .. }) => Vec::new(),
///     other => other?,
/// };
///
/// assert!(held.is_empty());
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
///
/// # Errors
///
/// * [`Error::NotFound`] if there is no config file for this repository.
/// * [`Error::Io`] if there is one and it cannot be read.
/// * [`Error::Syntax`] if it can be read but is not a config: not TOML, a
///   `sigils` key of the wrong type, or a key this build does not know, which
///   is refused rather than ignored so that a misspelled `sigil = [...]` is a
///   line to go and fix instead of a silently empty set.
///
/// A file that is readable TOML and simply has no `sigils` key holds no
/// sigils — that is a config, not a broken one, and it reads as an empty set.
pub fn load_sigils(home: impl AsRef<Path>, root: impl AsRef<Path>) -> Result<Vec<String>, Error> {
    let path = sigils_path(home, root);
    match fs::read_to_string(&path) {
        Ok(text) => match toml::from_str::<Config>(&text) {
            Ok(config) => Ok(config.sigils),
            Err(source) => Err(Error::Syntax { path, source }),
        },
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => {
            Err(Error::NotFound { path })
        }
        Err(source) => Err(Error::Io { path, source }),
    }
}

/// Write `sigils` as the set held for `root` on this machine, under `home`,
/// atomically.
///
/// The set is replaced, not added to: what is passed is what the file says
/// afterwards, and an empty slice writes `sigils = []` rather than deleting
/// anything. The directory is created if it is not there.
///
/// Atomic in the manifest's sense, and by the manifest's idiom: the text goes
/// to a temporary file *in the same directory* as the target, is flushed to the
/// disk, and is then renamed over it, so a reader sees either the whole old
/// file or the whole new one. On success no temporary is left behind; on
/// failure it is cleaned up on a best-effort basis.
///
/// This writes one file under `home` and touches nothing else. Nothing here
/// writes, or offers to write, anything inside the repository.
///
/// ```
/// use warlock_engine::{load_sigils, save_sigils};
///
/// let (home, root) = (tempfile::tempdir()?, tempfile::tempdir()?);
/// save_sigils(home.path(), root.path(), &["data-plane".to_owned()])?;
///
/// assert_eq!(load_sigils(home.path(), root.path())?, ["data-plane"]);
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
///
/// # Errors
///
/// [`Error::Serialize`] if the set cannot be written as TOML, or [`Error::Io`]
/// naming the path that failed if the directory cannot be created, the
/// temporary cannot be written, or the rename fails.
pub fn save_sigils(
    home: impl AsRef<Path>,
    root: impl AsRef<Path>,
    sigils: &[String],
) -> Result<(), Error> {
    // Serialise before touching the filesystem: a set that cannot be written as
    // TOML should not leave a new directory behind.
    let text = toml::to_string(&Config {
        sigils: sigils.to_vec(),
    })
    .map_err(|source| Error::Serialize { source })?;

    let dir = project_dir(home.as_ref(), root.as_ref());
    fs::create_dir_all(&dir).map_err(|source| Error::Io {
        path: dir.clone(),
        source,
    })?;

    let target = dir.join(SIGIL_FILE);
    let temp = dir.join(temp_file_name(SIGIL_FILE));
    if let Err(source) = write_and_sync(&temp, text.as_bytes()) {
        drop(fs::remove_file(&temp));
        return Err(Error::Io { path: temp, source });
    }

    if let Err(source) = fs::rename(&temp, &target) {
        drop(fs::remove_file(&temp));
        return Err(Error::Io {
            path: target,
            source,
        });
    }
    Ok(())
}

/// The repository root's own name, as one portable path segment.
///
/// Recognisability is the whole job — the digest beside it is what makes the
/// directory unique — so anything that is not an ASCII letter, digit, `-`, `_`
/// or `.` becomes a `-` rather than reaching a filesystem, and the result is
/// capped at [`MAXIMUM_NAME_CHARACTERS`]. Substituting rather than dropping
/// keeps it deterministic and keeps the name the same length as what it stands
/// for.
fn readable_name(canonical: &Path) -> String {
    let name = canonical.file_name().map_or_else(
        || UNNAMED_ROOT.to_owned(),
        |name| OsStr::to_string_lossy(name).into_owned(),
    );

    name.chars()
        .take(MAXIMUM_NAME_CHARACTERS)
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.') {
                character
            } else {
                '-'
            }
        })
        .collect()
}

/// The file's shape: a list of sigils and nothing else.
///
/// `deny_unknown_fields` because the alternative is worse than strictness here.
/// This file is short, hand-editable and has no version key, so a misspelled
/// `sigil = [...]` would otherwise read as a valid config holding nothing —
/// exactly the "false empty" the three-state read is arranged to avoid.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Config {
    /// The sigils held, in file order and exactly as written. Absent means an
    /// empty set: a config that says nothing about sigils holds none.
    #[serde(default)]
    sigils: Vec<String>,
}

/// Everything that can stop the machine-local sigils being read or written.
///
/// Hand-rolled like [`ManifestError`](crate::ManifestError) and
/// [`HashError`](crate::HashError): four variants do not pay for an
/// error-handling dependency, and each one names the path it happened to, so a
/// caller has one line to print and a file to go and look at.
#[derive(Debug)]
#[non_exhaustive]
pub enum Error {
    /// There is no config file for this repository on this machine.
    /// Distinguishes a checkout that was never configured from one whose
    /// config is corrupt.
    NotFound {
        /// The path that was looked for.
        path: PathBuf,
    },
    /// The config file could not be read or written.
    Io {
        /// The path being read or written.
        path: PathBuf,
        /// What the filesystem said.
        source: std::io::Error,
    },
    /// The file is there but is not a config: not TOML, or not this shape.
    Syntax {
        /// The file that could not be understood.
        path: PathBuf,
        /// What the TOML parser said, including where.
        source: toml::de::Error,
    },
    /// The sigils could not be turned into TOML.
    Serialize {
        /// What the TOML serialiser said.
        source: toml::ser::Error,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound { path } => {
                write!(f, "no sigil config at `{}`", path.display())
            }
            Self::Io { path, source } => {
                write!(f, "could not read or write `{}`: {source}", path.display())
            }
            Self::Syntax { path, source } => write!(
                f,
                "malformed sigil config at `{}`: {source}",
                path.display()
            ),
            Self::Serialize { source } => {
                write!(f, "could not write the sigil config as TOML: {source}")
            }
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::Syntax { source, .. } => Some(source),
            Self::Serialize { source } => Some(source),
            Self::NotFound { .. } => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};

    use super::{
        DIGEST_CHARACTERS, Error, MAXIMUM_NAME_CHARACTERS, SIGIL_FILE, load_sigils,
        project_directory, save_sigils, sigils_path,
    };

    /// A throwaway directory. Every test builds both its home *and* its
    /// repository root out of these, so nothing in this module reads or writes
    /// the developer's real home — which is the whole reason the home is a
    /// parameter rather than an environment variable.
    fn a_dir() -> tempfile::TempDir {
        tempfile::tempdir().expect("a temporary directory")
    }

    /// A directory called `name` inside `parent`, so a test can say something
    /// about the derived name rather than about whatever `tempfile` picked.
    fn named(parent: &Path, name: &str) -> PathBuf {
        let path = parent.join(name);
        fs::create_dir_all(&path).expect("creates the directory");
        path
    }

    /// Sigils as a caller of [`save_sigils`] holds them.
    fn owned(sigils: &[&str]) -> Vec<String> {
        sigils.iter().map(|sigil| (*sigil).to_owned()).collect()
    }

    /// Write `text` as the config for `root` under `home` without going through
    /// [`save_sigils`], the way a person with an editor would.
    fn hand_write(home: &Path, root: &Path, text: &str) {
        let path = sigils_path(home, root);
        fs::create_dir_all(path.parent().expect("the config has a directory"))
            .expect("creates the project directory");
        fs::write(&path, text).expect("writes the config");
    }

    /// The file names directly inside the project directory, sorted.
    fn project_dir_listing(home: &Path, root: &Path) -> Vec<String> {
        let dir = sigils_path(home, root);
        let dir = dir.parent().expect("the config has a directory");
        let mut names: Vec<String> = fs::read_dir(dir)
            .expect("the directory a save just created")
            .map(|entry| {
                entry
                    .expect("a readable entry")
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        names.sort();
        names
    }

    #[test]
    fn the_same_root_derives_the_same_directory_every_time() {
        let home = a_dir();
        let root = named(home.path(), "warlock");

        let derived = project_directory(&root);
        assert_eq!(derived, project_directory(&root), "byte for byte");
        assert_eq!(derived, project_directory(root.join(".")), "and again");

        let (name, digest) = derived
            .rsplit_once('-')
            .expect("a name, a dash and a digest");
        assert_eq!(name, "warlock", "the directory's own name, to read it by");
        assert_eq!(digest.len(), DIGEST_CHARACTERS);
        assert!(
            digest
                .chars()
                .all(|c| c.is_ascii_hexdigit() && !c.is_uppercase()),
            "{digest}"
        );
    }

    #[test]
    fn two_checkouts_with_the_same_name_derive_different_directories() {
        let (one, two) = (a_dir(), a_dir());
        let (here, there) = (named(one.path(), "warlock"), named(two.path(), "warlock"));
        assert_ne!(here, there, "two genuinely different absolute paths");

        assert!(project_directory(&here).starts_with("warlock-"));
        assert!(project_directory(&there).starts_with("warlock-"));
        assert_ne!(
            project_directory(&here),
            project_directory(&there),
            "the name alone collides across checkouts, which is what the \
             digest of the absolute path is for"
        );

        // And the consequence that matters: neither can read the other's file.
        let home = a_dir();
        save_sigils(home.path(), &here, &owned(&["data-plane"])).expect("saves");
        assert!(matches!(
            load_sigils(home.path(), &there),
            Err(Error::NotFound { .. })
        ));
    }

    #[test]
    fn a_root_that_cannot_be_canonicalised_still_derives_deterministically() {
        let home = a_dir();
        let gone = home.path().join("deleted-mid-run");

        assert_eq!(
            project_directory(&gone),
            project_directory(&gone),
            "the path as handed over is digested instead, rather than a panic"
        );
        assert_ne!(
            project_directory(&gone),
            project_directory(home.path().join("a-different-one")),
            "and it is still a derivation, not a constant"
        );
    }

    #[test]
    fn the_derived_directory_is_one_readable_path_segment() {
        let home = a_dir();
        let awkward = named(home.path(), "my repo (2)!");
        let derived = project_directory(&awkward);

        assert!(derived.starts_with("my-repo--2--"), "{derived}");
        assert_eq!(
            Path::new(&derived).components().count(),
            1,
            "whatever a repository is called, this is one directory name: {derived}"
        );

        let long = named(home.path(), &"a".repeat(MAXIMUM_NAME_CHARACTERS + 40));
        let derived = project_directory(&long);
        assert_eq!(
            derived.len(),
            MAXIMUM_NAME_CHARACTERS + 1 + DIGEST_CHARACTERS,
            "the name is capped so the segment fits a filesystem: {derived}"
        );
    }

    #[test]
    fn the_config_sits_under_the_home_it_was_given() {
        let (home, elsewhere) = (a_dir(), a_dir());
        let root = named(elsewhere.path(), "warlock");

        assert_eq!(
            sigils_path(home.path(), &root),
            home.path()
                .join(".warlock")
                .join(project_directory(&root))
                .join(SIGIL_FILE),
        );
        assert!(
            sigils_path(home.path(), &root).starts_with(home.path()),
            "the home is a parameter, and it is the only place written to"
        );
    }

    #[test]
    fn saving_then_loading_gives_the_set_back() {
        let (home, elsewhere) = (a_dir(), a_dir());
        let root = named(elsewhere.path(), "warlock");

        for set in [vec![], owned(&["data-plane"]), owned(&["billing", "web"])] {
            save_sigils(home.path(), &root, &set).expect("saves");
            assert_eq!(
                load_sigils(home.path(), &root).expect("loads"),
                set,
                "a set replaces whatever was held, empty included"
            );
        }
    }

    #[test]
    fn the_file_holds_a_sigils_key_and_nothing_else() {
        let (home, elsewhere) = (a_dir(), a_dir());
        let root = named(elsewhere.path(), "warlock");

        save_sigils(home.path(), &root, &owned(&["billing", "web"])).expect("saves");
        assert_eq!(
            fs::read_to_string(sigils_path(home.path(), &root)).expect("reads"),
            "sigils = [\"billing\", \"web\"]\n",
        );

        save_sigils(home.path(), &root, &[]).expect("saves an empty set");
        assert_eq!(
            fs::read_to_string(sigils_path(home.path(), &root)).expect("reads"),
            "sigils = []\n",
            "an empty set is written as one, not as an absent key or a deleted file"
        );
    }

    #[test]
    fn saving_creates_the_directory_and_leaves_no_temporary_behind() {
        let (home, elsewhere) = (a_dir(), a_dir());
        let root = named(elsewhere.path(), "warlock");
        assert!(!home.path().join(".warlock").exists(), "nothing there yet");

        save_sigils(home.path(), &root, &owned(&["data-plane"])).expect("saves");
        assert_eq!(project_dir_listing(home.path(), &root), [SIGIL_FILE]);

        // And a second save renames over the first rather than accumulating.
        save_sigils(home.path(), &root, &owned(&["billing"])).expect("saves again");
        assert_eq!(project_dir_listing(home.path(), &root), [SIGIL_FILE]);
        assert_eq!(load_sigils(home.path(), &root).expect("loads"), ["billing"]);
    }

    #[test]
    fn nothing_is_written_inside_the_repository() {
        let (home, elsewhere) = (a_dir(), a_dir());
        let root = named(elsewhere.path(), "warlock");

        save_sigils(home.path(), &root, &owned(&["data-plane"])).expect("saves");
        load_sigils(home.path(), &root).expect("loads");

        assert_eq!(
            fs::read_dir(&root).expect("reads the repository").count(),
            0,
            "a sigil is a fact about a machine, and never a file in a checkout"
        );
    }

    #[test]
    fn a_missing_config_is_not_found_rather_than_an_empty_set() {
        let (home, elsewhere) = (a_dir(), a_dir());
        let root = named(elsewhere.path(), "warlock");

        match load_sigils(home.path(), &root) {
            Err(Error::NotFound { path }) => assert_eq!(path, sigils_path(home.path(), &root)),
            other => panic!("expected a not-found error, got {other:?}"),
        }

        // A home with no `.warlock` at all is the same answer, not an I/O error
        // about the missing parent.
        assert!(matches!(
            load_sigils(home.path().join("nowhere"), &root),
            Err(Error::NotFound { .. })
        ));

        // And it stays distinguishable from the set that really is empty.
        save_sigils(home.path(), &root, &[]).expect("saves");
        assert_eq!(
            load_sigils(home.path(), &root).expect("loads"),
            Vec::<String>::new()
        );
    }

    #[test]
    fn a_malformed_config_is_an_error_and_never_a_panic() {
        let (home, elsewhere) = (a_dir(), a_dir());
        let root = named(elsewhere.path(), "warlock");

        for text in [
            "this is not toml\n",
            "sigils = 7\n",
            "sigils = [1, 2]\n",
            // The misspelling `deny_unknown_fields` exists to catch: read
            // leniently, this would be a config holding nothing.
            "sigil = [\"data-plane\"]\n",
        ] {
            hand_write(home.path(), &root, text);
            match load_sigils(home.path(), &root) {
                Err(error @ Error::Syntax { .. }) => assert!(
                    error.to_string().contains(SIGIL_FILE),
                    "the message names the file to go and fix: {error}"
                ),
                other => panic!("expected a syntax error for `{text}`, got {other:?}"),
            }
        }

        // Broken is never absent, and never an empty set either.
        hand_write(home.path(), &root, "sigils = [\"data-plane\"]\n");
        assert_eq!(
            load_sigils(home.path(), &root).expect("loads"),
            ["data-plane"]
        );
    }

    #[test]
    fn a_config_that_says_nothing_about_sigils_holds_none() {
        let (home, elsewhere) = (a_dir(), a_dir());
        let root = named(elsewhere.path(), "warlock");
        hand_write(home.path(), &root, "\n# nothing here yet\n");

        assert_eq!(
            load_sigils(home.path(), &root).expect("loads"),
            Vec::<String>::new()
        );
    }

    #[test]
    fn sigils_are_stored_and_read_back_exactly_as_written() {
        // No folding, no trimming, no rejection, no de-duplication: this is the
        // store, not the gate. `validate_sigil` is asked where a person types.
        let (home, elsewhere) = (a_dir(), a_dir());
        let root = named(elsewhere.path(), "warlock");
        let odd = owned(&["Data-Plane", " billing ", "*", "", "web", "web"]);

        save_sigils(home.path(), &root, &odd).expect("saves");
        assert_eq!(load_sigils(home.path(), &root).expect("loads"), odd);
    }

    #[test]
    fn every_error_variant_says_what_happened_and_where() {
        let not_found = Error::NotFound {
            path: PathBuf::from("/home/someone/.warlock/warlock-abc/config.toml"),
        };
        assert_eq!(
            not_found.to_string(),
            "no sigil config at `/home/someone/.warlock/warlock-abc/config.toml`"
        );

        let io = Error::Io {
            path: PathBuf::from("/home/someone/.warlock"),
            source: std::io::Error::new(std::io::ErrorKind::PermissionDenied, "denied"),
        };
        assert_eq!(
            io.to_string(),
            "could not read or write `/home/someone/.warlock`: denied"
        );

        let syntax = Error::Syntax {
            path: PathBuf::from("/home/someone/.warlock/warlock-abc/config.toml"),
            source: toml::from_str::<super::Config>("sigils = 7")
                .expect_err("a number is not a list"),
        };
        assert!(
            syntax.to_string().starts_with(
                "malformed sigil config at `/home/someone/.warlock/warlock-abc/config.toml`: "
            ),
            "{syntax}"
        );
    }

    #[test]
    fn errors_expose_the_cause_they_wrap() {
        use std::error::Error as _;

        assert!(
            Error::Io {
                path: PathBuf::from("x"),
                source: std::io::Error::other("boom"),
            }
            .source()
            .is_some()
        );
        assert!(
            Error::Syntax {
                path: PathBuf::from("x"),
                source: toml::from_str::<super::Config>("sigils = 7")
                    .expect_err("a number is not a list"),
            }
            .source()
            .is_some()
        );
        assert!(
            Error::NotFound {
                path: PathBuf::from("x")
            }
            .source()
            .is_none()
        );
    }

    /// Only on unix, because the fixture needs `std::os::unix::fs::symlink` to
    /// reach one directory by two paths at all.
    #[cfg(unix)]
    #[test]
    fn two_paths_to_one_checkout_derive_one_directory() {
        let home = a_dir();
        let root = named(home.path(), "warlock");
        let link = home.path().join("shortcut");
        std::os::unix::fs::symlink(&root, &link).expect("links to the checkout");

        assert_eq!(
            project_directory(&link),
            project_directory(&root),
            "canonicalising is what makes one checkout one project directory"
        );
    }

    /// Chmod cannot deny root anything, so the test checks the directory really
    /// is unwritable before asserting on it and steps aside when it is not.
    #[cfg(unix)]
    #[test]
    fn a_save_that_fails_leaves_no_temporary_and_no_half_written_file() {
        use std::os::unix::fs::PermissionsExt as _;

        let (home, elsewhere) = (a_dir(), a_dir());
        let root = named(elsewhere.path(), "warlock");
        save_sigils(home.path(), &root, &owned(&["data-plane"])).expect("saves");

        let config = sigils_path(home.path(), &root);
        let dir = config
            .parent()
            .expect("the config has a directory")
            .to_path_buf();
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o555)).expect("chmods");
        if fs::File::create(dir.join("probe")).is_ok() {
            // Running as root: there is no such thing as an unwritable
            // directory here, so there is nothing to assert against.
            fs::remove_file(dir.join("probe")).expect("removes the probe");
            fs::set_permissions(&dir, fs::Permissions::from_mode(0o755)).expect("chmods back");
            return;
        }

        let error =
            save_sigils(home.path(), &root, &owned(&["billing"])).expect_err("cannot be written");
        assert!(matches!(error, Error::Io { .. }), "{error:?}");

        fs::set_permissions(&dir, fs::Permissions::from_mode(0o755)).expect("chmods back");
        assert_eq!(
            project_dir_listing(home.path(), &root),
            [SIGIL_FILE],
            "the temporary is cleaned up rather than left beside the config"
        );
        assert_eq!(
            load_sigils(home.path(), &root).expect("loads"),
            ["data-plane"],
            "and the set that was held is the set that is still held"
        );
    }
}
