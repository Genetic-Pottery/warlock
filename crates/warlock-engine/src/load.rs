//! Building a [`Tree`] from a real directory.
//!
//! Section 5 of the design doc makes the tree of module READMEs the interface,
//! and section 12's modular invocation rule says the scope of a run is wherever
//! it was invoked — there is no privileged root. Both fall out of one function,
//! [`load_tree`]: the tree it returns is rooted at the working directory it was
//! given, while the single manifest that colours the nodes is read from the
//! repository root found by walking *up* from there.
//!
//! What makes a node:
//!
//! * A directory that directly contains a `README.md` is a module node, and
//!   carries it as [`Node::readme`]. Nothing here reads that file — Warlock
//!   cares only that one exists.
//! * A directory with no README of its own is a connector: it is kept, with
//!   `readme: None`, only when a module node sits somewhere below it. One with
//!   no module-node descendant is not in the tree at all.
//! * The directory the walk starts at is always a node, README or not.
//!
//! What the walk skips is not a list kept in this file: traversal is the
//! [`ignore`] crate, so `.gitignore` at every level, hidden directories
//! (`.git/` among them) and global excludes are all honoured as git honours
//! them. `.warlock/` is pruned unconditionally on top of that, symlinks are
//! never followed, and siblings come out ordered by directory name so two
//! loads of an unchanged tree are equal values.
//!
//! State is presence in the manifest and nothing more: an entry makes a node
//! [`NodeState::PactedStale`], no entry makes it [`NodeState::Unpacted`], and
//! nothing here can produce [`NodeState::PactedFresh`] — freshness needs a
//! subtree hash this crate does not compute yet.

use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::fmt;
use std::path::{Component, Path, PathBuf};

use ignore::WalkBuilder;

use crate::{Manifest, ManifestError, Node, NodeState, Tree, to_manifest_path};

/// The directory whose presence marks a repository root, and which is itself
/// never part of the tree.
const MANIFEST_DIR: &str = ".warlock";

/// The file whose presence in a directory makes that directory a module node.
const README_FILE: &str = "README.md";

/// Build the tree rooted at `working_dir`, coloured by the manifest above it.
///
/// The returned tree's [`root_path`](Tree::root_path) is `working_dir` itself,
/// made absolute; the manifest is loaded from the nearest ancestor holding a
/// `.warlock/` directory (see [`repository_root`]). A repository that has no
/// manifest yet, or an empty one, loads with every node
/// [`NodeState::Unpacted`]; a manifest that exists but cannot be understood is
/// an error rather than a silent empty one.
///
/// ```
/// use std::fs;
/// use warlock_engine::{NodeState, load_tree};
///
/// let repo = tempfile::tempdir()?;
/// fs::create_dir(repo.path().join(".warlock"))?;
/// fs::create_dir_all(repo.path().join("crates/engine"))?;
/// fs::write(repo.path().join("crates/engine/README.md"), "# engine\n")?;
///
/// let tree = load_tree(repo.path())?;
/// let paths: Vec<_> = tree.walk().map(|(node, _)| node.path.clone()).collect();
///
/// // `crates/` has no README of its own but is on the way to one that does.
/// assert_eq!(paths.len(), 3);
/// assert_eq!(tree.find(repo.path().join("crates")).unwrap().readme, None);
/// assert_eq!(tree.root.state, NodeState::Unpacted);
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
///
/// # Errors
///
/// * [`Error::NoRepositoryRoot`] if neither `working_dir` nor any of its
///   ancestors contains a `.warlock/` directory.
/// * [`Error::Io`] if `working_dir` cannot be made absolute.
/// * [`Error::Manifest`] if a manifest is there but cannot be read or parsed.
/// * [`Error::Walk`] if the directory tree cannot be walked.
pub fn load_tree(working_dir: impl AsRef<Path>) -> Result<Tree, Error> {
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
    Ok(Tree::new(builder.node(&working_dir)))
}

/// The nearest ancestor of `start` — `start` itself included — that contains a
/// `.warlock/` directory, or `None` if there is no such directory anywhere
/// above it.
///
/// This is the repository root in Warlock's sense: the one place a manifest
/// lives. It is deliberately not "wherever `.git/` is" — a repository can hold
/// several pacted checkouts, and it is the manifest that scopes a run.
///
/// `start` is used as given; [`load_tree`] makes its working directory absolute
/// before calling this, which is what a relative path needs for the walk
/// upwards to reach anything.
#[must_use]
pub fn repository_root(start: impl AsRef<Path>) -> Option<PathBuf> {
    start
        .as_ref()
        .ancestors()
        .find(|dir| dir.join(MANIFEST_DIR).is_dir())
        .map(Path::to_path_buf)
}

