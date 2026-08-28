//! Building a [`Tree`] from a real directory.
//!
//! Section 5 of the design doc makes the tree of module documents the
//! interface, and section 12's modular invocation rule says the scope of a run
//! is wherever it was invoked — there is no privileged root. Both fall out of
//! one function, [`load_tree`]: the tree it returns is rooted at the working
//! directory it was given, while the single manifest that colours the nodes is
//! read from the repository root found by walking *up* from there — the
//! nearest ancestor holding a `.git/` directory, whose manifest, if it has one
//! at all, is at `<root>/.warlock/pacts.toml`.
//!
//! What makes a node:
//!
//! * Every directory the walk reaches is a node, the directory it starts at
//!   included. Nothing is pruned for being undocumented.
//! * A directory that directly contains a `WARLOCK.md` is a module node, and
//!   carries it as [`Node::document`]. Nothing here reads that file — Warlock
//!   cares only that one exists.
//! * A directory with no document of its own is a node with `document: None`:
//!   an ordinary directory that has no documentation yet. Showing fewer nodes
//!   than the walk found is a view's business, not the loader's. A `README.md`
//!   makes no difference to any of this: it is the project's file, not
//!   Warlock's, so it documents no node and is listed like any other file.
//! * The files sitting directly in a directory come back on that node as
//!   [`Node::files`], in path order, gathered as the same single pass meets
//!   them — including the directory's own `WARLOCK.md`. They are a listing and
//!   nothing else: a file is not a node, has no state of its own, and is no
//!   input to any hash.
//!
//! What the walk skips is not a list kept in this file: traversal is the
//! [`ignore`] crate, so `.gitignore` at every level, hidden files and
//! directories (`.git/` among them) and global excludes are all honoured as git
//! honours them — and files inherit every one of those rules by coming out of
//! the same walk, so an ignored file is as absent from a node's listing as an
//! ignored directory is from the tree. `.warlock/` is pruned unconditionally on
//! top of that, symlinks are never followed and never listed, and both
//! directories and files come out in name order so two loads of an unchanged
//! tree are equal values.
//!
//! # Why `.warlockignore` prunes nothing here
//!
//! The repository's own `.warlockignore` is the one rule this walk deliberately
//! does not obey. Everywhere else it removes content — nothing it covers is
//! pacted, requested or hashed — but removing it from *this* walk would delete
//! the directory from the screen, and a reader who cannot see the folder of
//! images cannot see that Warlock is right not to cover it. Silence would look
//! like a bug. So every excluded directory keeps its row, keeps its files, and
//! loads [`NodeState::Unpacted`] like any other directory nobody pacted; it is
//! simply marked [`Node::ignored`], which says *why* it is gray and lets a
//! front end refuse to pact it without asking the filesystem. No state, no
//! colour and no shade is added.
//!
//! The mark is made of the same matcher rather than a second one: the walk
//! above runs again over the same root with `.warlockignore` registered, and a
//! directory the first pass found and the second did not is exactly a directory
//! the rules exclude. The root the load was handed is asked about separately,
//! because a walker does not apply the rules to its own root.
//!
//! State is not presence in the manifest: presence only decides whether the
//! question is worth asking. A node the manifest names is hashed over its own
//! subtree with [`subtree_hash`], and its colour is [`decide_state`]'s verdict
//! on that pair — so a granted hash somebody wrote by hand, still matching what
//! is on disk, comes back [`NodeState::PactedFresh`]. A node with no entry is
//! [`NodeState::Unpacted`] and is never hashed at all: unmanaged directories
//! cost a load nothing.
//!
//! Hashing can fail — a file that cannot be read is deliberately fatal to a
//! digest rather than skipped, see [`hash`](crate::hash) — and one such file is
//! one node's problem, not the tree's. So a node whose hash failed is coloured
//! [`NodeState::PactedStale`] (its content is unknown, and unknown is stale),
//! the failure is recorded in [`Loaded::problems`] with the node it happened at,
//! and the walk carries on colouring everything else correctly. No error text
//! and no partial read ever reaches a hash: the failed digest is simply not
//! there.
//!
//! A node's scope arrives the same way its state does: read off the entry the
//! manifest holds for that directory, and put on [`Node::scope`] as its own —
//! never an ancestor's, which is [`scope_covering`](crate::scope_covering)'s
//! question and not a node's. A scope [`validate_scope`] refuses is the other
//! non-fatal thing a load can report: the node keeps its state, its document,
//! its files and its children, reads as unscoped, and the string is named in a
//! [`Problem`] along with the rule it broke. Nothing is corrected — a scope
//! gates nothing, and taking a whole tree gray over one typo in a label would
//! cost far more than the label is worth.

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsStr;
use std::fmt;
use std::path::{Component, Path, PathBuf};

use ignore::WalkBuilder;

use crate::{
    HashError, Manifest, ManifestError, Node, NodeState, ScopeRule, Tree, decide_state, ignores,
    subtree_hash, to_manifest_path, validate_scope,
};

/// The directory whose presence marks a repository root. Git's own, read as a
/// name on disk and nothing more.
const GIT_DIR: &str = ".git";

/// The directory Warlock keeps its manifest in, and which is itself never part
/// of the tree.
const MANIFEST_DIR: &str = ".warlock";

/// The file whose presence in a directory makes that directory a module node:
/// Warlock's own document, and the one place in the workspace where its name
/// is spelled. No other name is special — a `README.md` belongs to the project
/// and is an ordinary file here.
const DOCUMENT_FILE: &str = "WARLOCK.md";

/// Build the tree rooted at `working_dir`, coloured by the manifest above it.
///
/// The returned tree's [`root_path`](Tree::root_path) is `working_dir` itself,
/// made absolute; the manifest is loaded from `.warlock/pacts.toml` under the
/// nearest ancestor holding a `.git/` directory (see [`repository_root`]). A
/// repository that has no manifest yet, or an empty one, loads with every node
/// [`NodeState::Unpacted`]; a manifest that exists but cannot be understood is
/// an error rather than a silent empty one.
///
/// Alongside the tree comes a list of [`Problem`]s: everything that went wrong
/// without stopping the load. It is empty on a healthy repository, and a caller
/// that ignores it gets a tree where each affected node is stale — which is
/// safe, just unexplained.
///
/// ```
/// use std::fs;
/// use warlock_engine::{Loaded, NodeState, load_tree};
///
/// let repo = tempfile::tempdir()?;
/// fs::create_dir(repo.path().join(".git"))?;
/// fs::create_dir_all(repo.path().join("crates/engine/src"))?;
/// fs::write(repo.path().join("crates/engine/WARLOCK.md"), "# engine\n")?;
///
/// let Loaded { tree, problems } = load_tree(repo.path())?;
/// let paths: Vec<_> = tree.walk().map(|(node, _)| node.path.clone()).collect();
///
/// // Every directory the walk reached is a node, documented or not.
/// assert_eq!(paths, [
///     repo.path().to_path_buf(),
///     repo.path().join("crates"),
///     repo.path().join("crates/engine"),
///     repo.path().join("crates/engine/src"),
/// ]);
/// // Only `crates/engine` has a document; the others simply have none yet.
/// let src = tree.find(repo.path().join("crates/engine/src")).unwrap();
/// assert_eq!(src.document, None);
/// // Files ride along on the directory holding them, that document included.
/// let engine = tree.find(repo.path().join("crates/engine")).unwrap();
/// assert_eq!(engine.files, [repo.path().join("crates/engine/WARLOCK.md")]);
/// assert!(src.files.is_empty(), "an empty directory lists nothing");
/// // Nothing is pacted, so nothing was hashed and nothing could go wrong.
/// assert_eq!(tree.root.state, NodeState::Unpacted);
/// assert!(problems.is_empty());
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
///
/// # Errors
///
/// The fatal cases, and only these: a load that could not be trusted at all,
/// as against a node that could not be coloured (which is a [`Problem`]).
///
/// * [`Error::NoRepositoryRoot`] if neither `working_dir` nor any of its
///   ancestors contains a `.git/` directory.
/// * [`Error::Io`] if `working_dir` cannot be made absolute.
/// * [`Error::Manifest`] if a manifest is there but cannot be read or parsed.
/// * [`Error::Walk`] if the directory tree cannot be walked.
pub fn load_tree(working_dir: impl AsRef<Path>) -> Result<Loaded, Error> {
    let working_dir = absolute(working_dir.as_ref())?;
    let repo_root = repository_root(&working_dir).ok_or_else(|| Error::NoRepositoryRoot {
        start: working_dir.clone(),
    })?;

    // A repository that has never pacted anything is a repository where every
    // node is unpacted, which is exactly an empty manifest. A corrupt one is a
    // different fact and is not swallowed here.
    let manifest = match Manifest::load(&repo_root) {
        Err(ManifestError::NotFound { .. }) => Manifest::new(),
        other => other.map_err(|source| Error::Manifest { source })?,
    };

    let builder = Builder {
        directories: walk(&working_dir)?,
        repo_root,
        manifest,
    };
    let mut problems = Vec::new();
    let root = builder.node(&working_dir, &mut problems);
    Ok(Loaded {
        tree: Tree::new(root),
        problems,
    })
}

