//! The pact manifest: `.warlock/pacts.toml`.
//!
//! Section 9 of the design doc asks for an escape hatch — whatever Warlock
//! records has to be something a human can open, read and hand-edit their way
//! out of. So there is exactly one manifest per repository, it is TOML, it is
//! committed to git, and it holds nothing but the list of pacted modules and
//! what was granted to each.
//!
//! This module is the in-memory shape of that file, the rules for turning it
//! into text and back, and the two functions that move it on and off disk:
//! [`Manifest::save`] and [`Manifest::load`]. Both are given the root directory
//! — the parent of `.warlock/` — and go straight to it; nothing here searches
//! upwards for a repository root.
//!
//! Two properties are worth stating up front, because everything else follows
//! from them:
//!
//! * **An entry with no granted hash is an entry that was never judged**, and
//!   the absent key is the whole representation — no sentinel hash, no empty
//!   string, no `judged = false`. It round-trips with the key still absent.
//!   Per [`NodeState`](crate::NodeState) unjudged *is* stale, so nothing is
//!   lost by saying nothing.
//! * **Paths are stored relative to the manifest's own directory, with forward
//!   slashes.** Two clones of the same commit sitting at different absolute
//!   paths, on different operating systems, produce the same bytes.

use std::fmt;
use std::fs;
use std::io::Write as _;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Deserializer, Serialize};

/// The directory the manifest lives in, directly under the repository root.
///
/// Committed to git, like the manifest itself: a pact is a fact about the
/// repository, not about one developer's checkout.
const MANIFEST_DIR: &str = ".warlock";

/// The manifest's file name inside [`MANIFEST_DIR`].
const MANIFEST_FILE: &str = "pacts.toml";

/// The schema version this build reads and writes.
///
/// A manifest carrying any other version is rejected outright with
/// [`Error::UnsupportedVersion`] rather than being read as if it were this
/// one: an old binary guessing at a newer file is how a manifest gets
/// silently rewritten and loses data.
pub const SCHEMA_VERSION: u32 = 1;

/// The whole manifest: a schema version and the pacted modules.
///
/// The fields are private, unlike the tree types, because there *is* an
/// invariant to protect here: the version is fixed at [`SCHEMA_VERSION`] and
/// entry order is the order the file is written in, which is what makes two
/// clones agree byte for byte.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    /// Serialised first, so the file opens with `version = 1` above the array
    /// of tables — TOML puts plain values before tables, and this is also the
    /// order a reader wants them in.
    #[serde(deserialize_with = "deserialize_version")]
    version: u32,
    /// One `[[pact]]` table per pacted module, in the order they are written.
    /// An empty manifest omits the key entirely rather than writing `pact = []`.
    #[serde(rename = "pact", default, skip_serializing_if = "Vec::is_empty")]
    entries: Vec<PactEntry>,
}

impl Manifest {
    /// An empty manifest at the current [`SCHEMA_VERSION`].
    #[must_use]
    pub fn new() -> Self {
        Self {
            version: SCHEMA_VERSION,
            entries: Vec::new(),
        }
    }

    /// A manifest holding `entries`, in the order given.
    ///
    /// The order is kept, not sorted: the caller decides what the file looks
    /// like, and a stable order is what keeps a save from churning the diff.
    #[must_use]
    pub fn with_entries(entries: impl IntoIterator<Item = PactEntry>) -> Self {
        Self {
            version: SCHEMA_VERSION,
            entries: entries.into_iter().collect(),
        }
    }

    /// The schema version, always [`SCHEMA_VERSION`] for a manifest this build
    /// produced or accepted.
    #[must_use]
    pub const fn version(&self) -> u32 {
        self.version
    }

    /// The pacted modules, in file order.
    #[must_use]
    pub fn entries(&self) -> &[PactEntry] {
        &self.entries
    }

    /// Append `entry` after the entries already held.
    pub fn push(&mut self, entry: PactEntry) {
        self.entries.push(entry);
    }

    /// The entry for the module stored at `module`, or `None`.
    ///
    /// `module` is compared as stored, i.e. in manifest-relative forward-slash
    /// form; [`to_manifest_path`] turns a caller's path into that form.
    #[must_use]
    pub fn entry(&self, module: &str) -> Option<&PactEntry> {
        self.entries.iter().find(|entry| entry.module == module)
    }

    /// The manifest as the text of a `pacts.toml` file.
    ///
    /// ```
    /// use warlock_engine::{Manifest, PactEntry};
    ///
    /// let entry = PactEntry::new(".", "crates/engine", "crates/engine/WARLOCK.md")?;
    /// let toml = Manifest::with_entries([entry]).to_toml_string()?;
    ///
    /// assert!(toml.starts_with("version = 1\n"));
    /// assert!(toml.contains("[[pact]]"));
    /// // Never judged, so the grant keys are simply not there.
    /// assert!(!toml.contains("granted_hash"));
    /// # Ok::<(), warlock_engine::ManifestError>(())
    /// ```
    ///
    /// # Errors
    ///
    /// [`Error::Serialize`] if the manifest cannot be written as TOML. Nothing
    /// in the current schema can trigger that, but the fallibility is the
    /// serialiser's, not ours to swallow.
    pub fn to_toml_string(&self) -> Result<String, Error> {
        toml::to_string(self).map_err(|source| Error::Serialize { source })
    }

