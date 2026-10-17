// The home directory is a parameter here for the reason it is one in
// `config.rs` and `keys.rs`: this function reads the machine's key store, and a
// module that resolved `HOME` itself could let a test in this crate read the
// developer's real credentials.

use std::fmt;
use std::path::{Path, PathBuf};

use crate::config::{self, config_path, load_key_binding, open_scopes};
use crate::keys::{self, keys_path, load_key_names};
use crate::manifest::{self, Manifest, ScopeRecord, manifest_path};
use crate::scope::{scope_covering, scope_opens_to};

/// ```
/// use warlock_engine::{
///     BoardScope, Manifest, ScopedDirectory, ScopeRecord, resolve_route, route, save_key,
///     save_key_binding, save_open_scopes,
/// };
///
/// let (home, root) = (tempfile::tempdir()?, tempfile::tempdir()?);
/// let manifest = Manifest::with_directories([
///     ScopedDirectory::new(root.path(), "crates", "data-plane")?,
/// ])
/// .with_scopes([ScopeRecord::new("data-plane", "WAR", "In Review", "area/data-plane")]);
///
/// save_open_scopes(home.path(), root.path(), &["web".to_owned()])?;
/// save_key_binding(home.path(), root.path(), "data-plane", "work")?;
/// save_key(home.path(), "work", "lin_api_example")?;
///
/// let route = resolve_route("crates/engine/src", root.path(), &manifest, home.path())?;
/// assert_eq!(route.scope(), "data-plane");
/// assert_eq!(BoardScope::of(route.record()).ticket_prefix(), "WAR");
/// assert_eq!(route.record().review_state(), "In Review");
/// assert_eq!(route.key(), "work");
/// // That scope is not open on this machine: a fact beside the route,
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

    let RouteFacts {
        scope,
        record,
        key,
        stored,
    } = route_facts(path, root, manifest, Some(home))?;

    let Some(scope) = scope else {
        return Err(Error::Unscoped {
            path: path.to_path_buf(),
        });
    };

    let Some(record) = record else {
        return Err(Error::Unrecorded {
            scope: scope.to_owned(),
            path: manifest_path(root),
        });
    };

    // Before the two key refusals below, which is what keeps a config that
    // will not parse a `Config` refusal here even though `route_facts` reads
    // the same file forgivingly: the broken file is reported as broken rather
    // than as a checkout bound to nothing.
    let held = open_scopes(home, root).map_err(|source| Error::Config { source })?;

    // Both refusals name the covering scope, because that is the scope the
    // binding is read for and the word a person types back at `warlock key
    // use`. A sentence about "this checkout" would send somebody to a file that
    // can hold a key for every other scope in the repository and still be
    // missing this one.
    let key = match (key, stored) {
        (None, _) => {
            return Err(Error::Unbound {
                scope: scope.to_owned(),
                path: config_path(home, root),
            });
        }
        (Some(key), false) => {
            return Err(Error::Dangling {
                scope: scope.to_owned(),
                key,
                path: keys_path(home),
            });
        }
        (Some(key), true) => key,
    };

    Ok(Route {
        scope,
        record,
        key,
        opens: scope_opens_to(Some(scope), &held),
    })
}

// The key refusals for `filing.rs`, which asks the question `resolve_route`
// does of a repository with no path to walk up. The scope is a parameter rather
// than something read here, because that caller has already picked which board
// it is filing to and the binding is per scope: deciding it twice is two
// answers that can disagree.
//
// A key store that is unreadable or will not parse is an error, while a binding
// that cannot be read is nothing bound — the same asymmetry `route_facts`
// documents, and it has to match, or a push and a route would answer one broken
// file two ways.
pub(crate) fn bound_key(home: &Path, root: &Path, scope: &str) -> Result<String, Error> {
    let Some(key) = load_key_binding(home, root, scope).unwrap_or_default() else {
        return Err(Error::Unbound {
            scope: scope.to_owned(),
            path: config_path(home, root),
        });
    };

    if !is_stored(home, &key)? {
        return Err(Error::Dangling {
            scope: scope.to_owned(),
            key,
            path: keys_path(home),
        });
    }

    Ok(key)
}

