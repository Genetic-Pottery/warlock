use std::fmt;
use std::fs;
use std::io::Write as _;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Deserializer, Serialize};

const MANIFEST_DIR: &str = ".warlock";

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
}

impl Manifest {
    #[must_use]
    pub fn new() -> Self {
        Self {
            version: SCHEMA_VERSION,
            entries: Vec::new(),
        }
    }

    #[must_use]
    pub fn with_entries(entries: impl IntoIterator<Item = PactEntry>) -> Self {
        Self {
            version: SCHEMA_VERSION,
            entries: entries.into_iter().collect(),
        }
    }

    #[must_use]
    pub const fn version(&self) -> u32 {
        self.version
    }

    #[must_use]
    pub fn entries(&self) -> &[PactEntry] {
        &self.entries
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

        // The temporary must sit in the same directory as the target, so the
        // rename below cannot cross a filesystem and stops being atomic.
        let temp = dir.join(temp_file_name(MANIFEST_FILE));
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
            Ok(text) => Self::from_toml_str(&text),
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
        Ok(Self {
            module: to_manifest_path(root, module)?,
            document: to_manifest_path(root, document)?,
            scope: None,
            granted_hash: None,
            granted_at: None,
            carry_hash: None,
        })
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

    #[must_use]
    pub fn granted_at(&self) -> Option<&str> {
        self.granted_at.as_deref()
    }

    #[must_use]
    pub fn is_judged(&self) -> bool {
        self.granted_hash.is_some()
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

// Reading in two passes — the version as whatever integer the file says, the
// entries still raw — is what buys the two error variants that name something:
// the version before it is trusted, and each entry by index. Deserialising
// straight into `Manifest` instead would give one blanket "this file is wrong".
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawManifest {
    version: i64,
    #[serde(default)]
    pact: Vec<toml::Value>,
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
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};

    use super::{
        Error, MANIFEST_FILE, Manifest, PactEntry, SCHEMA_VERSION, from_manifest_path,
        manifest_path, to_manifest_path,
    };

    fn unjudged() -> PactEntry {
        PactEntry::new(
            ".",
            "crates/warlock-engine",
            "crates/warlock-engine/WARLOCK.md",
        )
        .expect("a relative path inside the root is storable")
    }

    fn judged() -> PactEntry {
        unjudged().with_grant("d0f5a1", "2026-08-19T07:32:00Z")
    }

    fn a_de_error() -> toml::de::Error {
        toml::from_str::<Manifest>("version = \"one\"").expect_err("a string is not an integer")
    }

    fn a_root() -> tempfile::TempDir {
        tempfile::tempdir().expect("a temporary directory")
    }

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
    fn an_unscoped_entry_omits_the_scope_key() {
        let manifest = Manifest::with_entries([unjudged(), judged()]);
        let text = manifest.to_toml_string().expect("serialises");

        assert!(!text.contains("scope"), "{text}");
        assert_eq!(
            text,
            concat!(
                "version = 1\n\n",
                "[[pact]]\n",
                "module = \"crates/warlock-engine\"\n",
                "document = \"crates/warlock-engine/WARLOCK.md\"\n\n",
                "[[pact]]\n",
                "module = \"crates/warlock-engine\"\n",
                "document = \"crates/warlock-engine/WARLOCK.md\"\n",
                "granted_hash = \"d0f5a1\"\n",
                "granted_at = \"2026-08-19T07:32:00Z\"\n",
            ),
            "an entry with no scope serialises to exactly the bytes it did \
             before the field existed"
        );

        let parsed = Manifest::from_toml_str(&text).expect("parses");
        assert_eq!(parsed, manifest);
        assert_eq!(parsed.entries()[0].scope(), None);
        assert_eq!(parsed.entries()[1].scope(), None);
    }

    #[test]
    fn a_manifest_written_before_scopes_existed_loads_unscoped() {
        // Byte for byte the shape a previous build wrote: no `scope` key
        // anywhere, and no version bump to say the schema moved.
        let root = a_root();
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

        let loaded = Manifest::load(root.path()).expect("loads without a scope key in sight");
        assert_eq!(loaded.version(), SCHEMA_VERSION);
        assert!(
            loaded.entries().iter().all(|entry| entry.scope().is_none()),
            "every entry reads as unscoped"
        );

        loaded.save(root.path()).expect("saves");
        assert_eq!(
            fs::read_to_string(manifest_path(root.path())).expect("reads"),
            original,
            "and saving it back does not add the new key"
        );
    }

    #[test]
    fn a_scoped_entry_round_trips_byte_for_byte() {
        let root = a_root();
        // Hand-written, so this says where the key sits in the file and not
        // merely that the serialiser agrees with itself.
        let original = concat!(
            "version = 1\n\n",
            "[[pact]]\n",
            "module = \"crates/warlock-engine\"\n",
            "document = \"crates/warlock-engine/WARLOCK.md\"\n",
            "scope = \"data-plane\"\n",
            "granted_hash = \"d0f5a1\"\n",
            "granted_at = \"2026-08-19T07:32:00Z\"\n",
        );
        hand_write(root.path(), original);

        let loaded = Manifest::load(root.path()).expect("loads");
        assert_eq!(
            loaded,
            Manifest::with_entries([judged().with_scope("data-plane")]),
            "the file and the constructed entry are the same manifest"
        );
        assert_eq!(loaded.entries()[0].scope(), Some("data-plane"));

        loaded.save(root.path()).expect("saves");
        assert_eq!(
            fs::read_to_string(manifest_path(root.path())).expect("reads"),
            original,
            "a load-then-save is a no-op on the bytes of a scoped entry too"
        );
    }

    #[test]
    fn an_invalid_scope_loads_untouched_and_is_written_back_unchanged() {
        // The reader validates nothing and normalises nothing: these bytes are
        // committed, and correcting them on somebody's next save would put a
        // line in a diff they did not author. Whether a scope is well formed is
        // `validate_scope`'s question, asked elsewhere.
        let invalid = [
            "",
            "1data",
            "data-",
            "*",
            "abcdefghijklmnopqrstuvwxy", // 25 characters
            "données",
            "Data-Plane",
        ];

        for scope in invalid {
            let root = a_root();
            let original = format!(
                concat!(
                    "version = 1\n\n",
                    "[[pact]]\n",
                    "module = \"crates/warlock-engine\"\n",
                    "document = \"crates/warlock-engine/WARLOCK.md\"\n",
                    "scope = \"{}\"\n",
                    "granted_hash = \"d0f5a1\"\n",
                    "granted_at = \"2026-08-19T07:32:00Z\"\n",
                ),
                scope
            );
            hand_write(root.path(), &original);

            let loaded = Manifest::load(root.path())
                .unwrap_or_else(|error| panic!("`{scope}` is not fatal, got {error:?}"));
            let entry = &loaded.entries()[0];
            assert_eq!(entry.module(), "crates/warlock-engine", "for `{scope}`");
            assert_eq!(
                entry.document(),
                "crates/warlock-engine/WARLOCK.md",
                "for `{scope}`"
            );
            assert_eq!(entry.granted_hash(), Some("d0f5a1"), "for `{scope}`");
            assert_eq!(
                entry.granted_at(),
                Some("2026-08-19T07:32:00Z"),
                "for `{scope}`"
            );
            assert_eq!(
                entry.scope(),
                Some(scope),
                "read back as written, not repaired and not dropped"
            );

            loaded.save(root.path()).expect("saves");
            assert_eq!(
                fs::read_to_string(manifest_path(root.path())).expect("reads"),
                original,
                "the invalid line survives a load-then-save byte for byte"
            );
        }
    }

    #[test]
    fn setting_or_clearing_a_scope_leaves_the_rest_of_the_entry_alone() {
        let before = judged();
        let scoped = before.clone().with_scope("data-plane");
        let rescoped = scoped.clone().with_scope("billing");
        let unscoped = rescoped.clone().without_scope();

        for entry in [&scoped, &rescoped, &unscoped] {
            assert_eq!(entry.module(), before.module());
            assert_eq!(entry.document(), before.document());
            assert_eq!(entry.granted_hash(), before.granted_hash());
            assert_eq!(entry.granted_at(), before.granted_at());
            assert!(entry.is_judged());
        }

        assert_eq!(scoped.scope(), Some("data-plane"));
        assert_eq!(rescoped.scope(), Some("billing"), "one scope, replaced");
        assert_eq!(unscoped.scope(), None);
        // Clearing a scope that was never set is the entry it started as.
        assert_eq!(unscoped, before);
        assert_eq!(before.clone().without_scope(), before);
    }

    #[test]
    fn a_scope_is_stored_as_written_however_odd() {
        // No folding, no trimming, no rejection: `with_scope` is the store, not
        // the gate. Two spellings staying two spellings is exactly why the
        // folding belongs where a person types.
        for scope in ["Data-Plane", " data-plane ", "*", ""] {
            assert_eq!(judged().with_scope(scope).scope(), Some(scope));
        }
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