    /// Parse the text of a `pacts.toml` file.
    ///
    /// The version is checked before any entry is looked at, so a file from a
    /// future schema is rejected rather than half-read. Entries are then
    /// parsed one at a time, which is what lets a failure name the entry that
    /// caused it instead of pointing at the file as a whole.
    ///
    /// # Errors
    ///
    /// * [`Error::Syntax`] if the text is not TOML, or does not have the shape
    ///   of a manifest at all (no `version` key, `pact` not an array of
    ///   tables, an unknown top-level key).
    /// * [`Error::UnsupportedVersion`] if `version` is not [`SCHEMA_VERSION`].
    /// * [`Error::Entry`] if one of the `[[pact]]` tables is malformed, naming
    ///   it by index and, where it can be read, by module path.
    pub fn from_toml_str(text: &str) -> Result<Self, Error> {
        let raw: RawManifest = toml::from_str(text).map_err(|source| Error::Syntax { source })?;

        if u32::try_from(raw.version) != Ok(SCHEMA_VERSION) {
            return Err(Error::UnsupportedVersion {
                found: raw.version,
                supported: SCHEMA_VERSION,
            });
        }

        let mut entries = Vec::with_capacity(raw.pact.len());
        for (index, value) in raw.pact.into_iter().enumerate() {
            // Read the module path out of the raw table first: if the entry
            // fails to parse, this is what makes the error nameable.
            let module = value
                .get("module")
                .and_then(toml::Value::as_str)
                .map(ToOwned::to_owned);
            let entry = value
                .try_into::<PactEntry>()
                .map_err(|source| Error::Entry {
                    index,
                    module,
                    source,
                })?;
            entries.push(entry);
        }

        Ok(Self {
            version: SCHEMA_VERSION,
            entries,
        })
    }

    /// Write the manifest to `<root>/.warlock/pacts.toml`, atomically.
    ///
    /// `root` is the parent of `.warlock/` — the repository root — and is
    /// taken as given: nothing here walks upwards looking for one. The
    /// directory is created if it is not there.
    ///
    /// Atomic means the file is never seen half-written, not even if the
    /// process dies mid-save: the text goes to a temporary file *in the same
    /// directory* as the target (so the rename cannot cross a filesystem) and
    /// is then renamed over it. A reader either sees the whole old manifest or
    /// the whole new one. On success no temporary file is left behind; on
    /// failure it is cleaned up on a best-effort basis.
    ///
    /// ```
    /// use warlock_engine::{Manifest, PactEntry};
    ///
    /// let root = tempfile::tempdir()?;
    /// let entry = PactEntry::new(root.path(), "crates/engine", "crates/engine/WARLOCK.md")?;
    /// let manifest = Manifest::with_entries([entry]);
    ///
    /// manifest.save(root.path())?;
    /// assert_eq!(Manifest::load(root.path())?, manifest);
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    ///
    /// # Errors
    ///
    /// [`Error::Serialize`] if the manifest cannot be written as TOML, or
    /// [`Error::Io`] naming the path that failed if the directory cannot be
    /// created, the temporary file cannot be written, or the rename fails.
    pub fn save(&self, root: impl AsRef<Path>) -> Result<(), Error> {
        // Serialise before touching the filesystem: a manifest that cannot be
        // written as TOML should not leave a new directory behind.
        let text = self.to_toml_string()?;

        let dir = root.as_ref().join(MANIFEST_DIR);
        fs::create_dir_all(&dir).map_err(|source| Error::Io {
            path: dir.clone(),
            source,
        })?;

        let temp = dir.join(temp_file_name());
        if let Err(source) = write_and_sync(&temp, text.as_bytes()) {
            drop(fs::remove_file(&temp));
            return Err(Error::Io { path: temp, source });
        }

        let target = dir.join(MANIFEST_FILE);
        if let Err(source) = fs::rename(&temp, &target) {
            drop(fs::remove_file(&temp));
            return Err(Error::Io {
                path: target,
                source,
            });
        }
        Ok(())
    }

    /// Read the manifest at `<root>/.warlock/pacts.toml`.
    ///
    /// `root` is the parent of `.warlock/`, exactly as for [`Manifest::save`].
    ///
    /// **A missing file is [`Error::NotFound`], not an empty manifest.** The
    /// two are different facts — "this repository has never pacted anything"
    /// versus "this repository pacted nothing" — and only the caller knows
    /// which of the two it wants to act on, so nothing is invented here. A
    /// caller happy to treat the first as the second writes:
    ///
    /// ```
    /// use warlock_engine::{Manifest, ManifestError};
    ///
    /// let root = tempfile::tempdir()?;
    /// let manifest = match Manifest::load(root.path()) {
    ///     Err(ManifestError::NotFound { .. }) => Manifest::new(),
    ///     other => other?,
    /// };
    ///
    /// assert_eq!(manifest, Manifest::new());
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    ///
    /// # Errors
    ///
    /// * [`Error::NotFound`] if there is no manifest at that path.
    /// * [`Error::Io`] if there is one but it cannot be read.
    /// * [`Error::UnsupportedVersion`], [`Error::Syntax`] or [`Error::Entry`]
    ///   if it can be read but not understood, as per
    ///   [`Manifest::from_toml_str`] — a corrupt manifest is never confused
    ///   with a missing one.
    pub fn load(root: impl AsRef<Path>) -> Result<Self, Error> {
        let path = manifest_path(root);
        match fs::read_to_string(&path) {
            Ok(text) => Self::from_toml_str(&text),
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => {
                Err(Error::NotFound { path })
            }
            Err(source) => Err(Error::Io { path, source }),
        }
    }
}

impl Default for Manifest {
    /// An empty manifest at the current schema version — *not* version zero,
    /// which is why this is written out rather than derived.
    fn default() -> Self {
        Self::new()
    }
}