/// What a load produced: the coloured tree, and everything that went wrong on
/// the way without being bad enough to stop it.
///
/// A plain pair rather than a `Tree` with problems hung off it, because they
/// are answers to different questions and have different lifetimes: the tree is
/// the thing to render, the problems are the thing to report once. [`Node`]
/// gains no field for this — a node that could not be hashed is stale like any
/// other stale node, and a renderer needs to know nothing more.
#[derive(Debug)]
pub struct Loaded {
    /// The tree, every node coloured.
    pub tree: Tree,
    /// Everything non-fatal that went wrong, in the order the walk met it:
    /// children before their parents, siblings in name order. Empty is the
    /// normal case.
    pub problems: Vec<Problem>,
}

/// One thing that went wrong during a load without stopping it.
///
/// Two ways to get here, both of them a fact about one node that the load went
/// on around: a pacted node whose subtree could not be hashed, and a directory
/// whose manifest entry carries a string that is not a scope. Silence is the
/// thing being avoided in both cases — an unreadable file that simply dropped
/// out of a digest would hash exactly like a deleted one and could hand back a
/// green nobody earned, and a scope quietly ignored would leave somebody
/// believing they had written a boundary they had not.
#[derive(Debug)]
pub struct Problem {
    /// The node the problem happened at: the directory whose subtree hash was
    /// wanted, or the directory whose entry holds the bad scope. What exactly
    /// went wrong — the offending file, which is usually somewhere below it, or
    /// the rule the scope broke — is in `cause`.
    pub path: PathBuf,
    /// What went wrong there.
    pub cause: ProblemCause,
}

/// Why a node is in [`Loaded::problems`].
///
/// An enum rather than a bare [`HashError`], because the two things a load can
/// have to report about a node are not the same kind of failure: one is a
/// digest that could not be taken, the other a label that is not one. Each
/// keeps its own wording, and neither is dressed up as the other.
#[derive(Debug)]
#[non_exhaustive]
pub enum ProblemCause {
    /// The node's subtree could not be hashed, so it is coloured
    /// [`NodeState::PactedStale`]: content that cannot be read is content that
    /// cannot be vouched for.
    Hash(HashError),
    /// The node's manifest entry carries a string that is not a scope. The node
    /// keeps its state, its document, its files and its children, and reads as
    /// unscoped — [`Node::scope`] is `None` — for as long as the string is
    /// invalid. Nothing is corrected: the bytes stay in the manifest exactly as
    /// they were written, because rewriting somebody's committed line on the
    /// next save would put a change in a diff nobody authored.
    Scope {
        /// The string as the manifest holds it, unfolded and untrimmed.
        scope: String,
        /// The one rule it broke.
        rule: ScopeRule,
    },
}

impl fmt::Display for Problem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.cause {
            ProblemCause::Hash(cause) => write!(
                f,
                "`{}` could not be hashed and is stale: {}",
                self.path.display(),
                cause
            ),
            ProblemCause::Scope { scope, rule } => write!(
                f,
                "`{}` carries `{scope}`, which is not a scope, so it reads as \
                 unscoped: {rule}",
                self.path.display(),
            ),
        }
    }
}

impl std::error::Error for Problem {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match &self.cause {
            ProblemCause::Hash(cause) => Some(cause),
            ProblemCause::Scope { rule, .. } => Some(rule),
        }
    }
}

/// The nearest ancestor of `start` — `start` itself included — that contains a
/// `.git/` directory, or `None` if there is no such directory anywhere above
/// it.
///
/// This is the repository root in Warlock's sense: the one place a manifest
/// lives, at `<root>/.warlock/pacts.toml`. The anchor is `.git/` and not that
/// manifest, because a repository nobody has pacted yet is the normal way to
/// meet Warlock — it opens as a tree of unpacted modules, and the manifest
/// appears under the same root the first time something is pacted.
///
/// A filesystem check and nothing more: `.git` counts when it is a directory,
/// so a checkout where it is a *file* (a worktree, a submodule) is not a root
/// here.
///
/// `start` is used as given; [`load_tree`] makes its working directory absolute
/// before calling this, which is what a relative path needs for the walk
/// upwards to reach anything.
#[must_use]
pub fn repository_root(start: impl AsRef<Path>) -> Option<PathBuf> {
    start
        .as_ref()
        .ancestors()
        .find(|dir| dir.join(GIT_DIR).is_dir())
        .map(Path::to_path_buf)
}

/// Every directory at or below `root` that survives the ignore rules, each
/// mapped to what the walk found in it: whether it holds a `WARLOCK.md`, and
/// the files sitting directly inside it.
///
/// One pass over the filesystem answers both questions. The walker yields
/// directories and files interleaved, so a file is filed under its parent as it
/// arrives — which means no second walk, no [`read_dir`](std::fs::read_dir), and
/// files that obey the ignore rules because they came through them.
///
/// A [`BTreeMap`] rather than a `Vec` because [`Path`] orders by component:
/// iterating it yields parents before children and siblings in name order,
/// which is the whole of this loader's determinism. The files under each key are
/// sorted before the map is handed back, for the same reason: the order the
/// filesystem offers them in is nobody's guarantee.
///
/// `.warlockignore` removes nothing from what this yields; it only decides
/// which of the directories are marked excluded. See the module docs for why
/// the one walk in the crate that keeps that content is this one.
fn walk(root: &Path) -> Result<BTreeMap<PathBuf, Directory>, Error> {
    let mut directories: BTreeMap<PathBuf, Directory> = BTreeMap::new();
    for entry in builder(root).build() {
        let entry = entry.map_err(|source| Error::Walk { source })?;
        let file_type = entry.file_type();
        let path = entry.into_path();
        if file_type.is_some_and(|kind| kind.is_dir()) {
            // Asked for directly rather than inferred from the walk's file
            // entries: a document is what makes a module, so an ignore rule
            // that happens to cover it should not quietly unmake one.
            let has_document = path.join(DOCUMENT_FILE).is_file();
            // An entry rather than an insert: a file inside this directory may
            // have arrived first and already opened the record, and overwriting
            // it here would drop the listing.
            directories.entry(path).or_default().has_document = has_document;
        } else if file_type.is_some_and(|kind| kind.is_file()) {
            // Anything that is neither a directory nor a regular file — a
            // symlink above all, which this walk declines to follow — is
            // neither descended into nor listed.
            if let Some(parent) = path.parent().map(Path::to_path_buf) {
                directories.entry(parent).or_default().files.push(path);
            }
        }
    }
    for directory in directories.values_mut() {
        directory.files.sort();
    }
    mark_excluded(root, &mut directories)?;
    Ok(directories)
}

/// The walk this module makes, in the one place it is configured.
///
/// Shared so that the pass which finds the directories and the pass which
/// decides which of them the repository excluded differ in exactly one setting
/// — the custom ignore filename — and cannot drift into differing in another.
fn builder(root: &Path) -> WalkBuilder {
    let mut builder = WalkBuilder::new(root);
    builder
        // A symlinked directory is walked as a symlink, i.e. not descended
        // into, so a cycle of them terminates instead of recursing.
        .follow_links(false)
        // Fixtures and freshly-unpacked source trees have a `.gitignore` and
        // no `.git`; honouring the file either way is what keeps the skip list
        // out of this crate.
        .require_git(false)
        // `.warlock/` is Warlock's own bookkeeping, never a module of the
        // project. Pruned by name so it stays out even if it holds a document
        // and even if hidden directories are ever let back in.
        .filter_entry(|entry| entry.file_name() != OsStr::new(MANIFEST_DIR));
    builder
}

