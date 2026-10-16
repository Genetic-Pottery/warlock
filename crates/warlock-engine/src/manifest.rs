use std::fmt;
use std::fs;
use std::io::Write as _;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Deserializer, Serialize};

use crate::scope::{Rule, validate_scope};
use crate::walk::MANIFEST_DIR;

const MANIFEST_FILE: &str = "scopes.toml";

pub(crate) const ROOT_MODULE: &str = ".";

// A manifest declaring any other version is refused rather than read as if it
// were this one. The alternative — read it leniently and keep going — was
// rejected: an old binary that guesses at a newer file rewrites it with less
// than it came with.
pub const SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    #[serde(deserialize_with = "deserialize_version")]
    version: u32,
    #[serde(rename = "directory", default, skip_serializing_if = "Vec::is_empty")]
    directories: Vec<ScopedDirectory>,
    // After `directories` so the `[[scope]]` tables follow the `[[directory]]`
    // rows, and both after `version`: TOML puts every scalar before the first
    // table, so a field declared above `version` would move the version line
    // into whichever table happened to come first.
    #[serde(rename = "scope", default, skip_serializing_if = "Vec::is_empty")]
    scopes: Vec<ScopeRecord>,
}

impl Manifest {
    #[must_use]
    pub fn new() -> Self {
        Self {
            version: SCHEMA_VERSION,
            directories: Vec::new(),
            scopes: Vec::new(),
        }
    }

    #[must_use]
    pub fn with_directories(directories: impl IntoIterator<Item = ScopedDirectory>) -> Self {
        Self {
            version: SCHEMA_VERSION,
            directories: directories.into_iter().collect(),
            scopes: Vec::new(),
        }
    }

    /// Rebuild around a new set of `[[directory]]` rows, carrying the
    /// `[[scope]]` records across as they stand.
    ///
    /// ```
    /// use warlock_engine::{Manifest, ScopedDirectory, ScopeRecord};
    ///
    /// let manifest = Manifest::with_directories([ScopedDirectory::new(".", "crates/engine", "data-plane")?])
    ///     .with_scopes([ScopeRecord::new("data-plane", "WAR", "In Review", "area/data-plane")]);
    ///
    /// // Every scoped directory gone, and the record no row now names is still there.
    /// let emptied = manifest.rebuilt_with([]);
    /// assert!(emptied.directories().is_empty());
    /// assert_eq!(emptied.scopes(), manifest.scopes());
    /// # Ok::<(), warlock_engine::manifest::Error>(())
    /// ```
    // The counterpart to `with_directories` for every caller that has a
    // manifest in hand: that one is an associated function with nothing to copy
    // from, so a rebuild written through it silently drops the records. Records
    // are not pruned to what the rows name — unscoping the last directory under
    // a scope would otherwise delete where that work is filed.
    #[must_use]
    pub fn rebuilt_with(&self, directories: impl IntoIterator<Item = ScopedDirectory>) -> Self {
        Self {
            version: self.version,
            directories: directories.into_iter().collect(),
            scopes: self.scopes.clone(),
        }
    }

    /// The same manifest with the directory at `path` scoped `scope`, or
    /// unscoped for `None`. `path` is a stored path, as
    /// [`to_manifest_path`] spells it.
    ///
    /// ```
    /// use warlock_engine::{Manifest, ScopedDirectory};
    ///
    /// let manifest = Manifest::with_directories([ScopedDirectory::new(".", "crates", "platform")?]);
    ///
    /// let scoped = manifest.rescoped("crates/engine", Some("data-plane"));
    /// assert_eq!(scoped.directory("crates/engine").map(ScopedDirectory::scope), Some("data-plane"));
    /// assert_eq!(scoped.rescoped("crates", None).directories().len(), 1);
    /// # Ok::<(), warlock_engine::manifest::Error>(())
    /// ```
    // Every other row keeps its place, so a saved file differs from the one on
    // disk in one place; a directory scoped for the first time goes on the end.
    #[must_use]
    pub fn rescoped(&self, path: &str, scope: Option<&str>) -> Self {
        let mut directories: Vec<ScopedDirectory> = Vec::with_capacity(self.directories.len() + 1);
        let mut placed = false;
        for directory in &self.directories {
            if directory.path != path {
                directories.push(directory.clone());
            } else if let Some(scope) = scope {
                directories.push(ScopedDirectory {
                    path: path.to_owned(),
                    scope: scope.to_owned(),
                });
                placed = true;
            }
        }
        if let (false, Some(scope)) = (placed, scope) {
            directories.push(ScopedDirectory {
                path: path.to_owned(),
                scope: scope.to_owned(),
            });
        }
        self.rebuilt_with(directories)
    }

