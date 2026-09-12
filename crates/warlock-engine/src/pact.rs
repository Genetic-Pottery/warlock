use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsStr;
use std::fmt;
use std::fs;
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::str::Utf8Error;

use ignore::WalkBuilder;

use crate::document::{self, ATTEMPTS, Accepted, Defect, Fill};
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

pub(crate) const MANIFEST_DIR: &str = ".warlock";

pub(crate) const DOCUMENT_FILE: &str = "WARLOCK.md";

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
        repairs,
    } = describe_and_grant(
        &directories,
        root,
        &BTreeMap::new(),
        AboveFailure::Describe,
        agent,
        observer,
    );

    Ok(PactedSubtree {
        manifest: rewrite(manifest, &directories, root, outcomes),
        failures,
        problems,
        repairs,
    })
}

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

    // Read here, where the manifest is, so `describe_and_grant` keeps knowing
    // nothing about manifests: it is handed digests the same way it is handed
    // directories.
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
        repairs,
    } = describe_and_grant(&stale, root, &recorded, AboveFailure::Skip, agent, observer);

    // The empty slice, not `stale`: `rewrite` drops an entry only where the run
    // covered its module and earned nothing, and a refresh that could not
    // re-describe a directory must leave it stale rather than un-pact it.
    Ok(PactedSubtree {
        manifest: rewrite(manifest, &[], root, outcomes),
        failures,
        problems,
        repairs,
    })
}

// Neither way of having no answer is an error: an unnameable path and an
// unhashable directory both fall to the stale side, because calling either
// fresh would be a false green earned on no evidence.
fn is_fresh(manifest: &Manifest, root: &Path, directory: &Path) -> bool {
    let entry = to_manifest_path(root, directory)
        .ok()
        .and_then(|module| manifest.entry(&module));
    let Ok(computed) = subtree_hash(directory) else {
        return false;
    };
    decide_state(entry, &computed) == NodeState::PactedFresh
}

#[derive(Debug)]
struct Described {
    outcomes: BTreeMap<String, Outcome>,
    failures: Vec<Failure>,
    problems: Vec<Problem>,
    repairs: Vec<Repaired>,
}

// The narrow thing a run is allowed to say about an entry. A scope is a
// person's and a run has nowhere to put one, which is why `rewrite` takes these
// rather than whole `PactEntry`s: no way to hand it one, no way to erase a
// field of one.
#[derive(Debug)]
struct Outcome {
    module: String,
    document: String,
    grant: Option<Grant>,
}

// `carry` is `None` where the digest could not be taken, rather than the grant
// being dropped: no shortcut next time, but still a grant.
#[derive(Debug)]
struct Grant {
    hash: String,
    at: String,
    carry: Option<String>,
}

impl Outcome {
    fn apply(self, entry: &mut PactEntry) {
        entry.overwrite_run_fields(
            self.module,
            self.document,
            self.grant
                .map(|Grant { hash, at, carry }| (hash, at, carry)),
        );
    }

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

// The early cutoff. Every way of not knowing runs the pass — no recorded
// digest, none takeable this run, `None` never matching — so a manifest written
// before the field existed costs one full refresh and no correctness.
//
// The third condition is the one to keep: a digest match says nothing about
// whether the document survived, and a grant pointing at a file nobody wrote is
// the false green this module exists to avoid.
//
// Measured against a real model on a three-level fixture: prose edits reach no
// request and so cost no passes, while a code edit still costs the full path to
// the root — a pass rewords its whole document every run, and a moved child
// document is a moved parent request.
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

// What a run does with a directory above a failure. Neither can be granted —
// phase two refuses both the same way — so this is only ever about whether the
// pass is worth paying for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AboveFailure {
    // A refresh. The directory already has a document, and the one a pass would
    // write now does not survive: the directory below has to be described
    // again, a described child is a moved parent request, and the next run that
    // gets that far overwrites this answer with another. So the pass buys a
    // document nobody keeps and a grant nobody gets, and the run says which
    // failure took the directory down instead.
    Skip,
    // A pact. There is no document on disk to fall back on, so the one a pass
    // writes now is the only one this directory has, ungranted or not — which
    // is worth the pass in a way re-describing an existing document is not.
    Describe,
}

// The one prefix test the rest of this module asks its question through:
// whether a directory this run failed to document lies at or below `pacted`.
// `starts_with` is component-wise, so `src` never counts as an ancestor of
// `src-tests`.
//
// Four callers ask it — whether to skip a pass, whether to announce a pass
// done, whether a carry counts as unchanged, and whether a grant is owed — and
// they have to agree, because a directory announced done and then left
// ungranted is a front end painting green over yellow.
fn failure_below<'missing>(
    undocumented: &'missing [PathBuf],
    pacted: &Path,
) -> Option<&'missing PathBuf> {
    undocumented
        .iter()
        .find(|missing| missing.starts_with(pacted))
}

