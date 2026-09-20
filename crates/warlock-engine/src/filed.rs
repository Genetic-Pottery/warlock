use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Deserializer, Serialize};

use crate::manifest::{self, temp_file_name, to_manifest_path, write_and_sync};
use crate::walk::MANIFEST_DIR;

const FILED_FILE: &str = "filed.toml";

// Counted apart from the manifest's version on purpose: the two files are saved
// by different commands and one gaining a key is no reason to restale the
// other. A file declaring any other version is refused rather than read as if
// it were this one, for the reason `manifest::SCHEMA_VERSION` gives — an old
// binary that guesses at a newer file rewrites it with less than it came with,
// and what it would drop here is the address of a project that exists.
pub const SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Filed {
    #[serde(deserialize_with = "deserialize_version")]
    version: u32,
    #[serde(rename = "filed", default, skip_serializing_if = "Vec::is_empty")]
    records: Vec<FiledRecord>,
}

impl Filed {
    #[must_use]
    pub fn new() -> Self {
        Self {
            version: SCHEMA_VERSION,
            records: Vec::new(),
        }
    }

    #[must_use]
    pub fn with_records(records: impl IntoIterator<Item = FiledRecord>) -> Self {
        Self {
            version: SCHEMA_VERSION,
            records: records.into_iter().collect(),
        }
    }

    #[must_use]
    pub const fn version(&self) -> u32 {
        self.version
    }

    #[must_use]
    pub fn records(&self) -> &[FiledRecord] {
        &self.records
    }

    // A path already recorded is not refused here, and appending one is not an
    // error: this module never decides that a second push is a mistake, it only
    // answers what is recorded through `record` below. The refusal belongs to
    // the caller that is about to open a socket, because it is the one that can
    // print the recorded URL instead.
    pub fn push(&mut self, record: FiledRecord) {
        self.records.push(record);
    }

    /// ```
    /// use warlock_engine::{Filed, FiledRecord};
    ///
    /// let record = FiledRecord::new(
    ///     ".",
    ///     "docs/brief.md",
    ///     "b229262b-22aa-444a-a8af-0a2a3f4ef100",
    ///     "https://linear.app/acme/project/brief",
    ///     "warlock-team",
    ///     "WAR",
    ///     "2026-09-20T07:32:00Z",
    /// )?;
    /// let mut filed = Filed::new();
    /// filed.push(record);
    ///
    /// assert_eq!(
    ///     filed.record("docs/brief.md").map(FiledRecord::url),
    ///     Some("https://linear.app/acme/project/brief"),
    /// );
    /// assert_eq!(filed.record("docs/other.md"), None);
    /// # Ok::<(), warlock_engine::filed::Error>(())
    /// ```
    #[must_use]
    pub fn record(&self, path: &str) -> Option<&FiledRecord> {
        self.records.iter().find(|record| record.path == path)
    }

    /// ```
    /// use warlock_engine::Filed;
    ///
    /// let toml = Filed::new().to_toml_string()?;
    ///
    /// assert_eq!(toml, "version = 1\n");
    /// # Ok::<(), warlock_engine::filed::Error>(())
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
        let at = || path.map(Path::to_path_buf);

        let raw: RawFiled =
            toml::from_str(text).map_err(|source| Error::Syntax { path: at(), source })?;

        if u32::try_from(raw.version) != Ok(SCHEMA_VERSION) {
            return Err(Error::UnsupportedVersion {
                path: at(),
                found: raw.version,
                supported: SCHEMA_VERSION,
            });
        }

        let records = raw
            .filed
            .into_iter()
            .enumerate()
            .map(|(index, value)| {
                // Read the brief's path out of the raw table first: if the
                // record fails to parse, this is what makes the error nameable.
                let brief = value
                    .get("path")
                    .and_then(toml::Value::as_str)
                    .map(ToOwned::to_owned);
                value
                    .try_into::<FiledRecord>()
                    .map_err(|source| Error::Record {
                        file: at(),
                        index,
                        brief,
                        source: Box::new(source),
                    })
            })
            .collect::<Result<Vec<_>, _>>()?;