/// Every directory at or below `root` that survives the ignore rules, each
/// mapped to whether it directly contains a `README.md`.
///
/// A [`BTreeMap`] rather than a `Vec` because [`Path`] orders by component:
/// iterating it yields parents before children and siblings in name order,
/// which is the whole of this loader's determinism.
fn walk(root: &Path) -> Result<BTreeMap<PathBuf, bool>, Error> {
    let walker = WalkBuilder::new(root)
        // A symlinked directory is walked as a symlink, i.e. not descended
        // into, so a cycle of them terminates instead of recursing.
        .follow_links(false)
        // Fixtures and freshly-unpacked source trees have a `.gitignore` and
        // no `.git`; honouring the file either way is what keeps the skip list
        // out of this crate.
        .require_git(false)
        // `.warlock/` is Warlock's own bookkeeping, never a module of the
        // project. Pruned by name so it stays out even if it holds a README and
        // even if hidden directories are ever let back in.
        .filter_entry(|entry| entry.file_name() != OsStr::new(MANIFEST_DIR))
        .build();

    let mut directories = BTreeMap::new();
    for entry in walker {
        let entry = entry.map_err(|source| Error::Walk { source })?;
        if !entry.file_type().is_some_and(|kind| kind.is_dir()) {
            continue;
        }
        let path = entry.into_path();
        // Asked for directly rather than inferred from the walk's file
        // entries: a README is what makes a module, so an ignore rule that
        // happens to cover it should not quietly unmake one.
        let has_readme = path.join(README_FILE).is_file();
        directories.insert(path, has_readme);
    }
    Ok(directories)
}

/// The directories a walk found, plus everything needed to turn one into a
/// [`Node`].
#[derive(Debug)]
struct Builder {
    /// Directory to whether it holds a README, in path order.
    directories: BTreeMap<PathBuf, bool>,
    /// The manifest's own directory, which its paths are relative to.
    repo_root: PathBuf,
    /// The manifest, or an empty one where the repository has none.
    manifest: Manifest,
}

