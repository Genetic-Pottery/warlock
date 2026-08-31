//! Where a repository wants its briefs written: `.warlock/briefs.toml`.
//!
//! A brief is a document a person asked for, and the directory it lands in is
//! that repository's own convention — `docs/`, `plans/`, `notes/adr/`. So it is
//! one key in one small file, hand-written and committed beside the pact
//! manifest, and this module is the whole of reading it: [`briefs_path`] says
//! where it is and [`load_briefs`] answers with the directory that was asked
//! for. Four properties are worth stating up front, because everything below
//! follows from them:
//!
//! * **Absent is the default, and silently so.** A repository that never wrote
//!   the file has not failed at anything; it gets
//!   [`DEFAULT_BRIEF_DIRECTORY`] and no error. This is the one deliberate
//!   departure from [`Manifest::load`](crate::Manifest::load)'s absent-is-not-
//!   empty rule, and it is a departure because there is no second fact to tell
//!   apart here: an optional setting that was never set *is* the default.
//! * **Empty is the default too, written down rather than fallen into.** A file
//!   that parses and carries no `directory` — a zero-byte file, a file holding
//!   only a comment — is a statement that the repository has no preference, and
//!   the default is filled in at the point the file is deserialised. An empty
//!   statement is not a fault.
//! * **Broken is refused out loud.** Text that is not TOML, a key this build
//!   does not know, or a `directory` that is not a string is [`Error::Syntax`]
//!   naming the file and quoting the parser, never a quiet fall back to the
//!   default — the misspelling `directroy = "plans"` has to be a line to go and
//!   fix, not a silent write into `docs/`.
//! * **The value is a relative path, and that is a guardrail rather than a
//!   sandbox.** An absolute path is refused because this file is committed, and
//!   a `..` component is refused as a component, without canonicalising and
//!   without a containment check afterwards. Nothing here resolves a symlink,
//!   and a symlink inside the tree still goes wherever it goes.
//!
//! Nothing in this module writes, creates or renames anything: `briefs.toml` is
//! a file a person wrote, and warlock never produces one.

use std::fmt;
use std::fs;
use std::path::{Component, Path, PathBuf};

use serde::Deserialize;

use crate::manifest::manifest_path;

/// The file's name, inside the same directory the pact manifest lives in.
const BRIEFS_FILE: &str = "briefs.toml";

/// The directory briefs go in when the repository has not said otherwise,
/// relative to the repository root.
///
/// Spelled once, here, so the loader's answer and whatever a caller would have
/// hard-coded cannot become two different directories.
pub const DEFAULT_BRIEF_DIRECTORY: &str = "docs";

/// Where the brief settings live under `root`: `<root>/.warlock/briefs.toml`.
///
/// `root` is the parent of `.warlock/`, i.e. the repository root, exactly as
/// for [`manifest_path`](crate::manifest_path). There is no search: this is a
/// join, and the caller is the one that knows the root.
///
/// ```
/// use std::path::Path;
/// use warlock_engine::briefs_path;
///
/// assert_eq!(
///     briefs_path("/repo"),
///     Path::new("/repo").join(".warlock").join("briefs.toml"),
/// );
/// ```
#[must_use]
pub fn briefs_path(root: impl AsRef<Path>) -> PathBuf {
    // Built off the manifest's path rather than joining `.warlock` a second
    // time: the directory these two files share is named once in this crate,
    // in `manifest.rs`, so they cannot drift apart.
    manifest_path(root).with_file_name(BRIEFS_FILE)
}

/// The directory `root` asks its briefs to be written in, relative to `root`.
///
/// The answer is [`DEFAULT_BRIEF_DIRECTORY`] where the file is not there or
/// says nothing about a directory, and otherwise the value exactly as written
/// — this neither joins it onto `root`, creates it, nor asks whether it exists.
/// What it does ask is that the value is a relative path with no `..` in it,
/// which is the one thing that cannot wait until a document is finished and
/// there is nowhere to put it.
///
/// ```
/// use warlock_engine::{DEFAULT_BRIEF_DIRECTORY, load_briefs};
///
/// let root = tempfile::tempdir()?;
/// assert_eq!(load_briefs(root.path())?, DEFAULT_BRIEF_DIRECTORY);
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
///
/// # Errors
///
/// * [`Error::Io`] if the file is there and cannot be read. A file that is not
///   there is not an error: it is the default.
/// * [`Error::Syntax`] if it can be read but is not this file: not TOML, a key
///   this build does not know, or a `directory` that is not a string.
/// * [`Error::AbsoluteDirectory`] if `directory` is an absolute path, even one
///   pointing inside this repository.
/// * [`Error::ParentDirectory`] if any component of `directory` is `..`.
pub fn load_briefs(root: impl AsRef<Path>) -> Result<String, Error> {
    let path = briefs_path(root);

    let text = match fs::read_to_string(&path) {
        Ok(text) => text,
        // No file at all — and no `.warlock` at all — is the default, not a
        // fault: this setting is optional and never having set it is an answer.
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => {
            return Ok(DEFAULT_BRIEF_DIRECTORY.to_owned());
        }
        Err(source) => return Err(Error::Io { path, source }),
    };

    let briefs: Briefs = toml::from_str(&text).map_err(|source| Error::Syntax {
        path: path.clone(),
        source,
    })?;

    check_relative(&path, &briefs.directory)?;
    Ok(briefs.directory)
}

