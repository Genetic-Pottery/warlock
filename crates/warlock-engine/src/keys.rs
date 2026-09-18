// The home directory is a parameter here and is never read from the
// environment, for the reason it is a parameter in `sigils.rs` and for one more
// that holds only here: this file is where a person's Linear API keys live, so
// a module that resolved `HOME` itself could let a test in this crate read —
// or, once saving lands, overwrite — the developer's real key store.

use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;

const KEY_DIR: &str = ".warlock";

const KEY_FILE: &str = "keys.toml";

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
#[derive(Deserialize)]
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
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::Syntax { source, .. } => Some(source),
            Self::NotFound { .. } => None,
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
