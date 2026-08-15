//! Core engine for warlock.
//!
//! This crate owns the domain vocabulary. It never depends on the TUI or on
//! any terminal crate, and it reads no files and opens no sockets: the
//! dependency edge runs TUI -> engine and never back.

mod state;
mod stub;
mod tree;

/// The three-state vocabulary every node is coloured by.
pub use state::NodeState;
/// A placeholder tree, standing in for a filesystem loader that does not exist
/// yet.
pub use stub::stub_tree;
/// A depth-first walk over a tree, yielding each node with its depth.
pub use tree::DepthFirst;
/// One node of the project tree: its path, its README, its state, its children.
pub use tree::Node;
/// How many nodes of a tree sit in each state.
pub use tree::StateCounts;
/// A whole project tree, owning its root node.
pub use tree::Tree;
