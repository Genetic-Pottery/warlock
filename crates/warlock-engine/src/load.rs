use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::{Component, Path, PathBuf};

use crate::walk::{self, DOCUMENT_FILE};
use crate::{
    Manifest, Node, NodeState, Tree, decide_state, hash, ignores, manifest, scope, subtree_hash,
    to_manifest_path, validate_scope,
};

const GIT_DIR: &str = ".git";

/// ```
/// use std::fs;
/// use warlock_engine::{Loaded, NodeState, load_tree};
///
/// let repo = tempfile::tempdir()?;
/// fs::create_dir(repo.path().join(".git"))?;
/// fs::create_dir_all(repo.path().join("crates/engine/src"))?;
/// fs::write(repo.path().join("crates/engine/WARLOCK.md"), "# engine\n")?;
///
/// let Loaded { tree, problems, .. } = load_tree(repo.path())?;
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
pub fn load_tree(working_dir: impl AsRef<Path>) -> Result<Loaded, Error> {
    let working_dir = absolute(working_dir.as_ref())?;
    let repo_root = repository_root(&working_dir).ok_or_else(|| Error::NoRepositoryRoot {
        start: working_dir.clone(),
    })?;

    // A repository that has never pacted anything is a repository where every
    // node is unpacted, which is exactly an empty manifest. A corrupt one is a
    // different fact and is not swallowed here.
    let manifest = match Manifest::load(&repo_root) {
        Err(manifest::Error::NotFound { .. }) => Manifest::new(),
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
        manifest: builder.manifest,
        problems,
    })
}

#[derive(Debug)]
pub struct Loaded {
    pub tree: Tree,
    // The manifest the tree was coloured from, from the same read. A front end
    // that kept a copy of its own from another read would judge boundaries and
    // save runs against entries the screen is not showing.
    pub manifest: Manifest,
    pub problems: Vec<Problem>,
}

#[derive(Debug)]
pub struct Problem {
    pub path: PathBuf,
    pub cause: ProblemCause,
}

