//! The shape of the tree.
//!
//! Section 5 of the design doc makes the project tree the interface: a tree of
//! module documents, each coloured by its [`NodeState`]. This module gives
//! that tree a type. It is pure shape and nothing else — building a tree from
//! a real directory belongs to [`load_tree`](crate::load_tree), and the state
//! on a node is a plain stored field, never computed here.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::NodeState;

/// One node of the project tree: a directory, the `WARLOCK.md` documenting it,
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
    /// Warlock's document for this node, or `None` when the node has none of
    /// its own — an ordinary directory that has no documentation yet, which is
    /// a node like any other. Held separately from `path` because the file
    /// name is not Warlock's to assume.
    pub document: Option<PathBuf>,
    /// What Warlock knows about this node right now.
    pub state: NodeState,
    /// Child nodes, in the order they should be rendered. Empty for a leaf.
    pub children: Vec<Node>,
    /// The files sitting directly in this directory, in path order.
    ///
    /// A listing and nothing more. A file is not a node: it has no state, no
    /// document and no children, it is no part of [`is_leaf`](Node::is_leaf)
    /// — a node with files and no subdirectories is still a leaf — and nothing
    /// here is hashed, since a pacted subtree's digest is taken from disk
    /// rather than from this list. Subdirectories are not listed here either;
    /// they are `children`.
    ///
    /// A loaded node lists what the walk saw directly inside the directory,
    /// its own `WARLOCK.md` included: this is a faithful listing rather than a
    /// listing minus one special name, and a view that would rather not draw
    /// the document twice can leave it out on the way to the screen. Only what
    /// the walk yielded appears, so ignored and hidden files are absent
    /// exactly as ignored and hidden directories are — and, the same fact seen
    /// from the other side, a `WARLOCK.md` an ignore rule covers still
    /// documents its node through `document` while not appearing here.
    pub files: Vec<PathBuf>,
}

impl Node {
    /// A childless node at `path`, documented by `document`, in `state`,
    /// holding no files.
    ///
    /// `document` is anything path-like for a node that has one, or `None`
    /// for a node that does not — see [`IntoDocument`].
    ///
    /// Add children with [`Node::with_children`] and files with
    /// [`Node::with_files`], or by pushing onto [`Node::children`] and
    /// [`Node::files`] directly.
    ///
    /// ```
    /// use warlock_engine::{Node, NodeState};
    ///
    /// let module = Node::new("repo/docs", "repo/docs/WARLOCK.md", NodeState::Unpacted);
    /// assert!(module.document.is_some());
    /// assert!(module.files.is_empty());
    ///
    /// let undocumented = Node::new("repo/crates", None, NodeState::Unpacted);
    /// assert_eq!(undocumented.document, None);
    /// ```
    #[must_use]
    pub fn new(path: impl Into<PathBuf>, document: impl IntoDocument, state: NodeState) -> Self {
        Self {
            path: path.into(),
            document: document.into_document(),
            state,
            children: Vec::new(),
            files: Vec::new(),
        }
    }

    /// The same node with `children` attached, for building literals in one
    /// expression instead of a pile of `push` statements.
    #[must_use]
    pub fn with_children(mut self, children: impl IntoIterator<Item = Node>) -> Self {
        self.children = children.into_iter().collect();
        self
    }

    /// The same node with `files` attached, the companion to
    /// [`with_children`](Node::with_children) for the paths that are a listing
    /// rather than nodes of their own.
    ///
    /// The files are stored in the order given and are not sorted here, for
    /// the same reason children are not reordered: this is a constructor, not
    /// a policy. Whoever builds the node decides the order — the loader hands
    /// them over in path order.
    ///
    /// ```
    /// use warlock_engine::{Node, NodeState};
    ///
    /// let module = Node::new("repo/docs", "repo/docs/WARLOCK.md", NodeState::Unpacted)
    ///     .with_files(["repo/docs/WARLOCK.md", "repo/docs/adr.md"].map(std::path::PathBuf::from));
    /// assert_eq!(module.files.len(), 2);
    /// // Files are a listing, not children: a node with files is still a leaf.
    /// assert!(module.is_leaf());
    /// ```
    #[must_use]
    pub fn with_files(mut self, files: impl IntoIterator<Item = PathBuf>) -> Self {
        self.files = files.into_iter().collect();
        self
    }

    /// Whether this node has no children.
    ///
    /// Child *nodes*, that is: [`files`](Node::files) do not count, so a
    /// directory full of files and no subdirectories is a leaf.
    #[must_use]
    pub fn is_leaf(&self) -> bool {
        self.children.is_empty()
    }
}