impl Builder {
    /// The node for `dir`, with its kept descendants hanging off it.
    ///
    /// Pruning is bottom-up and happens here rather than in a pass of its own:
    /// a child is kept when it has a README or, transitively, when something
    /// below it does. `dir` itself is never pruned — the caller only ever asks
    /// for the walk root, which is a node whether or not it is documented.
    fn node(&self, dir: &Path) -> Node {
        let children: Vec<Node> = self
            .children_of(dir)
            .map(|child| self.node(child))
            .filter(|node| node.readme.is_some() || !node.is_leaf())
            .collect();

        let readme = self
            .directories
            .get(dir)
            .copied()
            .unwrap_or_default()
            .then(|| dir.join(README_FILE));

        Node::new(dir, readme, self.state_of(dir)).with_children(children)
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

    /// What the manifest says about `dir`, by presence alone.
    ///
    /// A path with no manifest form — one that is not valid UTF-8, say — can
    /// match no entry, so it is unpacted rather than an error: an oddly named
    /// directory somewhere in the tree should not fail the whole load.
    fn state_of(&self, dir: &Path) -> NodeState {
        match to_manifest_path(&self.repo_root, dir) {
            Ok(key) if self.manifest.entry(&key).is_some() => NodeState::PactedStale,
            _ => NodeState::Unpacted,
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
    /// Neither the working directory nor any ancestor holds a `.warlock/`
    /// directory, so there is no repository to load and no manifest to read.
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
                "no `{MANIFEST_DIR}` directory in `{}` or any of its parents",
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

    use super::{Error, load_tree, repository_root};
    use crate::{Manifest, NodeState, PactEntry};

    /// A repository with a `.warlock/` directory, `dirs` created under it, and
    /// a `README.md` written into each of `readmes`.
    fn fixture(dirs: &[&str], readmes: &[&str]) -> tempfile::TempDir {
        let repo = tempfile::tempdir().expect("a temporary directory");
        fs::create_dir(repo.path().join(".warlock")).expect("creates .warlock");
        for dir in dirs {
            fs::create_dir_all(repo.path().join(dir)).expect("creates a directory");
        }
        for dir in readmes {
            let path = repo.path().join(dir);
            fs::create_dir_all(&path).expect("creates a directory");
            fs::write(path.join("README.md"), "# module\n").expect("writes a README");
        }
        repo
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
    fn a_readme_makes_a_module_and_a_bare_directory_on_the_way_is_a_connector() {
        let repo = fixture(&["crates/engine/src"], &["crates/engine"]);
        let tree = load_tree(repo.path()).expect("loads");

        assert_eq!(
            relative_paths(&tree, repo.path()),
            ["", "crates", "crates/engine"],
            "src/ has no README and nothing documented below it, so it is pruned"
        );
        assert_eq!(tree.root.readme, None, "the fixture root has no README");
        assert_eq!(
            tree.find(repo.path().join("crates"))
                .expect("the connector is a node")
                .readme,
            None,
        );
        assert_eq!(
            tree.find(repo.path().join("crates/engine"))
                .expect("the module is a node")
                .readme,
            Some(repo.path().join("crates/engine/README.md")),
        );
    }

    #[test]
    fn the_tree_is_rooted_at_the_working_directory_and_the_manifest_is_found_above_it() {
        let repo = fixture(&[], &["crates/engine", "crates/tui"]);
        let module = repo.path().join("crates/engine");
        Manifest::with_entries([
            PactEntry::new(repo.path(), &module, module.join("README.md"))
                .expect("inside the root"),
        ])
        .save(repo.path())
        .expect("saves");

        let tree = load_tree(repo.path().join("crates")).expect("loads from a subdirectory");

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
        assert_eq!(
            tree,
            load_tree(repo.path().join("crates")).expect("reloads")
        );
    }

    #[test]
    fn a_repository_with_no_manifest_loads_entirely_unpacted() {
        let repo = fixture(&[], &["docs"]);
        let tree = load_tree(repo.path()).expect("a missing manifest is an empty one");

        assert_eq!(tree.counts().total(), 2);
        assert!(
            tree.walk()
                .all(|(node, _)| node.state == NodeState::Unpacted)
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
        fs::write(repo.path().join(".warlock/README.md"), "# not a module\n")
            .expect("writes a README inside .warlock");

        assert_eq!(
            relative_paths(&load_tree(repo.path()).expect("loads"), repo.path()),
            ["", "src"],
            "`target/` and `vendored/` are gitignored, `.git/` is git's own, \
             and `.warlock/` is ours — a README in any of them changes nothing"
        );
    }

    #[test]
    fn nesting_goes_as_deep_as_the_directories_do() {
        let repo = fixture(
            &["crates/engine/src/inner"],
            &["crates/engine", "crates/engine/src/inner/deep"],
        );

        let tree = load_tree(repo.path()).expect("loads");

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
            "`src/` and `inner/` are connectors four and five levels down, kept \
             only because a module sits below them"
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

        let tree = load_tree(repo.path()).expect("loads");

        assert_eq!(tree.counts().total(), 3);
        assert_eq!(tree.counts().unpacted, 3);
    }

    #[test]
    fn a_manifest_entry_colours_exactly_the_node_it_names() {
        let repo = fixture(&[], &["crates/engine", "crates/engine/src", "crates/tui"]);
        let module = repo.path().join("crates/engine");
        Manifest::with_entries([
            PactEntry::new(repo.path(), &module, module.join("README.md"))
                .expect("inside the root"),
        ])
        .save(repo.path())
        .expect("saves");

        let tree = load_tree(repo.path()).expect("loads");

        assert_eq!(
            tree.counts(),
            crate::StateCounts {
                unpacted: 4,
                pacted_stale: 1,
                pacted_fresh: 0,
            },
            "one entry, one stale node, and nothing this loader can make fresh"
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

        assert_eq!(
            relative_paths(&load_tree(repo.path()).expect("loads"), repo.path()),
            ["", "crates", "crates/engine"],
            "a symlinked directory is not descended into, so it is not a node"
        );
    }

    #[test]
    fn this_repository_loads_with_its_crates_and_nothing_ignored() {
        // The repository root from the crate being compiled, not the process's
        // working directory: `cargo test` sets that per invocation and a test
        // that depends on it passes or fails by where it was run from.
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(2)
            .expect("crates/warlock-engine sits two levels below the root");

        let tree = load_tree(root).expect("this repository has a `.warlock` directory");
        let paths = relative_paths(&tree, root);

        for expected in ["", "crates", "crates/warlock-engine", "crates/warlock-tui"] {
            assert!(paths.iter().any(|path| path == expected), "{paths:?}");
        }
        assert_eq!(
            tree.find(root.join("crates"))
                .expect("`crates/` is a connector")
                .readme,
            None,
            "`crates/` has no README of its own",
        );
        for ignored in ["target", ".git", ".red", ".forman"] {
            assert!(
                !paths
                    .iter()
                    .any(|path| path.split('/').any(|part| part == ignored)),
                "`{ignored}` is hidden or gitignored, so the walk should never \
                 have reached it: {paths:?}"
            );
        }
    }

    #[test]
    fn a_directory_outside_any_repository_is_an_error_that_says_so() {
        let outside = tempfile::tempdir().expect("a temporary directory");
        assert!(repository_root(outside.path()).is_none());

        let error = load_tree(outside.path()).expect_err("there is no `.warlock` anywhere above");
        assert!(matches!(error, Error::NoRepositoryRoot { .. }), "{error:?}");
        assert!(error.to_string().contains("`.warlock`"), "{error}");
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

    #[test]
    fn every_error_variant_says_what_happened() {
        let cases = [
            (
                Error::NoRepositoryRoot {
                    start: PathBuf::from("/elsewhere"),
                },
                "no `.warlock` directory in `/elsewhere` or any of its parents",
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