#[derive(Debug)]
#[non_exhaustive]
pub enum ProblemCause {
    Hash(hash::Error),
    Scope { scope: String, rule: scope::Rule },
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

// The anchor is `.git/` rather than `.warlock/`, because a repository nobody has
// pacted yet is the normal way to meet warlock: it opens as a tree of unpacted
// modules and the manifest appears under this root the first time something is
// pacted.
#[must_use]
pub fn repository_root(start: impl AsRef<Path>) -> Option<PathBuf> {
    start
        .as_ref()
        .ancestors()
        // `is_dir`, so a checkout where `.git` is a *file* — a worktree, a
        // submodule — is deliberately not a root here.
        .find(|dir| dir.join(GIT_DIR).is_dir())
        .map(Path::to_path_buf)
}

fn walk(root: &Path) -> Result<BTreeMap<PathBuf, Directory>, Error> {
    let mut directories: BTreeMap<PathBuf, Directory> = BTreeMap::new();
    for entry in walk::listing(root).build() {
        let entry = entry.map_err(|source| Error::Walk { source })?;
        let Some(file_type) = entry.file_type() else {
            continue;
        };
        let path = entry.into_path();
        if file_type.is_dir() {
            // Asked for directly rather than inferred from the walk's file
            // entries: a document is what makes a module, so an ignore rule
            // that happens to cover it should not quietly unmake one.
            let has_document = path.join(DOCUMENT_FILE).is_file();
            // An entry rather than an insert: a file inside this directory may
            // have arrived first and already opened the record, and overwriting
            // it here would drop the listing.
            directories.entry(path).or_default().has_document = has_document;
        } else if file_type.is_file() {
            // Regular files only, so a symlink is neither descended into nor
            // listed. Widening this to "not a directory" would put links back in
            // the listing that `follow_links(false)` above keeps out of the
            // tree.
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

// `.warlockignore` marks directories here and prunes none of them, which is the
// one place in the crate where those rules do not remove content. Registering
// the file on the walk above — the obvious simplification — was rejected: it
// would take the excluded directory off the screen, and a reader who cannot see
// the folder of images cannot see that warlock is right not to cover it.
// Silence would look like a bug.
//
// The mark comes from running the same walk again with the file registered,
// rather than from a second matcher, so this file still keeps no skip list of
// its own: a directory the first pass found and this one did not is excluded.
fn mark_excluded(root: &Path, directories: &mut BTreeMap<PathBuf, Directory>) -> Result<(), Error> {
    // A walker applies no rule to the root it is handed, so a load rooted
    // inside excluded content would otherwise mark nothing at all. Everything
    // under it goes with it: gitignore semantics do not let a rule re-include
    // content below an excluded directory.
    if ignores::is_ignored(root).map_err(|source| Error::Walk { source })? {
        for directory in directories.values_mut() {
            directory.ignored = true;
        }
        return Ok(());
    }

    let mut kept: BTreeSet<PathBuf> = BTreeSet::new();
    for entry in walk::honouring_ignores(root).build() {
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

#[derive(Debug, Default)]
struct Directory {
    has_document: bool,
    files: Vec<PathBuf>,
    ignored: bool,
}

#[derive(Debug)]
struct Builder {
    directories: BTreeMap<PathBuf, Directory>,
    repo_root: PathBuf,
    manifest: Manifest,
}

impl Builder {
    fn node(&self, dir: &Path, problems: &mut Vec<Problem>) -> Node {
        let children: Vec<Node> = self
            .children_of(dir)
            .map(|child| self.node(child, problems))
            .collect();

        let (document, files, ignored) = match self.directories.get(dir) {
            Some(directory) => (
                directory.has_document.then(|| dir.join(DOCUMENT_FILE)),
                directory.files.clone(),
                directory.ignored,
            ),
            None => (None, Vec::new(), false),
        };

        let state = self.state_of(dir, ignored, problems);
        let scope = self.scope_of(dir, problems);

        Node::new(dir, document, state)
            .with_children(children)
            .with_files(files)
            .with_ignored(ignored)
            .with_scope(scope)
    }

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

    // The manifest is consulted before anything is hashed, and that ordering is
    // the whole of the "hash only pacted subtrees" rule: an unpacted node is
    // unpacted whatever is under it, so reading those bytes would buy nothing.
    fn state_of(&self, dir: &Path, ignored: bool, problems: &mut Vec<Problem>) -> NodeState {
        // Before the manifest is read, so that an entry left over from before
        // the rule was written cannot send an excluded directory to
        // `subtree_hash`. That hash skips excluded content, so it could never
        // match the grant again: the row would be yellow for good, and the only
        // repair would be hand-editing the file this tree exists to replace.
        if ignored {
            return NodeState::Unpacted;
        }

        // A path with no manifest form — not valid UTF-8, say — can match no
        // entry, so it is unpacted rather than an error: an oddly named
        // directory should not fail the load, and it is nobody's problem to
        // report because nobody pacted it.
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
                // Stale, and no digest of any kind reaches `decide_state`:
                // hashing the error text or the bytes that were read would give
                // a comparison that looks like it happened, and one that could
                // match. Content that cannot be read cannot be vouched for, and
                // one such file is one node's problem rather than the tree's.
                problems.push(Problem {
                    path: dir.to_path_buf(),
                    cause: ProblemCause::Hash(cause),
                });
                NodeState::PactedStale
            }
        }
    }

    // `dir`'s own scope and never an ancestor's: a node says which boundary
    // starts at it, and `scope_covering` answers the other question by walking
    // up. So a directory nobody pacted has no scope here whatever sits above it.
    fn scope_of(&self, dir: &Path, problems: &mut Vec<Problem>) -> Option<String> {
        let key = to_manifest_path(&self.repo_root, dir).ok()?;
        let stored = self.manifest.entry(&key)?.scope()?;

        match validate_scope(stored) {
            Ok(()) => Some(stored.to_owned()),
            Err(rule) => {
                // Reported, not corrected and not fatal. Rewriting the string
                // into something valid was rejected: those bytes are committed,
                // so the next save would put a line in a diff nobody authored.
                // Failing the load was too, since a scope gates nothing on its
                // own and one typo would take a whole tree gray.
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

// Lexical, and `fs::canonicalize` was rejected: it resolves symlinks, so the
// tree would come back rooted somewhere other than the path the caller named.
// Normalising is still needed because `ancestors` would otherwise hand the
// repository-root search paths ending in `..`.
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

#[derive(Debug)]
#[non_exhaustive]
pub enum Error {
    NoRepositoryRoot {
        start: PathBuf,
    },
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    Manifest {
        source: manifest::Error,
    },
    Walk {
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
#[path = "tests/load.rs"]
mod tests;
