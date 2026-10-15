use super::{tree, tree_after_a_rescope};

#[test]
fn the_fixture_walks_in_a_fixed_depth_first_order() {
    let visited: Vec<(String, usize)> = tree()
        .walk()
        .map(|(node, depth)| (node.path.to_string_lossy().into_owned(), depth))
        .collect();

    assert_eq!(
        visited,
        [
            ("warlock".to_owned(), 0),
            ("warlock/crates".to_owned(), 1),
            ("warlock/crates/engine".to_owned(), 2),
            ("warlock/crates/tui".to_owned(), 2),
            ("warlock/assets".to_owned(), 1),
        ]
    );
}

#[test]
fn the_fixture_nests_more_than_one_level() {
    let deepest = tree()
        .walk()
        .map(|(_, depth)| depth)
        .max()
        .expect("the walk always yields the root");

    assert!(deepest > 1, "deepest node sat at depth {deepest}");
}

#[test]
fn the_fixture_holds_a_scoped_a_covered_and_an_unscoped_directory() {
    let tree = tree();
    let node = |path: &str| tree.find(path).expect("the fixture has it").clone();

    let crates = node("warlock/crates");
    assert_eq!(crates.scope.as_deref(), Some("platform"));
    let tui = node("warlock/crates/tui");
    assert_eq!(tui.scope, None);
    assert_eq!(tui.covering.as_deref(), Some("platform"));
    let engine = node("warlock/crates/engine");
    assert_eq!(engine.covering.as_deref(), Some("data-plane"));
    for unscoped in ["warlock", "warlock/assets"] {
        assert!(!node(unscoped).is_covered(), "{unscoped} is covered");
    }
    assert_eq!(tree.walk().count(), 5);
}

#[test]
fn the_fixture_lists_files_under_more_than_one_directory() {
    let listing: Vec<(String, Vec<String>)> = tree()
        .walk()
        .map(|(node, _)| {
            (
                node.path.to_string_lossy().into_owned(),
                node.files
                    .iter()
                    .map(|file| file.to_string_lossy().into_owned())
                    .collect(),
            )
        })
        .collect();

    assert_eq!(
        listing,
        [
            ("warlock".to_owned(), vec!["warlock/README.md".to_owned()],),
            // A directory of nothing but directories lists nothing.
            ("warlock/crates".to_owned(), Vec::new()),
            (
                "warlock/crates/engine".to_owned(),
                vec!["warlock/crates/engine/Cargo.toml".to_owned()],
            ),
            (
                "warlock/crates/tui".to_owned(),
                vec!["warlock/crates/tui/Cargo.toml".to_owned()],
            ),
            (
                "warlock/assets".to_owned(),
                vec!["warlock/assets/logo.svg".to_owned()],
            ),
        ]
    );
}

#[test]
fn files_sit_under_both_a_covered_and_an_uncovered_node() {
    let with_files: Vec<bool> = tree()
        .walk()
        .filter(|(node, _)| !node.files.is_empty())
        .map(|(node, _)| node.is_covered())
        .collect();

    // A file takes its directory's colour, so a fixture whose files all sat
    // under covered directories, or all under uncovered ones, could not tell a
    // view that got the colour right from one that picked a single colour.
    assert!(with_files.iter().any(|covered| *covered));
    assert!(with_files.iter().any(|covered| !covered));
    assert!(with_files.len() > 1);
}

#[test]
fn every_listed_file_sits_directly_in_the_directory_listing_it() {
    for (node, _) in tree().walk() {
        for file in &node.files {
            assert_eq!(
                file.parent(),
                Some(node.path.as_path()),
                "{} is not directly inside {}",
                file.display(),
                node.path.display()
            );
        }
        // The loader hands them over in path order, and a view drawing them
        // in the order it is given should be drawing them in that order.
        let mut sorted = node.files.clone();
        sorted.sort();
        assert_eq!(
            node.files,
            sorted,
            "{} lists out of order",
            node.path.display()
        );
        // Files are a listing, not children: no subdirectory is in here.
        for child in &node.children {
            assert!(!node.files.contains(&child.path));
        }
    }
}

#[test]
fn listing_files_leaves_the_nodes_and_their_leaves_alone() {
    let tree = tree();

    // Files are no part of the tree's shape: the same five nodes, and a
    // node with files and no subdirectories is still a leaf.
    assert_eq!(tree.walk().count(), 5);
    let engine = tree
        .find("warlock/crates/engine")
        .expect("the fixture has an engine node");
    assert!(!engine.files.is_empty());
    assert!(engine.is_leaf());
}

#[test]
fn the_second_fixture_is_the_first_one_with_one_directory_rescoped() {
    let before = tree();
    let after = tree_after_a_rescope();

    // The same walk, in the same order: one load later, not another tree.
    let paths = |tree: &super::Tree| -> Vec<String> {
        tree.walk()
            .map(|(node, _)| node.path.to_string_lossy().into_owned())
            .collect()
    };
    assert_eq!(paths(&before), paths(&after));

    let differing: Vec<String> = after
        .walk()
        .filter(|(node, _)| {
            let was = before.find(&node.path).expect("the same walk");
            was.scope != node.scope || was.covering != node.covering || was.files != node.files
        })
        .map(|(node, _)| node.path.to_string_lossy().into_owned())
        .collect();
    assert_eq!(differing, ["warlock/assets".to_owned()]);

    let assets = after.find("warlock/assets").expect("the same walk");
    assert_eq!(assets.scope.as_deref(), Some("assets"));
}