/// One pacted module: where it lives, what documents it, and what was granted.
///
/// Both paths are held in manifest-relative forward-slash form, which is why
/// they are private: [`PactEntry::new`] is the only way in from a caller's
/// path, and it is the thing that normalises. Read them back as stored with
/// [`PactEntry::module`] / [`PactEntry::document`], or as real paths under a
/// root with [`PactEntry::module_path`] / [`PactEntry::document_path`].
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PactEntry {
    /// The pacted directory, relative to the manifest's directory.
    module: String,
    /// The document describing it, relative to the manifest's directory. Held
    /// separately from `module` because the file name is not Warlock's to
    /// assume.
    document: String,
    /// The subtree hash captured when freshness was last granted. Opaque here:
    /// this crate neither computes nor verifies it. Absent means never judged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    granted_hash: Option<String>,
    /// When that grant happened, as an RFC 3339 timestamp. A plain string, so
    /// no date/time crate is dragged in to read a field nothing here does
    /// arithmetic on. Absent alongside an absent hash.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    granted_at: Option<String>,
}

impl PactEntry {
    /// An entry for `module`, documented by `document`, never judged.
    ///
    /// Both paths may be absolute or relative. An absolute path is made
    /// relative to `root`; a relative one is taken to already be relative to
    /// it. Either way what gets stored is forward-slash form, so the same
    /// logical entry under two different roots stores the same string.
    ///
    /// ```
    /// use warlock_engine::PactEntry;
    ///
    /// let under_a = PactEntry::new("/tmp/a", "/tmp/a/crates/engine", "/tmp/a/crates/engine/WARLOCK.md")?;
    /// let under_b = PactEntry::new("/tmp/b", "/tmp/b/crates/engine", "/tmp/b/crates/engine/WARLOCK.md")?;
    ///
    /// assert_eq!(under_a.module(), "crates/engine");
    /// assert_eq!(under_a, under_b);
    /// assert_eq!(under_a.granted_hash(), None);
    /// # Ok::<(), warlock_engine::ManifestError>(())
    /// ```
    ///
    /// # Errors
    ///
    /// [`Error::PathOutsideRoot`] if either path is not inside `root` (which
    /// includes a relative path that climbs out with `..`), or
    /// [`Error::NonUtf8Path`] if either cannot be written as UTF-8 text.
    pub fn new(
        root: impl AsRef<Path>,
        module: impl AsRef<Path>,
        document: impl AsRef<Path>,
    ) -> Result<Self, Error> {
        let root = root.as_ref();
        Ok(Self {
            module: to_manifest_path(root, module)?,
            document: to_manifest_path(root, document)?,
            granted_hash: None,
            granted_at: None,
        })
    }

    /// The same entry with a grant recorded: the subtree hash that was judged
    /// fresh, and when.
    ///
    /// `granted_at` is expected to be RFC 3339, but is stored as given —
    /// nothing here parses it.
    #[must_use]
    pub fn with_grant(
        mut self,
        granted_hash: impl Into<String>,
        granted_at: impl Into<String>,
    ) -> Self {
        self.granted_hash = Some(granted_hash.into());
        self.granted_at = Some(granted_at.into());
        self
    }

    /// The same entry with any grant dropped, i.e. back to never-judged.
    #[must_use]
    pub fn without_grant(mut self) -> Self {
        self.granted_hash = None;
        self.granted_at = None;
        self
    }

    /// The module directory as stored: relative, forward slashes, `"."` for
    /// the root of the repository itself.
    #[must_use]
    pub fn module(&self) -> &str {
        &self.module
    }

    /// The document as stored, in the same form as [`PactEntry::module`].
    #[must_use]
    pub fn document(&self) -> &str {
        &self.document
    }

    /// The module directory as a path under `root`.
    #[must_use]
    pub fn module_path(&self, root: impl AsRef<Path>) -> PathBuf {
        from_manifest_path(root, &self.module)
    }

    /// The document as a path under `root`.
    #[must_use]
    pub fn document_path(&self, root: impl AsRef<Path>) -> PathBuf {
        from_manifest_path(root, &self.document)
    }

    /// The subtree hash captured at the last grant, or `None` if this module
    /// has never been judged.
    ///
    /// The hash is opaque to this crate: nothing here computes or verifies it.
    #[must_use]
    pub fn granted_hash(&self) -> Option<&str> {
        self.granted_hash.as_deref()
    }

    /// When the grant happened, as stored (RFC 3339), or `None`.
    #[must_use]
    pub fn granted_at(&self) -> Option<&str> {
        self.granted_at.as_deref()
    }

    /// Whether this module has ever been judged.
    ///
    /// `false` means pacted-but-never-judged, which is
    /// [`NodeState::PactedStale`](crate::NodeState::PactedStale) — there is no
    /// fourth state for it, and this crate does not decide freshness either
    /// way, because that needs a hash it never computes.
    #[must_use]
    pub fn is_judged(&self) -> bool {
        self.granted_hash.is_some()
    }
}

