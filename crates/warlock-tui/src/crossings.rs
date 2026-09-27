//! What a session actually wrote, judged against what this machine holds.
//!
//! The diff of the working tree is the authority on that, and nothing a session
//! says about itself is. The `PreToolUse` gate hook sees `Edit` and `Write` and
//! never sees a `sed`, a `python -c` or a heredoc inside `Bash`, so a pass can
//! leave changes under a scope this machine does not hold without any refusal
//! having been available to give. Reading `git status` after the session is the
//! only check that covers every way a byte can reach the tree.
//!
//! [`crossings_in`] is a function over values: the parsed status entries, the
//! manifest and the flattened held sigils go in, and the two lists come out with
//! no repository, no filesystem and no `git` anywhere on the path. The
//! classification itself is [`scope_covering`] and [`scope_opens_to`] — the same
//! pair `r` and the gate ask — so there is one nearest-scope walk and one
//! membership test in the workspace.
//!
//! This is not the binary's `boundary::permits`: that answers whether an
//! operator may perform an operation *at* a directory, before it happens. This
//! reads what already happened, over paths rather than directories, and refuses
//! nothing.

use std::path::Path;

use warlock_engine::{Manifest, scope_covering, scope_opens_to};

use crate::git::{Dirty, Touched};

/// One changed path together with the scope covering it that this machine does
/// not hold.
///
/// The scope is named on every path rather than the paths being grouped under
/// it, because this is what a halt reason reads out and a reason that grouped
/// would have to be ungrouped again to say which file is where.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Crossing<'a> {
    pub path: &'a str,
    pub scope: &'a str,
}

/// What the tree's changes came to, once the boundary is applied to them.
///
/// Two lists and no third: a path under no scope, or under a directory nobody
/// has pacted, is open and appears in neither. Empty on both counts is the
/// ordinary session.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Crossings<'a> {
    /// Every path under a scope no held sigil opens. What a `crossed` halt names.
    pub crossed: Vec<Crossing<'a>>,
    /// Scopes this machine does hold, other than the one the ticket was pulled
    /// under, with the paths written under them — already the shape
    /// [`pull_request_body`](crate::pull_request_body) takes.
    pub touched: Vec<Touched<'a>>,
}

impl Crossings<'_> {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.crossed.is_empty() && self.touched.is_empty()
    }
}

/// ```
/// use warlock_tui::{Crossing, Dirty, crossings_in};
/// use warlock_engine::{Manifest, PactEntry};
///
/// let manifest = Manifest::with_entries([
///     PactEntry::new(".", "crates/engine", "crates/engine/WARLOCK.md")?.with_scope("data-plane"),
///     PactEntry::new(".", "crates/web", "crates/web/WARLOCK.md")?.with_scope("web"),
/// ]);
/// let held = ["web".to_owned()];
/// let dirty = [Dirty {
///     code: "R ".to_owned(),
///     path: "crates/web/src/route.rs".to_owned(),
///     from: Some("crates/engine/src/route.rs".to_owned()),
/// }];
///
/// let crossings = crossings_in(&dirty, ".".as_ref(), &manifest, &held, Some("web"));
///
/// // The file moved out of a scope this machine does not hold: the old side is
/// // a crossing, and the new side is the scope the ticket was pulled under.
/// assert_eq!(
///     crossings.crossed,
///     [Crossing { path: "crates/engine/src/route.rs", scope: "data-plane" }]
/// );
/// assert!(crossings.touched.is_empty());
/// # Ok::<(), warlock_engine::manifest::Error>(())
/// ```
///
/// `held` is the flattened list of sigils rather than the header's
/// [`Sigils`](crate::Sigils), for the same reason `boundary::permits` takes a
/// slice: a config that would not parse is a thing to say and not a third answer
/// to give here.
#[must_use]
pub fn crossings_in<'a>(
    dirty: &'a [Dirty],
    repo_root: &Path,
    manifest: &'a Manifest,
    held: &[String],
    pulled_under: Option<&str>,
) -> Crossings<'a> {
    let mut crossed: Vec<Crossing<'a>> = Vec::new();
    let mut touched: Vec<Touched<'a>> = Vec::new();

    // Both sides of a rename or a copy, so a file moved *out* of a closed scope
    // is caught: `git` reports the destination as the path and the source as a
    // second field, and only the source says which boundary the work reached
    // across.
    let sides = dirty
        .iter()
        .flat_map(|entry| [Some(entry.path.as_str()), entry.from.as_deref()])
        .flatten();

    // Deduplicated and in the order `git` named the paths, both lists: a halt
    // reason and a pull request body are rendered from one of these values, and
    // two readings that disagreed about the order or repeated a path would read
    // as two findings instead of one.
    for path in sides {
        // Nothing covering the path means nothing drew a boundary here, and a
        // path with no manifest-relative spelling reads the same way rather than
        // making this fallible: `git status` paths are repository-root-relative
        // already, so a path the root rejects takes a `repo_root` that is not the
        // root of the checkout the status came from — a caller's mistake to
        // report, and one every caller has a better sentence for than one
        // invented here.
        let Some(scope) = scope_covering(path, repo_root, manifest).ok().flatten() else {
            continue;
        };

        if scope_opens_to(Some(scope), held) {
            if Some(scope) == pulled_under {
                continue;
            }
            match touched.iter_mut().find(|foreign| foreign.scope == scope) {
                Some(entry) => {
                    if !entry.paths.contains(&path) {
                        entry.paths.push(path);
                    }
                }
                None => touched.push(Touched {
                    scope,
                    paths: vec![path],
                }),
            }
        } else if !crossed
            .iter()
            .any(|crossing| crossing.path == path && crossing.scope == scope)
        {
            crossed.push(Crossing { path, scope });
        }
    }

    Crossings { crossed, touched }
}

#[cfg(test)]
#[path = "tests/crossings.rs"]
mod tests;
