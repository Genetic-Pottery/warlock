use std::collections::BTreeMap;
use std::fmt;
use std::path::{Component, Path, PathBuf};

use crate::walk;
use crate::{
    Manifest, Node, ScopedDirectory, Tree, manifest, scope, scope_covering, to_manifest_path,
    validate_scope,
};

const GIT_DIR: &str = ".git";

/// ```
/// use std::fs;
/// use warlock_engine::{Loaded, Manifest, ScopedDirectory, load_tree};
///
/// let repo = tempfile::tempdir()?;
/// fs::create_dir(repo.path().join(".git"))?;
/// fs::create_dir_all(repo.path().join("crates/engine/src"))?;
/// fs::write(repo.path().join("crates/engine/lib.rs"), "")?;
/// Manifest::with_directories([ScopedDirectory::new(repo.path(), "crates/engine", "data-plane")?])
///     .save(repo.path())?;
///
/// let Loaded { tree, problems, .. } = load_tree(repo.path())?;
/// let paths: Vec<_> = tree.walk().map(|(node, _)| node.path.clone()).collect();
///
/// // Every directory the walk reached is a node, scoped or not.
/// assert_eq!(paths, [
///     repo.path().to_path_buf(),
///     repo.path().join("crates"),
///     repo.path().join("crates/engine"),
///     repo.path().join("crates/engine/src"),
/// ]);
/// // `crates/engine` carries the scope, and `src` below it is covered by it.
/// let engine = tree.find(repo.path().join("crates/engine")).unwrap();
/// assert_eq!(engine.scope.as_deref(), Some("data-plane"));
/// let src = tree.find(repo.path().join("crates/engine/src")).unwrap();
/// assert_eq!(src.scope, None);
/// assert_eq!(src.covering.as_deref(), Some("data-plane"));
/// // Nothing above it is scoped, so the root is covered by nothing.
/// assert_eq!(tree.root.covering, None);
/// // Files ride along on the directory holding them.
/// assert_eq!(engine.files, [repo.path().join("crates/engine/lib.rs")]);
/// assert!(src.files.is_empty(), "an empty directory lists nothing");
/// assert!(problems.is_empty());
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub fn load_tree(working_dir: impl AsRef<Path>) -> Result<Loaded, Error> {
    let working_dir = absolute(working_dir.as_ref())?;
    let repo_root = repository_root(&working_dir).ok_or_else(|| Error::NoRepositoryRoot {
        start: working_dir.clone(),
    })?;

    // A repository that has never scoped anything is a repository where every
    // node is unscoped, which is exactly an empty manifest. A corrupt one is a
    // different fact and is not swallowed here.
    let manifest = match Manifest::load(&repo_root) {
        Err(manifest::Error::NotFound { .. }) => Manifest::new(),
        other => other.map_err(|source| Error::Manifest { source })?,
    };

    // What covers the working directory from above it: a tree opened below the
    // repository root still sits inside whatever scope its ancestors drew.
    let above = working_dir
        .parent()
        .filter(|parent| parent.starts_with(&repo_root))
        .and_then(|parent| scope_covering(parent, &repo_root, &manifest).ok().flatten())
        .map(ToOwned::to_owned);

    let mut builder = Builder {
        directories: walk(&working_dir)?,
        repo_root,
        rows: BTreeMap::new(),
    };
    for row in manifest.directories() {
        builder.rows.entry(row.path()).or_insert(row);
    }
    let mut problems = Vec::new();
    let root = builder.node(&working_dir, above, &mut problems);
    Ok(Loaded {
        tree: Tree::new(root),
        manifest,
        problems,
    })
}

#[derive(Debug)]
pub struct Loaded {
    pub tree: Tree,
    // The manifest the tree was coloured from, from the same read. A front end
    // that kept a copy of its own from another read would judge boundaries
    // against rows the screen is not showing.
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
    Scope { scope: String, rule: scope::Rule },
}

impl fmt::Display for Problem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.cause {
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
            ProblemCause::Scope { rule, .. } => Some(rule),
        }
    }
}

// The anchor is `.git/` rather than `.warlock/`, because a repository nobody has
// scoped yet is the normal way to meet warlock: it opens as a tree of unscoped
// directories and the manifest appears under this root the first time something
// is scoped.
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

fn walk(root: &Path) -> Result<BTreeMap<PathBuf, Vec<PathBuf>>, Error> {
    let mut directories: BTreeMap<PathBuf, Vec<PathBuf>> = BTreeMap::new();
    for entry in walk::listing(root).build() {
        let entry = entry.map_err(|source| Error::Walk { source })?;
        let Some(file_type) = entry.file_type() else {
            continue;
        };
        let path = entry.into_path();
        if file_type.is_dir() {
            // An entry rather than an insert: a file inside this directory may
            // have arrived first and already opened the record, and overwriting
            // it here would drop the listing.
            directories.entry(path).or_default();
        } else if file_type.is_file() {
            // Regular files only, so a symlink is neither descended into nor
            // listed. Widening this to "not a directory" would put links back in
            // the listing that `follow_links(false)` above keeps out of the
            // tree.
            if let Some(parent) = path.parent().map(Path::to_path_buf) {
                directories.entry(parent).or_default().push(path);
            }
        }
    }
    for files in directories.values_mut() {
        files.sort();
    }
    Ok(directories)
}

#[derive(Debug)]
struct Builder<'m> {
    directories: BTreeMap<PathBuf, Vec<PathBuf>>,
    repo_root: PathBuf,
    rows: BTreeMap<&'m str, &'m ScopedDirectory>,
}

impl Builder<'_> {
    fn node(&mut self, dir: &Path, above: Option<String>, problems: &mut Vec<Problem>) -> Node {
        // A path with no manifest form — not valid UTF-8, say — can match no
        // row, so it is unscoped rather than an error: an oddly named directory
        // should not fail the load.
        let row = to_manifest_path(&self.repo_root, dir)
            .ok()
            .and_then(|key| self.rows.get(key.as_str()).copied());
        let scope = scope_of(dir, row, problems);
        let covering = scope.clone().or(above);

        let below: Vec<PathBuf> = self.children_of(dir).map(Path::to_path_buf).collect();
        let children: Vec<Node> = below
            .iter()
            .map(|child| self.node(child, covering.clone(), problems))
            .collect();
        let files = self.directories.remove(dir).unwrap_or_default();

        Node::new(dir)
            .with_children(children)
            .with_files(files)
            .with_scope(scope)
            .with_covering(covering)
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
}

// `dir`'s own scope and never an ancestor's: a node says which boundary
// starts at it, and `covering` answers the other question.
fn scope_of(
    dir: &Path,
    row: Option<&ScopedDirectory>,
    problems: &mut Vec<Problem>,
) -> Option<String> {
    let stored = row?.scope();

    match validate_scope(stored) {
        Ok(()) => Some(stored.to_owned()),
        Err(rule) => {
            // Reported, not corrected and not fatal. Rewriting the string
            // into something valid was rejected: those bytes are committed,
            // so the next save would put a line in a diff nobody authored.
            // Failing the load was too, since one typo would take a whole
            // tree gray.
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
            Self::Manifest { source } => write!(f, "could not read the scope manifest: {source}"),
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