    #[must_use]
    pub fn with_scopes(mut self, scopes: impl IntoIterator<Item = ScopeRecord>) -> Self {
        self.scopes = scopes.into_iter().collect();
        self
    }

    #[must_use]
    pub const fn version(&self) -> u32 {
        self.version
    }

    #[must_use]
    pub fn directories(&self) -> &[ScopedDirectory] {
        &self.directories
    }

    #[must_use]
    pub fn scopes(&self) -> &[ScopeRecord] {
        &self.scopes
    }

    pub fn push(&mut self, directory: ScopedDirectory) {
        self.directories.push(directory);
    }

    #[must_use]
    pub fn directory(&self, path: &str) -> Option<&ScopedDirectory> {
        self.directories
            .iter()
            .find(|directory| directory.path == path)
    }

    // Exact, and the one comparison every caller looks a scope up by: a lookup
    // that folded, trimmed or matched loosely would answer "no record" for a
    // name `warlock check` routes through, and a second record would be written
    // that the router never reads.
    #[must_use]
    pub fn scope_record(&self, name: &str) -> Option<&ScopeRecord> {
        self.scopes.iter().find(|record| record.name() == name)
    }

    /// ```
    /// use warlock_engine::{Manifest, ScopedDirectory};
    ///
    /// let directory = ScopedDirectory::new(".", "crates/engine", "data-plane")?;
    /// let toml = Manifest::with_directories([directory]).to_toml_string()?;
    ///
    /// assert!(toml.starts_with("version = 1\n"));
    /// assert!(toml.contains("[[directory]]"));
    /// # Ok::<(), warlock_engine::manifest::Error>(())
    /// ```
    pub fn to_toml_string(&self) -> Result<String, Error> {
        toml::to_string(self).map_err(|source| Error::Serialize { source })
    }

    pub fn from_toml_str(text: &str) -> Result<Self, Error> {
        Self::read(text, None)
    }

    // The path is threaded through rather than reached for, because this is also
    // the parser `from_toml_str` exposes and there is no file behind that one. A
    // record error names the file when `load` read it and says only which record
    // when a caller handed over text — as against carrying a made-up path, which
    // would send somebody to hand-edit a file that is not the one at fault.
    fn read(text: &str, path: Option<&Path>) -> Result<Self, Error> {
        let raw: RawManifest = toml::from_str(text).map_err(|source| Error::Syntax { source })?;

        if u32::try_from(raw.version) != Ok(SCHEMA_VERSION) {
            return Err(Error::UnsupportedVersion {
                found: raw.version,
                supported: SCHEMA_VERSION,
            });
        }

        let directories = raw
            .directory
            .into_iter()
            .enumerate()
            .map(|(index, value)| {
                // Read the path out of the raw table first: if the row fails to
                // parse, this is what makes the error nameable.
                let path = value
                    .get("path")
                    .and_then(toml::Value::as_str)
                    .map(ToOwned::to_owned);
                value
                    .try_into::<ScopedDirectory>()
                    .map_err(|source| Error::Directory {
                        index,
                        path,
                        source,
                    })
            })
            .collect::<Result<Vec<_>, _>>()?;

        let mut scopes: Vec<ScopeRecord> = Vec::new();
        for (index, value) in raw.scope.into_iter().enumerate() {
            // Same reason as the path above: read the name out of the raw
            // table first, so a record that will not parse is still nameable by
            // the name it did spell.
            let named = value
                .get("name")
                .and_then(toml::Value::as_str)
                .map(ToOwned::to_owned);
            let refused = |source| Error::Scope {
                path: path.map(Path::to_path_buf),
                index,
                name: named.clone(),
                source,
            };

            let record = value
                .try_into::<ScopeRecord>()
                .map_err(|source| refused(ScopeFault::Malformed(Box::new(source))))?;
            // The one rule, borrowed rather than restated: a record name is a
            // scope, so `validate_scope` judges it and nothing here does.
            validate_scope(&record.name).map_err(|rule| refused(ScopeFault::Refused(rule)))?;

            if scopes.iter().any(|existing| existing.name == record.name) {
                return Err(Error::DuplicateScope {
                    path: path.map(Path::to_path_buf),
                    index,
                    name: record.name,
                });
            }
            scopes.push(record);
        }

        Ok(Self {
            version: SCHEMA_VERSION,
            directories,
            scopes,
        })
    }

