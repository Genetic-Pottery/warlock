use std::fs;
use std::path::{Path, PathBuf};

use super::{Error, Loaded, ProblemCause, load_tree, repository_root};
use crate::{Manifest, ScopedDirectory, Tree, manifest_path, validate_scope};

fn fixture(dirs: &[&str]) -> tempfile::TempDir {
    let repo = tempfile::tempdir().expect("a temporary directory");
    fs::create_dir_all(repo.path().join(".git")).expect("creates .git");
    for dir in dirs {
        fs::create_dir_all(repo.path().join(dir)).expect("creates a directory");
    }
    repo
}

fn tree_of(dir: impl AsRef<Path>) -> Tree {
    let Loaded { tree, problems, .. } = load_tree(dir).expect("loads");
    assert!(problems.is_empty(), "{problems:?}");
    tree
}

fn scope(root: &Path, rows: &[(&str, &str)]) {
    Manifest::with_directories(rows.iter().map(|(dir, scope)| {
        ScopedDirectory::new(root, root.join(dir), *scope).expect("inside the root")
    }))
    .save(root)
    .expect("saves");
}

fn write_file(path: &Path, contents: &str) {
    fs::create_dir_all(path.parent().expect("a file has a parent")).expect("creates parents");
    fs::write(path, contents).expect("writes a file");
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

fn relative_paths(tree: &Tree, root: &Path) -> Vec<String> {
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

fn coverings(tree: &Tree, root: &Path) -> Vec<(String, Option<String>)> {
    relative_paths(tree, root)
        .into_iter()
        .zip(tree.walk().map(|(node, _)| node.covering.clone()))
        .collect()
}

fn covering_of(tree: &Tree, dir: impl AsRef<Path>) -> Option<String> {
    tree.find(dir.as_ref())
        .unwrap_or_else(|| panic!("`{}` is a node", dir.as_ref().display()))
        .covering
        .clone()
}

#[test]
fn every_walked_directory_is_a_node() {
    let repo = fixture(&["crates/engine/src"]);
    let tree = tree_of(repo.path());

    assert_eq!(
        relative_paths(&tree, repo.path()),
        ["", "crates", "crates/engine", "crates/engine/src"],
    );
}

#[test]
fn a_plain_readme_is_listed_like_any_other_file() {
    let repo = fixture(&["docs"]);
    write_file(&repo.path().join("docs/README.md"), "# for people\n");

    assert_eq!(
        file_names(&tree_of(repo.path()), repo.path().join("docs")),
        ["README.md"],
    );
}

#[test]
fn the_tree_is_rooted_at_the_working_directory_and_the_manifest_is_found_above_it() {
    let repo = fixture(&["crates/engine", "crates/tui"]);
    let engine = repo.path().join("crates/engine");
    scope(repo.path(), &[("crates/engine", "data-plane")]);

    let tree = tree_of(repo.path().join("crates"));

    assert_eq!(tree.root_path(), repo.path().join("crates"));
    assert_eq!(
        relative_paths(&tree, repo.path()),
        ["crates", "crates/engine", "crates/tui"],
        "siblings come out in name order"
    );
    assert_eq!(
        tree.find(&engine)
            .expect("the scoped directory")
            .scope
            .as_deref(),
        Some("data-plane"),
        "a row in the manifest above the working directory still reaches the node",
    );
    assert_eq!(
        tree.find(repo.path().join("crates/tui"))
            .expect("the unscoped directory")
            .scope,
        None,
    );
    assert_eq!(tree, tree_of(repo.path().join("crates")));
}

#[test]
fn a_repository_with_no_warlock_directory_at_all_opens_entirely_unscoped() {
    // The cold open: a repository warlock has never been run on, opened from
    // a subdirectory so the root has to be found by walking up to the
    // `.git/` — there is no `.warlock/` anywhere to find instead.
    let repo = fixture(&["crates/engine/src"]);
    assert!(!repo.path().join(".warlock").exists());

    let Loaded { tree, problems, .. } = load_tree(repo.path().join("crates"))
        .expect("a `.git/` and nothing else is still a repository");

    assert_eq!(
        relative_paths(&tree, repo.path()),
        ["crates", "crates/engine", "crates/engine/src"],
    );
    assert!(
        tree.walk()
            .all(|(node, _)| node.scope.is_none() && node.covering.is_none()),
    );
    assert!(
        problems.is_empty(),
        "a missing manifest is not a problem, it is the normal first state: {problems:?}",
    );
}

#[test]
fn the_first_scope_in_a_never_scoped_repository_creates_the_warlock_directory() {
    let repo = fixture(&["docs"]);

    scope(repo.path(), &[("docs", "docs")]);

    assert!(repo.path().join(".warlock").is_dir());
    assert!(manifest_path(repo.path()).is_file());
    assert_eq!(
        tree_of(repo.path())
            .find(repo.path().join("docs"))
            .expect("the scoped directory")
            .scope
            .as_deref(),
        Some("docs"),
        "and the next load sees it",
    );
}

#[test]
fn the_walk_skips_ignored_and_hidden_directories() {
    let repo = fixture(&[
        "target/debug",
        ".git/hooks",
        ".warlock",
        ".hidden",
        "vendored",
        "src",
    ]);
    fs::write(repo.path().join(".gitignore"), "/target\n/vendored\n").expect("writes a .gitignore");

    assert_eq!(
        relative_paths(&tree_of(repo.path()), repo.path()),
        ["", "src"],
    );
}

#[test]
fn a_node_lists_the_files_directly_inside_it_sorted() {
    let repo = fixture(&[]);
    let engine = repo.path().join("crates/engine");
    // Written in an order that is not the sorted one, so a listing that
    // simply kept what the filesystem offered would have to be lucky to
    // pass.
    write_file(&engine.join("zeta.rs"), "");
    write_file(&engine.join("alpha.rs"), "");
    write_file(&engine.join("src/lib.rs"), "");

    let tree = tree_of(repo.path());

    assert_eq!(file_names(&tree, &engine), ["alpha.rs", "zeta.rs"]);
    assert_eq!(
        file_names(&tree, engine.join("src")),
        ["lib.rs"],
        "a file belongs to the directory that holds it, not to an ancestor"
    );
    assert_eq!(file_names(&tree, repo.path()), [] as [String; 0]);
    assert!(!tree.find(&engine).expect("a node").is_leaf());
    assert!(
        tree.find(engine.join("src")).expect("a node").is_leaf(),
        "a directory whose only contents are files is a leaf",
    );
    assert_eq!(tree, tree_of(repo.path()), "two loads, one value");
}

#[test]
fn the_files_a_node_lists_obey_the_same_rules_as_its_directories() {
    let repo = fixture(&["target/debug"]);
    fs::write(repo.path().join(".gitignore"), "/target\nsecret.txt\n")
        .expect("writes a .gitignore");
    write_file(&repo.path().join("secret.txt"), "shh\n");
    write_file(&repo.path().join(".hidden"), "shh\n");
    write_file(&repo.path().join("src/secret.txt"), "shh\n");
    write_file(&repo.path().join("src/.warlock.md"), "# hidden\n");
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

    assert_eq!(listed, ["src/lib.rs"]);
}

#[test]
fn nesting_goes_as_deep_as_the_directories_do() {
    let repo = fixture(&["crates/engine/src/inner/deep"]);

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
fn an_empty_manifest_loads_entirely_unscoped() {
    let repo = fixture(&["docs", "src"]);
    Manifest::new()
        .save(repo.path())
        .expect("saves an empty manifest");

    let tree = tree_of(repo.path());

    assert_eq!(tree.walk().count(), 3);
    assert!(
        tree.walk()
            .all(|(node, _)| node.scope.is_none() && node.covering.is_none()),
    );
}

#[cfg(unix)]
#[test]
fn a_symlinked_directory_cycle_loads_and_terminates() {
    let repo = fixture(&["crates/engine"]);
    write_file(&repo.path().join("crates/engine/lib.rs"), "");

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
        ["lib.rs"],
        "and it is not listed as one of its parent's files either",
    );
}

#[test]
fn a_workspace_shaped_repository_loads_with_its_crates_and_nothing_ignored() {
    let repo = fixture(&[
        "target/debug",
        ".git/hooks",
        ".warlock",
        "crates/warlock-engine/src",
        "crates/warlock-tui/src",
    ]);
    fs::write(repo.path().join(".gitignore"), "/target\n").expect("writes a .gitignore");

    assert_eq!(
        relative_paths(&tree_of(repo.path()), repo.path()),
        [
            "",
            "crates",
            "crates/warlock-engine",
            "crates/warlock-engine/src",
            "crates/warlock-tui",
            "crates/warlock-tui/src",
        ],
    );
}

#[test]
fn a_tree_opened_from_inside_a_hidden_directory_still_has_its_root() {
    let repo = fixture(&[".config/tools"]);

    let tree = tree_of(repo.path().join(".config"));

    assert_eq!(
        relative_paths(&tree, repo.path()),
        [".config", ".config/tools"],
    );
}

#[test]
fn a_working_directory_with_dot_dot_in_it_is_normalised() {
    let repo = fixture(&["crates/engine", "crates/tui"]);

    let tree = tree_of(repo.path().join("crates/engine/../tui/."));

    assert_eq!(tree.root_path(), repo.path().join("crates/tui"));
}

#[test]
fn the_repository_root_is_the_nearest_directory_holding_a_git_directory() {
    let repo = fixture(&["crates/engine/src"]);

    assert_eq!(
        repository_root(repo.path().join("crates/engine/src")),
        Some(repo.path().to_path_buf()),
    );
    assert_eq!(
        repository_root(repo.path()),
        Some(repo.path().to_path_buf())
    );
}

#[test]
fn a_git_file_does_not_make_a_repository_root() {
    let repo = fixture(&["worktree/src"]);
    write_file(&repo.path().join("worktree/.git"), "gitdir: elsewhere\n");

    assert_eq!(
        repository_root(repo.path().join("worktree/src")),
        Some(repo.path().to_path_buf()),
        "a worktree's `.git` file is skipped for the directory above it",
    );
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
    let repo = fixture(&["docs"]);
    write_file(&manifest_path(repo.path()), "not a manifest\n");

    assert!(matches!(
        load_tree(repo.path()),
        Err(Error::Manifest { .. })
    ));
}

#[test]
fn a_manifest_of_an_unsupported_version_is_an_error() {
    let repo = fixture(&["docs"]);
    write_file(&manifest_path(repo.path()), "version = 3\n");

    assert!(matches!(
        load_tree(repo.path()),
        Err(Error::Manifest {
            source: crate::manifest::Error::UnsupportedVersion { .. }
        })
    ));
}

#[test]
fn a_scope_lands_on_the_directory_that_carries_it_and_on_no_other() {
    let repo = fixture(&["docs/src", "crates/engine"]);
    scope(repo.path(), &[("docs", "data-plane")]);

    let tree = tree_of(repo.path());

    assert_eq!(
        tree.find(repo.path().join("docs"))
            .expect("the scoped directory")
            .scope
            .as_deref(),
        Some("data-plane"),
    );
    for elsewhere in ["", "docs/src", "crates", "crates/engine"] {
        assert_eq!(
            tree.find(repo.path().join(elsewhere))
                .unwrap_or_else(|| panic!("`{elsewhere}` is a node"))
                .scope,
            None,
            "`{elsewhere}` carries no scope of its own — `scope` holds the \
                 boundary that starts at a node, never an inherited one",
        );
    }
}

#[test]
fn a_scope_covers_every_directory_below_the_one_that_carries_it() {
    let repo = fixture(&["docs/src/deep", "crates"]);
    scope(repo.path(), &[("docs", "data-plane")]);

    let tree = tree_of(repo.path());

    assert_eq!(
        coverings(&tree, repo.path()),
        [
            (String::new(), None),
            ("crates".to_owned(), None),
            ("docs".to_owned(), Some("data-plane".to_owned())),
            ("docs/src".to_owned(), Some("data-plane".to_owned())),
            ("docs/src/deep".to_owned(), Some("data-plane".to_owned())),
        ],
    );
}

#[test]
fn a_nearer_scope_overrides_the_one_above_it() {
    let repo = fixture(&["crates/engine/src", "crates/tui"]);
    scope(
        repo.path(),
        &[(".", "platform"), ("crates/engine", "data-plane")],
    );

    let tree = tree_of(repo.path());

    assert_eq!(
        coverings(&tree, repo.path()),
        [
            (String::new(), Some("platform".to_owned())),
            ("crates".to_owned(), Some("platform".to_owned())),
            ("crates/engine".to_owned(), Some("data-plane".to_owned())),
            (
                "crates/engine/src".to_owned(),
                Some("data-plane".to_owned())
            ),
            ("crates/tui".to_owned(), Some("platform".to_owned())),
        ],
    );
}

#[test]
fn a_tree_opened_below_a_scoped_directory_inherits_its_scope() {
    let repo = fixture(&["crates/engine/src/inner"]);
    scope(repo.path(), &[("crates", "platform")]);

    let tree = tree_of(repo.path().join("crates/engine/src"));

    assert_eq!(tree.root.scope, None, "the root carries no row of its own");
    assert_eq!(
        coverings(&tree, repo.path()),
        [
            ("crates/engine/src".to_owned(), Some("platform".to_owned())),
            (
                "crates/engine/src/inner".to_owned(),
                Some("platform".to_owned())
            ),
        ],
    );
}

#[test]
fn a_tree_opened_at_a_scoped_directory_is_covered_by_its_own_scope() {
    let repo = fixture(&["crates/engine/src"]);
    scope(
        repo.path(),
        &[("crates", "platform"), ("crates/engine", "data-plane")],
    );

    let tree = tree_of(repo.path().join("crates/engine"));

    assert_eq!(tree.root.scope.as_deref(), Some("data-plane"));
    assert_eq!(tree.root.covering.as_deref(), Some("data-plane"));
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
        let repo = fixture(&["docs/src"]);
        let docs = repo.path().join("docs");
        write_file(&docs.join("adr.md"), "a decision\n");
        scope(repo.path(), &[("docs", not_a_scope)]);

        let Loaded { tree, problems, .. } =
            load_tree(repo.path()).expect("a scope that is not one is not fatal");

        let node = tree.find(&docs).expect("the directory is still a node");
        assert_eq!(node.scope, None, "`{not_a_scope}` is not a scope");
        assert_eq!(
            node.covering, None,
            "`{not_a_scope}`: nothing above covers it"
        );
        assert_eq!(
            file_names(&tree, &docs),
            ["adr.md"],
            "`{not_a_scope}`: the node keeps its files",
        );
        assert_eq!(
            node.children
                .iter()
                .map(|child| child.path.clone())
                .collect::<Vec<_>>(),
            [docs.join("src")],
            "`{not_a_scope}`: and its children",
        );

        assert_eq!(problems.len(), 1, "`{not_a_scope}`: {problems:?}");
        assert_eq!(
            problems[0].path, docs,
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
            reported.contains(&docs.display().to_string()),
            "`{not_a_scope}`: the line names the directory: {reported}",
        );
        assert!(
            reported.contains(&rule.to_string()),
            "`{not_a_scope}`: the line names the rule broken: {reported}",
        );
    }
}

#[test]
fn an_invalid_scope_falls_back_to_the_covering_of_its_parent() {
    let repo = fixture(&["crates/engine/src"]);
    scope(
        repo.path(),
        &[("crates", "platform"), ("crates/engine", "Data-Plane")],
    );

    let Loaded { tree, problems, .. } = load_tree(repo.path()).expect("loads");

    let engine = repo.path().join("crates/engine");
    assert_eq!(tree.find(&engine).expect("a node").scope, None);
    assert_eq!(covering_of(&tree, &engine).as_deref(), Some("platform"));
    assert_eq!(
        covering_of(&tree, engine.join("src")).as_deref(),
        Some("platform"),
        "and so does everything below it",
    );
    assert_eq!(problems.len(), 1, "{problems:?}");
}

#[test]
fn an_invalid_scope_above_the_working_directory_falls_back_to_the_one_above_it() {
    let repo = fixture(&["crates/engine/src"]);
    scope(
        repo.path(),
        &[("crates", "platform"), ("crates/engine", "Data-Plane")],
    );

    let Loaded { tree, problems, .. } =
        load_tree(repo.path().join("crates/engine/src")).expect("loads");

    assert_eq!(tree.root.covering.as_deref(), Some("platform"));
    assert!(
        problems.is_empty(),
        "the bad row is not in this tree, so this load has nothing to report: {problems:?}",
    );
}

#[test]
fn a_bad_scope_on_one_directory_leaves_every_other_scope_alone() {
    let repo = fixture(&["docs", "crates/engine"]);
    scope(
        repo.path(),
        &[("docs", "Data-Plane"), ("crates/engine", "billing")],
    );

    let Loaded { tree, problems, .. } = load_tree(repo.path()).expect("loads");

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
