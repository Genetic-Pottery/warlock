// The home directory is a parameter here for the reason it is one in
// `sigils.rs` and `keys.rs`: this function reads the machine's key store, and a
// module that resolved `HOME` itself could let a test in this crate read the
// developer's real credentials.

use std::fmt;
use std::path::{Path, PathBuf};

use crate::keys::{self, keys_path, load_key_names};
use crate::manifest::{self, Manifest, ScopeRecord, manifest_path};
use crate::scope::{scope_covering, scope_opens_to};
use crate::sigils::{self, load_key_binding, load_sigils, sigils_path};

/// ```
/// use warlock_engine::{
///     Manifest, PactEntry, ScopeRecord, resolve_route, route, save_key, save_key_binding,
///     save_sigils,
/// };
///
/// let (home, root) = (tempfile::tempdir()?, tempfile::tempdir()?);
/// let manifest = Manifest::with_entries([
///     PactEntry::new(root.path(), "crates", "crates/WARLOCK.md")?.with_scope("data-plane"),
/// ])
/// .with_scopes([ScopeRecord::new("data-plane", "Data Plane", "In Review", "area/data-plane")]);
///
/// save_sigils(home.path(), root.path(), &["web".to_owned()])?;
/// save_key_binding(home.path(), root.path(), "work")?;
/// save_key(home.path(), "work", "lin_api_example")?;
///
/// let route = resolve_route("crates/engine/src", root.path(), &manifest, home.path())?;
/// assert_eq!(route.scope(), "data-plane");
/// assert_eq!(route.record().team(), "Data Plane");
/// assert_eq!(route.record().review_state(), "In Review");
/// assert_eq!(route.key(), "work");
/// // This machine holds no sigil for that scope: a fact beside the route,
/// // reported and never a refusal.
/// assert!(!route.opens());
///
/// // A path nothing covers is a refusal of its own, naming the path.
/// assert!(matches!(
///     resolve_route("docs/adr", root.path(), &manifest, home.path()),
///     Err(route::Error::Unscoped { .. })
/// ));
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
// The boundary rule is `scope_covering`'s and the membership test is
// `scope_opens_to`'s, both called once here and neither restated: a caller
// answering this by walking the manifest itself is a second copy of the rule
// that drifts from the first, which is the argument `check.rs` already makes.
pub fn resolve_route(
    path: impl AsRef<Path>,
    root: impl AsRef<Path>,
    manifest: &Manifest,
    home: impl AsRef<Path>,
) -> Result<Route<'_>, Error> {
    let (path, root, home) = (path.as_ref(), root.as_ref(), home.as_ref());

    let Some(scope) =
        scope_covering(path, root, manifest).map_err(|source| Error::Path { source })?
    else {
        return Err(Error::Unscoped {
            path: path.to_path_buf(),
        });
    };

    let Some(record) = manifest
        .scopes()
        .iter()
        .find(|record| record.name() == scope)
    else {
        return Err(Error::Unrecorded {
            scope: scope.to_owned(),
            path: manifest_path(root),
        });
    };

    // A missing config reads as an empty held set *here only*, against
    // `load_sigils`'s own insistence that absent and holds-nothing stay
    // different answers: this is the caller that knows which it is acting on,
    // and a machine that has never recorded a sigil opens no scope either way.
    // Unreadable and unparseable stay errors, because those are not an answer.
    let held = match load_sigils(home, root) {
        Err(sigils::Error::NotFound { .. }) => Vec::new(),
        other => other.map_err(|source| Error::Sigils { source })?,
    };

    let key = match load_key_binding(home, root) {
        Ok(Some(key)) => key,
        Ok(None) => {
            return Err(Error::Unbound {
                path: sigils_path(home, root),
            });
        }
        Err(sigils::Error::NotFound { path }) => return Err(Error::Unbound { path }),
        Err(source) => return Err(Error::Sigils { source }),
    };

    // `load_key_names` rather than `load_key`, so no key value is ever held by
    // this module — nothing here can then leak one into an error, a `Debug` or
    // a panic, and the never-print-a-secret rule costs no care at the other
    // end. An empty store and no store at all are one answer at this point:
    // both mean the bound name resolves to nothing, and `warlock key add` is
    // the fix for either.
    let stored = match load_key_names(home) {
        Ok(names) => names.contains(&key),
        Err(keys::Error::NotFound { .. }) => false,
        Err(source) => return Err(Error::Keys { source }),
    };
    if !stored {
        return Err(Error::Dangling {
            key,
            path: keys_path(home),
        });
    }

    Ok(Route {
        scope,
        record,
        key,
        opens: scope_opens_to(Some(scope), &held),
    })
}

// `opens` is carried rather than checked: a scope this machine holds no sigil
// for still resolves to its full route, because the decision this implements is
// that warlock says what is being crossed and continues. Refusing here would
// brick the ordinary case of one person covering for another.
//
// There is no counterpart field for "the bound name resolves to a stored key".
// That one is `Error::Dangling` instead, so a `Route` in hand is a route that
// can actually be filed; a boolean that was `true` on every value ever built
// would be a field nobody could branch on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Route<'m> {
    scope: &'m str,
    record: &'m ScopeRecord,
    key: String,
    opens: bool,
}

impl<'m> Route<'m> {
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
    pub const fn opens(&self) -> bool {
        self.opens
    }
}

// Four refusals rather than one "cannot route": each of them is fixed in a
// different file by a different command, and collapsing them would send
// somebody editing the wrong one.
#[derive(Debug)]
#[non_exhaustive]
pub enum Error {
    Unscoped { path: PathBuf },
    Unrecorded { scope: String, path: PathBuf },
    Unbound { path: PathBuf },
    Dangling { key: String, path: PathBuf },
    Path { source: manifest::Error },
    Sigils { source: sigils::Error },
    Keys { source: keys::Error },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unscoped { path } => write!(
                f,
                "no scope covers `{}`: pact a directory at or above it and give it a scope",
                path.display()
            ),
            Self::Unrecorded { scope, path } => write!(
                f,
                "the scope `{scope}` has no `[[scope]]` record: add one to `{}`",
                path.display()
            ),
            Self::Unbound { path } => write!(
                f,
                "no key is bound to this checkout in `{}`: bind one with `warlock key use <name>`",
                path.display()
            ),
            Self::Dangling { key, path } => write!(
                f,
                "this checkout is bound to `{key}`, which is not in `{}`: store it with `warlock key add {key}`",
                path.display()
            ),
            Self::Path { source } => {
                write!(f, "could not place the path in the repository: {source}")
            }
            Self::Sigils { source } => {
                write!(f, "could not read what this machine holds: {source}")
            }
            Self::Keys { source } => write!(f, "could not read the key store: {source}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Path { source } => Some(source),
            Self::Sigils { source } => Some(source),
            Self::Keys { source } => Some(source),
            Self::Unscoped { .. }
            | Self::Unrecorded { .. }
            | Self::Unbound { .. }
            | Self::Dangling { .. } => None,
        }
    }
}

#[cfg(test)]
#[path = "tests/route.rs"]
mod tests;