// `load_key_names` rather than `load_key`, so no key value is ever held by this
// module — nothing here can then leak one into an error, a `Debug` or a panic,
// and the never-print-a-secret rule costs no care at the other end. The name is
// all a caller gets back, and it fetches the value itself from `keys.rs`. An
// empty store and no store at all are one answer: both mean the bound name
// resolves to nothing, and `warlock key add` is the fix for either.
fn is_stored(home: &Path, name: &str) -> Result<bool, Error> {
    match load_key_names(home) {
        Ok(names) => Ok(names.iter().any(|stored| stored == name)),
        Err(keys::Error::NotFound { .. }) => Ok(false),
        Err(source) => Err(Error::Keys { source }),
    }
}

/// ```
/// use warlock_engine::{Manifest, ScopedDirectory, route_facts, save_key_binding};
///
/// let (home, root) = (tempfile::tempdir()?, tempfile::tempdir()?);
/// let manifest = Manifest::with_directories([
///     ScopedDirectory::new(root.path(), "crates", "data-plane")?,
/// ]);
/// save_key_binding(home.path(), root.path(), "data-plane", "work")?;
///
/// // A scope with no `[[scope]]` record, bound to a name this machine has not
/// // stored: two of the things `resolve_route` refuses on, both answers here.
/// let facts = route_facts("crates/engine/src", root.path(), &manifest, Some(home.path()))?;
/// assert_eq!(facts.scope(), Some("data-plane"));
/// assert!(facts.record().is_none());
/// assert_eq!(facts.key(), Some("work"));
/// assert!(!facts.stored());
///
/// // Nothing covers this one, so there is no scope to read a binding for: a
/// // key is bound to a scope, and a path outside every scope has none.
/// let facts = route_facts("docs/adr", root.path(), &manifest, Some(home.path()))?;
/// assert_eq!(facts.scope(), None);
/// assert_eq!(facts.key(), None);
/// assert!(!facts.stored());
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
// The reporting half of `resolve_route`, and the reason there are two: each of
// the four absences that function refuses on arrives here as a value beside the
// facts that *are* known. `warlock check` answers with exit 0 whatever it
// finds, and the refusing form cannot serve it — it returns `Unrecorded` before
// it has looked at the binding at all, so a checkout with an unrecorded scope
// and a perfectly good key could report neither half.
//
// Relaxing the four refusals in place was the alternative, and it would have
// been paid for by every other caller: a `Route` in hand is a route that can
// actually be filed, which is the whole guarantee `resolve_route` sells.
//
// No open scope is read here, deliberately. `open_scopes`, `scope_opens_to` and
// `opens` stay in the wrapper because a config that will not parse is
// `OpenScopes::Unknown` to `check` — an answer, printed, with exit 0 — and
// reading the open scopes here would turn it into a failure.
//
// `home` is `None` for a caller that could not resolve one, and that reads as
// no key bound: a machine with no home has no key store to bind to either, and
// `warlock key use` is the fix for both. A path no scope covers reads the same
// way, for a different reason: the binding is per scope, so with no covering
// scope there is no question to ask the config, and reporting whatever some
// other scope is bound to would name a key that has nothing to do with this
// path.
pub fn route_facts<'m>(
    path: impl AsRef<Path>,
    root: impl AsRef<Path>,
    manifest: &'m Manifest,
    home: Option<&Path>,
) -> Result<RouteFacts<'m>, Error> {
    let (path, root) = (path.as_ref(), root.as_ref());

    let scope = scope_covering(path, root, manifest).map_err(|source| Error::Path { source })?;
    let record = scope.and_then(|scope| manifest.scope_record(scope));

    let (Some(home), Some(covering)) = (home, scope) else {
        return Ok(RouteFacts {
            scope,
            record,
            key: None,
            stored: false,
        });
    };

    // A config that is unreadable or will not parse reads as nothing bound,
    // unlike the key store below, where the same breakage is an error. The
    // asymmetry is the one thing here that is not obvious: the caller wanting
    // these facts reads that same config for the open scopes and already
    // reports it as unknown, so a mangled config is visible either way, while a
    // mangled key store swallowed as "that name is not stored" would leave no
    // trace anywhere. Propagating it instead would make `warlock check` exit
    // non-zero on a machine whose config needs a hand fix, which is the one
    // thing that command never does.
    let key = load_key_binding(home, root, covering).unwrap_or_default();

    // A scope bound to nothing does not read the store at all, which is what
    // keeps a broken store an `Unbound` rather than a `Keys` refusal in the
    // wrapper.
    let stored = match &key {
        None => false,
        Some(name) => is_stored(home, name)?,
    };

    Ok(RouteFacts {
        scope,
        record,
        key,
        stored,
    })
}

