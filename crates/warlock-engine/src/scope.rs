use std::fmt;
use std::path::Path;

use crate::manifest::{self, Manifest, PactEntry, ROOT_MODULE, to_manifest_path};

const MAXIMUM_CHARACTERS: usize = 24;

// The one line every caller that asks a person for a scope shows, so a prompt
// cannot spell out a second copy of the rules and drift from the one that
// judges. The number cannot be interpolated into a `const`, so a test below
// holds this string and `MAXIMUM_CHARACTERS` together: change the ceiling and
// that test fails rather than a window quietly promising the old one. It leaves
// out the two rules about the ends on purpose — a hint long enough to need
// reading twice is not read once, and `Rule` has a line ready for the moment
// either is broken.
pub const RULES: &str = "1 to 24 characters: lowercase letters, digits, `-` and `_`";

const WILDCARD: &str = "*";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Rule {
    Empty,
    TooLong { characters: usize, maximum: usize },
    Character { character: char },
    Beginning { character: char },
    Ending { character: char },
}

impl fmt::Display for Rule {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => write!(f, "a scope cannot be empty"),
            Self::TooLong {
                characters,
                maximum,
            } => write!(
                f,
                "a scope is at most {maximum} characters, and this one is {characters}"
            ),
            Self::Character { character } => write!(
                f,
                "a scope holds only lowercase letters, digits, `-` and `_`, and this one holds `{character}`"
            ),
            Self::Beginning { character } => write!(
                f,
                "a scope begins with a lowercase letter, and this one begins with `{character}`"
            ),
            Self::Ending { character } => write!(
                f,
                "a scope does not end with `-` or `_`, and this one ends with `{character}`"
            ),
        }
    }
}

impl std::error::Error for Rule {}

// Judges, and deliberately never normalises: no `String` comes back, so there is
// no repaired scope anywhere in this crate to be confused with the one somebody
// wrote. Folding case here — the obvious kindness — would put the manifest
// reader in the business of correcting committed bytes. It belongs instead at
// the two places a person supplies the string, the scope prompt and
// `warlock config`, which lower-case and *then* ask.
pub fn validate_scope(scope: &str) -> Result<(), Rule> {
    // Taking the two end characters here is also the emptiness check: a string
    // with no first character is the empty one.
    let (Some(first), Some(last)) = (scope.chars().next(), scope.chars().next_back()) else {
        return Err(Rule::Empty);
    };

    // Characters rather than bytes: the two agree for everything this accepts,
    // and counting bytes would call a short non-ASCII name long as well as
    // ill-charactered, which is the less useful of the two things to say.
    let characters = scope.chars().count();
    if characters > MAXIMUM_CHARACTERS {
        return Err(Rule::TooLong {
            characters,
            maximum: MAXIMUM_CHARACTERS,
        });
    }

    if let Some(character) = scope
        .chars()
        .find(|character| !is_scope_character(*character))
    {
        return Err(Rule::Character { character });
    }

    if !first.is_ascii_lowercase() {
        return Err(Rule::Beginning { character: first });
    }

    if is_separator(last) {
        return Err(Rule::Ending { character: last });
    }

    Ok(())
}

// The wildcard is spellable on the sigil side and refused on the scope side, and
// the asymmetry is the design. A person holding nothing and a person holding
// everything are different claims. On a directory `*` would mean "open to
// anyone", which having no scope already means, and two spellings of one meaning
// is how a vocabulary rots. Everything else defers to `validate_scope`, so there
// is still exactly one character class in the workspace.
pub fn validate_sigil(sigil: &str) -> Result<(), Rule> {
    if sigil == WILDCARD {
        return Ok(());
    }

    validate_scope(sigil)
}

/// ```
/// use warlock_engine::{Manifest, PactEntry, scope_covering};
///
/// let manifest = Manifest::with_entries([
///     PactEntry::new(".", "crates", "crates/WARLOCK.md")?.with_scope("platform"),
///     PactEntry::new(".", "crates/engine", "crates/engine/WARLOCK.md")?.with_scope("data-plane"),
///     PactEntry::new(".", "crates/engine-tools", "crates/engine-tools/WARLOCK.md")?,
/// ]);
///
/// // The nearest scoped ancestor wins, and a directory answers with its own.
/// assert_eq!(scope_covering("crates/engine/src/lib.rs", ".", &manifest)?, Some("data-plane"));
/// assert_eq!(scope_covering("crates/engine", ".", &manifest)?, Some("data-plane"));
/// // A sibling that merely shares a prefix is not below it: `crates` covers this.
/// assert_eq!(scope_covering("crates/engine-tools/src", ".", &manifest)?, Some("platform"));
/// // Nothing at or above it carries a scope.
/// assert_eq!(scope_covering("docs/adr", ".", &manifest)?, None);
/// # Ok::<(), warlock_engine::manifest::Error>(())
/// ```
// At most one scope, nearest wins, and that is the design rather than a
// simplification waiting to be relaxed. An inner scope replaces an outer one
// outright; the outer one is a default for everything below that has said
// nothing, never a second gate to also satisfy. Nothing accumulates and there is
// no list.
pub fn scope_covering(
    path: impl AsRef<Path>,
    root: impl AsRef<Path>,
    manifest: &Manifest,
) -> Result<Option<&str>, manifest::Error> {
    let stored = to_manifest_path(root, path)?;
    Ok(at_or_above(&stored).find_map(|module| valid_scope(manifest.entry(module)?)))
}

