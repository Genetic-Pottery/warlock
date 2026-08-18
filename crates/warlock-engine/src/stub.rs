//! A hard-coded tree, standing in for a loader that does not exist yet.
//!
//! Section 12 of the design doc builds the engine before the TUI, which leaves
//! the TUI with nothing to draw until directory walking lands. This module is
//! that nothing-to-draw problem's stopgap and nothing more: one tree, written
//! out by hand, so the engine/TUI seam can be proven before the filesystem is
//! involved.

use std::path::PathBuf;

use crate::{Node, NodeState, Tree};

/// **Placeholder.** A small hard-coded tree, to be replaced by a real
/// filesystem loader.
///
/// This is not the loader and must not be mistaken for one. It reads no
/// directory, opens no file and computes no staleness: every path and every
/// state below is a literal typed into this file, chosen only to give a
/// renderer something with more than one level of nesting and at least one
/// node in each of the three states. When the loader arrives, this function
/// and its module go.
///
/// The shape it returns:
///
/// ```text
/// warlock                          pacted, stale
/// ├── warlock/crates               pacted, stale
/// │   ├── warlock/crates/engine    pacted, fresh
/// │   └── warlock/crates/tui       pacted, stale
/// └── warlock/assets               unpacted
/// ```
///
/// ```
/// use warlock_engine::{stub_tree, NodeState};
///
/// let tree = stub_tree();
/// assert_eq!(tree.counts().total(), 5);
/// assert_eq!(
///     tree.find("warlock/crates/engine").map(|node| node.state),
///     Some(NodeState::PactedFresh),
/// );
/// ```
#[must_use]
pub fn stub_tree() -> Tree {
    Tree::new(
        Node::new(
            "warlock",
            Some(PathBuf::from("warlock/README.md")),
            NodeState::PactedStale,
        )
        .with_children([
            Node::new(
                "warlock/crates",
                Some(PathBuf::from("warlock/crates/README.md")),
                NodeState::PactedStale,
            )
            .with_children([
                Node::new(
                    "warlock/crates/engine",
                    Some(PathBuf::from("warlock/crates/engine/README.md")),
                    NodeState::PactedFresh,
                ),
                Node::new(
                    "warlock/crates/tui",
                    Some(PathBuf::from("warlock/crates/tui/README.md")),
                    NodeState::PactedStale,
                ),
            ]),
            Node::new(
                "warlock/assets",
                Some(PathBuf::from("warlock/assets/README.md")),
                NodeState::Unpacted,
            ),
        ]),
    )
}

#[cfg(test)]
mod tests {
    use super::stub_tree;
    use crate::NodeState;

    #[test]
    fn walk_visits_the_stub_depth_first_in_a_fixed_order() {
        let tree = stub_tree();
        let visited: Vec<(String, usize)> = tree
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
    fn the_stub_nests_more_than_one_level() {
        let deepest = stub_tree()
            .walk()
            .map(|(_, depth)| depth)
            .max()
            .expect("the walk always yields the root");
        assert!(deepest > 1, "deepest node sat at depth {deepest}");
    }

    #[test]
    fn counts_of_the_stub_are_the_hand_written_ones() {
        let counts = stub_tree().counts();
        assert_eq!(counts.unpacted, 1);
        assert_eq!(counts.pacted_stale, 3);
        assert_eq!(counts.pacted_fresh, 1);
        assert_eq!(counts.total(), 5);
    }

    #[test]
    fn every_state_appears_in_the_stub() {
        let counts = stub_tree().counts();
        for state in NodeState::ALL {
            assert!(counts.get(state) > 0, "no node is {state:?}");
        }
    }
}
