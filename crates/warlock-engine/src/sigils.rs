// The home directory is a parameter here and is never read from the
// environment. Resolving `HOME` inside this module was rejected: it would let a
// test in this crate read or write the developer's real home, and the caller
// that resolved a home is the one that knows which home it means.

use std::ffi::OsStr;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::manifest::{temp_file_name, write_and_sync};

const SIGIL_DIR: &str = ".warlock";

const SIGIL_FILE: &str = "config.toml";

// These four decide the derived directory name, so changing any of them is the
// day everybody's sigils appear to vanish. Move `PROJECT_CONTEXT` only when a
// repository that moved nowhere would derive a different directory anyway;
// there is no migration and there is not meant to be one.
const PROJECT_CONTEXT: &str = "warlock project directory v1 2026-08-28";

const DIGEST_CHARACTERS: usize = 16;

const MAXIMUM_NAME_CHARACTERS: usize = 32;

const UNNAMED_ROOT: &str = "root";

/// ```
/// use warlock_engine::project_directory;
///
/// let root = tempfile::tempdir()?;
/// assert_eq!(project_directory(root.path()), project_directory(root.path()));
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[must_use]
pub fn project_directory(root: impl AsRef<Path>) -> String {
    let root = root.as_ref();
    // Canonicalisation fails on a root deleted mid-run. Panicking or inventing
    // a name were both rejected: digesting the path as handed over keeps this
    // deterministic for a given input, and neither branch can ever derive
    // another checkout's directory.
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

fn project_dir(home: &Path, root: &Path) -> PathBuf {
    home.join(SIGIL_DIR).join(project_directory(root))
}

/// ```
/// use warlock_engine::{sigils, load_sigils};
///
/// let (home, root) = (tempfile::tempdir()?, tempfile::tempdir()?);
/// let held = match load_sigils(home.path(), root.path()) {
///     Err(sigils::Error::NotFound { .. }) => Vec::new(),
///     other => other?,
/// };
///
/// assert!(held.is_empty());
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
// Absent is `NotFound`, following `Manifest::load`. Returning an empty set for
// a missing file was rejected: "never configured for this checkout" and "holds
// no sigils" are different facts, and only the caller knows which it is acting
// on. Unreadable and unparseable stay named for the same reason — broken must
// never be indistinguishable from holds-nothing.
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

/// ```
/// use warlock_engine::{load_sigils, save_sigils};
///
/// let (home, root) = (tempfile::tempdir()?, tempfile::tempdir()?);
/// save_sigils(home.path(), root.path(), &["data-plane".to_owned()])?;
///
/// assert_eq!(load_sigils(home.path(), root.path())?, ["data-plane"]);
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
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
    // The temporary must sit in the same directory as the target, so the rename
    // below cannot cross a filesystem and stops being atomic.
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

// `deny_unknown_fields` rather than a lenient read: this file is short,
// hand-editable and has no version key, so a misspelled `sigil = [...]` would
// otherwise parse as a valid config holding nothing.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Config {
    #[serde(default)]
    sigils: Vec<String>,
}

#[derive(Debug)]
#[non_exhaustive]
pub enum Error {
    NotFound {
        path: PathBuf,
    },
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    Syntax {
        path: PathBuf,
        source: toml::de::Error,
    },
    Serialize {
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

    fn a_dir() -> tempfile::TempDir {
        tempfile::tempdir().expect("a temporary directory")
    }

    fn named(parent: &Path, name: &str) -> PathBuf {
        let path = parent.join(name);
        fs::create_dir_all(&path).expect("creates the directory");
        path
    }

    fn owned(sigils: &[&str]) -> Vec<String> {
        sigils.iter().map(|sigil| (*sigil).to_owned()).collect()
    }

    fn hand_write(home: &Path, root: &Path, text: &str) {
        let path = sigils_path(home, root);
        fs::create_dir_all(path.parent().expect("the config has a directory"))
            .expect("creates the project directory");
        fs::write(&path, text).expect("writes the config");
    }

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