// A scope the validator refuses reads as no scope, indistinguishable from an
// absent one to every caller: one typo in a hand-edited manifest widens the
// boundary to its parent's rather than inventing a boundary nobody wrote, and
// this never hands back a string that is not a scope. Crate-visible so that a
// caller walking the entries itself calls this rule rather than writing a second
// copy of it.
pub(crate) fn valid_scope(entry: &PactEntry) -> Option<&str> {
    entry.scope().filter(|scope| validate_scope(scope).is_ok())
}

// Segments cut at `/`, which is what keeps the walk segment-wise rather than
// textual: the ancestors of `crates/engine-tools` are `crates` and `.`, and
// `crates/engine` is never among them however much of a prefix it looks like.
fn at_or_above(stored: &str) -> impl Iterator<Item = &str> {
    let mut next = Some(stored);
    std::iter::from_fn(move || {
        let current = next?;
        next = match current.rsplit_once('/') {
            Some((parent, _)) => Some(parent),
            None if current == ROOT_MODULE => None,
            None => Some(ROOT_MODULE),
        };
        Some(current)
    })
}

/// ```
/// use warlock_engine::scope_opens_to;
///
/// let held = ["web".to_owned(), "billing".to_owned()];
///
/// // A directory nobody has scoped is open to anyone.
/// assert!(scope_opens_to(None, &held));
/// // Any one held sigil opens the scope it matches.
/// assert!(scope_opens_to(Some("billing"), &held));
/// // A scope none of them matches is closed.
/// assert!(!scope_opens_to(Some("data-plane"), &held));
/// // Holding nothing opens nothing that is scoped.
/// assert!(!scope_opens_to(Some("data-plane"), &[]));
/// // But an unscoped directory is still open to a machine holding nothing.
/// assert!(scope_opens_to(None, &[]));
/// // The wildcard sigil opens everything.
/// assert!(scope_opens_to(Some("data-plane"), &["*".to_owned()]));
/// ```
#[must_use]
pub fn scope_opens_to(covering: Option<&str>, held: &[String]) -> bool {
    let Some(scope) = covering else {
        // Nobody drew a boundary here, so there is none to be outside of.
        return true;
    };

    // No special case for an empty `held`, deliberately. It falls through to the
    // membership test and matches nothing, which is the answer: a sigil is what
    // opens a scope, so holding none opens none. Reading the empty set as
    // unrestricted — the symmetrical-looking counterpart to the `None` above —
    // would make the default state of every machine a universal bypass, and the
    // boundary would then hold only for people who had already opted into being
    // bound by it. Onboarding is where this is paid for instead.
    held.iter().any(|sigil| sigil == WILDCARD || sigil == scope)
}

fn is_scope_character(character: char) -> bool {
    character.is_ascii_lowercase() || character.is_ascii_digit() || is_separator(character)
}

fn is_separator(character: char) -> bool {
    character == '-' || character == '_'
}

// String work on the manifest's own stored paths, never a question for the
// filesystem, so an entry whose directory is gone still answers.
pub(crate) fn at_or_below(module: &str, selected: &str) -> bool {
    // The repository root is above everything, itself included.
    selected == ROOT_MODULE
        || module == selected
        // The `/` is what makes this segment-wise. A plain `starts_with` would
        // have `crates/engine` swallow `crates/engine-tools`.
        || module
            .strip_prefix(selected)
            .is_some_and(|below| below.starts_with('/'))
}

// The downward question — what does an un-pact reach — and not
// `scope_covering`/`scope_opens_to`, which walk up. Not interchangeable here:
// coverage reads an unscoped `crates` as the absence of a statement, which
// would let somebody standing above a boundary destroy it by aiming at its
// parent.
/// ```
/// use warlock_engine::{Manifest, PactEntry, closed_scopes_at_or_below};
///
/// let entry = |module: &str| PactEntry::new(".", module, format!("{module}/WARLOCK.md"));
/// let manifest = Manifest::with_entries([
///     entry("crates")?,
///     entry("crates/engine")?.with_scope("data-plane"),
///     entry("crates/engine-tools")?.with_scope("tooling"),
/// ]);
/// let held = ["tooling".to_owned()];
///
/// // `crates` is unscoped, so its own boundary opens — but the un-pact reaches
/// // one this machine is outside of.
/// let blocking = closed_scopes_at_or_below("crates", ".", &manifest, &held)?;
/// assert_eq!(blocking, ["data-plane"]);
///
/// // A sibling that merely shares a prefix is not below, and its own scope is
/// // held.
/// let blocking = closed_scopes_at_or_below("crates/engine-tools", ".", &manifest, &held)?;
/// assert!(blocking.is_empty());
/// # Ok::<(), warlock_engine::manifest::Error>(())
/// ```
pub fn closed_scopes_at_or_below<'manifest>(
    directory: impl AsRef<Path>,
    root: impl AsRef<Path>,
    manifest: &'manifest Manifest,
    held: &[String],
) -> Result<Vec<&'manifest str>, manifest::Error> {
    let selected = to_manifest_path(root, directory)?;

    let mut blocking: Vec<&str> = Vec::new();
    let below = manifest
        .entries()
        .iter()
        .filter(|entry| at_or_below(entry.module(), &selected))
        .filter_map(valid_scope);
    for scope in below {
        if !scope_opens_to(Some(scope), held) && !blocking.contains(&scope) {
            blocking.push(scope);
        }
    }
    Ok(blocking)
}

#[cfg(test)]
#[path = "tests/scope.rs"]
mod tests;