/// What [`Node::new`] accepts for a node's document.
///
/// A node's document is optional, but most nodes have one and saying `Some`
/// at every such call site would be noise. This trait takes both forms:
/// anything path-like means that document, and `None` means the node has none.
///
/// The impls are written out one type at a time rather than blanketed over
/// `Into<PathBuf>`, because a blanket impl plus one for `Option` overlap as
/// far as coherence is concerned. Listing them also keeps `None` on its own
/// inferring to `Option<PathBuf>`, since that is the only impl it can match.
pub trait IntoDocument {
    /// This value as a node stores it: the document's path, or `None` for a
    /// node without one.
    fn into_document(self) -> Option<PathBuf>;
}

impl IntoDocument for Option<PathBuf> {
    fn into_document(self) -> Option<PathBuf> {
        self
    }
}

impl IntoDocument for PathBuf {
    fn into_document(self) -> Option<PathBuf> {
        Some(self)
    }
}

impl IntoDocument for &Path {
    fn into_document(self) -> Option<PathBuf> {
        Some(self.to_path_buf())
    }
}

impl IntoDocument for &str {
    fn into_document(self) -> Option<PathBuf> {
        Some(PathBuf::from(self))
    }
}

impl IntoDocument for String {
    fn into_document(self) -> Option<PathBuf> {
        Some(PathBuf::from(self))
    }
}

