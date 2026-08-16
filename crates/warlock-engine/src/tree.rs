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
    pub fn new(path: impl Into<PathBuf>, readme: impl Into<PathBuf>, state: NodeState) -> Self {
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

    /// Every node, depth first and parents before children, each paired with
    /// how deep it sits. The root is depth `0`, its children depth `1`, and so
    /// on.
    ///
    /// The depth is yielded rather than left implicit because the renderer
    /// indents by it: walking the tree and knowing where you are in it should
    /// be one pass, not two. Siblings come in the order they are stored.
    ///
    /// ```
    /// use warlock_engine::{Node, NodeState, Tree};
    ///
    /// let tree = Tree::new(
    ///     Node::new("repo", "repo/README.md", NodeState::PactedStale)
    ///         .with_children([Node::new(
    ///             "repo/docs",
    ///             "repo/docs/README.md",
    ///             NodeState::PactedFresh,
    ///         )]),
    /// );
    ///
    /// let lines: Vec<String> = tree
    ///     .walk()
    ///     .map(|(node, depth)| format!("{}{}", "  ".repeat(depth), node.path.display()))
    ///     .collect();
    /// assert_eq!(lines, ["repo", "  repo/docs"]);
    /// ```
    #[must_use]
    pub fn walk(&self) -> DepthFirst<'_> {
        DepthFirst::new(&self.root)
    }

    /// How many nodes sit in each state.
    ///
    /// The result is a fixed struct with one field per state, so a state can
    /// neither be missed nor invented; a state with no nodes counts zero.
    ///
    /// ```
    /// use warlock_engine::{Node, NodeState, Tree};
    ///
    /// let tree = Tree::new(
    ///     Node::new("repo", "repo/README.md", NodeState::PactedStale)
    ///         .with_children([Node::new(
    ///             "repo/docs",
    ///             "repo/docs/README.md",
    ///             NodeState::PactedFresh,
    ///         )]),
    /// );
    ///
    /// let counts = tree.counts();
    /// assert_eq!(counts.pacted_stale, 1);
    /// assert_eq!(counts.pacted_fresh, 1);
    /// assert_eq!(counts.unpacted, 0);
    /// assert_eq!(counts.total(), 2);
    /// ```
    #[must_use]
    pub fn counts(&self) -> StateCounts {
        let mut counts = StateCounts::default();
        for (node, _) in self.walk() {
            *counts.get_mut(node.state) += 1;
        }
        counts
    }

    /// The node at `path`, or `None` if the tree holds no such node.
    ///
    /// Paths are compared as stored, with no normalisation and no filesystem
    /// access: this crate never touches the disk, so it cannot canonicalise
    /// and will not pretend to.
    ///
    /// ```
    /// use warlock_engine::{Node, NodeState, Tree};
    ///
    /// let tree = Tree::new(
    ///     Node::new("repo", "repo/README.md", NodeState::PactedStale)
    ///         .with_children([Node::new(
    ///             "repo/docs",
    ///             "repo/docs/README.md",
    ///             NodeState::PactedFresh,
    ///         )]),
    /// );
    ///
    /// let found = tree.find("repo/docs").expect("docs is in the tree");
    /// assert_eq!(found.state, NodeState::PactedFresh);
    /// assert!(tree.find("repo/nowhere").is_none());
    /// ```
    #[must_use]
    pub fn find(&self, path: impl AsRef<Path>) -> Option<&Node> {
        let path = path.as_ref();
        self.walk()
            .find_map(|(node, _)| (node.path == path).then_some(node))
    }
}

/// A depth-first walk over a tree, yielding each node with its depth.
///
/// Built by [`Tree::walk`]. Parents come before their children and siblings
/// keep the order they are stored in, so the sequence is exactly what a
/// renderer draws top to bottom.
#[derive(Debug, Clone)]
pub struct DepthFirst<'a> {
    /// Nodes still owed, with their depth, nearest first. Children are pushed
    /// in reverse so the leftmost sibling comes off the stack first.
    stack: Vec<(&'a Node, usize)>,
}

impl<'a> DepthFirst<'a> {
    /// A walk starting at `root`, which is reported at depth `0`.
    fn new(root: &'a Node) -> Self {
        Self {
            stack: vec![(root, 0)],
        }
    }
}

