//! Pacting: what a model pass gets to see, what is done with what it says, and
//! how a whole subtree of directories is pacted or refreshed at once.
//!
//! Three operations, stacked. [`pact_directory`] is one directory, and it is
//! five steps with nothing else in them: gather the
//! directory into a request, describe whatever was too big to send, fit what
//! that comes to inside the request budget, run one pass through an [`Agent`],
//! and write what came back to `<directory>/WARLOCK.md`. It records nothing — no manifest entry, no hash,
//! no grant — because a pact of one directory ends in one document, whatever
//! number of passes it took to write it. [`pact_subtree`] is the operation a
//! keystroke runs:
//! every directory at and below the selected one, children first, and *then*
//! the hashing and the granting that turn what was written into a manifest.
//! [`refresh_subtree`] is the same run over a shorter list: it asks
//! [`decide_state`] about each of those directories in turn and keeps only the
//! ones that are not green, so editing one file costs the passes on the path
//! from it to the refreshed root rather than a pass per directory in the
//! subtree.
//!
//! The two subtree operations are the same code. [`describe_and_grant`] is both
//! phases — write every document, then hash and grant every directory — over
//! exactly the list of directories it is handed, and it neither chooses that
//! list nor reads the manifest. What is left on either side of it is small and
//! is the whole of the difference: which directories reach phase one, and what
//! becomes of the entry of a directory that got no document this run.
//! [`pact_subtree`] hands over everything [`pactable_directories`] found and
//! lets [`rewrite`] drop what earned nothing; [`refresh_subtree`] hands over the
//! stale ones and carries every other entry through byte-identical, because a
//! refresh removes no entry and drops no grant.
//!
//! Section 11 of the design doc calls context scoping "the actual
//! differentiator: maximal relevant context, minimal waste". That sentence is
//! made mechanical one module along, in [`fitting`](crate::fitting): what a
//! directory's request holds, the two byte caps, the sent → summarised → listed
//! ladder that meets them, and the account cache under `.warlock/summaries/`
//! are all argued there. Nothing in this module decides any of it — a pact
//! calls [`fit`] and runs the pass on what comes back.
//!
//! Two of its guarantees are worth restating here, because they are what let
//! this module have no opinion about the caps at all. **Neither cap can fail a
//! pact**: a file too big to send is described, and one that cannot be described
//! is named and sized, so there is always a request to run. And **over budget is
//! never fatal**: every omission is a [`Problem`] reported beside a request that
//! is still perfectly good, never an [`Error`].
//!
//! sorted, so two builds of an unchanged directory are equal values.
//!
//! # What the repository says Warlock is not about
//!
//! `.warlockignore` is the repository's own list of content Warlock does not
//! cover, in gitignore's syntax and read by gitignore's own matcher (the
//! `ignores` module holds the one spelling of the name and the root check).
//! Both of this module's walks read it:
//! an excluded file reaches no request in any of its three states — not sent
//! whole, not listed by name and size, not summarised — and an excluded
//! directory is not in [`pactable_directories`], so a pact of an ancestor gives
//! it no `WARLOCK.md` and no manifest entry. Selecting it directly is no way
//! round that: `pactable_directories` asks about the root it was handed as
//! well, because a walker applies its rules to what it descends into and not to
//! where it was told to start.
//!
//! **Adopting, editing or removing a `.warlockignore` restales every directory
//! whose covered content it changes, all at once.** The rules decide which
//! files a [`subtree_hash`] is taken over, so a rule that excludes a file
//! changes the digest of that file's directory and of every directory above it,
//! and each of them stops matching the hash it was granted for. Turning a
//! shelf of a repository yellow with one line in one file looks alarming and is
//! exactly right: those documents were written from content that is no longer
//! part of what Warlock covers, so what they say about the directory is a claim
//! nobody has re-earned. It is not a bug, it needs no special handling, and
//! there is nothing to suppress — a refresh describes the affected directories
//! once and the repository is green again on the rules it now has. (The rules
//! file is itself an ordinary file in the walk, so it is hashed like any other;
//! see [`hash`] for why that double move is also correct.)
//!
//! # The answer, and how it is turned down
//!
//! A pass does not write the document. It is handed the directory's files and
//! its children's documents and asked to fill a fixed object — one line per
//! file, one per child, a purpose and three short lists — and
//! [`document`](crate::document) checks that answer against the request it was
//! built from and lays the `WARLOCK.md` out itself. What is checked is shape,
//! mechanically: every file the pass was shown has an entry and nothing else
//! does, every entry is one line under its cap, and a route that names a
//! symbol names one that occurs in the file it points at. An answer that fails
//! is sent back once with its defects listed ([`ATTEMPTS`]) and then given up
//! on; an answer that passes is rendered and written. Nothing here reads what
//! a line *says*: a document is a map to be checked against the source, and
//! the git diff is where a wrong line is caught.
//!
//! The document is written the way [`Manifest::save`] writes a manifest,
//! through the same two helpers: to a hidden temporary beside it, then renamed
//! over the document. A pact is long enough to be worth cancelling, so a front
//! end has to be free to kill the pass and quit at any moment without leaving
//! half a `WARLOCK.md` on disk.
//!
//! A pass is never shown the directory's previous document. It is the last
//! pass's claim, not evidence, and a pass that had it carried its sentences
//! forward whether or not the files still supported them — see
//! [`agent::Request`] for the whole argument. Every document is written from
//! the files and the children's documents alone.
//!
//! # Which directories a subtree pact covers, and in what order
//!
//! [`pact_directory`] is one directory, and a pact is a subtree. The list of
//! directories that subtree comes to is [`pactable_directories`], and it is
//! deliberately the *same* list [`load_tree`](crate::load_tree) would have made
//! nodes of — same walk, same ignore rules — so that "everything under here"
//! means on screen what it means to this module. It is ordered children before
//! parents, which is what makes [`agent::ChildDocument`] worth anything: a parent
//! is only pacted once every child below it has written the document the parent
//! will be handed.
//!
//! # Write everything, then hash everything
//!
//! [`pact_subtree`] runs in two phases, and the split is the whole reason it is
//! an operation rather than a loop a caller could write. A directory's hash
//! covers every file below it, its children's `WARLOCK.md` among them, so a
//! per-directory *write, hash, grant* loop grants a parent a hash that the very
//! next write invalidates — and finishes with a subtree that is yellow
//! everywhere except its deepest leaves. So phase one writes every document and
//! records nothing, and phase two starts only once phase one is over: hash each
//! directory, build one [`PactEntry`] for it, and grant it the hash just
//! computed.
//!
//! Nothing is granted that was not earned. A directory whose own document
//! failed gets no entry at all, and every ancestor of it *inside the pact* gets
//! an entry with no grant — which by [`decide_state`]'s
//! rule is pacted and stale, i.e. yellow. That is what the manifest's optional
//! grant was for, so partial completion needs no new state and no new field.
//!
//! # A hash that fails, before a pass and after one
//!
//! Hashing can fail: a file that cannot be read, a name that is not UTF-8, a
//! walk that gave up. Phase two answers that with [`Failure::Hash`] — the
//! document is written, the entry is recorded, and only the grant is missing —
//! and that meaning is exactly what it was: a verdict about a directory a pass
//! has already run on, produced nowhere but phase two.
//!
//! [`refresh_subtree`] hashes each directory earlier and for a different
//! question, deciding stale-or-skip, and a failure there is not a failure at
//! all. A directory with no hash has nothing to compare a grant against, so the
//! honest answer to "is this still the content it was granted for" is no, and
//! it is described. The consequence is worth stating plainly, because a pass is
//! paid for it: such a directory is described, phase two hashes it again, that
//! hash almost certainly fails again for the same reason it failed the first
//! time, and it lands as a [`Failure::Hash`] with an ungranted entry — yellow,
//! with a pass spent on it. That is the honest outcome for a directory
//! something is really wrong with. The alternative is to skip it, and skipping
//! is what a refresh does to green directories: it would call a directory
//! nobody can read fresh, on no evidence, and say nothing about it at all.
//! Better to spend the pass and end up yellow with a named failure.
//!
//! # Saying where a pact is, and stopping it
//!
//! A subtree pact is minutes of model passes, so [`pact_subtree`] takes an
//! [`Observer`]: before each directory it says which one is next, what number it
//! is out of how many, and it listens to the answer. [`Pacting::Stop`] ends the
//! descent there and then, and [`Unwatched`] is the answer for a caller with
//! nothing to show and nothing to cancel.
//!
//! Inside a directory, the same observer is told about each summarising pass
//! just before it runs: which file, which pass of
//! how many that file costs. A two-megabyte lockfile is a dozen model passes
//! inside one directory's turn, and without this the fraction of directories
//! would sit still through all of them. That one only announces — it answers
//! nothing, it stops nothing, and it has a default body that does nothing, so
//! it costs an existing observer no code.
//!
//! Two things this deliberately is not. It is not a *progress channel* — the
//! engine hands a borrowed path to a caller-supplied trait object, with no
//! [`Send`], no [`Sync`], no queue and no opinion about which thread a pact runs
//! on; a front end that wants those wraps them around this. And it is not a
//! *kill switch* for the pass in flight: the question is asked **between**
//! directories only, because the running `claude` belongs to whoever spawned it
//! (see the crate docs on the [`Agent`] seam) and this crate has no way to reach
//! it. The longest a cancel can take, then, is one directory's pass.
//!
//! Cancelling is not failing. A stopped pact is a pact that covered fewer
//! directories, so it reports no [`Failure`] of its own, and phase two runs on
//! exactly what phase one got written: the directories reached keep their
//! documents and earn their entries by the rule above, and the ones never
//! reached are simply undocumented — no entry, gray. Since the walk is children
//! before parents, stopping part way always stops before the ancestors of what
//! is left, so a cancel takes out whole prefixes of the order rather than
//! punching holes in it.
//!
//! # Un-pacting keeps the documents
//!
//! [`unpact_subtree`] is the reverse, and it is deliberately not symmetric: it
//! drops the manifest entries for a directory and everything below it and
//! leaves every `WARLOCK.md` exactly where it is. Nothing in this module — or
//! this crate — deletes a document. A pact is a claim that Warlock keeps
//! judging a directory; the document it produced is the project's, reviewed in
//! the git diff like any other file, and taking back the claim is no reason to
//! throw away the writing. Un-pacting is pure manifest editing: no walk, no
//! hash, no agent, no file opened for writing.
//!
//! **What it does take is the scope.** An entry is the only home a scope has,
//! so dropping the entries at and below the un-pacted directory drops their
//! scopes with them — nothing here special-cases that, and nothing has to. Said
//! out loud because it is the difference between a boundary that quietly
//! evaporated and one somebody chose to tear up: un-pacting is the one
//! operation in this module allowed to lose a scope, and it is a deliberate
//! press on a directory rather than a side effect of a run. A run cannot lose
//! one — it hands over run outcomes, and an outcome has nowhere to put a scope,
//! so a refresh, a pact over a parent, a cancelled run and a partially
//! completed one all leave every scope exactly as they found it. Re-pacting an
//! un-pacted directory brings back an entry with no scope on it; the boundary
//! is written again by the person who wants it, which is the only way one is
//! ever written.

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsStr;
use std::fmt;
use std::fs;
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::str::Utf8Error;

use ignore::WalkBuilder;

use crate::document::{self, ATTEMPTS, Defect, Fill};
use crate::fitting::{
    Fitted, PER_FILE_BYTE_CAP, Problem, byte_count, carried_bytes, carry_hash, fit,
};
use crate::ignores;
use crate::manifest::{ROOT_MODULE, temp_file_name, write_and_sync};
use crate::scope::valid_scope;
use crate::{
    Agent, Manifest, NodeState, PactEntry, agent, decide_state, hash, manifest, now_rfc3339,
    scope_opens_to, subtree_hash, to_manifest_path,
};

/// The directory holding Warlock's own bookkeeping, never part of a request.
pub(crate) const MANIFEST_DIR: &str = ".warlock";

/// The document a directory is described by, and the only file name a child
/// directory contributes to its parent's request.
pub(crate) const DOCUMENT_FILE: &str = "WARLOCK.md";

/// Pact `directory` and everything below it: write every document first, then
/// hash and grant.
///
/// The operation a keystroke runs. `directory` is the selected directory,
/// `root` is the repository root the manifest's paths are relative to,
/// `manifest` is what `.warlock/pacts.toml` says today, `observer` is told where
/// the pact has got to and may stop it, and what comes back is what the manifest
/// should say tomorrow — **this function saves nothing**. A pact writes
/// its manifest once, at the end, through [`Manifest::save`], and doing that is
/// the caller's business for the same reason [`pact_directory`] records
/// nothing: the code that owns the file is the code that decides when it is
/// written and what to say when writing it fails.
///
/// # The two phases
///
/// **Phase one writes.** [`pact_directory`] runs over every directory
/// [`pactable_directories`] found, children before parents, so each parent's
/// pass is handed the documents its children have just written. Nothing is
/// hashed and nothing is recorded here. Before each directory `observer` is
/// told which one is about to be pacted, its position — 1-based, so the first
/// directory is 1 of `total` — and how many there are altogether; a total that
/// never changes over one call.
///
/// **Phase two hashes and grants**, and only starts once phase one has
/// finished for every directory. A directory's [`subtree_hash`] covers its
/// children's documents, so a hash taken before the last write is a hash of
/// something that no longer exists — see the [module docs](self) for why this
/// is a phase rather than a step in a loop. One [`PactEntry`] is built per
/// documented directory, granted the hash just computed, and stamped with a
/// single [`now_rfc3339`] taken for the whole pact.
///
/// # What ends up in the manifest
///
/// Exactly one entry per pacted directory that got a document, whose module is
/// that directory and whose document is that directory's `WARLOCK.md`. An entry
/// already there for one of those directories is *replaced where it sits*, so
/// nothing is duplicated and a manifest's order — and its diff — stays stable.
/// Entries for directories outside the pact are carried through untouched.
///
/// Three things leave a directory less than green, and none of them stops the
/// rest of the pact:
///
/// * **Its document failed.** The directory gets no entry — including no
///   surviving older one, because this run is what the manifest now describes —
///   and it renders gray.
/// * **A directory below it failed.** It gets an entry with no grant: pacted,
///   never judged, yellow.
/// * **Its hash failed.** Same shape, for a different reason: the document is
///   written and there is no hash to grant against, so the entry goes in
///   ungranted rather than the pact falling over.
///
/// # Stopping part way
///
/// An `observer` that answers [`Pacting::Stop`] ends phase one there: the
/// directory it was just offered is not pacted, nor is any directory after it,
/// and phase two runs immediately over what phase one did write. So a cancelled
/// pact comes back as a smaller pact rather than as an error — no [`Failure`] is
/// invented for a directory nobody asked for — and the manifest it hands back is
/// the ordinary one for a subtree that is documented in part: entries, hashed
/// and granted, for the directories that finished, and no entry at all for the
/// ones never reached. The question is asked between directories only; the pass
/// already running is not interrupted, because the process behind it is the
/// caller's ([`Agent`]) and not this crate's.
///
/// ```
/// use std::fs;
/// use warlock_engine::{
///     Agent, Manifest, NodeState, PactedSubtree, Unwatched, agent, decide_state,
///     document::Fill, pact_subtree, subtree_hash,
/// };
///
/// /// The engine's own tests reach a model exactly like this: they don't.
/// struct Canned;
///
/// impl Agent for Canned {
///     fn run(&self, request: &agent::Request) -> Result<agent::Response, agent::Error> {
///         Ok(agent::Response::new(Fill::stub(request).to_json()))
///     }
/// }
///
/// let repo = tempfile::tempdir()?;
/// let engine = repo.path().join("crates").join("engine");
/// fs::create_dir_all(engine.join("src"))?;
/// fs::write(engine.join("src").join("lib.rs"), "//! Core engine.\n")?;
///
/// // `Unwatched` is the caller with nothing to report and nothing to cancel.
/// let PactedSubtree { manifest, failures, .. } =
///     pact_subtree(&engine, repo.path(), &Manifest::new(), &Canned, &mut Unwatched)?;
///
/// assert!(failures.is_empty());
/// assert_eq!(manifest.entries().len(), 2, "the directory, and the one below it");
/// let entry = manifest.entry("crates/engine").expect("the selected directory is pacted");
/// assert_eq!(entry.document(), "crates/engine/WARLOCK.md");
/// assert_eq!(decide_state(Some(entry), &subtree_hash(&engine)?), NodeState::PactedFresh);
///
/// // Saving is the caller's, once, at the end.
/// manifest.save(repo.path())?;
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
///
/// # Errors
///
/// [`Error::Walk`], and nothing else: the one thing that fails the operation as
/// a whole is not being able to list the subtree, because a pact planned from
/// half a walk would silently leave directories out. Everything that goes wrong
/// after that goes wrong for one directory, and comes back as a [`Failure`] in
/// [`PactedSubtree::failures`] alongside the manifest the rest of the subtree
/// earned. A cancelled pact is not an error either — see above.
pub fn pact_subtree(
    directory: impl AsRef<Path>,
    root: impl AsRef<Path>,
    manifest: &Manifest,
    agent: &dyn Agent,
    observer: &mut dyn Observer,
) -> Result<PactedSubtree, Error> {
    let (directory, root) = (directory.as_ref(), root.as_ref());
    let directories = pactable_directories(directory)?;

    let Described {
        outcomes,
        failures,
        problems,
    } = describe_and_grant(&directories, root, &BTreeMap::new(), agent, observer);

    Ok(PactedSubtree {
        manifest: rewrite(manifest, &directories, root, outcomes),
        failures,
        problems,
    })
}

/// Refresh `directory` and everything below it: describe what has gone stale,
/// pass over what is still green.
///
/// The other way back to green, and the cheap one. [`pact_subtree`] buys a model
/// pass for every directory in a subtree whether anything under it moved or not;
/// a refresh asks [`decide_state`] about each directory first and hands the pass
/// only to the ones it calls anything other than [`PactedFresh`]. Editing one
/// file in a forty-directory repository then costs the passes on the path from
/// that file up to the refreshed root, and nothing else. Everything after that
/// choice is [`pact_subtree`]'s machinery unchanged, because it is literally the
/// same code: the two phases, the deepest-first order, the single
/// [`now_rfc3339`] for the whole run, cancellation, [`Failure`]s and
/// [`Problem`]s all behave exactly as they are documented there.
///
/// The arguments and the return type are [`pact_subtree`]'s, down to the
/// promise that **this function saves nothing**: `directory` is the selected
/// directory, `root` the repository root the manifest's paths are relative to,
/// `manifest` what `.warlock/pacts.toml` says today — which is also the whole of
/// what deciding staleness needs, so nothing extra is asked of a caller — and
/// what comes back is what the manifest should say tomorrow.
///
/// # Which directories are described
///
/// Every directory [`pactable_directories`] finds whose entry in `manifest`,
/// judged against what that directory hashes to now, is not [`PactedFresh`]:
/// unpacted, pacted-but-never-judged and pacted-against-other-content are all
/// stale, exactly as [`decide_state`] says. A directory whose
/// path cannot be stored in a manifest has no entry to be fresh by, and a
/// directory whose [`subtree_hash`] fails here has no hash to be fresh against,
/// so both are stale and both are described.
///
/// The described set keeps `pactable_directories`' deepest-first order, so a
/// stale parent is still re-described from its children's newly written
/// documents. And it can be a small set safely, because a grant means more than
/// it looks: [`pact_subtree`] withholds the grant from any directory with an
/// undocumented descendant, so a directory that is fresh implies every directory
/// beneath it is fresh too.
///
/// # What happens to a directory that is skipped
///
/// Nothing at all. Its entry is carried through byte-identical — same module,
/// same document, same `granted_hash`, same `granted_at` — because a refresh
/// removes no entry and drops no grant, anywhere. That is this function's own
/// rule and not the shared core's: [`pact_subtree`] is right to drop the entry
/// of a directory it covered and that earned nothing, since a pact is a claim
/// about the whole subtree it walked, while a refresh is a claim only about the
/// directories it actually described.
///
/// Freshness is still only ever earned. Every grant a refresh writes follows a
/// pass that ran on that directory and a hash taken afterwards; a skipped
/// directory keeps the grant it already had rather than being handed a new one.
///
/// ```
/// use std::cell::Cell;
/// use std::fs;
/// use warlock_engine::{
///     Agent, Manifest, NodeState, PactedSubtree, Unwatched, agent, decide_state,
///     document::Fill, pact_subtree, refresh_subtree, subtree_hash,
/// };
///
/// /// The engine's own tests reach a model exactly like this: they don't.
/// struct Canned {
///     passes: Cell<usize>,
/// }
///
/// impl Agent for Canned {
///     fn run(&self, request: &agent::Request) -> Result<agent::Response, agent::Error> {
///         self.passes.set(self.passes.get() + 1);
///         Ok(agent::Response::new(Fill::stub(request).to_json()))
///     }
/// }
///
/// let repo = tempfile::tempdir()?;
/// let engine = repo.path().join("crates").join("engine");
/// fs::create_dir_all(engine.join("src"))?;
/// fs::write(engine.join("src").join("lib.rs"), "//! Core engine.\n")?;
/// let agent = Canned {
///     passes: Cell::new(0),
/// };
///
/// // A pact first, to have something to refresh: both directories go green.
/// let PactedSubtree { manifest, .. } =
///     pact_subtree(&engine, repo.path(), &Manifest::new(), &agent, &mut Unwatched)?;
/// assert_eq!(agent.passes.get(), 2, "one pass each, children before parents");
///
/// // Nothing has moved, so a refresh describes nothing and costs nothing.
/// let PactedSubtree { manifest, .. } =
///     refresh_subtree(&engine, repo.path(), &manifest, &agent, &mut Unwatched)?;
/// assert_eq!(agent.passes.get(), 2, "nothing stale, no pass");
///
/// // Now a file changes in the parent directory only.
/// let below = manifest.entry("crates/engine/src").expect("the child is pacted").clone();
/// fs::write(engine.join("Cargo.toml"), "[package]\nname = \"engine\"\n")?;
///
/// let PactedSubtree { manifest, failures, .. } =
///     refresh_subtree(&engine, repo.path(), &manifest, &agent, &mut Unwatched)?;
///
/// assert!(failures.is_empty());
/// assert_eq!(agent.passes.get(), 3, "the changed directory, and not the one below it");
/// assert_eq!(manifest.entry("crates/engine/src"), Some(&below), "skipped, grant and all");
/// let entry = manifest.entry("crates/engine").expect("the described directory is pacted");
/// assert_eq!(decide_state(Some(entry), &subtree_hash(&engine)?), NodeState::PactedFresh);
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
///
/// # Errors
///
/// [`Error::Walk`], and nothing else, for [`pact_subtree`]'s reason: a run
/// planned from half a walk would silently leave directories out. A hash that
/// fails while staleness is being decided is not an error — it is a directory
/// to describe.
///
/// [`PactedFresh`]: crate::NodeState::PactedFresh
pub fn refresh_subtree(
    directory: impl AsRef<Path>,
    root: impl AsRef<Path>,
    manifest: &Manifest,
    agent: &dyn Agent,
    observer: &mut dyn Observer,
) -> Result<PactedSubtree, Error> {
    let (directory, root) = (directory.as_ref(), root.as_ref());
    let stale: Vec<PathBuf> = pactable_directories(directory)?
        .into_iter()
        .filter(|candidate| !is_fresh(manifest, root, candidate))
        .collect();

    // What each stale directory looked like when it was last granted, for the
    // early cutoff in phase one. Read here, where the manifest is, so `describe_and_grant`
    // keeps knowing nothing about manifests: it is handed the digests the same
    // way it is handed the list of directories.
    //
    // A directory with no recorded digest simply has no entry here, which is
    // the same answer as a digest that does not match — the pass runs. That is
    // what makes an old manifest, written before the field existed, cost one
    // full refresh and no correctness at all.
    let recorded: BTreeMap<PathBuf, String> = stale
        .iter()
        .filter_map(|candidate| {
            let module = to_manifest_path(root, candidate).ok()?;
            let carry = manifest.entry(&module)?.carry_hash()?;
            Some((candidate.clone(), carry.to_string()))
        })
        .collect();

    let Described {
        outcomes,
        failures,
        problems,
    } = describe_and_grant(&stale, root, &recorded, agent, observer);

    // Covering nothing is the whole of the carry-through: `rewrite` drops an
    // existing entry only where the run covered its module and it earned
    // nothing this time, and a refresh never claims that about a directory. So
    // a skipped directory's entry survives untouched, and so does the entry of
    // a stale directory whose pass failed — a refresh that could not re-describe
    // something leaves it as stale as it found it rather than un-pacting it.
    Ok(PactedSubtree {
        manifest: rewrite(manifest, &[], root, outcomes),
        failures,
        problems,
    })
}

