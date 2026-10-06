// The home directory is a parameter here and is never read from the
// environment. Resolving `HOME` inside this module was rejected: it would let a
// test in this crate read or write the developer's real home, and the caller
// that resolved a home is the one that knows which home it means.

use std::ffi::OsStr;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::manifest::write_atomically;
use crate::scope::{Rule, validate_scope};

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

// `pub(crate)` so `pulls.rs` derives its run directory from this one join rather
// than repeating `.warlock/<project>` itself: the two would then be free to
// disagree, and the day they did every run record on the machine would appear to
// vanish while the sigils beside them still read.
pub(crate) fn project_dir(home: &Path, root: &Path) -> PathBuf {
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
    read(home.as_ref(), root.as_ref()).map(|config| config.sigils)
}

// Every door that acts on what is held reads it through this, so they agree
// that a checkout nobody configured holds nothing. What a config that is there
// and will not read means stays each door's call — the panel and `warlock
// check` print it as unknown and carry on, filing and routing refuse — which is
// why it comes back as an error rather than being folded here too.
pub fn held_sigils(home: impl AsRef<Path>, root: impl AsRef<Path>) -> Result<Vec<String>, Error> {
    match load_sigils(home, root) {
        Err(Error::NotFound { .. }) => Ok(Vec::new()),
        other => other,
    }
}

/// ```
/// use warlock_engine::{sigils, load_key_binding, save_key_binding};
///
/// let (home, root) = (tempfile::tempdir()?, tempfile::tempdir()?);
/// // A checkout nobody has configured has no config to be bound by.
/// assert!(matches!(
///     load_key_binding(home.path(), root.path()),
///     Err(sigils::Error::NotFound { .. })
/// ));
///
/// save_key_binding(home.path(), root.path(), "work")?;
/// assert_eq!(load_key_binding(home.path(), root.path())?.as_deref(), Some("work"));
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
// Unbound is `Ok(None)` and unconfigured is `Err(NotFound)`, following
// `load_key`: "this checkout is bound to no key" and "this checkout has no
// config at all" are answers a caller sends a person to different places with,
// and collapsing them here would take that choice away.
pub fn load_key_binding(
    home: impl AsRef<Path>,
    root: impl AsRef<Path>,
) -> Result<Option<String>, Error> {
    read(home.as_ref(), root.as_ref()).map(|config| config.key)
}

/// ```
/// use warlock_engine::{load_sigils, save_key_binding, save_sigils};
///
/// let (home, root) = (tempfile::tempdir()?, tempfile::tempdir()?);
/// save_sigils(home.path(), root.path(), &["data-plane".to_owned()])?;
/// save_key_binding(home.path(), root.path(), "work")?;
///
/// // The binding is written beside the sigils, not over them.
/// assert_eq!(load_sigils(home.path(), root.path())?, ["data-plane"]);
/// // A name that is not a scope name is refused, and nothing is written.
/// assert!(save_key_binding(home.path(), root.path(), "Work!").is_err());
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
// Judged by `validate_scope`, so a key name, a scope and a sigil are one
// character class across the workspace rather than three that drift. A looser
// rule of its own for key names was rejected: the name is typed at a prompt and
// read back off a config line, which is exactly what that class is for.
pub fn save_key_binding(
    home: impl AsRef<Path>,
    root: impl AsRef<Path>,
    name: &str,
) -> Result<(), Error> {
    validate_scope(name).map_err(|rule| Error::Name {
        name: name.to_owned(),
        rule,
    })?;

    let mut config = held(home.as_ref(), root.as_ref());
    config.key = Some(name.to_owned());
    write(home.as_ref(), root.as_ref(), &config)
}

/// ```
/// use warlock_engine::{held_base_branch, save_base_branch};
///
/// let (home, root) = (tempfile::tempdir()?, tempfile::tempdir()?);
/// // A checkout nobody has configured takes the remote's default.
/// assert_eq!(held_base_branch(home.path(), root.path())?, None);
///
/// save_base_branch(home.path(), root.path(), Some("develop"))?;
/// assert_eq!(held_base_branch(home.path(), root.path())?.as_deref(), Some("develop"));
///
/// save_base_branch(home.path(), root.path(), None)?;
/// assert_eq!(held_base_branch(home.path(), root.path())?, None);
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
// Absent and unconfigured are one answer here, unlike the key binding: both mean
// "use the remote's default", and no caller sends a person anywhere different
// for one than for the other. A config that is there and will not read stays an
// error, for `load_sigils`'s reason.
pub fn held_base_branch(
    home: impl AsRef<Path>,
    root: impl AsRef<Path>,
) -> Result<Option<String>, Error> {
    match read(home.as_ref(), root.as_ref()) {
        Ok(config) => Ok(config.branch),
        Err(Error::NotFound { .. }) => Ok(None),
        Err(error) => Err(error),
    }
}