    /// ```
    /// use warlock_engine::{Manifest, ScopedDirectory};
    ///
    /// let root = tempfile::tempdir()?;
    /// let directory = ScopedDirectory::new(root.path(), "crates/engine", "data-plane")?;
    /// let manifest = Manifest::with_directories([directory]);
    ///
    /// manifest.save(root.path())?;
    /// assert_eq!(Manifest::load(root.path())?, manifest);
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn save(&self, root: impl AsRef<Path>) -> Result<(), Error> {
        // Serialise before touching the filesystem: a manifest that cannot be
        // written as TOML should not leave a new directory behind.
        let text = self.to_toml_string()?;

        let dir = root.as_ref().join(MANIFEST_DIR);
        fs::create_dir_all(&dir).map_err(|source| Error::Io {
            path: dir.clone(),
            source,
        })?;

        write_atomically(&dir, MANIFEST_FILE, text.as_bytes())
            .map_err(|(path, source)| Error::Io { path, source })
    }

    /// ```
    /// use warlock_engine::{manifest, Manifest};
    ///
    /// let root = tempfile::tempdir()?;
    /// let manifest = match Manifest::load(root.path()) {
    ///     Err(manifest::Error::NotFound { .. }) => Manifest::new(),
    ///     other => other?,
    /// };
    ///
    /// assert_eq!(manifest, Manifest::new());
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    // Absent is not empty. Answering `Ok(Manifest::new())` for a missing file
    // was the rejected alternative: "this repository has never scoped anything"
    // and "this repository scoped nothing" are different facts, and only the
    // caller knows which one it means to act on. The doctest above is the
    // caller that wants them to be the same fact, written out.
    pub fn load(root: impl AsRef<Path>) -> Result<Self, Error> {
        let path = manifest_path(root);
        match fs::read_to_string(&path) {
            Ok(text) => Self::read(&text, Some(&path)),
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => {
                Err(Error::NotFound { path })
            }
            Err(source) => Err(Error::Io { path, source }),
        }
    }
}

impl Default for Manifest {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScopedDirectory {
    path: String,
    // Read back exactly as written, never validated or folded here. These bytes
    // are committed, so normalising a hand-edited scope on the way through would
    // put a line in somebody's diff on a save they only asked for because they
    // scoped something else. `validate_scope` is where a caller asks.
    scope: String,
}

impl ScopedDirectory {
    /// ```
    /// use warlock_engine::ScopedDirectory;
    ///
    /// let under_a = ScopedDirectory::new("/tmp/a", "/tmp/a/crates/engine", "data-plane")?;
    /// let under_b = ScopedDirectory::new("/tmp/b", "/tmp/b/crates/engine", "data-plane")?;
    ///
    /// assert_eq!(under_a.path(), "crates/engine");
    /// assert_eq!(under_a.scope(), "data-plane");
    /// assert_eq!(under_a, under_b);
    /// # Ok::<(), warlock_engine::manifest::Error>(())
    /// ```
    pub fn new(
        root: impl AsRef<Path>,
        path: impl AsRef<Path>,
        scope: impl Into<String>,
    ) -> Result<Self, Error> {
        Ok(Self {
            path: to_manifest_path(root, path)?,
            scope: scope.into(),
        })
    }

