// The home directory is a parameter here and is never read from the
// environment, for the reason it is one in `sigils.rs`, `keys.rs` and
// `route.rs`, and for one more that holds here and nowhere else in this crate:
// the target below carries the key *value*, so a module that resolved `HOME`
// itself could let a test in this crate read the developer's real credentials
// into a value that is then printed by whatever fails next.

use std::fmt;
use std::path::{Path, PathBuf};

use crate::keys::{keys_path, load_key};
use crate::manifest::{Manifest, ScopeRecord, manifest_path};
use crate::route::{self, bound_key};
use crate::scope::{scope_opens_to, valid_scope};
use crate::sigils::{self, load_sigils};

/// ```
/// use warlock_engine::{
///     Manifest, PactEntry, ScopeRecord, filing, resolve_filing, save_key, save_key_binding,
///     save_sigils,
/// };
///
/// let (home, root) = (tempfile::tempdir()?, tempfile::tempdir()?);
/// let manifest = Manifest::with_entries([
///     PactEntry::new(root.path(), "crates", "crates/WARLOCK.md")?.with_scope("data-plane"),
/// ])
/// .with_scopes([
///     ScopeRecord::new("data-plane", "Data Plane", "In Review", "warlock"),
///     ScopeRecord::new("web", "Web", "In Review", "warlock"),
/// ]);
///
/// save_sigils(home.path(), root.path(), &["data-plane".to_owned()])?;
/// save_key_binding(home.path(), root.path(), "work")?;
/// save_key(home.path(), "work", "lin_api_example")?;
///
/// // One sigil, one record of that name: one board, and no question to ask.
/// let target = resolve_filing(&manifest, root.path(), home.path(), None)?;
/// assert_eq!(target.scope(), "data-plane");
/// assert_eq!(target.record().team(), "Data Plane");
/// assert_eq!(target.key(), "work");
/// assert_eq!(target.value(), "lin_api_example");
///
/// // The wildcard holds every recorded scope, so this repository now has two
/// // candidates and nothing to break the tie with: a brief is not about a
/// // directory, so there is no path to walk up and no nearest scope to win.
/// save_sigils(home.path(), root.path(), &["*".to_owned()])?;
/// assert!(matches!(
///     resolve_filing(&manifest, root.path(), home.path(), None),
///     Err(filing::Error::Several { .. })
/// ));
///
/// // A name picks one of them, and a name that is not a candidate is refused.
/// let target = resolve_filing(&manifest, root.path(), home.path(), Some("web"))?;
/// assert_eq!(target.record().team(), "Web");
/// assert!(matches!(
///     resolve_filing(&manifest, root.path(), home.path(), Some("billing")),
///     Err(filing::Error::Unknown { .. })
/// ));
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
// The repository-level counterpart to `resolve_route`, and the reason there are
// two: that one answers for a path, by walking up to the nearest scope, and a
// brief is not about a directory. There is nothing to walk here, so the sigil
// is the whole statement of which board this is, and anything other than
// exactly one candidate refuses rather than picks — ambiguity resolved by
// guessing is somebody's project on the wrong board, found out later by
// somebody else.
//
// Membership is `scope_opens_to`'s, called once here and never restated: a
// second comparison written in this file is a second copy of the boundary rule,
// and it is the copy that would forget the wildcard.
pub fn resolve_filing<'m>(
    manifest: &'m Manifest,
    root: impl AsRef<Path>,
    home: impl AsRef<Path>,
    scope: Option<&str>,
) -> Result<Target<'m>, Error> {
    let (root, home) = (root.as_ref(), home.as_ref());

    // A missing config reads as an empty held set *here only*, as it does in
    // `resolve_route` and against `load_sigils`'s own insistence that absent and
    // holds-nothing stay different answers: a machine that has never recorded a
    // sigil files to no board either way. Unreadable and unparseable stay
    // errors, because those are not an answer — and reading them before the key
    // half below is what keeps a broken config reported as broken rather than as
    // a checkout that holds nothing.
    let held = match load_sigils(home, root) {
        Err(sigils::Error::NotFound { .. }) => Vec::new(),
        other => other.map_err(|source| Error::Sigils { source })?,
    };

    let candidates: Vec<&ScopeRecord> = manifest
        .scopes()
        .iter()
        .filter(|record| scope_opens_to(Some(record.name()), &held))
        .collect();

    if candidates.is_empty() {
        return Err(no_candidate(manifest, root, &held));
    }

    // Both board refusals below are decided before the key store is touched, so
    // a checkout that is both ambiguous and unbound is told about the board: the
    // person has two things to fix and `--scope` is the one that is about this
    // push. The reverse order was rejected for that reason, not for cost.
    //
    // An explicit name skips the several-candidates refusal rather than being
    // checked after it, because that refusal's whole instruction is to supply
    // this name; a name that is not a candidate names them all anyway, so no
    // answer is lost by taking this branch first.
    let record = match scope {
        Some(name) => candidates
            .iter()
            .copied()
            .find(|record| record.name() == name)
            .ok_or_else(|| Error::Unknown {
                name: name.to_owned(),
                candidates: named(&candidates),
            })?,
        None if candidates.len() > 1 => {
            return Err(Error::Several {
                candidates: named(&candidates),
            });
        }
        None => candidates[0],
    };

    let key = bound_key(home, root).map_err(|source| Error::Key { source })?;

    // `load_key` rather than `load_key_names`, unlike everything else that asks
    // this question: a caller filing to a board needs the value, and fetching it
    // at the other end would mean a second read that can disagree with this one.
    // It is why `Target` has a redacting `Debug` and why no variant of `Error`
    // below holds a value.
    let value = load_key(home, &key)
        .map_err(|source| Error::Key {
            source: route::Error::Keys { source },
        })?
        // `bound_key` has just found this name in the store, so this is the
        // store losing it between two reads. That is what `Dangling` says, and
        // saying it in a sentence of this module's own would be a second wording
        // of one thing for a case nobody will reproduce.
        .ok_or_else(|| Error::Key {
            source: route::Error::Dangling {
                key: key.clone(),
                path: keys_path(home),
            },
        })?;

    Ok(Target {
        scope: record.name(),
        record,
        key,
        value,
    })
}

