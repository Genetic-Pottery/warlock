//! Which pacted directories a branch made stale, and the pass that puts them
//! back before a pull request is opened.
//!
//! Two halves, and the first of them — [`made_stale`] — is a function over
//! values: no git, no pass, no write, and nothing read but the working tree's own
//! bytes. The second — [`freshened`] — is the pass the pull loop spends: it asks
//! the checkout what the branch changed, gates every selected directory on the
//! boundary, refreshes what is left through the one road `warlock refresh` runs
//! on, and makes the one commit the documents go in.
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
//!
//! The pass composes things that already exist and invents none of them.
//! [`descend`] with [`Descent::Refresh`] is the road `warlock refresh` and the
//! panel's `r` take, so the manifest is written the same number of times and by
//! the same line; [`permits`] is the gate both of those doors ask; the reasons
//! reported are [`boundary`](crate::boundary)'s own sentence and the engine's.
//! What is new here is the order, the report, and the commit.
//!
//! Nothing a pass does ends the run. A closed scope, a descent that refused and a
//! model pass that failed are each *left stale with a reason* rather than an
//! `Err`: the sub-task commits are already on the branch, the push is next, and a
//! document that could not be rewritten is a fact for the pull request body to
//! carry rather than grounds for throwing the work away. The one failure that
//! does leave through an `Err` is the checkout's, because a `git` that cannot say
//! what the branch changed or cannot make the commit is how every other step of
//! the loop fails and neither is a thing to guess at.
//!
//! [`descend`]: crate::descent::descend
//! [`Descent::Refresh`]: crate::descent::Descent::Refresh
//! [`permits`]: crate::boundary::permits

use std::cmp::Ordering;
use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::path::Path;

use warlock_engine::{
    Agent, Manifest, NodeState, PactEntry, decide_state, from_manifest_path, manifest_path, pact,
    subtree_hash, to_manifest_path,
};
use warlock_tui::{Cancel, GitError, Repository};

use crate::boundary::{Operation, permits};
use crate::descent::{Descent, RunEvent, descend};
use crate::error::one_line;
use crate::pulling::StaleDirectory;

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

/// Everything one freshness pass is asked about.
///
/// A struct rather than five parameters because the pass is also a
/// [`Freshens`] method and a free function that takes an agent, a say-when and a
/// sink beside these: eight arguments on one line is the shape that gets one of
/// them passed in the wrong position.
///
/// `repo` is a `&dyn` where the loop's own field is generic, so the seam crosses
/// into a `main.rs` module without a sixth type parameter following it; nothing
/// here calls anything a stand-in cannot answer.
///
/// No `Debug`, for [`Pulling`](crate::pulling::Pulling)'s reason: this is a
/// manifest, a sigil list and a checkout, and a failing assertion anywhere in the
/// suite would dump a screenful of them.
pub(crate) struct Freshening<'a> {
    /// `WAR-141`: the first word of the refresh commit's message, and the only
    /// thing the pass knows about the ticket.
    pub(crate) ticket: &'a str,
    pub(crate) repo: &'a dyn Repository,
    pub(crate) root: &'a Path,
    /// The manifest in hand rather than one loaded here, as
    /// [`descend`](crate::descent::descend) takes one, and the manifest every
    /// descent below threads its answer into.
    pub(crate) manifest: &'a Manifest,
    /// The flattened sigils this machine holds, as
    /// [`permits`](crate::boundary::permits) takes them.
    pub(crate) held: &'a [String],
}

/// What one freshness pass came to: the directories whose documents it put back,
/// and the ones it did not, each with the reason.
///
/// Two lists and not three. A pass that failed is a directory left stale whose
/// reason says what failed, because that is the honest account of the state the
/// reviewer will find: the directory is still stale either way, and "failed"
/// versus "refused" is the sentence rather than the category. Empty on both sides
/// is a branch that left nothing stale, which is the ordinary case and renders as
/// no freshness section at all.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Freshened {
    /// Manifest spellings, in the order the passes ran: children before parents.
    pub(crate) refreshed: Vec<String>,
    pub(crate) left_stale: Vec<StaleDirectory>,
}