    #[must_use]
    pub fn path(&self) -> &str {
        &self.path
    }

    #[must_use]
    pub fn scope(&self) -> &str {
        &self.scope
    }

    #[must_use]
    pub fn path_on_disk(&self, root: impl AsRef<Path>) -> PathBuf {
        from_manifest_path(root, &self.path)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScopeRecord {
    // All four are read back exactly as written, for the reason on
    // `ScopedDirectory::scope`: these bytes are committed and hand-edited, so
    // folding one on the way through would put a line in the diff of somebody
    // who only asked to scope something else. `name` is judged by `validate_scope` when
    // the manifest is read and never repaired; the other three are not judged at
    // all, because what a team key, a review state or a label may contain is
    // the tracker's business and not this crate's.
    name: String,
    // The field says what the value is — a Linear team key, `WAR` — and the
    // TOML key cannot follow it: `team` is what every committed manifest
    // already spells, those bytes are hand-edited, and `deny_unknown_fields`
    // above turns a renamed key into a refusal to read a manifest that was valid
    // yesterday.
    #[serde(rename = "team")]
    team_key: String,
    review_state: String,
    label: String,
}

impl ScopeRecord {
    /// ```
    /// use warlock_engine::ScopeRecord;
    ///
    /// let record = ScopeRecord::new("data-plane", "WAR", "In Review", "area/data");
    ///
    /// assert_eq!(record.name(), "data-plane");
    /// // Stored as spelled: no folding, no trimming, no judgement.
    /// assert_eq!(record.team_key(), "WAR");
    /// assert_eq!(record.review_state(), "In Review");
    /// assert_eq!(record.label(), "area/data");
    /// ```
    #[must_use]
    pub fn new(
        name: impl Into<String>,
        team_key: impl Into<String>,
        review_state: impl Into<String>,
        label: impl Into<String>,
    ) -> Self {
        Self {
            name: name.into(),
            team_key: team_key.into(),
            review_state: review_state.into(),
            label: label.into(),
        }
    }

    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    #[must_use]
    pub fn team_key(&self) -> &str {
        &self.team_key
    }

    #[must_use]
    pub fn review_state(&self) -> &str {
        &self.review_state
    }

    #[must_use]
    pub fn label(&self) -> &str {
        &self.label
    }
}

/// One `[[scope]]` record read as a scope that files to Linear.
///
/// ```
/// use warlock_engine::{LinearScope, ScopeRecord};
///
/// let record = ScopeRecord::new("data-plane", "WAR", "In Review", "area/data");
/// let linear = LinearScope::of(&record)?;
///
/// assert_eq!(linear.name(), "data-plane");
/// assert_eq!(linear.team_key(), "WAR");
/// assert_eq!(linear.review_state(), "In Review");
/// assert_eq!(linear.label(), "area/data");
/// # Ok::<(), warlock_engine::manifest::NotLinear>(())
/// ```
// Borrowed and with private fields, so [`of`](Self::of) is the only way to hold
// one: a `From`, a public field or a second constructor would let a caller
// assert a record files to Linear without that having been read off the record,
// which is the whole of what every flow taking this type is relying on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LinearScope<'r> {
    name: &'r str,
    team_key: &'r str,
    review_state: &'r str,
    label: &'r str,
}

impl<'r> LinearScope<'r> {
    /// The record seen as a Linear scope, or a refusal naming it.
    // Fallible with nothing here that can refuse, on purpose: every record is a
    // Linear one until `ScopeRecord` carries a tracker, and this is the one
    // place that will answer `Err` when one does. An infallible signature now is
    // a signature every caller would have to be rewritten against then, and the
    // sentence `NotLinear` prints is fixed from here.
    pub fn of(record: &'r ScopeRecord) -> Result<Self, NotLinear> {
        Ok(Self {
            name: &record.name,
            team_key: &record.team_key,
            review_state: &record.review_state,
            label: &record.label,
        })
    }

    #[must_use]
    pub const fn name(&self) -> &'r str {
        self.name
    }

    #[must_use]
    pub const fn team_key(&self) -> &'r str {
        self.team_key
    }

