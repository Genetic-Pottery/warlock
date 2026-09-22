//! Two clones of one commit, on two machines, at two absolute paths, must
//! agree: nothing but the sorted relative paths and the file bytes goes into
//! the digest.

use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::manifest;
use crate::walk::{self, DOCUMENT_FILE};

// The rule for the `v1` below: the version moves when a repository that changed
// nothing would hash differently, and not otherwise. Not when this file
// changes, and not when the walk gains a source of rules — teaching it to read
// `.warlockignore` adds no rules to a repository that has none, so the same
// files go in and the digest is byte-identical. Moving it there would restale
// every repository in the world over a feature none of them opted into.
//
// The key derivation is domain separation: this digest cannot collide with a
// plain blake3 of the same bytes taken for some other purpose.
const HASH_CONTEXT: &str = "warlock subtree hash v1 2026-08-19";

// The digest of one file's bytes, for the per-file record in a `PactEntry`.
//
// Its own context and not `HASH_CONTEXT`: this hashes bytes where the subtree
// digest hashes a directory's paths and contents together, and two digests that
// could ever be compared to one another must not be able to collide by having
// been derived the same way. Bumping this restales every per-file line in
// existence, the same as bumping the other.
const FILE_CONTEXT: &str = "warlock file hash v1 2026-09-14";

/// ```
/// use std::fs;
/// use warlock_engine::file_hash;
///
/// let dir = tempfile::tempdir()?;
/// let path = dir.path().join("reading.rs");
/// fs::write(&path, "pub fn read_one() {}\n")?;
///
/// let first = file_hash(&path)?;
/// assert_eq!(first, file_hash(&path)?, "the same bytes hash the same");
///
/// fs::write(&path, "pub fn read_one(at: usize) {}\n")?;
/// assert_ne!(first, file_hash(&path)?, "and moved bytes do not");
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub fn file_hash(path: impl AsRef<Path>) -> Result<String, Error> {
    let path = path.as_ref();
    let bytes = fs::read(path).map_err(|source| Error::Read {
        path: path.to_path_buf(),
        source,
    })?;
    Ok(bytes_hash(&bytes))
}

pub(crate) fn bytes_hash(bytes: &[u8]) -> String {
    let mut hasher = blake3::Hasher::new_derive_key(FILE_CONTEXT);
    update_prefixed(&mut hasher, bytes);
    hasher.finalize().to_hex().to_string()
}

// Bumping this restales every `[pact.lines]` table in existence: nothing
// recorded under the old string can match, so the first refresh after the bump
// re-describes every file once and is cheap again after that.
const LINE_CONTEXT: &str = "warlock line hash v1 2026-09-17";

// What a `[pact.lines]` entry records: the file's digest and the line warlock
// wrote about it, together, so that reuse asks both questions at once.
//
// The source hash alone was not enough. Reuse keyed on it says a line may be
// kept because the file behind it has not moved, and says nothing about where
// the line came from — so a line somebody typed into the document by hand was
// carried forward and granted as though a pass had written it. Binding the two
// means an edited line fails to match and is re-described, at the cost of that
// one file.
pub(crate) fn line_hash(file: &str, line: &str) -> String {
    let mut hasher = blake3::Hasher::new_derive_key(LINE_CONTEXT);
    update_prefixed(&mut hasher, file.as_bytes());
    update_prefixed(&mut hasher, line.as_bytes());
    hasher.finalize().to_hex().to_string()
}