/// Refuse a `directory` that is not a plain relative path.
///
/// Walked as components, and judged as components: `docs/../plans` is refused
/// because one of its components is `..`, not because normalising it would
/// leave the repository. There is no canonicalisation and no containment check
/// on the far side of one, so nothing here follows a symlink or touches the
/// filesystem at all — this catches a mistake in a committed file, and is not
/// a boundary anything is secured by.
fn check_relative(path: &Path, directory: &str) -> Result<(), Error> {
    for component in Path::new(directory).components() {
        match component {
            // `RootDir` and `Prefix` rather than `Path::is_absolute`, so that a
            // rooted path is refused on every platform the same way.
            Component::RootDir | Component::Prefix(_) => {
                return Err(Error::AbsoluteDirectory {
                    path: path.to_path_buf(),
                    directory: directory.to_owned(),
                });
            }
            Component::ParentDir => {
                return Err(Error::ParentDirectory {
                    path: path.to_path_buf(),
                    directory: directory.to_owned(),
                });
            }
            Component::CurDir | Component::Normal(_) => {}
        }
    }
    Ok(())
}

/// The file's shape: a directory and nothing else.
///
/// `deny_unknown_fields` for the reason the sigil config has it — this file is
/// short, hand-edited and has no version key, so `directroy = "plans"` read
/// leniently would be a valid file expressing nothing, and the brief would go
/// somewhere its author never asked for.
///
/// The absent key is filled in by [`default_directory`] rather than becoming an
/// `Option` for the loader to unwrap: "this repository has no preference" is a
/// decision with an answer, and the answer is written down where the shape of
/// the file is.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Briefs {
    /// The directory briefs are written in, relative to the repository root.
    #[serde(default = "default_directory")]
    directory: String,
}

/// [`DEFAULT_BRIEF_DIRECTORY`] as an owned string, for `serde`'s `default`.
fn default_directory() -> String {
    DEFAULT_BRIEF_DIRECTORY.to_owned()
}