    #[must_use]
    pub const fn review_state(&self) -> &'r str {
        self.review_state
    }

    #[must_use]
    pub const fn label(&self) -> &'r str {
        self.label
    }
}

/// A scope whose record does not file to Linear, which is the only board
/// warlock can file to.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct NotLinear {
    scope: String,
}

// Ends in what it cost, because every caller of `LinearScope::of` is on its way
// to a board and this is the sentence that arrives instead: nothing was sent.
impl fmt::Display for NotLinear {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "`{}` files to Jira, and warlock can only file to Linear, so nothing was sent",
            self.scope
        )
    }
}

impl std::error::Error for NotLinear {}

/// ```
/// use std::path::Path;
/// use warlock_engine::to_manifest_path;
///
/// assert_eq!(to_manifest_path("/repo", Path::new("/repo").join("docs").join("adr"))?, "docs/adr");
/// assert_eq!(to_manifest_path("/repo", "./docs/adr")?, "docs/adr");
/// assert_eq!(to_manifest_path("/repo", "/repo")?, ".");
/// # Ok::<(), warlock_engine::manifest::Error>(())
/// ```
// Relative to the manifest's own directory, with forward slashes, because these
// bytes are committed: two clones of one commit at different absolute paths, on
// different operating systems, have to produce the same file.
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
        // `root` itself: the repository root can be scoped too.
        return Ok(ROOT_MODULE.to_owned());
    }
    Ok(parts.join("/"))
}

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

/// ```
/// use std::path::Path;
/// use warlock_engine::manifest_path;
///
/// assert_eq!(
///     manifest_path("/repo"),
///     Path::new("/repo").join(".warlock").join("scopes.toml"),
/// );
/// ```
#[must_use]
pub fn manifest_path(root: impl AsRef<Path>) -> PathBuf {
    root.as_ref().join(MANIFEST_DIR).join(MANIFEST_FILE)
}

// The leading dot is load-bearing, not cosmetic: the tree's walk skips hidden
// entries, so a temporary written beside its target never shows in the tree
// while it exists.
fn temp_file_name(target: &str) -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!(".{target}.{}.{n}.tmp", std::process::id())
}

pub(crate) fn write_atomically(
    dir: &Path,
    name: &str,
    bytes: &[u8],
) -> Result<(), (PathBuf, std::io::Error)> {
    write_atomically_with(dir, name, bytes, |_| Ok(()))
}

// The temporary must sit in the same directory as the target, so the rename
// cannot cross a filesystem and stop being atomic. `prepare` runs on the
// temporary before a byte is written into it. The error names the path that
// failed: the temporary when `prepare` or the write did, the target when the
// rename did.
pub(crate) fn write_atomically_with(
    dir: &Path,
    name: &str,
    bytes: &[u8],
    prepare: impl FnOnce(&Path) -> std::io::Result<()>,
) -> Result<(), (PathBuf, std::io::Error)> {
    let temp = dir.join(temp_file_name(name));
    let target = dir.join(name);
    let written = prepare(&temp)
        .and_then(|()| write_and_sync(&temp, bytes))
        .map_err(|source| (temp.clone(), source))
        .and_then(|()| fs::rename(&temp, &target).map_err(|source| (target, source)));
    if written.is_err() {
        drop(fs::remove_file(&temp));
    }
    written
}

