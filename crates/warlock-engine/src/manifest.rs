use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::io::Write as _;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Deserializer, Serialize};

use crate::scope::{Rule, validate_scope};
use crate::walk::MANIFEST_DIR;

const MANIFEST_FILE: &str = "pacts.toml";

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
    #[serde(rename = "pact", default, skip_serializing_if = "Vec::is_empty")]
    entries: Vec<PactEntry>,
    // After `entries` so the `[[scope]]` tables follow the `[[pact]]` rows, and
    // both after `version`: TOML puts every scalar before the first table, so a
    // field declared above `version` would move the version line into whichever
    // table happened to come first.
    #[serde(rename = "scope", default, skip_serializing_if = "Vec::is_empty")]
    scopes: Vec<ScopeRecord>,
}

impl Manifest {
    #[must_use]
    pub fn new() -> Self {
        Self {
            version: SCHEMA_VERSION,
            entries: Vec::new(),
            scopes: Vec::new(),
        }
    }

    #[must_use]
    pub fn with_entries(entries: impl IntoIterator<Item = PactEntry>) -> Self {
        Self {
            version: SCHEMA_VERSION,
            entries: entries.into_iter().collect(),
            scopes: Vec::new(),
        }
    }

    /// Rebuild around a new set of `[[pact]]` rows, carrying the `[[scope]]`
    /// records across as they stand.
    ///
    /// ```
    /// use warlock_engine::{Manifest, PactEntry, ScopeRecord};
    ///
    /// let manifest = Manifest::with_entries([PactEntry::new(".", "crates/engine", "crates/engine/WARLOCK.md")?])
    ///     .with_scopes([ScopeRecord::new("data-plane", "Data Plane", "In Review", "area/data-plane")]);
    ///
    /// // Every pact gone, and the record that no pact now names is still there.
    /// let emptied = manifest.rebuilt_with([]);
    /// assert!(emptied.entries().is_empty());
    /// assert_eq!(emptied.scopes(), manifest.scopes());
    /// # Ok::<(), warlock_engine::manifest::Error>(())
    /// ```
    // The counterpart to `with_entries` for every caller that has a manifest in
    // hand: that one is an associated function with nothing to copy from, so a
    // rebuild written through it silently drops the records. Records are not
    // pruned to what the new entries name — a record outlives the pacts that
    // spelled its scope by design, and un-pacting the last directory under a
    // scope would otherwise delete where that work is filed.
    #[must_use]
    pub fn rebuilt_with(&self, entries: impl IntoIterator<Item = PactEntry>) -> Self {
        Self {
            version: self.version,
            entries: entries.into_iter().collect(),
            scopes: self.scopes.clone(),
        }
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
    pub fn entries(&self) -> &[PactEntry] {
        &self.entries
    }

    #[must_use]
    pub fn scopes(&self) -> &[ScopeRecord] {
        &self.scopes
    }

    pub fn push(&mut self, entry: PactEntry) {
        self.entries.push(entry);
    }

    #[must_use]
    pub fn entry(&self, module: &str) -> Option<&PactEntry> {
        self.entries.iter().find(|entry| entry.module == module)
    }

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

        let entries = raw
            .pact
            .into_iter()
            .enumerate()
            .map(|(index, value)| {
                // Read the module path out of the raw table first: if the entry
                // fails to parse, this is what makes the error nameable.
                let module = value
                    .get("module")
                    .and_then(toml::Value::as_str)
                    .map(ToOwned::to_owned);
                value
                    .try_into::<PactEntry>()
                    .map_err(|source| Error::Entry {
                        index,
                        module,
                        source,
                    })
            })
            .collect::<Result<Vec<_>, _>>()?;

        let mut scopes: Vec<ScopeRecord> = Vec::new();
        for (index, value) in raw.scope.into_iter().enumerate() {
            // Same reason as the module above: read the name out of the raw
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
            entries,
            scopes,
        })
    }

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
    pub fn save(&self, root: impl AsRef<Path>) -> Result<(), Error> {
        // Serialise before touching the filesystem: a manifest that cannot be
        // written as TOML should not leave a new directory behind.
        let text = self.to_toml_string()?;

        let dir = root.as_ref().join(MANIFEST_DIR);
        fs::create_dir_all(&dir).map_err(|source| Error::Io {
            path: dir.clone(),
            source,
        })?;

        replace_atomically(&dir, MANIFEST_FILE, text.as_bytes(), None)
            .map(drop)
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
    // was the rejected alternative: "this repository has never pacted anything"
    // and "this repository pacted nothing" are different facts, and only the
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
pub struct PactEntry {
    module: String,
    document: String,
    // Read back exactly as written, never validated or folded here. These bytes
    // are committed, so normalising a hand-edited scope on the way through would
    // put a line in somebody's diff on a save they only asked for because they
    // pacted something else. `validate_scope` is where a caller asks.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    scope: Option<String>,
    // The absent key is the whole representation of "never judged" — no sentinel
    // hash, no empty string, no `judged = false`. An entry with no grant
    // round-trips with the keys still missing, and unjudged is already stale, so
    // nothing is lost by saying nothing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    granted_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    granted_at: Option<String>,
    // Safety-critical rather than a cache: a run reads it to decide it need not
    // pay for a pass, so an answer that said "unchanged" when the inputs had
    // moved would grant freshness no pass earned. It lives beside the grant so
    // the same atomic save writes both and they cannot drift. Absent means no
    // shortcut and the pass runs, which is why every pre-carry-hash entry is
    // safe to read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    carry_hash: Option<String>,
    // One hash per file the document holds a line for, so a run can re-ask
    // about the files that moved and keep the lines of the files that did not.
    // Absent is "no lines recorded", which is every entry written before
    // per-file granularity and every entry a whole-directory pass writes:
    // nothing is reusable, so the pass runs, exactly as `carry_hash` reads when
    // it is missing.
    //
    // In the entry rather than in a store of its own because a line and its
    // grant have to be written by the same atomic save. Two files would drift,
    // and a line believed current against a hash from another save is worse
    // than no shortcut at all.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    lines: Option<BTreeMap<String, String>>,
}

impl PactEntry {
    /// ```
    /// use warlock_engine::PactEntry;
    ///
    /// let under_a = PactEntry::new("/tmp/a", "/tmp/a/crates/engine", "/tmp/a/crates/engine/WARLOCK.md")?;
    /// let under_b = PactEntry::new("/tmp/b", "/tmp/b/crates/engine", "/tmp/b/crates/engine/WARLOCK.md")?;
    ///
    /// assert_eq!(under_a.module(), "crates/engine");
    /// assert_eq!(under_a, under_b);
    /// assert_eq!(under_a.granted_hash(), None);
    /// # Ok::<(), warlock_engine::manifest::Error>(())
    /// ```
    pub fn new(
        root: impl AsRef<Path>,
        module: impl AsRef<Path>,
        document: impl AsRef<Path>,
    ) -> Result<Self, Error> {
        let root = root.as_ref();
        Ok(Self::stored(
            to_manifest_path(root, module)?,
            to_manifest_path(root, document)?,
        ))
    }

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

    #[must_use]
    pub fn with_carry_hash(mut self, carry_hash: impl Into<String>) -> Self {
        self.carry_hash = Some(carry_hash.into());
        self
    }

    #[must_use]
    pub fn with_lines(mut self, lines: BTreeMap<String, String>) -> Self {
        self.lines = Some(lines);
        self
    }

    pub(crate) fn set_lines(&mut self, lines: BTreeMap<String, String>) {
        self.lines = Some(lines);
    }

    #[must_use]
    pub fn without_grant(mut self) -> Self {
        self.granted_hash = None;
        self.granted_at = None;
        self.carry_hash = None;
        self
    }

    /// ```
    /// use warlock_engine::PactEntry;
    ///
    /// let entry = PactEntry::new(".", "crates/engine", "crates/engine/WARLOCK.md")?
    ///     .with_grant("d0f5a1", "2026-08-19T07:32:00Z")
    ///     .with_scope("data-plane");
    ///
    /// assert_eq!(entry.scope(), Some("data-plane"));
    /// assert_eq!(entry.granted_hash(), Some("d0f5a1"));
    /// assert_eq!(entry.clone().without_scope().scope(), None);
    /// assert_eq!(entry.without_scope().granted_hash(), Some("d0f5a1"));
    /// # Ok::<(), warlock_engine::manifest::Error>(())
    /// ```
    #[must_use]
    pub fn with_scope(mut self, scope: impl Into<String>) -> Self {
        self.scope = Some(scope.into());
        self
    }

    #[must_use]
    pub fn without_scope(mut self) -> Self {
        self.scope = None;
        self
    }

    pub(crate) fn stored(module: String, document: String) -> Self {
        Self {
            module,
            document,
            scope: None,
            granted_hash: None,
            granted_at: None,
            carry_hash: None,
            lines: None,
        }
    }

    // A mutation rather than a fresh entry to swap in, so that a field a person
    // owns survives every pact and refresh without anyone remembering to carry
    // it across: `scope` is not a parameter and is not assigned below, so no run
    // can write one and no run can clear one. The grant travels as one triple
    // for the same reason — a carry digest left behind by a cleared grant is a
    // stale answer that could later skip a pass that was owed.
    pub(crate) fn overwrite_run_fields(
        &mut self,
        module: String,
        document: String,
        grant: Option<(String, String, Option<String>)>,
    ) {
        let (granted_hash, granted_at, carry_hash) = match grant {
            Some((hash, at, carry)) => (Some(hash), Some(at), carry),
            None => (None, None, None),
        };
        self.module = module;
        self.document = document;
        self.granted_hash = granted_hash;
        self.granted_at = granted_at;
        self.carry_hash = carry_hash;
        // The run wrote the document, so whatever lines were recorded against
        // the last one describe a file that is no longer on the page. A caller
        // that wrote per-file lines records them itself afterwards; everything
        // else leaves the entry saying nothing is reusable, which is the only
        // honest thing it can say.
        self.lines = None;
    }

    #[must_use]
    pub fn module(&self) -> &str {
        &self.module
    }

    #[must_use]
    pub fn document(&self) -> &str {
        &self.document
    }

    #[must_use]
    pub fn scope(&self) -> Option<&str> {
        self.scope.as_deref()
    }

    #[must_use]
    pub fn module_path(&self, root: impl AsRef<Path>) -> PathBuf {
        from_manifest_path(root, &self.module)
    }

    #[must_use]
    pub fn document_path(&self, root: impl AsRef<Path>) -> PathBuf {
        from_manifest_path(root, &self.document)
    }

    #[must_use]
    pub fn granted_hash(&self) -> Option<&str> {
        self.granted_hash.as_deref()
    }

    #[must_use]
    pub fn carry_hash(&self) -> Option<&str> {
        self.carry_hash.as_deref()
    }

    /// The hash each file had when its line was last written, or `None` where
    /// no line was ever recorded against a file.
    #[must_use]
    pub fn lines(&self) -> Option<&BTreeMap<String, String>> {
        self.lines.as_ref()
    }

    #[must_use]
    pub fn granted_at(&self) -> Option<&str> {
        self.granted_at.as_deref()
    }

    #[must_use]
    pub fn is_judged(&self) -> bool {
        self.granted_hash.is_some()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScopeRecord {
    // All four are read back exactly as written, for the reason on
    // `PactEntry::scope`: these bytes are committed and hand-edited, so folding
    // one on the way through would put a line in the diff of somebody who only
    // asked to pact something else. `name` is judged by `validate_scope` when
    // the manifest is read and never repaired; the other three are not judged at
    // all, because what a team slug, a review state or a label may contain is
    // the tracker's business and not this crate's.
    name: String,
    team: String,
    review_state: String,
    label: String,
}

impl ScopeRecord {
    /// ```
    /// use warlock_engine::ScopeRecord;
    ///
    /// let record = ScopeRecord::new("data-plane", "Data Plane", "In Review", "area/data");
    ///
    /// assert_eq!(record.name(), "data-plane");
    /// // Stored as spelled: no folding, no trimming, no judgement.
    /// assert_eq!(record.team(), "Data Plane");
    /// assert_eq!(record.review_state(), "In Review");
    /// assert_eq!(record.label(), "area/data");
    /// ```
    #[must_use]
    pub fn new(
        name: impl Into<String>,
        team: impl Into<String>,
        review_state: impl Into<String>,
        label: impl Into<String>,
    ) -> Self {
        Self {
            name: name.into(),
            team: team.into(),
            review_state: review_state.into(),
            label: label.into(),
        }
    }

    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    #[must_use]
    pub fn team(&self) -> &str {
        &self.team
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
        // `root` itself: the repository root can be a pacted module too.
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
///     Path::new("/repo").join(".warlock").join("pacts.toml"),
/// );
/// ```
#[must_use]
pub fn manifest_path(root: impl AsRef<Path>) -> PathBuf {
    root.as_ref().join(MANIFEST_DIR).join(MANIFEST_FILE)
}

// The leading dot is load-bearing, not cosmetic: the `ignore` walks the rest of
// the crate is built on skip hidden entries, so a temporary sitting in a module
// directory cannot reach a tree, a subtree hash or a pact request while it
// exists. Naming it without the dot would let a half-written file be hashed.
pub(crate) fn temp_file_name(target: &str) -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!(".{target}.{}.{n}.tmp", std::process::id())
}

pub(crate) fn write_and_sync(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let mut file = fs::File::create(path)?;
    file.write_all(bytes)?;
    // The `sync_all` is the point of this function: without it the rename that
    // follows can land before the contents do, and a crash in between leaves an
    // empty file where a good manifest used to be.
    file.sync_all()
}

// Every durable file this crate writes goes through here, and the durability is
// in the sequence rather than in any one step: the temporary is built in `dir`
// so the rename cannot cross a filesystem and stop being atomic, and it is
// removed on the way out of a failure so a crash cannot leave a dot file
// standing where the next run will find it. A caller that spells the sequence
// out again gets to forget one of those, and both are silent.
//
// `prepare` runs against the temporary before anything is written to it, which
// is the only point at which a mode can be set without leaving a window where
// the real contents sit at whatever the umask allows; `rename` then carries the
// mode to the target. See `keys::owner_only`, the one caller that passes it.
//
// The error names the path the failure happened on — the temporary for a write,
// the target for a rename — because a caller reporting a path chooses between
// the two: the stores name whichever one failed, and `CLAUDE.md` and
// `WARLOCK.md` name the target whatever happened, on the grounds that how a
// document got written is not the caller's business.
pub(crate) fn replace_atomically(
    dir: &Path,
    name: &str,
    bytes: &[u8],
    prepare: Option<fn(&Path) -> std::io::Result<()>>,
) -> Result<PathBuf, (PathBuf, std::io::Error)> {
    let target = dir.join(name);
    let temp = dir.join(temp_file_name(name));

    let written = prepare
        .map_or(Ok(()), |prepare| prepare(&temp))
        .and_then(|()| write_and_sync(&temp, bytes))
        .map_err(|source| (temp.clone(), source))
        .and_then(|()| fs::rename(&temp, &target).map_err(|source| (target.clone(), source)));

    match written {
        Ok(()) => Ok(target),
        Err(failure) => {
            // Best effort: the caller is already being told the file was not
            // written, and a stray dot file is invisible to every walk here.
            drop(fs::remove_file(&temp));
            Err(failure)
        }
    }
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
    Entry {
        index: usize,
        module: Option<String>,
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
    // is paying for the failure. `Error::Entry` sits right at that line already.
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
// pact entries and scope records still raw — is what buys the error variants
// that name something: the version before it is trusted, and each entry or
// record by its index and by the name it did spell. Deserialising straight into
// `Manifest` instead would give one blanket "this file is wrong".
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawManifest {
    version: i64,
    #[serde(default)]
    pact: Vec<toml::Value>,
    #[serde(default)]
    scope: Vec<toml::Value>,
}

// Looks redundant beside the check in `from_toml_str` and is not: this is the
// derived path, and without it `toml::from_str::<Manifest>` would quietly accept
// a version `from_toml_str` refuses.
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
#[path = "tests/manifest.rs"]
mod tests;