/// Everything that can stop the brief directory being read.
///
/// Hand-rolled like [`ManifestError`](crate::ManifestError) and
/// [`SigilError`](crate::SigilError): four variants do not pay for an
/// error-handling dependency, and each one names the file it happened to, so a
/// caller has one line to say and a file to go and look at.
///
/// A missing file is deliberately not in here — see [`load_briefs`].
#[derive(Debug)]
#[non_exhaustive]
pub enum Error {
    /// The file is there and could not be read.
    Io {
        /// The path being read.
        path: PathBuf,
        /// What the filesystem said.
        source: std::io::Error,
    },
    /// The file is there but is not this file: not TOML, an unknown key, or a
    /// `directory` of the wrong type.
    Syntax {
        /// The file that could not be understood.
        path: PathBuf,
        /// What the TOML parser said, including where.
        source: toml::de::Error,
    },
    /// `directory` is an absolute path.
    AbsoluteDirectory {
        /// The file that says so.
        path: PathBuf,
        /// The value it says, as written.
        directory: String,
    },
    /// `directory` has a `..` component.
    ParentDirectory {
        /// The file that says so.
        path: PathBuf,
        /// The value it says, as written.
        directory: String,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, source } => {
                write!(f, "could not read `{}`: {source}", path.display())
            }
            Self::Syntax { path, source } => {
                write!(
                    f,
                    "malformed brief config at `{}`: {source}",
                    path.display()
                )
            }
            Self::AbsoluteDirectory { path, directory } => write!(
                f,
                "`{}` sets directory = \"{directory}\", which is an absolute path: \
                 briefs.toml is committed, and an absolute path is a fact about one \
                 machine that resolves to nothing on a colleague's clone",
                path.display()
            ),
            Self::ParentDirectory { path, directory } => write!(
                f,
                "`{}` sets directory = \"{directory}\", which has a `..` component: \
                 the brief directory is written relative to the repository root, and \
                 this is a guardrail against a mistake rather than a sandbox",
                path.display()
            ),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::Syntax { source, .. } => Some(source),
            Self::AbsoluteDirectory { .. } | Self::ParentDirectory { .. } => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};

    use super::{BRIEFS_FILE, Briefs, DEFAULT_BRIEF_DIRECTORY, Error, briefs_path, load_briefs};

    /// A throwaway directory to stand in for a repository root. Each test gets
    /// its own, so the suite stays parallel-safe and leaves nothing behind.
    fn a_root() -> tempfile::TempDir {
        tempfile::tempdir().expect("a temporary directory")
    }

    /// Write `text` to `<root>/.warlock/briefs.toml`, the way a person with an
    /// editor would — which is the only way this file is ever written.
    fn hand_write(root: &Path, text: &str) {
        let path = briefs_path(root);
        fs::create_dir_all(path.parent().expect("the file has a directory"))
            .expect("creates .warlock");
        fs::write(&path, text).expect("writes briefs.toml");
    }

    /// A parser error to hang the `Display` and `source` tests off, since one
    /// cannot be constructed directly.
    fn a_de_error() -> toml::de::Error {
        toml::from_str::<Briefs>("directory = 7").expect_err("a number is not a string")
    }

    #[test]
    fn the_file_sits_beside_the_manifest() {
        assert_eq!(
            briefs_path("/repo"),
            Path::new("/repo").join(".warlock").join(BRIEFS_FILE),
        );
        assert_eq!(
            briefs_path("/repo").parent(),
            warlock_engine_manifest_dir().as_deref(),
            "one `.warlock`, named once, for both files"
        );
    }

    /// The directory the manifest lives in, asked of the manifest itself rather
    /// than spelled again here.
    fn warlock_engine_manifest_dir() -> Option<PathBuf> {
        crate::manifest_path("/repo")
            .parent()
            .map(Path::to_path_buf)
    }

    #[test]
    fn no_file_at_all_is_the_default() {
        let root = a_root();
        assert_eq!(
            load_briefs(root.path()).expect("an absent file is not a fault"),
            DEFAULT_BRIEF_DIRECTORY,
        );
        assert_eq!(DEFAULT_BRIEF_DIRECTORY, "docs");

        // A root that does not exist at all reads the same way, rather than as
        // an I/O error about the missing parent.
        assert_eq!(
            load_briefs(root.path().join("nowhere")).expect("still the default"),
            DEFAULT_BRIEF_DIRECTORY,
        );
    }

    #[test]
    fn a_file_that_says_nothing_about_a_directory_is_the_default() {
        for text in ["", "\n", "# nothing decided yet\n"] {
            let root = a_root();
            hand_write(root.path(), text);
            assert_eq!(
                load_briefs(root.path())
                    .unwrap_or_else(|error| panic!("`{text}` is not a fault, got {error:?}")),
                DEFAULT_BRIEF_DIRECTORY,
                "an empty statement is not a fault: {text:?}"
            );
        }
    }

    #[test]
    fn a_directory_is_answered_exactly_as_written() {
        for value in ["plans", "docs", "notes/adr", "./plans", "a..b", "..hidden"] {
            let root = a_root();
            hand_write(root.path(), &format!("directory = \"{value}\"\n"));
            assert_eq!(
                load_briefs(root.path())
                    .unwrap_or_else(|error| panic!("`{value}` is a directory, got {error:?}")),
                value,
                "not joined onto the root, not normalised, not created"
            );
        }
    }

    #[test]
    fn text_that_is_not_this_file_is_a_syntax_error_naming_it() {
        for text in [
            "this is not toml\n",
            // The misspelling `deny_unknown_fields` exists to catch: read
            // leniently, this is a file that quietly means `docs`.
            "directroy = \"plans\"\n",
            "directory = 7\n",
            "directory = [\"plans\"]\n",
            "directory = \"plans\"\nextra = true\n",
        ] {
            let root = a_root();
            hand_write(root.path(), text);
            match load_briefs(root.path()) {
                Err(error @ Error::Syntax { .. }) => assert!(
                    error.to_string().contains(BRIEFS_FILE),
                    "the message names the file to go and fix: {error}"
                ),
                other => panic!("expected a syntax error for `{text}`, got {other:?}"),
            }
        }
    }

    #[test]
    fn the_unknown_key_error_names_the_offending_key() {
        let root = a_root();
        hand_write(root.path(), "directroy = \"plans\"\n");

        let error = load_briefs(root.path()).expect_err("a misspelled key is a fault");
        assert!(
            error.to_string().contains("directroy"),
            "the parser's own reason points at the key: {error}"
        );
    }

    #[test]
    fn an_absolute_directory_is_refused_even_inside_this_repository() {
        let root = a_root();
        let inside = root.path().join("docs");
        hand_write(
            root.path(),
            &format!("directory = \"{}\"\n", inside.display()),
        );

        match load_briefs(root.path()) {
            Err(error @ Error::AbsoluteDirectory { .. }) => {
                let said = error.to_string();
                assert!(said.contains(BRIEFS_FILE), "{said}");
                assert!(said.contains("absolute path"), "{said}");
                assert!(
                    said.contains("committed") && said.contains("clone"),
                    "the refusal says why a committed file may not carry one \
                     machine's path: {said}"
                );
            }
            other => panic!("expected an absolute-path error, got {other:?}"),
        }
    }

    #[test]
    fn a_parent_component_is_refused_as_a_component() {
        for value in ["../plans", "docs/../plans", "..", "docs/.."] {
            let root = a_root();
            hand_write(root.path(), &format!("directory = \"{value}\"\n"));

            match load_briefs(root.path()) {
                Err(error @ Error::ParentDirectory { .. }) => {
                    let said = error.to_string();
                    assert!(said.contains(BRIEFS_FILE), "{said}");
                    assert!(
                        said.contains("guardrail") && said.contains("sandbox"),
                        "the refusal says what it is and is not: {said}"
                    );
                }
                other => panic!("expected a `..` error for `{value}`, got {other:?}"),
            }
        }
    }

    #[test]
    fn reading_creates_nothing() {
        let root = a_root();

        // With no file: not even the directory is brought into being.
        load_briefs(root.path()).expect("the default");
        assert!(
            !root.path().join(".warlock").exists(),
            "a read that found nothing left nothing behind"
        );

        // And with one: the file a person wrote is the only file there, byte
        // for byte, before and after every kind of read.
        let text = "directory = \"plans\"\n";
        hand_write(root.path(), text);
        load_briefs(root.path()).expect("plans");
        hand_write(root.path(), "directory = \"../plans\"\n");
        drop(load_briefs(root.path()));

        let listing: Vec<String> = fs::read_dir(root.path().join(".warlock"))
            .expect("the directory the fixture created")
            .map(|entry| {
                entry
                    .expect("a readable entry")
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        assert_eq!(
            listing,
            [BRIEFS_FILE],
            "no manifest, no temporary, no backup"
        );
    }

    #[test]
    fn every_error_variant_says_what_happened_and_where() {
        let path = PathBuf::from("/repo/.warlock/briefs.toml");

        let io = Error::Io {
            path: path.clone(),
            source: std::io::Error::new(std::io::ErrorKind::PermissionDenied, "denied"),
        };
        assert_eq!(
            io.to_string(),
            "could not read `/repo/.warlock/briefs.toml`: denied"
        );

        let syntax = Error::Syntax {
            path: path.clone(),
            source: a_de_error(),
        };
        assert!(
            syntax
                .to_string()
                .starts_with("malformed brief config at `/repo/.warlock/briefs.toml`: "),
            "{syntax}"
        );

        let absolute = Error::AbsoluteDirectory {
            path: path.clone(),
            directory: "/srv/plans".to_owned(),
        };
        assert_eq!(
            absolute.to_string(),
            "`/repo/.warlock/briefs.toml` sets directory = \"/srv/plans\", which is an \
             absolute path: briefs.toml is committed, and an absolute path is a fact \
             about one machine that resolves to nothing on a colleague's clone"
        );

        let parent = Error::ParentDirectory {
            path,
            directory: "docs/../plans".to_owned(),
        };
        assert_eq!(
            parent.to_string(),
            "`/repo/.warlock/briefs.toml` sets directory = \"docs/../plans\", which has a \
             `..` component: the brief directory is written relative to the repository \
             root, and this is a guardrail against a mistake rather than a sandbox"
        );

        // One line each: the file, then the reason. `Syntax` is the exception
        // and deliberately so — it quotes the TOML parser verbatim, layout
        // included, exactly as a bad `pacts.toml` is already quoted.
        for error in [io, absolute, parent] {
            assert!(!error.to_string().contains('\n'), "{error}");
        }
        assert!(
            syntax.to_string().lines().next().is_some_and(|first| first
                .starts_with("malformed brief config at `/repo/.warlock/briefs.toml`: ")),
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
                source: a_de_error(),
            }
            .source()
            .is_some()
        );
        assert!(
            Error::AbsoluteDirectory {
                path: PathBuf::from("x"),
                directory: "/srv/plans".to_owned(),
            }
            .source()
            .is_none()
        );
        assert!(
            Error::ParentDirectory {
                path: PathBuf::from("x"),
                directory: "../plans".to_owned(),
            }
            .source()
            .is_none()
        );
    }
}