/// Whether `directory` is green as the manifest stands: a grant recorded for it
/// that equals what it hashes to now.
///
/// [`decide_state`]'s judgement and nothing on top of it, with the two ways of
/// having no answer folded into the stale side. A directory that cannot be
/// spelled as a manifest path has no entry, which decides as
/// [`Unpacted`](crate::NodeState::Unpacted); a directory whose [`subtree_hash`]
/// fails has nothing to compare a grant against, and the honest answer to "is
/// this still the content it was granted for" is then no. Both come back
/// `false`, so [`refresh_subtree`] describes them.
fn is_fresh(manifest: &Manifest, root: &Path, directory: &Path) -> bool {
    let entry = to_manifest_path(root, directory)
        .ok()
        .and_then(|module| manifest.entry(&module));
    let Ok(computed) = subtree_hash(directory) else {
        return false;
    };
    decide_state(entry, &computed) == NodeState::PactedFresh
}

/// What the two phases came to: the outcomes they earned, and everything that
/// went wrong on the way without stopping them.
///
/// Not [`PactedSubtree`], because there is no manifest here yet — and not
/// entries either, because a run does not own a whole entry. The outcomes are
/// keyed by stored module path, ready for [`rewrite`] — deciding what an
/// existing entry for a directory this run did not describe deserves is the
/// caller's, not the core's.
#[derive(Debug)]
struct Described {
    /// One [`Outcome`] per directory that got a document, keyed by the path the
    /// manifest stores it under. Granted where the subtree came out whole and
    /// its hash could be taken, ungranted where it did not.
    outcomes: BTreeMap<String, Outcome>,
    /// Every directory that failed, phase one's before phase two's.
    failures: Vec<Failure>,
    /// Every file the byte caps left out of a request.
    problems: Vec<Problem>,
}

/// Everything a run has to say about one directory, and nothing else.
///
/// A pact writes a document and may earn a grant for it; that is the whole of
/// its authority over an entry. Handing [`rewrite`] outcomes rather than whole
/// [`PactEntry`] values is what makes that a rule the compiler keeps: a field
/// on `PactEntry` that a person owns rather than a run cannot be spelled here,
/// so it cannot be overwritten there.
#[derive(Debug)]
struct Outcome {
    /// The pacted directory as the manifest stores it — [`to_manifest_path`]'s
    /// form, which is also the key this outcome is filed under.
    module: String,
    /// The document this run wrote for it, stored the same way.
    document: String,
    /// The grant it earned, or `None` for a directory left pacted and
    /// unjudged. Hash and timestamp travel as one so an outcome cannot express
    /// a hash without the timestamp saying when it was earned.
    grant: Option<Grant>,
}

/// The half of an [`Outcome`] that says freshness was judged: the subtree hash
/// that was granted, and when.
#[derive(Debug)]
struct Grant {
    /// What the subtree hashed to once every document was on disk.
    hash: String,
    /// The run's single [`now_rfc3339`] reading, RFC 3339.
    at: String,
    /// What this directory must still look like for this grant to be carried
    /// forward without a pass, from [`carry_hash`](crate::fitting::carry_hash):
    /// its request's inputs and the document this grant is for.
    ///
    /// `None` where it could not be taken. Recorded that way rather than
    /// dropped, because a grant with no input digest is a grant with no
    /// shortcut next time, and that is the honest and the safe pair.
    carry: Option<String>,
}

impl Outcome {
    /// Write this outcome onto an entry the manifest already holds: module,
    /// document, `granted_hash` and `granted_at` overwritten, every other field
    /// on `entry` left exactly as it was.
    fn apply(self, entry: &mut PactEntry) {
        entry.overwrite_run_fields(
            self.module,
            self.document,
            self.grant
                .map(|Grant { hash, at, carry }| (hash, at, carry)),
        );
    }

    /// This outcome as a brand-new entry, for a module the manifest has never
    /// held: there is nothing there to preserve, so an outcome is the whole of
    /// it.
    fn into_entry(self) -> PactEntry {
        let entry = PactEntry::stored(self.module, self.document);
        match self.grant {
            Some(Grant { hash, at, carry }) => {
                let entry = entry.with_grant(hash, at);
                match carry {
                    Some(carry) => entry.with_carry_hash(carry),
                    None => entry,
                }
            }
            None => entry,
        }
    }
}

/// The document `pacted` already has, where this run need not pay for another:
/// `Some(path)` to carry that document forward, `None` to run the pass.
///
/// # What is being decided
///
/// The directory is stale — [`refresh_subtree`] filtered for that — so
/// something at or below it moved. Stale is the trigger to *look*, and this is
/// the look: if what a pass would be shown is byte for byte what the last pass
/// was shown, then the document it would write is a document about the same
/// evidence, and there is nothing to pay a model to re-read. The existing
/// document stands, and phase two grants it against the directory's new subtree
/// hash.
///
/// This is a build system's early cutoff, and it is the same bargain: the
/// trigger stays mechanical, no opinion about whether a change *mattered* is
/// asked for or offered, and what gets skipped is only ever work that provably
/// had nothing new to read.
///
/// # Why it compounds
///
/// Editing one file restales every directory from it up to the root, and each
/// of those ancestors holds a request made of its own files and its children's
/// documents — nothing from further down. So if the edit did not change the
/// leaf's document, no ancestor's request changed either, and the refresh costs
/// the one pass at the bottom rather than one per level. That is why the digest
/// is taken in phase one, where the children are already final.
///
/// # Where it fires, measured rather than assumed
///
/// On a real model, over a three-level fixture: a markdown file added or edited
/// anywhere below cost three passes and now costs none, because prose reaches
/// no request and so moves no digest at any level. **A code edit still costs
/// the full path**, and will keep doing so — a pass rewrites its whole document
/// in fresh wording every time it runs, so the leaf's document always moves,
/// and a moved child document is a moved parent request. One measured case: a
/// directory whose own file was byte-identical came back with every line
/// reworded, same substance throughout.
///
/// The cutoff is therefore worth a great deal on prose and nothing on code, and
/// it is never worse than the cascade it replaces. Closing the code half means
/// digesting what a child's document *asserts* — its file and directory keys,
/// its declared symbols — rather than the sentences it says them in, so that
/// rewording stops propagating. That is a change to what goes into the digest
/// and not to any of the reasoning above.
///
/// # The three conditions, all mechanical
///
/// * **A digest was recorded** to compare against. An entry granted before the
///   field existed has none, and pays for one pass to get one.
/// * **This run could take one.** [`carry_hash`] answers `None` on any failure
///   and `None` never matches, so every way of not knowing runs the pass.
/// * **The document is actually on disk.** A grant pointing at a file nobody
///   wrote is precisely the false green the module exists to avoid, and a
///   digest match says nothing about whether the document survived.
fn carried_document(
    pacted: &Path,
    carry: Option<&str>,
    recorded: &BTreeMap<PathBuf, String>,
) -> Option<PathBuf> {
    let before = recorded.get(pacted)?;
    if carry? != before {
        return None;
    }
    let carried = pacted.join(DOCUMENT_FILE);
    carried.is_file().then_some(carried)
}

/// The two phases themselves, over exactly the `directories` handed in:
/// describe every one of them, then hash and grant.
///
/// The shared middle of the subtree operations. `directories` is the list to
/// describe, deepest-first as [`pactable_directories`] orders it, and it is the
/// whole of what this function will touch: nothing here decides which
/// directories belong on it, and nothing here looks at the manifest. `root` is
/// the repository root the entries' paths are relative to, `agent` runs the
/// passes and `observer` is told where the run has got to and may stop it —
/// with a `total` that is `directories.len()`, so a run counts the directories
/// it will actually describe.
///
/// Both phases, the cancellation rule, the single [`now_rfc3339`] for the whole
/// run and the ungranted entry a directory with an undocumented descendant
/// earns are described in full on [`pact_subtree`], which is this function plus
/// a walk and a [`rewrite`].
fn describe_and_grant(
    directories: &[PathBuf],
    root: &Path,
    recorded: &BTreeMap<PathBuf, String>,
    agent: &dyn Agent,
    observer: &mut dyn Observer,
) -> Described {
    let mut failures = Vec::new();
    let mut problems = Vec::new();

    // Phase one: every document, children before parents, and nothing else.
    // `documents` is what got written and `undocumented` is what did not; both
    // are read in phase two and neither is acted on before it.
    let total = directories.len();
    let mut documents = BTreeMap::new();
    // What each directory that got a document this run must still look like for
    // its grant to be carried next time, kept from phase one so phase two
    // records the digest that goes with the document actually on disk.
    let mut carries: BTreeMap<PathBuf, Option<String>> = BTreeMap::new();
    let mut undocumented = Vec::new();
    for (index, pacted) in directories.iter().enumerate() {
        // Asked before the pass, not after it, so a front end names the
        // directory that is being worked rather than the one that just
        // finished — and so a cancel arriving now costs no pass at all.
        if observer.starting(pacted, index + 1, total) == Pacting::Stop {
            // Everything from here down is undocumented by this run, this
            // directory included: it was offered and turned down. Recorded the
            // same way a failure is, so phase two's partial rule needs to know
            // nothing about cancellation — but with no `Failure` beside it,
            // because nobody asked for these and nothing went wrong.
            undocumented.extend(directories[index..].iter().cloned());
            break;
        }
        // What this directory would have to be for its recorded grant to stand
        // untouched, asked now rather than in phase two: the children below it
        // have already had their turn, so their documents are final and this is
        // the request that would go out.
        let carry = carry_hash(pacted);

        // Early cutoff: the pass is skipped where nothing it would read moved.
        if let Some(carried) = carried_document(pacted, carry.as_deref(), recorded) {
            carries.insert(pacted.clone(), carry);
            documents.insert(pacted.clone(), carried);
            if !undocumented
                .iter()
                .any(|missing| missing.starts_with(pacted))
            {
                observer.unchanged(pacted);
            }
            continue;
        }

        // Through the watched form, so every summarising pass this directory
        // pays for is announced to the same observer that was just asked about
        // the directory itself.
        match pact_directory_watched(pacted, agent, observer) {
            Ok(Pacted {
                document,
                problems: caps,
            }) => {
                problems.extend(caps);
                // Taken again, now the pass has written its document: the
                // document is part of this digest, so the reading from before
                // the pass describes the directory as it no longer is. Only
                // this directory's own `WARLOCK.md` moved in between — it is
                // prose, so it is in no request and no other directory's digest
                // — which is what makes the two readings comparable at all.
                carries.insert(pacted.clone(), carry_hash(pacted));
                documents.insert(pacted.clone(), document);
                // Children before parents means everything under this
                // directory has already had its turn, so whether its subtree
                // is whole is decided here and now — the same prefix test
                // phase two grants by. Announced only when it is, because the
                // announcement is what a front end colours done: a directory
                // above a failure is going to be recorded without a grant,
                // and that is not a colour to preview.
                if !undocumented
                    .iter()
                    .any(|missing| missing.starts_with(pacted))
                {
                    observer.documented(pacted);
                }
            }
            Err(error) => {
                undocumented.push(pacted.clone());
                failures.push(Failure::Document { source: error });
            }
        }
    }

    // Phase two: hashing and granting, now that every document phase one was
    // going to write is on disk. One timestamp for the whole pact — the entries
    // record a single event, and a per-directory clock reading would only
    // invite someone to read an ordering into it.
    let granted_at = now_rfc3339();
    let mut outcomes = BTreeMap::new();
    for pacted in directories {
        let Some(document) = documents.get(pacted) else {
            // No document, no outcome: a directory this run failed to describe
            // is not a directory this run pacted.
            continue;
        };
        // Both paths spelled the manifest's way here, where the directory to
        // blame is still in hand — an outcome carries stored paths and no root
        // to re-derive them from.
        let stored = to_manifest_path(root, pacted)
            .and_then(|module| to_manifest_path(root, document).map(|document| (module, document)));
        let (module, document) = match stored {
            Ok(paths) => paths,
            Err(source) => {
                failures.push(Failure::Record {
                    directory: pacted.clone(),
                    source,
                });
                continue;
            }
        };

        // No grant is the whole representation of partial completion: pacted,
        // never judged, yellow. `starts_with` is a component-wise prefix test,
        // so `src` never counts as an ancestor of `src-tests`.
        let ungranted = undocumented
            .iter()
            .any(|missing| missing.starts_with(pacted));
        let grant = if ungranted {
            None
        } else {
            match subtree_hash(pacted) {
                Ok(hash) => Some(Grant {
                    hash,
                    at: granted_at.clone(),
                    carry: carries.get(pacted).cloned().flatten(),
                }),
                Err(source) => {
                    failures.push(Failure::Hash {
                        directory: pacted.clone(),
                        source,
                    });
                    None
                }
            }
        };

        outcomes.insert(
            module.clone(),
            Outcome {
                module,
                document,
                grant,
            },
        );
    }

    Described {
        outcomes,
        failures,
        problems,
    }
}

/// `manifest` with the run's outcomes written into it: the fields a run owns
/// updated, everything else left exactly as it was.
///
/// `outcomes` is what the run earned, keyed by stored module path, and
/// `directories` is everything it covered — including the directories that
/// earned nothing, whose entries go. An outcome met by an entry that is already
/// there writes exactly four fields on it — module, document, `granted_hash`
/// and `granted_at`, the last two cleared when the outcome earned no grant —
/// and leaves every other field that entry carries untouched. It also keeps its
/// position, so a re-pact moves no lines around; modules the manifest has never
/// seen are appended in stored-path order, which puts a parent above the
/// children it gained.
///
/// Taking outcomes rather than entries is the point: there is no way to hand
/// this function a whole [`PactEntry`], so there is no way for a run to erase a
/// field of one.
fn rewrite(
    manifest: &Manifest,
    directories: &[PathBuf],
    root: &Path,
    mut outcomes: BTreeMap<String, Outcome>,
) -> Manifest {
    // Every module the pact is entitled to speak for. A directory whose path
    // cannot be stored has no entry to match against anyway, so a failure to
    // name one here can only leave an entry alone, never drop the wrong one.
    let covered: BTreeSet<String> = directories
        .iter()
        .filter_map(|pacted| to_manifest_path(root, pacted).ok())
        .collect();

    let mut kept = Vec::with_capacity(manifest.entries().len() + outcomes.len());
    for existing in manifest.entries() {
        if let Some(outcome) = outcomes.remove(existing.module()) {
            // Written onto the entry that is already there, where it sat: one
            // entry per module, never two, and nothing on it lost but the
            // fields the run is entitled to speak for.
            let mut entry = existing.clone();
            outcome.apply(&mut entry);
            kept.push(entry);
        } else if !covered.contains(existing.module()) {
            kept.push(existing.clone());
        }
    }
    kept.extend(outcomes.into_values().map(Outcome::into_entry));
    Manifest::with_entries(kept)
}

/// `manifest` with the pact on `directory` and on every directory below it
/// dropped — and **nothing on disk touched at all**.
///
/// The other half of the pact key: pressing it on a directory that is already
/// pacted un-pacts that directory and its whole subtree, which is a manifest
/// edit and only a manifest edit. Every `WARLOCK.md` those pacts wrote stays
/// exactly where it is with exactly the bytes it had — the document belongs to
/// the project, and the pact was only Warlock's standing claim to keep judging
/// it. Dropping the claim is not a reason to delete somebody's documentation,
/// and this function opens no file for writing, so it cannot.
///
/// Like [`pact_subtree`], **this saves nothing**: `directory` and `root` are the
/// selected directory and the repository root, `manifest` is what
/// `.warlock/pacts.toml` says today, and what comes back is what it should say
/// tomorrow. Entries outside the subtree are carried through in place,
/// unchanged, grants and scopes and all.
///
/// # The scopes go with the entries
///
/// An entry at or below `directory` takes its scope with it. The entry is the
/// only home a scope has, so dropping the entry is dropping the scope, and this
/// function does not special-case it — but it is worth saying in as many words,
/// because it is the one place a boundary is lost. It is lost because somebody
/// asked for the subtree holding it, not because a run wandered over it: no pact
/// operation can touch a scope, since a run hands over outcomes and an outcome
/// has nowhere to put one. A later re-pact of the same directory brings back an
/// entry with no scope, to be written again by whoever wants it. Scopes on
/// entries outside the subtree are untouched, like everything else on them.
///
/// **Who may ask is not settled here.** This function is arithmetic on a
/// manifest and holds no opinion about sigils; both callers of it — the `p` key
/// and `warlock unpact` — ask [`closed_scopes_at_or_below`] first and refuse
/// when any scope at or below the directory is one this machine does not hold,
/// so a boundary is never dropped by a machine standing outside it. The rule and
/// its reasoning are
/// `docs/warlock-decision-un-pacting-across-a-descendant-scope.md`.
///
/// # What counts as below
///
/// Descendants are matched on the paths **as the manifest stores them** —
/// relative, forward slashes, [`to_manifest_path`]'s form — not on the
/// filesystem, so an entry is dropped whether or not its directory is still
/// there. The match is by whole path segment: `crates/engine` covers
/// `crates/engine/src` and does not cover `crates/engine-tools`, which a plain
/// string prefix would have taken with it.
///
/// The repository root stores as `"."`, and it is every module's ancestor: un-
/// pacting the root drops every entry there is. It is also nobody's descendant,
/// so un-pacting `crates/engine` never drops the root's own entry.
///
/// ```
/// use warlock_engine::{Manifest, PactEntry, unpact_subtree};
///
/// let entry = |module: &str| PactEntry::new(".", module, format!("{module}/WARLOCK.md"));
/// let manifest = Manifest::with_entries([
///     entry("crates/engine")?,
///     entry("crates/engine/src")?,
///     entry("crates/engine-tools")?,
/// ]);
///
/// let manifest = unpact_subtree("crates/engine", ".", &manifest)?;
///
/// let modules: Vec<&str> = manifest.entries().iter().map(PactEntry::module).collect();
/// assert_eq!(modules, ["crates/engine-tools"], "a sibling that shares a prefix is not below");
/// # Ok::<(), warlock_engine::manifest::Error>(())
/// ```
///
/// # Errors
///
/// [`manifest::Error::PathOutsideRoot`] or [`manifest::Error::NonUtf8Path`] if
/// `directory` has no manifest-relative form, i.e. it does not sit under `root`
/// or cannot be spelled as text. Such a directory has no entry in this manifest
/// to drop, but saying so is better than handing back a manifest that quietly
/// dropped nothing when the caller asked for a subtree to go.
pub fn unpact_subtree(
    directory: impl AsRef<Path>,
    root: impl AsRef<Path>,
    manifest: &Manifest,
) -> Result<Manifest, manifest::Error> {
    let selected = to_manifest_path(root, directory)?;
    Ok(Manifest::with_entries(
        manifest
            .entries()
            .iter()
            .filter(|entry| !at_or_below(entry.module(), &selected))
            .cloned(),
    ))
}

/// Whether the stored module path `module` is `selected` itself or sits below
/// it.
///
/// Both are in the manifest's own form, so this is string work on forward-slash
/// paths and never a question for the filesystem. The `/` in the prefix test is
/// what makes it segment-wise rather than textual — without it `crates/engine`
/// would swallow `crates/engine-tools`.
fn at_or_below(module: &str, selected: &str) -> bool {
    // The repository root is above everything, itself included.
    selected == ROOT_MODULE
        || module == selected
        || module
            .strip_prefix(selected)
            .is_some_and(|below| below.starts_with('/'))
}

/// The distinct scopes carried at or below `directory` that a machine holding
/// `held` does not open: what an un-pact of that subtree would destroy from
/// outside it.
///
/// [`unpact_subtree`] drops the entry of every module at or below the directory
/// it is handed, and an entry is the only home a scope has, so an un-pact is the
/// one act in warlock that loses a boundary. [`scope_covering`](crate::scope_covering)
/// and [`scope_opens_to`] answer whether the *target* is open, which is a different
/// question — coverage walks up, and an unscoped `crates` is the absence of a
/// statement rather than permission over the statements below it. This is the
/// downward question: what does this act **reach**. A boundary that goes when
/// somebody stands above it and aims at its parent is not a boundary.
///
/// It is a report and nothing else. Like every other answer in this crate it
/// refuses no keystroke and prints no words — an empty answer is "nothing at or
/// below this is shut to you", a non-empty one is the scopes a caller may name
/// in a refusal, and what to do about either belongs to the caller.
///
/// # Below means what an un-pact means by it
///
/// The same test as [`unpact_subtree`]'s, on the manifest's own stored paths, so
/// the set of entries asked about here is exactly the set that would go: matched
/// by whole path segment, so `crates/engine` does not swallow
/// `crates/engine-tools`; matched on the stored form rather than the filesystem,
/// so a directory that is no longer there still answers; and the repository root
/// is above everything, so `unpact .` is asked about every entry in the
/// manifest, whatever the root's own entry says or does not say.
///
/// # A scope that is not one does not block
///
/// The scope on each entry is read through the same rule as everywhere else: a
/// string [`validate_scope`](crate::validate_scope) refuses reads as no scope,
/// so it neither blocks nor is named. One place reading a hand-edited typo as a
/// boundary when [`scope_covering`](crate::scope_covering) reads it as none
/// would make the boundary two rules instead
/// of one. Naming an invalid scope in a *report* of what an un-pact dropped is a
/// different act — that is a word somebody put in the file — and it is not this
/// function's.
///
/// # Distinct scopes, in manifest order
///
/// The answer is what a person would have to hold to proceed, deduplicated, in
/// the order first met walking the entries as the manifest stores them. There
/// are few of them by design — a boundary is architecture — while the paths
/// carrying them are unbounded, which is why this answers with the scopes and
/// leaves locating them to a query about a path. The order is a property of the
/// manifest alone, so two callers asking the same question of the same manifest
/// on the same machine name the same blocking scope in the same words.
///
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
///
/// # Errors
///
/// [`manifest::Error::PathOutsideRoot`] or [`manifest::Error::NonUtf8Path`] if
/// `directory` has no manifest-relative form, i.e. it does not sit under `root`
/// or cannot be spelled as text — the same refusal, on the same grounds, that
/// [`unpact_subtree`] gives for that directory, so a caller asking both never
/// gets a clear answer here and an error there.
pub fn closed_scopes_at_or_below<'manifest>(
    directory: impl AsRef<Path>,
    root: impl AsRef<Path>,
    manifest: &'manifest Manifest,
    held: &[String],
) -> Result<Vec<&'manifest str>, manifest::Error> {
    let selected = to_manifest_path(root, directory)?;

    let mut blocking: Vec<&str> = Vec::new();
    for entry in manifest.entries() {
        if !at_or_below(entry.module(), &selected) {
            continue;
        }
        // `Some(scope)` here is a scope the engine agrees is one; anything else
        // on the entry has already read as saying nothing.
        let Some(scope) = valid_scope(entry) else {
            continue;
        };
        if !scope_opens_to(Some(scope), held) && !blocking.contains(&scope) {
            blocking.push(scope);
        }
    }
    Ok(blocking)
}