/// Mark every directory in `directories` the repository's `.warlockignore`
/// excludes, removing none of them.
///
/// The same walk again, with `.warlockignore` registered on top: whatever it
/// yields is what the rules keep, so a directory the first pass found and this
/// one did not is excluded — and so is everything below it, which falls out for
/// free because the walker never descends into it. One matcher used twice, not
/// a second matcher: this file still keeps no skip list.
///
/// `root` is asked about on its own first, because a walker applies no rule to
/// the root it is handed — a load rooted inside excluded content would
/// otherwise mark nothing at all. When it is excluded, so is everything under
/// it: gitignore semantics do not let a rule re-include content below an
/// excluded directory.
///
/// A `.warlockignore` that cannot be *used* is not silently read as "no rules"
/// where the answer is a verdict — [`subtree_hash`] and the pact walks fail
/// loudly, and a failed hash still reaches the caller as a [`Problem`] against
/// the node it happened at. Here the walker's own soft errors are left where
/// the pass above leaves them, since this mark is a hint for the front end and
/// not the thing that keeps excluded content out of a pact.
fn mark_excluded(root: &Path, directories: &mut BTreeMap<PathBuf, Directory>) -> Result<(), Error> {
    if ignores::is_ignored(root).map_err(|source| Error::Walk { source })? {
        for directory in directories.values_mut() {
            directory.ignored = true;
        }
        return Ok(());
    }

    let mut kept: BTreeSet<PathBuf> = BTreeSet::new();
    for entry in builder(root)
        .add_custom_ignore_filename(ignores::FILENAME)
        .build()
    {
        let entry = entry.map_err(|source| Error::Walk { source })?;
        if entry.file_type().is_some_and(|kind| kind.is_dir()) {
            kept.insert(entry.into_path());
        }
    }
    for (path, directory) in directories.iter_mut() {
        directory.ignored = !kept.contains(path);
    }
    Ok(())
}

/// What one pass of the walk learned about a single directory.
///
/// Widened from the bare `bool` it used to be so that files could ride along
/// with the directory that holds them: the map is keyed by directory either
/// way, which is what lets [`Builder::children_of`] treat every key as one.
#[derive(Debug, Default)]
struct Directory {
    /// Whether the directory directly contains a `WARLOCK.md`.
    has_document: bool,
    /// The files directly inside it, in path order, its `WARLOCK.md` among
    /// them. See [`Node::files`] for what a listing is and is not.
    files: Vec<PathBuf>,
    /// Whether the repository's `.warlockignore` excludes it. Filled in by
    /// [`mark_excluded`] after the walk, and carried onto [`Node::ignored`].
    ignored: bool,
}

/// The directories a walk found, plus everything needed to turn one into a
/// [`Node`].
#[derive(Debug)]
struct Builder {
    /// What the walk found in each directory, in path order.
    directories: BTreeMap<PathBuf, Directory>,
    /// The manifest's own directory, which its paths are relative to.
    repo_root: PathBuf,
    /// The manifest, or an empty one where the repository has none.
    manifest: Manifest,
}

impl Builder {
    /// The node for `dir`, with every directory below it hanging off it.
    ///
    /// Nothing is dropped: a directory the walk reached is a node whether or
    /// not it is documented, so the tree is the shape of the working directory
    /// and not an opinion about which parts of it are interesting. A view that
    /// wants only the documented ones filters what it renders. The files the
    /// walk met in `dir` are copied onto the node as a listing; they make no
    /// difference to its children, its state or its document.
    ///
    /// Anything that went wrong colouring a node, without being worth failing
    /// the load over, is pushed onto `problems`.
    fn node(&self, dir: &Path, problems: &mut Vec<Problem>) -> Node {
        let children: Vec<Node> = self
            .children_of(dir)
            .map(|child| self.node(child, problems))
            .collect();

        let found = self.directories.get(dir);
        let document = found
            .is_some_and(|directory| directory.has_document)
            .then(|| dir.join(DOCUMENT_FILE));
        let files = found
            .map(|directory| directory.files.clone())
            .unwrap_or_default();
        // A mark and nothing else: the node keeps its row, its files and its
        // colour, and only gains the reason it can never be pacted.
        let ignored = found.is_some_and(|directory| directory.ignored);

        let state = self.state_of(dir, problems);
        let scope = self.scope_of(dir, problems);

        Node::new(dir, document, state)
            .with_children(children)
            .with_files(files)
            .with_ignored(ignored)
            .with_scope(scope)
    }

    /// The directories directly inside `dir`, in name order.
    fn children_of<'a>(&'a self, dir: &'a Path) -> impl Iterator<Item = &'a Path> {
        // Every walked directory sorts after its own parent and before that
        // parent's next sibling, so the descendants of `dir` are one contiguous
        // run: the scan can stop at the first key that leaves it.
        self.directories
            .range(dir.to_path_buf()..)
            .map(|(path, _)| path.as_path())
            .skip_while(move |path| *path == dir)
            .take_while(move |path| path.starts_with(dir))
            .filter(move |path| path.parent() == Some(dir))
    }

    /// The colour of `dir`: what the manifest granted it, against what it
    /// hashes to now.
    ///
    /// The manifest is consulted first and the hash is only computed when there
    /// is an entry to compare it against. That ordering is the whole of the
    /// "hash only pacted subtrees" rule: an unpacted node is [`Unpacted`]
    /// whatever is under it, so reading those bytes would buy nothing.
    ///
    /// A path with no manifest form — one that is not valid UTF-8, say — can
    /// match no entry, so it is unpacted rather than an error: an oddly named
    /// directory somewhere in the tree should not fail the whole load, and it
    /// is not a problem to report either, because nobody pacted it.
    ///
    /// [`Unpacted`]: NodeState::Unpacted
    fn state_of(&self, dir: &Path, problems: &mut Vec<Problem>) -> NodeState {
        let Ok(key) = to_manifest_path(&self.repo_root, dir) else {
            return NodeState::Unpacted;
        };
        let Some(entry) = self.manifest.entry(&key) else {
            return NodeState::Unpacted;
        };

        // `dir` itself, not the manifest-relative key: a pact is granted
        // against the content of its own module, so the same module hashes the
        // same wherever the repository is checked out to.
        match subtree_hash(dir) {
            Ok(hash) => decide_state(Some(entry), &hash),
            Err(cause) => {
                // No hash at all is the point: nothing partial and no error
                // text goes anywhere near `decide_state`, so this cannot be
                // mistaken for a comparison that happened and failed to match.
                problems.push(Problem {
                    path: dir.to_path_buf(),
                    cause: ProblemCause::Hash(cause),
                });
                NodeState::PactedStale
            }
        }
    }

    /// The scope written on `dir`'s own manifest entry, or `None` where there
    /// is no entry, no scope on it, or no scope worth the name.
    ///
    /// `dir`'s own and never an ancestor's: a node says which boundary starts
    /// at it, and [`scope_covering`](crate::scope_covering) answers the other
    /// question by walking up. A directory nobody pacted therefore has no scope
    /// here whatever sits above it, which is the same invariant seen from the
    /// tree: no pact, no scope.
    ///
    /// A string the validator refuses is reported as a [`Problem`] and left out
    /// of the node. Not fatal, not corrected and not silently kept: the load
    /// finishes, the node keeps its state, document, files and children, the
    /// manifest keeps the bytes somebody wrote, and the directory reads as
    /// unscoped for as long as they are not a scope.
    fn scope_of(&self, dir: &Path, problems: &mut Vec<Problem>) -> Option<String> {
        // As in `state_of`: a path with no manifest form matches no entry, so
        // it carries no scope and is nobody's problem.
        let key = to_manifest_path(&self.repo_root, dir).ok()?;
        let stored = self.manifest.entry(&key)?.scope()?;

        match validate_scope(stored) {
            Ok(()) => Some(stored.to_owned()),
            Err(rule) => {
                problems.push(Problem {
                    path: dir.to_path_buf(),
                    cause: ProblemCause::Scope {
                        scope: stored.to_owned(),
                        rule,
                    },
                });
                None
            }
        }
    }
}