/// ```
/// use warlock_engine::save_base_branch;
///
/// let (home, root) = (tempfile::tempdir()?, tempfile::tempdir()?);
/// assert!(save_base_branch(home.path(), root.path(), Some(" ")).is_err());
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
// Judged only for being one word. Whether it is a branch `git` will accept, and
// whether the remote has it, is `git`'s answer at the next pull: a second set of
// ref-name rules written here would drift from the real one.
pub fn save_base_branch(
    home: impl AsRef<Path>,
    root: impl AsRef<Path>,
    branch: Option<&str>,
) -> Result<(), Error> {
    if let Some(name) = branch
        && (name.is_empty() || name.chars().any(char::is_whitespace))
    {
        return Err(Error::BranchName {
            name: name.to_owned(),
        });
    }

    let mut config = held(home.as_ref(), root.as_ref());
    config.branch = branch.map(ToOwned::to_owned);
    write(home.as_ref(), root.as_ref(), &config)
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
    let mut config = held(home.as_ref(), root.as_ref());
    config.sigils = sigils.to_vec();
    write(home.as_ref(), root.as_ref(), &config)
}

fn read(home: &Path, root: &Path) -> Result<Config, Error> {
    let path = sigils_path(home, root);
    let text = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => {
            return Err(Error::NotFound { path });
        }
        Err(source) => return Err(Error::Io { path, source }),
    };

    toml::from_str::<Config>(&text).map_err(|source| Error::Syntax { path, source })
}

// What a save must carry forward: the fields it was not asked to change. A file
// that will not read or will not parse yields the default rather than an error,
// so saving sigils onto a config broken by hand still works — refusing it was
// rejected, because there is no binding recoverable from bytes that do not
// parse, and refusing would leave a person with a file warlock can neither read
// nor replace.
fn held(home: &Path, root: &Path) -> Config {
    read(home, root).unwrap_or_default()
}

fn write(home: &Path, root: &Path, config: &Config) -> Result<(), Error> {
    // Serialise before touching the filesystem: a config that cannot be written
    // as TOML should not leave a new directory behind.
    let text = toml::to_string(config).map_err(|source| Error::Serialize { source })?;

    let dir = project_dir(home, root);
    fs::create_dir_all(&dir).map_err(|source| Error::Io {
        path: dir.clone(),
        source,
    })?;

    write_atomically(&dir, SIGIL_FILE, text.as_bytes())
        .map_err(|(path, source)| Error::Io { path, source })
}

fn readable_name(canonical: &Path) -> String {
    let name = canonical
        .file_name()
        .map_or_else(|| UNNAMED_ROOT.into(), OsStr::to_string_lossy);

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
#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Config {
    #[serde(default)]
    sigils: Vec<String>,
    // `skip_serializing_if` rather than `Option`'s serialiser doing the right
    // thing by itself: serde writes `None` as a TOML value, so without this an
    // unbound checkout's config would gain a `key = ` line it cannot read back.
    // A file nobody has bound stays byte-identical to one written before this
    // field existed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    key: Option<String>,
    // Skipped when unset for `key`'s reason above.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    branch: Option<String>,
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
    Name {
        name: String,
        rule: Rule,
    },
    BranchName {
        name: String,
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
            Self::Name { name, rule } => {
                write!(f, "`{name}` is not a key name: {rule}")
            }
            Self::BranchName { name } => {
                write!(
                    f,
                    "`{name}` is not a branch name: it is empty or holds a space"
                )
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
            Self::Name { rule, .. } => Some(rule),
            Self::NotFound { .. } | Self::BranchName { .. } => None,
        }
    }
}

#[cfg(test)]
#[path = "tests/sigils.rs"]
mod tests;
