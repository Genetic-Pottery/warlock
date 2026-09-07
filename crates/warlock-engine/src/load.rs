use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsStr;
use std::fmt;
use std::path::{Component, Path, PathBuf};

use ignore::WalkBuilder;

use crate::{
    Manifest, Node, NodeState, Tree, decide_state, hash, ignores, manifest, scope, subtree_hash,
    to_manifest_path, validate_scope,
};

const GIT_DIR: &str = ".git";

const MANIFEST_DIR: &str = ".warlock";

const DOCUMENT_FILE: &str = "WARLOCK.md";

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
        problems,
    })
}

#[derive(Debug)]
pub struct Loaded {
    pub tree: Tree,
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

// One builder for both passes so that the pass which finds the directories and
// the pass which decides which of them are excluded differ in exactly one
// setting — the custom ignore filename — and cannot drift into differing in
// another.
fn builder(root: &Path) -> WalkBuilder {
    let mut builder = WalkBuilder::new(root);
    builder
        // Not merely the crate's default: a symlinked directory walked as a
        // symlink is never descended into, which is what makes a cycle of them
        // terminate.
        .follow_links(false)
        // Fixtures and freshly-unpacked source trees have a `.gitignore` and no
        // `.git`; honouring the file either way is what keeps a skip list out
        // of this crate.
        .require_git(false)
        // By name rather than by relying on the hidden-file rule, so `.warlock/`
        // stays out even if it holds a document and even if hidden directories
        // are ever let back in.
        .filter_entry(|entry| entry.file_name() != OsStr::new(MANIFEST_DIR));
    builder
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

        let found = self.directories.get(dir);
        let document = found
            .is_some_and(|directory| directory.has_document)
            .then(|| dir.join(DOCUMENT_FILE));
        let files = found
            .map(|directory| directory.files.clone())
            .unwrap_or_default();
        let ignored = found.is_some_and(|directory| directory.ignored);

        let state = self.state_of(dir, problems);
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
    fn state_of(&self, dir: &Path, problems: &mut Vec<Problem>) -> NodeState {
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
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};

    use super::{Error, Loaded, ProblemCause, load_tree, repository_root};
    use crate::{
        Manifest, NodeState, PactEntry, StateCounts, Tree, hash, manifest_path, subtree_hash,
        validate_scope,
    };

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

    fn tree_of(dir: impl AsRef<Path>) -> Tree {
        let Loaded { tree, problems } = load_tree(dir).expect("loads");
        assert!(problems.is_empty(), "{problems:?}");
        tree
    }

    fn pact(root: &Path, modules: &[&str]) {
        Manifest::with_entries(modules.iter().map(|module| {
            let module = root.join(module);
            PactEntry::new(root, &module, module.join("WARLOCK.md")).expect("inside the root")
        }))
        .save(root)
        .expect("saves");
    }

    fn write_file(path: &Path, contents: &str) {
        fs::create_dir_all(path.parent().expect("a file has a parent")).expect("creates parents");
        fs::write(path, contents).expect("writes a file");
    }

    // The long way round on purpose: a `granted_hash` cannot be produced by this
    // workspace at all — nothing in it grants freshness — so a test that needs a
    // fresh node writes one the way the only human who can grant one would.
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

    fn states(tree: &Tree, root: &Path) -> Vec<(String, NodeState)> {
        relative_paths(tree, root)
            .into_iter()
            .zip(tree.walk().map(|(node, _)| node.state))
            .collect()
    }

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
                ProblemCause::Hash(hash::Error::Read { .. })
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
