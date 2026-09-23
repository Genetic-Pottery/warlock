use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use crate::document::{self, Defect};
use crate::fitting::{Assembled, Problem, Snapshot, Synthesised};
use crate::hash::carry_hash;
use crate::ignores;
use crate::manifest::{ROOT_MODULE, temp_file_name, write_and_sync};
use crate::scope::at_or_below;
use crate::walk::{self, DOCUMENT_FILE};
use crate::{
    Agent, Manifest, NodeState, PactEntry, agent, decide_state, from_manifest_path, hash, manifest,
    now_rfc3339, subtree_hash, to_manifest_path,
};

/// ```
/// use std::fs;
/// use warlock_engine::{
///     Agent, Manifest, NodeState, PactedSubtree, Unwatched, agent, decide_state,
///     document, pact_subtree, subtree_hash,
/// };
///
/// /// The engine's own tests reach a model exactly like this: they don't.
/// struct Canned;
///
/// impl Agent for Canned {
///     fn run(&self, request: &agent::Request) -> Result<agent::Response, agent::Error> {
///         Ok(agent::Response::new(document::stub_answer(request)))
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
///     document, pact_subtree, refresh_subtree, subtree_hash,
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
///         Ok(agent::Response::new(document::stub_answer(request)))
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
/// // One pass per file, then one over the lines to say how they fit together:
/// // `src` has a file and the directory above it has none, so three.
/// assert_eq!(agent.passes.get(), 3, "children before parents");
///
/// // Nothing has moved, so a refresh describes nothing and costs nothing.
/// let PactedSubtree { manifest, .. } =
///     refresh_subtree(&engine, repo.path(), &manifest, &agent, &mut Unwatched)?;
/// assert_eq!(agent.passes.get(), 3, "nothing stale, no pass");
///
/// // Now a file changes in the parent directory only.
/// let below = manifest.entry("crates/engine/src").expect("the child is pacted").clone();
/// fs::write(engine.join("Cargo.toml"), "[package]\nname = \"engine\"\n")?;
///
/// let PactedSubtree { manifest, failures, .. } =
///     refresh_subtree(&engine, repo.path(), &manifest, &agent, &mut Unwatched)?;
///
/// assert!(failures.is_empty());
/// // One new file to describe, and one pass over the lines to place it. The
/// // directory below is untouched and costs nothing, which is the whole prize.
/// assert_eq!(agent.passes.get(), 5, "the changed directory, and not the one below it");
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
    // nothing about manifests: it is handed the digest a directory was granted
    // on and the lines it was last known to hold the same way it is handed the
    // directories themselves.
    let mut recorded: BTreeMap<PathBuf, String> = BTreeMap::new();
    let mut line_hashes: BTreeMap<PathBuf, BTreeMap<String, String>> = BTreeMap::new();
    for candidate in &stale {
        let entry = to_manifest_path(root, candidate)
            .ok()
            .and_then(|module| manifest.entry(&module));
        let Some(entry) = entry else {
            continue;
        };
        if let Some(carry) = entry.carry_hash() {
            recorded.insert(candidate.clone(), carry.to_string());
        }
        if let Some(lines) = entry.lines() {
            line_hashes.insert(candidate.clone(), lines.clone());
        }
    }

    let Described {
        outcomes,
        failures,
        problems,
        repairs,
    } = describe_and_grant(
        &stale,
        root,
        &recorded,
        &line_hashes,
        AboveFailure::Skip,
        agent,
        observer,
    );

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
    // What each file hashed to as its line was settled. Written with the grant
    // by the same save, so a line believed current can never be judged against
    // a hash from another one.
    lines: BTreeMap<String, String>,
}

impl Outcome {
    fn apply(self, entry: &mut PactEntry) {
        let lines = self.grant.as_ref().map(|grant| grant.lines.clone());
        entry.overwrite_run_fields(
            self.module,
            self.document,
            self.grant.map(
                |Grant {
                     hash, at, carry, ..
                 }| (hash, at, carry),
            ),
        );
        // After `overwrite_run_fields`, which clears the field: what this run
        // recorded is what stands, and a run that granted nothing leaves the
        // entry saying no line is reusable.
        if let Some(lines) = lines.filter(|lines| !lines.is_empty()) {
            entry.set_lines(lines);
        }
    }