        Ok(Self {
            version: SCHEMA_VERSION,
            records,
        })
    }

    /// ```
    /// use warlock_engine::{Filed, FiledRecord};
    ///
    /// let root = tempfile::tempdir()?;
    /// let record = FiledRecord::new(
    ///     root.path(),
    ///     root.path().join("docs").join("brief.md"),
    ///     "b229262b-22aa-444a-a8af-0a2a3f4ef100",
    ///     "https://linear.app/acme/project/brief",
    ///     "warlock-team",
    ///     "WAR",
    ///     "2026-09-20T07:32:00Z",
    /// )?;
    /// let filed = Filed::with_records([record]);
    ///
    /// filed.save(root.path())?;
    /// assert_eq!(Filed::load(root.path())?, filed);
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn save(&self, root: impl AsRef<Path>) -> Result<(), Error> {
        // Serialise before touching the filesystem: records that cannot be
        // written as TOML should not leave a new directory behind.
        let text = self.to_toml_string()?;

        let dir = root.as_ref().join(MANIFEST_DIR);
        fs::create_dir_all(&dir).map_err(|source| Error::Io {
            path: dir.clone(),
            source,
        })?;

        // The temporary must sit in the same directory as the target, so the
        // rename below cannot cross a filesystem and stops being atomic.
        let temp = dir.join(temp_file_name(FILED_FILE));
        let target = dir.join(FILED_FILE);

        let written = write_and_sync(&temp, text.as_bytes())
            .map_err(|source| Error::Io {
                path: temp.clone(),
                source,
            })
            .and_then(|()| {
                fs::rename(&temp, &target).map_err(|source| Error::Io {
                    path: target,
                    source,
                })
            });

        if written.is_err() {
            drop(fs::remove_file(&temp));
        }
        written
    }

    /// ```
    /// use warlock_engine::{filed, Filed};
    ///
    /// let root = tempfile::tempdir()?;
    /// let filed = match Filed::load(root.path()) {
    ///     Err(filed::Error::NotFound { .. }) => Filed::new(),
    ///     other => other?,
    /// };
    ///
    /// assert!(filed.records().is_empty());
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    // Absent is not empty, following `Manifest::load` and `load_sigils`: "this
    // repository has never filed anything" and "this repository records no
    // filing" are different facts, and only the caller knows which one it means
    // to act on. The doctest above is the caller that wants them to be the same
    // fact, written out.
    pub fn load(root: impl AsRef<Path>) -> Result<Self, Error> {
        let path = filed_path(root);
        match fs::read_to_string(&path) {
            Ok(text) => Self::read(&text, Some(&path)),
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => {
                Err(Error::NotFound { path })
            }
            Err(source) => Err(Error::Io { path, source }),
        }
    }
}

impl Default for Filed {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FiledRecord {
    // Relative to the repository root with forward slashes, through
    // `to_manifest_path`, for the reason that function gives: these bytes are
    // committed, so two clones of one commit at different absolute paths on
    // different operating systems have to produce the same file.
    path: String,
    // All five are read back exactly as written, for the reason on
    // `PactEntry::scope` and `ScopeRecord`: they are committed and hand-edited,
    // so folding one on the way through would put a line in the diff of
    // somebody who only asked to file something else. Nothing here judges a
    // scope name either — what a tracker's id, URL, team key or timestamp may
    // contain is the tracker's business and not this crate's.
    project_id: String,
    url: String,
    scope: String,
    team: String,
    // Supplied by the caller and never read from a clock in this module, the
    // way the home is a parameter through `sigils.rs`. `clock::now_rfc3339`
    // called here would make every test of this module depend on the wall clock
    // and would take the instant away from the caller, which is the one that
    // knows whether the project was created a moment ago or an hour ago.
    filed_at: String,
}

impl FiledRecord {
    /// ```
    /// use warlock_engine::FiledRecord;
    ///
    /// let under_a = FiledRecord::new(
    ///     "/tmp/a",
    ///     "/tmp/a/docs/brief.md",
    ///     "b229262b",
    ///     "https://linear.app/acme/project/brief",
    ///     "warlock-team",
    ///     "WAR",
    ///     "2026-09-20T07:32:00Z",
    /// )?;
    /// let under_b = FiledRecord::new(
    ///     "/tmp/b",
    ///     "/tmp/b/docs/brief.md",
    ///     "b229262b",
    ///     "https://linear.app/acme/project/brief",
    ///     "warlock-team",
    ///     "WAR",
    ///     "2026-09-20T07:32:00Z",
    /// )?;
    ///
    /// assert_eq!(under_a.path(), "docs/brief.md");
    /// assert_eq!(under_a, under_b);
    /// # Ok::<(), warlock_engine::filed::Error>(())
    /// ```
    pub fn new(
        root: impl AsRef<Path>,
        path: impl AsRef<Path>,
        project_id: impl Into<String>,
        url: impl Into<String>,
        scope: impl Into<String>,
        team: impl Into<String>,
        filed_at: impl Into<String>,
    ) -> Result<Self, Error> {
        Ok(Self {
            path: to_manifest_path(root, path).map_err(|source| Error::Path {
                source: Box::new(source),
            })?,
            project_id: project_id.into(),
            url: url.into(),
            scope: scope.into(),
            team: team.into(),
            filed_at: filed_at.into(),
        })
    }

