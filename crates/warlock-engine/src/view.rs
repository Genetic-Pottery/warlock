use std::fmt;
use std::fs;
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::str::Utf8Error;

use crate::fitting::{PER_FILE_BYTE_CAP, byte_count};

/// ```
/// use std::fs;
/// use warlock_engine::{Viewed, view_file};
///
/// let dir = tempfile::tempdir()?;
/// let path = dir.path().join("WARLOCK.md");
/// fs::write(&path, "# engine\n\nThe core.\n")?;
///
/// let Viewed { text, cut } = view_file(&path)?;
/// assert_eq!(text, "# engine\n\nThe core.\n");
/// assert!(!cut);
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub fn view_file(path: impl AsRef<Path>) -> Result<Viewed, Unviewable> {
    let path = path.as_ref();
    let mut bytes = read_capped(path).map_err(|source| Unviewable::Unreadable {
        path: path.to_path_buf(),
        source,
    })?;

    // The read stops one byte past the cap, so one byte over is the whole of
    // "there is more to this file", and dropping it needs no cast from the
    // cap's `u64` to an index.
    let cut = byte_count(bytes.len()) > PER_FILE_BYTE_CAP;
    if cut {
        bytes.truncate(bytes.len() - 1);
    }

    let text = match str::from_utf8(&bytes) {
        Ok(text) => text,
        // A cut inside a character is the cap's doing, not the file's, so it
        // costs that one character and nothing else. Everything before
        // `valid_up_to` was just checked, so the floor is unreachable.
        Err(source) if cut && source.error_len().is_none() => {
            str::from_utf8(&bytes[..source.valid_up_to()]).unwrap_or_default()
        }
        Err(source) => {
            return Err(Unviewable::NotText {
                path: path.to_path_buf(),
                source,
            });
        }
    }
    .to_owned();

    Ok(Viewed { text, cut })
}

fn read_capped(path: &Path) -> std::io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    fs::File::open(path)?
        .take(PER_FILE_BYTE_CAP + 1)
        .read_to_end(&mut bytes)?;
    Ok(bytes)
}

#[derive(Debug)]
pub struct Viewed {
    pub text: String,
    pub cut: bool,
}

#[derive(Debug)]
#[non_exhaustive]
pub enum Unviewable {
    Unreadable {
        path: PathBuf,
        source: std::io::Error,
    },
    NotText {
        path: PathBuf,
        source: Utf8Error,
    },
}

impl Unviewable {
    #[must_use]
    pub fn path(&self) -> &Path {
        match self {
            Self::Unreadable { path, .. } | Self::NotText { path, .. } => path,
        }
    }
}

impl fmt::Display for Unviewable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unreadable { path, source } => {
                write!(f, "could not read `{}`: {source}", path.display())
            }
            Self::NotText { path, source } => write!(
                f,
                "`{}` is not text ({source}), so there is nothing to show",
                path.display()
            ),
        }
    }
}

impl std::error::Error for Unviewable {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Unreadable { source, .. } => Some(source),
            Self::NotText { source, .. } => Some(source),
        }
    }
}

#[cfg(test)]
#[path = "tests/view.rs"]
mod tests;
