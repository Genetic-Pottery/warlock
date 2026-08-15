//! Core engine for warlock.
//!
//! This crate owns the domain vocabulary. It never depends on the TUI or on
//! any terminal crate, and it reads no files and opens no sockets: the
//! dependency edge runs TUI -> engine and never back.

mod state;
mod tree;

/// The three-state vocabulary every node is coloured by.
pub use state::NodeState;
/// A depth-first walk over a tree, yielding each node with its depth.
pub use tree::DepthFirst;
/// One node of the project tree: its path, its README, its state, its children.
pub use tree::Node;
/// How many nodes of a tree sit in each state.
pub use tree::StateCounts;
/// A whole project tree, owning its root node.
pub use tree::Tree;

#[cfg(test)]
mod tests {
    /// Placeholder: keeps the test target wired up until real behaviour lands.
    #[test]
    fn crate_is_named_warlock_engine() {
        assert_eq!(env!("CARGO_PKG_NAME"), "warlock-engine");
    }
}
