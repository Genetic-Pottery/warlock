// The home directory is a parameter here and is never read from the
// environment, for the reason it is a parameter in `sigils.rs` and for one more
// that holds only here: this file is where a person's Linear API keys live, so
// a module that resolved `HOME` itself could let a test in this crate read — or
// overwrite — the developer's real key store.

use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::manifest::{temp_file_name, write_and_sync};
use crate::scope::{Rule, validate_scope};

const KEY_DIR: &str = ".warlock";

const KEY_FILE: &str = "keys.toml";

#[cfg(unix)]
const OWNER_ONLY: u32 = 0o600;

/// ```
/// use warlock_engine::keys_path;
///
/// let home = tempfile::tempdir()?;
/// assert_eq!(
///     keys_path(home.path()),
///     home.path().join(".warlock").join("keys.toml"),
/// );
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[must_use]
pub fn keys_path(home: impl AsRef<Path>) -> PathBuf {
    home.as_ref().join(KEY_DIR).join(KEY_FILE)
}

/// ```
/// use warlock_engine::{keys, load_key_names};
///
/// let home = tempfile::tempdir()?;
/// let held = match load_key_names(home.path()) {
///     Err(keys::Error::NotFound { .. }) => Vec::new(),
///     other => other?,
/// };
///
/// assert!(held.is_empty());
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
// Sorted, because a `BTreeMap` is what the file deserialises into: the order
// names were typed in is not recoverable from a TOML table anyway, so a caller
// listing them gets the one order that is the same on every machine.
pub fn load_key_names(home: impl AsRef<Path>) -> Result<Vec<String>, Error> {
    Ok(read(home.as_ref())?.keys.into_keys().collect())
}

/// ```
/// use warlock_engine::{keys_path, load_key};
///
/// let home = tempfile::tempdir()?;
/// std::fs::create_dir_all(home.path().join(".warlock"))?;
/// std::fs::write(keys_path(home.path()), "work = \"lin_api_example\"\n")?;
///
/// assert_eq!(load_key(home.path(), "work")?.as_deref(), Some("lin_api_example"));
/// assert_eq!(load_key(home.path(), "personal")?, None);
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
// A name nobody has stored is `Ok(None)` and a store nobody has written is
// `Err(NotFound)`: "that name is not in your keys" and "you have no keys on
// this machine" send a person to different places, and only a caller holding
// both answers can say which.
pub fn load_key(home: impl AsRef<Path>, name: &str) -> Result<Option<String>, Error> {
    let mut held = read(home.as_ref())?;
    Ok(held.keys.remove(name))
}

/// ```
/// use warlock_engine::{load_key, load_key_names, save_key};
///
/// let home = tempfile::tempdir()?;
/// save_key(home.path(), "work", "lin_api_example")?;
/// save_key(home.path(), "personal", "lin_api_other")?;
///
/// // A name already stored is replaced, never doubled.
/// save_key(home.path(), "work", "lin_api_rotated")?;
/// assert_eq!(load_key(home.path(), "work")?.as_deref(), Some("lin_api_rotated"));
/// assert_eq!(load_key_names(home.path())?, ["personal", "work"]);
///
/// // A name that is not a scope name is refused, and nothing is written.
/// assert!(save_key(home.path(), "Work!", "lin_api_example").is_err());
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
// Judged by `validate_scope`, so a key name, a scope and a sigil are one
// character class across the workspace rather than three that drift. The key
// itself is judged by nothing: this is the store, and a key warlock refused to
// hold because it did not look like one is a key a person cannot use warlock
// with at all.
pub fn save_key(home: impl AsRef<Path>, name: &str, key: &str) -> Result<(), Error> {
    validate_scope(name).map_err(|rule| Error::Name {
        name: name.to_owned(),
        rule,
    })?;

    let mut stored = held(home.as_ref())?;
    stored.keys.insert(name.to_owned(), key.to_owned());
    write(home.as_ref(), &stored)
}