/// A whole project tree, owning its root node.
///
/// A tree is a root and nothing more; every node below it hangs off
/// [`Tree::root`]. Callers outside the crate build one with [`Tree::new`], so
/// a test or another front end can hand the engine a tree without going
/// through any loader.
///
/// A loaded tree holds a node per directory, documented or not: one whose
/// [`document`](Node::document) is `None` is an ordinary directory that has no
/// documentation yet, not a lesser kind of node. Rendering fewer of them than
/// the tree holds is a view's decision, taken on the way to the screen.
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
    ///     Node::new("repo", "repo/WARLOCK.md", NodeState::PactedStale)
    ///         .with_children([Node::new(
    ///             "repo/docs",
    ///             "repo/docs/WARLOCK.md",
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
    ///     Node::new("repo", "repo/WARLOCK.md", NodeState::PactedStale)
    ///         .with_children([Node::new(
    ///             "repo/docs",
    ///             "repo/docs/WARLOCK.md",
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
    /// access: a tree is just values, and whoever built it — a loader, a test,
    /// a deserialiser — decided what those paths say.
    ///
    /// ```
    /// use warlock_engine::{Node, NodeState, Tree};
    ///
    /// let tree = Tree::new(
    ///     Node::new("repo", "repo/WARLOCK.md", NodeState::PactedStale)
    ///         .with_children([Node::new(
    ///             "repo/docs",
    ///             "repo/docs/WARLOCK.md",
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
    use std::path::PathBuf;

    use super::{Node, Tree};
    use crate::NodeState;

    /// A hand-written fixture: a root with one leaf child and one child that
    /// has a leaf of its own, so nesting is more than one level deep.
    fn fixture() -> Tree {
        Tree::new(
            Node::new("repo", "repo/WARLOCK.md", NodeState::PactedStale).with_children([
                // A directory with no document of its own, and a node all
                // the same.
                Node::new("repo/crates", None, NodeState::Unpacted),
                Node::new("repo/docs", "repo/docs/WARLOCK.md", NodeState::PactedFresh)
                    .with_children([Node::new(
                        "repo/docs/adr",
                        "repo/docs/adr/WARLOCK.md",
                        NodeState::PactedStale,
                    )]),
            ]),
        )
    }

    /// The same fixture with two files listed on every node, so a test can ask
    /// what difference files make by holding the two trees side by side.
    fn fixture_with_files() -> Tree {
        fn listing(node: &Node) -> Node {
            Node::new(&node.path, node.document.clone(), node.state)
                .with_children(node.children.iter().map(listing))
                .with_files([node.path.join("Cargo.toml"), node.path.join("notes.md")])
        }
        Tree::new(listing(&fixture().root))
    }

    #[test]
    fn new_node_starts_childless() {
        let node = Node::new("a", "a/WARLOCK.md", NodeState::Unpacted);
        assert!(node.children.is_empty());
        assert!(node.files.is_empty());
        assert!(node.is_leaf());
    }

    #[test]
    fn with_files_attaches_them_in_order_and_leaves_the_node_a_leaf() {
        // A plain `README.md` rides along as an ordinary file: it is a listing
        // entry like any other and documents nothing.
        let node = Node::new("a", "a/WARLOCK.md", NodeState::Unpacted)
            .with_files(["a/WARLOCK.md", "a/README.md", "a/Cargo.toml"].map(PathBuf::from));
        assert_eq!(
            node.files,
            [
                PathBuf::from("a/WARLOCK.md"),
                PathBuf::from("a/README.md"),
                PathBuf::from("a/Cargo.toml"),
            ],
            "stored as given: ordering is the caller's business",
        );
        assert!(
            node.is_leaf(),
            "files are a listing, not children, so they cannot unmake a leaf",
        );
        assert!(node.children.is_empty());
    }

    #[test]
    fn files_are_no_part_of_walking_or_counting() {
        let listed: Vec<_> = fixture_with_files()
            .walk()
            .map(|(node, depth)| (node.path.clone(), depth))
            .collect();
        let bare: Vec<_> = fixture()
            .walk()
            .map(|(node, depth)| (node.path.clone(), depth))
            .collect();

        assert_eq!(listed, bare, "a file is never yielded as a node");
        assert_eq!(
            fixture_with_files().counts(),
            fixture().counts(),
            "the tally counts nodes, and a file is not one",
        );
        assert!(
            fixture_with_files()
                .walk()
                .all(|(node, _)| node.files.len() == 2),
            "the files really are there to be missed",
        );
    }

    #[test]
    fn a_document_is_stored_however_it_was_given() {
        let from_str = Node::new("a", "a/WARLOCK.md", NodeState::Unpacted);
        let from_path_buf = Node::new(
            "a",
            std::path::PathBuf::from("a/WARLOCK.md"),
            NodeState::Unpacted,
        );
        let from_option = Node::new(
            "a",
            Some(std::path::PathBuf::from("a/WARLOCK.md")),
            NodeState::Unpacted,
        );
        assert_eq!(from_str, from_path_buf);
        assert_eq!(from_str, from_option);
        assert_eq!(
            from_str.document,
            Some(std::path::PathBuf::from("a/WARLOCK.md"))
        );
    }

    #[test]
    fn a_node_with_no_document_stores_none() {
        let node = Node::new("repo/crates", None, NodeState::Unpacted);
        assert_eq!(node.document, None);
        assert_eq!(
            fixture().find("repo/crates").expect("in the tree").document,
            None
        );
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
        assert_eq!(
            tree.root.document,
            Some(std::path::PathBuf::from("repo/WARLOCK.md"))
        );
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
        let tree = Tree::new(Node::new("solo", "solo/WARLOCK.md", NodeState::Unpacted));
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
        let tree = Tree::new(Node::new("solo", "solo/WARLOCK.md", NodeState::Unpacted));
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
            tree.find("repo/docs/adr")
                .map(|node| node.document.as_deref()),
            Some(Some(std::path::Path::new("repo/docs/adr/WARLOCK.md")))
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

    #[test]
    fn a_tree_carrying_files_survives_a_serde_round_trip() {
        use serde_test::{Token, assert_tokens};

        // A root with one child, each listing files, so the round trip covers
        // a nested node and a `files` list that is neither empty nor shared.
        // The tokens are written out by hand rather than derived from the
        // value: a renamed or reordered field fails here instead of passing
        // silently. No format is involved, so nothing below fixes an on-disk
        // representation.
        let tree = Tree::new(
            Node::new("repo", "repo/WARLOCK.md", NodeState::PactedStale)
                .with_children([Node::new("repo/docs", None, NodeState::Unpacted)
                    .with_files([PathBuf::from("repo/docs/adr.md")])])
                .with_files([
                    PathBuf::from("repo/Cargo.toml"),
                    PathBuf::from("repo/WARLOCK.md"),
                ]),
        );

        assert_tokens(
            &tree,
            &[
                Token::Struct {
                    name: "Tree",
                    len: 1,
                },
                Token::Str("root"),
                Token::Struct {
                    name: "Node",
                    len: 5,
                },
                Token::Str("path"),
                Token::Str("repo"),
                Token::Str("document"),
                Token::Some,
                Token::Str("repo/WARLOCK.md"),
                Token::Str("state"),
                Token::UnitVariant {
                    name: "NodeState",
                    variant: "PactedStale",
                },
                Token::Str("children"),
                Token::Seq { len: Some(1) },
                Token::Struct {
                    name: "Node",
                    len: 5,
                },
                Token::Str("path"),
                Token::Str("repo/docs"),
                Token::Str("document"),
                Token::None,
                Token::Str("state"),
                Token::UnitVariant {
                    name: "NodeState",
                    variant: "Unpacted",
                },
                Token::Str("children"),
                Token::Seq { len: Some(0) },
                Token::SeqEnd,
                Token::Str("files"),
                Token::Seq { len: Some(1) },
                Token::Str("repo/docs/adr.md"),
                Token::SeqEnd,
                Token::StructEnd,
                Token::SeqEnd,
                Token::Str("files"),
                Token::Seq { len: Some(2) },
                Token::Str("repo/Cargo.toml"),
                Token::Str("repo/WARLOCK.md"),
                Token::SeqEnd,
                Token::StructEnd,
                Token::StructEnd,
            ],
        );
    }
}