/// `path` in the form the manifest stores: relative to `root`, forward
/// slashes, and `"."` for `root` itself.
///
/// An absolute `path` is made relative to `root`; a relative one is taken to
/// already be relative to it, and `root` is then only used to describe an
/// error. `.` components are dropped.
///
/// ```
/// use std::path::Path;
/// use warlock_engine::to_manifest_path;
///
/// assert_eq!(to_manifest_path("/repo", Path::new("/repo").join("docs").join("adr"))?, "docs/adr");
/// assert_eq!(to_manifest_path("/repo", "./docs/adr")?, "docs/adr");
/// assert_eq!(to_manifest_path("/repo", "/repo")?, ".");
/// # Ok::<(), warlock_engine::ManifestError>(())
/// ```
///
/// # Errors
///
/// [`Error::PathOutsideRoot`] if `path` does not sit inside `root`, including
/// a relative path that climbs out of it with `..`; [`Error::NonUtf8Path`] if
/// any component is not valid UTF-8, since the manifest is text.
pub fn to_manifest_path(root: impl AsRef<Path>, path: impl AsRef<Path>) -> Result<String, Error> {
    let (root, path) = (root.as_ref(), path.as_ref());
    let outside = || Error::PathOutsideRoot {
        root: root.to_path_buf(),
        path: path.to_path_buf(),
    };

    let relative = if path.is_absolute() {
        path.strip_prefix(root).map_err(|_| outside())?
    } else {
        path
    };

    let mut parts = Vec::new();
    for component in relative.components() {
        match component {
            Component::CurDir => {}
            Component::Normal(part) => {
                let part = part.to_str().ok_or_else(|| Error::NonUtf8Path {
                    path: path.to_path_buf(),
                })?;
                parts.push(part);
            }
            // `..` escapes the root, and a root or drive component means this
            // was never relative to it in the first place.
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(outside());
            }
        }
    }

    if parts.is_empty() {
        // `root` itself: the repository root can be a pacted module too.
        return Ok(".".to_owned());
    }
    Ok(parts.join("/"))
}

/// A path stored in the manifest, back as a path under `root`.
///
/// The inverse of [`to_manifest_path`]: forward-slash segments are joined onto
/// `root` with the platform's separator, and `"."` gives `root` itself.
///
/// ```
/// use std::path::Path;
/// use warlock_engine::from_manifest_path;
///
/// assert_eq!(from_manifest_path("/repo", "docs/adr"), Path::new("/repo").join("docs").join("adr"));
/// assert_eq!(from_manifest_path("/repo", "."), Path::new("/repo"));
/// ```
#[must_use]
pub fn from_manifest_path(root: impl AsRef<Path>, stored: &str) -> PathBuf {
    let mut path = root.as_ref().to_path_buf();
    for part in stored.split('/') {
        if part.is_empty() || part == "." {
            continue;
        }
        path.push(part);
    }
    path
}

/// Where the manifest lives under `root`: `<root>/.warlock/pacts.toml`.
///
/// `root` is the parent of `.warlock/`, i.e. the repository root. There is no
/// search: this is a join, and the caller is the one that knows the root.
///
/// ```
/// use std::path::Path;
/// use warlock_engine::manifest_path;
///
/// assert_eq!(
///     manifest_path("/repo"),
///     Path::new("/repo").join(".warlock").join("pacts.toml"),
/// );
/// ```
#[must_use]
pub fn manifest_path(root: impl AsRef<Path>) -> PathBuf {
    root.as_ref().join(MANIFEST_DIR).join(MANIFEST_FILE)
}

/// A file name for the temporary file a save writes before renaming.
///
/// It only has to be unique among whatever else might be writing this
/// directory right now — process id for other processes, a counter for other
/// threads in this one. It is a dot file so that a save interrupted hard
/// enough to leave one behind at least leaves it out of a casual `ls`.
fn temp_file_name() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!(".{MANIFEST_FILE}.{}.{n}.tmp", std::process::id())
}

/// Write `bytes` to a fresh file at `path` and flush them to the disk.
///
/// The `sync_all` is the point: without it the rename can land before the
/// contents do, and a crash in between leaves an empty manifest where a good
/// one used to be.
fn write_and_sync(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let mut file = fs::File::create(path)?;
    file.write_all(bytes)?;
    file.sync_all()
}

/// Everything that can go wrong reading, writing or building a manifest.
///
/// Hand-rolled rather than derived: an error-handling dependency would buy
/// nothing over the four impls below, and the variants are the vocabulary a
/// front end shows a user, so they are worth writing out.
///
/// `Display` includes the wrapped cause where there is one, so printing an
/// error with `{}` alone says the whole story; [`Error::source`] still exposes
/// it for callers that walk the chain themselves.
///
/// [`Error::source`]: std::error::Error::source
#[derive(Debug)]
#[non_exhaustive]
pub enum Error {
    /// The manifest file is not there. Distinguishes a repository that has
    /// never pacted anything from one whose manifest is corrupt.
    NotFound {
        /// The path that was looked for.
        path: PathBuf,
    },
    /// The manifest could not be read or written.
    Io {
        /// The path being read or written.
        path: PathBuf,
        /// What the filesystem said.
        source: std::io::Error,
    },
    /// The text is not valid TOML, or has nothing like the shape of a
    /// manifest.
    Syntax {
        /// What the TOML parser said, including where.
        source: toml::de::Error,
    },
    /// The manifest declares a schema version this build does not understand.
    /// Refusing is deliberate: guessing would risk rewriting a newer file with
    /// less than it came with.
    UnsupportedVersion {
        /// The version the file declares.
        found: i64,
        /// The version this build reads and writes, i.e. [`SCHEMA_VERSION`].
        supported: u32,
    },
    /// One `[[pact]]` table is malformed or has a key of the wrong type.
    /// Named by position, and by module path when that much could be read, so
    /// a hand-editor knows which entry to go and look at.
    Entry {
        /// Which `[[pact]]` table it was, counting from zero.
        index: usize,
        /// Its `module` path, if the table had a readable one.
        module: Option<String>,
        /// What the TOML deserialiser said.
        source: toml::de::Error,
    },
    /// The manifest could not be turned into TOML.
    Serialize {
        /// What the TOML serialiser said.
        source: toml::ser::Error,
    },
    /// A path was handed in that does not sit inside the manifest's root, so
    /// it has no manifest-relative form.
    PathOutsideRoot {
        /// The root paths are measured against.
        root: PathBuf,
        /// The path that fell outside it.
        path: PathBuf,
    },
    /// A path is not valid UTF-8, and the manifest is a text file people edit.
    NonUtf8Path {
        /// The path that could not be written as text.
        path: PathBuf,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound { path } => {
                write!(f, "no pact manifest at `{}`", path.display())
            }
            Self::Io { path, source } => {
                write!(f, "could not read or write `{}`: {source}", path.display())
            }
            Self::Syntax { source } => write!(f, "malformed pact manifest: {source}"),
            Self::UnsupportedVersion { found, supported } => write!(
                f,
                "pact manifest schema version {found} is not supported; this build reads version {supported}"
            ),
            Self::Entry {
                index,
                module: Some(module),
                source,
            } => write!(f, "pact entry {index} (`{module}`) is malformed: {source}"),
            Self::Entry {
                index,
                module: None,
                source,
            } => write!(f, "pact entry {index} is malformed: {source}"),
            Self::Serialize { source } => {
                write!(f, "could not write the pact manifest as TOML: {source}")
            }
            Self::PathOutsideRoot { root, path } => write!(
                f,
                "`{}` is not inside the manifest root `{}`",
                path.display(),
                root.display()
            ),
            Self::NonUtf8Path { path } => write!(
                f,
                "`{}` is not valid UTF-8 and cannot be stored in the manifest",
                path.display()
            ),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::Syntax { source } | Self::Entry { source, .. } => Some(source),
            Self::Serialize { source } => Some(source),
            Self::NotFound { .. }
            | Self::UnsupportedVersion { .. }
            | Self::PathOutsideRoot { .. }
            | Self::NonUtf8Path { .. } => None,
        }
    }
}