fn write_and_sync(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let mut file = fs::File::create(path)?;
    file.write_all(bytes)?;
    // The `sync_all` is the point of this function: without it the rename that
    // follows can land before the contents do, and a crash in between leaves an
    // empty file where a good manifest used to be.
    file.sync_all()
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
        source: toml::de::Error,
    },
    UnsupportedVersion {
        found: i64,
        supported: u32,
    },
    Directory {
        index: usize,
        path: Option<String>,
        source: toml::de::Error,
    },
    Scope {
        path: Option<PathBuf>,
        index: usize,
        name: Option<String>,
        source: ScopeFault,
    },
    DuplicateScope {
        path: Option<PathBuf>,
        index: usize,
        name: String,
    },
    Serialize {
        source: toml::ser::Error,
    },
    PathOutsideRoot {
        root: PathBuf,
        path: PathBuf,
    },
    NonUtf8Path {
        path: PathBuf,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound { path } => {
                write!(f, "no scope manifest at `{}`", path.display())
            }
            Self::Io { path, source } => {
                write!(f, "could not read or write `{}`: {source}", path.display())
            }
            Self::Syntax { source } => write!(f, "malformed scope manifest: {source}"),
            Self::UnsupportedVersion { found, supported } => write!(
                f,
                "scope manifest schema version {found} is not supported; this build reads version {supported}"
            ),
            Self::Directory {
                index,
                path: Some(path),
                source,
            } => write!(
                f,
                "scoped directory {index} (`{path}`) is malformed: {source}"
            ),
            Self::Directory {
                index,
                path: None,
                source,
            } => write!(f, "scoped directory {index} is malformed: {source}"),
            Self::Scope {
                path,
                index,
                name,
                source,
            } => {
                write!(f, "scope record {index}")?;
                if let Some(name) = name {
                    write!(f, " (`{name}`)")?;
                }
                if let Some(path) = path {
                    write!(f, " in `{}`", path.display())?;
                }
                write!(f, " is refused: {source}")
            }
            Self::DuplicateScope { path, index, name } => {
                write!(f, "scope record {index}")?;
                if let Some(path) = path {
                    write!(f, " in `{}`", path.display())?;
                }
                write!(f, " repeats the name `{name}`")
            }
            Self::Serialize { source } => {
                write!(f, "could not write the scope manifest as TOML: {source}")
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
            Self::Syntax { source } | Self::Directory { source, .. } => Some(source),
            Self::Scope { source, .. } => Some(source),
            Self::Serialize { source } => Some(source),
            Self::NotFound { .. }
            | Self::UnsupportedVersion { .. }
            | Self::DuplicateScope { .. }
            | Self::PathOutsideRoot { .. }
            | Self::NonUtf8Path { .. } => None,
        }
    }
}

// Two ways one record can be turned down, kept apart rather than flattened into
// a string: a key that is missing or wrongly typed is the TOML reader's finding,
// and a name the character rule refuses is `validate_scope`'s. A caller that
// wants to tell them apart can, and neither has to be reconstructed by reading
// the message.
#[derive(Debug)]
pub enum ScopeFault {
    // Boxed because `toml::de::Error` is 88 bytes on its own, and this variant
    // also carries a path and a name: inline it and `manifest::Error` grows past
    // the size at which every `Result` in the workspace that can fail this way
    // is paying for the failure. `Error::Directory` sits right at that line
    // already.
    Malformed(Box<toml::de::Error>),
    Refused(Rule),
}

impl fmt::Display for ScopeFault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Malformed(source) => write!(f, "{source}"),
            Self::Refused(rule) => write!(f, "{rule}"),
        }
    }
}

impl std::error::Error for ScopeFault {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Malformed(source) => Some(source),
            Self::Refused(rule) => Some(rule),
        }
    }
}

// Reading in two passes — the version as whatever integer the file says, the
// directories and scope records still raw — is what buys the error variants
// that name something: the version before it is trusted, and each row or record
// by its index and by the name it did spell. Deserialising straight into
// `Manifest` instead would give one blanket "this file is wrong".
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawManifest {
    version: i64,
    #[serde(default)]
    directory: Vec<toml::Value>,
    #[serde(default)]
    scope: Vec<toml::Value>,
}

// Looks redundant beside the check in `Manifest::read` and is not: this is the
// derived path, and without it `toml::from_str::<Manifest>` would quietly accept
// a version `Manifest::read` refuses.
fn deserialize_version<'de, D: Deserializer<'de>>(deserializer: D) -> Result<u32, D::Error> {
    let version = u32::deserialize(deserializer)?;
    if version == SCHEMA_VERSION {
        Ok(version)
    } else {
        Err(serde::de::Error::custom(format!(
            "scope manifest schema version {version} is not supported; this build reads version {SCHEMA_VERSION}"
        )))
    }
}

#[cfg(test)]
#[path = "tests/manifest.rs"]
mod tests;
