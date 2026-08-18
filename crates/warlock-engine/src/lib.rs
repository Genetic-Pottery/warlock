//! Core engine for warlock.
//!
//! This crate owns the domain vocabulary. It never depends on the TUI or on
//! any terminal crate: the dependency edge runs TUI -> engine and never back.
//! It does read and write files — the pact manifest at `.warlock/pacts.toml`
//! is its business — but it opens no sockets and spawns no subprocesses, and
//! it only ever touches the paths a caller hands it.

mod manifest;
mod state;
mod stub;
mod tree;

/// Everything that can go wrong reading, writing or building a manifest.
pub use manifest::Error as ManifestError;
/// The record of which modules are pacted: one `.warlock/pacts.toml` per
/// repository.
pub use manifest::Manifest;
/// One pacted module: its directory, its README, and whatever was granted to
/// it.
pub use manifest::PactEntry;
/// The manifest schema version this build reads and writes.
pub use manifest::SCHEMA_VERSION;
/// A manifest-relative path, back as a path under a root directory.
pub use manifest::from_manifest_path;
/// Where the manifest lives under a repository root.
pub use manifest::manifest_path;
/// A caller's path in the form the manifest stores: relative, forward slashes.
pub use manifest::to_manifest_path;
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