/// The manifest as read before anything is believed about it: the version as
/// whatever integer the file says, and the entries still raw.
///
/// Parsing in two passes is what buys the two error variants that name things
/// — the version before it is trusted, and each entry by index — instead of
/// one blanket "this file is wrong".
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawManifest {
    /// The declared schema version, unchecked.
    version: i64,
    /// The `[[pact]]` tables, unvalidated.
    #[serde(default)]
    pact: Vec<toml::Value>,
}

/// Reject a foreign schema version even on the derived path, so
/// `toml::from_str::<Manifest>` cannot quietly do what
/// [`Manifest::from_toml_str`] refuses to.
fn deserialize_version<'de, D: Deserializer<'de>>(deserializer: D) -> Result<u32, D::Error> {
    let version = u32::deserialize(deserializer)?;
    if version == SCHEMA_VERSION {
        Ok(version)
    } else {
        Err(serde::de::Error::custom(format!(
            "pact manifest schema version {version} is not supported; this build reads version {SCHEMA_VERSION}"
        )))
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};

    use super::{
        Error, MANIFEST_FILE, Manifest, PactEntry, SCHEMA_VERSION, from_manifest_path,
        manifest_path, to_manifest_path,
    };

    /// An entry that has never been judged.
    fn unjudged() -> PactEntry {
        PactEntry::new(
            ".",
            "crates/warlock-engine",
            "crates/warlock-engine/WARLOCK.md",
        )
        .expect("a relative path inside the root is storable")
    }

    /// The same entry, judged: a hash and the time it was granted.
    fn judged() -> PactEntry {
        unjudged().with_grant("d0f5a1", "2026-08-19T07:32:00Z")
    }

    /// A parser error to hang the `Display` tests off, since one cannot be
    /// constructed directly.
    fn a_de_error() -> toml::de::Error {
        toml::from_str::<Manifest>("version = \"one\"").expect_err("a string is not an integer")
    }

    /// A throwaway directory to stand in for a repository root. Each test gets
    /// its own, so the suite stays parallel-safe and leaves nothing behind.
    fn a_root() -> tempfile::TempDir {
        tempfile::tempdir().expect("a temporary directory")
    }

    /// The file names directly inside `<root>/.warlock`, sorted.
    fn warlock_dir_listing(root: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(root.join(".warlock"))
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

    /// Write `text` to `<root>/.warlock/pacts.toml` without going through
    /// [`Manifest::save`], the way a human or a merge conflict would.
    fn hand_write(root: &Path, text: &str) {
        let path = manifest_path(root);
        fs::create_dir_all(path.parent().expect("the manifest has a directory"))
            .expect("creates .warlock");
        fs::write(&path, text).expect("writes the manifest");
    }

    #[test]
    fn an_unjudged_entry_omits_both_grant_keys() {
        let manifest = Manifest::with_entries([unjudged()]);
        let text = manifest.to_toml_string().expect("serialises");

        assert!(!text.contains("granted_hash"), "{text}");
        assert!(!text.contains("granted_at"), "{text}");

        let parsed = Manifest::from_toml_str(&text).expect("parses");
        assert_eq!(parsed, manifest);
        assert_eq!(parsed.entries()[0].granted_hash(), None);
        assert_eq!(parsed.entries()[0].granted_at(), None);
        assert!(!parsed.entries()[0].is_judged());
        // The round trip re-emits the file without the keys: no sentinel value
        // has crept in on the way through.
        assert_eq!(parsed.to_toml_string().expect("re-serialises"), text);
    }

    #[test]
    fn a_judged_entry_carries_both_grant_keys() {
        let manifest = Manifest::with_entries([judged()]);
        let text = manifest.to_toml_string().expect("serialises");

        assert!(text.contains("granted_hash = \"d0f5a1\""), "{text}");
        assert!(
            text.contains("granted_at = \"2026-08-19T07:32:00Z\""),
            "{text}"
        );

        let parsed = Manifest::from_toml_str(&text).expect("parses");
        assert_eq!(parsed, manifest);
        assert_eq!(parsed.entries()[0].granted_hash(), Some("d0f5a1"));
        assert_eq!(
            parsed.entries()[0].granted_at(),
            Some("2026-08-19T07:32:00Z")
        );
        assert!(parsed.entries()[0].is_judged());
        assert_eq!(parsed.to_toml_string().expect("re-serialises"), text);
    }

    #[test]
    fn dropping_a_grant_drops_both_keys_again() {
        let text = Manifest::with_entries([judged().without_grant()])
            .to_toml_string()
            .expect("serialises");
        assert!(!text.contains("granted"), "{text}");
    }

    #[test]
    fn a_manifest_with_one_judged_and_one_unjudged_entry_round_trips() {
        let manifest = Manifest::with_entries([judged(), unjudged()]);
        let text = manifest.to_toml_string().expect("serialises");
        assert_eq!(Manifest::from_toml_str(&text).expect("parses"), manifest);
        assert_eq!(manifest.entries().len(), 2);
        assert_eq!(manifest.version(), SCHEMA_VERSION);
    }

    #[test]
    fn an_empty_manifest_is_just_the_version() {
        let text = Manifest::new().to_toml_string().expect("serialises");
        assert_eq!(text, "version = 1\n");
        assert_eq!(
            Manifest::from_toml_str(&text).expect("parses"),
            Manifest::default()
        );
    }

    #[test]
    fn push_and_entry_lookup_use_stored_form() {
        let mut manifest = Manifest::new();
        manifest.push(unjudged());
        assert_eq!(
            manifest.entry("crates/warlock-engine"),
            Some(&unjudged()),
            "looked up by the path as stored"
        );
        assert!(manifest.entry("crates/warlock-tui").is_none());
    }

    #[test]
    fn paths_are_stored_with_forward_slashes() {
        // Built with the platform separator, stored with slashes.
        let module = Path::new("crates").join("warlock-engine");
        let document = module.join("WARLOCK.md");
        let entry = PactEntry::new(".", &module, &document).expect("inside the root");

        assert_eq!(entry.module(), "crates/warlock-engine");
        assert_eq!(entry.document(), "crates/warlock-engine/WARLOCK.md");

        let text = Manifest::with_entries([entry])
            .to_toml_string()
            .expect("serialises");
        assert!(
            text.contains("module = \"crates/warlock-engine\""),
            "{text}"
        );
    }

    #[test]
    fn the_same_content_under_two_roots_serialises_identically() {
        let under = |root: &str| {
            let root = PathBuf::from(root);
            let module = root.join("crates").join("warlock-engine");
            let document = module.join("WARLOCK.md");
            Manifest::with_entries([
                PactEntry::new(&root, &module, &document)
                    .expect("inside the root")
                    .with_grant("d0f5a1", "2026-08-19T07:32:00Z"),
                PactEntry::new(&root, &root, root.join("WARLOCK.md")).expect("inside the root"),
            ])
            .to_toml_string()
            .expect("serialises")
        };

        assert_eq!(under("/tmp/one"), under("/home/someone/else/two"));
    }

    #[test]
    fn absolute_paths_become_relative_and_relative_ones_are_left_alone() {
        assert_eq!(
            to_manifest_path("/repo", Path::new("/repo").join("docs")).expect("inside"),
            "docs"
        );
        assert_eq!(
            to_manifest_path("/repo", "docs/adr").expect("inside"),
            "docs/adr"
        );
        assert_eq!(
            to_manifest_path("/repo", "./docs/./adr").expect("inside"),
            "docs/adr"
        );
    }

    #[test]
    fn the_root_itself_is_stored_as_a_dot() {
        assert_eq!(
            to_manifest_path("/repo", "/repo").expect("is the root"),
            "."
        );
        assert_eq!(to_manifest_path("/repo", "").expect("is the root"), ".");
        assert_eq!(from_manifest_path("/repo", "."), Path::new("/repo"));
    }

    #[test]
    fn stored_paths_convert_back_to_paths_under_a_root() {
        let entry = unjudged();
        assert_eq!(
            entry.module_path("/repo"),
            Path::new("/repo").join("crates").join("warlock-engine")
        );
        assert_eq!(
            entry.document_path("/repo"),
            Path::new("/repo")
                .join("crates")
                .join("warlock-engine")
                .join("WARLOCK.md")
        );
        // And back again, unchanged.
        assert_eq!(
            to_manifest_path("/repo", entry.module_path("/repo")).expect("inside"),
            entry.module()
        );
    }

    #[test]
    fn a_path_outside_the_root_is_rejected() {
        assert!(matches!(
            to_manifest_path("/repo", "/elsewhere/docs"),
            Err(Error::PathOutsideRoot { .. })
        ));
        assert!(matches!(
            to_manifest_path("/repo", "../escape"),
            Err(Error::PathOutsideRoot { .. })
        ));
        assert!(matches!(
            PactEntry::new("/repo", "/elsewhere", "/elsewhere/WARLOCK.md"),
            Err(Error::PathOutsideRoot { .. })
        ));
    }

    #[cfg(unix)]
    #[test]
    fn a_non_utf8_path_is_rejected() {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;

        let path = Path::new(OsStr::from_bytes(b"crates/\xff"));
        assert!(matches!(
            to_manifest_path(".", path),
            Err(Error::NonUtf8Path { .. })
        ));
    }

    #[test]
    fn an_unrecognised_version_is_rejected_before_the_entries_are_read() {
        let text = "version = 999\n\n[[pact]]\nmodule = \"x\"\ndocument = \"x/WARLOCK.md\"\n";
        match Manifest::from_toml_str(text) {
            Err(Error::UnsupportedVersion { found, supported }) => {
                assert_eq!(found, 999);
                assert_eq!(supported, SCHEMA_VERSION);
            }
            other => panic!("expected an unsupported-version error, got {other:?}"),
        }
        // The derived path refuses it too, rather than reading it as version 1.
        assert!(toml::from_str::<Manifest>(text).is_err());
    }

    #[test]
    fn a_malformed_entry_names_itself() {
        let text = concat!(
            "version = 1\n\n",
            "[[pact]]\nmodule = \"crates/warlock-engine\"\ndocument = \"crates/warlock-engine/WARLOCK.md\"\n\n",
            "[[pact]]\nmodule = \"crates/warlock-tui\"\ndocument = 7\n",
        );
        match Manifest::from_toml_str(text) {
            Err(error @ Error::Entry { index: 1, .. }) => {
                assert!(error.to_string().contains("crates/warlock-tui"), "{error}");
            }
            other => panic!("expected an entry error, got {other:?}"),
        }
    }

    #[test]
    fn an_entry_with_an_unknown_key_is_an_error_too() {
        let text =
            "version = 1\n\n[[pact]]\nmodule = \"x\"\ndocument = \"x/WARLOCK.md\"\nfresh = true\n";
        assert!(matches!(
            Manifest::from_toml_str(text),
            Err(Error::Entry { .. })
        ));
    }

    #[test]
    fn the_old_readme_key_is_an_error_rather_than_an_alias() {
        // The document key was called `readme` before Warlock's artifact was
        // named. There are no manifests in the wild carrying it, so it is an
        // unknown key like any other — no alias, no migration, no quiet
        // acceptance that would let two spellings of one field coexist.
        let text = "version = 1\n\n[[pact]]\nmodule = \"x\"\nreadme = \"x/WARLOCK.md\"\n";
        assert!(matches!(
            Manifest::from_toml_str(text),
            Err(Error::Entry { .. })
        ));

        // And the key that is written is the new one.
        let written = Manifest::with_entries([unjudged()])
            .to_toml_string()
            .expect("serialises");
        assert!(
            written.contains("document = \"crates/warlock-engine/WARLOCK.md\""),
            "{written}"
        );
        assert!(!written.contains("readme"), "{written}");
    }

    #[test]
    fn text_that_is_not_a_manifest_is_a_syntax_error() {
        assert!(matches!(
            Manifest::from_toml_str("not toml"),
            Err(Error::Syntax { .. })
        ));
        assert!(matches!(
            Manifest::from_toml_str(""),
            Err(Error::Syntax { .. })
        ));
    }

    #[test]
    fn every_error_variant_says_what_happened() {
        let cases = [
            (
                Error::NotFound {
                    path: PathBuf::from("/repo/.warlock/pacts.toml"),
                },
                "no pact manifest at `/repo/.warlock/pacts.toml`",
            ),
            (
                Error::Io {
                    path: PathBuf::from("/repo/.warlock/pacts.toml"),
                    source: std::io::Error::new(std::io::ErrorKind::PermissionDenied, "denied"),
                },
                "could not read or write `/repo/.warlock/pacts.toml`: denied",
            ),
            (
                Error::UnsupportedVersion {
                    found: 999,
                    supported: SCHEMA_VERSION,
                },
                "pact manifest schema version 999 is not supported; this build reads version 1",
            ),
            (
                Error::PathOutsideRoot {
                    root: PathBuf::from("/repo"),
                    path: PathBuf::from("/elsewhere"),
                },
                "`/elsewhere` is not inside the manifest root `/repo`",
            ),
            (
                Error::NonUtf8Path {
                    path: PathBuf::from("/repo/odd"),
                },
                "`/repo/odd` is not valid UTF-8 and cannot be stored in the manifest",
            ),
        ];
        for (error, expected) in cases {
            assert_eq!(error.to_string(), expected);
        }

        // The three that wrap a cause: the message names the entry or the file
        // and then quotes what the underlying crate said.
        let syntax = Error::Syntax {
            source: a_de_error(),
        };
        assert!(
            syntax.to_string().starts_with("malformed pact manifest: "),
            "{syntax}"
        );

        let named = Error::Entry {
            index: 1,
            module: Some("crates/warlock-tui".to_owned()),
            source: a_de_error(),
        };
        assert!(
            named
                .to_string()
                .starts_with("pact entry 1 (`crates/warlock-tui`) is malformed: "),
            "{named}"
        );

        let unnamed = Error::Entry {
            index: 0,
            module: None,
            source: a_de_error(),
        };
        assert!(
            unnamed
                .to_string()
                .starts_with("pact entry 0 is malformed: "),
            "{unnamed}"
        );
    }

    #[test]
    fn errors_expose_the_cause_they_wrap() {
        use std::error::Error as _;

        assert!(
            Error::Syntax {
                source: a_de_error()
            }
            .source()
            .is_some()
        );
        assert!(
            Error::Entry {
                index: 0,
                module: None,
                source: a_de_error(),
            }
            .source()
            .is_some()
        );
        assert!(
            Error::Io {
                path: PathBuf::from("x"),
                source: std::io::Error::other("boom"),
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

    #[test]
    fn saving_creates_the_directory_and_leaves_no_temporary_file_behind() {
        let root = a_root();
        assert!(!root.path().join(".warlock").exists(), "nothing there yet");

        Manifest::with_entries([judged(), unjudged()])
            .save(root.path())
            .expect("saves");

        // Exactly one file: the temporary the save wrote through has been
        // renamed away, not left lying next to the manifest.
        assert_eq!(warlock_dir_listing(root.path()), [MANIFEST_FILE]);
        assert!(manifest_path(root.path()).is_file());
    }

    #[test]
    fn saving_again_replaces_the_manifest_and_still_leaves_one_file() {
        let root = a_root();
        Manifest::with_entries([judged()])
            .save(root.path())
            .expect("saves");

        let replacement = Manifest::with_entries([unjudged()]);
        replacement.save(root.path()).expect("saves over");

        assert_eq!(warlock_dir_listing(root.path()), [MANIFEST_FILE]);
        assert_eq!(Manifest::load(root.path()).expect("loads"), replacement);
    }

    #[test]
    fn the_same_manifest_saved_under_two_roots_gives_byte_identical_files() {
        let saved_under = |root: &Path| {
            let module = root.join("crates").join("warlock-engine");
            let manifest = Manifest::with_entries([
                PactEntry::new(root, &module, module.join("WARLOCK.md"))
                    .expect("inside the root")
                    .with_grant("d0f5a1", "2026-08-19T07:32:00Z"),
                PactEntry::new(root, root, root.join("WARLOCK.md")).expect("inside the root"),
            ]);
            manifest.save(root).expect("saves");
            fs::read(manifest_path(root)).expect("reads the file back")
        };

        let (one, two) = (a_root(), a_root());
        assert_ne!(one.path(), two.path(), "two different absolute roots");
        assert_eq!(saved_under(one.path()), saved_under(two.path()));
    }

    #[test]
    fn loading_what_was_saved_gives_the_manifest_back() {
        let root = a_root();
        let manifest = Manifest::with_entries([judged(), unjudged()]);
        manifest.save(root.path()).expect("saves");

        let loaded = Manifest::load(root.path()).expect("loads");
        assert_eq!(loaded, manifest);
        assert_eq!(loaded.entries()[0].granted_hash(), Some("d0f5a1"));
        assert_eq!(loaded.entries()[1].granted_hash(), None);
    }

    #[test]
    fn saving_what_was_loaded_gives_the_file_back_byte_for_byte() {
        let root = a_root();
        // Hand-written rather than produced by a save, so this is a real
        // statement about the file format and not about the serialiser
        // agreeing with itself.
        let original = concat!(
            "version = 1\n\n",
            "[[pact]]\n",
            "module = \"crates/warlock-engine\"\n",
            "document = \"crates/warlock-engine/WARLOCK.md\"\n",
            "granted_hash = \"d0f5a1\"\n",
            "granted_at = \"2026-08-19T07:32:00Z\"\n\n",
            "[[pact]]\n",
            "module = \"crates/warlock-tui\"\n",
            "document = \"crates/warlock-tui/WARLOCK.md\"\n",
        );
        hand_write(root.path(), original);

        let loaded = Manifest::load(root.path()).expect("loads");
        loaded.save(root.path()).expect("saves");

        assert_eq!(
            fs::read_to_string(manifest_path(root.path())).expect("reads"),
            original,
            "a load-then-save is a no-op on the bytes, so it does not churn the diff"
        );
        assert_eq!(Manifest::load(root.path()).expect("reloads"), loaded);
    }

    #[test]
    fn loading_a_manifest_from_a_future_schema_is_a_version_error() {
        let root = a_root();
        hand_write(
            root.path(),
            "version = 999\n\n[[pact]]\nmodule = \"x\"\ndocument = \"x/WARLOCK.md\"\n",
        );

        match Manifest::load(root.path()) {
            Err(Error::UnsupportedVersion { found, supported }) => {
                assert_eq!(found, 999);
                assert_eq!(supported, SCHEMA_VERSION);
            }
            other => panic!("expected an unsupported-version error, got {other:?}"),
        }
    }

    #[test]
    fn loading_a_manifest_with_a_bad_entry_names_that_entry() {
        let root = a_root();
        hand_write(
            root.path(),
            concat!(
                "version = 1\n\n",
                "[[pact]]\nmodule = \"crates/warlock-engine\"\ndocument = \"crates/warlock-engine/WARLOCK.md\"\n\n",
                "[[pact]]\nmodule = \"crates/warlock-tui\"\ndocument = 7\n",
            ),
        );

        let error = Manifest::load(root.path()).expect_err("a number is not a path");
        assert!(matches!(error, Error::Entry { index: 1, .. }), "{error:?}");
        assert!(
            error.to_string().contains("crates/warlock-tui"),
            "the message points at the entry to go and hand-edit: {error}"
        );
    }

    #[test]
    fn loading_a_manifest_that_is_not_there_is_not_found() {
        let root = a_root();

        // Documented behaviour: absent is `NotFound`, not an empty manifest,
        // so a caller can tell "never pacted" from "pacted nothing".
        match Manifest::load(root.path()) {
            Err(Error::NotFound { path }) => assert_eq!(path, manifest_path(root.path())),
            other => panic!("expected a not-found error, got {other:?}"),
        }

        // And a directory with no `.warlock` at all is the same answer, not an
        // I/O error about the missing parent.
        assert!(matches!(
            Manifest::load(root.path().join("nowhere")),
            Err(Error::NotFound { .. })
        ));
    }

    #[test]
    fn a_missing_manifest_is_distinguishable_from_a_corrupt_one() {
        let (missing, corrupt) = (a_root(), a_root());
        hand_write(corrupt.path(), "this is not a manifest\n");

        assert!(matches!(
            Manifest::load(missing.path()),
            Err(Error::NotFound { .. })
        ));
        assert!(matches!(
            Manifest::load(corrupt.path()),
            Err(Error::Syntax { .. })
        ));
    }
}