/// ```
/// use warlock_engine::{keys::Forgotten, forget_key, save_key};
///
/// let home = tempfile::tempdir()?;
/// save_key(home.path(), "work", "lin_api_example")?;
///
/// assert_eq!(forget_key(home.path(), "work")?, Forgotten::Key);
/// assert_eq!(forget_key(home.path(), "work")?, Forgotten::Nothing);
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
// The name is not validated on the way in, unlike `save_key`'s. The file is
// hand-editable, so a name `validate_scope` refuses can still be sitting in it,
// and a store warlock can name but not remove from would be a trap. A name
// nobody stored is `Forgotten::Nothing` rather than an error: nothing went
// wrong, and the caller is the one that decides whether a person meant to type
// a name that was not there.
pub fn forget_key(home: impl AsRef<Path>, name: &str) -> Result<Forgotten, Error> {
    let mut stored = read(home.as_ref())?;
    if stored.keys.remove(name).is_none() {
        return Ok(Forgotten::Nothing);
    }

    write(home.as_ref(), &stored)?;
    Ok(Forgotten::Key)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Forgotten {
    Key,
    Nothing,
}

// What a save must carry forward. Unlike `sigils::held`, an unreadable or
// unparseable store is an error here rather than a fresh empty one: replacing a
// sigil config nobody can parse costs a person a list they can retype, and
// replacing a key store costs them credentials that exist nowhere else on the
// machine. Only absence reads as nothing held, because that is the first save.
fn held(home: &Path) -> Result<Config, Error> {
    match read(home) {
        Err(Error::NotFound { .. }) => Ok(Config::default()),
        other => other,
    }
}

fn write(home: &Path, config: &Config) -> Result<(), Error> {
    // Serialise before touching the filesystem: a store that cannot be written
    // as TOML should not leave a new directory behind.
    let text = toml::to_string(config).map_err(|_| Error::Serialize)?;

    let dir = home.join(KEY_DIR);
    fs::create_dir_all(&dir).map_err(|source| Error::Io {
        path: dir.clone(),
        source,
    })?;

    let target = dir.join(KEY_FILE);
    // The temporary must sit in the same directory as the target, so the rename
    // below cannot cross a filesystem and stops being atomic.
    let temp = dir.join(temp_file_name(KEY_FILE));
    let written = owner_only(&temp)
        .and_then(|()| write_and_sync(&temp, text.as_bytes()))
        .map_err(|source| Error::Io {
            path: temp.clone(),
            source,
        })
        .and_then(|()| {
            fs::rename(&temp, &target).map_err(|source| Error::Io {
                path: target.clone(),
                source,
            })
        });

    if written.is_err() {
        drop(fs::remove_file(&temp));
    }
    written
}

// The mode is arranged here rather than in `write_and_sync`, which sets none:
// that function also writes the manifest and `CLAUDE.md`, two files that are
// committed and meant to be readable by everyone, and this module is not the
// place to quietly change their modes.
//
// The file is created empty and owner-only *before* the key is written into it,
// and `File::create` inside `write_and_sync` then truncates it without touching
// the mode. Chmodding afterwards was rejected: it leaves a window in which a
// real key sits on disk at whatever the umask allows. The `set_permissions`
// closes the other hole, a temporary left by an earlier process at this name
// with a looser mode, and `rename` carries the mode on to the target.
#[cfg(unix)]
fn owner_only(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::{OpenOptionsExt as _, PermissionsExt as _};

    fs::OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .mode(OWNER_ONLY)
        .open(path)?;

    fs::set_permissions(path, fs::Permissions::from_mode(OWNER_ONLY))
}

#[cfg(not(unix))]
fn owner_only(_path: &Path) -> std::io::Result<()> {
    Ok(())
}

fn read(home: &Path) -> Result<Config, Error> {
    let path = keys_path(home);
    let text = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => {
            return Err(Error::NotFound { path });
        }
        Err(source) => return Err(Error::Io { path, source }),
    };

    toml::from_str::<Config>(&text).map_err(|source| Error::Syntax {
        path,
        source: Unparseable::of(&source, &text),
    })
}

// A flat `name = "key"` table, so the file stays one a person can open and edit
// without learning a schema. An array of tables was rejected: it admits two
// records carrying one name, and here the name is the identity.
//
// No derived `Debug`: it would print every key in the store, and the whole
// point of this module is that a key value never reaches a message, a log or a
// panic.
#[derive(Default, Serialize, Deserialize)]
#[serde(transparent)]
struct Config {
    keys: BTreeMap<String, String>,
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
        source: Unparseable,
    },
    // No source, unlike `sigils::Error::Serialize`. A `toml::ser::Error` can
    // carry the value it choked on, and here the values are the keys; a store
    // of `String` to `String` has nothing in it that TOML cannot write, so what
    // is discarded is a message no caller can ever be shown.
    Serialize,
    Name {
        name: String,
        rule: Rule,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound { path } => write!(f, "no key store at `{}`", path.display()),
            Self::Io { path, source } => {
                write!(f, "could not read or write `{}`: {source}", path.display())
            }
            Self::Syntax { path, source } => {
                write!(f, "malformed key store at `{}`: {source}", path.display())
            }
            Self::Serialize => write!(f, "could not write the key store as TOML"),
            Self::Name { name, rule } => write!(f, "`{name}` is not a key name: {rule}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::Syntax { source, .. } => Some(source),
            Self::Name { rule, .. } => Some(rule),
            Self::NotFound { .. } | Self::Serialize => None,
        }
    }
}

// `toml::de::Error` renders the line it choked on, and in this file that line
// reads `name = "<a Linear API key>"`. Carrying the parse error as the source,
// the way `sigils::Error` can afford to, would put a key into anything that
// prints an error chain or `Debug`s one. It is reduced to a position on the way
// in instead: that is what sends someone to the right line, and it is all of
// the parse error that is safe to keep.
#[derive(Debug)]
pub struct Unparseable {
    line: Option<usize>,
}

impl Unparseable {
    fn of(source: &toml::de::Error, text: &str) -> Self {
        let line = source
            .span()
            .and_then(|span| text.get(..span.start))
            .map(|before| before.matches('\n').count() + 1);

        Self { line }
    }
}

impl fmt::Display for Unparseable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.line {
            Some(line) => write!(f, "line {line} is not `name = \"key\"`"),
            None => write!(f, "not a `name = \"key\"` table"),
        }
    }
}

impl std::error::Error for Unparseable {}

#[cfg(test)]
#[path = "tests/keys.rs"]
mod tests;
