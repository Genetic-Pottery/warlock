//! The shape of the tree.
//!
//! Section 5 of the design doc makes the project tree the interface: a tree of
//! module READMEs, each coloured by its [`NodeState`]. This module gives that
//! tree a type. It is pure shape and nothing else — building a tree from a real
//! directory belongs to a filesystem loader that does not exist yet, and the
//! state on a node is a plain stored field, never computed here.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::NodeState;

/// One node of the project tree: a directory, the README that documents it,
/// and whatever Warlock currently knows about it.
///
/// The fields are public on purpose. A renderer walks this structure with each
/// node's depth and state in hand, and hiding `children` behind an accessor
/// would make that walk awkward for no gain — there is no invariant between
/// the fields to protect.
///
/// `state` is stored, not derived. Nothing in this crate computes staleness;
/// whoever builds the node decides what it says.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Node {
    /// The directory this node stands for.
    pub path: PathBuf,
    /// The README documenting this node. Held separately from `path` because
    /// the file name is not Warlock's to assume.
    pub readme: PathBuf,
    /// What Warlock knows about this node right now.
    pub state: NodeState,
    /// Child nodes, in the order they should be rendered. Empty for a leaf.
    pub children: Vec<Node>,
}

impl Node {
    /// A childless node at `path`, documented by `readme`, in `state`.
    ///
    /// Add children with [`Node::with_children`] or by pushing onto
    /// [`Node::children`] directly.
    #[must_use]
    pub fn new(
        path: impl Into<PathBuf>,
        readme: impl Into<PathBuf>,
        state: NodeState,
    ) -> Self {
        Self {
            path: path.into(),
            readme: readme.into(),
            state,
            children: Vec::new(),
        }
    }

    /// The same node with `children` attached, for building literals in one
    /// expression instead of a pile of `push` statements.
    #[must_use]
    pub fn with_children(mut self, children: impl IntoIterator<Item = Node>) -> Self {
        self.children = children.into_iter().collect();
        self
    }

    /// Whether this node has no children.
    #[must_use]
    pub fn is_leaf(&self) -> bool {
        self.children.is_empty()
    }
}

/// A whole project tree, owning its root node.
///
/// A tree is a root and nothing more; every node below it hangs off
/// [`Tree::root`]. Callers outside the crate build one with [`Tree::new`], so
/// a test or another front end can hand the engine a tree without going
/// through any loader.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Tree {
    /// The node every other node descends from.
    pub root: Node,
}

impl Tree {
    /// A tree rooted at `root`.
    #[must_use]
    pub fn new(root: Node) -> Self {
        Self { root }
    }

    /// The path of the root node, which is the path the whole tree is scoped
    /// to.
    #[must_use]
    pub fn root_path(&self) -> &Path {
        &self.root.path
    }
}

#[cfg(test)]
mod tests {
    use super::{Node, Tree};
    use crate::NodeState;

    /// A hand-written fixture: a root with one leaf child and one child that
    /// has a leaf of its own, so nesting is more than one level deep.
    fn fixture() -> Tree {
        Tree::new(
            Node::new("repo", "repo/README.md", NodeState::PactedStale).with_children([
                Node::new("repo/crates", "repo/crates/README.md", NodeState::Unpacted),
                Node::new("repo/docs", "repo/docs/README.md", NodeState::PactedFresh)
                    .with_children([Node::new(
                        "repo/docs/adr",
                        "repo/docs/adr/README.md",
                        NodeState::PactedStale,
                    )]),
            ]),
        )
    }

    #[test]
    fn new_node_starts_childless() {
        let node = Node::new("a", "a/README.md", NodeState::Unpacted);
        assert!(node.children.is_empty());
        assert!(node.is_leaf());
    }

    #[test]
    fn with_children_attaches_them_in_order() {
        let tree = fixture();
        assert!(!tree.root.is_leaf());
        let names: Vec<_> = tree
            .root
            .children
            .iter()
            .map(|child| child.path.to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["repo/crates", "repo/docs"]);
    }

    #[test]
    fn tree_owns_its_root() {
        let tree = fixture();
        assert_eq!(tree.root_path(), std::path::Path::new("repo"));
        assert_eq!(tree.root.state, NodeState::PactedStale);
        assert_eq!(tree.root.readme, std::path::PathBuf::from("repo/README.md"));
    }

    #[test]
    fn children_nest_more_than_one_level() {
        let tree = fixture();
        let docs = &tree.root.children[1];
        assert_eq!(docs.children.len(), 1);
        assert_eq!(docs.children[0].state, NodeState::PactedStale);
        assert!(docs.children[0].is_leaf());
    }
}
