//! Which pacted directories a branch made stale. One function over values: no
//! git, no pass, no write, and nothing read but the working tree's own bytes.
//!
//! The selection rule has three parts and no fourth. A directory is refreshed
//! when it is pacted, when the engine's own judgement calls it stale, and when
//! the branch changed a path at or below it. Staleness is
//! [`decide_state`](warlock_engine::decide_state) against
//! [`subtree_hash`](warlock_engine::subtree_hash) and nothing else — a second
//! opinion invented here would be a second answer to the question `r` and
//! `warlock refresh` already answer, and the two would drift. "Pacted" is not a
//! test either: a directory with no `[[pact]]` row has no entry to judge, so it
//! falls out of the manifest walk below without being asked about.
//!
//! The third part is what keeps a pull request's refresh to the branch's own
//! mess. A repository can be stale in directories nobody on this branch went
//! near — somebody else's uncommitted work, a pact never granted — and paying
//! for those passes here would put documents in a review that has nothing to do
//! with them.
//!
//! Because `subtree_hash` covers a whole subtree, one changed file usually
//! leaves every pacted ancestor stale as well, and all of them are selected.
//! That is the intent rather than an accident, and it is why the order out of
//! here is deepest first: a parent's document is written from its children's, so
//! a parent refreshed before its child is written from a document about to
//! change.

use std::cmp::Ordering;
use std::collections::BTreeSet;
use std::path::Path;

use warlock_engine::{
    Manifest, NodeState, PactEntry, decide_state, subtree_hash, to_manifest_path,
};

// The manifest's spelling of the repository root, which is a pacted module like
// any other. Spelled here because the engine keeps its own copy crate-private.
const ROOT_MODULE: &str = ".";

/// The manifest spelling of every directory to refresh, deepest first.
///
/// `changed` is the branch's changed paths as
/// [`changed_against`](warlock_tui::Repository::changed_against) gives them:
/// repository-root-relative, and including the ones the branch deleted, which
/// still count as changing the directory they were in.
///
/// The answer is module spellings rather than paths because that is the one
/// spelling of a directory the manifest, the boundary and the report all share;
/// a caller that needs the directory itself takes
/// [`from_manifest_path`](warlock_engine::from_manifest_path), which cannot
/// fail.
#[allow(
    dead_code,
    reason = "the freshness pass that gates and runs these directories lands next"
)]
pub(crate) fn made_stale(repo_root: &Path, manifest: &Manifest, changed: &[String]) -> Vec<String> {
    let touched = touched_directories(repo_root, changed);
    let mut selected: Vec<&str> = manifest
        .entries()
        .iter()
        .filter(|entry| touched.contains(entry.module()))
        .filter(|entry| is_stale(repo_root, entry))
        .map(PactEntry::module)
        .collect();
    selected.sort_unstable_by(|left, right| deepest_first(left, right));
    selected.into_iter().map(str::to_owned).collect()
}

// Every directory at or above a changed path. `git` names files rather than the
// directories holding them, so a changed path contributes its ancestors and not
// itself: the file `crates/engine/src/lib.rs` is a change to `crates/engine/src`
// and to everything above it, and no pacted module is ever a file. The root
// module is in the set as soon as there is one changed path at all, since every
// path in the repository is below it.
//
// A path with no manifest spelling is dropped rather than refused. It takes a
// listing from outside this repository to produce one, the function's whole
// answer is which directories inside the repository to refresh, and there is
// nothing truthful to say about such a path here that the caller does not
// already know.
fn touched_directories(repo_root: &Path, changed: &[String]) -> BTreeSet<String> {
    let mut touched = BTreeSet::new();
    for path in changed {
        let Ok(module) = to_manifest_path(repo_root, path) else {
            continue;
        };
        touched.insert(ROOT_MODULE.to_owned());
        let mut parts: Vec<&str> = module.split('/').collect();
        // The changed path itself.
        parts.pop();
        while !parts.is_empty() {
            touched.insert(parts.join("/"));
            parts.pop();
        }
    }
    touched
}

// A hash that cannot be taken is the stale side of this rule, which is what
// `decide_state` is documented as being total for: the content the grant would
// be compared against is unknown. So a pacted directory the branch deleted
// outright is selected, its pass fails, and the caller reports it as left stale
// with that reason — which is a manifest row that needs un-pacting said out
// loud, where dropping it here would be the same row hidden.
fn is_stale(repo_root: &Path, entry: &PactEntry) -> bool {
    let Ok(hash) = subtree_hash(entry.module_path(repo_root)) else {
        return true;
    };
    decide_state(Some(entry), &hash) == NodeState::PactedStale
}

// Deepest first, and alphabetical among equals so two runs over one repository
// order the same list the same way.
fn deepest_first(left: &str, right: &str) -> Ordering {
    depth(right).cmp(&depth(left)).then_with(|| left.cmp(right))
}

fn depth(module: &str) -> usize {
    if module == ROOT_MODULE {
        0
    } else {
        module.split('/').count()
    }
}

#[cfg(test)]
#[path = "tests/freshness.rs"]
mod tests;
