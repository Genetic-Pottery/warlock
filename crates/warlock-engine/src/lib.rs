//! Core engine for warlock.
//!
//! This crate owns the domain vocabulary. It never depends on the TUI or on
//! any terminal crate: the dependency edge runs TUI -> engine and never back.
//! It does touch the filesystem — it reads and writes the pact manifest at
//! `.warlock/pacts.toml`, it walks a directory to build a [`Tree`] from it,
//! and it reads the bytes of the files under a directory to hash them
//! ([`subtree_hash`]) — but it opens no sockets and spawns no subprocesses,
//! and it never follows a symlink out of the directory a caller hands it.

mod decide;
mod hash;
mod load;
mod manifest;
mod state;
mod stub;
mod tree;

/// The colour of a node, from its manifest entry and the hash of its content:
/// no entry is unpacted, a granted hash equal to the computed one is fresh, and
/// everything else — including never judged — is stale.
///
/// Nothing in this project grants freshness. There is no refresh pass and no
/// code in this workspace that writes a `granted_hash`, so
/// [`NodeState::PactedFresh`] is reachable only against a hash a human wrote
/// into `.warlock/pacts.toml` by hand.
pub use decide::decide_state;
/// Everything that can stop a subtree being hashed.
pub use hash::Error as HashError;
/// The hash of everything at and below a directory.
pub use hash::subtree_hash;
/// Everything that can stop a directory becoming a tree.
pub use load::Error as LoadError;
/// What a load produced: the coloured tree, plus its non-fatal problems.
pub use load::Loaded;
/// One node a load could not colour properly, and why. Non-fatal by
/// definition: the load that produced it finished.
pub use load::Problem as LoadProblem;
/// Build a tree from a directory on disk, coloured by the manifest above it.
pub use load::load_tree;
/// The nearest ancestor of a directory that holds a `.warlock/` directory.
pub use load::repository_root;
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
/// A hard-coded tree for exercising a renderer. Not the loader: that is
/// [`load_tree`].
pub use stub::stub_tree;
/// A depth-first walk over a tree, yielding each node with its depth.
pub use tree::DepthFirst;
/// What `Node::new` accepts for a README: anything path-like, or `None`.
pub use tree::IntoReadme;
/// One node of the project tree: its path, its README, its state, its children.
pub use tree::Node;
/// How many nodes of a tree sit in each state.
pub use tree::StateCounts;
/// A whole project tree, owning its root node.
pub use tree::Tree;
