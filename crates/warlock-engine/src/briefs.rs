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
#[path = "tests/briefs.rs"]
mod tests;