// Three sentences rather than one "nothing to file to", because the machine
// holding nothing, the machine holding something this repository has never
// heard of and the repository owing a `[[scope]]` record are fixed in three
// different files by three different people.
fn no_candidate(manifest: &Manifest, root: &Path, held: &[String]) -> Error {
    if held.is_empty() {
        return Error::Unsigiled;
    }

    // A held sigil whose scope some pact carries, with no `[[scope]]` record to
    // say where that scope files: the manifest is one line short rather than the
    // machine being wrong, which is the opposite fix to the one below.
    if let Some(scope) = manifest
        .entries()
        .iter()
        .filter_map(|entry| valid_scope(entry))
        .find(|scope| scope_opens_to(Some(scope), held))
    {
        return Error::Unrecorded {
            scope: scope.to_owned(),
            path: manifest_path(root),
        };
    }

    Error::Unmatched {
        held: held.to_vec(),
        path: manifest_path(root),
    }
}

fn named(candidates: &[&ScopeRecord]) -> Vec<String> {
    candidates
        .iter()
        .map(|record| record.name().to_owned())
        .collect()
}

fn listed(names: &[String]) -> String {
    names
        .iter()
        .map(|name| format!("`{name}`"))
        .collect::<Vec<_>>()
        .join(", ")
}

// The key value is carried, unlike `Route`'s name-only answer, because the
// caller that has a target is the caller about to make the request with it.
#[derive(Clone)]
pub struct Target<'m> {
    scope: &'m str,
    record: &'m ScopeRecord,
    key: String,
    value: String,
}

impl<'m> Target<'m> {
    #[must_use]
    pub const fn scope(&self) -> &'m str {
        self.scope
    }

    #[must_use]
    pub const fn record(&self) -> &'m ScopeRecord {
        self.record
    }

    #[must_use]
    pub fn key(&self) -> &str {
        &self.key
    }

    #[must_use]
    pub fn value(&self) -> &str {
        &self.value
    }
}

// Hand-written and redacting rather than derived, for the reason `keys::Config`
// has no `Debug` at all: this is the one type in the crate that holds a Linear
// API key, and a derived one would put it into every `{:?}`, every `unwrap`
// panic and every failing assertion that happens to have a target in scope.
impl fmt::Debug for Target<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Target")
            .field("scope", &self.scope)
            .field("record", self.record)
            .field("key", &self.key)
            .field("value", &"<redacted>")
            .finish()
    }
}

// Names only, never a key value, so nothing a caller prints on a refusal can
// leak one — the same rule `route::Error` keeps, and the reason `Key` wraps a
// `route::Error` rather than restating it.
#[derive(Debug)]
#[non_exhaustive]
pub enum Error {
    Unsigiled,
    Unmatched {
        held: Vec<String>,
        path: PathBuf,
    },
    Unrecorded {
        scope: String,
        path: PathBuf,
    },
    Several {
        candidates: Vec<String>,
    },
    Unknown {
        name: String,
        candidates: Vec<String>,
    },
    Key {
        source: route::Error,
    },
    Sigils {
        source: sigils::Error,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unsigiled => write!(
                f,
                "this machine holds no sigil, so nothing says which board to file to: hold one with `warlock config`"
            ),
            Self::Unmatched { held, path } => write!(
                f,
                "this machine holds {}, and `{}` records no scope of any of those names",
                listed(held),
                path.display()
            ),
            Self::Unrecorded { scope, path } => write!(
                f,
                "this machine holds `{scope}`, and that scope has no `[[scope]]` record: add one to `{}`",
                path.display()
            ),
            Self::Several { candidates } => write!(
                f,
                "this machine can file to {}: pick one with `--scope <name>`",
                listed(candidates)
            ),
            Self::Unknown { name, candidates } => write!(
                f,
                "`{name}` is not a scope this machine can file to here: {}",
                listed(candidates)
            ),
            // Verbatim, because these are the two sentences `route.rs` already
            // words and a person meets them from `warlock check` first.
            Self::Key { source } => write!(f, "{source}"),
            Self::Sigils { source } => {
                write!(f, "could not read what this machine holds: {source}")
            }
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Key { source } => Some(source),
            Self::Sigils { source } => Some(source),
            Self::Unsigiled
            | Self::Unmatched { .. }
            | Self::Unrecorded { .. }
            | Self::Several { .. }
            | Self::Unknown { .. } => None,
        }
    }
}

#[cfg(test)]
#[path = "tests/filing.rs"]
mod tests;