// The two phases must not be folded into one loop. A directory's subtree hash
// covers its children's `WARLOCK.md`, so a per-directory write-hash-grant loop
// grants a parent a hash the very next child write invalidates, and ends with a
// subtree yellow everywhere but its deepest leaves. Phase two therefore starts
// only once every document phase one was going to write is on disk.
fn describe_and_grant(
    directories: &[PathBuf],
    root: &Path,
    recorded: &BTreeMap<PathBuf, String>,
    above_failure: AboveFailure,
    agent: &dyn Agent,
    observer: &mut dyn Observer,
) -> Described {
    let mut failures = Vec::new();
    let mut problems = Vec::new();
    let mut repairs = Vec::new();

    let total = directories.len();
    let mut documents = BTreeMap::new();
    let mut carries: BTreeMap<PathBuf, Option<String>> = BTreeMap::new();
    let mut undocumented = Vec::new();
    for (index, pacted) in directories.iter().enumerate() {
        // Asked before the pass, so a front end names the directory being
        // worked rather than the one that just finished, and a cancel arriving
        // now costs no pass at all.
        if observer.starting(pacted, index + 1, total) == Pacting::Stop {
            // Offered and turned down, recorded the way a failure is so phase
            // two's partial rule need know nothing about cancellation — but
            // with no `Failure` beside it, because nothing went wrong.
            undocumented.extend(directories[index..].iter().cloned());
            break;
        }

        // Asked before anything is spent, and by the same prefix test phase two
        // grants by: a directory above something this run failed to document is
        // already decided, and on a refresh there is nothing left to buy. It is
        // announced rather than passed over in silence — a run that quietly
        // described three fewer directories than it started is the thing this
        // is meant to stop looking like progress.
        if above_failure == AboveFailure::Skip
            && let Some(below) = failure_below(&undocumented, pacted)
        {
            observer.skipped(pacted, below);
            continue;
        }

        // Taken now rather than in phase two: the children below have already
        // had their turn, so their documents are final and this is the request
        // that would actually go out.
        let carry = carry_hash(pacted);

        if let Some(carried) = carried_document(pacted, carry.as_deref(), recorded) {
            carries.insert(pacted.clone(), carry);
            documents.insert(pacted.clone(), carried);
            if failure_below(&undocumented, pacted).is_none() {
                observer.unchanged(pacted);
            }
            continue;
        }

        match pact_directory_watched(pacted, agent, observer) {
            Ok(Pacted {
                document,
                problems: caps,
                repairs: mended,
            }) => {
                problems.extend(caps);
                // Alongside the caps' problems and for the same reason: a
                // mended directory was described, hashed and granted like any
                // other, so this is a note about the run and not a failure in
                // it.
                repairs.extend(mended);
                // Taken again: the document is part of this digest, so the
                // pre-pass reading describes the directory as it no longer is.
                // Only this directory's own `WARLOCK.md` moved in between, and
                // it is prose in no request, which is what makes the two
                // readings comparable at all.
                carries.insert(pacted.clone(), carry_hash(pacted));
                documents.insert(pacted.clone(), document);
                // Announced only where the subtree below is whole, by the same
                // prefix test phase two grants by: the announcement is what a
                // front end colours done, and a directory above a failure will
                // be recorded without a grant.
                if failure_below(&undocumented, pacted).is_none() {
                    observer.documented(pacted);
                }
            }
            Err(error) => {
                undocumented.push(pacted.clone());
                failures.push(Failure::Document { source: error });
            }
        }
    }

    // One timestamp for the whole pact: the entries record a single event, and
    // a per-directory clock reading would only invite someone to read an
    // ordering into it.
    let granted_at = now_rfc3339();
    let mut outcomes = BTreeMap::new();
    for pacted in directories {
        let Some(document) = documents.get(pacted) else {
            continue;
        };
        // Both paths spelled the manifest's way here, where the directory to
        // blame is still in hand: an outcome carries stored paths and no root
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
        // never judged, yellow.
        let grant = if failure_below(&undocumented, pacted).is_some() {
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
        repairs,
    }
}

// `directories` is what the run claims to have covered, and an entry inside it
// that earned no outcome is dropped. Hence `pact_subtree` passing its whole
// walk and `refresh_subtree` an empty slice: passing the stale list for
// symmetry would un-pact a directory whose refresh pass failed.
fn rewrite(
    manifest: &Manifest,
    directories: &[PathBuf],
    root: &Path,
    mut outcomes: BTreeMap<String, Outcome>,
) -> Manifest {
    // A directory whose path cannot be stored has no entry to match anyway, so
    // dropping it here can only leave an entry alone, never drop the wrong one.
    let covered: BTreeSet<String> = directories
        .iter()
        .filter_map(|pacted| to_manifest_path(root, pacted).ok())
        .collect();

    let mut kept = Vec::with_capacity(manifest.entries().len() + outcomes.len());
    for existing in manifest.entries() {
        if let Some(outcome) = outcomes.remove(existing.module()) {
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

// Deliberately not the symmetric reverse of a pact: this opens no file for
// writing, so it cannot delete a `WARLOCK.md`. The document belongs to the
// project and is reviewed in the git diff like any other file.
//
// It does take the scopes, because an entry is the only home a scope has — the
// one operation here that can lose a boundary, and a deliberate press on a
// directory rather than a side effect of a run. Who may ask is not settled
// here: both callers ask `closed_scopes_at_or_below` first.
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

// String work on the manifest's own stored paths, never a question for the
// filesystem, so an entry whose directory is gone still answers.
fn at_or_below(module: &str, selected: &str) -> bool {
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
    for entry in manifest.entries() {
        if !at_or_below(entry.module(), &selected) {
            continue;
        }
        let Some(scope) = valid_scope(entry) else {
            continue;
        };
        if !scope_opens_to(Some(scope), held) && !blocking.contains(&scope) {
            blocking.push(scope);
        }
    }
    Ok(blocking)
}

// A pass is never shown the directory's previous document: it is the last
// pass's claim rather than evidence, and a pass that had it carried its
// sentences forward whether or not the files still supported them. So the
// document is written over without being read first.
//
// Nothing is recorded either — no entry, no hash, no grant. Deciding what the
// manifest should then say needs the rest of the subtree and is the caller's.
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
/// let Pacted { document, problems, repairs } = pact_directory(dir.path(), &Canned)?;
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
///
/// // A pass that got everything right leaves warlock nothing to mend.
/// assert!(repairs.is_empty());
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub fn pact_directory(directory: impl AsRef<Path>, agent: &dyn Agent) -> Result<Pacted, Error> {
    pact_directory_watched(directory.as_ref(), agent, &mut Unwatched)
}

fn pact_directory_watched(
    directory: &Path,
    agent: &dyn Agent,
    observer: &mut dyn Observer,
) -> Result<Pacted, Error> {
    // Neither byte cap can fail a fit, which is why no `Error` below is
    // reachable from anything about them. The prompt is written onto the
    // request afterwards, because what a pass is asked to fill depends on which
    // files and children the fitting left in it.
    let Fitted {
        request,
        problems,
        described,
    } = fit("", directory, agent, observer)?;
    let expected = document::Expected::of(&request);

    // The first pass is asked for the whole object; a second is asked only for
    // what the first got wrong and answers with a patch over it
    // ([`document::Repair`]), unless the first was not an object at all and is
    // asked cold again. A transport failure ends it at once: a pass that
    // produced no answer is not a pass that produced a wrong one, and retrying
    // a missing `claude` finds it still missing.
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

        // `previous.zip(repair)` is exactly what a repair pass carries: both or
        // neither, since a repair with nothing to repair from is a first pass.
        match document::accept(previous.as_ref().zip(repair.as_ref()), text, &expected) {
            Accepted::Filled(fill) => {
                accepted = Some(fill);
                break;
            }
            Accepted::Defective {
                fill,
                defects: found,
            } => {
                observer.rejected(directory, &found, attempt, ATTEMPTS);
                defects = found;
                // Kept as what the next pass repairs from: it is mostly right,
                // and asking for all of it again is how a pass that left out
                // one file of eighteen comes back having left out another.
                previous = Some(fill);
            }
            Accepted::Unparsed(defect) => {
                let found = vec![defect];
                observer.rejected(directory, &found, attempt, ATTEMPTS);
                defects = found;
                // `previous` is left where it was: an answer that is not an
                // object is nothing to repair from, so the next pass is asked
                // for whatever the last readable one was still missing.
            }
        }
    }
    // The floor under the exhausted loop. A gate is a shape the document has to
    // hold, not a test the pass has to pass: once the asking has run out, one
    // wrong slot out of thirty-six is mended here from what warlock already
    // measured rather than costing the directory — and every directory above it
    // — its grant. So `Malformed` is left for the one thing no mend can reach,
    // an answer that was never an object at all: `previous` is `Some` exactly
    // when some attempt parsed, and `Accepted::Unparsed` only ever carries
    // `Defect::NotJson`.
    let (fill, repairs) = if let Some(fill) = accepted {
        (fill, Vec::new())
    } else {
        let Some(best) = previous else {
            return Err(Error::Refused {
                directory: directory.to_path_buf(),
                cause: Refusal::Malformed {
                    defects,
                    attempts: ATTEMPTS,
                },
            });
        };
        let (mended, mends) = document::mend(&best, &expected, &described);
        let repairs = mends
            .into_iter()
            .map(|mend| {
                // Announced one at a time as it is made, and carried as data
                // besides: a front end wants to say so while the run is still
                // going, and a caller that was not watching still has to be
                // able to find out.
                observer.repaired(directory, &mend);
                Repaired {
                    directory: directory.to_path_buf(),
                    mend,
                }
            })
            .collect();
        (mended, repairs)
    };

    // The directory's name and not its path: the path is absolute, it is the
    // reader's home directory, and it would be committed.
    let name = directory.file_name().map_or_else(
        || directory.display().to_string(),
        |name| name.to_string_lossy().into_owned(),
    );
    let text = document::render(&name, &fill, &expected, &described);

    // Written beside and renamed over, the same idiom as `Manifest::save`. A
    // front end that quits mid-pact — killing the pass, restoring the terminal,
    // never waiting for this function to come back — must not be able to leave
    // half a document behind, and a rename is the only way to make that safe:
    // the file is the old document or the new one, never a prefix of either.
    // The temporary is named with a leading dot because hidden entries are
    // skipped by every [`ignore`] walk in this crate, so it is in no tree, no
    // subtree hash and no request for the moment it exists.
    let document = directory.join(DOCUMENT_FILE);
    let temp = directory.join(temp_file_name(DOCUMENT_FILE));
    let write = write_and_sync(&temp, text.as_bytes()).and_then(|()| fs::rename(&temp, &document));
    if let Err(source) = write {
        // Best effort: the caller is already being told the document was not
        // written, and a stray dot file is invisible to everything here.
        drop(fs::remove_file(&temp));
        return Err(Error::Write {
            // The document, not the temporary: the caller asked for
            // `WARLOCK.md`, and how it got written is not theirs to hear about.
            directory: directory.to_path_buf(),
            path: document,
            source,
        });
    }

    Ok(Pacted {
        document,
        problems,
        repairs,
    })
}

// Deliberately the same list `load_tree` would have made nodes of, so that
// "everything under here" means on screen what it means here. Nothing is
// filtered on top: no "already has a `WARLOCK.md`" test and no "has source in
// it" test, because an undocumented directory is exactly the one a pact exists
// to give a document to. Crate-private for the same reason — callers outside
// the crate have `load_tree`.
pub(crate) fn pactable_directories(root: &Path) -> Result<Vec<PathBuf>, Error> {
    // Asked separately because the walker below will not apply the rules to the
    // root it is handed. A directory the repository excluded has no pactable
    // directories at all, itself included.
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
        // no `.git` still has to be ignored, and `.warlock/` is Warlock's own
        // bookkeeping rather than content of the module.
        .follow_links(false)
        .require_git(false)
        .filter_entry(|entry| entry.file_name() != OsStr::new(MANIFEST_DIR))
        // The same matcher that reads `.gitignore`, so that what a pact covers
        // is what a tree shows and what a hash judges.
        .add_custom_ignore_filename(ignores::FILENAME)
        .build();

    // A set, so whatever order the walker offered is discarded rather than
    // reversed: the ordering below is a property of the paths, not of the
    // filesystem.
    let mut directories = BTreeSet::new();
    for entry in walker {
        let entry = entry.map_err(|source| Error::Walk {
            directory: root.to_path_buf(),
            source,
        })?;
        // A rule file the walker could not use is reported beside its
        // directory rather than in place of it, and taking that as "no rules"
        // would pact the content the repository excluded.
        if let Some(source) = entry.error() {
            return Err(Error::Walk {
                directory: root.to_path_buf(),
                source: source.clone(),
            });
        }
        // With `follow_links(false)` a symlinked directory reports as a
        // symlink, so it is neither descended into nor pacted as its target.
        if entry.file_type().is_some_and(|kind| kind.is_dir()) {
            directories.insert(entry.into_path());
        }
    }
    // Children before parents, and the `.rev()` is the whole of it: every
    // descendant sorts after its own ancestor. A parent's request carries its
    // children's documents, so pacting a parent first would hand the pass a
    // stale account of the subtree. Sibling order is arbitrary but fixed.
    Ok(directories.into_iter().rev().collect())
}

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
pub fn view_file(path: impl AsRef<Path>) -> Result<Viewed, Unviewable> {
    let path = path.as_ref();
    let mut bytes = read_capped(path).map_err(|source| Unviewable::Unreadable {
        path: path.to_path_buf(),
        source,
    })?;

    // The read stops one byte past the cap, so one byte over is the whole of
    // "there is more to this file", and dropping it needs no cast from the
    // cap's `u64` to an index.
    let cut = byte_count(bytes.len()) > PER_FILE_BYTE_CAP;
    if cut {
        bytes.truncate(bytes.len() - 1);
    }

    let text = match str::from_utf8(&bytes) {
        Ok(text) => text,
        // A cut inside a character is the cap's doing, not the file's, so it
        // costs that one character and nothing else. Everything before
        // `valid_up_to` was just checked, so the floor is unreachable.
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

fn read_capped(path: &Path) -> std::io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    fs::File::open(path)?
        .take(PER_FILE_BYTE_CAP + 1)
        .read_to_end(&mut bytes)?;
    Ok(bytes)
}

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
    fn starting(&mut self, directory: &Path, position: usize, total: usize) -> Pacting;

    fn requesting(&mut self, files: usize, bytes: u64) {
        let _ = (files, bytes);
    }

    fn rejected(&mut self, directory: &Path, defects: &[Defect], attempt: usize, attempts: usize) {
        let _ = (directory, defects, attempt, attempts);
    }

    /// One slot warlock mended itself, once the attempts behind
    /// [`rejected`](Observer::rejected) ran out. The document was still
    /// written: this is what it cost, not a reason it was not.
    fn repaired(&mut self, directory: &Path, mend: &document::Mend) {
        let _ = (directory, mend);
    }

    fn documented(&mut self, directory: &Path) {
        let _ = directory;
    }

    fn unchanged(&mut self, directory: &Path) {
        let _ = directory;
    }

    /// A directory a refresh did not describe, because `below` — a directory
    /// under it — failed and took its grant with it. No pass ran and nothing
    /// was written; the document it has is the one it had.
    fn skipped(&mut self, directory: &Path, below: &Path) {
        let _ = (directory, below);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pacting {
    Continue,
    Stop,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Unwatched;

impl Observer for Unwatched {
    fn starting(&mut self, _directory: &Path, _position: usize, _total: usize) -> Pacting {
        Pacting::Continue
    }
}

#[derive(Debug)]
pub struct PactedSubtree {
    pub manifest: Manifest,
    pub failures: Vec<Failure>,
    pub problems: Vec<Problem>,
    pub repairs: Vec<Repaired>,
}

#[derive(Debug)]
pub struct Pacted {
    pub document: PathBuf,
    pub problems: Vec<Problem>,
    pub repairs: Vec<Repaired>,
}

/// One slot warlock filled in itself rather than refusing the document, carried
/// the way [`Problem`] is: a repair is no more a failure than a file left out
/// of a request is, and both are things a caller is owed the list of.
///
/// The directory is here because the list travels up out of a whole subtree,
/// where the `field` alone — `purpose`, `files["writing.rs"]` — says nothing
/// about which document it was in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Repaired {
    pub directory: PathBuf,
    pub mend: document::Mend,
}

impl fmt::Display for Repaired {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.directory.display(), self.mend)
    }
}

#[derive(Debug)]
pub struct Viewed {
    pub text: String,
    pub cut: bool,
}

#[derive(Debug)]
#[non_exhaustive]
pub enum Unviewable {
    Unreadable {
        path: PathBuf,
        source: std::io::Error,
    },
    NotText {
        path: PathBuf,
        source: Utf8Error,
    },
}

impl Unviewable {
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

#[derive(Debug)]
#[non_exhaustive]
pub enum Refusal {
    Agent {
        source: agent::Error,
    },
    Malformed {
        defects: Vec<Defect>,
        attempts: usize,
    },
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Agent { source } => write!(f, "the model pass produced no answer: {source}"),
            Self::Malformed { defects, attempts } => {
                // One short line: this is what a footer shows, and a list of
                // thirty missing entries is not a footer line.
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

#[derive(Debug)]
#[non_exhaustive]
pub enum Error {
    Walk {
        directory: PathBuf,
        source: ignore::Error,
    },
    Path {
        directory: PathBuf,
        path: PathBuf,
        source: Box<manifest::Error>,
    },
    Refused {
        directory: PathBuf,
        cause: Refusal,
    },
    Write {
        directory: PathBuf,
        path: PathBuf,
        source: std::io::Error,
    },
}

impl Error {
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

#[derive(Debug)]
#[non_exhaustive]
pub enum Failure {
    Document {
        source: Error,
    },
    Record {
        directory: PathBuf,
        source: manifest::Error,
    },
    Hash {
        directory: PathBuf,
        source: hash::Error,
    },
}

impl Failure {
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

    struct Canned {
        text: Option<String>,
        seen: std::cell::RefCell<Vec<agent::Request>>,
    }

    impl Canned {
        fn new(text: impl Into<String>) -> Self {
            Self {
                text: Some(text.into()),
                seen: std::cell::RefCell::new(Vec::new()),
            }
        }

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

    struct Fails(fn() -> agent::Error);

    impl Agent for Fails {
        fn run(&self, _request: &agent::Request) -> Result<agent::Response, agent::Error> {
            Err(self.0())
        }
    }

    fn document(bytes: usize) -> String {
        let head = "# engine\n\nCore engine for warlock. ";
        assert!(bytes > head.len(), "a document has room for its heading");
        format!("{head}{}", "x".repeat(bytes - head.len()))
    }

    fn written(dir: &Path) -> Option<Vec<u8>> {
        fs::read(dir.join("WARLOCK.md")).ok()
    }

    fn write(dir: &Path, name: &str, contents: impl AsRef<[u8]>) -> PathBuf {
        let path = dir.join(name);
        fs::create_dir_all(path.parent().expect("a file has a parent")).expect("creates parents");
        fs::write(&path, contents).expect("writes a file");
        path
    }

    fn filler(size: u64) -> Vec<u8> {
        vec![b'x'; usize::try_from(size).expect("a test file fits in memory")]
    }

    fn share(percent: u64) -> u64 {
        REQUEST_BYTE_CAP * percent / 100
    }

    fn share_bytes(percent: u64) -> usize {
        usize::try_from(share(percent)).expect("a share of the cap fits in memory")
    }

    fn not_text(size: u64) -> Vec<u8> {
        let mut bytes = filler(size);
        *bytes.last_mut().expect("a fixture has bytes") = 0xff;
        bytes
    }

    const AMPLE_CAP: u64 = PER_FILE_BYTE_CAP * 4;

    fn request_for(dir: &Path) -> agent::Request {
        let Gathered { request, problems } =
            gather_request("summarise", dir, AMPLE_CAP).expect("gathers");
        assert!(problems.is_empty(), "{problems:?}");
        request
    }

    fn is_document_pass(request: &agent::Request) -> bool {
        request.prompt().starts_with(document::PROMPT)
    }

    fn pass(seen: &[agent::Request]) -> &agent::Request {
        let request = seen.last().expect("the directory was pacted");
        assert!(
            is_document_pass(request),
            "the last pass of a pact is the pact"
        );
        request
    }

    fn modules(manifest: &Manifest) -> Vec<&str> {
        manifest.entries().iter().map(PactEntry::module).collect()
    }

    fn scopes(manifest: &Manifest) -> Vec<(&str, Option<&str>)> {
        manifest
            .entries()
            .iter()
            .map(|entry| (entry.module(), entry.scope()))
            .collect()
    }

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

    fn file_paths(request: &agent::Request) -> Vec<&str> {
        request.files().iter().map(agent::File::path).collect()
    }

    fn file<'a>(request: &'a agent::Request, path: &str) -> &'a agent::File {
        request
            .files()
            .iter()
            .find(|file| file.path() == path)
            .unwrap_or_else(|| panic!("`{path}` is in the request: {:?}", file_paths(request)))
    }

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

    struct Counting {
        script: Vec<Result<String, fn() -> agent::Error>>,
        beyond: String,
        seen: std::cell::RefCell<Vec<agent::Request>>,
    }

    impl Counting {
        fn new(beyond: impl Into<String>) -> Self {
            Self {
                script: Vec::new(),
                beyond: beyond.into(),
                seen: std::cell::RefCell::new(Vec::new()),
            }
        }

        fn scripted(
            mut self,
            script: impl IntoIterator<Item = Result<String, fn() -> agent::Error>>,
        ) -> Self {
            self.script = script.into_iter().collect();
            self
        }

        fn passes(&self) -> usize {
            self.seen.borrow().len()
        }

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

    // The mend: the floor under an exhausted attempt loop.

    // A pass that answers with the right shape and the same slot wrong every
    // time, however often it is asked. Four attempts that change nothing is
    // the only road to the mend: it is what is left when the asking has run
    // out, not a substitute for asking.
    struct Defective {
        break_it: fn(&mut serde_json::Map<String, serde_json::Value>),
        seen: std::cell::RefCell<Vec<agent::Request>>,
    }

    impl Defective {
        fn with(break_it: fn(&mut serde_json::Map<String, serde_json::Value>)) -> Self {
            Self {
                break_it,
                seen: std::cell::RefCell::new(Vec::new()),
            }
        }

        fn document_passes(&self) -> usize {
            self.seen
                .borrow()
                .iter()
                .filter(|request| is_document_pass(request))
                .count()
        }
    }

    impl Agent for Defective {
        fn run(&self, request: &agent::Request) -> Result<agent::Response, agent::Error> {
            self.seen.borrow_mut().push(request.clone());
            let answer = document::stub_answer(request);
            if !is_document_pass(request) {
                return Ok(agent::Response::new(answer));
            }
            let mut parsed: serde_json::Value =
                serde_json::from_str(&answer).expect("a document pass is answered with an object");
            (self.break_it)(parsed.as_object_mut().expect("a fill is an object"));
            Ok(agent::Response::new(parsed.to_string()))
        }
    }

    fn blank_purpose(answer: &mut serde_json::Map<String, serde_json::Value>) {
        answer.insert(
            "purpose".to_owned(),
            serde_json::Value::String(String::new()),
        );
    }

    const OVERLONG: usize = document::ENTRY_CHARS + 120;

    // Two wrong slots of two different kinds: one value warlock can cut back
    // to its cap out of the answer's own text, and one it has to fill in from
    // what it measured itself.
    fn blank_purpose_and_overlong_entries(answer: &mut serde_json::Map<String, serde_json::Value>) {
        blank_purpose(answer);
        let files = answer
            .get_mut("files")
            .and_then(serde_json::Value::as_object_mut)
            .expect("a fill holds an entry per file");
        for value in files.values_mut() {
            *value = serde_json::Value::String("x".repeat(OVERLONG));
        }
    }

    fn named(root: &Path, path: &Path) -> String {
        relative_to(root, std::slice::from_ref(&path.to_path_buf()))
            .pop()
            .expect("one directory in, one name out")
    }

    struct Mending {
        rejections: Vec<(PathBuf, usize)>,
        repairs: Vec<(PathBuf, document::Mend)>,
    }

    impl Mending {
        fn new() -> Self {
            Self {
                rejections: Vec::new(),
                repairs: Vec::new(),
            }
        }

        fn turned_down(&self, root: &Path) -> Vec<(String, usize)> {
            self.rejections
                .iter()
                .map(|(directory, attempt)| (named(root, directory), *attempt))
                .collect()
        }

        fn announced(&self, root: &Path) -> Vec<(String, String, document::Mended)> {
            self.repairs
                .iter()
                .map(|(directory, mend)| (named(root, directory), mend.field.clone(), mend.done))
                .collect()
        }
    }

    impl Observer for Mending {
        fn starting(&mut self, _directory: &Path, _position: usize, _total: usize) -> Pacting {
            Pacting::Continue
        }

        fn rejected(
            &mut self,
            directory: &Path,
            _defects: &[document::Defect],
            attempt: usize,
            _attempts: usize,
        ) {
            self.rejections.push((directory.to_path_buf(), attempt));
        }

        fn repaired(&mut self, directory: &Path, mend: &document::Mend) {
            self.repairs.push((directory.to_path_buf(), mend.clone()));
        }
    }

    #[test]
    fn an_answer_that_stays_defective_is_mended_into_a_document_and_a_grant() {
        let repo = project();
        let src = repo.path().join("crates/tui/src");
        let agent = Defective::with(blank_purpose);

        let PactedSubtree {
            manifest,
            failures,
            repairs,
            ..
        } = pact_subtree(&src, repo.path(), &Manifest::new(), &agent, &mut Unwatched)
            .expect("an answer warlock can mend is an answer warlock writes");

        assert_eq!(
            agent.document_passes(),
            document::ATTEMPTS,
            "the asking happened first, in full: the mend is the floor under the \
             loop and not a shortcut through it",
        );
        assert!(failures.is_empty(), "{failures:?}");

        // The document, written and renamed like any other.
        let text = String::from_utf8(written(&src).expect("a document")).expect("text");
        assert!(text.starts_with(STAMP), "{text}");
        assert!(text.contains("`main.rs`"), "{text}");

        // And the grant, recorded where a grant is recorded.
        manifest.save(repo.path()).expect("saves");
        let recorded = Manifest::load(repo.path()).expect("loads");
        assert_eq!(
            recorded
                .entry("crates/tui/src")
                .and_then(PactEntry::granted_hash),
            Some(subtree_hash(&src).expect("hashes").as_str()),
            "the hash covers the directory as it now stands, document and all",
        );
        assert_eq!(
            state(&recorded, repo.path(), "crates/tui/src"),
            NodeState::PactedFresh,
            "which is what makes the directory green",
        );
        assert!(
            fs::read_to_string(repo.path().join(".warlock/pacts.toml"))
                .expect("reads")
                .contains("crates/tui/src"),
            "and it is in the file, not only in memory",
        );

        assert_eq!(repairs.len(), 1, "{repairs:?}");
        assert_eq!(repairs[0].directory, src);
        assert_eq!(repairs[0].mend.field, "purpose");
    }

    #[test]
    fn an_answer_that_is_never_an_object_still_refuses_and_leaves_the_old_document_alone() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        write(dir.path(), "lib.rs", "//! Core engine.\n");
        let before = format!("{STAMP}\n# engine\n\nThe document that is already here.\n");
        write(dir.path(), DOCUMENT_FILE, &before);
        let agent = Canned::new("Here is some prose instead of the object you asked for.");

        let error =
            pact_directory(dir.path(), &agent).expect_err("there is no slot in prose to mend");

        assert!(
            matches!(
                error,
                super::Error::Refused {
                    cause: Refusal::Malformed { .. },
                    ..
                }
            ),
            "an answer that was never an object is the one thing no mend reaches: {error:?}",
        );
        assert_eq!(
            written(dir.path()).map(|bytes| String::from_utf8(bytes).expect("text")),
            Some(before),
            "byte for byte what was there: a refusal writes nothing, and half a \
             document is worse than yesterday's",
        );
    }

    // A directory laid out to reach every section `render` writes: a file with
    // symbols in it, a file with none, a file that is not text, a child with a
    // document, and an answer carrying structure, rules and a route.
    fn spelled_out() -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("a temporary directory");
        write(
            dir.path(),
            "reading.rs",
            "pub fn read_one() {}\npub struct Reader;\n",
        );
        write(dir.path(), "writing.rs", "fn scratch() {}\n");
        write(dir.path(), "table.bin", [0xff_u8, 0x00, 0xfe, 0x01]);
        write(
            dir.path(),
            "inner/WARLOCK.md",
            format!("{STAMP}\n# inner\n"),
        );
        dir
    }

    const SPELLED_OUT_ANSWER: &str = r#"{
      "purpose": "Reading and writing for the fixture, kept apart from the table beside them.",
      "files": {
        "reading.rs": "The reading half: one entry point and the type it hands back.",
        "writing.rs": "The writing half, which is a single unexported helper for now."
      },
      "directories": { "inner": "A child directory carrying a document of its own." },
      "structure": ["Reading and writing are separate files and share no state."],
      "rules": ["Anything binary stays out of the two source files."],
      "lookups": [{ "for": "reading a record", "open": "reading.rs", "symbol": "read_one" }]
    }"#;

    #[test]
    fn a_pass_that_got_everything_right_writes_the_document_it_wrote_before_the_mend() {
        // The mend is a floor under an exhausted loop, so the one thing it must
        // not do is change what a clean pass produces. The document is spelled
        // out here rather than compared against `render`, `STAMP` or any other
        // constant the code could move with it: a stamp reworded, a section
        // reordered, a size formatted differently or a repaired value reaching
        // an answer that had nothing wrong with it all fail this, which is the
        // whole point of writing the bytes out by hand. The bytes below are not
        // this branch's output written down: this test was run unchanged (minus
        // the `repairs` field, which did not exist yet) against e1a3dae, the
        // commit before the mend, and passed there too.
        let dir = spelled_out();
        let agent = Canned::new(SPELLED_OUT_ANSWER);

        let Pacted {
            document,
            problems,
            repairs,
        } = pact_directory(dir.path(), &agent).expect("a right answer is a written document");

        assert!(problems.is_empty(), "{problems:?}");
        assert!(
            repairs.is_empty(),
            "a pass that got everything right leaves nothing to mend: {repairs:?}",
        );
        assert_eq!(
            fs::read_to_string(&document).expect("reads"),
            format!(
                "<!-- warlock -->\n\
                 > Written by a model pass over this directory alone, to be read before its \
                 source and to say which source to read. A map, not a specification: check \
                 anything you are about to rely on against the files themselves, and where this \
                 document and the code disagree, the code is right.\n\
                 \n\
                 # {}\n\
                 \n\
                 Reading and writing for the fixture, kept apart from the table beside them.\n\
                 \n\
                 ## Files\n\
                 \n\
                 - `reading.rs` (40 B) — The reading half: one entry point and the type it hands \
                 back. · declares `read_one`, `Reader`\n\
                 - `table.bin` (4 B) — not text; name and size only\n\
                 - `writing.rs` (16 B) — The writing half, which is a single unexported helper \
                 for now. · declares `scratch`\n\
                 \n\
                 ## Directories\n\
                 \n\
                 - `inner/` — A child directory carrying a document of its own.\n\
                 \n\
                 ## Structure\n\
                 \n\
                 - Reading and writing are separate files and share no state.\n\
                 \n\
                 ## Rules\n\
                 \n\
                 - Anything binary stays out of the two source files.\n\
                 \n\
                 ## Where to look\n\
                 \n\
                 - reading a record → `reading.rs` `read_one`\n",
                dir.path()
                    .file_name()
                    .expect("a temporary directory has a name")
                    .to_string_lossy(),
            ),
        );

        // And written the way it has always been written: beside and renamed
        // over, leaving the temporary behind nowhere.
        let mut left: Vec<String> = fs::read_dir(dir.path())
            .expect("reads")
            .map(|entry| {
                entry
                    .expect("an entry")
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        left.sort();
        assert_eq!(
            left,
            [
                "WARLOCK.md",
                "inner",
                "reading.rs",
                "table.bin",
                "writing.rs"
            ],
        );
    }

    #[test]
    fn a_mended_directory_is_not_a_failure_and_the_subtree_is_still_pacted() {
        let repo = project();
        let engine = repo.path().join("crates/engine");
        let agent = Defective::with(blank_purpose);

        let PactedSubtree {
            manifest,
            failures,
            repairs,
            ..
        } = pact_subtree(
            &engine,
            repo.path(),
            &Manifest::new(),
            &agent,
            &mut Unwatched,
        )
        .expect("a subtree of mended passes is a pacted subtree");

        assert!(
            failures.is_empty(),
            "a directory warlock mended was documented, hashed and granted, so it \
             is a note about the run and not a failure in it: {failures:?}",
        );
        let mended: Vec<PathBuf> = repairs
            .iter()
            .map(|repaired| repaired.directory.clone())
            .collect();
        for module in [
            "crates/engine/tests",
            "crates/engine/src/inner",
            "crates/engine/src",
            "crates/engine",
        ] {
            let directory = from_manifest_path(repo.path(), module);
            assert!(written(&directory).is_some(), "`{module}` is documented");
            assert!(
                manifest
                    .entry(module)
                    .and_then(PactEntry::granted_hash)
                    .is_some(),
                "`{module}` is granted, the same as a clean pass would leave it",
            );
            assert!(
                failures
                    .iter()
                    .all(|failure| failure.directory() != directory),
                "and `{module}` is in no failure: {failures:?}",
            );
            assert!(
                mended.contains(&directory),
                "`{module}` was mended, which is why this is worth asserting: {repairs:?}",
            );
        }
    }

    #[test]
    fn every_mend_is_carried_out_of_the_run_and_announced_as_it_is_made() {
        let repo = project();
        let src = repo.path().join("crates/tui/src");
        let agent = Defective::with(blank_purpose_and_overlong_entries);
        let mut observer = Mending::new();

        let PactedSubtree {
            failures, repairs, ..
        } = pact_subtree(&src, repo.path(), &Manifest::new(), &agent, &mut observer)
            .expect("pacts");

        assert!(failures.is_empty(), "{failures:?}");
        assert_eq!(
            observer.turned_down(repo.path()),
            (1..=document::ATTEMPTS)
                .map(|attempt| ("crates/tui/src".to_owned(), attempt))
                .collect::<Vec<_>>(),
            "the loop ran out first, and said so each time",
        );

        let carried: Vec<(String, String, document::Mended)> = repairs
            .iter()
            .map(|repaired| {
                (
                    named(repo.path(), &repaired.directory),
                    repaired.mend.field.clone(),
                    repaired.mend.done,
                )
            })
            .collect();
        assert_eq!(
            carried,
            [
                (
                    "crates/tui/src".to_owned(),
                    "purpose".to_owned(),
                    document::Mended::Supplied,
                ),
                (
                    "crates/tui/src".to_owned(),
                    "files[\"main.rs\"]".to_owned(),
                    document::Mended::Cut {
                        from: OVERLONG,
                        to: document::ENTRY_CHARS,
                    },
                ),
            ],
            "one value per slot, naming the directory and the slot in `Defect`'s \
             own spelling",
        );
        assert_eq!(
            observer.announced(repo.path()),
            carried,
            "and the calls a front end saw are the list a caller is handed: the \
             hook is the same facts, earlier",
        );
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

    const FAT_SHARES: [(&str, u64); 5] = [
        ("a.bin", 31),
        ("b.bin", 35),
        ("c.bin", 39),
        ("d.bin", 43),
        ("e.bin", 47),
    ];

    fn fat() -> [(&'static str, u64); 5] {
        FAT_SHARES.map(|(name, percent)| (name, share(percent)))
    }

    fn sent(request: &agent::Request) -> Vec<&str> {
        request
            .files()
            .iter()
            .filter(|file| file.bytes().is_some())
            .map(agent::File::path)
            .collect()
    }

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
    fn a_file_the_budget_gives_up_reaches_the_pass_as_a_sample_of_itself() {
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
        assert!(
            listed(pass).is_empty(),
            "nothing reaches the pass as a bare name any more: the file the \
             budget gave up carries a sample of its own text instead",
        );
        assert!(
            carried(pass) <= REQUEST_BYTE_CAP,
            "{} bytes is over the {REQUEST_BYTE_CAP}-byte cap",
            carried(pass),
        );
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert_eq!(problems[0].path, dir.path().join("a.bin"));
        assert!(
            matches!(
                problems[0].cause,
                Omission::Unreducible { size } | Omission::OverBudget { size } if size == share(35)
            ),
            "filler has no declarations to lift, so it is still reported as \
             unreducible — most of it really is missing — but it is reported \
             beside a sample rather than instead of one: {:?}",
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

    struct FailsFor {
        directory: PathBuf,
        asked: std::cell::RefCell<Vec<PathBuf>>,
    }

    impl FailsFor {
        fn at(directory: impl Into<PathBuf>) -> Self {
            Self {
                directory: directory.into(),
                asked: std::cell::RefCell::new(Vec::new()),
            }
        }

        // Every directory a request went out for, refused or not — which is
        // what a skip is measured against: a directory nobody paid for is a
        // directory that is not in here.
        fn asked(&self, root: &Path) -> Vec<String> {
            relative_to(root, &self.asked.borrow())
        }
    }

    impl Agent for FailsFor {
        fn run(&self, request: &agent::Request) -> Result<agent::Response, agent::Error> {
            self.asked
                .borrow_mut()
                .push(request.directory().to_path_buf());
            if request.directory() == self.directory {
                return Err(agent::Error::EmptyOutput);
            }
            Ok(agent::Response::new(document::stub_answer(request)))
        }
    }

    struct Watching {
        stop_after: Option<usize>,
        calls: Vec<(PathBuf, usize, usize)>,
        documented: Vec<PathBuf>,
        skipped: Vec<(PathBuf, PathBuf)>,
    }

    impl Watching {
        fn patient() -> Self {
            Self {
                stop_after: None,
                calls: Vec::new(),
                documented: Vec::new(),
                skipped: Vec::new(),
            }
        }

        fn stopping_after(directories: usize) -> Self {
            Self {
                stop_after: Some(directories),
                calls: Vec::new(),
                documented: Vec::new(),
                skipped: Vec::new(),
            }
        }

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

        fn offered(&self) -> Vec<PathBuf> {
            self.calls
                .iter()
                .map(|(directory, ..)| directory.clone())
                .collect()
        }

        fn done(&self, root: &Path) -> Vec<String> {
            relative_to(root, &self.documented)
        }

        // Both halves, because the pair is the announcement: the directory that
        // got no pass is only half an answer without the one that cost it.
        fn passed_over(&self, root: &Path) -> Vec<(String, String)> {
            self.skipped
                .iter()
                .map(|(directory, below)| {
                    let named = |path: &PathBuf| {
                        relative_to(root, std::slice::from_ref(path))
                            .pop()
                            .expect("one directory in, one name out")
                    };
                    (named(directory), named(below))
                })
                .collect()
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

        fn skipped(&mut self, directory: &Path, below: &Path) {
            self.skipped
                .push((directory.to_path_buf(), below.to_path_buf()));
        }
    }

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
        let agent = FailsFor::at(failing.clone());

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
            ..
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
        let agent = FailsFor::at(failing);
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
        let agent = FailsFor::at(failing.clone());
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

    // Announcing the request itself: what the directory's own pass was handed.

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

    fn pacted(modules: &[&str]) -> Manifest {
        Manifest::with_entries(modules.iter().map(|module| {
            PactEntry::new(".", module, format!("{module}/WARLOCK.md"))
                .expect("a relative path inside the root is storable")
                .with_grant(format!("hash-of-{module}"), "2026-08-21T09:00:00Z")
        }))
    }

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
            ..
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
            ..
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
            1,
            "the deepest stale directory is the only one that reached the \
             agent: the two above it are ancestors of its failure and were \
             skipped rather than paid for: {failures:?}",
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

    // The whole point of the skip, and the reason it is worth a test of its
    // own: a refresh that hits a failure used to walk on up the tree, pay for a
    // pass at every directory above it, and record each one without a grant —
    // documents the next run has to write again, because the directory below
    // still has to be described and a described child is a moved parent
    // request. The spend was real and nothing survived it.
    #[test]
    fn a_refresh_above_a_failed_pass_skips_the_ancestor_rather_than_paying_for_it() {
        let repo = project();
        let engine = repo.path().join("crates/engine");
        let before = refreshable(repo.path());
        write(
            repo.path(),
            "crates/engine/src/inner/deep.rs",
            "fn deeper() {}\n",
        );
        let agent = FailsFor::at(engine.join("src").join("inner"));
        let mut observer = Watching::patient();

        let PactedSubtree {
            manifest, failures, ..
        } = refresh_subtree(&engine, repo.path(), &before, &agent, &mut observer)
            .expect("one refused pass does not fail the refresh");

        assert_eq!(failures.len(), 1, "{failures:?}");
        assert_eq!(failures[0].directory(), engine.join("src").join("inner"));
        assert_eq!(
            agent.asked(repo.path()),
            ["crates/engine/src/inner"],
            "one request went out, for the one stale directory that was not \
             above a failure — the two above it cost nothing",
        );
        assert_eq!(
            observer.passed_over(repo.path()),
            [
                (
                    "crates/engine/src".to_owned(),
                    "crates/engine/src/inner".to_owned()
                ),
                (
                    "crates/engine".to_owned(),
                    "crates/engine/src/inner".to_owned()
                ),
            ],
            "and each one says which failure below it took it down, so a run \
             that describes fewer directories than it started does not look \
             like one that finished",
        );
        for untouched in [
            "crates/engine/src/inner",
            "crates/engine/src",
            "crates/engine",
            "crates/engine/tests",
        ] {
            assert_eq!(
                manifest.entry(untouched),
                before.entry(untouched),
                "`{untouched}` kept the entry it had, grant and all: a refresh \
                 that did not re-describe a directory has nothing to say about \
                 it, and the one it could not re-describe is not un-pacted",
            );
        }
        for stale in [
            "crates/engine/src/inner",
            "crates/engine/src",
            "crates/engine",
        ] {
            assert_eq!(
                state(&manifest, repo.path(), stale),
                NodeState::PactedStale,
                "`{stale}` reads stale on the grant it kept, because the hash \
                 under it moved and no pass has been granted since",
            );
        }
    }

    // The bytes themselves. Everything above asserts about entries; this
    // asserts about the file, so that a change of shape — a key that moves, a
    // blank line that appears, an entry that is appended where it used to be
    // replaced in place — fails the build instead of passing quietly.

    fn hash_of(repo: &Path, module: &str) -> String {
        subtree_hash(from_manifest_path(repo, module)).expect("the subtree hashes")
    }

    fn carry_of(repo: &Path, module: &str) -> String {
        super::carry_hash(&from_manifest_path(repo, module)).expect("the directory digests")
    }

    fn granted_at_of(manifest: &Manifest, module: &str) -> String {
        manifest
            .entry(module)
            .unwrap_or_else(|| panic!("`{module}` is pacted"))
            .granted_at()
            .unwrap_or_else(|| panic!("`{module}` is granted"))
            .to_owned()
    }

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
            ..
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
            ..
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

    fn scoped_path(repo: &Path) -> Manifest {
        with_scopes(
            &refreshable(repo),
            &[
                ("crates/engine", "engine"),
                ("crates/engine/src", "data-plane"),
                ("crates/engine/src/inner", "deep"),
                ("crates/engine/tests", "harness"),
            ],
        )
    }

    #[test]
    fn a_partly_completed_refresh_keeps_every_scope() {
        let repo = project();
        let engine = repo.path().join("crates/engine");
        let before = scoped_path(repo.path());
        write(
            repo.path(),
            "crates/engine/src/inner/deep.rs",
            "fn deeper() {}\n",
        );

        // One pass refuses, so the two scoped directories above it are skipped
        // and the scoped directory that refused is left where it was.
        let PactedSubtree {
            manifest, failures, ..
        } = refresh_subtree(
            &engine,
            repo.path(),
            &before,
            &FailsFor::at(engine.join("src").join("inner")),
            &mut Unwatched,
        )
        .expect("one refused pass does not fail the refresh");

        assert_eq!(failures.len(), 1, "{failures:?}");
        assert_eq!(
            scopes(&manifest),
            scopes(&before),
            "a refresh that got part way holds every boundary somebody drew: \
             it did not describe these directories, so it has nothing to say \
             about them",
        );
    }

    // The other half, on the caller that still describes above a failure.
    // `AboveFailure::Describe` is now the only route to an ungranted entry
    // written by a run that meant to grant it, and a scope has to survive it.
    #[test]
    fn a_partly_completed_pact_keeps_the_scope_of_every_entry_it_keeps() {
        let repo = project();
        let engine = repo.path().join("crates/engine");
        let before = scoped_path(repo.path());

        let PactedSubtree {
            manifest, failures, ..
        } = pact_subtree(
            &engine,
            repo.path(),
            &before,
            &FailsFor::at(engine.join("src").join("inner")),
            &mut Unwatched,
        )
        .expect("one refused pass does not fail the pact");

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
            scopes(&before)
                .into_iter()
                .filter(|(module, _)| *module != "crates/engine/src/inner")
                .collect::<Vec<_>>(),
            "the grant is a field a run owns and clears; the scope is not, so \
             the two ungranted directories keep their boundaries. The one that \
             earned nothing loses its entry, and a scope has no home outside an \
             entry — a pact is the direction that can take a boundary away",
        );
    }

    fn cap() -> usize {
        usize::try_from(PER_FILE_BYTE_CAP).expect("the cap fits an index")
    }

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
