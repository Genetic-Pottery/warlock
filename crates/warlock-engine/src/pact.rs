use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsStr;
use std::fmt;
use std::fs;
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::str::Utf8Error;

use ignore::WalkBuilder;

use crate::document::{self, Defect};
use crate::fitting::{
    PER_FILE_BYTE_CAP, Problem, byte_count, carry_hash, child_documents, measured, one_file,
    own_files,
};
use crate::ignores;
use crate::manifest::{ROOT_MODULE, temp_file_name, write_and_sync};
use crate::scope::valid_scope;
use crate::{
    Agent, Manifest, NodeState, PactEntry, agent, decide_state, from_manifest_path, hash, manifest,
    now_rfc3339, scope_opens_to, subtree_hash, to_manifest_path,
};

pub(crate) const MANIFEST_DIR: &str = ".warlock";

pub(crate) const DOCUMENT_FILE: &str = "WARLOCK.md";

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

    // Read here, where the manifest is, for the same reason the carry hashes
    // are: `describe_and_grant` is handed what a directory was last known to
    // hold and keeps knowing nothing about manifests.
    let line_hashes: BTreeMap<PathBuf, BTreeMap<String, String>> = stale
        .iter()
        .filter_map(|candidate| {
            let module = to_manifest_path(root, candidate).ok()?;
            let lines = manifest.entry(&module)?.lines()?;
            Some((candidate.clone(), lines.clone()))
        })
        .collect();

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
        match self.grant {
            Some(Grant {
                hash,
                at,
                carry,
                lines,
            }) => {
                let entry = entry.with_grant(hash, at);
                let entry = match carry {
                    Some(carry) => entry.with_carry_hash(carry),
                    None => entry,
                };
                if lines.is_empty() {
                    entry
                } else {
                    entry.with_lines(lines)
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

// Pure: the manifest comes back changed and nothing is saved, because the
// caller owns the one write of `pacts.toml`.
//
// Scope is not consulted, and that is the whole difference from a keypress. A
// scope lives on an entry, so this drops boundaries along with the entries that
// held them — but the repository itself said the content is out, in a file that
// is committed beside the scopes, so there is no machine-side sigil to ask.
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
    } = assemble_lines(directory, carried, agent, observer)?;

    let mut repairs: Vec<Repaired> = Vec::new();
    for name in &mended {
        // A line warlock wrote itself, carried the way a mended slot is: the
        // directory was described and granted, so it is a note about the run
        // and not a failure in it.
        let mend = document::Mend {
            field: format!("files[{name:?}]"),
            done: document::Mended::Supplied,
        };
        observer.repaired(directory, &mend);
        repairs.push(Repaired {
            directory: directory.to_path_buf(),
            mend,
        });
    }

    let Synthesised {
        fill,
        described,
        mends,
    } = synthesise(directory, &lines, agent, observer)?;
    for mend in mends {
        observer.repaired(directory, &mend);
        repairs.push(Repaired {
            directory: directory.to_path_buf(),
            mend,
        });
    }

    // The directory's name and not its path: the path is absolute, it is the
    // reader's home directory, and it would be committed.
    let name = directory.file_name().map_or_else(
        || directory.display().to_string(),
        |name| name.to_string_lossy().into_owned(),
    );
    let request = expected_for(directory)?;
    let expected = document::Expected::of(&request);
    let text = document::render(&name, &fill, &expected, &described);
    let document = write_document(directory, &text)?;

    Ok(Pacted {
        document,
        problems,
        repairs,
        hashes,
    })
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

// Names and sizes for `render`, which prints both in front of every line it
// writes. The text is nobody's here: the lines came from passes of their own.
fn expected_for(directory: &Path) -> Result<agent::Request, Error> {
    let mut files = Vec::new();
    for (name, path) in own_files(directory)? {
        let size = fs::metadata(&path).map_or(0, |found| found.len());
        files.push(agent::File::omitted(name, size));
    }
    Ok(agent::Request::new("", directory)
        .with_files(files)
        .with_child_documents(child_documents(directory)?))
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
/// The directory-wide slots, written from the assembled lines.
///
/// The second half of a per-file document. It is shown the lines and never the
/// source, and what it says is checked against warlock's own walk of the
/// directory rather than against the request — see `Expected::knows`, which is
/// what makes a pass that was shown no files checkable at all.
///
/// Ends in a fill either way, like every other road here: [`document::mend`] is
/// the floor under an exhausted loop, so a directory is never lost because its
/// synthesis could not be got right.
pub fn synthesise(
    directory: impl AsRef<Path>,
    lines: &BTreeMap<String, String>,
    agent: &dyn Agent,
    observer: &mut dyn Observer,
) -> Result<Synthesised, Error> {
    let directory = directory.as_ref();
    let described = measured(directory)?;

    // Names and sizes, and no text: the lines are the evidence and the files
    // are here so that a lookup can name one. `Expected` reads the sizes for
    // the fallback line, which is why they are measured rather than invented.
    let mut files = Vec::new();
    for (name, path) in own_files(directory)? {
        let size = std::fs::metadata(&path).map_or(0, |found| found.len());
        files.push(agent::File::omitted(name, size));
    }
    let children = child_documents(directory)?;

    // Announced before the first attempt waits on a model, so a front end's
    // clock counts what is being waited on rather than going quiet after the
    // last file. The bytes are the two things this request actually carries:
    // the lines, and the documents of the directories below. The files are in
    // it by name and size only — counting them here would report a payload
    // that was never sent.
    let carried = lines
        .values()
        .map(|line| byte_count(line.len()))
        .sum::<u64>()
        + children
            .iter()
            .map(|child| byte_count(child.text().len()))
            .sum::<u64>();
    observer.requesting(lines.len(), carried);

    let request = agent::Request::new(document::SYNTHESIS_PROMPT, directory)
        .with_files(files)
        .with_child_documents(children);
    let expected = document::Expected::of(&request);
    let name = directory.file_name().map_or_else(
        || directory.to_string_lossy().into_owned(),
        |name| name.to_string_lossy().into_owned(),
    );

    let mut rejected = Vec::new();
    let mut best = None;
    for attempt in 1..=document::ATTEMPTS {
        let asked = request
            .clone()
            .with_prompt(document::synthesis_instructions(
                &name, lines, &expected, &rejected,
            ));
        let answer = agent.run(&asked).map_err(|source| Error::Refused {
            directory: directory.to_path_buf(),
            cause: Refusal::Agent { source },
        })?;
        match document::accept_synthesis(answer.text(), lines, &expected, &described) {
            document::Accepted::Filled(fill) => {
                return Ok(Synthesised {
                    fill,
                    described,
                    mends: Vec::new(),
                });
            }
            document::Accepted::Defective { fill, defects } => {
                observer.rejected(directory, &defects, attempt, document::ATTEMPTS);
                best = Some(fill);
                rejected = defects;
            }
            document::Accepted::Unparsed(defect) => {
                let defects = vec![defect];
                observer.rejected(directory, &defects, attempt, document::ATTEMPTS);
                rejected = defects;
            }
        }
    }

    let unusable = best.unwrap_or_else(|| document::Fill {
        files: lines.clone(),
        ..document::Fill::default()
    });
    let (fill, mends) = document::mend(&unusable, &expected, &described);
    Ok(Synthesised {
        fill,
        described,
        mends,
    })
}

/// A directory's own slots, and what warlock had to write itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Synthesised {
    pub fill: document::Fill,
    /// The walk behind the check, kept because `render` needs the same one.
    pub described: document::Described,
    pub mends: Vec<document::Mend>,
}

/// Every line a directory's document needs, asking only about what moved.
///
/// The document is the store: a file whose bytes hash to what the manifest
/// recorded keeps the line already on the page, and every other file costs one
/// pass. Both halves have to agree before a line is reused — a hash with no
/// line on the page is a document somebody edited, and a line with no hash is a
/// file nobody has measured — and either way the answer is to ask again, which
/// costs a pass and never a wrong line.
///
/// ```
/// use std::cell::Cell;
/// use std::fs;
/// use warlock_engine::{Agent, Unwatched, agent, assemble_lines, file_hash};
///
/// struct Counting {
///     passes: Cell<usize>,
/// }
///
/// impl Agent for Counting {
///     fn run(&self, _request: &agent::Request) -> Result<agent::Response, agent::Error> {
///         self.passes.set(self.passes.get() + 1);
///         Ok(agent::Response::new(r#"{"line": "A line about one file alone."}"#))
///     }
/// }
///
/// let dir = tempfile::tempdir()?;
/// fs::write(dir.path().join("reading.rs"), "pub fn read_one() {}\n")?;
/// fs::write(dir.path().join("writing.rs"), "fn scratch() {}\n")?;
/// let agent = Counting { passes: Cell::new(0) };
///
/// // Nothing recorded: every file is asked about.
/// let first = assemble_lines(dir.path(), None, &agent, &mut Unwatched)?;
/// assert_eq!(agent.passes.get(), 2);
/// assert_eq!(first.asked, ["reading.rs", "writing.rs"]);
///
/// // The page and the hashes from that run, and one file changed under them.
/// fs::write(dir.path().join("writing.rs"), "fn scratch(at: usize) {}\n")?;
/// let page = first
///     .lines
///     .iter()
///     .map(|(path, line)| format!("- `{path}` (1 B) — {line}"))
///     .collect::<Vec<_>>()
///     .join("\n");
/// let page = format!("\n## Files\n\n{page}\n");
///
/// let again = assemble_lines(dir.path(), Some((&page, &first.hashes)), &agent, &mut Unwatched)?;
/// assert_eq!(agent.passes.get(), 3, "one changed file, one pass");
/// assert_eq!(again.asked, ["writing.rs"]);
/// assert_eq!(again.lines["reading.rs"], first.lines["reading.rs"]);
/// assert_eq!(again.hashes["writing.rs"], file_hash(dir.path().join("writing.rs"))?);
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub fn assemble_lines(
    directory: impl AsRef<Path>,
    carried: Option<(&str, &BTreeMap<String, String>)>,
    agent: &dyn Agent,
    observer: &mut dyn Observer,
) -> Result<Assembled, Error> {
    let directory = directory.as_ref();
    let files = own_files(directory)?;
    let (page, recorded) = match carried {
        Some((page, recorded)) => (document::lines_of(page), recorded.clone()),
        None => (BTreeMap::new(), BTreeMap::new()),
    };

    // Every file is settled against the page before the first pass runs, so
    // that `Observer::describing` can be handed a denominator: what a front end
    // needs is the count of files this directory will *pay* for, and that is
    // not known until every hash has been compared. Hashing the whole directory
    // up front costs one walk's worth of reads either way — the loop below did
    // the same reads, one file later.
    let planned: Vec<(String, PathBuf, Option<String>, Option<String>)> = files
        .into_iter()
        .map(|(name, path)| {
            // A file that cannot be hashed is a file nothing can be said to
            // know, so it is asked about rather than taken on trust. The hash is
            // left out of the record, which costs a pass next run and no
            // correctness.
            let hash = crate::hash::file_hash(&path).ok();
            let unmoved = hash
                .as_ref()
                .zip(recorded.get(&name))
                .is_some_and(|(now, before)| now == before);
            let kept = unmoved.then(|| page.get(&name).cloned()).flatten();
            (name, path, hash, kept)
        })
        .collect();

    let paying = planned.iter().filter(|(.., kept)| kept.is_none()).count();

    let mut assembled = Assembled::default();
    let mut position = 0;
    for (name, path, hash, kept) in planned {
        if let Some(line) = kept {
            assembled.lines.insert(name.clone(), line);
            assembled.kept.push(name.clone());
        } else {
            position += 1;
            let bytes = fs::metadata(&path).map_or(0, |file| file.len());
            observer.describing(directory, &name, bytes, position, paying);
            let described = describe_file(directory, &name, agent, observer)?;
            if described.mended {
                assembled.mended.push(name.clone());
            }
            assembled.problems.extend(described.problem);
            assembled.lines.insert(name.clone(), described.line);
            assembled.asked.push(name.clone());
        }

        if let Some(hash) = hash {
            assembled.hashes.insert(name, hash);
        }
    }
    Ok(assembled)
}

/// A directory's file lines, and what each one cost.
#[derive(Debug, Default)]
pub struct Assembled {
    pub lines: BTreeMap<String, String>,
    /// What each file hashed to as its line was settled, for the manifest to
    /// record. A file that could not be hashed is absent and will be asked
    /// about again.
    pub hashes: BTreeMap<String, String>,
    /// Files a pass was paid for this run.
    pub asked: Vec<String>,
    /// Files whose line came off the page unchanged.
    pub kept: Vec<String>,
    /// Files whose every attempt was spent, so warlock wrote the line.
    pub mended: Vec<String>,
    /// Files the pass was shown a name and a size for, and why.
    pub problems: Vec<Problem>,
}

/// Describe one file: the unit of work per-file granularity is built on.
///
/// The asking is [`document::ATTEMPTS`] deep like a directory's, and ends the
/// same way — with a line warlock wrote itself rather than a refusal, because
/// one unusable answer about one file is no reason to lose the directory it
/// sits in. What it cannot do is invent a file: a name that is not there is an
/// error, since the caller walked the directory to get it.
///
/// ```
/// use std::fs;
/// use warlock_engine::{Agent, Unwatched, agent, describe_file, document};
///
/// struct Lining;
///
/// impl Agent for Lining {
///     fn run(&self, request: &agent::Request) -> Result<agent::Response, agent::Error> {
///         assert!(request.prompt().starts_with(document::FILE_PROMPT));
///         assert_eq!(request.files().len(), 1, "one file, one pass");
///         Ok(agent::Response::new(
///             r#"{"line": "The reading half: one entry point and the type it hands back."}"#,
///         ))
///     }
/// }
///
/// let dir = tempfile::tempdir()?;
/// fs::write(dir.path().join("reading.rs"), "pub fn read_one() {}\n")?;
///
/// let described = describe_file(dir.path(), "reading.rs", &Lining, &mut Unwatched)?;
/// assert_eq!(
///     described.line,
///     "The reading half: one entry point and the type it hands back."
/// );
/// assert!(!described.mended, "the pass answered, so warlock wrote nothing");
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub fn describe_file(
    directory: impl AsRef<Path>,
    name: &str,
    agent: &dyn Agent,
    observer: &mut dyn Observer,
) -> Result<DescribedFile, Error> {
    let directory = directory.as_ref();
    let (request, described, problem) = one_file(document::FILE_PROMPT, directory, name)?;
    let expected = document::Expected::of(&request);

    let mut rejected = Vec::new();
    for attempt in 1..=document::ATTEMPTS {
        let asked = request
            .clone()
            .with_prompt(document::file_instructions(name, &rejected));
        // A transport failure ends it at once, the same as a directory's pass:
        // a pass that produced no answer is not a pass that produced a wrong
        // one, and retrying a missing `claude` finds it still missing.
        let answer = agent.run(&asked).map_err(|source| Error::Refused {
            directory: directory.to_path_buf(),
            cause: Refusal::Agent { source },
        })?;
        match document::accept_file(answer.text(), name, &expected, &described) {
            Ok(line) => {
                return Ok(DescribedFile {
                    line,
                    mended: false,
                    problem,
                });
            }
            Err(defects) => {
                observer.rejected(directory, &defects, attempt, document::ATTEMPTS);
                rejected = defects;
            }
        }
    }

    Ok(DescribedFile {
        line: document::file_fallback(name, &expected, &described),
        mended: true,
        problem,
    })
}

/// One file's line, and whether a pass wrote it.
#[derive(Debug)]
pub struct DescribedFile {
    pub line: String,
    /// True where every attempt was spent and warlock wrote the line itself
    /// from the file's name, size and declared symbols.
    pub mended: bool,
    /// Why the pass was shown a name and a size instead of the file, where it
    /// was: too large for the per-file cap, or unreadable.
    pub problem: Option<Problem>,
}

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
    use std::fs;
    use std::path::{Path, PathBuf};

    use super::{
        DOCUMENT_FILE, Failure, Observer, PactedSubtree, Pacting, Refusal, Unviewable, Unwatched,
        Viewed, assemble_lines, closed_scopes_at_or_below, describe_file, pact_directory,
        pact_subtree, pactable_directories, refresh_subtree, synthesise, unpact_ignored,
        unpact_subtree, view_file,
    };
    use crate::document::{self, STAMP};

    use crate::fitting::Omission;
    use crate::ignores;
    use crate::{
        Agent, Loaded, Manifest, NodeState, PactEntry, agent, decide_state, from_manifest_path,
        load_tree, manifest, subtree_hash,
    };

    struct Canned {
        text: Option<String>,
        seen: std::cell::RefCell<Vec<agent::Request>>,
    }

    impl Canned {
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

    // The pass that happens once per directory, whatever its files cost. A
    // directory is described by one synthesis pass and as many per-file passes
    // as it has files that moved, so counting anything else counts files.
    fn is_document_pass(request: &agent::Request) -> bool {
        request.prompt().starts_with(document::SYNTHESIS_PROMPT)
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

    struct Lining {
        answer: String,
        passes: std::cell::Cell<usize>,
    }

    impl Lining {
        fn saying(answer: impl Into<String>) -> Self {
            Self {
                answer: answer.into(),
                passes: std::cell::Cell::new(0),
            }
        }
    }

    impl Agent for Lining {
        fn run(&self, _request: &agent::Request) -> Result<agent::Response, agent::Error> {
            self.passes.set(self.passes.get() + 1);
            Ok(agent::Response::new(self.answer.clone()))
        }
    }

    fn one_file_directory() -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("a temporary directory");
        write(
            dir.path(),
            "reading.rs",
            "pub fn read_one() -> Reader { Reader }\n",
        );
        dir
    }

    fn page_of(lines: &[(&str, &str)]) -> String {
        let rows: Vec<String> = lines
            .iter()
            .map(|(path, line)| format!("- `{path}` (1 B) — {line}"))
            .collect();
        format!("\n## Files\n\n{}\n", rows.join("\n"))
    }

    #[test]
    fn synthesis_is_shown_the_lines_and_checked_against_the_directory() {
        // The pass sees no source at all, so every name it uses is one the
        // request cannot vouch for. What makes the answer checkable is
        // warlock's own walk, and what makes this test worth having is that the
        // answer names `read_one` — a symbol in the file and in no line.
        let dir = one_file_directory();
        let agent = Lining::saying(
            r#"{"purpose": "A directory of one reading file, for the tests below it.",
                "structure": [{"line": "`reading.rs` is the only file here.", "names": ["reading.rs"]}],
                "lookups": [{"for": "reading one record", "open": "reading.rs", "symbol": "read_one"}]}"#,
        );
        let lines = [(
            "reading.rs".to_owned(),
            "The reading half of the fixture.".to_owned(),
        )]
        .into_iter()
        .collect();

        let synthesised =
            synthesise(dir.path(), &lines, &agent, &mut Unwatched).expect("a fill either way");

        assert_eq!(agent.passes.get(), 1, "a clean answer is taken at once");
        assert!(synthesised.mends.is_empty(), "{:?}", synthesised.mends);
        assert_eq!(
            synthesised.fill.files, lines,
            "the lines are the caller's and pass through untouched",
        );
        assert_eq!(
            synthesised.fill.lookups[0].symbol.as_deref(),
            Some("read_one"),
            "a symbol no line spells is still checkable against the walk",
        );
    }

    #[test]
    fn synthesis_that_cannot_be_got_right_is_mended_rather_than_lost() {
        let dir = one_file_directory();
        let agent = Lining::saying(r#"{"purpose": "", "structure": [], "lookups": []}"#);
        let lines = [(
            "reading.rs".to_owned(),
            "The reading half of the fixture.".to_owned(),
        )]
        .into_iter()
        .collect();

        let synthesised =
            synthesise(dir.path(), &lines, &agent, &mut Unwatched).expect("a fill either way");

        assert_eq!(agent.passes.get(), document::ATTEMPTS);
        assert_eq!(
            synthesised
                .mends
                .iter()
                .map(|mend| mend.field.as_str())
                .collect::<Vec<_>>(),
            ["purpose"],
            "{:?}",
            synthesised.mends,
        );
        assert!(!synthesised.fill.purpose.is_empty());
    }

    #[test]
    fn a_hash_with_no_line_on_the_page_is_asked_about_again() {
        // The document was edited by hand, or written by a warlock that did not
        // record lines. The hash says the file has not moved and there is still
        // nothing to reuse, so the pass runs.
        let dir = one_file_directory();
        let agent = Lining::saying(r#"{"line": "A line about one file alone."}"#);
        let hash = crate::hash::file_hash(dir.path().join("reading.rs")).expect("hashes");
        let recorded = [("reading.rs".to_owned(), hash)].into_iter().collect();

        let assembled = assemble_lines(dir.path(), Some(("", &recorded)), &agent, &mut Unwatched)
            .expect("lines");

        assert_eq!(assembled.asked, ["reading.rs"]);
        assert!(assembled.kept.is_empty());
        assert_eq!(agent.passes.get(), 1);
    }

    #[test]
    fn a_line_with_no_hash_behind_it_is_asked_about_again() {
        // The other half of the same rule: a page says what the file was, and
        // nothing says the file still is that. Trusting the page here is how a
        // document outlives the code it describes.
        let dir = one_file_directory();
        let agent = Lining::saying(r#"{"line": "A line about one file alone."}"#);
        let page = page_of(&[("reading.rs", "The line already on the page.")]);

        let assembled = assemble_lines(
            dir.path(),
            Some((&page, &BTreeMap::new())),
            &agent,
            &mut Unwatched,
        )
        .expect("lines");

        assert_eq!(assembled.asked, ["reading.rs"]);
        assert_eq!(agent.passes.get(), 1);
        assert_eq!(
            assembled.lines["reading.rs"], "A line about one file alone.",
            "the answer, not the page",
        );
    }

    #[test]
    fn a_file_the_page_and_the_hashes_agree_on_costs_nothing() {
        let dir = one_file_directory();
        let agent = Lining::saying(r#"{"line": "A line no pass should be asked for."}"#);
        let hash = crate::hash::file_hash(dir.path().join("reading.rs")).expect("hashes");
        let recorded = [("reading.rs".to_owned(), hash.clone())]
            .into_iter()
            .collect();
        let page = page_of(&[("reading.rs", "The line already on the page.")]);

        let assembled =
            assemble_lines(dir.path(), Some((&page, &recorded)), &agent, &mut Unwatched)
                .expect("lines");

        assert_eq!(agent.passes.get(), 0, "the run paid for nothing");
        assert_eq!(assembled.kept, ["reading.rs"]);
        assert!(assembled.asked.is_empty());
        assert_eq!(
            assembled.lines["reading.rs"],
            "The line already on the page."
        );
        assert_eq!(
            assembled.hashes["reading.rs"], hash,
            "recorded again as it stands"
        );
    }

    #[test]
    fn a_file_warlock_had_to_write_itself_is_named_as_such() {
        let dir = one_file_directory();
        let agent = Lining::saying("prose where an object was asked for");

        let assembled = assemble_lines(dir.path(), None, &agent, &mut Unwatched).expect("lines");

        assert_eq!(assembled.asked, ["reading.rs"]);
        assert_eq!(assembled.mended, ["reading.rs"]);
        assert!(assembled.lines["reading.rs"].contains("reading.rs"));
    }

    #[test]
    fn a_file_whose_line_is_never_usable_is_written_by_warlock_rather_than_refused() {
        let dir = one_file_directory();
        let over = "x".repeat(document::ENTRY_CHARS + 40);
        let agent = Lining::saying(format!("{{\"line\": \"{over}\"}}"));

        let described = describe_file(dir.path(), "reading.rs", &agent, &mut Unwatched)
            .expect("a line either way");

        assert_eq!(
            agent.passes.get(),
            document::ATTEMPTS,
            "asked in full first"
        );
        assert!(described.mended);
        assert!(
            described.line.contains("reading.rs"),
            "{:?}",
            described.line
        );
        assert!(
            described.line.contains("read_one"),
            "the fallback is the file's own declared names: {:?}",
            described.line,
        );
    }

    #[test]
    fn a_file_answered_with_prose_is_mended_where_a_directory_would_be_refused() {
        // `pact_directory` refuses an answer that was never an object, because
        // there is no slot in prose to repair from and the whole document is at
        // stake. One file is not: warlock knows its name, its size and what it
        // declares, so losing the directory over one file's punctuation is the
        // trade brief 16 was written against.
        let dir = one_file_directory();
        let agent = Lining::saying("Here is some prose instead of the object you asked for.");

        let described = describe_file(dir.path(), "reading.rs", &agent, &mut Unwatched)
            .expect("a line either way");

        assert!(described.mended);
        assert!(
            described.line.contains("reading.rs"),
            "{:?}",
            described.line
        );
    }

    #[test]
    fn a_file_that_is_not_there_is_an_error_and_not_a_line_about_nothing() {
        let dir = one_file_directory();
        let agent = Lining::saying(r#"{"line": "a line about a file that does not exist"}"#);

        let error = describe_file(dir.path(), "writing.rs", &agent, &mut Unwatched)
            .expect_err("the caller walked the directory to get this name");

        assert!(matches!(error, super::Error::Walk { .. }), "{error:?}");
        assert_eq!(agent.passes.get(), 0, "nothing was asked");
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
            [(
                "crates/tui/src".to_owned(),
                "purpose".to_owned(),
                document::Mended::Supplied,
            )],
            "one value per slot, naming the directory and the slot in `Defect`'s own spelling",
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
        // The synthesis pass and not the first: a per-file pass is shown one
        // file and no child at all, and `## Directories` is written from a
        // child's own document by the pass that writes the directory's slots.
        let synthesis = seen
            .iter()
            .find(|request| is_document_pass(request))
            .expect("a directory is synthesised");
        assert_eq!(
            synthesis
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
            .filter(|request| is_document_pass(request))
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
                // The pass that happens once per directory: a directory also
                // costs one pass per file that moved, and those are not what an
                // offer is counted against.
                .filter(|request| is_document_pass(request))
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

    // Announcing the passes themselves: a file at a time, then the handover to
    // the one that fits them together.

    #[derive(Default)]
    struct Weighing {
        described: Vec<(String, u64, usize, usize)>,
        requested: Vec<(usize, u64)>,
    }

    impl Observer for Weighing {
        fn starting(&mut self, _directory: &Path, _position: usize, _total: usize) -> Pacting {
            Pacting::Continue
        }

        fn describing(
            &mut self,
            _directory: &Path,
            name: &str,
            bytes: u64,
            position: usize,
            total: usize,
        ) {
            self.described
                .push((name.to_owned(), bytes, position, total));
        }

        fn requesting(&mut self, files: usize, bytes: u64) {
            self.requested.push((files, bytes));
        }
    }

    #[test]
    fn every_file_a_directory_pays_for_is_announced_once_and_counted_to_the_same_total() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        write(dir.path(), "reading.rs", "pub fn read() {}\n");
        write(dir.path(), "writing.rs", "pub fn write() {}\n");
        let agent = Lining::saying(r#"{"line": "A line about one file alone."}"#);
        let mut watched = Weighing::default();

        assemble_lines(dir.path(), None, &agent, &mut watched).expect("lines");

        assert_eq!(
            watched.described,
            [
                ("reading.rs".to_owned(), 17, 1, 2),
                ("writing.rs".to_owned(), 18, 2, 2),
            ],
            "a file, its size, and where it is in the files being paid for",
        );
    }

    #[test]
    fn a_file_taken_off_the_page_is_never_announced_and_never_counted() {
        // The denominator is the run that is left, which is what makes it worth
        // drawing a bar against: a directory of two files with one moved counts
        // to one, not to two with the first already behind it.
        let dir = tempfile::tempdir().expect("a temporary directory");
        write(dir.path(), "reading.rs", "pub fn read() {}\n");
        write(dir.path(), "writing.rs", "pub fn write() {}\n");
        let agent = Lining::saying(r#"{"line": "A line about one file alone."}"#);
        let hash = crate::hash::file_hash(dir.path().join("reading.rs")).expect("hashes");
        let recorded = [("reading.rs".to_owned(), hash)].into_iter().collect();
        let page = page_of(&[("reading.rs", "The line already on the page.")]);
        let mut watched = Weighing::default();

        assemble_lines(dir.path(), Some((&page, &recorded)), &agent, &mut watched).expect("lines");

        assert_eq!(watched.described, [("writing.rs".to_owned(), 18, 1, 1)]);
    }

    #[test]
    fn a_file_asked_about_twice_is_announced_once() {
        // Attempts are `rejected`, not a second `describing`: a bar that moved
        // on a retry would be counting the asking rather than the work.
        let dir = one_file_directory();
        let agent = Lining::saying("prose where an object was asked for");
        let mut watched = Weighing::default();

        let assembled = assemble_lines(dir.path(), None, &agent, &mut watched).expect("lines");

        assert_eq!(assembled.mended, ["reading.rs"], "every attempt was spent");
        assert!(agent.passes.get() > 1, "the retries this is about happened");
        assert_eq!(watched.described.len(), 1);
    }

    #[test]
    fn the_handover_counts_the_lines_and_the_documents_below_and_not_the_files() {
        // A directory with no file of its own still carries its children's
        // documents, and a handover reporting no bytes over a pass that is
        // about to wait on a model is the panel's clock labelled with a lie.
        let dir = tempfile::tempdir().expect("a temporary directory");
        write(dir.path(), "src/lib.rs", "pub fn one() {}\n");
        write(dir.path(), "src/WARLOCK.md", "# src\n\nA document below.\n");
        let agent = Lining::saying("prose where an object was asked for");
        let mut watched = Weighing::default();

        synthesise(dir.path(), &BTreeMap::new(), &agent, &mut watched).expect("a fill");

        assert_eq!(
            watched.requested,
            [(0, 25)],
            "no lines of its own, and the bytes of the document below it",
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

    // The directories are made for real, because `is_ignored` reads an absent
    // path as not excluded: a fixture of names alone would have every one of
    // these tests pass without a rule ever being consulted.
    fn ignoring(rules: &str, directories: &[&str]) -> tempfile::TempDir {
        let repo = tempfile::tempdir().expect("a temporary directory");
        for directory in directories {
            fs::create_dir_all(repo.path().join(directory)).expect("creates a fixture directory");
        }
        write(repo.path(), ignores::FILENAME, rules);
        repo
    }

    #[test]
    fn an_excluded_entry_goes_and_takes_its_scope_with_it() {
        let repo = ignoring("vendor/\n", &["vendor", "crates/engine"]);
        let manifest = with_scopes(
            &pacted(&["crates/engine", "vendor"]),
            &[("vendor", "third-party")],
        );

        let left = unpact_ignored(&manifest, repo.path(), repo.path()).expect("the rules are read");

        assert_eq!(
            scopes(&left),
            [("crates/engine", None)],
            "the repository said the content is out, and a scope is no reason \
             to keep the entry that held it",
        );
    }

    #[test]
    fn an_entry_below_an_excluded_directory_goes_though_that_directory_has_no_entry() {
        let repo = ignoring("vendor/\n", &["vendor/acme/src", "crates"]);
        let manifest = pacted(&["crates", "vendor/acme", "vendor/acme/src"]);

        let left = unpact_ignored(&manifest, repo.path(), repo.path()).expect("the rules are read");

        assert_eq!(
            modules(&left),
            ["crates"],
            "the rule names an ancestor nothing pacted, so the ancestors are \
             what gets asked, not the entries alone",
        );
    }

    #[test]
    fn a_negated_rule_keeps_the_directory_it_re_includes() {
        // The negation sits beside the exclusion rather than under it: gitignore
        // will not re-include anything below a directory already excluded.
        let repo = ignoring("vendor/*\n!vendor/keep\n", &["vendor/acme", "vendor/keep"]);
        let manifest = pacted(&["vendor/acme", "vendor/keep"]);

        let left = unpact_ignored(&manifest, repo.path(), repo.path()).expect("the rules are read");

        assert_eq!(
            modules(&left),
            ["vendor/keep"],
            "exclusion is whatever the matcher says, negation included",
        );
    }

    #[test]
    fn a_manifest_with_nothing_excluded_comes_back_equal() {
        let repo = ignoring("target/\n", &["crates/engine", "crates/tui", "target"]);
        let manifest = with_scopes(
            &pacted(&["crates/engine", "crates/tui"]),
            &[("crates/engine", "data-plane")],
        );

        let left = unpact_ignored(&manifest, repo.path(), repo.path()).expect("the rules are read");

        assert_eq!(
            left, manifest,
            "grant and scope intact: a rule that matches nothing pacted is not \
             an edit to the manifest",
        );
    }

    #[test]
    fn an_excluded_entry_outside_the_loaded_root_is_left_alone() {
        let repo = ignoring("vendor/\n", &["vendor", "crates/engine"]);
        let manifest = pacted(&["crates/engine", "vendor"]);

        let left = unpact_ignored(&manifest, repo.path(), repo.path().join("crates"))
            .expect("the rules are read");

        assert_eq!(
            modules(&left),
            ["crates/engine", "vendor"],
            "`vendor` is excluded and stays anyway: this session loaded \
             `crates` and never read the rules above it",
        );
    }

    #[test]
    fn removing_entries_writes_nothing_to_disk() {
        let repo = ignoring("vendor/\n", &["vendor", "crates"]);
        let manifest = pacted(&["crates", "vendor"]);
        manifest.save(repo.path()).expect("saves");
        let path = repo.path().join(".warlock").join("pacts.toml");
        let before = fs::read(&path).expect("a readable manifest");

        let left = unpact_ignored(&manifest, repo.path(), repo.path()).expect("the rules are read");

        assert_eq!(modules(&left), ["crates"], "an entry really was removed");
        assert_eq!(
            fs::read(&path).expect("a readable manifest"),
            before,
            "and the file on disk is byte-identical: the caller owns the write",
        );
    }

    #[test]
    fn rules_that_cannot_be_used_fail_rather_than_guessing() {
        // A range that runs backwards: a glob the matcher will not compile.
        let repo = ignoring("a[z-a]\n", &["crates/engine"]);
        let manifest = pacted(&["crates/engine"]);

        let error = unpact_ignored(&manifest, repo.path(), repo.path())
            .expect_err("a manifest cannot be cleaned against rules that cannot be read");

        assert!(matches!(error, super::Error::Walk { .. }), "{error:?}");
        assert!(
            error.to_string().contains(ignores::FILENAME),
            "the one line back names the file to go and fix: {error}",
        );
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
            .filter(|request| is_document_pass(request))
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
            agent
                .seen
                .borrow()
                .iter()
                .filter(|request| is_document_pass(request))
                .count(),
            3,
            "one synthesis pass per directory that had something new to read — \
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
    #[derive(Default)]
    struct Probing {
        said: Vec<String>,
    }

    impl Observer for Probing {
        fn starting(&mut self, directory: &Path, _position: usize, _total: usize) -> Pacting {
            self.said.push(format!("starting {}", directory.display()));
            Pacting::Continue
        }

        fn unchanged(&mut self, directory: &Path) {
            self.said.push(format!("unchanged {}", directory.display()));
        }

        fn skipped(&mut self, directory: &Path, below: &Path) {
            self.said.push(format!(
                "skipped {} below {}",
                directory.display(),
                below.display()
            ));
        }

        fn documented(&mut self, directory: &Path) {
            self.said
                .push(format!("documented {}", directory.display()));
        }
    }

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
        let mut probe = Probing::default();
        let PactedSubtree {
            manifest, failures, ..
        } = refresh_subtree(&engine, repo.path(), &manifest, &agent, &mut probe)
            .expect("a hash nobody can take is a directory to describe, not an error");

        assert_eq!(
            described_by(&agent, repo.path()),
            ["crates/engine/tests"],
            "no hash is no answer to `is this still the content it was granted \
             for`, so the directory holding the unreadable file is described",
        );
        assert!(
            probe
                .said
                .contains(&format!("unchanged {}", engine.display())),
            "and the one above it is offered and then cut off: its own files and \
             its children's documents are where they were, so re-describing it \
             would buy the same document twice. Being unhashable is what keeps \
             it from a grant, not what earns it a pass: {:?}",
            probe.said,
        );
        // And then it plays out exactly as the module docs say it does: phase
        // two hashes them again, that hash fails again, and each lands as a
        // `Failure::Hash` with an ungranted entry — yellow, which is the honest
        // outcome for a directory something is really wrong with, whether or
        // not this run paid for a pass over it.
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

    fn line_of(repo: &Path, module: &str, file: &str) -> String {
        crate::hash::file_hash(repo.join(module).join(file)).expect("the fixture is readable")
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
             [pact.lines]\n\
             \"lib.rs\" = \"{src_line}\"\n\
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
             [pact.lines]\n\
             \"Cargo.toml\" = \"{root_line}\"\n\
             \n\
             [[pact]]\n\
             module = \"crates/engine/src/inner\"\n\
             document = \"crates/engine/src/inner/WARLOCK.md\"\n\
             granted_hash = \"{inner}\"\n\
             granted_at = \"{granted_at}\"\n\
             carry_hash = \"{inner_carry}\"\n\
             \n\
             [pact.lines]\n\
             \"deep.rs\" = \"{inner_line}\"\n\
             \n\
             [[pact]]\n\
             module = \"crates/engine/tests\"\n\
             document = \"crates/engine/tests/WARLOCK.md\"\n\
             granted_hash = \"{tests}\"\n\
             granted_at = \"{granted_at}\"\n\
             carry_hash = \"{tests_carry}\"\n\
             \n\
             [pact.lines]\n\
             \"it.rs\" = \"{tests_line}\"\n",
            root = hash_of(repo, "crates/engine"),
            src = hash_of(repo, "crates/engine/src"),
            inner = hash_of(repo, "crates/engine/src/inner"),
            tests = hash_of(repo, "crates/engine/tests"),
            root_carry = carry_of(repo, "crates/engine"),
            root_line = line_of(repo, "crates/engine", "Cargo.toml"),
            src_carry = carry_of(repo, "crates/engine/src"),
            src_line = line_of(repo, "crates/engine/src", "lib.rs"),
            inner_carry = carry_of(repo, "crates/engine/src/inner"),
            inner_line = line_of(repo, "crates/engine/src/inner", "deep.rs"),
            tests_carry = carry_of(repo, "crates/engine/tests"),
            tests_line = line_of(repo, "crates/engine/tests", "it.rs"),
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
             [pact.lines]\n\
             \"lib.rs\" = \"{src_line}\"\n\
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
             [pact.lines]\n\
             \"Cargo.toml\" = \"{root_line}\"\n\
             \n\
             [[pact]]\n\
             module = \"crates/engine/src/inner\"\n\
             document = \"crates/engine/src/inner/WARLOCK.md\"\n\
             granted_hash = \"{inner}\"\n\
             granted_at = \"{pacted_at}\"\n\
             carry_hash = \"{inner_carry}\"\n\
             \n\
             [pact.lines]\n\
             \"deep.rs\" = \"{inner_line}\"\n\
             \n\
             [[pact]]\n\
             module = \"crates/engine/tests\"\n\
             document = \"crates/engine/tests/WARLOCK.md\"\n\
             granted_hash = \"{tests}\"\n\
             granted_at = \"{refreshed_at}\"\n\
             carry_hash = \"{tests_carry}\"\n\
             \n\
             [pact.lines]\n\
             \"it.rs\" = \"{tests_line}\"\n",
            root = hash_of(repo, "crates/engine"),
            src = hash_of(repo, "crates/engine/src"),
            inner = hash_of(repo, "crates/engine/src/inner"),
            tests = hash_of(repo, "crates/engine/tests"),
            root_carry = carry_of(repo, "crates/engine"),
            root_line = line_of(repo, "crates/engine", "Cargo.toml"),
            src_carry = carry_of(repo, "crates/engine/src"),
            src_line = line_of(repo, "crates/engine/src", "lib.rs"),
            inner_carry = carry_of(repo, "crates/engine/src/inner"),
            inner_line = line_of(repo, "crates/engine/src/inner", "deep.rs"),
            tests_carry = carry_of(repo, "crates/engine/tests"),
            tests_line = line_of(repo, "crates/engine/tests", "it.rs"),
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