// `opens` is carried rather than checked: a scope that is not open on this
// machine still resolves to its full route, because the decision this implements is
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

// Every field is an absence a person fixes somewhere different — a scope on a
// directory, a `[[scope]]` record, `warlock key use`, `warlock key add` — which is
// why they are four values and not one "routable" boolean: a caller that only
// learns the answer is no cannot say which file to go to.
//
// `stored` is `false` for a checkout bound to nothing, so it is never read on
// its own: "no name" and "a name nothing answers to" are the same false and
// different sentences, and `key` is what tells them apart.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RouteFacts<'m> {
    scope: Option<&'m str>,
    record: Option<&'m ScopeRecord>,
    key: Option<String>,
    stored: bool,
}

impl<'m> RouteFacts<'m> {
    #[must_use]
    pub const fn scope(&self) -> Option<&'m str> {
        self.scope
    }

    #[must_use]
    pub const fn record(&self) -> Option<&'m ScopeRecord> {
        self.record
    }

    #[must_use]
    pub fn key(&self) -> Option<&str> {
        self.key.as_deref()
    }

    #[must_use]
    pub const fn stored(&self) -> bool {
        self.stored
    }
}

// Four refusals rather than one "cannot route": each of them is fixed in a
// different file by a different command, and collapsing them would send
// somebody editing the wrong one.
#[derive(Debug)]
#[non_exhaustive]
pub enum Error {
    Unscoped {
        path: PathBuf,
    },
    Unrecorded {
        scope: String,
        path: PathBuf,
    },
    Unbound {
        scope: String,
        path: PathBuf,
    },
    Dangling {
        scope: String,
        key: String,
        path: PathBuf,
    },
    Path {
        source: manifest::Error,
    },
    Config {
        source: config::Error,
    },
    Keys {
        source: keys::Error,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unscoped { path } => write!(
                f,
                "no scope covers `{}`: scope a directory at or above it",
                path.display()
            ),
            Self::Unrecorded { scope, path } => write!(
                f,
                "the scope `{scope}` has no `[[scope]]` record: add one to `{}`",
                path.display()
            ),
            // Both name the scope, because a checkout binds a key per scope:
            // the fix is one line for one scope, and the sentence has to say
            // which one or a person rebinds the scope that already worked.
            Self::Unbound { scope, path } => write!(
                f,
                "no key is bound to the scope `{scope}` in `{}`: bind one with `warlock key use {scope} <name>`",
                path.display()
            ),
            Self::Dangling { scope, key, path } => write!(
                f,
                "the scope `{scope}` is bound to `{key}`, which is not in `{}`: store it with `warlock key add {key}`",
                path.display()
            ),
            Self::Path { source } => {
                write!(f, "could not place the path in the repository: {source}")
            }
            Self::Config { source } => {
                write!(
                    f,
                    "could not read which scopes are open on this machine: {source}"
                )
            }
            Self::Keys { source } => write!(f, "could not read the key store: {source}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Path { source } => Some(source),
            Self::Config { source } => Some(source),
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