/// Pact one directory: gather it, describe what was too big to send, run one
/// pass through `agent`, and write what came back to `<directory>/WARLOCK.md`.
///
/// # Fitting the directory into the request
///
/// What the pass is handed is [`fit`]'s answer, and this
/// function makes none of the decisions in it. The walk, the per-file cap, the
/// accounts of whatever was over it, the whole-request cap and the demotions
/// that meet it all live in [`fitting`](crate::fitting), behind that one call,
/// and the guarantee that matters here is the one it makes: **neither cap can
/// fail a fit**. A file too big to send is described, a file that cannot be
/// described is named and sized, and either way there is a request to run. So
/// no [`Error`] variant below is reachable from anything about the caps.
///
/// The [`Problem`]s that came back with it are files that did not go in whole.
/// They are reported, not acted on — a pact over budget is still a pact — and a
/// file that reached the pass as a summary is not among them, because nothing
/// about it was left out.
///
/// The pass is asked to fill the object [`document`] derives from the fitted
/// request, and the document is what [`document::render`] lays out from an
/// accepted fill: warlock's shape, the model's lines. An answer
/// [`document::accept`] turns down is asked for once more with its defects
/// listed, and a second refusal is [`Error::Refused`]. The document is written
/// over whatever was there before, unconditionally and without reading it: an
/// existing document is not a special case anywhere in this operation, and no
/// pass is shown it.
///
/// The write is **atomic**: the bytes go to a hidden temporary file in the same
/// directory and are renamed over `WARLOCK.md`, so `WARLOCK.md` holds the whole
/// old document or the whole new one and never a prefix of either — not even if
/// the process is killed in the middle of a pact. The temporary is never
/// visible to this crate's walks (hidden entries are skipped, so it reaches no
/// tree, no [`subtree_hash`] and no request) and is left behind on neither the
/// success nor the failure path.
///
/// Nothing is recorded. No manifest entry, no subtree hash, no grant: this is
/// one request, one response, one file, and a caller that wants the directory
/// to go green does that afterwards with what it knows about the rest of the
/// subtree.
///
/// Nothing is cached either. A file too big to send is reduced to its own
/// declaration lines, which costs a pass over the text in memory and no model
/// pass at all, so there is nothing worth keeping between runs.
///
/// `&dyn Agent` rather than a generic: there is one code path whatever the
/// implementation is, a boxed agent works without a second signature, and a
/// concrete fake in a test still coerces at the call site.
///
/// There is no `root` parameter and nothing under `.warlock/` is touched: a
/// directory is fitted from its own files and its children's documents, and
/// the files too big to send are reduced to their declaration lines by the
/// language table rather than described by passes whose answers had to be
/// cached. One directory in, one document out.
///
/// ```
/// use std::fs;
/// use warlock_engine::{agent, document::Fill, Agent, Pacted, pact_directory};
///
/// /// The engine's own tests reach a model exactly like this: they don't. A
/// /// stub fill is the answer a pass would give with every slot filled in.
/// struct Canned;
///
/// impl Agent for Canned {
///     fn run(&self, request: &agent::Request) -> Result<agent::Response, agent::Error> {
///         Ok(agent::Response::new(Fill::stub(request).to_json()))
///     }
/// }
///
/// let dir = tempfile::tempdir()?;
/// fs::write(dir.path().join("lib.rs"), "//! Core engine.\n")?;
///
/// let Pacted { document, problems } = pact_directory(dir.path(), &Canned)?;
///
/// assert_eq!(document, dir.path().join("WARLOCK.md"));
///
/// // Warlock's layout, behind the one constant it puts in front of every
/// // document saying what kind of thing it is: a map of the directory to be
/// // checked against the source, not a specification of it.
/// let written = fs::read_to_string(&document)?;
/// assert!(written.starts_with("<!-- warlock -->"));
/// assert!(written.contains("## Files\n\n- `lib.rs` (17 B) — "));
/// assert!(problems.is_empty());
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
///
/// # Errors
///
/// [`Error`], every variant of which names `directory` (see
/// [`Error::directory`]), because a caller pacting a subtree is holding a list
/// of these and has to be able to say which directory each one is about.
///
/// * [`Error::Walk`] or [`Error::Path`] if there is no request to build — see
///   [`gather_request`](crate::fitting::gather_request). Neither byte cap is ever one of these.
/// * [`Error::Refused`] if the pass produced no usable document: the agent
///   returned an [`agent::Error`], or the answer did not fit the object it was
///   asked to fill in any of [`ATTEMPTS`] passes. **Nothing is written on this
///   path**: a directory with no document still has none, and an existing
///   document is byte-identical to what it was before.
/// * [`Error::Write`] if the document could not be written, whether the
///   temporary file or the rename over it was what failed. A different kind of
///   failure from a refusal — the answer was good and the disk said no — and
///   either way `WARLOCK.md` is byte for byte what it was before.
pub fn pact_directory(directory: impl AsRef<Path>, agent: &dyn Agent) -> Result<Pacted, Error> {
    pact_directory_watched(directory.as_ref(), agent, &mut Unwatched)
}

/// [`pact_directory`], with somewhere to announce the summarising passes to.
///
/// The whole of the difference is `observer`, which hears
/// [`summarising`](Observer::summarising) immediately before every model pass
/// spent describing a file too big to send — and hears nothing at all for a file
/// whose account came from the cache, because that file costs no passes — and
/// hears [`requesting`](Observer::requesting) once, when the request those
/// passes were spent on is handed over.
/// [`starting`](Observer::starting) is not called from here: which directory a
/// pact is on is [`pact_subtree`]'s to say, and this function pacts exactly one.
///
/// Private, and the public entry point is the three-argument
/// [`pact_directory`] above it: a caller pacting one directory has nothing to
/// report progress about, and a caller that does have a front end reaches this
/// through [`pact_subtree`], which hands down the observer it was given.
fn pact_directory_watched(
    directory: &Path,
    agent: &dyn Agent,
    observer: &mut dyn Observer,
) -> Result<Pacted, Error> {
    // The whole of what a directory costs to turn into a request: the walk, the
    // per-file cap, the accounts of whatever was over it, the request cap, and
    // the announcement of what the finished request weighs. One call rather than
    // four, because the order they go in and the numbers that get announced are
    // not this function's business to get right — see [`mod@crate::fitting`].
    //
    // The prompt is written onto the request afterwards, not here: what a pass
    // is asked to fill is derived from which files and children the fitting
    // left in the request, and that is not known until it comes back.
    //
    // `root` is where `.warlock/` — and so the summary cache — is found by
    // joining, and is taken here rather than discovered; see the docs above.
    let Fitted {
        request,
        problems,
        described,
    } = fit("", directory, agent, observer)?;
    let expected = document::Expected::of(&request);

    // Up to `ATTEMPTS` passes. The first is asked for the whole object. A
    // second is asked only for what the first got wrong, with the first's
    // defects and its own words for those slots in front of it, and its answer
    // is a patch written over the first ([`document::Repair`]) — unless the
    // first was not an object at all, in which case it is asked cold again.
    // An entry for a file that is not here is dropped without a pass. A
    // transport failure ends it at once — a pass that produced no answer is not
    // a pass that produced a wrong one, and retrying a missing `claude` finds
    // it still missing.
    let mut defects: Vec<Defect> = Vec::new();
    let mut previous: Option<Fill> = None;
    let mut accepted = None;
    for attempt in 1..=ATTEMPTS {
        let repair = previous
            .as_ref()
            .map(|_| document::Repair::of(&defects))
            .filter(|repair| !repair.is_empty());
        let prompt = match (&previous, &repair) {
            (Some(fill), Some(repair)) => {
                document::repair_instructions(&expected, fill, &defects, repair)
            }
            _ => document::instructions(&expected, &defects),
        };
        let asked = request.clone().with_prompt(prompt);
        if attempt > 1 {
            // A second pass is a second wait, announced like the first so a
            // front end's clock counts what is actually being waited on.
            observer.requesting(
                asked.files().len(),
                carried_bytes(asked.files(), asked.child_documents()),
            );
        }
        let response = agent.run(&asked).map_err(|source| Error::Refused {
            directory: directory.to_path_buf(),
            cause: Refusal::Agent { source },
        })?;
        let text = response.text();

        // What this attempt amounts to, once any patch is written over what
        // came before: a fill, and every defect it still has.
        let (candidate, found) = match (&previous, &repair) {
            (Some(fill), Some(repair)) => match document::parse_fill(text) {
                Ok(patch) => {
                    let mended = repair.apply(fill, &patch, &expected);
                    let found = document::check_fill(&mended, &expected);
                    (Some(mended), found)
                }
                Err(defect) => (None, vec![defect]),
            },
            _ => match document::parse_fill(text) {
                Ok(fill) => {
                    // Entries for files that are not here go without a pass;
                    // whatever is wrong after that is what a repair is for.
                    let mended = document::Repair::default().apply(&fill, &fill, &expected);
                    let found = document::check_fill(&mended, &expected);
                    (Some(mended), found)
                }
                Err(defect) => (None, vec![defect]),
            },
        };
        if found.is_empty() {
            accepted = candidate;
            break;
        }
        observer.rejected(directory, &found, attempt, ATTEMPTS);
        defects = found;
        if candidate.is_some() {
            previous = candidate;
        }
    }
    let Some(fill) = accepted else {
        return Err(Error::Refused {
            directory: directory.to_path_buf(),
            cause: Refusal::Malformed {
                defects,
                attempts: ATTEMPTS,
            },
        });
    };

    // Headed with the directory's name rather than its path: the path is
    // absolute, it is the reader's home directory, and it would be committed.
    // Where the document sits says the rest.
    let name = directory.file_name().map_or_else(
        || directory.display().to_string(),
        |name| name.to_string_lossy().into_owned(),
    );
    let text = document::render(&name, &fill, &expected, &described);

    // Written beside and renamed over, the same idiom as `Manifest::save` and
    // through the same two helpers. A pact is minutes of model passes that a
    // user is invited to cancel, and a front end that quits mid-pact — killing
    // the pass, restoring the terminal, and never waiting for this function to
    // come back — must not be able to leave half a document behind. A rename is
    // the only way to make that safe: the file is the old document or the new
    // one, never a prefix of either. The temporary lands in the directory just
    // described, which is exactly why it is named with a leading dot: hidden
    // entries are skipped by every [`ignore`] walk in this crate, so it is in no
    // tree, no subtree hash and no request for the moment it exists, and it is
    // removed on both ways out.
    let document = directory.join(DOCUMENT_FILE);
    let temp = directory.join(temp_file_name(DOCUMENT_FILE));
    let write = write_and_sync(&temp, text.as_bytes()).and_then(|()| fs::rename(&temp, &document));
    if let Err(source) = write {
        // Best effort, and nothing to report if it fails: the caller is already
        // being told the document was not written, and a stray dot file is
        // invisible to everything this crate does.
        drop(fs::remove_file(&temp));
        return Err(Error::Write {
            // The document, not the temporary: the caller asked for
            // `WARLOCK.md` and the mechanics of how it is written are this
            // function's business, not something to name in an error.
            directory: directory.to_path_buf(),
            path: document,
            source,
        });
    }

    Ok(Pacted { document, problems })
}

/// Every directory a pact of `root` covers: `root` itself and every non-ignored
/// directory below it, children before parents.
///
/// This is the shape of a subtree pact, and it is one walk with two properties.
///
/// **The same directories the tree has.** The walk is
/// [`load`](crate::load)'s and [`hash`](crate::hash)'s — the [`ignore`] crate,
/// `follow_links(false)`, `require_git(false)`, `.warlock/` pruned by name,
/// `.warlockignore` honoured — so a directory that is gitignored, hidden
/// (`.git/` with it), excluded by the repository or Warlock's own bookkeeping
/// is as absent from a pact as it is from a tree or a digest.
/// Nothing is filtered on top of that: an undocumented directory is exactly the
/// one a pact exists to give a document to, so there is no "already has a
/// `WARLOCK.md`" test here and no "has source in it" test either.
///
/// `root` itself is checked against `.warlockignore` before the walk, because a
/// walker applies its rules to what it descends into and not to where it was
/// told to start. An excluded directory comes back as no directories at all, so
/// pacting it — by keypress, by manifest entry, by any route that reaches
/// [`pact_subtree`] — writes no document and records no entry, exactly as
/// selecting a directory with nothing in it would.
///
/// **Children before parents.** A parent's request carries its immediate
/// children's documents ([`agent::ChildDocument`](crate::agent::ChildDocument)), so pacting a parent before its
/// children hands the pass a stale account of the subtree — or none at all.
/// Reverse path order gets this for free and costs a sort nobody has to trust:
/// every descendant sorts after its own ancestor, so reversing puts every
/// directory after everything below it. Siblings come out in reverse name order,
/// which is arbitrary but fixed — the guarantee is depth, and determinism on top
/// of it.
///
/// Crate-private on purpose: this is [`pact_subtree`]'s ordering, not a second
/// public way to enumerate a project. Callers outside the crate that want the
/// directories of a subtree already have [`load_tree`](crate::load_tree).
///
/// # Errors
///
/// [`Error::Walk`], naming `root`, if the directory cannot be walked: it is not
/// there, it cannot be listed, something vanished from under the walk, or a
/// `.warlockignore` governing it cannot be parsed. Unusable rules are never
/// read as "no rules": a pact that could not tell what the repository excluded
/// would write documents from the very content it asked to keep out. There
/// is no partial answer — a pact planned from half a subtree would silently
/// leave directories out.
pub(crate) fn pactable_directories(root: &Path) -> Result<Vec<PathBuf>, Error> {
    // Asked first, and separately, because the walker below will not apply the
    // rules to the root it is handed. A directory the repository excluded is
    // pactable in no sense — it has no directories, not even itself.
    let ignored = ignores::is_ignored(root).map_err(|source| Error::Walk {
        directory: root.to_path_buf(),
        source,
    })?;
    if ignored {
        return Ok(Vec::new());
    }

    let walker = WalkBuilder::new(root)
        // The same three rules as `load` and `hash`, for the same reasons: a
        // symlinked cycle has to terminate, a fixture with a `.gitignore` and
        // no `.git` still has to be ignored properly, and `.warlock/` is
        // Warlock's own bookkeeping rather than content of the module.
        .follow_links(false)
        .require_git(false)
        .filter_entry(|entry| entry.file_name() != OsStr::new(MANIFEST_DIR))
        // The repository's own exclusions, read by the same crate that reads
        // `.gitignore` and with the same semantics, because it is the same
        // matcher — and read here so that what a pact covers is what a tree
        // shows and what a hash judges.
        .add_custom_ignore_filename(ignores::FILENAME)
        .build();

    // A set, so whatever order the walker offered is thrown away rather than
    // reversed: the ordering below is a property of the paths, not of the
    // filesystem. Files are not collected at all — a pact is over directories,
    // and each one gathers its own files when its turn comes.
    let mut directories = BTreeSet::new();
    for entry in walker {
        let entry = entry.map_err(|source| Error::Walk {
            directory: root.to_path_buf(),
            source,
        })?;
        // A rule file the walker could not use is reported beside its directory
        // rather than in place of it, and taking that as "no rules" would pact
        // the content the repository excluded. So it is promoted to the failure
        // it is, naming the file and the line.
        if let Some(source) = entry.error() {
            return Err(Error::Walk {
                directory: root.to_path_buf(),
                source: source.clone(),
            });
        }
        // Directories only. With `follow_links(false)` a symlinked directory
        // reports as a symlink, so it is neither descended into nor pacted as
        // whatever it points at.
        if entry.file_type().is_some_and(|kind| kind.is_dir()) {
            directories.insert(entry.into_path());
        }
    }
    Ok(directories.into_iter().rev().collect())
}

/// Read one file for a person to look at: its text, cut at
/// [`PER_FILE_BYTE_CAP`].
///
/// The same cap [`gather_request`](crate::fitting::gather_request) sends a file under, with no second constant
/// anywhere: what a reader is shown of a file is exactly what a model is shown
/// of it. For a file at or under the cap that is byte for byte the bytes its
/// [`agent::File`] carries. Above the cap the two diverge in the only way they
/// can — a request drops the file's contents and lists its size, while a reader
/// gets the first `PER_FILE_BYTE_CAP` bytes and is told they are not all of it.
///
/// Nothing is written and nothing is run: this opens the file, reads at most
/// one byte past the cap, and closes it. The file's bytes are the same
/// afterwards, and an enormous file is never held whole in memory on the way.
///
/// # The cut is a fact, not a sentence
///
/// [`Viewed::cut`] is a `bool`, and there is no marker line anywhere in
/// [`Viewed::text`]. **The caller words the cut.** The words belong to whatever
/// is showing the file — a panel says it in its own voice, at its own width, in
/// the vocabulary its other lines use — and a sentence of the engine's mixed
/// into the text would be a line the file does not have, indistinguishable from
/// one it does. The text is the file's bytes and nothing else; splitting it
/// into lines is the caller's too.
///
/// # The cut never splits a character
///
/// The cap counts bytes, so it can land inside a multi-byte character. That
/// character is dropped rather than replaced: the text ends at the last
/// character boundary at or before the cap, so it is a prefix of the file in
/// characters as well as in bytes, at most three bytes shorter than the cap,
/// and no `U+FFFD` is ever manufactured.
///
/// ```
/// use std::fs;
/// use warlock_engine::{Viewed, view_file};
///
/// let dir = tempfile::tempdir()?;
/// let path = dir.path().join("WARLOCK.md");
/// fs::write(&path, "# engine\n\nThe core.\n")?;
///
/// let Viewed { text, cut } = view_file(&path)?;
/// assert_eq!(text, "# engine\n\nThe core.\n");
/// assert!(!cut);
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
///
/// # Errors
///
/// Two answers, because they call for different words:
///
/// * [`Unviewable::Unreadable`] if the file cannot be read — it is not there,
///   it is a directory, the permissions say no, or it vanished since something
///   listed it.
/// * [`Unviewable::NotText`] if the bytes read are not UTF-8. There is no text
///   in this case and none is invented: no replacement characters, and not even
///   the part that did decode, because half a binary shown as text is a worse
///   answer than none. A file whose bytes stop being text only past the cap is
///   not this case — what was read is text, and it is all that was ever going
///   to be shown.
pub fn view_file(path: impl AsRef<Path>) -> Result<Viewed, Unviewable> {
    let path = path.as_ref();
    let mut bytes = read_capped(path).map_err(|source| Unviewable::Unreadable {
        path: path.to_path_buf(),
        source,
    })?;

    // The read stops one byte past the cap, so one byte over is the whole of
    // "there is more to this file": dropping it leaves exactly the first
    // `PER_FILE_BYTE_CAP` bytes, with no cast from the cap's `u64` to an index.
    let cut = byte_count(bytes.len()) > PER_FILE_BYTE_CAP;
    if cut {
        bytes.truncate(bytes.len() - 1);
    }

    let text = match str::from_utf8(&bytes) {
        Ok(text) => text,
        // A cut inside a character is the cap's doing, not the file's, so it
        // costs that one character and nothing else. Everything before
        // `valid_up_to` was just checked, which is why the second look cannot
        // fail; the floor is unreachable rather than a fallback.
        Err(source) if cut && source.error_len().is_none() => {
            str::from_utf8(&bytes[..source.valid_up_to()]).unwrap_or_default()
        }
        Err(source) => {
            return Err(Unviewable::NotText {
                path: path.to_path_buf(),
                source,
            });
        }
    }
    .to_owned();

    Ok(Viewed { text, cut })
}

/// The first [`PER_FILE_BYTE_CAP`] bytes of `path`, plus one more if there is
/// one.
///
/// The extra byte is how the caller knows the file went on, and reading through
/// [`std::io::Read::take`] is how a four-megabyte lockfile is never pulled into memory
/// to have all but the first 128 KiB of it thrown away.
fn read_capped(path: &Path) -> std::io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    fs::File::open(path)?
        .take(PER_FILE_BYTE_CAP + 1)
        .read_to_end(&mut bytes)?;
    Ok(bytes)
}

// The summary cache: five small functions over `<root>/.warlock/summaries/`,
// and nothing else. A key from a file's bytes, the two names that key becomes
// on disk, a read that answers `None` to everything that is not a good entry,
// and a write that can be ignored. None of them can fail a pact: there is no
// `Error` variant and no `Omission` for a cache, because every way one of these
// can go wrong is already the ordinary path — summarise the file.
//
// [`summarise_over_cap`](crate::fitting::summarise_over_cap) is their only caller: it asks for a key over the bytes
// it has just read, looks the entry up, and writes one back when a map-reduce
// actually produced an account.

/// Where a subtree pact has got to, and whether it should carry on: the port a
/// front end shows progress through and cancels through.
///
/// [`pact_subtree`] calls [`starting`](Observer::starting) once per directory,
/// just before that directory's pass, in the order the pact reaches them —
/// children before parents. The engine asks; what is done with the answer is
/// entirely the caller's: draw a line, send it down a channel, count it, ignore
/// it.
///
/// Inside a directory it also calls [`rejected`](Observer::rejected), once for
/// every answer the document schema turned down and asked again for. That one
/// is an announcement rather than a question: it answers nothing, and it has a
/// default body that does nothing, so an observer only interested in
/// directories implements [`starting`](Observer::starting) and stops there.
///
/// There is no longer a call per file: reducing a file too big to send is a
/// table lookup rather than the map-reduce of model passes it once was, so the
/// minutes of apparent silence that hook existed to fill are not spent any
/// more.
///
/// Between those two it calls [`requesting`](Observer::requesting), once per
/// directory, at the moment that directory's request is handed to the
/// [`Agent`]: how many files went into it and how many bytes that is. The
/// silence a reader is looking at between then and the pass coming back is the
/// pass itself, and these are the two numbers that say why it is as long as it
/// is. An announcement with the same do-nothing default as `summarising`.
///
/// And when a directory comes out of its pass with its document written — and
/// every directory beneath it already has one — it calls
/// [`documented`](Observer::documented), so a front end can mark work done as
/// it is done rather than when the whole run is. An announcement like
/// `summarising`, with the same do-nothing default.
///
/// # What this trait is careful not to require
///
/// **No [`Send`], no [`Sync`], no `'static`.** The engine does not decide which
/// thread a pact runs on, so it asks for nothing that would decide it. A front
/// end that runs the pact on a worker thread already owns that choice and can
/// give its observer whatever bounds *it* needs; one that pacts on the thread it
/// is already on can hand over a plain `&mut` to something on its own stack.
///
/// **Nothing about the pass in flight.** Cancellation is a question asked
/// between directories, so [`Pacting::Stop`] never interrupts a model pass that
/// is already running — killing a subprocess is the business of whoever spawned
/// it, which by the [`Agent`] seam is never this crate. Answering `Stop` while a
/// pass runs means the pact ends when that pass comes back.
///
/// ```
/// use std::path::{Path, PathBuf};
/// use warlock_engine::{pact, Pacting};
///
/// /// Remembers where the pact got to, and gives up after two directories.
/// struct Impatient(Vec<PathBuf>);
///
/// impl pact::Observer for Impatient {
///     fn starting(&mut self, directory: &Path, position: usize, total: usize) -> Pacting {
///         assert!((1..=total).contains(&position), "1-based, and inside the total");
///         self.0.push(directory.to_path_buf());
///         if position > 2 { Pacting::Stop } else { Pacting::Continue }
///     }
/// }
/// ```
pub trait Observer {
    /// `directory` is about to be pacted: it is number `position` of `total`,
    /// counting from one.
    ///
    /// `total` is every directory the pact covers and is the same on every call
    /// of one pact, so `position` of `total` is a fraction that only goes
    /// forwards. The answer decides whether `directory` is pacted at all:
    /// [`Pacting::Continue`] runs its pass, [`Pacting::Stop`] ends the pact
    /// before it, leaving `directory` and everything after it undocumented.
    fn starting(&mut self, directory: &Path, position: usize, total: usize) -> Pacting;

    /// This directory's request is going to the [`Agent`] now: `files` files,
    /// `bytes` bytes of them.
    ///
    /// Called once per directory, immediately before the pass that writes the
    /// document, so what follows it is the wait for that pass and nothing else.
    /// A front end with a line for that wait can say what is being waited on
    /// instead of saying only that something is: a directory that is slow is
    /// usually slow because of these two numbers.
    ///
    /// # What the two numbers count
    ///
    /// `files` is how many files the request carries, each one a name with
    /// either its text, an account of it written by a summarising pass, or
    /// neither. `bytes` is everything the request carries counted the way the
    /// budget counts it — the files, plus every child directory's document —
    /// so the two do not cover quite the same set, and `bytes` is the number
    /// the caps are checked against. Both are read off the request as it
    /// stands; nothing is measured for this call that was not measured already.
    ///
    /// # Why this is not part of [`starting`](Observer::starting)
    ///
    /// Because neither number is true yet when `starting` is called. That is
    /// asked before the directory is read at all, and what the request holds is
    /// settled only after gathering, after the summarising passes over files too
    /// big to send, and after the demotions that bring the whole request under
    /// its cap. Carrying the counts on `starting` would mean asking about
    /// cancelling after all of that work, and answering [`Pacting::Stop`] would
    /// then cost a directory's worth of passes to act on — the property worth
    /// keeping is that a cancel costs nothing.
    ///
    /// # Nothing is asked
    ///
    /// An announcement, like [`rejected`](Observer::rejected) and for the
    /// same reason: the pass it is about is handed over in the next breath, and
    /// no answer here could be acted on before it comes back. The default body
    /// does nothing, so an observer that does not care what a request weighs
    /// writes none of this.
    fn requesting(&mut self, files: usize, bytes: u64) {
        let _ = (files, bytes);
    }