    #[must_use]
    pub fn path(&self) -> &str {
        &self.path
    }

    #[must_use]
    pub fn project_id(&self) -> &str {
        &self.project_id
    }

    #[must_use]
    pub fn url(&self) -> &str {
        &self.url
    }

    #[must_use]
    pub fn scope(&self) -> &str {
        &self.scope
    }

    #[must_use]
    pub fn team(&self) -> &str {
        &self.team
    }

    #[must_use]
    pub fn filed_at(&self) -> &str {
        &self.filed_at
    }
}

/// ```
/// use std::path::Path;
/// use warlock_engine::filed_path;
///
/// assert_eq!(
///     filed_path("/repo"),
///     Path::new("/repo").join(".warlock").join("filed.toml"),
/// );
/// ```
#[must_use]
pub fn filed_path(root: impl AsRef<Path>) -> PathBuf {
    root.as_ref().join(MANIFEST_DIR).join(FILED_FILE)
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
        path: Option<PathBuf>,
        source: toml::de::Error,
    },
    UnsupportedVersion {
        path: Option<PathBuf>,
        found: i64,
        supported: u32,
    },
    Record {
        file: Option<PathBuf>,
        index: usize,
        brief: Option<String>,
        // Boxed for the reason `ScopeFault::Malformed` gives: `toml::de::Error`
        // is large on its own, and this variant also carries a path, an index
        // and a name. Inline it and every `Result` in the workspace that can
        // fail this way pays for the failure.
        source: Box<toml::de::Error>,
    },
    Serialize {
        source: toml::ser::Error,
    },
    // The manifest's wording, carried rather than restated: a path outside the
    // root is refused by `to_manifest_path`, which already says so in the terms
    // it uses everywhere else.
    Path {
        source: Box<manifest::Error>,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound { path } => {
                write!(f, "no filed records at `{}`", path.display())
            }
            Self::Io { path, source } => {
                write!(f, "could not read or write `{}`: {source}", path.display())
            }
            Self::Syntax { path, source } => {
                write!(f, "malformed filed records")?;
                if let Some(path) = path {
                    write!(f, " at `{}`", path.display())?;
                }
                write!(f, ": {source}")
            }
            Self::UnsupportedVersion {
                path,
                found,
                supported,
            } => {
                write!(f, "filed records")?;
                if let Some(path) = path {
                    write!(f, " at `{}`", path.display())?;
                }
                write!(
                    f,
                    " declare schema version {found}, which is not supported; this build reads version {supported}"
                )
            }
            Self::Record {
                file,
                index,
                brief,
                source,
            } => {
                write!(f, "filed record {index}")?;
                if let Some(brief) = brief {
                    write!(f, " (`{brief}`)")?;
                }
                if let Some(file) = file {
                    write!(f, " in `{}`", file.display())?;
                }
                write!(f, " is malformed: {source}")
            }
            Self::Serialize { source } => {
                write!(f, "could not write the filed records as TOML: {source}")
            }
            Self::Path { source } => write!(f, "{source}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::Syntax { source, .. } => Some(source),
            Self::Record { source, .. } => Some(source),
            Self::Serialize { source } => Some(source),
            Self::Path { source } => Some(source),
            Self::NotFound { .. } | Self::UnsupportedVersion { .. } => None,
        }
    }
}

// Reading in two passes — the version as whatever integer the file says, the
// records still raw — is what buys the error variants that name something: the
// version before it is trusted, and each record by its index and by the path it
// did spell. Deserialising straight into `Filed` instead would give one blanket
// "this file is wrong".
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawFiled {
    version: i64,
    #[serde(default)]
    filed: Vec<toml::Value>,
}

// Looks redundant beside the check in `read` and is not: this is the derived
// path, and without it `toml::from_str::<Filed>` would quietly accept a version
// `read` refuses.
fn deserialize_version<'de, D: Deserializer<'de>>(deserializer: D) -> Result<u32, D::Error> {
    let version = u32::deserialize(deserializer)?;
    if version == SCHEMA_VERSION {
        Ok(version)
    } else {
        Err(serde::de::Error::custom(format!(
            "filed records declare schema version {version}, which is not supported; this build reads version {SCHEMA_VERSION}"
        )))
    }
}

#[cfg(test)]
#[path = "tests/filed.rs"]
mod tests;