impl<'a> Iterator for DepthFirst<'a> {
    /// A node and how deep it sits below the root.
    type Item = (&'a Node, usize);

    fn next(&mut self) -> Option<Self::Item> {
        let (node, depth) = self.stack.pop()?;
        self.stack
            .extend(node.children.iter().rev().map(|child| (child, depth + 1)));
        Some((node, depth))
    }
}

/// How many nodes sit in each state.
///
/// One field per [`NodeState`] variant, so no state can be missing from a
/// tally and no fourth state can appear in one. A state with no nodes is zero,
/// which is what [`Default`] gives.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct StateCounts {
    /// Nodes outside Warlock's management.
    pub unpacted: usize,
    /// Pacted nodes owing a freshness pass.
    pub pacted_stale: usize,
    /// Pacted nodes that have been granted freshness.
    pub pacted_fresh: usize,
}

impl StateCounts {
    /// The count for one state, for callers that hold a state rather than a
    /// field name (a legend, say, iterating [`NodeState::ALL`]).
    #[must_use]
    pub const fn get(&self, state: NodeState) -> usize {
        match state {
            NodeState::Unpacted => self.unpacted,
            NodeState::PactedStale => self.pacted_stale,
            NodeState::PactedFresh => self.pacted_fresh,
        }
    }

    /// How many nodes were counted in total.
    #[must_use]
    pub const fn total(&self) -> usize {
        self.unpacted + self.pacted_stale + self.pacted_fresh
    }

    /// The field for `state`, so counting stays a match on the enum and
    /// cannot silently skip a variant.
    fn get_mut(&mut self, state: NodeState) -> &mut usize {
        match state {
            NodeState::Unpacted => &mut self.unpacted,
            NodeState::PactedStale => &mut self.pacted_stale,
            NodeState::PactedFresh => &mut self.pacted_fresh,
        }
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
    fn walk_is_depth_first_with_parents_before_children() {
        let tree = fixture();
        let visited: Vec<(String, usize)> = tree
            .walk()
            .map(|(node, depth)| (node.path.to_string_lossy().into_owned(), depth))
            .collect();
        assert_eq!(
            visited,
            [
                ("repo".to_owned(), 0),
                ("repo/crates".to_owned(), 1),
                ("repo/docs".to_owned(), 1),
                ("repo/docs/adr".to_owned(), 2),
            ]
        );
    }

    #[test]
    fn walk_of_a_lone_node_yields_only_the_root_at_depth_zero() {
        let tree = Tree::new(Node::new("solo", "solo/README.md", NodeState::Unpacted));
        let visited: Vec<_> = tree
            .walk()
            .map(|(node, depth)| (&node.state, depth))
            .collect();
        assert_eq!(visited, [(&NodeState::Unpacted, 0)]);
    }

    #[test]
    fn counts_tally_every_state() {
        let counts = fixture().counts();
        assert_eq!(counts.unpacted, 1);
        assert_eq!(counts.pacted_stale, 2);
        assert_eq!(counts.pacted_fresh, 1);
        assert_eq!(counts.total(), 4);
        for state in NodeState::ALL {
            let expected = fixture()
                .walk()
                .filter(|(node, _)| node.state == state)
                .count();
            assert_eq!(counts.get(state), expected, "count for {state:?}");
        }
    }

    #[test]
    fn counts_of_an_absent_state_are_zero() {
        let tree = Tree::new(Node::new("solo", "solo/README.md", NodeState::Unpacted));
        let counts = tree.counts();
        assert_eq!(counts.unpacted, 1);
        assert_eq!(counts.pacted_stale, 0);
        assert_eq!(counts.pacted_fresh, 0);
    }

    #[test]
    fn find_reaches_the_root_and_the_deepest_node() {
        let tree = fixture();
        assert_eq!(
            tree.find("repo").map(|node| node.state),
            Some(NodeState::PactedStale)
        );
        assert_eq!(
            tree.find("repo/docs/adr").map(|node| &node.readme),
            Some(&std::path::PathBuf::from("repo/docs/adr/README.md"))
        );
    }

    #[test]
    fn find_returns_none_for_a_path_not_in_the_tree() {
        let tree = fixture();
        assert!(tree.find("repo/nowhere").is_none());
        // Paths are compared as stored: no normalisation, no filesystem.
        assert!(tree.find("./repo").is_none());
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
