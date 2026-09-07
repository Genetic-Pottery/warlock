use std::fmt;
use std::fs;
use std::path::{Component, Path, PathBuf};

use serde::Deserialize;

use crate::manifest::manifest_path;

const BRIEFS_FILE: &str = "briefs.toml";

pub const DEFAULT_BRIEF_DIRECTORY: &str = "docs";

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

/// ```
/// use warlock_engine::{DEFAULT_BRIEF_DIRECTORY, load_briefs};
///
/// let root = tempfile::tempdir()?;
/// assert_eq!(load_briefs(root.path())?, DEFAULT_BRIEF_DIRECTORY);
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
// The deliberate departure from `Manifest::load`'s absent-is-not-empty rule.
// There is no second fact to tell apart here — an optional setting that was
// never set *is* the default — so a missing file answers rather than errors.
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

// A guardrail against a mistake in a committed file, not a sandbox, and the
// distinction decides how it is written. Components are judged as components:
// `docs/../plans` is refused because one of them is `..`, not because
// normalising it would leave the repository. Replacing this with the
// plausible-looking `canonicalize` plus a containment check would follow
// symlinks and touch the filesystem for a file that may not be on disk yet, and
// would then read as a security boundary it is not — a symlink inside the tree
// still goes wherever it goes.
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

// `deny_unknown_fields` earns its place on a short hand-edited file with no
// version key: read leniently, `directroy = "plans"` is a valid file expressing
// nothing, and the brief would be written somewhere its author never asked for.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Briefs {
    #[serde(default = "default_directory")]
    directory: String,
}

fn default_directory() -> String {
    DEFAULT_BRIEF_DIRECTORY.to_owned()
}

#[derive(Debug)]
#[non_exhaustive]
pub enum Error {
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    Syntax {
        path: PathBuf,
        source: toml::de::Error,
    },
    AbsoluteDirectory {
        path: PathBuf,
        directory: String,
    },
    ParentDirectory {
        path: PathBuf,
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

    fn a_root() -> tempfile::TempDir {
        tempfile::tempdir().expect("a temporary directory")
    }

    fn hand_write(root: &Path, text: &str) {
        let path = briefs_path(root);
        fs::create_dir_all(path.parent().expect("the file has a directory"))
            .expect("creates .warlock");
        fs::write(&path, text).expect("writes briefs.toml");
    }

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