/// The freshness pass, behind a seam, as [`Splits`](crate::pulling::Splits) and
/// [`Works`](crate::pulling::Works) are.
///
/// A trait because the pass is a `main.rs` module —
/// [`descend`](crate::descent::descend) and
/// [`permits`](crate::boundary::permits) are private to the binary — and because
/// the loop it is called from is driven in its own tests by fakes that spend no
/// model pass. The implementation on the real road holds the agent and the
/// say-when and calls [`freshened`].
pub(crate) trait Freshens {
    fn freshen(&self, asked: &Freshening<'_>) -> Result<Freshened, GitError>;
}

/// One freshness pass: what the branch made stale, refreshed children before
/// parents under the boundary, and committed once.
///
/// The five steps are in this order for five reasons. The branch's changed paths
/// come first because everything else is a function of them. The selection is
/// [`made_stale`]'s and is taken once, before any pass, so a document a pass
/// writes cannot enlarge the list it is being written from. The gate is asked per
/// directory and before its pass, because asked afterwards it would have
/// overwritten a `WARLOCK.md` that no report puts back. Each descent's manifest is
/// threaded into the next, so the fresh grants a child earned are what its parent
/// is judged against rather than a stale copy. The commit is last and is one.
///
/// Nothing here saves the manifest or writes `.warlock/pacts.toml`:
/// [`descend`](crate::descent::descend) does that once per call, which is the
/// count `warlock refresh` makes, and a save added here would be a second writer
/// of the one file warlock promises to own.
pub(crate) fn freshened(
    asked: &Freshening<'_>,
    agent: &dyn Agent,
    cancel: &Cancel,
    sink: &mut dyn FnMut(RunEvent),
) -> Result<Freshened, GitError> {
    let base = asked.repo.default_branch()?;
    let changed = asked.repo.changed_against(&base)?;
    let selected = made_stale(asked.root, asked.manifest, &changed);
    // Nothing stale is the ordinary ending of a branch that touched no pacted
    // directory, and it asks the checkout nothing further: no status is read, and
    // a commit of nothing would be `git commit` refusing over an empty diff.
    if selected.is_empty() {
        return Ok(Freshened::default());
    }

    let mut manifest = asked.manifest.clone();
    let mut outcome = Freshened::default();
    for module in selected {
        let directory = from_manifest_path(asked.root, &module);
        // The same question `warlock refresh <directory>` is gated on, and no
        // wider. A pass over a directory still descends that directory's whole
        // subtree, so a closed directory underneath a selected ancestor can be
        // re-described by the ancestor's pass — exactly as `warlock refresh .`
        // would re-describe it, and for the reason `boundary`'s doc gives: gating
        // a root refresh on holding every sigil in a monorepo would refuse the
        // ordinary gesture. What this pass promises is narrower and is the thing
        // worth promising: no directory is itself the target of a pass whose
        // covering scope this machine does not hold. It is reported left stale on
        // the strength of that refusal, because refusing is all this pass did
        // about it.
        let verdict = permits(
            Operation::Refresh,
            &directory,
            asked.root,
            &manifest,
            asked.held,
        );
        if let Some(refusal) = verdict.message(&module) {
            outcome.left_stale.push(StaleDirectory {
                directory: module,
                reason: refusal,
            });
            continue;
        }

        match descend(
            Descent::Refresh,
            &directory,
            asked.root,
            &manifest,
            agent,
            cancel,
            sink,
        ) {
            Ok(subtree) => {
                manifest = subtree.manifest;
                match failed(asked.root, &subtree.failures) {
                    Some(reason) => outcome.left_stale.push(StaleDirectory {
                        directory: module,
                        reason,
                    }),
                    None => outcome.refreshed.push(module),
                }
            }
            // A descent that returned an error saved nothing, so the manifest in
            // hand is still the last one a descent handed back rather than a
            // half-written record of this one.
            Err(error) => outcome.left_stale.push(StaleDirectory {
                directory: module,
                reason: one_line(&error.to_string()),
            }),
        }
    }

    commit(asked, &manifest)?;
    Ok(outcome)
}

/// Why a descent that came back left its directory stale, or `None` for one that
/// did not.
///
/// Any failure in the subtree leaves the directory the pass was asked for stale,
/// and that is the engine's arithmetic rather than this module's: a refresh
/// describes children before parents and skips everything above a failure, so a
/// descent with a failure anywhere in it is a descent whose top was not
/// described.
///
/// Every failing directory is named, once each, because the reason is read in a
/// pull request body by whoever has to go and look — and a first failure with the
/// rest dropped is the half of the list that does not help. Each sentence is the
/// engine's own, flattened, as the headless report flattens them.
fn failed(root: &Path, failures: &[pact::Failure]) -> Option<String> {
    let mut reason = String::new();
    let mut named_already: Vec<String> = Vec::new();
    for failure in failures {
        let directory = named(root, failure.directory());
        if named_already.contains(&directory) {
            continue;
        }
        let separator = if reason.is_empty() { "" } else { "; " };
        let _ = write!(
            reason,
            "{separator}`{directory}` — {}",
            one_line(&failure.to_string())
        );
        named_already.push(directory);
    }

    if reason.is_empty() {
        return None;
    }
    Some(format!("the refresh pass failed: {reason}"))
}

/// The one commit the refresh makes, or none at all.
///
/// Decided by what is on disk rather than by what the outcome says, and the two
/// are genuinely different: a pass that failed partway still wrote the documents
/// it got through, and a pass that succeeded over a directory whose document was
/// already word for word what the model produced changed nothing. `git commit`
/// refuses an empty diff, so the question has to be asked of the tree.
fn commit(asked: &Freshening<'_>, manifest: &Manifest) -> Result<(), GitError> {
    let paths = written(asked.repo, asked.root, manifest)?;
    if paths.is_empty() {
        return Ok(());
    }
    asked
        .repo
        .commit_paths(&refresh_message(asked.ticket), &paths)
}

/// Every path in the working tree the refresh is allowed to commit and did
/// change, in `git status`' own order.
///
/// The filter is what keeps the promise that no code file is in this commit.
/// Anything the tree holds that is neither a document the manifest records nor
/// the manifest itself is left where it is — there should be nothing, because the
/// loop refuses a dirty tree and commits every sub-task before this runs, and a
/// "should be nothing" that is committed anyway under a refresh's message is the
/// one mistake this list cannot be allowed to make.
fn written(
    repo: &dyn Repository,
    root: &Path,
    manifest: &Manifest,
) -> Result<Vec<String>, GitError> {
    let allowed = documents_and_manifest(root, manifest);
    Ok(repo
        .dirty()?
        .into_iter()
        .map(|dirty| dirty.path)
        .filter(|path| allowed.contains(path))
        .collect())
}

/// The spellings of every `WARLOCK.md` the manifest records and of the manifest
/// file itself.
///
/// Taken from the manifest rather than by matching file names: the document of a
/// pacted directory is a recorded field, `.warlock/pacts.toml` is
/// [`manifest_path`](warlock_engine::manifest_path)'s answer put back into
/// manifest spelling, and neither is a string this module gets to decide.
fn documents_and_manifest(root: &Path, manifest: &Manifest) -> BTreeSet<String> {
    let mut allowed: BTreeSet<String> = manifest
        .entries()
        .iter()
        .map(|entry| entry.document().to_owned())
        .collect();
    if let Ok(spelling) = to_manifest_path(root, manifest_path(root)) {
        allowed.insert(spelling);
    }
    allowed
}

/// `WAR-141: refresh WARLOCK.md`.
///
/// One message for every refresh commit, naming the ticket and nothing else about
/// the run: the directories are in the pull request body, and a commit subject
/// listing them would be a paragraph in `git log`.
fn refresh_message(ticket: &str) -> String {
    format!("{}: refresh WARLOCK.md", ticket.trim())
}

// A directory in the manifest's spelling, which is how the report, the boundary
// and the headless progress lines all name one. The display form is the fallback
// for a path with no spelling relative to this root, which a failure carried out
// of the engine cannot have and which is not worth a panic if it ever does.
fn named(root: &Path, directory: &Path) -> String {
    to_manifest_path(root, directory).unwrap_or_else(|_| directory.display().to_string())
}

#[cfg(test)]
#[path = "tests/freshness.rs"]
mod tests;