/// ```
/// use std::fs;
/// use warlock_engine::subtree_hash;
///
/// let dir = tempfile::tempdir()?;
/// fs::write(dir.path().join("WARLOCK.md"), "# module\n")?;
///
/// let before = subtree_hash(dir.path())?;
/// assert_eq!(before, subtree_hash(dir.path())?, "the same bytes hash the same");
///
/// fs::write(dir.path().join("WARLOCK.md"), "# module, revised\n")?;
/// assert_ne!(before, subtree_hash(dir.path())?);
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub fn subtree_hash(dir: impl AsRef<Path>) -> Result<String, Error> {
    let dir = dir.as_ref();
    let mut hasher = blake3::Hasher::new_derive_key(HASH_CONTEXT);

    let files = walk::subtree_files(dir).map_err(|source| match source {
        walk::Error::Walk(source) => Error::Walk { source },
        walk::Error::Path { path, source } => Error::Path { path, source },
    })?;
    for (relative, path) in files {
        // Length-prefixed, so no arrangement of names and contents can be
        // mistaken for another: `a/b` holding `c` and `a` holding `bc` are
        // different inputs and must be different digests.
        update_prefixed(&mut hasher, relative.as_bytes());

        // Read whole rather than streamed: the length goes in ahead of the
        // bytes, and a length taken from metadata can disagree with what is
        // then read.
        //
        // An unreadable file fails the whole hash rather than being skipped. A
        // skipped file contributes exactly what a deleted one does, so a
        // subtree granted while it was absent would come back fresh on a run
        // where it merely could not be opened — a false green. Hashing the
        // error text instead would make the digest depend on the operating
        // system's wording.
        let bytes = fs::read(&path).map_err(|source| Error::Read { path, source })?;
        update_prefixed(&mut hasher, &bytes);
    }

    Ok(hasher.finalize().to_hex().to_string())
}

const CARRY_HASH_CONTEXT: &str = "warlock carry hash v1 2026-09-06";

// The digest of exactly what a directory's pass would be shown, taken before and
// after it: a match with the recorded one is the early cutoff. `None` wherever
// anything could not be read, which never matches and so always runs the pass.
pub(crate) fn carry_hash(directory: &Path) -> Option<String> {
    let found = walk::own(directory).ok()?;
    let mut hasher = blake3::Hasher::new_derive_key(CARRY_HASH_CONTEXT);

    // Two sections, each length-prefixed and each announced by its count, so no
    // arrangement of one can be read as the other: a directory holding a file
    // named `x` and one holding a child `x` with a document are different
    // inputs and must be different digests.
    update_section(&mut hasher, &found.files)?;
    update_section(&mut hasher, &found.child_documents)?;

    // The third section: the document itself, absent and empty told apart by
    // the marker byte, so a directory with no document cannot digest as one
    // holding a document of nothing.
    match fs::read(directory.join(DOCUMENT_FILE)) {
        Ok(bytes) => {
            hasher.update(&[1]);
            update_prefixed(&mut hasher, &bytes);
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            hasher.update(&[0]);
        }
        Err(_) => return None,
    }

    Some(hasher.finalize().to_hex().to_string())
}

fn update_section(hasher: &mut blake3::Hasher, entries: &BTreeMap<String, PathBuf>) -> Option<()> {
    hasher.update(&byte_count(entries.len()).to_le_bytes());
    for (name, path) in entries {
        update_prefixed(hasher, name.as_bytes());
        update_prefixed(hasher, &fs::read(path).ok()?);
    }
    Some(())
}

fn update_prefixed(hasher: &mut blake3::Hasher, bytes: &[u8]) {
    hasher.update(&byte_count(bytes.len()).to_le_bytes());
    hasher.update(bytes);
}

/// Saturating rather than fallible or panicking: the clamp is unreachable on
/// every target this builds for, and a hash function is the last place to
/// introduce a panic over a case that cannot happen.
pub(crate) fn byte_count(bytes: usize) -> u64 {
    u64::try_from(bytes).unwrap_or(u64::MAX)
}

/// Hand-rolled like [`manifest::Error`] rather than pulling in an
/// error-handling dependency for three variants.
#[derive(Debug)]
#[non_exhaustive]
pub enum Error {
    Walk {
        source: ignore::Error,
    },
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    Path {
        path: PathBuf,
        // Boxed: a manifest error carries a parser error inside it, and the
        // other variants are a path and an errno.
        source: Box<manifest::Error>,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Walk { source } => write!(f, "could not walk the subtree: {source}"),
            Self::Read { path, source } => write!(
                f,
                "could not read `{}`, so the subtree has no hash: {source}",
                path.display()
            ),
            Self::Path { path, source } => write!(
                f,
                "could not name `{}` relative to the hashed directory: {source}",
                path.display()
            ),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Walk { source } => Some(source),
            Self::Read { source, .. } => Some(source),
            Self::Path { source, .. } => Some(source.as_ref()),
        }
    }
}

#[cfg(test)]
#[path = "tests/hash.rs"]
mod tests;