    fn into_entry(self) -> PactEntry {
        let entry = PactEntry::stored(self.module, self.document);
        let Some(Grant {
            hash,
            at,
            carry,
            lines,
        }) = self.grant
        else {
            return entry;
        };

        let mut entry = entry.with_grant(hash, at);
        if let Some(carry) = carry {
            entry = entry.with_carry_hash(carry);
        }
        if !lines.is_empty() {
            entry = entry.with_lines(lines);
        }
        entry
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
    line_hashes: &BTreeMap<PathBuf, BTreeMap<String, String>>,
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
    let mut lines: BTreeMap<PathBuf, BTreeMap<String, String>> = BTreeMap::new();
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

        let page = fs::read_to_string(pacted.join(DOCUMENT_FILE)).ok();
        let reusable = page
            .as_deref()
            .zip(line_hashes.get(pacted))
            .filter(|_| above_failure == AboveFailure::Skip);
        match pact_directory_watched(pacted, reusable, agent, observer) {
            Ok(Pacted {
                document,
                problems: caps,
                repairs: mended,
                hashes,
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
                lines.insert(pacted.clone(), hashes);
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

    let (outcomes, recording) = grant(
        directories,
        root,
        &Finished {
            documents,
            carries,
            lines,
            undocumented,
        },
    );
    failures.extend(recording);

    Described {
        outcomes,
        failures,
        problems,
        repairs,
    }
}

/// What phase one left behind, for [`grant`] to decide the entries from.
struct Finished {
    documents: BTreeMap<PathBuf, PathBuf>,
    carries: BTreeMap<PathBuf, Option<String>>,
    lines: BTreeMap<PathBuf, BTreeMap<String, String>>,
    /// Directories this run did not document, whether it failed on them or was
    /// stopped before it reached them. A prefix of one of these is what
    /// withholds a grant.
    undocumented: Vec<PathBuf>,
}

// Phase two, and it runs no pass: every document is on disk by the time this is
// called, which is what lets a parent be hashed over children that are already
// final. Split out of `describe_and_grant` because the two halves share only
// what `Finished` carries — one spends money, this one reads a directory that
// has stopped moving.
fn grant(
    directories: &[PathBuf],
    root: &Path,
    finished: &Finished,
) -> (BTreeMap<String, Outcome>, Vec<Failure>) {
    // One timestamp for the whole pact: the entries record a single event, and
    // a per-directory clock reading would only invite someone to read an
    // ordering into it.
    let granted_at = now_rfc3339();
    let mut failures = Vec::new();
    let mut outcomes = BTreeMap::new();
    for pacted in directories {
        let Some(document) = finished.documents.get(pacted) else {
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
        let grant = if failure_below(&finished.undocumented, pacted).is_some() {
            None
        } else {
            match subtree_hash(pacted) {
                Ok(hash) => Some(Grant {
                    hash,
                    at: granted_at.clone(),
                    carry: finished.carries.get(pacted).cloned().flatten(),
                    lines: finished.lines.get(pacted).cloned().unwrap_or_default(),
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

    (outcomes, failures)
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
    manifest.rebuilt_with(kept)
}

// Deliberately not the symmetric reverse of a pact: this opens no file for
// writing, so it cannot delete a `WARLOCK.md`. The document belongs to the
// project and is reviewed in the git diff like any other file.
//
// It does take the scopes off the entries it drops, because an entry is the
// only place a boundary is stated — the one operation here that can lose one,
// and a deliberate press on a directory rather than a side effect of a run. Who
// may ask is not settled here: both callers ask `closed_scopes_at_or_below`
// first. A `[[scope]]` record is not a boundary and is never pruned to what the
// surviving entries name, which is why the rebuild goes through `rebuilt_with`.
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
    Ok(manifest.rebuilt_with(
        manifest
            .entries()
            .iter()
            .filter(|entry| !at_or_below(entry.module(), &selected))
            .cloned(),
    ))
}

// Pure: the manifest comes back changed and nothing is saved, because the
// caller owns the one write of `pacts.toml`.
//
// Scope is not consulted, and that is the whole difference from a keypress. A
// boundary lives on an entry, so this drops boundaries along with the entries
// that held them — but the repository itself said the content is out, in a file
// that is committed beside the scopes, so there is no machine-side sigil to ask.
// The `[[scope]]` records come through untouched: every removal here is an
// `unpact_subtree`, and that carries them.
/// ```
/// use std::fs;
/// use warlock_engine::{Manifest, PactEntry, unpact_ignored};
///
/// let repo = tempfile::tempdir()?;
/// fs::create_dir_all(repo.path().join("vendor").join("acme"))?;
/// fs::create_dir_all(repo.path().join("crates"))?;
/// fs::write(repo.path().join(".warlockignore"), "vendor/\n")?;
///
/// let entry = |module: &str| PactEntry::new(".", module, format!("{module}/WARLOCK.md"));
/// let manifest = Manifest::with_entries([
///     entry("crates")?,
///     entry("vendor")?.with_scope("third-party"),
///     entry("vendor/acme")?,
/// ]);
///
/// let manifest = unpact_ignored(&manifest, repo.path(), repo.path())?;
///
/// let modules: Vec<&str> = manifest.entries().iter().map(PactEntry::module).collect();
/// assert_eq!(modules, ["crates"], "the scoped entry goes with the rest of the subtree");
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub fn unpact_ignored(
    manifest: &Manifest,
    root: impl AsRef<Path>,
    loaded: impl AsRef<Path>,
) -> Result<Manifest, Error> {
    let (root, loaded) = (root.as_ref(), loaded.as_ref());
    let loaded_module = to_manifest_path(root, loaded).map_err(|source| Error::Path {
        directory: loaded.to_path_buf(),
        path: loaded.to_path_buf(),
        source: Box::new(source),
    })?;

    let mut verdicts: BTreeMap<&str, bool> = BTreeMap::new();
    let mut excluded: BTreeSet<&str> = BTreeSet::new();
    for entry in manifest.entries() {
        // An entry above or beside the root this session loaded is left alone
        // even when the rules would exclude it. `refresh_subtree` already
        // refuses to reach past the loaded root, and reaching further here
        // would act on `.warlockignore` files this session never read. Dropping
        // a pact is the un-pact direction, which the mutating keys refuse
        // without a covering scope — so it is the direction to be narrow in.
        if !at_or_below(entry.module(), &loaded_module) {
            continue;
        }
        // `is_ignored` answers for a directory's own name only, so the
        // ancestors are asked too: gitignore does not let a rule re-include
        // anything below an excluded directory, and a walk that starts inside
        // one would never see the rule that removed it.
        for module in ancestry(entry.module(), &loaded_module) {
            let ignored = if let Some(&known) = verdicts.get(module) {
                known
            } else {
                let directory = from_manifest_path(root, module);
                let ignored = ignores::is_ignored(&directory)
                    .map_err(|source| Error::Walk { directory, source })?;
                verdicts.insert(module, ignored);
                ignored
            };
            if ignored {
                excluded.insert(module);
                break;
            }
        }
    }

    let mut kept = manifest.clone();
    for module in excluded {
        // `unpact_subtree` and nothing that walks: `pactable_directories`
        // returns an empty vector for a root `is_ignored` calls excluded, so a
        // walk-driven removal would find nothing to remove on exactly the
        // directories this function exists for.
        kept = unpact_subtree(from_manifest_path(root, module), root, &kept).map_err(|source| {
            Error::Path {
                directory: loaded.to_path_buf(),
                path: from_manifest_path(root, module),
                source: Box::new(source),
            }
        })?;
    }
    Ok(kept)
}

// Stops at the loaded root, and at `ROOT_MODULE` whatever the root: without the
// second stop a module with no `/` left to cut would name the repository root
// forever.
fn ancestry<'module>(module: &'module str, loaded: &str) -> Vec<&'module str> {
    let mut candidates = vec![module];
    let mut current = module;
    while current != loaded && current != ROOT_MODULE {
        current = match current.rsplit_once('/') {
            Some((above, _)) => above,
            None => ROOT_MODULE,
        };
        candidates.push(current);
    }
    candidates
}

// This entry point is never shown the directory's previous document — it hands
// `None` down as `carried` — so every file is described from source and the
// document is written over without being read first.
//
// A refresh does read it: `describe_and_grant` passes the page down under
// `AboveFailure::Skip`, and `Snapshot::assemble` keeps a line from it wherever the
// recorded digest still matches. That digest covers the file *and* the line
// (`hash::line_hash`), so what is reused is only ever a line warlock wrote
// about a file that has not moved. A document is warlock's to write: an edit
// somebody makes by hand is described again on the next run and disappears,
// silently and at the cost of that one file.
//
// Nothing is recorded either — no entry, no hash, no grant. Deciding what the
// manifest should then say needs the rest of the subtree and is the caller's.
/// ```
/// use std::fs;
/// use warlock_engine::{agent, document, Agent, Pacted, pact_directory};
///
/// /// The engine's own tests reach a model exactly like this: they don't. A
/// /// stub fill is the answer a pass would give with every slot filled in.
/// struct Canned;
///
/// impl Agent for Canned {
///     fn run(&self, request: &agent::Request) -> Result<agent::Response, agent::Error> {
///         Ok(agent::Response::new(document::stub_answer(request)))
///     }
/// }
///
/// let dir = tempfile::tempdir()?;
/// fs::write(dir.path().join("lib.rs"), "//! Core engine.\n")?;
///
/// let Pacted { document, problems, repairs, .. } = pact_directory(dir.path(), &Canned)?;
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
    pact_directory_watched(directory.as_ref(), None, agent, &mut Unwatched)
}

fn pact_directory_watched(
    directory: &Path,
    carried: Option<(&str, &BTreeMap<String, String>)>,
    agent: &dyn Agent,
    observer: &mut dyn Observer,
) -> Result<Pacted, Error> {
    let snapshot =
        Snapshot::take(directory).map_err(|source| Error::from_walk(directory, source))?;

    // Two kinds of pass, in this order, and the order is the whole design: one
    // per file that moved, then one over every line to say how they fit
    // together. A file's line is written without its siblings and routes no
    // worse for it (docs/warlock-per-file-isolation-measurement.md); what needs
    // the whole directory is the part that is about the whole directory.
    let Assembled {
        lines,
        hashes,
        mended,
        problems,
        ..
    } = snapshot.assemble(carried, agent, observer)?;

    let mut repairs: Vec<Repaired> = mended
        .iter()
        .map(|name| {
            let mend = document::Mend {
                field: format!("files[{name:?}]"),
                done: document::Mended::Supplied,
            };
            announce_repair(directory, mend, observer)
        })
        .collect();

    let Synthesised { fill, mends } = snapshot.fill(&lines, agent, observer)?;
    repairs.extend(
        mends
            .into_iter()
            .map(|mend| announce_repair(directory, mend, observer)),
    );

    let document = write_document(directory, &snapshot.render(&fill))?;

    Ok(Pacted {
        document,
        problems,
        repairs,
        hashes,
    })
}

fn announce_repair(
    directory: &Path,
    mend: document::Mend,
    observer: &mut dyn Observer,
) -> Repaired {
    observer.repaired(directory, &mend);
    Repaired {
        directory: directory.to_path_buf(),
        mend,
    }
}

// Written beside and renamed over, the same idiom as `Manifest::save`. A front
// end that quits mid-pact — killing the pass, restoring the terminal, never
// waiting for this function to come back — must not be able to leave half a
// document behind, and a rename is the only way to make that safe: the file is
// the old document or the new one, never a prefix of either. The temporary is
// named with a leading dot because hidden entries are skipped by every
// [`ignore`] walk in this crate, so it is in no tree, no subtree hash and no
// request for the moment it exists.
fn write_document(directory: &Path, text: &str) -> Result<PathBuf, Error> {
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
    Ok(document)
}

pub(crate) fn pactable_directories(root: &Path) -> Result<Vec<PathBuf>, Error> {
    walk::pactable_directories(root).map_err(|source| Error::from_walk(root, source))
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

    /// One file about to be described, and the fraction it is of the files this
    /// directory is paying for.
    ///
    /// `position` and `total` count the files a pass will be spent on and not
    /// the files in the directory: a file whose line came off the page
    /// unchanged cost nothing and is never announced here, so a directory of
    /// eighteen files with four moved counts to four. That is the denominator a
    /// progress bar can be drawn against, because it is the work that remains.
    ///
    /// Fired once per file, not once per attempt — a second attempt at the same
    /// file is [`rejected`](Observer::rejected), which is where a front end
    /// learns the first one was thrown away.
    fn describing(
        &mut self,
        directory: &Path,
        name: &str,
        bytes: u64,
        position: usize,
        total: usize,
    ) {
        let _ = (directory, name, bytes, position, total);
    }

    /// The handover to the synthesis pass: every line assembled, and the bytes
    /// they come to. One per directory, after the last
    /// [`describing`](Observer::describing).
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
    /// What each file hashed to as its line was settled, for the manifest to
    /// record beside the grant. A run that reuses a line records the hash it
    /// matched, so the two never drift.
    pub hashes: BTreeMap<String, String>,
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
#[non_exhaustive]
pub enum Refusal {
    Agent { source: agent::Error },
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Agent { source } => write!(f, "the model pass produced no answer: {source}"),
        }
    }
}

impl std::error::Error for Refusal {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Agent { source } => Some(source),
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
    pub(crate) fn from_walk(directory: &Path, source: walk::Error) -> Self {
        match source {
            walk::Error::Walk(source) => Self::Walk {
                directory: directory.to_path_buf(),
                source,
            },
            walk::Error::Path { path, source } => Self::Path {
                directory: directory.to_path_buf(),
                path,
                source,
            },
        }
    }

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
#[path = "tests/pact.rs"]
mod tests;