    /// `directory`'s pass answered, and the answer was turned down: attempt
    /// `attempt` of `attempts`, for the `defects` listed.
    ///
    /// Called immediately after [`document::accept`] refuses an answer and
    /// before anything is done about it. When `attempt` is under `attempts`
    /// another pass follows, with these defects listed at the top of its
    /// request, and [`requesting`](Observer::requesting) is announced again
    /// for it; when the two are equal the directory is about to fail with
    /// [`Refusal::Malformed`] carrying the same list. A front end with a line
    /// for it can say why a directory is taking two passes instead of one, and
    /// what the model got wrong.
    ///
    /// An announcement, not a question, with a default body that does nothing
    /// — exactly as [`documented`](Observer::documented) is, and for the
    /// same reason.
    fn rejected(&mut self, directory: &Path, defects: &[Defect], attempt: usize, attempts: usize) {
        let _ = (directory, defects, attempt, attempts);
    }

    /// `directory`'s pass has written its document, and so has every pass under
    /// it: nothing phase one can still do will take this directory's grant away.
    ///
    /// The announcement a front end colours a finished directory with, made the
    /// moment it becomes true instead of at the end of the run. It is
    /// deliberately *not* "the pass finished" — a directory whose pass wrote a
    /// document above a descendant that failed is going to be recorded without
    /// a grant, and announcing it as done would be announcing a colour the
    /// manifest is about to contradict. The pact works children before parents,
    /// so by the time a directory's own pass is over the question has an
    /// answer, and a directory this is never called for is one that failed or
    /// sits above one that did.
    ///
    /// Still short of a promise: granting happens in phase two, where a hash
    /// that cannot be read leaves the entry ungranted — see [`Failure::Hash`].
    /// What a front end paints on this is a preview, and the manifest the pact
    /// hands back is the record.
    ///
    /// An announcement, not a question, with a default body that does nothing —
    /// exactly as [`rejected`](Observer::rejected) is, and for the same
    /// reason.
    fn documented(&mut self, directory: &Path) {
        let _ = directory;
    }

    /// `directory` needed no pass: what one would have been shown, and the
    /// document it would have been judged against, are exactly what they were
    /// when the grant now being carried was earned.
    ///
    /// Sent *in place of* [`documented`](Observer::documented) and never beside
    /// it, so a front end hears one word per directory and that word is a true
    /// one. Everything `documented` means still holds here — this directory and
    /// everything below it has a current document, and phase two is about to
    /// grant it — so a reader that paints a row green on one should paint it
    /// green on the other. The only difference is what it says happened, and
    /// saying *wrote* would name a write that did not occur.
    ///
    /// An announcement, not a question, with a default body that does nothing,
    /// so an [`Observer`] written before the cutoff existed keeps compiling and
    /// simply hears nothing about a saving it never asked about.
    fn unchanged(&mut self, directory: &Path) {
        let _ = directory;
    }
}

/// What an [`Observer`] says about the directory it was just offered: pact it,
/// or stop here.
///
/// Two variants and a name for each, rather than a `bool`: a call site reading
/// `Pacting::Stop` needs nothing explained to it, and `false` at the end of a
/// progress callback could as easily mean "nothing to report".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pacting {
    /// Pact this directory, and go on to ask about the next one.
    Continue,
    /// Stop: leave this directory unpacted, pact nothing after it, and finish
    /// the pact with the documents already written. Not a failure — see
    /// [`pact_subtree`] for what the manifest then says.
    Stop,
}

/// The [`Observer`] for a caller that has nothing to show and nothing to
/// cancel: it watches every directory go past and always answers
/// [`Pacting::Continue`].
///
/// A pact through `&mut Unwatched` is the pact this function had before there
/// was an observer at all, which is what makes it worth a name: an example, a
/// test or a script says `&mut Unwatched` and the reader can stop thinking about
/// progress there.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Unwatched;

impl Observer for Unwatched {
    /// Pact every directory, and say nothing about any of them.
    fn starting(&mut self, _directory: &Path, _position: usize, _total: usize) -> Pacting {
        Pacting::Continue
    }
}

/// What a subtree pact produced: the manifest to save, and everything that went
/// wrong on the way without stopping it.
///
/// The manifest is a value, not a file — [`pact_subtree`] writes nothing to
/// `.warlock/` — and the two lists are what a front end reports. They are
/// separate because they call for different reactions: a [`Failure`] is a
/// directory that did not come out of the pact the way it was meant to, while a
/// [`Problem`] is a file left out of a request that succeeded anyway.
#[derive(Debug)]
pub struct PactedSubtree {
    /// The manifest as it should now be written: the pact's entries, plus every
    /// entry from outside the pacted subtree, unchanged. Save it once, with
    /// [`Manifest::save`].
    pub manifest: Manifest,
    /// Every directory that failed: the ones with no document first, in the
    /// order the pact reached them, then the ones that could not be recorded or
    /// hashed, since that is a later phase. Within a phase it is children
    /// before parents, like the pact itself. Empty is the whole-subtree
    /// success, and a non-empty list still comes with a manifest worth saving.
    pub failures: Vec<Failure>,
    /// Every file the byte caps left out of a request, gathered from each
    /// directory's pact as it happened. Nothing here means anything failed.
    pub problems: Vec<Problem>,
}

/// What a pact produced: the document it wrote, and everything the byte caps
/// left out of the request behind it.
///
/// A plain pair like [`Gathered`](crate::fitting::Gathered), for the same reason: the document is the
/// thing that happened, the problems are the thing to report once. Reaching
/// this type at all means a document was written — there is no "pacted but not
/// written" case, because every way of not writing one is an [`Error`].
#[derive(Debug)]
pub struct Pacted {
    /// The document that was written: `<directory>/WARLOCK.md`. Given back
    /// rather than left to be recomputed, because a caller recording a pact
    /// needs exactly this path and should not have to know the file name to
    /// build it.
    pub document: PathBuf,
    /// Every file whose contents the pass never saw: what [`gather_request`](crate::fitting::gather_request)
    /// left out, less the over-cap files that were then described, plus the
    /// reason for each one that could not be. Empty is the normal
    /// case, and a non-empty list never means the document is worse — only that
    /// it was written about slightly less than the whole directory.
    pub problems: Vec<Problem>,
}

/// What [`view_file`] read: the file's text, and whether the cap cut it short.
///
/// A plain pair like [`Gathered`](crate::fitting::Gathered), and for the same reason: the text is the
/// thing to show, the cut is the thing to say once. `cut` is deliberately a
/// fact rather than a sentence — see [`view_file`] for why the words belong to
/// whatever is drawing the text — and `text` is the file's own bytes with
/// nothing added to them, so `cut` is the only place the cap is admitted to.
#[derive(Debug)]
pub struct Viewed {
    /// The file's text, from its first byte, at most [`PER_FILE_BYTE_CAP`]
    /// bytes of it and never a byte that is not the file's own. Unsplit: a
    /// caller wanting lines makes them.
    pub text: String,
    /// Whether the file goes on past what `text` holds, because it is larger
    /// than [`PER_FILE_BYTE_CAP`]. `false` means `text` is the whole file.
    pub cut: bool,
}

/// Why there is no text to show for a file.
///
/// Two cases and no more, because the two call for different words in front of
/// a person: one is the filesystem saying no, which is a thing to go and look
/// at, and the other is a file doing nothing wrong at all — a PNG, a compiled
/// artefact — that simply has no text in it to show. Neither is fatal to
/// anything: a caller that shows a line about it and carries on has responded
/// fully.
///
/// Both variants name the file, reachable uniformly through
/// [`Unviewable::path`], so a caller can word the failure without matching on
/// the variant to find out which file it is about.
///
/// Not an [`Omission`](crate::Omission): that type is about what a *request* did not carry and
/// its causes include the byte caps, while nothing here is a cap — a file cut
/// at [`PER_FILE_BYTE_CAP`] is a [`Viewed`] with `cut` set, not an error.
#[derive(Debug)]
#[non_exhaustive]
pub enum Unviewable {
    /// The file could not be read: it is not there, it is a directory, the
    /// permissions say no, or it vanished since something listed it.
    Unreadable {
        /// The file that could not be read.
        path: PathBuf,
        /// What the filesystem said.
        source: std::io::Error,
    },
    /// The bytes read are not valid UTF-8, so there is no text to show and none
    /// is invented.
    ///
    /// Separate from [`Unviewable::Unreadable`] because nothing is wrong: the
    /// read worked, and what came back is a file that is not text. The same
    /// judgement [`Omission::NotText`](crate::Omission::NotText) makes about summarising, made for the
    /// same reason.
    NotText {
        /// The file that is not text.
        path: PathBuf,
        /// Where the bytes stopped being text, as [`std::str::from_utf8`]
        /// reported it.
        source: Utf8Error,
    },
}

impl Unviewable {
    /// The file this failure is about, whichever way it failed.
    #[must_use]
    pub fn path(&self) -> &Path {
        match self {
            Self::Unreadable { path, .. } | Self::NotText { path, .. } => path,
        }
    }
}

impl fmt::Display for Unviewable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unreadable { path, source } => {
                write!(f, "could not read `{}`: {source}", path.display())
            }
            Self::NotText { path, source } => write!(
                f,
                "`{}` is not text ({source}), so there is nothing to show",
                path.display()
            ),
        }
    }
}

impl std::error::Error for Unviewable {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Unreadable { source, .. } => Some(source),
            Self::NotText { source, .. } => Some(source),
        }
    }
}

/// Why a pass produced no document, when it was not the filesystem's fault.
///
/// Two cases and no more, which is the whole rejection policy: the pass did not
/// come back with an answer, or what it came back with did not fit the object
/// it was asked to fill — in any of [`ATTEMPTS`] passes, the later ones shown
/// the earlier ones' defects. Every check behind the second case is a check on
/// *shape*, made in [`document`]; nothing here reads what a line says.
///
/// Separate from [`Error`] because a caller may well want to treat these
/// differently from a walk that failed: a refusal is worth retrying, and a
/// directory that cannot be listed is not.
#[derive(Debug)]
#[non_exhaustive]
pub enum Refusal {
    /// The agent came back with an error instead of an answer: no `claude` on
    /// `PATH`, a non-zero exit, empty output, a timeout, or any other way the
    /// transport reported not reaching a model.
    Agent {
        /// What the agent said, in the engine's vocabulary rather than the
        /// transport's.
        source: agent::Error,
    },
    /// Every pass answered, and no answer fitted the object it was asked to
    /// fill. An empty or whitespace-only answer is this case — it is not an
    /// object — rather than a variant of its own.
    Malformed {
        /// What was wrong with the last answer, in the order the checks found
        /// it. Never empty.
        defects: Vec<Defect>,
        /// How many passes were spent before giving up: [`ATTEMPTS`].
        attempts: usize,
    },
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Agent { source } => write!(f, "the model pass produced no answer: {source}"),
            Self::Malformed { defects, attempts } => {
                // One line, and a short one: the first few defects and a count
                // of the rest, because this is what a footer shows and a list
                // of thirty missing entries is not a footer line.
                const SHOWN: usize = 3;
                write!(
                    f,
                    "no answer fitted the document's shape in {attempts} passes: "
                )?;
                for (index, defect) in defects.iter().take(SHOWN).enumerate() {
                    if index > 0 {
                        f.write_str("; ")?;
                    }
                    write!(f, "{defect}")?;
                }
                if defects.len() > SHOWN {
                    write!(f, "; and {} more", defects.len() - SHOWN)?;
                }
                Ok(())
            }
        }
    }
}

impl std::error::Error for Refusal {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Agent { source } => Some(source),
            Self::Malformed { .. } => None,
        }
    }
}

/// Everything that can stop a directory getting a document.
///
/// Hand-rolled like every other error in this crate, and deliberately short:
/// neither byte cap is in here, because neither cap can fail a pact.
///
/// Every variant carries the directory it is about, reachable uniformly through
/// [`Error::directory`]. A caller pacting a subtree collects a pile of these
/// and has to be able to say which directory each one belongs to without
/// matching on the variant to find out.
#[derive(Debug)]
#[non_exhaustive]
pub enum Error {
    /// The directory could not be walked: it is not there, it cannot be
    /// listed, or something vanished from under the walk.
    Walk {
        /// The directory that was being pacted.
        directory: PathBuf,
        /// What the walker said, including which path it was on.
        source: ignore::Error,
    },
    /// A file's path has no relative, forward-slash, UTF-8 form, so it cannot
    /// be named to a model.
    Path {
        /// The directory that was being pacted.
        directory: PathBuf,
        /// The path that could not be named.
        path: PathBuf,
        /// Why it could not be. Boxed for the same reason as
        /// [`hash::Error::Path`]: a manifest error
        /// carries a parser error inside it, and the other variant here is an
        /// `ignore::Error`.
        source: Box<manifest::Error>,
    },
    /// The pass ran and produced nothing worth writing. **Nothing was
    /// written**: whatever was in the directory before is exactly what is in it
    /// now.
    Refused {
        /// The directory that was being pacted.
        directory: PathBuf,
        /// Which of the two refusals it was: no answer, or no answer that
        /// fitted.
        cause: Refusal,
    },
    /// The answer was good and the document could not be written anyway.
    ///
    /// Its own variant rather than a [`Refusal`], because it is a different
    /// failure with a different answer: nothing is wrong with the model, the
    /// disk is full or the directory is read-only, and a caller retrying the
    /// pass is retrying the expensive half of something that already worked.
    ///
    /// The write is atomic, so this is also the variant that says the document
    /// on disk is untouched: whatever `WARLOCK.md` held before the pass, it
    /// still holds.
    Write {
        /// The directory that was being pacted.
        directory: PathBuf,
        /// The document that could not be written: `<directory>/WARLOCK.md`,
        /// never the temporary the write went through — that is a mechanism,
        /// not something a user asked for or can act on.
        path: PathBuf,
        /// What the filesystem said.
        source: std::io::Error,
    },
}

impl Error {
    /// The directory this failure is about, whichever way it failed.
    #[must_use]
    pub fn directory(&self) -> &Path {
        match self {
            Self::Walk { directory, .. }
            | Self::Path { directory, .. }
            | Self::Refused { directory, .. }
            | Self::Write { directory, .. } => directory,
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Walk { directory, source } => write!(
                f,
                "could not walk `{}` to pact it: {source}",
                directory.display()
            ),
            Self::Path {
                directory,
                path,
                source,
            } => write!(
                f,
                "could not name `{}` relative to `{}`, the directory being pacted: {source}",
                path.display(),
                directory.display(),
            ),
            Self::Refused { directory, cause } => write!(
                f,
                "nothing was written for `{}`: {cause}",
                directory.display()
            ),
            Self::Write {
                directory,
                path,
                source,
            } => write!(
                f,
                "the pass over `{}` produced a document but `{}` could not be written: {source}",
                directory.display(),
                path.display(),
            ),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Walk { source, .. } => Some(source),
            Self::Path { source, .. } => Some(source.as_ref()),
            Self::Refused { cause, .. } => Some(cause),
            Self::Write { source, .. } => Some(source),
        }
    }
}

/// One directory a subtree pact did not finish with, and how far it got.
///
/// Three cases because there are three answers, and a caller showing a user one
/// line per failure should not have to flatten them into "something went
/// wrong": there is no document, there is a document nobody can record, and
/// there is a document with no hash to grant it against. Every variant names
/// its directory, uniformly through [`Failure::directory`].
///
/// A failure is never the end of a pact. Each one is about one directory, the
/// rest of the subtree carries on, and the manifest that comes back alongside
/// them is a manifest worth saving — see [`pact_subtree`] for what each case
/// leaves in it.
#[derive(Debug)]
#[non_exhaustive]
pub enum Failure {
    /// No document was written for this directory, so it gets no entry at all.
    /// The ordinary case: the pass was refused, or the disk said no.
    Document {
        /// What stopped it, naming the directory itself.
        source: Error,
    },
    /// The document was written and the entry could not be built: the directory
    /// does not sit under the manifest's root, or its path is not UTF-8, so
    /// there is no way to spell it in a TOML file.
    Record {
        /// The directory that was pacted.
        directory: PathBuf,
        /// Why it cannot be named in the manifest.
        source: manifest::Error,
    },
    /// The document was written and the directory could not be hashed, so its
    /// entry goes in without a grant.
    ///
    /// Its own case rather than a [`Failure::Document`], because what happened
    /// is the opposite: the expensive half worked and the cheap half did not.
    /// The document on disk is real, the entry is real, and only the grant —
    /// the one thing that must never be invented — is missing.
    Hash {
        /// The directory that was pacted.
        directory: PathBuf,
        /// Why it has no hash: a file under it could not be read, or named, or
        /// the walk itself failed.
        source: hash::Error,
    },
}

impl Failure {
    /// The directory this failure is about, whichever way it failed.
    #[must_use]
    pub fn directory(&self) -> &Path {
        match self {
            Self::Document { source } => source.directory(),
            Self::Record { directory, .. } | Self::Hash { directory, .. } => directory,
        }
    }
}

impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            // Delegated: the error already reads as a sentence about its own
            // directory, and wrapping it would say the directory twice.
            Self::Document { source } => write!(f, "{source}"),
            Self::Record { directory, source } => write!(
                f,
                "`{}` was documented but cannot be recorded in the manifest: {source}",
                directory.display()
            ),
            Self::Hash { directory, source } => write!(
                f,
                "`{}` was documented but could not be hashed, so it is pacted without a grant: \
                 {source}",
                directory.display()
            ),
        }
    }
}

impl std::error::Error for Failure {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Document { source } => Some(source),
            Self::Record { source, .. } => Some(source),
            Self::Hash { source, .. } => Some(source),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::error::Error as _;
    use std::fs;
    use std::path::{Path, PathBuf};

    use super::{
        DOCUMENT_FILE, Failure, Observer, Pacted, PactedSubtree, Pacting, Refusal, Unviewable,
        Unwatched, Viewed, closed_scopes_at_or_below, pact_directory, pact_subtree,
        pactable_directories, refresh_subtree, unpact_subtree, view_file,
    };
    use crate::document::{self, STAMP};
    use crate::fitting::{
        Gathered, Omission, PER_FILE_BYTE_CAP, REQUEST_BYTE_CAP, byte_count, gather_request,
    };
    use crate::{
        Agent, Loaded, Manifest, NodeState, PactEntry, agent, decide_state, from_manifest_path,
        load_tree, manifest, subtree_hash,
    };

    /// The whole point of the agent seam, in one struct: a model pass that
    /// answers with canned markdown and keeps what it was asked. No `claude`,
    /// no network, no terminal, no mocking framework.
    struct Canned {
        /// What every pass answers: a fixed string, or the object the pass was
        /// asked to fill.
        text: Option<String>,
        /// Every request that reached it, in call order.
        seen: std::cell::RefCell<Vec<agent::Request>>,
    }

    impl Canned {
        /// A fake answering `text` to anything — how a test hands the engine
        /// an answer it should turn down.
        fn new(text: impl Into<String>) -> Self {
            Self {
                text: Some(text.into()),
                seen: std::cell::RefCell::new(Vec::new()),
            }
        }

        /// A fake whose every answer the engine accepts: the slots the request
        /// defines, filled with stand-in lines.
        fn filling() -> Self {
            Self {
                text: None,
                seen: std::cell::RefCell::new(Vec::new()),
            }
        }
    }

    impl Agent for Canned {
        fn run(&self, request: &agent::Request) -> Result<agent::Response, agent::Error> {
            self.seen.borrow_mut().push(request.clone());
            let text = self
                .text
                .clone()
                .unwrap_or_else(|| crate::document::stub_answer(request));
            Ok(agent::Response::new(text))
        }
    }

    /// The other half of a fake: one that never comes back with an answer. The
    /// failure is a function rather than a field because [`agent::Error`](crate::agent::Error) is not
    /// [`Clone`], and a test that wants a particular one should be able to say
    /// so at the call site.
    struct Fails(fn() -> agent::Error);

    impl Agent for Fails {
        fn run(&self, _request: &agent::Request) -> Result<agent::Response, agent::Error> {
            Err(self.0())
        }
    }

    /// A plausible document of exactly `bytes` bytes, with no whitespace at
    /// either end so its trimmed length is its length.
    fn document(bytes: usize) -> String {
        let head = "# engine\n\nCore engine for warlock. ";
        assert!(bytes > head.len(), "a document has room for its heading");
        format!("{head}{}", "x".repeat(bytes - head.len()))
    }

    /// What is in `dir`'s `WARLOCK.md`, or `None` if it has none.
    fn written(dir: &Path) -> Option<Vec<u8>> {
        fs::read(dir.join("WARLOCK.md")).ok()
    }

    /// Write `contents` at `dir/name`, creating whatever directories it needs.
    fn write(dir: &Path, name: &str, contents: impl AsRef<[u8]>) -> PathBuf {
        let path = dir.join(name);
        fs::create_dir_all(path.parent().expect("a file has a parent")).expect("creates parents");
        fs::write(&path, contents).expect("writes a file");
        path
    }

    /// `size` bytes of something, cheap to make and impossible to confuse with
    /// a fixture's real text.
    fn filler(size: u64) -> Vec<u8> {
        vec![b'x'; usize::try_from(size).expect("a test file fits in memory")]
    }

    /// `percent` of [`REQUEST_BYTE_CAP`](crate::fitting::REQUEST_BYTE_CAP), in bytes.
    ///
    /// Every fixture below that is about the budget biting is written in these
    /// rather than in kibibytes, because what those tests are about is a size
    /// *relative to the cap* — three files at 39% apiece are over budget and two
    /// are not, whatever the cap happens to be this year. They used to be
    /// absolute, against a 256 KiB cap, and raising that cap turned a directory
    /// that was deliberately over budget into one that fit, which is a fixture
    /// silently ceasing to test what it was written for rather than a test
    /// failing honestly.
    fn share(percent: u64) -> u64 {
        REQUEST_BYTE_CAP * percent / 100
    }

    /// The same share, as the `usize` a document's length is measured in.
    fn share_bytes(percent: u64) -> usize {
        usize::try_from(share(percent)).expect("a share of the cap fits in memory")
    }

    /// `size` bytes that are not text: what a checked-in PNG, a compiled
    /// artefact or a fixture of random bytes looks like to the chunker.
    ///
    /// One byte does it, and it goes at the end so that a file which is text
    /// almost all the way through is still not text — the same rule the
    /// chunker applies to the whole of a file rather than to its beginning.
    fn not_text(size: u64) -> Vec<u8> {
        let mut bytes = filler(size);
        *bytes.last_mut().expect("a fixture has bytes") = 0xff;
        bytes
    }

    /// A request budget nothing in a fixture can reach, so that the per-file
    /// cap is the only thing able to leave anything out.
    ///
    /// Four times [`PER_FILE_BYTE_CAP`] rather than a number typed in: what
    /// these fixtures probe is the per-file boundary, and a request budget that
    /// could also demote a file would make a failure here ambiguous about which
    /// cap caused it. Derived from the other cap so it cannot drift under it.
    const AMPLE_CAP: u64 = PER_FILE_BYTE_CAP * 4;

    /// The request for `dir`, insisting nothing was left out of it.
    ///
    /// Most of these fixtures are small enough to send whole, so an empty
    /// problem list is part of what they assert: a gather that quietly started
    /// dropping files would fail here rather than pass unnoticed. It gathers
    /// against [`AMPLE_CAP`] for that reason — the only cap allowed to drop
    /// anything here is the per-file one.
    fn request_for(dir: &Path) -> agent::Request {
        let Gathered { request, problems } =
            gather_request("summarise", dir, AMPLE_CAP).expect("gathers");
        assert!(problems.is_empty(), "{problems:?}");
        request
    }

    /// Whether a request is a directory's own pass — the one whose answer is
    /// checked against the document's shape.
    fn is_document_pass(request: &agent::Request) -> bool {
        request.prompt().starts_with(document::PROMPT)
    }

    /// The last request a fake was asked to run: the directory's own pass.
    fn pass(seen: &[agent::Request]) -> &agent::Request {
        let request = seen.last().expect("the directory was pacted");
        assert!(
            is_document_pass(request),
            "the last pass of a pact is the pact"
        );
        request
    }

