use warlock_engine::NodeState;

use super::{tree, tree_after_a_run};

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
fn every_state_appears_in_the_fixture() {
    let counts = tree().counts();

    for state in NodeState::ALL {
        assert!(counts.get(state) > 0, "no node is {state:?}");
    }
    assert_eq!(counts.total(), 5);
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
            (
                "warlock".to_owned(),
                vec![
                    // A plain README beside the document, ordinary in every
                    // way: the loader lists it and nothing else notices it.
                    "warlock/README.md".to_owned(),
                    "warlock/WARLOCK.md".to_owned(),
                ],
            ),
            // A directory of nothing but directories lists nothing.
            ("warlock/crates".to_owned(), Vec::new()),
            (
                "warlock/crates/engine".to_owned(),
                vec![
                    "warlock/crates/engine/Cargo.toml".to_owned(),
                    "warlock/crates/engine/WARLOCK.md".to_owned(),
                ],
            ),
            (
                "warlock/crates/tui".to_owned(),
                vec!["warlock/crates/tui/WARLOCK.md".to_owned()],
            ),
            (
                "warlock/assets".to_owned(),
                vec![
                    "warlock/assets/WARLOCK.md".to_owned(),
                    "warlock/assets/logo.svg".to_owned(),
                ],
            ),
        ]
    );
}

#[test]
fn files_sit_under_both_a_pacted_and_an_unpacted_node() {
    let with_files: Vec<NodeState> = tree()
        .walk()
        .filter(|(node, _)| !node.files.is_empty())
        .map(|(node, _)| node.state)
        .collect();

    // A file takes its module's colour, so a fixture whose files all sat
    // under one state could not tell a view that got the colour right from
    // one that picked a single colour for every file.
    assert!(with_files.iter().any(|state| state.is_pacted()));
    assert!(with_files.iter().any(|state| !state.is_pacted()));
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
    assert_eq!(tree.counts().total(), 5);
    let engine = tree
        .find("warlock/crates/engine")
        .expect("the fixture has an engine node");
    assert!(!engine.files.is_empty());
    assert!(engine.is_leaf());
}

#[test]
fn the_second_fixture_is_the_first_one_with_a_document_the_run_wrote() {
    let before = tree();
    let after = tree_after_a_run();

    // The same walk, in the same order: one load later, not another tree.
    let paths = |tree: &super::Tree| -> Vec<String> {
        tree.walk()
            .map(|(node, _)| node.path.to_string_lossy().into_owned())
            .collect()
    };
    assert_eq!(paths(&before), paths(&after));

    // And exactly one node differs, in exactly the way a finished pact
    // makes one differ: a document where there was none, and the file it
    // was written in listed beside it.
    let differing: Vec<String> = after
        .walk()
        .filter(|(node, _)| {
            let was = before.find(&node.path).expect("the same walk");
            was.document != node.document || was.state != node.state || was.files != node.files
        })
        .map(|(node, _)| node.path.to_string_lossy().into_owned())
        .collect();
    assert_eq!(differing, ["warlock/crates".to_owned()]);

    let crates = after.find("warlock/crates").expect("the same walk");
    assert!(crates.document.is_some());
    assert_eq!(crates.state, NodeState::PactedFresh);
    assert_eq!(crates.files.len(), 1);
}

#[test]
fn the_fixture_holds_a_directory_with_no_document() {
    let undocumented = tree()
        .find("warlock/crates")
        .expect("the undocumented directory is in the fixture")
        .clone();

    assert_eq!(undocumented.document, None);
    assert!(
        !undocumented.is_leaf(),
        "documented modules sit below it as well"
    );
}
