// The home directory is a parameter here and is never read from the
// environment, for the reason it is one in `config.rs`, `keys.rs` and
// `route.rs`, and for one more that holds here and nowhere else in this crate:
// the target below carries the key *value*, so a module that resolved `HOME`
// itself could let a test in this crate read the developer's real credentials
// into a value that is then printed by whatever fails next.

use std::fmt;
use std::path::{Path, PathBuf};

use crate::config::{self, open_scopes};
use crate::keys::{keys_path, load_key};
use crate::manifest::{BoardScope, Manifest, ScopeRecord, manifest_path};
use crate::route::{self, bound_key};
use crate::scope::{scope_opens_to, valid_scope};

/// ```
/// use warlock_engine::{
///     LinearScope, Manifest, ScopedDirectory, ScopeRecord, filing, resolve_filing, save_key,
///     save_key_binding, save_open_scopes,
/// };
///
/// let (home, root) = (tempfile::tempdir()?, tempfile::tempdir()?);
/// let manifest = Manifest::with_directories([
///     ScopedDirectory::new(root.path(), "crates", "data-plane")?,
/// ])
/// .with_scopes([
///     ScopeRecord::new("data-plane", "WAR", "In Review", "warlock"),
///     ScopeRecord::new("web", "WEB", "In Review", "warlock"),
/// ]);
///
/// save_open_scopes(home.path(), root.path(), &["data-plane".to_owned()])?;
/// save_key_binding(home.path(), root.path(), "data-plane", "work")?;
/// save_key(home.path(), "work", "lin_api_example")?;
/// // The other scope files to another workspace, under a key of its own.
/// save_key_binding(home.path(), root.path(), "web", "personal")?;
/// save_key(home.path(), "personal", "lin_api_other")?;
///
/// // One open scope, one record of that name: one board, and no question to ask.
/// let target = resolve_filing(&manifest, root.path(), home.path(), None)?;
/// assert_eq!(target.scope(), "data-plane");
/// assert_eq!(LinearScope::of(target.record())?.team_key(), "WAR");
/// assert_eq!(target.key(), "work");
/// assert_eq!(target.value(), "lin_api_example");
///
/// // The wildcard opens every recorded scope, so this repository now has two
/// // candidates and nothing to break the tie with: a brief is not about a
/// // directory, so there is no path to walk up and no nearest scope to win.
/// save_open_scopes(home.path(), root.path(), &["*".to_owned()])?;
/// assert!(matches!(
///     resolve_filing(&manifest, root.path(), home.path(), None),
///     Err(filing::Error::Several { .. })
/// ));
///
/// // A name picks one of them, and a name that is not a candidate is refused.
/// let target = resolve_filing(&manifest, root.path(), home.path(), Some("web"))?;
/// assert_eq!(LinearScope::of(target.record())?.team_key(), "WEB");
/// assert_eq!(target.key(), "personal");
/// assert!(matches!(
///     resolve_filing(&manifest, root.path(), home.path(), Some("billing")),
///     Err(filing::Error::Unknown { .. })
/// ));
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
// The repository-level counterpart to `resolve_route`, and the reason there are
// two: that one answers for a path, by walking up to the nearest scope, and a
// brief is not about a directory. There is nothing to walk here, so the open
// scope is the whole statement of which board this is, and anything other than
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

    // Before the key half below, so a broken config is reported as broken
    // rather than as a checkout that opens nothing.
    let held = open_scopes(home, root).map_err(|source| Error::Config { source })?;

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

    // The record's own name, so the key resolved is the key of the board this
    // is about to file to: a repository whose scopes sit in two Linear
    // workspaces has two bindings, and picking the board and then resolving
    // some other scope's key is the push that lands in the wrong workspace.
    let key = bound_key(home, root, record.name()).map_err(|source| Error::Key { source })?;

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
                scope: record.name().to_owned(),
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