    /// The modules a manifest holds, in file order.
    fn modules(manifest: &Manifest) -> Vec<&str> {
        manifest.entries().iter().map(PactEntry::module).collect()
    }

    /// Every entry's module and the scope written on it, in file order: a
    /// manifest's boundaries in one comparable value.
    fn scopes(manifest: &Manifest) -> Vec<(&str, Option<&str>)> {
        manifest
            .entries()
            .iter()
            .map(|entry| (entry.module(), entry.scope()))
            .collect()
    }

    /// `manifest` with a scope written on each named module, the way a person
    /// would: through the entry, which is a scope's only home.
    ///
    /// Every name must already be pacted, because there is deliberately no way
    /// to scope a module with no entry — a typo here fails the fixture rather
    /// than quietly testing a manifest with no scopes in it.
    fn with_scopes(manifest: &Manifest, scoped: &[(&str, &str)]) -> Manifest {
        for (module, _) in scoped {
            assert!(
                manifest.entry(module).is_some(),
                "`{module}` is not pacted, so nothing can scope it",
            );
        }
        Manifest::with_entries(manifest.entries().iter().map(|entry| {
            match scoped.iter().find(|(module, _)| *module == entry.module()) {
                Some((_, scope)) => entry.clone().with_scope(*scope),
                None => entry.clone(),
            }
        }))
    }

    /// The paths of a request's files, in the order it carries them.
    fn file_paths(request: &agent::Request) -> Vec<&str> {
        request.files().iter().map(agent::File::path).collect()
    }