/// `path` as an absolute path, with `.` and `..` components resolved away.
///
/// Absolute because the walk upwards for a repository root has to be able to
/// leave the working directory, and normalised because `ancestors` would
/// otherwise hand back paths ending in `..`. This is lexical: unlike
/// [`fs::canonicalize`](std::fs::canonicalize) it touches no filesystem and
/// resolves no symlink, so the tree comes back rooted at the path the caller
/// actually named.
fn absolute(path: &Path) -> Result<PathBuf, Error> {
    let absolute = std::path::absolute(path).map_err(|source| Error::Io {
        path: path.to_path_buf(),
        source,
    })?;

    let mut normalised = PathBuf::new();
    for component in absolute.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalised.pop();
            }
            other => normalised.push(other.as_os_str()),
        }
    }
    Ok(normalised)
}

/// Everything that can stop a directory becoming a [`Tree`].
///
/// Hand-rolled for the same reason as [`ManifestError`]: four variants do not
/// pay for an error-handling dependency, and these are the sentences a front
/// end shows a user.
#[derive(Debug)]
#[non_exhaustive]
pub enum Error {
    /// Neither the working directory nor any ancestor holds a `.git/`
    /// directory, so there is no repository to load and nowhere a manifest
    /// could live.
    NoRepositoryRoot {
        /// The working directory the search started from.
        start: PathBuf,
    },
    /// The working directory could not be resolved to an absolute path.
    Io {
        /// The path that could not be resolved.
        path: PathBuf,
        /// What the filesystem said.
        source: std::io::Error,
    },
    /// A manifest is there but could not be read or understood. A *missing*
    /// manifest is not an error: it loads as an empty one.
    Manifest {
        /// What reading the manifest said.
        source: ManifestError,
    },
    /// The directory tree could not be walked.
    Walk {
        /// What the walker said, including which path it was on.
        source: ignore::Error,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoRepositoryRoot { start } => write!(
                f,
                "no `{GIT_DIR}` directory in `{}` or any of its parents",
                start.display()
            ),
            Self::Io { path, source } => {
                write!(f, "could not resolve `{}`: {source}", path.display())
            }
            Self::Manifest { source } => write!(f, "could not read the pact manifest: {source}"),
            Self::Walk { source } => write!(f, "could not walk the project tree: {source}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::Manifest { source } => Some(source),
            Self::Walk { source } => Some(source),
            Self::NoRepositoryRoot { .. } => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};

    use super::{Error, Loaded, ProblemCause, load_tree, repository_root};
    use crate::{
        HashError, Manifest, NodeState, PactEntry, StateCounts, Tree, manifest_path, subtree_hash,
        validate_scope,
    };

    /// A repository with a `.git/` directory — what makes it a repository — and
    /// a `.warlock/` one beside it, `dirs` created under them, and a
    /// `WARLOCK.md` written into each of `documents`.
    ///
    /// `.warlock/` is not needed to find the root any more, but it is where the
    /// manifest goes and it is pruned from the walk, so the fixture keeps
    /// making one: every test below sees the same tree shape either way.
    fn fixture(dirs: &[&str], documents: &[&str]) -> tempfile::TempDir {
        let repo = tempfile::tempdir().expect("a temporary directory");
        fs::create_dir_all(repo.path().join(".git")).expect("creates .git");
        fs::create_dir_all(repo.path().join(".warlock")).expect("creates .warlock");
        for dir in dirs {
            fs::create_dir_all(repo.path().join(dir)).expect("creates a directory");
        }
        for dir in documents {
            let path = repo.path().join(dir);
            fs::create_dir_all(&path).expect("creates a directory");
            fs::write(path.join("WARLOCK.md"), "# module\n").expect("writes a document");
        }
        repo
    }

    /// A repository nobody has pacted: a `.git/` directory, no `.warlock/`
    /// anywhere, and a `WARLOCK.md` in each of `documents`.
    ///
    /// Separate from [`fixture`] rather than a weakening of it, because the two
    /// answer different questions. Most tests want a repository with somewhere
    /// to put a manifest; the cold-open tests below want the state a repository
    /// is in the very first time Warlock is pointed at it, which is precisely
    /// the absence [`fixture`] fills in.
    fn git_only_fixture(documents: &[&str]) -> tempfile::TempDir {
        let repo = tempfile::tempdir().expect("a temporary directory");
        fs::create_dir_all(repo.path().join(".git")).expect("creates .git");
        for dir in documents {
            let path = repo.path().join(dir);
            fs::create_dir_all(&path).expect("creates a directory");
            fs::write(path.join("WARLOCK.md"), "# module\n").expect("writes a document");
        }
        assert!(
            !repo.path().join(".warlock").exists(),
            "the point of this fixture is that there is no `.warlock/` in it",
        );
        repo
    }

    /// The tree for `dir`, insisting the load found nothing to complain about.
    ///
    /// Most of these fixtures are healthy, so an empty problem list is part of
    /// what they assert: a load that quietly started reporting problems would
    /// fail here rather than pass unnoticed.
    fn tree_of(dir: impl AsRef<Path>) -> Tree {
        let Loaded { tree, problems } = load_tree(dir).expect("loads");
        assert!(problems.is_empty(), "{problems:?}");
        tree
    }

    /// Pact the modules at `modules` (paths relative to `root`), through the
    /// manifest API — which writes no grant, so every one of them is stale
    /// unless a test hand-writes one.
    fn pact(root: &Path, modules: &[&str]) {
        Manifest::with_entries(modules.iter().map(|module| {
            let module = root.join(module);
            PactEntry::new(root, &module, module.join("WARLOCK.md")).expect("inside the root")
        }))
        .save(root)
        .expect("saves");
    }

    /// Write `contents` at `path`, creating whatever directories it needs.
    fn write_file(path: &Path, contents: &str) {
        fs::create_dir_all(path.parent().expect("a file has a parent")).expect("creates parents");
        fs::write(path, contents).expect("writes a file");
    }

    /// A manifest written to `<root>/.warlock/pacts.toml` as text, the way a
    /// person with an editor would.
    ///
    /// The long way round on purpose: a `granted_hash` cannot be produced by
    /// this workspace at all — nothing in it grants freshness — so a test that
    /// needs a fresh node has to write one by hand, exactly as the only human
    /// who can grant one would.
    fn hand_write_manifest(root: &Path, pacts: &[(&str, Option<&str>)]) {
        use std::fmt::Write as _;

        let mut text = String::from("version = 1\n");
        for (module, granted) in pacts {
            write!(
                text,
                "\n[[pact]]\nmodule = \"{module}\"\ndocument = \"{module}/WARLOCK.md\"\n"
            )
            .expect("a string never fails to be written to");
            if let Some(hash) = granted {
                write!(
                    text,
                    "granted_hash = \"{hash}\"\ngranted_at = \"2026-08-19T07:32:00Z\"\n"
                )
                .expect("a string never fails to be written to");
            }
        }
        fs::write(manifest_path(root), text).expect("writes the manifest");
    }

    /// The file names the node at `dir` lists, in the order it lists them.
    ///
    /// Names rather than whole paths, because what is under test is which
    /// files a node claims and in what order; that they sit under the node is
    /// the loader's business and is asserted where it belongs.
    fn file_names(tree: &Tree, dir: impl AsRef<Path>) -> Vec<String> {
        tree.find(dir.as_ref())
            .unwrap_or_else(|| panic!("`{}` is a node", dir.as_ref().display()))
            .files
            .iter()
            .map(|file| {
                file.file_name()
                    .expect("a file has a name")
                    .to_string_lossy()
                    .into_owned()
            })
            .collect()
    }

    /// Every node path in the tree, relative to `root`, depth first.
    fn relative_paths(tree: &crate::Tree, root: &Path) -> Vec<String> {
        tree.walk()
            .map(|(node, _)| {
                node.path
                    .strip_prefix(root)
                    .expect("every node sits under the root")
                    .to_string_lossy()
                    .replace('\\', "/")
            })
            .collect()
    }

    #[test]
    fn a_document_makes_a_module_and_an_undocumented_directory_is_still_a_node() {
        let repo = fixture(&["crates/engine/src"], &["crates/engine"]);
        let tree = tree_of(repo.path());

        assert_eq!(
            relative_paths(&tree, repo.path()),
            ["", "crates", "crates/engine", "crates/engine/src"],
            "every walked directory is a node; a document only decides the \
             `document` field"
        );
        assert_eq!(tree.root.document, None, "the fixture root has no document");
        assert_eq!(
            tree.find(repo.path().join("crates"))
                .expect("an undocumented directory is a node")
                .document,
            None,
        );
        assert_eq!(
            tree.find(repo.path().join("crates/engine/src"))
                .expect("an undocumented leaf is a node")
                .document,
            None,
        );
        assert_eq!(
            tree.find(repo.path().join("crates/engine"))
                .expect("the module is a node")
                .document,
            Some(repo.path().join("crates/engine/WARLOCK.md")),
        );
    }

    #[test]
    fn a_plain_readme_documents_nothing_and_is_listed_like_any_other_file() {
        // Two directories that differ only in which file they hold: one has
        // Warlock's document, the other the project's README.
        let repo = fixture(&["docs"], &["crates/engine"]);
        write_file(&repo.path().join("docs/README.md"), "# for people\n");

        let tree = tree_of(repo.path());

        assert_eq!(
            tree.find(repo.path().join("crates/engine"))
                .expect("the module is a node")
                .document,
            Some(repo.path().join("crates/engine/WARLOCK.md")),
            "a `WARLOCK.md` documents the directory holding it",
        );
        assert_eq!(
            tree.find(repo.path().join("docs"))
                .expect("`docs/` is a node all the same")
                .document,
            None,
            "a `README.md` is the project's file, not Warlock's: it documents \
             nothing",
        );
        assert_eq!(
            file_names(&tree, repo.path().join("docs")),
            ["README.md"],
            "and it is listed under its directory like any other file",
        );
    }

    #[test]
    fn a_plain_readme_is_hashed_like_any_other_file_below_a_pacted_node() {
        let repo = fixture(&["docs"], &[]);
        let module = repo.path().join("docs");
        write_file(&module.join("README.md"), "# for people\n");

        let granted = subtree_hash(&module).expect("the module hashes");
        hand_write_manifest(repo.path(), &[("docs", Some(&granted))]);

        assert_eq!(
            tree_of(repo.path())
                .find(&module)
                .expect("the pacted module")
                .state,
            NodeState::PactedFresh,
            "the grant was taken over content that includes the README",
        );

        write_file(&module.join("README.md"), "# for people, revised\n");

        assert_eq!(
            tree_of(repo.path())
                .find(&module)
                .expect("the pacted module")
                .state,
            NodeState::PactedStale,
            "editing it moves the subtree hash exactly as editing any other \
             file would",
        );
    }

    #[test]
    fn a_repository_with_no_document_anywhere_still_loads_every_directory() {
        let repo = fixture(&["crates/engine/src"], &[]);
        let tree = tree_of(repo.path());

        assert_eq!(
            relative_paths(&tree, repo.path()),
            ["", "crates", "crates/engine", "crates/engine/src"],
            "nothing is documented, so nothing has a document — and every \
             directory is still a node"
        );
        assert_eq!(
            tree.find(repo.path().join("crates/engine/src"))
                .expect("the deepest directory is a node")
                .document,
            None,
        );
        assert!(tree.walk().all(|(node, _)| node.document.is_none()));
    }

    #[test]
    fn the_tree_is_rooted_at_the_working_directory_and_the_manifest_is_found_above_it() {
        let repo = fixture(&[], &["crates/engine", "crates/tui"]);
        let module = repo.path().join("crates/engine");
        pact(repo.path(), &["crates/engine"]);

        let tree = tree_of(repo.path().join("crates"));

        assert_eq!(tree.root_path(), repo.path().join("crates"));
        assert_eq!(
            relative_paths(&tree, repo.path()),
            ["crates", "crates/engine", "crates/tui"],
            "siblings come out in name order"
        );
        assert_eq!(
            tree.find(&module).expect("the pacted module").state,
            NodeState::PactedStale,
            "an entry in the manifest above the working directory still colours the node",
        );
        assert_eq!(
            tree.find(repo.path().join("crates/tui"))
                .expect("the unpacted module")
                .state,
            NodeState::Unpacted,
        );
        assert_eq!(tree, tree_of(repo.path().join("crates")));
    }

    #[test]
    fn a_repository_with_no_manifest_loads_entirely_unpacted() {
        let repo = fixture(&[], &["docs"]);
        let tree = tree_of(repo.path());

        assert_eq!(tree.counts().total(), 2);
        assert!(
            tree.walk()
                .all(|(node, _)| node.state == NodeState::Unpacted)
        );
    }

    #[test]
    fn a_repository_with_no_warlock_directory_at_all_opens_entirely_unpacted() {
        // The cold open: a repository Warlock has never been run on, opened
        // from a subdirectory so the root has to be found by walking up to the
        // `.git/` — there is no `.warlock/` anywhere to find instead.
        let repo = git_only_fixture(&["crates/engine", "crates/engine/src"]);

        let Loaded { tree, problems } = load_tree(repo.path().join("crates"))
            .expect("a `.git/` and nothing else is still a repository");

        assert_eq!(
            relative_paths(&tree, repo.path()),
            ["crates", "crates/engine", "crates/engine/src"],
        );
        assert!(
            tree.walk()
                .all(|(node, _)| node.state == NodeState::Unpacted),
            "with no manifest nothing is pacted, so the whole tree is gray: {:?}",
            tree.counts(),
        );
        assert!(
            problems.is_empty(),
            "a missing manifest is not a problem, it is the normal first state: {problems:?}",
        );
    }

    #[test]
    fn the_first_pact_in_a_never_pacted_repository_creates_the_warlock_directory() {
        let repo = git_only_fixture(&["docs"]);

        // No setup step in between: straight from a repository that has never
        // heard of Warlock to a saved pact.
        pact(repo.path(), &["docs"]);

        assert!(
            repo.path().join(".warlock").is_dir(),
            "saving makes the directory it needs",
        );
        assert!(
            manifest_path(repo.path()).is_file(),
            "and the manifest goes inside it, at `<root>/.warlock/pacts.toml`",
        );

        let manifest = Manifest::load(repo.path()).expect("loads what was just saved");
        let entry = manifest.entry("docs").expect("the entry just written");
        assert_eq!(entry.document(), "docs/WARLOCK.md");
        assert_eq!(
            tree_of(repo.path())
                .find(repo.path().join("docs"))
                .expect("the pacted module")
                .state,
            NodeState::PactedStale,
            "and the next load sees it: a pact with no grant on it is stale",
        );
    }

    #[test]
    fn the_walk_skips_ignored_and_warlock_directories() {
        let repo = fixture(
            &["target/debug"],
            &["target/debug", ".git/hooks", "vendored", "src"],
        );
        fs::write(repo.path().join(".gitignore"), "/target\n/vendored\n")
            .expect("writes a .gitignore");
        fs::write(repo.path().join(".warlock/WARLOCK.md"), "# not a module\n")
            .expect("writes a document inside .warlock");

        assert_eq!(
            relative_paths(&tree_of(repo.path()), repo.path()),
            ["", "src"],
            "`target/` and `vendored/` are gitignored, `.git/` is git's own, \
             and `.warlock/` is ours — a document in any of them changes \
             nothing"
        );
    }

    /// Every node in the tree as `(path relative to `root`, state)`, in walk
    /// order — the whole of what a colour-blind reload has to reproduce.
    fn states(tree: &Tree, root: &Path) -> Vec<(String, NodeState)> {
        relative_paths(tree, root)
            .into_iter()
            .zip(tree.walk().map(|(node, _)| node.state))
            .collect()
    }

    /// A repository whose `.warlockignore` excludes every directory named
    /// `notes/`, with one such directory inside the pacted module `docs/` and
    /// an ordinary `docs/src/` beside it.
    fn excluding_fixture() -> tempfile::TempDir {
        let repo = fixture(&[], &["docs"]);
        write_file(&repo.path().join(".warlockignore"), "notes/\n");
        write_file(&repo.path().join("docs/notes/one.md"), "a thought\n");
        write_file(&repo.path().join("docs/notes/gone.md"), "another\n");
        write_file(&repo.path().join("docs/src/lib.rs"), "pub fn one() {}\n");
        repo
    }

    #[test]
    fn an_excluded_directory_keeps_its_row_and_is_marked_rather_than_pruned() {
        let repo = excluding_fixture();
        let tree = tree_of(repo.path());

        assert_eq!(
            relative_paths(&tree, repo.path()),
            ["", "docs", "docs/notes", "docs/src"],
            "`.warlockignore` prunes nothing here: the reader is meant to see \
             what the repository excluded",
        );

        let notes = tree
            .find(repo.path().join("docs/notes"))
            .expect("the excluded directory is a node like any other");
        assert!(notes.is_ignored(), "and it says so");
        assert_eq!(
            notes.state,
            NodeState::Unpacted,
            "no fourth state: an excluded directory is gray like any other \
             directory nobody pacted",
        );
        assert_eq!(
            file_names(&tree, repo.path().join("docs/notes")),
            ["gone.md", "one.md"],
            "the row is a real one, files and all",
        );

        for covered in ["", "docs", "docs/src"] {
            assert!(
                !tree
                    .find(repo.path().join(covered))
                    .expect("a node")
                    .is_ignored(),
                "`{covered}` is covered content and no rule names it",
            );
        }
    }

    #[test]
    fn everything_below_an_excluded_directory_is_marked_too() {
        let repo = fixture(&[], &["docs"]);
        write_file(&repo.path().join(".warlockignore"), "notes/\n");
        write_file(&repo.path().join("docs/notes/deep/deeper/a.md"), "a\n");

        let tree = tree_of(repo.path());

        for below in ["docs/notes", "docs/notes/deep", "docs/notes/deep/deeper"] {
            assert!(
                tree.find(repo.path().join(below))
                    .unwrap_or_else(|| panic!("`{below}` is still a row"))
                    .is_ignored(),
                "`{below}` sits inside excluded content: gitignore does not let \
                 a rule re-include below an excluded directory",
            );
        }
    }

    #[test]
    fn a_load_rooted_at_an_excluded_directory_marks_it_all_the_same() {
        // The walker applies no rule to the root it is handed, so this is the
        // case a registered filename alone would miss.
        let repo = excluding_fixture();
        let notes = repo.path().join("docs/notes");
        write_file(&notes.join("deep/a.md"), "a\n");

        let tree = tree_of(&notes);

        assert_eq!(
            tree.root_path(),
            notes,
            "the tree is rooted where it was told"
        );
        assert!(
            tree.walk().all(|(node, _)| node.is_ignored()),
            "the root the load was given is excluded, and so is everything \
             under it",
        );
        assert!(
            tree.walk()
                .all(|(node, _)| node.state == NodeState::Unpacted),
        );
    }

    #[test]
    fn editing_adding_or_deleting_excluded_content_changes_no_colour() {
        let repo = excluding_fixture();
        let module = repo.path().join("docs");
        // A grant taken over the module's real content, so there is a green
        // here for a stray edit to lose.
        let granted = subtree_hash(&module).expect("the module hashes");
        hand_write_manifest(repo.path(), &[("docs", Some(&granted))]);

        let before = states(&tree_of(repo.path()), repo.path());
        assert!(
            before.contains(&("docs".to_owned(), NodeState::PactedFresh)),
            "the fixture has to start green for this test to mean anything: \
             {before:?}",
        );

        // Edit, add and delete, all inside content the repository excluded.
        write_file(&module.join("notes/one.md"), "a revised thought\n");
        write_file(&module.join("notes/two.md"), "an added one\n");
        fs::remove_file(module.join("notes/gone.md")).expect("deletes an excluded file");

        assert_eq!(
            states(&tree_of(repo.path()), repo.path()),
            before,
            "nothing Warlock covers changed, so nothing changed colour",
        );

        // And the control: the same three moves against covered content do.
        write_file(&module.join("src/lib.rs"), "pub fn two() {}\n");
        assert_ne!(
            states(&tree_of(repo.path()), repo.path()),
            before,
            "a covered edit still restales, or this test proves nothing",
        );
    }

    #[test]
    fn a_repository_with_no_rules_marks_nothing() {
        let repo = fixture(&["docs/notes"], &["docs"]);
        assert!(
            tree_of(repo.path())
                .walk()
                .all(|(node, _)| !node.is_ignored()),
            "with no `.warlockignore` anywhere, every directory is covered",
        );
    }

    #[test]
    fn a_node_lists_the_files_directly_inside_it_document_included() {
        let repo = fixture(&[], &["crates/engine"]);
        let module = repo.path().join("crates/engine");
        // Written in an order that is not the sorted one, so a listing that
        // simply kept what the filesystem offered would have to be lucky to
        // pass.
        write_file(&module.join("zeta.rs"), "");
        write_file(&module.join("alpha.rs"), "");
        write_file(&module.join("src/lib.rs"), "");

        let tree = tree_of(repo.path());

        assert_eq!(
            file_names(&tree, &module),
            ["WARLOCK.md", "alpha.rs", "zeta.rs"],
            "sorted, and the module's own document is one of its files"
        );
        assert_eq!(
            file_names(&tree, module.join("src")),
            ["lib.rs"],
            "a file belongs to the directory that holds it, not to an ancestor"
        );
        assert_eq!(
            file_names(&tree, repo.path()),
            [] as [String; 0],
            "the fixture root holds no files of its own"
        );
        assert!(
            !tree.find(&module).expect("the module").is_leaf(),
            "the module has a child directory, so it is no leaf",
        );
        assert!(
            tree.find(module.join("src"))
                .expect("the child directory")
                .is_leaf(),
            "and a directory whose only contents are files still is one",
        );
        assert_eq!(tree, tree_of(repo.path()), "two loads, one value");
    }

    #[test]
    fn the_files_a_node_lists_obey_the_same_rules_as_its_directories() {
        // The mirror of `the_walk_skips_ignored_and_warlock_directories`, one
        // level down: the same four kinds of thing a directory is skipped for,
        // asserted against files instead.
        let repo = fixture(&["target/debug"], &["src"]);
        fs::write(repo.path().join(".gitignore"), "/target\nsecret.txt\n")
            .expect("writes a .gitignore");
        write_file(&repo.path().join("secret.txt"), "shh\n");
        write_file(&repo.path().join(".hidden"), "shh\n");
        write_file(&repo.path().join("src/secret.txt"), "shh\n");
        write_file(&repo.path().join("target/debug/build.log"), "noise\n");
        write_file(&repo.path().join(".git/config"), "[core]\n");
        write_file(&repo.path().join(".warlock/notes.md"), "# ours\n");
        write_file(&repo.path().join("src/lib.rs"), "");

        let tree = tree_of(repo.path());
        let listed: Vec<String> = tree
            .walk()
            .flat_map(|(node, _)| {
                node.files.iter().map(|file| {
                    file.strip_prefix(repo.path())
                        .expect("every file sits under the root")
                        .to_string_lossy()
                        .replace('\\', "/")
                })
            })
            .collect();

        assert_eq!(
            listed,
            ["src/WARLOCK.md", "src/lib.rs"],
            "gitignored, hidden and `.warlock/` files come through the same \
             walk as directories, so they never arrive at all"
        );
    }

    #[test]
    fn nesting_goes_as_deep_as_the_directories_do() {
        let repo = fixture(
            &["crates/engine/src/inner"],
            &["crates/engine", "crates/engine/src/inner/deep"],
        );

        let tree = tree_of(repo.path());

        assert_eq!(
            relative_paths(&tree, repo.path()),
            [
                "",
                "crates",
                "crates/engine",
                "crates/engine/src",
                "crates/engine/src/inner",
                "crates/engine/src/inner/deep",
            ],
            "`src/` and `inner/` have no document of their own and are nodes \
             four and five levels down all the same"
        );
        assert_eq!(
            tree.walk()
                .map(|(_, depth)| depth)
                .max()
                .expect("a non-empty tree"),
            5,
        );
    }

    #[test]
    fn an_empty_manifest_loads_entirely_unpacted() {
        let repo = fixture(&[], &["docs", "src"]);
        Manifest::new()
            .save(repo.path())
            .expect("saves an empty manifest");

        let tree = tree_of(repo.path());

        assert_eq!(tree.counts().total(), 3);
        assert_eq!(tree.counts().unpacted, 3);
    }

    #[test]
    fn a_manifest_entry_colours_exactly_the_node_it_names() {
        let repo = fixture(&[], &["crates/engine", "crates/engine/src", "crates/tui"]);
        let module = repo.path().join("crates/engine");
        pact(repo.path(), &["crates/engine"]);

        let tree = tree_of(repo.path());

        assert_eq!(
            tree.counts(),
            StateCounts {
                unpacted: 4,
                pacted_stale: 1,
                pacted_fresh: 0,
            },
            "one entry with no grant on it: one stale node, and nothing else \
             even hashed"
        );
        assert_eq!(
            tree.find(&module).expect("the pacted module").state,
            NodeState::PactedStale,
        );
        assert_eq!(
            tree.find(module.join("src"))
                .expect("the module's own child")
                .state,
            NodeState::Unpacted,
            "an entry colours its own node, not the ones under it",
        );
    }

    #[test]
    fn a_hand_written_matching_grant_is_fresh_until_something_below_it_changes() {
        let repo = fixture(&[], &["crates/engine", "crates/tui"]);
        let module = repo.path().join("crates/engine");
        write_file(&module.join("src/lib.rs"), "pub fn one() {}\n");

        // The hash is taken over the module's own directory, which is what the
        // loader hashes too — not the repository root, and not the
        // manifest-relative path.
        let granted = subtree_hash(&module).expect("the module hashes");
        hand_write_manifest(repo.path(), &[("crates/engine", Some(&granted))]);

        let tree = tree_of(repo.path());
        assert_eq!(
            tree.find(&module).expect("the pacted module").state,
            NodeState::PactedFresh,
            "a grant that still matches the content it was granted against",
        );
        assert_eq!(
            tree.counts(),
            StateCounts {
                unpacted: 4,
                pacted_stale: 0,
                pacted_fresh: 1,
            },
        );

        // A file below the node, not the node's document: the trigger is
        // everything at and below the module.
        write_file(&module.join("src/lib.rs"), "pub fn two() {}\n");

        assert_eq!(
            tree_of(repo.path())
                .find(&module)
                .expect("the pacted module")
                .state,
            NodeState::PactedStale,
            "the same manifest, the same load, different content below it",
        );
    }

    /// Only on unix, because there is no portable way to make a file
    /// unreadable. What is under test — a hash that fails colours one node and
    /// is reported, rather than failing the load — is not platform-specific.
    #[cfg(unix)]
    #[test]
    fn an_unreadable_file_makes_one_node_stale_and_leaves_the_rest_coloured() {
        use std::os::unix::fs::PermissionsExt as _;

        let repo = fixture(&[], &["crates/engine", "crates/tui", "docs"]);
        let module = repo.path().join("crates/engine");
        let unreadable = module.join("src/lib.rs");
        write_file(&unreadable, "pub fn one() {}\n");

        let tui = repo.path().join("crates/tui");
        let granted = subtree_hash(&tui).expect("the other module hashes");
        hand_write_manifest(
            repo.path(),
            &[("crates/engine", None), ("crates/tui", Some(&granted))],
        );

        fs::set_permissions(&unreadable, fs::Permissions::from_mode(0o000)).expect("chmods");
        if fs::read(&unreadable).is_ok() {
            // Running as root: no file is unreadable, so there is nothing here
            // to assert against.
            return;
        }

        let Loaded { tree, problems } = load_tree(repo.path()).expect("a bad file is not fatal");

        assert_eq!(
            tree.find(&module)
                .expect("the module with the hole in it")
                .state,
            NodeState::PactedStale,
            "content that cannot be read is content that cannot be vouched for",
        );
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert_eq!(problems[0].path, module, "the problem names the node");
        assert!(
            matches!(
                problems[0].cause,
                ProblemCause::Hash(HashError::Read { .. })
            ),
            "{:?}",
            problems[0],
        );
        assert!(
            problems[0].to_string().contains("lib.rs"),
            "the cause names the file: {}",
            problems[0],
        );
        assert_eq!(
            tree.counts(),
            StateCounts {
                unpacted: 4,
                pacted_stale: 1,
                pacted_fresh: 1,
            },
            "one node lost its hash; every other node is coloured as it would \
             have been",
        );

        fs::set_permissions(&unreadable, fs::Permissions::from_mode(0o644)).expect("chmods back");
    }

    #[cfg(unix)]
    #[test]
    fn an_unpacted_node_is_never_hashed_so_a_file_it_cannot_read_is_no_problem() {
        use std::os::unix::fs::PermissionsExt as _;

        let repo = fixture(&[], &["crates/engine"]);
        let unreadable = repo.path().join("crates/engine/src/lib.rs");
        write_file(&unreadable, "pub fn one() {}\n");
        fs::set_permissions(&unreadable, fs::Permissions::from_mode(0o000)).expect("chmods");
        if fs::read(&unreadable).is_ok() {
            return; // Running as root, as above.
        }

        // Nothing is pacted, so nothing is hashed, so the file is never opened
        // and the load has nothing to say about it.
        let tree = tree_of(repo.path());

        assert_eq!(tree.counts().total(), 4);
        assert_eq!(tree.counts().unpacted, 4);

        fs::set_permissions(&unreadable, fs::Permissions::from_mode(0o644)).expect("chmods back");
    }

    /// Only on unix, because the fixture needs `std::os::unix::fs::symlink` to
    /// build the cycle at all. The behaviour under test — that the walk does
    /// not follow links — is not platform-specific.
    #[cfg(unix)]
    #[test]
    fn a_symlinked_directory_cycle_loads_and_terminates() {
        let repo = fixture(&[], &["crates/engine"]);

        // Two links, so neither a self-cycle nor a link back to an ancestor is
        // enough to send the walk round for ever.
        std::os::unix::fs::symlink(repo.path(), repo.path().join("crates/engine/up"))
            .expect("links back to the root");
        std::os::unix::fs::symlink(
            repo.path().join("crates"),
            repo.path().join("crates/engine/sideways"),
        )
        .expect("links back to an ancestor");

        let tree = tree_of(repo.path());
        assert_eq!(
            relative_paths(&tree, repo.path()),
            ["", "crates", "crates/engine"],
            "a symlinked directory is not descended into, so it is not a node"
        );
        assert_eq!(
            file_names(&tree, repo.path().join("crates/engine")),
            ["WARLOCK.md"],
            "and it is not listed as one of its parent's files either: a \
             symlink is neither walked nor listed"
        );
    }

    /// A cargo workspace shaped like this one — two crates under `crates/`, a
    /// gitignored `target/`, a `.git/` and Warlock's own `.warlock/` — built in
    /// a temporary directory rather than read off disk. Nothing in this crate's
    /// test suite asserts on the contents of the warlock repository itself: a
    /// test that did would change its verdict whenever the repository it lives
    /// in gained a directory.
    #[test]
    fn a_workspace_shaped_repository_loads_with_its_crates_and_nothing_ignored() {
        let repo = fixture(
            &["target/debug", ".git/hooks", "crates/warlock-tui/src"],
            &[
                "",
                "crates/warlock-engine",
                "crates/warlock-engine/src",
                "crates/warlock-tui",
            ],
        );
        fs::write(repo.path().join(".gitignore"), "/target\n").expect("writes a .gitignore");
        fs::write(
            repo.path().join("target/debug/WARLOCK.md"),
            "# not a module\n",
        )
        .expect("writes a document in build output");

        let tree = tree_of(repo.path());
        let paths = relative_paths(&tree, repo.path());

        assert_eq!(
            paths,
            [
                "",
                "crates",
                "crates/warlock-engine",
                "crates/warlock-engine/src",
                "crates/warlock-tui",
                "crates/warlock-tui/src",
            ],
        );
        assert_eq!(
            tree.find(repo.path().join("crates"))
                .expect("`crates/` is a node")
                .document,
            None,
            "`crates/` has no document of its own",
        );
        assert_eq!(
            tree.find(repo.path().join("crates/warlock-tui/src"))
                .expect("an undocumented `src/` is a node")
                .document,
            None,
            "and neither has `crates/warlock-tui/src`",
        );
        for ignored in ["target", ".git", ".warlock"] {
            assert!(
                !paths
                    .iter()
                    .any(|path| path.split('/').any(|part| part == ignored)),
                "`{ignored}` is hidden, gitignored or ours, so the walk should \
                 never have reached it: {paths:?}"
            );
        }
    }

    #[test]
    fn a_directory_outside_any_repository_is_an_error_that_says_so() {
        // Deliberately not a `tempfile::tempdir()`: `$TMPDIR` may itself sit
        // inside a checkout, and then the temporary directory has a `.git/`
        // ancestor and is not outside a repository at all. A name directly
        // under the filesystem root has exactly two ancestors — itself and `/`
        // — so nothing but a `/.git` could make it a repository. It need not
        // exist, either: `load_tree` makes the path absolute lexically and
        // resolves the root before it walks anything, so this reaches
        // `NoRepositoryRoot` without ever touching the disk.
        let outside = Path::new("/").join("warlock-no-repository-lives-here");
        assert!(
            repository_root(&outside).is_none(),
            "`{}` has no `.git` above it unless the filesystem root is a \
             repository",
            outside.display(),
        );

        let error = load_tree(&outside).expect_err("there is no `.git` anywhere above");
        assert!(matches!(error, Error::NoRepositoryRoot { .. }), "{error:?}");
        assert!(error.to_string().contains("`.git`"), "{error}");
    }

    #[test]
    fn a_corrupt_manifest_is_an_error_rather_than_an_empty_one() {
        let repo = fixture(&[], &["docs"]);
        fs::write(repo.path().join(".warlock/pacts.toml"), "not a manifest\n").expect("writes");

        assert!(matches!(
            load_tree(repo.path()),
            Err(Error::Manifest { .. })
        ));
    }

    /// A manifest written as text, the way somebody with an editor would, with
    /// a scope on each entry — including one nobody would call a scope.
    ///
    /// Hand-written rather than built through [`PactEntry::with_scope`] for the
    /// same reason [`hand_write_manifest`] is: the invalid cases below cannot
    /// be produced by any prompt in this workspace, and a person with an editor
    /// is exactly who produces them.
    fn hand_write_scoped_manifest(root: &Path, pacts: &[(&str, &str)]) {
        use std::fmt::Write as _;

        let mut text = String::from("version = 1\n");
        for (module, scope) in pacts {
            write!(
                text,
                "\n[[pact]]\nmodule = \"{module}\"\ndocument = \"{module}/WARLOCK.md\"\n\
                 scope = \"{scope}\"\n"
            )
            .expect("a string never fails to be written to");
        }
        fs::write(manifest_path(root), text).expect("writes the manifest");
    }

    #[test]
    fn a_scope_lands_on_the_directory_that_carries_it_and_on_no_other() {
        let repo = fixture(&["docs/src"], &["docs", "crates/engine"]);
        hand_write_scoped_manifest(repo.path(), &[("docs", "data-plane")]);

        let tree = tree_of(repo.path());

        let docs = tree
            .find(repo.path().join("docs"))
            .expect("the pacted module");
        assert_eq!(
            docs.scope.as_deref(),
            Some("data-plane"),
            "the scope on a directory's own entry is the scope on its node",
        );
        assert_eq!(
            docs.state,
            NodeState::PactedStale,
            "a scope colours nothing: this entry has no grant, so it is stale",
        );

        for elsewhere in ["", "docs/src", "crates", "crates/engine"] {
            assert_eq!(
                tree.find(repo.path().join(elsewhere))
                    .unwrap_or_else(|| panic!("`{elsewhere}` is a node"))
                    .scope,
                None,
                "`{elsewhere}` carries no scope of its own — a node holds the \
                 boundary that starts at it, never an inherited one",
            );
        }
    }

    #[test]
    fn a_directory_with_no_entry_carries_no_scope() {
        // The invariant seen from the tree: a scope lives on a pact entry, so a
        // directory nobody pacted has nowhere to have got one from.
        let repo = fixture(&["docs/src"], &["docs"]);

        assert!(
            tree_of(repo.path())
                .walk()
                .all(|(node, _)| node.scope.is_none()),
        );
    }

    /// Every string this repository's tests insist is not a scope, each one a
    /// different rule broken. Written as a manifest a person edited by hand,
    /// since nothing in this workspace would store them.
    const NOT_SCOPES: [&str; 7] = [
        "",
        "1data",
        "data-",
        "*",
        "aaaaaaaaaaaaaaaaaaaaaaaaa", // Twenty-five characters.
        "données",
        "Data-Plane",
    ];

    #[test]
    fn an_invalid_scope_is_reported_reads_as_unscoped_and_stops_nothing() {
        for not_a_scope in NOT_SCOPES {
            let repo = fixture(&["docs/src"], &["docs"]);
            let module = repo.path().join("docs");
            write_file(&module.join("adr.md"), "a decision\n");
            hand_write_scoped_manifest(repo.path(), &[("docs", not_a_scope)]);

            let Loaded { tree, problems } =
                load_tree(repo.path()).expect("a scope that is not one is not fatal");

            let docs = tree
                .find(&module)
                .expect("the pacted module is still a node");
            assert_eq!(
                docs.scope, None,
                "`{not_a_scope}` is not a scope, so the directory is unscoped",
            );
            assert_eq!(
                docs.state,
                NodeState::PactedStale,
                "`{not_a_scope}`: the node keeps the state its entry earned",
            );
            assert_eq!(
                docs.document,
                Some(module.join("WARLOCK.md")),
                "`{not_a_scope}`: and its document",
            );
            assert_eq!(
                file_names(&tree, &module),
                ["WARLOCK.md", "adr.md"],
                "`{not_a_scope}`: and its files",
            );
            assert_eq!(
                docs.children
                    .iter()
                    .map(|child| child.path.clone())
                    .collect::<Vec<_>>(),
                [module.join("src")],
                "`{not_a_scope}`: and its children",
            );

            assert_eq!(problems.len(), 1, "`{not_a_scope}`: {problems:?}");
            assert_eq!(
                problems[0].path, module,
                "`{not_a_scope}`: the problem names the directory",
            );
            let rule = validate_scope(not_a_scope).expect_err("not a scope");
            assert!(
                matches!(
                    &problems[0].cause,
                    ProblemCause::Scope { scope, rule: broken }
                        if scope == not_a_scope && *broken == rule
                ),
                "`{not_a_scope}`: {:?}",
                problems[0],
            );
            let reported = problems[0].to_string();
            assert!(
                reported.contains(&module.display().to_string()),
                "`{not_a_scope}`: the line names the directory: {reported}",
            );
            assert!(
                reported.contains(&rule.to_string()),
                "`{not_a_scope}`: the line names the rule broken: {reported}",
            );
        }
    }

    #[test]
    fn a_bad_scope_on_one_directory_leaves_every_other_scope_alone() {
        let repo = fixture(&[], &["docs", "crates/engine"]);
        hand_write_scoped_manifest(
            repo.path(),
            &[("docs", "Data-Plane"), ("crates/engine", "billing")],
        );

        let Loaded { tree, problems } = load_tree(repo.path()).expect("loads");

        assert_eq!(
            tree.find(repo.path().join("docs")).expect("a node").scope,
            None,
        );
        assert_eq!(
            tree.find(repo.path().join("crates/engine"))
                .expect("a node")
                .scope
                .as_deref(),
            Some("billing"),
            "one typo is one directory's problem, not the tree's",
        );
        assert_eq!(problems.len(), 1, "{problems:?}");
    }

    #[test]
    fn every_error_variant_says_what_happened() {
        let cases = [
            (
                Error::NoRepositoryRoot {
                    start: PathBuf::from("/elsewhere"),
                },
                "no `.git` directory in `/elsewhere` or any of its parents",
            ),
            (
                Error::Io {
                    path: PathBuf::from("/elsewhere"),
                    source: std::io::Error::other("boom"),
                },
                "could not resolve `/elsewhere`: boom",
            ),
        ];
        for (error, expected) in cases {
            assert_eq!(error.to_string(), expected);
        }
    }
}