// Four sentences rather than one "nothing to file to", because a repository
// that records no scope at all, a machine that opens nothing, a machine that
// opens only scopes this repository has never heard of and a scope some
// directory carries with no `[[scope]]` record are fixed in four different files
// by four different people.
//
// The repository is asked about first, before anything about this machine,
// because the candidates above are built from `scopes()` alone: with no record
// there, no scope this machine could open would make a board, so the sentence
// about opening one is advice that cannot work — the person opens a scope,
// pushes again, and fails again on a different sentence. Each absence gets one
// sentence naming one fix, so the order is which fix comes first and not which
// absence is worse.
fn no_candidate(manifest: &Manifest, root: &Path, held: &[String]) -> Error {
    if manifest.scopes().is_empty() {
        return Error::Boardless {
            path: manifest_path(root),
        };
    }

    if held.is_empty() {
        return Error::NothingOpen;
    }

    // An open scope that some directory carries, with no `[[scope]]` record to
    // say where that scope files: the manifest is one line short rather than
    // the machine being wrong, which is the opposite fix to the one below.
    if let Some(scope) = manifest
        .directories()
        .iter()
        .filter_map(valid_scope)
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

    #[must_use]
    pub fn destination(&self) -> Destination {
        let scope = BoardScope::of(self.record);
        Destination {
            scope: self.scope.to_owned(),
            ticket_prefix: scope.ticket_prefix().to_owned(),
            label: scope.label().to_owned(),
            key: self.key.clone(),
        }
    }
}

// What a `Target` says about where work goes, owned and without the key value,
// so it can sit in a window or cross onto a worker thread where a `Target`,
// which borrows the manifest and carries the key, cannot. The key is here by
// name only: nothing that holds a `Destination` can print a key value.
//
// The prefix is the only field of a board's own that is here, and it is
// `BoardScope::ticket_prefix`: what a reader of this type does with it is name
// the board a ticket key belongs to, so a record that files to Jira carries its
// project key here exactly as a Linear one carries its team key, and nothing
// holding a `Destination` has to ask which board it came from. A base URL, or
// anything else a request is addressed with, belongs to whatever opens the
// client and not here.
//
// The project a brief becomes is not here either. A draft is handed one on its
// command line rather than reading it from the `[[scope]]` record, and the push
// that is about to make one has none to carry.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Destination {
    scope: String,
    ticket_prefix: String,
    label: String,
    key: String,
}

impl Destination {
    #[must_use]
    pub fn new(
        scope: impl Into<String>,
        ticket_prefix: impl Into<String>,
        label: impl Into<String>,
        key: impl Into<String>,
    ) -> Self {
        Self {
            scope: scope.into(),
            ticket_prefix: ticket_prefix.into(),
            label: label.into(),
            key: key.into(),
        }
    }

    #[must_use]
    pub fn scope(&self) -> &str {
        &self.scope
    }

    #[must_use]
    pub fn ticket_prefix(&self) -> &str {
        &self.ticket_prefix
    }

    #[must_use]
    pub fn label(&self) -> &str {
        &self.label
    }

    #[must_use]
    pub fn key(&self) -> &str {
        &self.key
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
    Boardless {
        path: PathBuf,
    },
    NothingOpen,
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
    Config {
        source: config::Error,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            // No scope named and no `warlock open`: the fix is a line in the
            // file this names, and a sentence about what the machine opens would
            // send the person to the other one of the two.
            Self::Boardless { path } => write!(
                f,
                "`{}` records no `[[scope]]` record, so there is nowhere to file: add one",
                path.display()
            ),
            Self::NothingOpen => write!(
                f,
                "AI can work in no scope on this machine, so nothing says which board to file to: open one with `warlock open <scope>`"
            ),
            Self::Unmatched { held, path } => write!(
                f,
                "AI can work in {} on this machine, and `{}` records no scope of any of those names",
                listed(held),
                path.display()
            ),
            Self::Unrecorded { scope, path } => write!(
                f,
                "`{scope}` is open on this machine, and that scope has no `[[scope]]` record: add one to `{}`",
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
            Self::Config { source } => {
                write!(
                    f,
                    "could not read which scopes are open on this machine: {source}"
                )
            }
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Key { source } => Some(source),
            Self::Config { source } => Some(source),
            Self::Boardless { .. }
            | Self::NothingOpen
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