    /// The file a request carries at `path`.
    fn file<'a>(request: &'a agent::Request, path: &str) -> &'a agent::File {
        request
            .files()
            .iter()
            .find(|file| file.path() == path)
            .unwrap_or_else(|| panic!("`{path}` is in the request: {:?}", file_paths(request)))
    }

    /// How many bytes a request actually carries: the files sent whole, the
    /// summaries of the files described, and the children's documents.
    ///
    /// Written from the public accessors rather than by calling
    /// [`super::carried_bytes`], so the module's own accounting has something
    /// independent to agree with. A listed file contributes nothing — its size
    /// is a fact about the directory, not bytes in the request.
    fn carried(request: &agent::Request) -> u64 {
        let files: u64 = request
            .files()
            .iter()
            .map(|file| {
                let bytes = file.bytes().map_or(0, <[u8]>::len);
                let summary = file.summary().map_or(0, str::len);
                (bytes + summary) as u64
            })
            .sum();
        let children: u64 = request
            .child_documents()
            .iter()
            .map(|child| child.text().len() as u64)
            .sum();
        files + children
    }

    /// The counting fake: a model pass that answers from a script, keeps every
    /// request it was handed, and can therefore be asked afterwards how many
    /// passes a file cost and what each one was told. Hand-written like every
    /// other fake in this crate — no `claude`, no network, no terminal, no
    /// mocking framework.
    struct Counting {
        /// What the first passes answer, in call order: the text of an answer,
        /// or a function making the [`agent::Error`](crate::agent::Error) the pass fails with. A
        /// function because `agent::Error` is not [`Clone`], the same trick
        /// [`Fails`] uses.
        script: Vec<Result<String, fn() -> agent::Error>>,
        /// What every pass past the end of the script answers. A test that
        /// expects no such pass proves it by counting, not by panicking here:
        /// "it ran four passes when it should have run two" is a better
        /// failure than a panic from inside a fake.
        beyond: String,
        /// Every request that reached it, whole and in call order.
        seen: std::cell::RefCell<Vec<agent::Request>>,
    }

    impl Counting {
        /// A fake answering `beyond` to every pass it is asked for.
        fn new(beyond: impl Into<String>) -> Self {
            Self {
                script: Vec::new(),
                beyond: beyond.into(),
                seen: std::cell::RefCell::new(Vec::new()),
            }
        }

        /// The same fake, with its first passes answered by `script`.
        fn scripted(
            mut self,
            script: impl IntoIterator<Item = Result<String, fn() -> agent::Error>>,
        ) -> Self {
            self.script = script.into_iter().collect();
            self
        }

        /// How many passes it was asked for.
        fn passes(&self) -> usize {
            self.seen.borrow().len()
        }

        /// The prompt of every pass, in call order.
        fn prompts(&self) -> Vec<String> {
            self.seen
                .borrow()
                .iter()
                .map(|request| request.prompt().to_owned())
                .collect()
        }
    }

    impl Agent for Counting {
        fn run(&self, request: &agent::Request) -> Result<agent::Response, agent::Error> {
            let index = self.passes();
            self.seen.borrow_mut().push(request.clone());
            match self.script.get(index) {
                Some(Err(fail)) => Err(fail()),
                _ if is_document_pass(request) => {
                    Ok(agent::Response::new(document::stub_answer(request)))
                }
                Some(Ok(text)) => Ok(agent::Response::new(text.clone())),
                None => Ok(agent::Response::new(self.beyond.clone())),
            }
        }
    }

    #[test]
    fn a_pact_leaves_nothing_behind_but_the_document() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        write(dir.path(), "lib.rs", "//! Core engine.\n");

        pact_directory(dir.path(), &Canned::filling()).expect("pacts");

        let mut left = fs::read_dir(dir.path())
            .expect("lists")
            .map(|entry| entry.expect("an entry").file_name())
            .collect::<Vec<_>>();
        left.sort();
        assert_eq!(
            left,
            ["WARLOCK.md", "lib.rs"],
            "no temporary file leaks into the directory the pact just described",
        );
    }

    /// Only on unix, because there is no portable way to make a directory
    /// unwritable. What is under test — that a document the filesystem refuses
    /// is [`Error::Write`], naming the document — is not platform-specific.
    #[cfg(unix)]
    #[test]
    fn a_document_that_cannot_be_written_names_itself_and_leaves_the_old_one() {
        use std::os::unix::fs::PermissionsExt as _;

        let dir = tempfile::tempdir().expect("a temporary directory");
        let before = "# engine\n\nWhat it used to say.\n";
        write(dir.path(), DOCUMENT_FILE, before);
        // Readable and listable, so the gather still works, but nothing new can
        // be created in it — neither the temporary nor a rename over the
        // document.
        fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o555)).expect("chmods");
        if fs::write(dir.path().join("probe"), "").is_ok() {
            // Running as root: no directory is unwritable, so there is nothing
            // here to assert against.
            fs::remove_file(dir.path().join("probe")).expect("removes the probe");
            fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o755))
                .expect("chmods back");
            return;
        }

        let error = pact_directory(dir.path(), &Canned::filling())
            .expect_err("a read-only directory takes no document");

        match &error {
            super::Error::Write { path, .. } => {
                assert_eq!(
                    path,
                    &dir.path().join(DOCUMENT_FILE),
                    "the document, not the temporary"
                );
            }
            other => panic!("expected a write failure, got {other:?}"),
        }
        assert_eq!(error.directory(), dir.path());
        assert_eq!(
            written(dir.path()).as_deref(),
            Some(before.as_bytes()),
            "the write is atomic, so a failure leaves the old document whole",
        );

        fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o755)).expect("chmods back");
        let mut left = fs::read_dir(dir.path())
            .expect("lists")
            .map(|entry| entry.expect("an entry").file_name())
            .collect::<Vec<_>>();
        left.sort();
        assert_eq!(
            left,
            [DOCUMENT_FILE],
            "and no temporary behind on the failure path"
        );
    }

    #[test]
    fn an_agent_that_fails_writes_nothing_and_names_the_directory() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        write(dir.path(), "lib.rs", "//! Core engine.\n");
        let agent = Fails(|| agent::Error::Failed {
            code: Some(2),
            stderr: "Invalid API key\n".to_owned(),
        });

        let error = pact_directory(dir.path(), &agent).expect_err("a failed pass is no document");

        assert!(
            matches!(
                error,
                super::Error::Refused {
                    cause: Refusal::Agent {
                        source: agent::Error::Failed { code: Some(2), .. }
                    },
                    ..
                }
            ),
            "{error:?}",
        );
        assert_eq!(error.directory(), dir.path());
        assert_eq!(
            written(dir.path()),
            None,
            "a directory with no document still has none",
        );
    }

    #[test]
    fn an_empty_or_whitespace_only_answer_is_rejected() {
        for answer in ["", "   \n\t\n   "] {
            let dir = tempfile::tempdir().expect("a temporary directory");

            let error = pact_directory(dir.path(), &Canned::new(answer))
                .expect_err("there is nothing here to write");

            assert!(
                matches!(
                    error,
                    super::Error::Refused {
                        cause: Refusal::Malformed { .. },
                        ..
                    }
                ),
                "whitespace is not a document: {error:?}",
            );
            assert_eq!(written(dir.path()), None);
        }
    }

    #[test]
    fn every_document_opens_by_saying_it_is_a_map_and_not_a_specification() {
        // The misreading this exists to head off: a document treated as the
        // specification of a directory, so that what it does not mention is
        // taken not to exist. Warlock writes the correction rather than asking
        // a pass for it, so it is the same in every document and cannot be
        // reworded, shortened or dropped under a long request.
        let dir = tempfile::tempdir().expect("a temporary directory");
        write(dir.path(), "lib.rs", "//! Core engine.\n");
        let _answer = document(300);

        pact_directory(dir.path(), &Canned::filling()).expect("pacts");

        let written = String::from_utf8(written(dir.path()).expect("a document")).expect("text");
        assert!(written.starts_with(STAMP), "{written}");
        assert!(
            written.contains("check anything you are about to rely on against the files"),
            "the reader is told to verify: {written}"
        );
        assert!(
            written.contains("the code is right"),
            "and told which side wins when it does not match: {written}"
        );
        assert_eq!(
            written.matches("<!-- warlock -->").count(),
            1,
            "and it is there exactly once"
        );
    }

    #[test]
    fn an_over_cap_file_that_is_not_text_stays_a_name_and_a_size_and_costs_no_pass() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let size = PER_FILE_BYTE_CAP + 1;
        let blob = write(dir.path(), "blob.bin", not_text(size));
        let answer = document(300);
        let agent = Counting::new(&answer);

        let Pacted { problems, .. } =
            pact_directory(dir.path(), &agent).expect("an over-cap file never fails a pact");

        assert!(
            written(dir.path()).is_some(),
            "the directory is documented anyway"
        );
        assert_eq!(
            agent.passes(),
            1,
            "the directory pass and nothing else: not one pass is spent on bytes that are \
             not text",
        );
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert_eq!(problems[0].path, blob);
        assert!(
            matches!(problems[0].cause, Omission::NotText { size: reported, .. } if reported == size),
            "the cause is why there is no summary, in place of the cap that listed it: {:?}",
            problems[0],
        );

        // Read off the request the pass actually saw, rather than trusting
        // that gathering did what its own tests say it does.
        let seen = agent.seen.borrow();
        let listed = file(&seen[0], "blob.bin");
        assert!(listed.is_omitted(), "the pass was not sent the bytes");
        assert_eq!(listed.path(), "blob.bin", "but it was told the name");
        assert_eq!(listed.size(), size, "and the size");
        assert_eq!(listed.bytes(), None, "and no part of the file at all");
        assert_eq!(listed.summary(), None, "and nothing made up about it");
    }

    /// A fat directory: five files that come to nearly twice the request cap
    /// between them, named so that alphabetical order is the reverse of size
    /// order — an operation that gave files up in path order would fail on it.
    ///
    /// Written in [`share`]s of the cap, for the reason given there. The
    /// thirty-one-to-forty-seven spread leaves the two smallest fitting and the
    /// third not.
    const FAT_SHARES: [(&str, u64); 5] = [
        ("a.bin", 31),
        ("b.bin", 35),
        ("c.bin", 39),
        ("d.bin", 43),
        ("e.bin", 47),
    ];

    /// The files of [`FAT_SHARES`], with their sizes, in the same order.
    fn fat() -> [(&'static str, u64); 5] {
        FAT_SHARES.map(|(name, percent)| (name, share(percent)))
    }

    /// The paths of the files a request carries whole, in its own order.
    fn sent(request: &agent::Request) -> Vec<&str> {
        request
            .files()
            .iter()
            .filter(|file| file.bytes().is_some())
            .map(agent::File::path)
            .collect()
    }

    /// The paths of the files a request carries as a name and a size alone, in
    /// its own order.
    fn listed(request: &agent::Request) -> Vec<&str> {
        request
            .files()
            .iter()
            .filter(|file| file.is_omitted())
            .map(agent::File::path)
            .collect()
    }

    #[test]
    fn a_fat_directory_of_files_that_cannot_be_described_still_falls_to_names_and_sizes() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        // The same five files, none of them text: the ladder's bottom rung is
        // exactly where it always was, and the cause says which rung failed.
        for (name, size) in fat() {
            write(dir.path(), name, not_text(size));
        }
        let agent = Counting::new(document(300));

        let Pacted { problems, .. } = pact_directory(dir.path(), &agent).expect("still pactable");

        assert_eq!(
            agent.passes(),
            1,
            "not one pass is spent on bytes that are not text",
        );
        let seen = agent.seen.borrow();
        let pass = pass(&seen);
        assert_eq!(sent(pass), ["a.bin", "b.bin"]);
        assert_eq!(
            listed(pass),
            ["c.bin", "d.bin", "e.bin"],
            "a name and a size is still the floor",
        );
        assert_eq!(
            problems
                .iter()
                .map(|problem| problem.path.clone())
                .collect::<Vec<_>>(),
            ["e.bin", "d.bin", "c.bin"].map(|name| dir.path().join(name)),
            "and the pact reports each one, largest first, having succeeded anyway",
        );
        assert!(
            problems
                .iter()
                .all(|problem| matches!(problem.cause, Omission::NotText { .. })),
            "one file, one problem, and its cause is why there is no account of it \
             rather than the cap that listed it: {problems:?}",
        );
    }

    #[test]
    fn an_account_that_does_not_fit_leaves_the_file_on_the_cliff_it_was_taken_to() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        // Three files of one size: the budget takes one of them, and which one
        // is decided by path because the sizes cannot decide it. Sized against
        // the trim's reserved target rather than the cap — 105% of the budget
        // has to come down to the 75% `trim_to_budget` aims for, and one file
        // of the three is what does it.
        for name in ["a.bin", "b.bin", "c.bin"] {
            write(dir.path(), name, filler(share(35)));
        }
        // An account far too long to fit in what is left of the budget: two
        // files of 100 KiB are already in the request. One pass makes it —
        // filler has no line to cut on, so a file of it is one chunk and one
        // chunk is one map pass with no reduce over it.
        let agent = Counting::new(document(300)).scripted([Ok(document(share_bytes(31)))]);

        let Pacted { problems, .. } = pact_directory(dir.path(), &agent)
            .expect("an account with nowhere to go is not a failure");

        let seen = agent.seen.borrow();
        let pass = pass(&seen);
        assert_eq!(
            sent(pass),
            ["b.bin", "c.bin"],
            "ties are broken by path, so the file given up is a value and not a race",
        );
        assert_eq!(
            listed(pass),
            ["a.bin"],
            "and it stays given up: an account that does not fit is not carried",
        );
        assert!(
            carried(pass) <= REQUEST_BYTE_CAP,
            "{} bytes is over the {REQUEST_BYTE_CAP}-byte cap",
            carried(pass),
        );
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert_eq!(problems[0].path, dir.path().join("a.bin"));
        assert!(
            matches!(problems[0].cause, Omission::Unreducible { size } if size == share(35)),
            "filler has no declarations to lift, so there is nothing to send but its name: {:?}",
            problems[0],
        );
    }

    #[test]
    fn every_childs_document_survives_the_ladder_whole_and_in_its_place() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        // A child document larger by itself than the whole request may be: no
        // rung of the ladder can make room for it, and none is allowed to try by
        // giving it up.
        let pathological = "x".repeat(usize::try_from(REQUEST_BYTE_CAP).expect("fits") + 1);
        let ordinary = "# tests\n\nThe integration tests.\n";
        write(dir.path(), "src/WARLOCK.md", &pathological);
        write(dir.path(), "tests/WARLOCK.md", ordinary);
        write(dir.path(), "lib.rs", filler(1024));
        write(dir.path(), "main.rs", filler(2048));
        let agent = Counting::new(document(300));

        let Pacted { problems, .. } =
            pact_directory(dir.path(), &agent).expect("over the cap is never a failure");

        assert_eq!(
            agent.passes(),
            1,
            "and not one pass is spent describing a file into a request that has \
             no room for the account either: {:?}",
            agent.prompts(),
        );
        let seen = agent.seen.borrow();
        let pass = pass(&seen);
        assert_eq!(
            pass.child_documents()
                .iter()
                .map(|child| (child.directory(), child.text().len()))
                .collect::<Vec<_>>(),
            [("src", pathological.len()), ("tests", ordinary.len())],
            "both children keep their place in the request, in order, and the \
             pathological one keeps every byte: an account of a whole subtree is \
             never demoted and never dropped",
        );
        assert_eq!(
            pass.child_documents()[0].text(),
            pathological,
            "byte for byte, because there is nothing else that says what is under \
             `src/`",
        );
        assert!(
            carried(pass) > REQUEST_BYTE_CAP,
            "so the request legitimately stays over the cap — {} bytes — rather \
             than the pact going nowhere",
            carried(pass),
        );
        // The files do not give way either. Each is small enough that naming it
        // would free less than the account the lift would then buy, so the
        // trade loses before it starts — and against a child document that is
        // the whole budget on its own, no number of them could have helped.
        // They keep their text, and nothing is reported as given up.
        assert!(
            listed(pass).is_empty(),
            "files too small for the trade to pay keep their contents: {:?}",
            listed(pass),
        );
        assert!(
            problems.is_empty(),
            "and nothing was left out to report: {problems:?}",
        );
        assert!(
            problems
                .iter()
                .all(|problem| matches!(problem.cause, Omission::OverBudget { .. })),
            "{problems:?}",
        );
    }

    #[test]
    fn a_pass_is_sent_its_childrens_documents_and_none_of_their_source() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        write(dir.path(), "Cargo.toml", "[package]\n");
        write(dir.path(), "src/WARLOCK.md", "# src\n\nThe code.\n");
        write(
            dir.path(),
            "src/lib.rs",
            "//! Not for the parent to read.\n",
        );
        write(dir.path(), "tests/it.rs", "#[test] fn works() {}\n");
        let agent = Canned::filling();

        pact_directory(dir.path(), &agent).expect("pacts");

        let seen = agent.seen.borrow();
        assert_eq!(
            seen[0]
                .child_documents()
                .iter()
                .map(|child| (child.directory(), child.text()))
                .collect::<Vec<_>>(),
            [("src", "# src\n\nThe code.\n")],
            "the child describes itself; `tests/` has no document and \
             contributes no entry, which is not an error",
        );
        assert_eq!(
            file_paths(&seen[0]),
            ["Cargo.toml"],
            "and the child's source is not a file of the parent",
        );
        assert!(
            !format!("{:?}", seen[0]).contains("Not for the parent to read"),
            "nor is it anywhere else in the request",
        );
    }

    #[test]
    fn a_directory_that_cannot_be_gathered_never_reaches_the_agent() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let missing = dir.path().join("nowhere");
        let agent = Canned::filling();

        let error = pact_directory(&missing, &agent).expect_err("there is nothing to walk");

        assert!(matches!(error, super::Error::Walk { .. }), "{error:?}");
        assert_eq!(
            error.directory(),
            missing,
            "a walk that failed still says which directory it was",
        );
        assert!(
            agent.seen.borrow().is_empty(),
            "no request, no pass: the expensive half never runs",
        );
    }

    /// A repository with one directory of every kind the walk has an opinion
    /// about, so that "the same rules as the loader" is asserted against
    /// something and not just claimed.
    ///
    /// Under `crates/engine` — the subtree the tests below pact — sit three
    /// ordinary directories, a gitignored one, a hidden one and a `.warlock/`
    /// one; outside it sit a sibling crate and a gitignored `target/`, so a walk
    /// that started from the wrong place would be caught too.
    fn repository() -> tempfile::TempDir {
        let repo = tempfile::tempdir().expect("a temporary directory");
        write(repo.path(), ".git/config", "[core]\n");
        write(repo.path(), ".gitignore", "/target\ngenerated/\n");
        write(repo.path(), ".warlock/pacts.toml", "version = 1\n");
        for dir in [
            "crates/engine/src/inner",
            "crates/engine/tests",
            "crates/engine/generated/schema",
            "crates/engine/.hidden/cache",
            "crates/engine/.warlock",
            "crates/tui/src",
            "target/debug",
        ] {
            fs::create_dir_all(repo.path().join(dir)).expect("creates a directory");
        }
        repo
    }

    /// `paths` spelled relative to `root`, with forward slashes: what the
    /// assertions below are written in, rather than temporary directory names
    /// nobody can predict.
    fn relative_to(root: &Path, paths: &[PathBuf]) -> Vec<String> {
        paths
            .iter()
            .map(|path| {
                path.strip_prefix(root)
                    .expect("every directory sits under the root")
                    .to_string_lossy()
                    .replace('\\', "/")
            })
            .collect()
    }

    #[test]
    fn a_subtree_is_exactly_the_directories_the_loader_makes_nodes_of() {
        let repo = repository();
        let subtree = repo.path().join("crates/engine");

        let pacted = pactable_directories(&subtree).expect("walks");

        // The loader is the authority on which directories exist, because it is
        // what the user is looking at when they press the key. Compared as sets,
        // since the two orders are deliberately opposite.
        let Loaded { tree, problems } = load_tree(&subtree).expect("loads");
        assert!(problems.is_empty(), "{problems:?}");
        let mut walked: Vec<PathBuf> = tree.walk().map(|(node, _)| node.path.clone()).collect();
        let mut sorted = pacted.clone();
        walked.sort();
        sorted.sort();
        assert_eq!(
            sorted, walked,
            "a pact covers the nodes of the subtree, no more and no fewer",
        );

        assert_eq!(
            relative_to(repo.path(), &sorted),
            [
                "crates/engine",
                "crates/engine/src",
                "crates/engine/src/inner",
                "crates/engine/tests",
            ],
            "the selected directory and every ordinary directory below it; \
             `generated/` is gitignored, `.hidden/` is hidden and `.warlock/` \
             is ours, so none of them — nor anything inside them — is pactable",
        );
    }

    #[test]
    fn every_child_comes_before_its_parent_and_the_selected_directory_is_last() {
        let repo = repository();
        let subtree = repo.path().join("crates/engine");

        let pacted = pactable_directories(&subtree).expect("walks");

        assert_eq!(
            relative_to(repo.path(), &pacted),
            [
                "crates/engine/tests",
                "crates/engine/src/inner",
                "crates/engine/src",
                "crates/engine",
            ],
            "deepest first, and the directory the pact was asked for last",
        );
        // Said again as the property rather than the listing: a parent's request
        // carries its children's documents, so no directory may be pacted before
        // anything below it has written one.
        for (index, directory) in pacted.iter().enumerate() {
            for (other, descendant) in pacted.iter().enumerate() {
                if descendant != directory && descendant.starts_with(directory) {
                    assert!(
                        other < index,
                        "`{}` is below `{}` and has to come first",
                        descendant.display(),
                        directory.display(),
                    );
                }
            }
        }
        assert_eq!(
            pacted.last().map(PathBuf::as_path),
            Some(subtree.as_path()),
            "and the last pass is the one the whole subtree was gathered for",
        );
    }

    #[test]
    fn a_directory_with_nothing_below_it_is_a_subtree_of_one() {
        let repo = repository();
        let leaf = repo.path().join("crates/engine/src/inner");

        assert_eq!(
            pactable_directories(&leaf).expect("walks"),
            [leaf],
            "a pact always covers the directory it was asked for, documented \
             or not, empty or not",
        );
    }

    #[test]
    fn a_subtree_that_cannot_be_walked_says_which_directory_it_was() {
        let repo = repository();
        let missing = repo.path().join("crates/engine/nowhere");

        let error = pactable_directories(&missing).expect_err("there is nothing to walk");

        assert!(matches!(error, super::Error::Walk { .. }), "{error:?}");
        assert_eq!(error.directory(), missing);
    }

    /// A fake that answers everywhere but one directory, which is how partial
    /// completion is reached without a filesystem trick: exactly one pass
    /// refuses, and everything else in the subtree is ordinary.
    struct FailsFor {
        /// The one directory nothing is ever written for.
        directory: PathBuf,
    }

    impl Agent for FailsFor {
        fn run(&self, request: &agent::Request) -> Result<agent::Response, agent::Error> {
            if request.directory() == self.directory {
                return Err(agent::Error::EmptyOutput);
            }
            Ok(agent::Response::new(document::stub_answer(request)))
        }
    }

    /// The whole front-end side of a pact with no front end in it: an observer
    /// that writes down every call and can stop the descent.
    struct Watching {
        /// How many directories are let through before the next one offered is
        /// turned down, or `None` for an observer that never cancels.
        stop_after: Option<usize>,
        /// Every call, in order: the directory offered, its position and the
        /// total it was one of.
        calls: Vec<(PathBuf, usize, usize)>,
        /// Every `documented` announcement, in the order it was made.
        documented: Vec<PathBuf>,
    }

    impl Watching {
        /// An observer that only watches: every directory is pacted.
        fn patient() -> Self {
            Self {
                stop_after: None,
                calls: Vec::new(),
                documented: Vec::new(),
            }
        }

        /// An observer that lets `directories` directories be pacted and stops
        /// the pact at the next one it is offered.
        fn stopping_after(directories: usize) -> Self {
            Self {
                stop_after: Some(directories),
                calls: Vec::new(),
                documented: Vec::new(),
            }
        }

        /// What it was told, with each directory named relative to `root`.
        fn calls(&self, root: &Path) -> Vec<(String, usize, usize)> {
            self.calls
                .iter()
                .map(|(directory, position, total)| {
                    let named = relative_to(root, std::slice::from_ref(directory))
                        .pop()
                        .expect("one directory in, one name out");
                    (named, *position, *total)
                })
                .collect()
        }

        /// The directories it was offered, in the order they were offered.
        fn offered(&self) -> Vec<PathBuf> {
            self.calls
                .iter()
                .map(|(directory, ..)| directory.clone())
                .collect()
        }

        /// The directories announced documented, named relative to `root`, in
        /// the order they were announced.
        fn done(&self, root: &Path) -> Vec<String> {
            relative_to(root, &self.documented)
        }
    }

    impl Observer for Watching {
        fn starting(&mut self, directory: &Path, position: usize, total: usize) -> Pacting {
            self.calls.push((directory.to_path_buf(), position, total));
            match self.stop_after {
                Some(limit) if position > limit => Pacting::Stop,
                _ => Pacting::Continue,
            }
        }

        fn documented(&mut self, directory: &Path) {
            self.documented.push(directory.to_path_buf());
        }
    }

    /// A repository with files in it, so that the directories below have
    /// something to hash and something to be described from.
    ///
    /// The same shape as [`repository`] — an ignored `target/`, a `.warlock/`,
    /// a sibling crate outside the subtree the tests below pact — with content
    /// added, because a pact that hashes nothing proves nothing about hashing.
    fn project() -> tempfile::TempDir {
        let repo = tempfile::tempdir().expect("a temporary directory");
        write(repo.path(), ".git/config", "[core]\n");
        write(repo.path(), ".gitignore", "/target\n");
        write(repo.path(), ".warlock/pacts.toml", "version = 1\n");
        write(repo.path(), "Cargo.toml", "[workspace]\n");
        write(repo.path(), "crates/engine/Cargo.toml", "[package]\n");
        write(
            repo.path(),
            "crates/engine/src/lib.rs",
            "//! Core engine.\n",
        );
        write(
            repo.path(),
            "crates/engine/src/inner/deep.rs",
            "fn deep() {}\n",
        );
        write(
            repo.path(),
            "crates/engine/tests/it.rs",
            "#[test] fn works() {}\n",
        );
        write(repo.path(), "crates/tui/src/main.rs", "fn main() {}\n");
        write(repo.path(), "target/debug/build.log", "noise\n");
        repo
    }

    /// What `module` renders as right now: its entry, judged against what its
    /// directory hashes to at this moment.
    fn state(manifest: &Manifest, root: &Path, module: &str) -> NodeState {
        let hash = subtree_hash(from_manifest_path(root, module)).expect("the subtree hashes");
        decide_state(manifest.entry(module), &hash)
    }

    #[test]
    fn every_directory_is_pacted_before_the_one_above_it() {
        let repo = project();
        let engine = repo.path().join("crates/engine");
        let agent = Canned::filling();

        pact_subtree(
            &engine,
            repo.path(),
            &Manifest::new(),
            &agent,
            &mut Unwatched,
        )
        .expect("pacts");

        let seen: Vec<PathBuf> = agent
            .seen
            .borrow()
            .iter()
            .map(|request| request.directory().to_path_buf())
            .collect();
        assert_eq!(
            relative_to(repo.path(), &seen),
            [
                "crates/engine/tests",
                "crates/engine/src/inner",
                "crates/engine/src",
                "crates/engine",
            ],
            "one pass per directory, deepest first, the selected directory last",
        );
        // Said again as the property, since the listing above is one fixture and
        // this is the rule: no request may be issued for a directory before
        // every request below it has been.
        for (index, directory) in seen.iter().enumerate() {
            for (other, descendant) in seen.iter().enumerate() {
                if descendant != directory && descendant.starts_with(directory) {
                    assert!(
                        other < index,
                        "`{}` is below `{}` and has to be pacted first",
                        descendant.display(),
                        directory.display(),
                    );
                }
            }
        }
        // And this is what the ordering is *for*: the last pass was handed the
        // documents the earlier ones had already written.
        let seen = agent.seen.borrow();
        let parent = seen.last().expect("the selected directory was pacted");
        assert_eq!(
            parent
                .child_documents()
                .iter()
                .map(agent::ChildDocument::directory)
                .collect::<Vec<_>>(),
            ["src", "tests"],
            "a parent reads its children's finished documents, not their source",
        );
    }

    #[test]
    fn a_directory_the_repository_excluded_is_no_part_of_a_pact_above_it() {
        let repo = project();
        let engine = repo.path().join("crates/engine");
        write(&engine, ".warlockignore", "tests/\n");
        let excluded = engine.join("tests");

        let PactedSubtree {
            manifest, failures, ..
        } = pact_subtree(
            &engine,
            repo.path(),
            &Manifest::new(),
            &Canned::filling(),
            &mut Unwatched,
        )
        .expect("pacts");

        assert!(failures.is_empty(), "{failures:?}");
        assert_eq!(
            modules(&manifest),
            [
                "crates/engine",
                "crates/engine/src",
                "crates/engine/src/inner",
            ],
            "the excluded directory earns no entry, and the rest of the \
             subtree is pacted exactly as it always was",
        );
        assert_eq!(
            written(&excluded),
            None,
            "and no document was written into it: a pact of an ancestor is not \
             a way round what the repository excluded",
        );
    }

    #[test]
    fn rules_that_cannot_be_parsed_fail_the_pact_rather_than_meaning_no_rules() {
        let repo = project();
        let engine = repo.path().join("crates/engine");
        // A range that runs backwards: a glob the matcher will not compile.
        write(&engine, ".warlockignore", "a[z-a]\n");
        let agent = Canned::filling();

        let error = pact_subtree(
            &engine,
            repo.path(),
            &Manifest::new(),
            &agent,
            &mut Unwatched,
        )
        .expect_err("a pact that cannot tell what is excluded must not run");

        assert!(matches!(error, super::Error::Walk { .. }), "{error:?}");
        assert!(
            error.to_string().contains(".warlockignore"),
            "the one line back names the file to go and fix: {error}"
        );
        assert!(
            agent.seen.borrow().is_empty(),
            "and it fails before a single pass is spent",
        );
    }

    #[test]
    fn a_directory_with_no_document_gets_no_entry_and_costs_its_ancestors_their_grants() {
        let repo = project();
        let engine = repo.path().join("crates/engine");
        let failing = engine.join("src").join("inner");
        let agent = FailsFor {
            directory: failing.clone(),
        };

        let PactedSubtree {
            manifest, failures, ..
        } = pact_subtree(
            &engine,
            repo.path(),
            &Manifest::new(),
            &agent,
            &mut Unwatched,
        )
        .expect("one directory failing is not the pact failing");

        assert!(
            manifest.entry("crates/engine/src/inner").is_none(),
            "a directory this run could not describe is not one it pacted",
        );
        assert_eq!(
            written(&failing),
            None,
            "and nothing was written for it either",
        );

        for module in ["crates/engine/src", "crates/engine"] {
            let entry = manifest.entry(module).expect("pacted, if not judged");
            assert_eq!(
                entry.granted_hash(),
                None,
                "`{module}` has an incomplete subtree below it, so it earned no grant",
            );
            assert_eq!(entry.granted_at(), None, "and no timestamp for one");
            assert_eq!(
                state(&manifest, repo.path(), module),
                NodeState::PactedStale,
                "which renders yellow, by the existing freshness rule",
            );
        }

        let sibling = manifest
            .entry("crates/engine/tests")
            .expect("a completed subtree is still pacted");
        assert_eq!(
            sibling.granted_hash(),
            Some(subtree_hash(engine.join("tests")).expect("hashes").as_str()),
        );
        assert_eq!(
            state(&manifest, repo.path(), "crates/engine/tests"),
            NodeState::PactedFresh,
            "one failure elsewhere does not take a finished subtree's grant away",
        );

        assert_eq!(failures.len(), 1, "{failures:?}");
        assert!(
            matches!(&failures[0], Failure::Document { .. }),
            "{:?}",
            failures[0],
        );
        assert_eq!(failures[0].directory(), failing);
        assert!(
            failures[0]
                .to_string()
                .contains(&failing.display().to_string()),
            "a failure says which directory it is about: {}",
            failures[0],
        );
    }

    #[test]
    fn the_repository_root_is_a_module_like_any_other_and_stores_as_a_dot() {
        let repo = project();

        let PactedSubtree {
            manifest, failures, ..
        } = pact_subtree(
            repo.path(),
            repo.path(),
            &Manifest::new(),
            &Canned::filling(),
            &mut Unwatched,
        )
        .expect("pacts");

        assert!(failures.is_empty(), "{failures:?}");
        assert_eq!(
            modules(&manifest),
            [
                ".",
                "crates",
                "crates/engine",
                "crates/engine/src",
                "crates/engine/src/inner",
                "crates/engine/tests",
                "crates/tui",
                "crates/tui/src",
            ],
            "the root stores as `.`, and `target/`, `.git/` and `.warlock/` are \
             not modules",
        );

        let root = manifest.entry(".").expect("the root is pacted too");
        assert_eq!(
            root.document(),
            "WARLOCK.md",
            "documented by the `WARLOCK.md` sitting in the root itself",
        );
        assert_eq!(root.module_path(repo.path()), repo.path());
        assert_eq!(
            state(&manifest, repo.path(), "."),
            NodeState::PactedFresh,
            "and a whole-repository pact leaves the whole repository green",
        );
    }

    /// Only on unix, because there is no portable way to make a file
    /// unreadable. What is under test — that a hash nobody can compute leaves an
    /// entry ungranted instead of taking the pact down — is not
    /// platform-specific.
    #[cfg(unix)]
    #[test]
    fn a_directory_that_cannot_be_hashed_is_pacted_without_a_grant() {
        use std::os::unix::fs::PermissionsExt as _;

        let repo = project();
        let engine = repo.path().join("crates/engine");
        let unreadable = engine.join("tests").join("it.rs");
        fs::set_permissions(&unreadable, fs::Permissions::from_mode(0o000)).expect("chmods");
        if fs::read(&unreadable).is_ok() {
            // Running as root: no file is unreadable, so there is nothing here
            // to assert against.
            return;
        }

        let PactedSubtree {
            manifest,
            failures,
            problems,
        } = pact_subtree(
            &engine,
            repo.path(),
            &Manifest::new(),
            &Canned::filling(),
            &mut Unwatched,
        )
        .expect("a file nobody can read never fails the pact");

        // Both directories whose hash would have covered the unreadable file.
        for module in ["crates/engine/tests", "crates/engine"] {
            let entry = manifest.entry(module).expect("documented, so pacted");
            assert_eq!(
                entry.granted_hash(),
                None,
                "`{module}` has no hash, and a hash nobody computed is never invented",
            );
        }
        assert_eq!(
            manifest
                .entry("crates/engine/src")
                .and_then(PactEntry::granted_hash),
            Some(subtree_hash(engine.join("src")).expect("hashes").as_str()),
            "the part of the subtree that can be hashed is still granted",
        );

        assert_eq!(failures.len(), 2, "{failures:?}");
        assert!(
            failures
                .iter()
                .all(|failure| matches!(failure, Failure::Hash { .. })),
            "a document that was written is never reported as one that was not: \
             {failures:?}",
        );
        assert!(
            problems.iter().any(|problem| problem.path == unreadable
                && matches!(problem.cause, Omission::Unreadable { .. })),
            "and the request that could not read it said so, non-fatally: {problems:?}",
        );

        fs::set_permissions(&unreadable, fs::Permissions::from_mode(0o644)).expect("chmods back");
    }

    // Progress and cancellation: the observer port.

    #[test]
    fn every_directory_is_announced_once_before_it_is_pacted() {
        let repo = project();
        let engine = repo.path().join("crates/engine");
        let agent = Canned::filling();
        let mut observer = Watching::patient();

        let PactedSubtree { failures, .. } = pact_subtree(
            &engine,
            repo.path(),
            &Manifest::new(),
            &agent,
            &mut observer,
        )
        .expect("pacts");

        assert!(failures.is_empty(), "{failures:?}");
        assert_eq!(
            observer.calls(repo.path()),
            [
                ("crates/engine/tests".to_owned(), 1, 4),
                ("crates/engine/src/inner".to_owned(), 2, 4),
                ("crates/engine/src".to_owned(), 3, 4),
                ("crates/engine".to_owned(), 4, 4),
            ],
            "once per directory, in the pact's own order, 1-based, out of a \
             total that does not move while the pact runs",
        );
        assert_eq!(
            observer.offered(),
            agent
                .seen
                .borrow()
                .iter()
                .map(|request| request.directory().to_path_buf())
                .collect::<Vec<_>>(),
            "and each one names the directory whose pass runs next, not the one \
             that has just finished",
        );
    }

    #[test]
    fn a_directory_is_announced_documented_the_moment_its_pass_delivers() {
        let repo = project();
        let engine = repo.path().join("crates/engine");
        let mut observer = Watching::patient();

        let PactedSubtree { failures, .. } = pact_subtree(
            &engine,
            repo.path(),
            &Manifest::new(),
            &Canned::filling(),
            &mut observer,
        )
        .expect("pacts");

        // One announcement per directory, in the order the passes finish —
        // which on a clean run is the order they were offered in, children
        // before parents. Each lands before the next directory is offered,
        // which is what lets a front end colour work done while the run is
        // still paying for the rest.
        assert!(failures.is_empty(), "{failures:?}");
        assert_eq!(
            observer.done(repo.path()),
            [
                "crates/engine/tests",
                "crates/engine/src/inner",
                "crates/engine/src",
                "crates/engine",
            ],
        );
    }

    #[test]
    fn a_directory_above_a_failure_is_never_announced_documented() {
        let repo = project();
        let engine = repo.path().join("crates/engine");
        let failing = engine.join("src").join("inner");
        let agent = FailsFor { directory: failing };
        let mut observer = Watching::patient();

        let PactedSubtree { failures, .. } = pact_subtree(
            &engine,
            repo.path(),
            &Manifest::new(),
            &agent,
            &mut observer,
        )
        .expect("one refused pass does not fail the pact");

        // `tests` is a whole subtree this run documented, so it is announced.
        // `src/inner` failed, `src` and `engine` sit above the failure, and
        // all three are headed for an entry with no grant or none at all —
        // the announcement stays honest by saying nothing about any of them,
        // even though `src` and `engine` did write documents.
        assert_eq!(failures.len(), 1, "{failures:?}");
        assert_eq!(observer.done(repo.path()), ["crates/engine/tests"]);
    }

    #[test]
    fn a_cancelled_pact_stops_between_directories_and_keeps_what_it_wrote() {
        let repo = project();
        let engine = repo.path().join("crates/engine");
        let mut observer = Watching::stopping_after(2);

        let PactedSubtree {
            manifest, failures, ..
        } = pact_subtree(
            &engine,
            repo.path(),
            &Manifest::new(),
            &Canned::filling(),
            &mut observer,
        )
        .expect("a pact somebody stopped is not a pact that failed");

        assert_eq!(
            observer.calls(repo.path()).len(),
            3,
            "the third directory was offered and turned down, and there was no \
             fourth question: {:?}",
            observer.calls(repo.path()),
        );
        assert!(
            failures.is_empty(),
            "nothing went wrong — fewer directories were asked for: {failures:?}",
        );

        for documented in ["crates/engine/tests", "crates/engine/src/inner"] {
            let directory = from_manifest_path(repo.path(), documented);
            assert!(
                written(&directory).is_some(),
                "`{documented}` was pacted before the cancel, so its document stays on disk",
            );
        }
        for untouched in ["crates/engine/src", "crates/engine"] {
            let directory = from_manifest_path(repo.path(), untouched);
            assert_eq!(
                written(&directory),
                None,
                "`{untouched}` is at or past the cancel, so no pass ran for it",
            );
        }

        assert_eq!(
            modules(&manifest),
            ["crates/engine/src/inner", "crates/engine/tests"],
            "a directory the pact never reached is undocumented by this run, \
             and an undocumented directory gets no entry",
        );
        for module in modules(&manifest) {
            assert_eq!(
                state(&manifest, repo.path(), module),
                NodeState::PactedFresh,
                "`{module}` is a whole subtree this run documented, so it is \
                 granted like any other",
            );
        }
    }

    #[test]
    fn a_cancel_leaves_a_documented_ancestor_of_a_failure_pacted_without_a_grant() {
        let repo = project();
        let engine = repo.path().join("crates/engine");
        let failing = engine.join("src").join("inner");
        let agent = FailsFor {
            directory: failing.clone(),
        };
        // Everything but the selected directory itself, so the run holds all
        // three cases at once: `crates/engine/tests` finished, `crates/engine/src`
        // is documented above a directory that is not, and `crates/engine` is
        // never reached.
        let mut observer = Watching::stopping_after(3);

        let PactedSubtree {
            manifest, failures, ..
        } = pact_subtree(
            &engine,
            repo.path(),
            &Manifest::new(),
            &agent,
            &mut observer,
        )
        .expect("neither a failure nor a cancel fails the pact");

        assert!(
            manifest.entry("crates/engine/src/inner").is_none(),
            "no document, no entry — the cancel changes none of that rule",
        );
        let src = manifest
            .entry("crates/engine/src")
            .expect("documented, so pacted");
        assert_eq!(
            src.granted_hash(),
            None,
            "it has an undocumented descendant, so it earned no grant",
        );
        assert_eq!(
            state(&manifest, repo.path(), "crates/engine/src"),
            NodeState::PactedStale,
            "which renders yellow, by the existing freshness rule",
        );
        assert!(
            manifest.entry("crates/engine").is_none(),
            "and the directory the cancel landed on was never pacted at all",
        );

        let finished = manifest
            .entry("crates/engine/tests")
            .expect("a completed subtree is still pacted");
        assert_eq!(
            finished.granted_hash(),
            Some(subtree_hash(engine.join("tests")).expect("hashes").as_str()),
        );
        assert_eq!(
            state(&manifest, repo.path(), "crates/engine/tests"),
            NodeState::PactedFresh,
            "what finished before the cancel keeps what it earned",
        );

        assert_eq!(
            failures.len(),
            1,
            "the directory that failed is reported; the ones nobody asked for \
             are not: {failures:?}",
        );
        assert_eq!(failures[0].directory(), failing);
    }

    #[test]
    fn an_unwatched_pact_is_the_pact_that_never_stops() {
        let repo = project();
        let engine = repo.path().join("crates/engine");

        let PactedSubtree {
            manifest, failures, ..
        } = pact_subtree(
            &engine,
            repo.path(),
            &Manifest::new(),
            &Canned::filling(),
            &mut Unwatched,
        )
        .expect("pacts");

        assert!(failures.is_empty(), "{failures:?}");
        assert_eq!(
            modules(&manifest),
            [
                "crates/engine",
                "crates/engine/src",
                "crates/engine/src/inner",
                "crates/engine/tests",
            ],
            "the caller that watches nothing gets every directory pacted",
        );
        assert_eq!(Unwatched.starting(&engine, 1, 4), Pacting::Continue);
    }

    // Announcing the summarising passes: what the observer hears while one
    // directory's big file is being read, and in what order.

    // Announcing the request itself: what the directory's own pass was handed.

    /// An observer that hears what every request weighed and stops nothing.
    ///
    /// [`Observer::requesting`] only, plus the [`Observer::starting`] the trait
    /// requires: the point of it is that the announcement carries its numbers
    /// on its own, with no directory to match up and nothing else to remember.
    #[derive(Default)]
    struct Weighing(Vec<(usize, u64)>);

    impl Observer for Weighing {
        fn starting(&mut self, _directory: &Path, _position: usize, _total: usize) -> Pacting {
            Pacting::Continue
        }

        fn requesting(&mut self, files: usize, bytes: u64) {
            self.0.push((files, bytes));
        }
    }

    #[test]
    fn the_announced_bytes_are_the_budget_total_and_not_just_the_files() {
        let repo = tempfile::tempdir().expect("a temporary directory");
        let engine = repo.path().join("crates/engine");
        write(&engine, "Cargo.toml", "[package]\n");
        write(&engine, "src/lib.rs", "//! Core engine.\n");

        let mut watching = Weighing::default();
        pact_subtree(
            &engine,
            repo.path(),
            &Manifest::new(),
            &Canned::filling(),
            &mut watching,
        )
        .expect("pacts");

        // `src` first, with its one file and nothing under it; then the parent,
        // whose one file is `Cargo.toml` and whose total also carries the
        // document `src` has just been given. The counts cover different sets on
        // purpose: the bytes are what the caps are checked against.
        let child = fs::metadata(engine.join("src").join(DOCUMENT_FILE))
            .expect("the child was documented")
            .len();
        assert_eq!(watching.0.len(), 2, "one announcement per directory");
        let (files, bytes) = watching.0[1];
        assert_eq!(
            files, 1,
            "the parent's own file, with the child's not in it"
        );
        assert!(
            bytes > child,
            "the total carries the child's document as well as the file: {bytes} against {child}",
        );
    }

    // Un-pacting: dropping the entries and keeping the documents.

    /// A manifest of hand-built entries for `modules`, each documented by its
    /// own `WARLOCK.md` and each granted, so that "kept unchanged" is a claim
    /// about the grants too and not only about the paths.
    fn pacted(modules: &[&str]) -> Manifest {
        Manifest::with_entries(modules.iter().map(|module| {
            PactEntry::new(".", module, format!("{module}/WARLOCK.md"))
                .expect("a relative path inside the root is storable")
                .with_grant(format!("hash-of-{module}"), "2026-08-21T09:00:00Z")
        }))
    }

    /// Every file at and below `dir`, as its path relative to `dir` and its
    /// bytes. The whole filesystem state a test cares about, in one value, so
    /// "nothing was written and nothing was deleted" is a single assertion.
    fn snapshot(dir: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
        let mut files = BTreeMap::new();
        let mut pending = vec![dir.to_path_buf()];
        while let Some(next) = pending.pop() {
            for entry in fs::read_dir(&next).expect("a readable directory") {
                let path = entry.expect("a readable entry").path();
                if path.is_dir() {
                    pending.push(path);
                } else {
                    let relative = path
                        .strip_prefix(dir)
                        .expect("under the root")
                        .to_path_buf();
                    files.insert(relative, fs::read(&path).expect("a readable file"));
                }
            }
        }
        files
    }

    #[test]
    fn an_un_pact_drops_the_directory_and_everything_below_it() {
        let manifest = pacted(&[
            ".",
            "crates/engine",
            "crates/engine/src",
            "crates/engine/src/inner",
            "crates/engine/tests",
            "crates/engine-tools",
            "crates/tui",
        ]);

        let left = unpact_subtree("crates/engine", ".", &manifest).expect("un-pacts");

        assert_eq!(
            modules(&left),
            [".", "crates/engine-tools", "crates/tui"],
            "the directory and its descendants go, and nothing else does",
        );
    }

    #[test]
    fn a_sibling_that_shares_a_prefix_is_not_a_descendant() {
        // The whole reason the match is by path segment: `engine-tools` sorts
        // right next to `engine` and starts with every character of it.
        let manifest = pacted(&[
            "crates/engine",
            "crates/engine-tools",
            "crates/engine-tools/src",
            "crates/engineering",
        ]);

        let left = unpact_subtree("crates/engine", ".", &manifest).expect("un-pacts");

        assert_eq!(
            modules(&left),
            [
                "crates/engine-tools",
                "crates/engine-tools/src",
                "crates/engineering"
            ],
        );
    }

    #[test]
    fn the_repository_root_is_below_nothing_but_itself() {
        let manifest = pacted(&[".", "crates/engine/src"]);

        let left = unpact_subtree("crates/engine", ".", &manifest).expect("un-pacts");

        assert_eq!(
            modules(&left),
            ["."],
            "a pact on the repository as a whole is not a pact on the subtree, \
             so un-pacting the subtree leaves it alone",
        );
    }

    #[test]
    fn un_pacting_something_that_was_never_pacted_changes_nothing() {
        let manifest = pacted(&["crates/engine", "crates/engine/src"]);

        let left = unpact_subtree("docs/adr", ".", &manifest).expect("un-pacts");
        assert_eq!(left, manifest);

        // And doing it twice says the same thing as doing it once.
        let once = unpact_subtree("crates/engine", ".", &manifest).expect("un-pacts");
        let twice = unpact_subtree("crates/engine", ".", &once).expect("un-pacts again");
        assert_eq!(twice, once);
    }

    #[test]
    fn a_directory_with_no_manifest_relative_form_is_an_error() {
        let manifest = pacted(&["crates/engine"]);
        assert!(matches!(
            unpact_subtree("/elsewhere/crates", "/repo", &manifest),
            Err(manifest::Error::PathOutsideRoot { .. })
        ));
    }

    /// A manifest of hand-built entries in the order given, each carrying
    /// whatever scope it is paired with — `None` for a module nobody has drawn
    /// a boundary on. The order is the point: manifest file order is what a
    /// blocking answer comes back in.
    fn scoped(modules: &[(&str, Option<&str>)]) -> Manifest {
        Manifest::with_entries(modules.iter().map(|(module, scope)| {
            let entry = PactEntry::new(".", module, format!("{module}/WARLOCK.md"))
                .expect("a relative path inside the root is storable");
            match scope {
                Some(scope) => entry.with_scope(*scope),
                None => entry,
            }
        }))
    }

    /// The sigils a machine holds, in the form the engine takes them.
    fn held(sigils: &[&str]) -> Vec<String> {
        sigils.iter().map(|sigil| (*sigil).to_owned()).collect()
    }

    #[test]
    fn a_scoped_descendant_this_machine_does_not_open_blocks() {
        // The case the whole question is about: the target says nothing, so
        // coverage on the target alone answers "open", and the boundary is one
        // directory down.
        let manifest = scoped(&[
            ("crates", None),
            ("crates/engine", Some("data-plane")),
            ("crates/tui", None),
        ]);

        assert_eq!(
            closed_scopes_at_or_below("crates", ".", &manifest, &held(&["web"]))
                .expect("a path inside the root"),
            ["data-plane"],
        );
        // And the entry carrying it is exactly one an un-pact would drop.
        assert!(
            !modules(&unpact_subtree("crates", ".", &manifest).expect("un-pacts"))
                .contains(&"crates/engine"),
        );
    }

    #[test]
    fn a_scoped_descendant_this_machine_opens_does_not_block() {
        let manifest = scoped(&[("crates", None), ("crates/engine", Some("data-plane"))]);

        for sigils in [
            held(&["data-plane"]),
            held(&["web", "data-plane"]),
            held(&["*"]),
        ] {
            assert!(
                closed_scopes_at_or_below("crates", ".", &manifest, &sigils)
                    .expect("a path inside the root")
                    .is_empty(),
                "{sigils:?} opens it",
            );
        }
    }

    #[test]
    fn the_target_s_own_scope_is_asked_about_too_and_does_not_license_what_is_below() {
        let manifest = scoped(&[
            ("crates", Some("platform")),
            ("crates/engine", Some("data-plane")),
        ]);

        // Holding the target's own scope is not permission over the boundary
        // inside it: these are two questions, not one.
        assert_eq!(
            closed_scopes_at_or_below("crates", ".", &manifest, &held(&["platform"]))
                .expect("a path inside the root"),
            ["data-plane"],
        );
        // "At or below" is at, too: the target's own closed scope is blocking.
        assert_eq!(
            closed_scopes_at_or_below("crates", ".", &manifest, &held(&["data-plane"]))
                .expect("a path inside the root"),
            ["platform"],
        );
    }

    #[test]
    fn an_unscoped_root_buys_nothing_over_the_scopes_below_it() {
        // `unpact .` drops every entry there is, so it is asked about every
        // entry there is — the root's silence is the absence of a statement,
        // not permission over the statements under it.
        let manifest = scoped(&[
            (".", None),
            ("crates/engine", Some("data-plane")),
            ("docs", None),
        ]);

        for directory in [".", "/repo"] {
            assert_eq!(
                closed_scopes_at_or_below(directory, "/repo", &manifest, &held(&["web"]))
                    .expect("a path inside the root"),
                ["data-plane"],
                "{directory}",
            );
        }
        // Nothing scoped below, or all of it held, and the root un-pact is the
        // ordinary one it has always been.
        assert!(
            closed_scopes_at_or_below(".", "/repo", &manifest, &held(&["data-plane"]))
                .expect("a path inside the root")
                .is_empty(),
        );
        assert!(
            closed_scopes_at_or_below(".", "/repo", &pacted(&[".", "crates"]), &[])
                .expect("a path inside the root")
                .is_empty(),
        );
    }

    #[test]
    fn a_sibling_that_shares_a_prefix_carries_no_blocking_scope() {
        // The same segment-wise match `unpact_subtree` uses: what is not
        // dropped cannot block.
        let manifest = scoped(&[
            ("crates/engine", None),
            ("crates/engine-tools", Some("tooling")),
            ("crates/engineering", Some("estimates")),
        ]);

        assert!(
            closed_scopes_at_or_below("crates/engine", ".", &manifest, &[])
                .expect("a path inside the root")
                .is_empty(),
        );
        assert_eq!(
            closed_scopes_at_or_below("crates/engine-tools", ".", &manifest, &[])
                .expect("a path inside the root"),
            ["tooling"],
        );
    }

    #[test]
    fn a_descendant_scope_that_is_not_a_scope_does_not_block() {
        // Read as no scope, exactly as `scope_covering` reads it: one rule for
        // what a boundary is, not two. A valid one beside it still blocks, and
        // the string that is not a scope is not named in the answer.
        let manifest = scoped(&[
            ("crates", None),
            ("crates/engine", Some("Data Plane!")),
            ("crates/tui", Some("")),
            ("crates/store", Some("data-plane")),
        ]);

        assert_eq!(
            closed_scopes_at_or_below("crates", ".", &manifest, &[])
                .expect("a path inside the root"),
            ["data-plane"],
        );
        assert!(
            closed_scopes_at_or_below("crates/engine", ".", &manifest, &[])
                .expect("a path inside the root")
                .is_empty(),
        );
    }

    #[test]
    fn a_path_with_no_manifest_relative_form_is_the_same_error_an_un_pact_gives() {
        let manifest = scoped(&[("crates/engine", Some("data-plane"))]);

        assert!(matches!(
            closed_scopes_at_or_below("/elsewhere/crates", "/repo", &manifest, &[]),
            Err(manifest::Error::PathOutsideRoot { .. })
        ));
    }

    #[test]
    fn un_pacting_a_real_subtree_leaves_every_document_on_disk_untouched() {
        let repo = project();
        let engine = repo.path().join("crates/engine");
        let PactedSubtree { manifest, .. } = pact_subtree(
            &engine,
            repo.path(),
            &pacted(&["crates/tui"]),
            &Canned::filling(),
            &mut Unwatched,
        )
        .expect("pacts");
        assert_eq!(
            modules(&manifest),
            [
                "crates/tui",
                "crates/engine",
                "crates/engine/src",
                "crates/engine/src/inner",
                "crates/engine/tests",
            ],
        );

        let before = snapshot(repo.path());
        assert_eq!(
            before
                .keys()
                .filter(|path| path.ends_with(DOCUMENT_FILE))
                .count(),
            4,
            "four documents were written, and they are what must survive",
        );

        let left = unpact_subtree(&engine, repo.path(), &manifest).expect("un-pacts");

        assert_eq!(modules(&left), ["crates/tui"]);
        assert_eq!(
            snapshot(repo.path()),
            before,
            "un-pacting deletes no file, writes no file and changes no byte — \
             the documents stay, the manifest on disk is the caller's to save",
        );
        for module in [
            "crates/engine",
            "crates/engine/src",
            "crates/engine/src/inner",
            "crates/engine/tests",
        ] {
            let document = from_manifest_path(repo.path(), module).join(DOCUMENT_FILE);
            assert!(document.is_file(), "`{}` was deleted", document.display());
        }
    }

    // Refreshing a subtree: describing what has gone stale and passing over
    // what has not.

    /// A whole-subtree pact over `crates/engine`, insisted on as green: the
    /// starting state of every refresh below, because a refresh only has
    /// something to skip once something is fresh.
    /// A change in every directory on the path up from `inner`, leaving
    /// `tests/` alone.
    ///
    /// One write at the bottom no longer makes a refresh describe the whole
    /// path: [`carried_document`]'s early cutoff stops at the first ancestor
    /// whose request did not move, and an ancestor's request is its own files
    /// plus its children's documents. A fixture that wants a pass in each of
    /// three directories has to give each of them something new to read, which
    /// is what this does — and `tests/` is still left alone, so a run built on
    /// it still has both kinds of directory in it.
    fn restale_the_path(repo: &Path) {
        write(repo, "crates/engine/src/inner/deep.rs", "fn deeper() {}\n");
        write(repo, "crates/engine/src/wider.rs", "fn wider() {}\n");
        write(repo, "crates/engine/outer.rs", "fn outer() {}\n");
    }

    fn refreshable(repo: &Path) -> Manifest {
        let PactedSubtree {
            manifest,
            failures,
            problems,
        } = pact_subtree(
            repo.join("crates/engine"),
            repo,
            &Manifest::new(),
            &Canned::filling(),
            &mut Unwatched,
        )
        .expect("pacts");

        assert!(failures.is_empty(), "{failures:?}");
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(
            modules(&manifest),
            [
                "crates/engine",
                "crates/engine/src",
                "crates/engine/src/inner",
                "crates/engine/tests",
            ],
        );
        for module in modules(&manifest) {
            assert_eq!(
                state(&manifest, repo, module),
                NodeState::PactedFresh,
                "`{module}` starts green, or there is nothing here to skip",
            );
        }
        manifest
    }

    /// Every directory a fake was asked about, named relative to `root`, in the
    /// order it was asked.
    ///
    /// The fixtures below are small enough that no file is ever summarised, so
    /// this is also one entry per pass: what a refresh cost, and on what.
    fn described_by(agent: &Canned, root: &Path) -> Vec<String> {
        let asked: Vec<PathBuf> = agent
            .seen
            .borrow()
            .iter()
            .map(|request| request.directory().to_path_buf())
            .collect();
        relative_to(root, &asked)
    }

    #[test]
    fn a_refresh_describes_every_stale_directory_and_none_it_calls_fresh() {
        let repo = project();
        let engine = repo.path().join("crates/engine");
        let manifest = refreshable(repo.path());

        // Two ways of being stale and one of being fresh, in one subtree: a
        // directory whose content changed, a directory nobody ever pacted, and
        // a `src/` nothing has touched since it was granted.
        write(
            repo.path(),
            "crates/engine/tests/it.rs",
            "#[test] fn works_differently() {}\n",
        );
        write(
            repo.path(),
            "crates/engine/benches/speed.rs",
            "fn bench() {}\n",
        );
        let agent = Canned::filling();

        let PactedSubtree {
            manifest, failures, ..
        } = refresh_subtree(&engine, repo.path(), &manifest, &agent, &mut Unwatched)
            .expect("refreshes");

        assert!(failures.is_empty(), "{failures:?}");
        assert_eq!(
            described_by(&agent, repo.path()),
            [
                "crates/engine/tests",
                "crates/engine/benches",
                "crates/engine",
            ],
            "every directory `decide_state` calls stale — changed, unpacted, \
             and the directory above both — and no directory it calls fresh",
        );
        // Said again from the other side: what was fresh is exactly what was
        // never handed to a pass.
        for skipped in ["crates/engine/src", "crates/engine/src/inner"] {
            assert!(
                !described_by(&agent, repo.path()).contains(&skipped.to_owned()),
                "`{skipped}` hashes to what it was granted for, so it is not \
                 described",
            );
        }
        for module in modules(&manifest) {
            assert_eq!(
                state(&manifest, repo.path(), module),
                NodeState::PactedFresh,
                "`{module}` ends green: what was described earned a grant, what \
                 was skipped kept one",
            );
        }
    }

    #[test]
    fn a_refresh_leaves_the_entry_of_every_directory_it_skipped_as_it_found_it() {
        let repo = project();
        let engine = repo.path().join("crates/engine");
        let before = refreshable(repo.path());

        restale_the_path(repo.path());
        let agent = Canned::filling();

        let PactedSubtree {
            manifest, failures, ..
        } = refresh_subtree(&engine, repo.path(), &before, &agent, &mut Unwatched)
            .expect("refreshes");

        assert!(failures.is_empty(), "{failures:?}");
        let described = [
            "crates/engine/src/inner",
            "crates/engine/src",
            "crates/engine",
        ];
        assert_eq!(described_by(&agent, repo.path()), described);

        let skipped: Vec<&str> = modules(&before)
            .into_iter()
            .filter(|module| !described.contains(module))
            .collect();
        assert_eq!(skipped, ["crates/engine/tests"], "the fixture skips one");
        for module in skipped {
            let was = before.entry(module).expect("pacted before the refresh");
            let now = manifest.entry(module).expect("still pacted after it");
            assert_eq!(
                (
                    now.module(),
                    now.document(),
                    now.granted_hash(),
                    now.granted_at()
                ),
                (
                    was.module(),
                    was.document(),
                    was.granted_hash(),
                    was.granted_at()
                ),
                "`{module}` was skipped, so its entry keeps its module, its \
                 document, its hash and its timestamp",
            );
            assert_eq!(now, was, "and the whole entry with them");
        }

        for module in described {
            assert_eq!(
                state(&manifest, repo.path(), module),
                NodeState::PactedFresh,
                "`{module}` was described and hashed afterwards, so it ends green",
            );
        }
    }

    #[test]
    fn a_change_in_every_directory_costs_one_pass_for_each_of_them_and_no_others() {
        let repo = project();
        let engine = repo.path().join("crates/engine");
        let manifest = refreshable(repo.path());

        restale_the_path(repo.path());
        let agent = Canned::filling();

        let PactedSubtree { failures, .. } =
            refresh_subtree(&engine, repo.path(), &manifest, &agent, &mut Unwatched)
                .expect("refreshes");

        assert!(failures.is_empty(), "{failures:?}");
        assert_eq!(
            described_by(&agent, repo.path()),
            [
                "crates/engine/src/inner",
                "crates/engine/src",
                "crates/engine",
            ],
            "the path from the changed files up to the refreshed root, deepest \
             first, and nothing beside it",
        );
        assert_eq!(
            agent.seen.borrow().len(),
            3,
            "one pass per directory that had something new to read — \
             `crates/engine/tests` is a quarter of the subtree and costs nothing",
        );
    }

    #[test]
    fn a_hand_edited_document_is_never_carried_forward_by_the_cutoff() {
        let repo = project();
        let engine = repo.path().join("crates/engine");
        let manifest = refreshable(repo.path());

        // Nothing about the code moves. Somebody edits the document instead,
        // which is the one thing the request never sees: it is prose, so it
        // reaches no pass and moves no other digest. If the cutoff went on the
        // request alone, this would be carried forward and stamped granted —
        // a person's sentences recorded as a pass's work.
        let document = engine.join("src").join(DOCUMENT_FILE);
        let edited = format!(
            "{}\n\nA sentence a person added by hand.\n",
            fs::read_to_string(&document).expect("the pact wrote a document"),
        );
        fs::write(&document, &edited).expect("writes the edit");

        let agent = Canned::filling();
        let PactedSubtree { failures, .. } =
            refresh_subtree(&engine, repo.path(), &manifest, &agent, &mut Unwatched)
                .expect("refreshes");

        assert!(failures.is_empty(), "{failures:?}");
        assert!(
            described_by(&agent, repo.path()).contains(&"crates/engine/src".to_string()),
            "the edited directory is described again rather than carried: the \
             only road back to fresh is a pass",
        );
        assert_ne!(
            fs::read_to_string(&document).expect("still a document"),
            edited,
            "and the pass overwrote the hand-written sentence rather than \
             granting it",
        );
    }

    #[test]
    fn a_change_below_an_unmoved_request_costs_no_pass_at_the_directory_above_it() {
        let repo = project();
        let engine = repo.path().join("crates/engine");
        let manifest = refreshable(repo.path());

        // One file, at the bottom. Every directory from it to the root is
        // stale — the subtree hash says so and that has not changed — but only
        // the directories whose *request* moved are worth a pass.
        write(
            repo.path(),
            "crates/engine/src/inner/deep.rs",
            "fn deeper() {}\n",
        );
        let agent = Canned::filling();

        let PactedSubtree { failures, .. } =
            refresh_subtree(&engine, repo.path(), &manifest, &agent, &mut Unwatched)
                .expect("refreshes");

        assert!(failures.is_empty(), "{failures:?}");
        assert_eq!(
            described_by(&agent, repo.path()),
            ["crates/engine/src/inner", "crates/engine/src"],
            "the pass runs where the file changed, and at the parent whose \
             child document changed under it — and stops at `crates/engine`, \
             whose own files and children's documents are what they were",
        );

        // The cutoff is a saving, never a downgrade: the directory that paid
        // for no pass is as green as the ones that did, because its document
        // was granted against the subtree as it now stands.
        for module in [
            "crates/engine",
            "crates/engine/src",
            "crates/engine/src/inner",
        ] {
            assert_eq!(
                state(&manifest, repo.path(), module),
                NodeState::PactedStale,
                "`{module}` is stale before the refresh",
            );
        }
    }

    #[test]
    fn a_refresh_with_nothing_stale_runs_no_pass_and_changes_no_entry() {
        let repo = project();
        let engine = repo.path().join("crates/engine");
        let before = refreshable(repo.path());
        let agent = Canned::filling();
        let mut observer = Watching::patient();

        let PactedSubtree {
            manifest,
            failures,
            problems,
        } = refresh_subtree(&engine, repo.path(), &before, &agent, &mut observer)
            .expect("refreshes");

        assert!(
            agent.seen.borrow().is_empty(),
            "nothing is stale, so nothing is described and no pass is bought",
        );
        assert!(
            observer.calls(repo.path()).is_empty(),
            "and there is no directory to announce: {:?}",
            observer.calls(repo.path()),
        );
        assert!(failures.is_empty(), "{failures:?}");
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(
            manifest, before,
            "the manifest comes back with every entry, every grant and every \
             timestamp exactly as it went in",
        );
    }

    #[test]
    fn the_total_announced_counts_the_directories_a_refresh_will_describe() {
        let repo = project();
        let engine = repo.path().join("crates/engine");
        let manifest = refreshable(repo.path());
        write(
            repo.path(),
            "crates/engine/tests/it.rs",
            "#[test] fn works_differently() {}\n",
        );
        let mut observer = Watching::patient();

        let PactedSubtree { failures, .. } = refresh_subtree(
            &engine,
            repo.path(),
            &manifest,
            &Canned::filling(),
            &mut observer,
        )
        .expect("refreshes");

        assert!(failures.is_empty(), "{failures:?}");
        assert_eq!(
            observer.calls(repo.path()),
            [
                ("crates/engine/tests".to_owned(), 1, 2),
                ("crates/engine".to_owned(), 2, 2),
            ],
            "two of two: the directories this run will actually describe, not \
             the four in the subtree",
        );
        assert_eq!(
            observer.done(repo.path()),
            ["crates/engine/tests", "crates/engine"],
            "and each is announced documented as its pass delivers, exactly as \
             in a pact",
        );
    }

    /// Only on unix, because there is no portable way to make a file
    /// unreadable. What is under test — that a directory with no hash is a
    /// directory to describe — is not platform-specific.
    #[cfg(unix)]
    #[test]
    fn a_directory_whose_hash_fails_while_staleness_is_decided_is_described_anyway() {
        use std::os::unix::fs::PermissionsExt as _;

        let repo = project();
        let engine = repo.path().join("crates/engine");
        let manifest = refreshable(repo.path());

        let unreadable = engine.join("tests").join("it.rs");
        fs::set_permissions(&unreadable, fs::Permissions::from_mode(0o000)).expect("chmods");
        if fs::read(&unreadable).is_ok() {
            // Running as root: no file is unreadable, so there is nothing here
            // to assert against.
            return;
        }

        let agent = Canned::filling();
        let PactedSubtree {
            manifest, failures, ..
        } = refresh_subtree(&engine, repo.path(), &manifest, &agent, &mut Unwatched)
            .expect("a hash nobody can take is a directory to describe, not an error");

        assert_eq!(
            described_by(&agent, repo.path()),
            ["crates/engine/tests", "crates/engine"],
            "no hash is no answer to `is this still the content it was granted \
             for`, so both directories the unreadable file sits under are \
             described",
        );
        // And then it plays out exactly as the module docs say it does: phase
        // two hashes them again, that hash fails again, and each lands as a
        // `Failure::Hash` with an ungranted entry — yellow, with a pass paid
        // for it, which is the honest outcome for a directory something is
        // really wrong with.
        for module in ["crates/engine/tests", "crates/engine"] {
            let entry = manifest.entry(module).expect("described, so pacted");
            assert_eq!(
                entry.granted_hash(),
                None,
                "`{module}` was described and still has no hash to grant against",
            );
        }
        assert_eq!(failures.len(), 2, "{failures:?}");
        assert!(
            failures
                .iter()
                .all(|failure| matches!(failure, Failure::Hash { .. })),
            "the documents were written; only the hashes failed: {failures:?}",
        );
        assert_eq!(
            state(&manifest, repo.path(), "crates/engine/src"),
            NodeState::PactedFresh,
            "the part of the subtree nothing is wrong with is skipped and stays \
             green",
        );

        fs::set_permissions(&unreadable, fs::Permissions::from_mode(0o644)).expect("chmods back");
    }

    #[test]
    fn a_refresh_whose_passes_all_fail_removes_no_entry_and_drops_no_grant() {
        let repo = project();
        let engine = repo.path().join("crates/engine");
        let before = refreshable(repo.path());
        restale_the_path(repo.path());

        let PactedSubtree {
            manifest, failures, ..
        } = refresh_subtree(
            &engine,
            repo.path(),
            &before,
            &Fails(|| agent::Error::EmptyOutput),
            &mut Unwatched,
        )
        .expect("a refused pass does not fail the refresh");

        assert_eq!(
            failures.len(),
            3,
            "one per stale directory, and none for the fresh one nobody asked \
             about: {failures:?}",
        );
        assert!(
            failures
                .iter()
                .all(|failure| matches!(failure, Failure::Document { .. })),
            "{failures:?}",
        );
        assert_eq!(
            manifest, before,
            "a refresh that could not re-describe anything leaves the manifest \
             exactly as stale as it found it: no entry removed, no grant \
             dropped",
        );
        assert_eq!(
            state(&manifest, repo.path(), "crates/engine"),
            NodeState::PactedStale,
            "still yellow, which is what a stale directory nobody managed to \
             re-describe should be",
        );
    }

    #[test]
    fn a_cancelled_refresh_keeps_what_it_described_and_leaves_the_rest_alone() {
        let repo = project();
        let engine = repo.path().join("crates/engine");
        let before = refreshable(repo.path());
        write(
            repo.path(),
            "crates/engine/src/inner/deep.rs",
            "fn deeper() {}\n",
        );
        let agent = Canned::filling();
        let mut observer = Watching::stopping_after(1);

        let PactedSubtree {
            manifest, failures, ..
        } = refresh_subtree(&engine, repo.path(), &before, &agent, &mut observer)
            .expect("a refresh somebody stopped is not a refresh that failed");

        assert_eq!(
            observer.calls(repo.path()),
            [
                ("crates/engine/src/inner".to_owned(), 1, 3),
                ("crates/engine/src".to_owned(), 2, 3),
            ],
            "the second directory was offered and turned down, and there was no \
             third question",
        );
        assert!(
            failures.is_empty(),
            "nothing went wrong — fewer directories were asked for: {failures:?}",
        );
        assert_eq!(
            described_by(&agent, repo.path()),
            ["crates/engine/src/inner"],
            "and the cancel cost no pass at all",
        );

        assert_eq!(
            modules(&manifest),
            modules(&before),
            "a cancelled refresh drops no entry either",
        );
        assert_eq!(
            state(&manifest, repo.path(), "crates/engine/src/inner"),
            NodeState::PactedFresh,
            "what finished before the cancel is granted like any other",
        );
        for untouched in ["crates/engine/src", "crates/engine"] {
            assert_eq!(
                manifest.entry(untouched),
                before.entry(untouched),
                "`{untouched}` is at or past the cancel, so it keeps the entry \
                 the refresh found",
            );
            assert_eq!(
                state(&manifest, repo.path(), untouched),
                NodeState::PactedStale,
                "which is to say it is exactly as stale as it was",
            );
        }
    }

    #[test]
    fn a_refresh_above_a_failed_pass_records_the_ancestor_without_a_grant() {
        let repo = project();
        let engine = repo.path().join("crates/engine");
        let before = refreshable(repo.path());
        write(
            repo.path(),
            "crates/engine/src/inner/deep.rs",
            "fn deeper() {}\n",
        );

        let PactedSubtree {
            manifest, failures, ..
        } = refresh_subtree(
            &engine,
            repo.path(),
            &before,
            &FailsFor {
                directory: engine.join("src").join("inner"),
            },
            &mut Unwatched,
        )
        .expect("one refused pass does not fail the refresh");

        assert_eq!(failures.len(), 1, "{failures:?}");
        assert_eq!(failures[0].directory(), engine.join("src").join("inner"));
        assert_eq!(
            manifest.entry("crates/engine/src/inner"),
            before.entry("crates/engine/src/inner"),
            "the directory whose pass failed keeps the entry it had — a refresh \
             that could not re-describe it does not un-pact it",
        );
        for above in ["crates/engine/src", "crates/engine"] {
            assert_eq!(
                manifest
                    .entry(above)
                    .expect("documented, so pacted")
                    .granted_hash(),
                None,
                "`{above}` has an undocumented descendant, so it earned no \
                 grant — partial completion reads in a refresh exactly as it \
                 does in a pact",
            );
            assert_eq!(state(&manifest, repo.path(), above), NodeState::PactedStale);
        }
        assert_eq!(
            manifest.entry("crates/engine/tests"),
            before.entry("crates/engine/tests"),
            "and the fresh directory beside all of it is untouched",
        );
    }

    // The bytes themselves. Everything above asserts about entries; this
    // asserts about the file, so that a change of shape — a key that moves, a
    // blank line that appears, an entry that is appended where it used to be
    // replaced in place — fails the build instead of passing quietly.

    /// What `module` hashes to right now, as the manifest would store it.
    fn hash_of(repo: &Path, module: &str) -> String {
        subtree_hash(from_manifest_path(repo, module)).expect("the subtree hashes")
    }

    /// The carry digest recorded against `module`, taken from the fixture for
    /// the same reason [`hash_of`] is: pasting a digest proves only that
    /// somebody pasted it, and taking it here makes the literal insist that
    /// each entry records the digest of its own directory as it stands.
    fn carry_of(repo: &Path, module: &str) -> String {
        super::carry_hash(&from_manifest_path(repo, module)).expect("the directory digests")
    }

    /// The `granted_at` recorded against `module`, which a run mints once for
    /// the whole of itself.
    fn granted_at_of(manifest: &Manifest, module: &str) -> String {
        manifest
            .entry(module)
            .unwrap_or_else(|| panic!("`{module}` is pacted"))
            .granted_at()
            .unwrap_or_else(|| panic!("`{module}` is granted"))
            .to_owned()
    }

    /// Every byte the manifest should hold after the pact below: the entry the
    /// starting manifest already had for a covered module re-granted where it
    /// sat, the entry for the module the pact never covered carried through
    /// with the grant it came in with, and the three modules the pact gained
    /// appended after both in stored-path order.
    ///
    /// `granted_at` is the run's timestamp, read back off what it produced —
    /// the one thing here that is not fixed. The hashes are, so they are taken
    /// from the fixture rather than pasted, which also makes this insist that
    /// each entry records the hash of its own subtree as it stands now.
    fn expected_after_the_pact(repo: &Path, granted_at: &str) -> String {
        format!(
            "version = 1\n\
             \n\
             [[pact]]\n\
             module = \"crates/engine/src\"\n\
             document = \"crates/engine/src/WARLOCK.md\"\n\
             granted_hash = \"{src}\"\n\
             granted_at = \"{granted_at}\"\n\
             carry_hash = \"{src_carry}\"\n\
             \n\
             [[pact]]\n\
             module = \"crates/tui\"\n\
             document = \"crates/tui/WARLOCK.md\"\n\
             granted_hash = \"othercrate\"\n\
             granted_at = \"2026-02-02T00:00:00Z\"\n\
             \n\
             [[pact]]\n\
             module = \"crates/engine\"\n\
             document = \"crates/engine/WARLOCK.md\"\n\
             granted_hash = \"{root}\"\n\
             granted_at = \"{granted_at}\"\n\
             carry_hash = \"{root_carry}\"\n\
             \n\
             [[pact]]\n\
             module = \"crates/engine/src/inner\"\n\
             document = \"crates/engine/src/inner/WARLOCK.md\"\n\
             granted_hash = \"{inner}\"\n\
             granted_at = \"{granted_at}\"\n\
             carry_hash = \"{inner_carry}\"\n\
             \n\
             [[pact]]\n\
             module = \"crates/engine/tests\"\n\
             document = \"crates/engine/tests/WARLOCK.md\"\n\
             granted_hash = \"{tests}\"\n\
             granted_at = \"{granted_at}\"\n\
             carry_hash = \"{tests_carry}\"\n",
            root = hash_of(repo, "crates/engine"),
            src = hash_of(repo, "crates/engine/src"),
            inner = hash_of(repo, "crates/engine/src/inner"),
            tests = hash_of(repo, "crates/engine/tests"),
            root_carry = carry_of(repo, "crates/engine"),
            src_carry = carry_of(repo, "crates/engine/src"),
            inner_carry = carry_of(repo, "crates/engine/src/inner"),
            tests_carry = carry_of(repo, "crates/engine/tests"),
        )
    }

    /// Every byte the manifest should hold after the refresh below: the same
    /// five entries in the same five places, two of them re-granted at
    /// `refreshed_at` because their content moved, three still carrying
    /// `pacted_at` because the refresh never described them.
    fn expected_after_the_refresh(repo: &Path, pacted_at: &str, refreshed_at: &str) -> String {
        format!(
            "version = 1\n\
             \n\
             [[pact]]\n\
             module = \"crates/engine/src\"\n\
             document = \"crates/engine/src/WARLOCK.md\"\n\
             granted_hash = \"{src}\"\n\
             granted_at = \"{pacted_at}\"\n\
             carry_hash = \"{src_carry}\"\n\
             \n\
             [[pact]]\n\
             module = \"crates/tui\"\n\
             document = \"crates/tui/WARLOCK.md\"\n\
             granted_hash = \"othercrate\"\n\
             granted_at = \"2026-02-02T00:00:00Z\"\n\
             \n\
             [[pact]]\n\
             module = \"crates/engine\"\n\
             document = \"crates/engine/WARLOCK.md\"\n\
             granted_hash = \"{root}\"\n\
             granted_at = \"{refreshed_at}\"\n\
             carry_hash = \"{root_carry}\"\n\
             \n\
             [[pact]]\n\
             module = \"crates/engine/src/inner\"\n\
             document = \"crates/engine/src/inner/WARLOCK.md\"\n\
             granted_hash = \"{inner}\"\n\
             granted_at = \"{pacted_at}\"\n\
             carry_hash = \"{inner_carry}\"\n\
             \n\
             [[pact]]\n\
             module = \"crates/engine/tests\"\n\
             document = \"crates/engine/tests/WARLOCK.md\"\n\
             granted_hash = \"{tests}\"\n\
             granted_at = \"{refreshed_at}\"\n\
             carry_hash = \"{tests_carry}\"\n",
            root = hash_of(repo, "crates/engine"),
            src = hash_of(repo, "crates/engine/src"),
            inner = hash_of(repo, "crates/engine/src/inner"),
            tests = hash_of(repo, "crates/engine/tests"),
            root_carry = carry_of(repo, "crates/engine"),
            src_carry = carry_of(repo, "crates/engine/src"),
            inner_carry = carry_of(repo, "crates/engine/src/inner"),
            tests_carry = carry_of(repo, "crates/engine/tests"),
        )
    }

    #[test]
    fn a_pact_and_a_refresh_over_a_granted_manifest_write_these_exact_bytes() {
        let repo = project();
        let engine = repo.path().join("crates/engine");

        // A manifest that already says something: one entry for a directory the
        // pact will cover, carrying a grant from a run that is not this one, and
        // one entry for a directory it will not cover at all. The covered one is
        // written first so that keeping its position is visible in the bytes —
        // it must stay at the top with the newly gained entries below it, not be
        // dropped and re-appended in sorted order.
        let entry = |module: &str| {
            PactEntry::new(repo.path(), module, format!("{module}/{DOCUMENT_FILE}"))
                .expect("the fixture's paths are spellable")
        };
        let before = Manifest::with_entries([
            entry("crates/engine/src").with_grant("stalehash", "2026-01-01T00:00:00Z"),
            entry("crates/tui").with_grant("othercrate", "2026-02-02T00:00:00Z"),
        ]);

        let PactedSubtree {
            manifest,
            failures,
            problems,
        } = pact_subtree(
            &engine,
            repo.path(),
            &before,
            &Canned::filling(),
            &mut Unwatched,
        )
        .expect("pacts");
        assert!(failures.is_empty(), "{failures:?}");
        assert!(problems.is_empty(), "{problems:?}");

        // The one thing a run does not decide for itself: the clock. Read back
        // off the manifest rather than guessed at, and read back once, so the
        // literal below still insists that every entry the run granted carries
        // the same timestamp.
        let pacted_at = granted_at_of(&manifest, "crates/engine");

        assert_eq!(
            manifest.to_toml_string().expect("serialises"),
            expected_after_the_pact(repo.path(), &pacted_at),
            "the whole file, not a fragment of it",
        );

        // Now a refresh over that manifest, with one file moved under `tests/`.
        // Two directories go stale — `tests` and the `crates/engine` above it —
        // and everything else, covered or not, is carried through byte for byte,
        // grants and positions and all.
        write(
            repo.path(),
            "crates/engine/tests/it.rs",
            "#[test] fn works_differently() {}\n",
        );

        let PactedSubtree {
            manifest,
            failures,
            problems,
        } = refresh_subtree(
            &engine,
            repo.path(),
            &manifest,
            &Canned::filling(),
            &mut Unwatched,
        )
        .expect("refreshes");
        assert!(failures.is_empty(), "{failures:?}");
        assert!(problems.is_empty(), "{problems:?}");

        // Read the same way, and deliberately not asserted to differ from
        // `pacted_at`: the clock is only to the second, so two runs in one test
        // very often mint the same string. What the literal below pins is which
        // entries got the refresh's timestamp and which kept the pact's, and
        // that reads the same either way.
        let refreshed_at = granted_at_of(&manifest, "crates/engine");

        assert_eq!(
            manifest.to_toml_string().expect("serialises"),
            expected_after_the_refresh(repo.path(), &pacted_at, &refreshed_at),
            "the whole file again: two entries re-granted where they sat, three \
             carried through untouched",
        );
    }

    // Scopes: what a run may not touch, and what un-pacting takes with it.
    //
    // Every test below passes by construction — a scope lives on an entry, a
    // run hands over `Outcome`s, and an `Outcome` has nowhere to put one — so
    // these are regression guards rather than proofs of new code. They are what
    // fails loudly if somebody ever widens the outcome type, which is the
    // mistake worth catching early: a run that can write a scope is a run that
    // can quietly move a boundary somebody drew on purpose.

    #[test]
    fn a_refresh_leaves_every_scope_exactly_as_it_found_it() {
        let repo = project();
        let engine = repo.path().join("crates/engine");
        let before = with_scopes(
            &refreshable(repo.path()),
            &[
                ("crates/engine", "engine"),
                ("crates/engine/src", "data-plane"),
                ("crates/engine/tests", "harness"),
            ],
        );

        // A change in each directory on the path up, so the refresh describes
        // all three and skips `tests/` — both kinds of directory in one run.
        restale_the_path(repo.path());
        let agent = Canned::filling();

        let PactedSubtree {
            manifest, failures, ..
        } = refresh_subtree(&engine, repo.path(), &before, &agent, &mut Unwatched)
            .expect("refreshes");

        assert!(failures.is_empty(), "{failures:?}");
        assert_eq!(
            described_by(&agent, repo.path()),
            [
                "crates/engine/src/inner",
                "crates/engine/src",
                "crates/engine",
            ],
            "two scoped directories were described and one scoped directory was \
             skipped, or this proves nothing about either",
        );
        assert_eq!(
            scopes(&manifest),
            scopes(&before),
            "a refresh rewrites documents, hashes and timestamps, and no scope: \
             described and skipped directories alike keep the boundary somebody \
             drew on them",
        );
    }

    #[test]
    fn a_cancelled_run_keeps_every_scope() {
        let repo = project();
        let engine = repo.path().join("crates/engine");
        let before = with_scopes(
            &refreshable(repo.path()),
            &[
                ("crates/engine", "engine"),
                ("crates/engine/src", "data-plane"),
                ("crates/engine/src/inner", "deep"),
                ("crates/engine/tests", "harness"),
            ],
        );
        write(
            repo.path(),
            "crates/engine/src/inner/deep.rs",
            "fn deeper() {}\n",
        );
        let agent = Canned::filling();
        // The same cancellation the refresh tests above use: one directory
        // described, the next offered and turned down, the rest never asked.
        let mut observer = Watching::stopping_after(1);

        let PactedSubtree {
            manifest, failures, ..
        } = refresh_subtree(&engine, repo.path(), &before, &agent, &mut observer)
            .expect("a refresh somebody stopped is not a refresh that failed");

        assert!(failures.is_empty(), "{failures:?}");
        assert_eq!(
            described_by(&agent, repo.path()),
            ["crates/engine/src/inner"],
            "one scoped directory got its pass, and three scoped directories \
             were cut off mid-run",
        );
        assert_eq!(
            scopes(&manifest),
            scopes(&before),
            "stopping a run part way through takes out documents and grants \
             nobody asked for, and no boundary anybody drew",
        );
    }

    #[test]
    fn a_partly_completed_run_keeps_every_scope() {
        let repo = project();
        let engine = repo.path().join("crates/engine");
        let before = with_scopes(
            &refreshable(repo.path()),
            &[
                ("crates/engine", "engine"),
                ("crates/engine/src", "data-plane"),
                ("crates/engine/src/inner", "deep"),
                ("crates/engine/tests", "harness"),
            ],
        );
        write(
            repo.path(),
            "crates/engine/src/inner/deep.rs",
            "fn deeper() {}\n",
        );

        // One pass refuses, so the two directories above it are described and
        // recorded without a grant: partial completion, the ungranted-entry
        // path through phase two.
        let PactedSubtree {
            manifest, failures, ..
        } = refresh_subtree(
            &engine,
            repo.path(),
            &before,
            &FailsFor {
                directory: engine.join("src").join("inner"),
            },
            &mut Unwatched,
        )
        .expect("one refused pass does not fail the refresh");

        assert_eq!(failures.len(), 1, "{failures:?}");
        for above in ["crates/engine/src", "crates/engine"] {
            let entry = manifest.entry(above).expect("documented, so pacted");
            assert_eq!(
                entry.granted_hash(),
                None,
                "`{above}` really did come out of the run ungranted, or the \
                 partial-completion path was never taken",
            );
        }
        assert_eq!(
            scopes(&manifest),
            scopes(&before),
            "the grant is a field a run owns and clears; the scope is not, so a \
             directory can go yellow without its boundary moving",
        );
    }

    /// The per-file cap as an index, for the tests that slice a fixture at it.
    fn cap() -> usize {
        usize::try_from(PER_FILE_BYTE_CAP).expect("the cap fits an index")
    }

    /// Insist `path` still holds exactly `contents`: viewing a file writes no
    /// byte of it, whichever way the view turned out.
    fn untouched(path: &Path, contents: &[u8]) {
        assert_eq!(
            fs::read(path).expect("the file is still there"),
            contents,
            "`{}` was changed by being looked at",
            path.display(),
        );
    }

    #[test]
    fn a_view_of_an_ordinary_file_is_the_whole_of_it() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let contents = "# engine\n\nThe core.\n";
        let path = write(dir.path(), "WARLOCK.md", contents);

        let Viewed { text, cut } = view_file(&path).expect("an ordinary file reads");

        assert_eq!(text, contents);
        assert!(!cut, "a file under the cap is not cut");
        untouched(&path, contents.as_bytes());
    }

    #[test]
    fn an_empty_file_is_an_empty_view_rather_than_a_failure() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let path = write(dir.path(), "empty.rs", "");

        let Viewed { text, cut } = view_file(&path).expect("an empty file reads");

        assert!(text.is_empty(), "{text:?}");
        assert!(!cut, "there is nothing past nothing");
        untouched(&path, b"");
    }

    #[test]
    fn a_file_over_the_cap_is_cut_at_the_cap_and_says_so() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let mut contents = filler(PER_FILE_BYTE_CAP);
        contents.extend_from_slice(b"and there is more after the cap\n");
        let path = write(dir.path(), "Cargo.lock", &contents);

        let Viewed { text, cut } = view_file(&path).expect("a huge file still reads");

        assert!(cut, "the file goes on past what was read");
        assert_eq!(
            byte_count(text.len()),
            PER_FILE_BYTE_CAP,
            "exactly the cap, not the cap plus the byte that proved there was more",
        );
        assert_eq!(
            text.as_bytes(),
            &contents[..cap()],
            "the first {PER_FILE_BYTE_CAP} bytes of the file, verbatim",
        );
        assert!(
            !text.contains("cut") && !text.ends_with('\u{2026}'),
            "the cut is a fact on `Viewed`, never a sentence in the text",
        );
        untouched(&path, &contents);
    }

    #[test]
    fn a_view_is_the_bytes_the_same_file_puts_in_a_request() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let contents = "//! Core engine.\n\nfn describe() {}\n";
        let path = write(dir.path(), "lib.rs", contents);

        let request = request_for(dir.path());
        let carried = file(&request, "lib.rs")
            .bytes()
            .expect("a file under the cap is sent whole");
        let Viewed { text, cut } = view_file(&path).expect("reads");

        // Only meaningful at or under the cap, which is the whole of the
        // parity: `gather_request` leaves an over-cap file's contents out
        // entirely — a name and a size, no bytes at all — so there is nothing
        // above the cap for a view to be equal to. What a reader is shown of a
        // file a model reads is exactly what the model reads.
        assert_eq!(text.as_bytes(), carried);
        assert!(!cut);
        untouched(&path, contents.as_bytes());
    }

    #[test]
    fn a_file_of_exactly_the_cap_is_whole_and_uncut_on_both_sides_of_the_seam() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let contents = filler(PER_FILE_BYTE_CAP);
        let path = write(dir.path(), "generated.rs", &contents);

        let request = request_for(dir.path());
        let Viewed { text, cut } = view_file(&path).expect("reads");

        assert!(
            !cut,
            "the cap is a ceiling the file reaches, not one it passes"
        );
        assert_eq!(text.as_bytes(), &contents[..]);
        assert_eq!(
            file(&request, "generated.rs").bytes(),
            Some(text.as_bytes()),
            "the last size at which a reader and a model see the same file",
        );
        untouched(&path, &contents);
    }

    #[test]
    fn the_cut_drops_a_split_character_rather_than_replacing_it() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        // The last character starting inside the cap is two bytes long and its
        // second byte is one past it, so cutting on the byte count alone lands
        // in the middle of it.
        let mut contents = filler(PER_FILE_BYTE_CAP - 1);
        contents.extend_from_slice("é and more after the cap\n".as_bytes());
        let path = write(dir.path(), "notes.md", &contents);

        let Viewed { text, cut } = view_file(&path).expect("reads");

        assert!(cut);
        assert_eq!(
            byte_count(text.len()),
            PER_FILE_BYTE_CAP - 1,
            "the split character costs itself and nothing else",
        );
        assert!(
            !text.contains('\u{fffd}'),
            "a cut inside a character is never patched up with a replacement one",
        );
        assert!(
            String::from_utf8(contents.clone())
                .expect("the fixture is text")
                .starts_with(&text),
            "what came back is still a prefix of the file",
        );
        untouched(&path, &contents);
    }

    #[test]
    fn bytes_that_are_not_text_come_back_as_no_text_at_all() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let contents = not_text(4 * 1024);
        let path = write(dir.path(), "logo.png", &contents);

        let error = view_file(&path).expect_err("there is no text in it to show");

        assert!(
            matches!(error, Unviewable::NotText { .. }),
            "not the filesystem's fault: {error:?}",
        );
        assert_eq!(error.path(), path);
        let said = error.to_string();
        assert!(said.contains(&path.display().to_string()), "{said}");
        assert!(
            !said.contains('\u{fffd}'),
            "not even the message shows a replacement character: {said}",
        );
        assert!(error.source().is_some(), "the UTF-8 error is kept");
        untouched(&path, &contents);
    }

    #[test]
    fn a_file_that_is_not_there_is_a_read_failure_naming_it() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let path = dir.path().join("gone.rs");

        let error = view_file(&path).expect_err("nothing to read");

        assert!(matches!(error, Unviewable::Unreadable { .. }), "{error:?}");
        assert_eq!(error.path(), path);
        assert!(
            error.to_string().contains(&path.display().to_string()),
            "{error}",
        );
    }

    /// Only on unix, because there is no portable way to make a file
    /// unreadable. What is under test — that permission to read is the
    /// filesystem saying no rather than a file that is not text — is not
    /// platform-specific.
    #[cfg(unix)]
    #[test]
    fn a_file_that_may_not_be_read_is_a_read_failure_naming_it() {
        use std::os::unix::fs::PermissionsExt as _;

        let dir = tempfile::tempdir().expect("a temporary directory");
        let contents = "# engine\n\nThe core.\n";
        let path = write(dir.path(), "WARLOCK.md", contents);
        fs::set_permissions(&path, fs::Permissions::from_mode(0o000)).expect("chmods");
        if fs::read(&path).is_ok() {
            // Running as root: no file is unreadable, so there is nothing here
            // to assert against.
            return;
        }

        let error = view_file(&path).expect_err("the permissions say no");

        assert!(matches!(error, Unviewable::Unreadable { .. }), "{error:?}");
        assert_eq!(error.path(), path);
        assert!(
            error.to_string().contains(&path.display().to_string()),
            "{error}",
        );

        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).expect("chmods back");
        untouched(&path, contents.as_bytes());
    }

    #[test]
    fn a_directory_is_a_read_failure_naming_it() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        write(dir.path(), "src/lib.rs", "//! Core engine.\n");
        let path = dir.path().join("src");

        let error = view_file(&path).expect_err("a directory has no text");

        assert!(
            matches!(error, Unviewable::Unreadable { .. }),
            "a directory is the filesystem saying no, not a file that is not text: {error:?}",
        );
        assert_eq!(error.path(), path);
        assert!(
            error.to_string().contains(&path.display().to_string()),
            "{error}",
        );
        untouched(&path.join("lib.rs"), b"//! Core engine.\n");
    }

    #[test]
    fn a_second_view_reads_the_file_as_it_is_now() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let path = write(dir.path(), "WARLOCK.md", "# engine\n");

        let first = view_file(&path).expect("reads").text;
        fs::write(&path, "# engine\n\nRewritten.\n").expect("rewrites the file");
        let second = view_file(&path).expect("reads again").text;

        assert_eq!(first, "# engine\n");
        assert_eq!(
            second, "# engine\n\nRewritten.\n",
            "every view is a read from disk, so nothing is cached to go stale",
        );
    }
}
