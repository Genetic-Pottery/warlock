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
//! The shape below is load-bearing — change it and the line-by-line
//! assertions in `ui.rs` and the counts in `app.rs` change with it.

use warlock_engine::{Node, NodeState, Tree};

/// A small hand-written tree covering everything the renderer has to handle.
///
/// Chosen to exercise, in one value: more than one level of nesting, a node in
/// each of the three [`NodeState`]s, and a connector — a directory kept in the
/// tree only because documented modules sit below it, and so carrying no
/// README of its own.
///
/// ```text
/// warlock                          README, pacted, stale
/// ├── warlock/crates               no README (connector), unpacted
/// │   ├── warlock/crates/engine    README, pacted, fresh
/// │   └── warlock/crates/tui       README, pacted, stale
/// └── warlock/assets               README, unpacted
/// ```
///
/// The paths are literals, related to this repository's layout only so that a
/// failing assertion reads like something recognisable; nothing here is read
/// off disk.
pub(crate) fn tree() -> Tree {
    Tree::new(
        Node::new("warlock", "warlock/README.md", NodeState::PactedStale).with_children([
            // A connector: no README of its own, so `None` and unpacted, which
            // is what the loader makes of such a directory.
            Node::new("warlock/crates", None, NodeState::Unpacted).with_children([
                Node::new(
                    "warlock/crates/engine",
                    "warlock/crates/engine/README.md",
                    NodeState::PactedFresh,
                ),
                Node::new(
                    "warlock/crates/tui",
                    "warlock/crates/tui/README.md",
                    NodeState::PactedStale,
                ),
            ]),
            Node::new(
                "warlock/assets",
                "warlock/assets/README.md",
                NodeState::Unpacted,
            ),
        ]),
    )
}

#[cfg(test)]
mod tests {
    use warlock_engine::NodeState;

    use super::tree;

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
    fn the_fixture_holds_a_connector_with_no_readme() {
        let connector = tree()
            .find("warlock/crates")
            .expect("the connector is in the fixture")
            .clone();

        assert_eq!(connector.readme, None);
        assert!(!connector.is_leaf(), "a connector exists for its children");
    }
}
