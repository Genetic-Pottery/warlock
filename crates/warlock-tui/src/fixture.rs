//! The tree the front end's own tests are written against.
//!
//! The app and draw tests need a tree with a known shape: they assert the
//! exact lines that end up on screen, the order rows come out in, and the
//! tally under them. Loading one from disk would make those assertions depend
//! on whatever repository the test happened to run in, so the tree is written
//! out here by hand instead — no directory is read, no file is opened and no
//! staleness is computed, which is why `cargo test` needs neither a terminal
//! nor a filesystem.
//!
//! This is a fixture and nothing else. It is not a loader, it is not a
//! fallback for a failed load, and nothing outside `#[cfg(test)]` can reach
//! it: building a real tree is [`warlock_engine::load_tree`]'s job, and the
//! front end learns about trees only through [`Tree`].
//!
//! There are two of them, and the second exists for one reason: a view that is
//! re-seated on a freshly loaded tree can only be shown carrying anything if
//! there are two trees to carry it between. [`tree_after_a_run`] is
//! [`tree`] one load later, differing where a finished pact makes a directory
//! differ and nowhere else, so a test that re-seats between them is asserting
//! about the carrying rather than about two unrelated shapes.
//!
//! The shapes below are load-bearing — change them and the line-by-line
//! assertions in `ui.rs` and the counts in `app.rs` change with them.

use std::path::{Path, PathBuf};

use warlock_engine::{Node, NodeState, Tree};

/// A small hand-written tree covering everything the renderer has to handle.
///
/// Chosen to exercise, in one value: more than one level of nesting, a node in
/// each of the three [`NodeState`]s, an ordinary directory that has no
/// documentation yet — a node like any other, carrying no document of its own —
/// and files listed on more than one directory, under a pacted node and under
/// an unpacted one, so that a view drawing them in their module's colour has
/// more than one colour to get right.
///
/// ```text
/// warlock                          document, pacted, stale
/// │   README.md, WARLOCK.md
/// ├── warlock/crates               no document yet, unpacted, no files of its own
/// │   ├── warlock/crates/engine    document, pacted, fresh
/// │   │       Cargo.toml, WARLOCK.md
/// │   └── warlock/crates/tui       document, pacted, stale
/// │           WARLOCK.md
/// └── warlock/assets               document, unpacted
///             WARLOCK.md, logo.svg
/// ```
///
/// The listings follow the loader's own rules, so that a view tested against
/// this fixture is tested against something a real load could produce: they are
/// in path order, they hold no subdirectory — those are children — and a
/// directory's own `WARLOCK.md` is among them, since the loader lists what the
/// walk saw rather than what the walk saw minus one special name. `crates/`
/// lists nothing, because a directory holding only directories is a real case
/// too. The root's `README.md` is in there for the same reason and is nothing
/// more than an ordinary file: Warlock's document is `WARLOCK.md`, and a README
/// beside it documents nothing as far as the tree is concerned.
///
/// The paths are literals, related to this repository's layout only so that a
/// failing assertion reads like something recognisable; nothing here is read
/// off disk.
pub(crate) fn tree() -> Tree {
    Tree::new(
        Node::new("warlock", "warlock/WARLOCK.md", NodeState::PactedStale)
            .with_files(files("warlock", ["README.md", "WARLOCK.md"]))
            .with_children([
                // No document of its own yet, so `None` and unpacted, which is
                // what the loader makes of such a directory.
                Node::new("warlock/crates", None, NodeState::Unpacted).with_children([
                    Node::new(
                        "warlock/crates/engine",
                        "warlock/crates/engine/WARLOCK.md",
                        NodeState::PactedFresh,
                    )
                    .with_files(files("warlock/crates/engine", ["Cargo.toml", "WARLOCK.md"])),
                    Node::new(
                        "warlock/crates/tui",
                        "warlock/crates/tui/WARLOCK.md",
                        NodeState::PactedStale,
                    )
                    .with_files(files("warlock/crates/tui", ["WARLOCK.md"])),
                ]),
                Node::new(
                    "warlock/assets",
                    "warlock/assets/WARLOCK.md",
                    NodeState::Unpacted,
                )
                .with_files(files("warlock/assets", ["WARLOCK.md", "logo.svg"])),
            ]),
    )
}

/// The same tree as [`tree`], as a second load would find it after a pact run
/// over `warlock/crates` — the tree the front end is re-seated on.
///
/// One directory differs, and it differs the way a finished run makes a
/// directory differ: `warlock/crates` had no `WARLOCK.md` and now has one, so it
/// carries a document, it is pacted and fresh, and the file the run wrote is in
/// its listing. Everything else — the shape, the order, the other four nodes and
/// their files — is [`tree`]'s, because a re-seat has to be shown carrying a
/// view across a tree that has changed *somewhere*, not across a tree that has
/// been replaced.
///
/// ```text
/// warlock                          document, pacted, stale
/// │   README.md, WARLOCK.md
/// ├── warlock/crates               document now, pacted, fresh
/// │   │   WARLOCK.md               <- what the run wrote
/// │   ├── warlock/crates/engine    document, pacted, fresh
/// │   │       Cargo.toml, WARLOCK.md
/// │   └── warlock/crates/tui       document, pacted, stale
/// │           WARLOCK.md
/// └── warlock/assets               document, unpacted
///             WARLOCK.md, logo.svg
/// ```
///
/// Written out rather than derived from [`tree`] by mutation: a fixture whose
/// shape has to be worked out by reading a patch is a fixture the assertions
/// against it cannot be read beside.
pub(crate) fn tree_after_a_run() -> Tree {
    Tree::new(
        Node::new("warlock", "warlock/WARLOCK.md", NodeState::PactedStale)
            .with_files(files("warlock", ["README.md", "WARLOCK.md"]))
            .with_children([
                // The run wrote this document and granted its hash, so the
                // directory that was undocumented and unpacted is neither now.
                Node::new(
                    "warlock/crates",
                    "warlock/crates/WARLOCK.md",
                    NodeState::PactedFresh,
                )
                .with_files(files("warlock/crates", ["WARLOCK.md"]))
                .with_children([
                    Node::new(
                        "warlock/crates/engine",
                        "warlock/crates/engine/WARLOCK.md",
                        NodeState::PactedFresh,
                    )
                    .with_files(files("warlock/crates/engine", ["Cargo.toml", "WARLOCK.md"])),
                    Node::new(
                        "warlock/crates/tui",
                        "warlock/crates/tui/WARLOCK.md",
                        NodeState::PactedStale,
                    )
                    .with_files(files("warlock/crates/tui", ["WARLOCK.md"])),
                ]),
                Node::new(
                    "warlock/assets",
                    "warlock/assets/WARLOCK.md",
                    NodeState::Unpacted,
                )
                .with_files(files("warlock/assets", ["WARLOCK.md", "logo.svg"])),
            ]),
    )
}

/// The `names` as paths inside `directory`, which is how the loader spells a
/// directory's file listing: whole paths, not bare names.
fn files<'a>(
    directory: &'a str,
    names: impl IntoIterator<Item = &'a str> + 'a,
) -> impl Iterator<Item = PathBuf> + 'a {
    names
        .into_iter()
        .map(|name| Path::new(directory).join(name))
}

#[cfg(test)]
mod tests {
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
}
