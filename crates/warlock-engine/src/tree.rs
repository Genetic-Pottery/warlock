use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::NodeState;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Node {
    pub path: PathBuf,
    pub document: Option<PathBuf>,
    pub state: NodeState,
    #[serde(default)]
    pub ignored: bool,
    #[serde(default)]
    pub scope: Option<String>,
    pub children: Vec<Node>,
    pub files: Vec<PathBuf>,
}

impl Node {
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
            ignored: false,
            scope: None,
            children: Vec::new(),
            files: Vec::new(),
        }
    }

    #[must_use]
    pub fn with_children(mut self, children: impl IntoIterator<Item = Node>) -> Self {
        self.children = children.into_iter().collect();
        self
    }

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

    /// ```
    /// use warlock_engine::{Node, NodeState};
    ///
    /// let assets = Node::new("repo/assets", None, NodeState::Unpacted).with_ignored(true);
    /// assert!(assets.is_ignored());
    /// // Still an ordinary unpacted node: no fourth state, no fourth colour.
    /// assert_eq!(assets.state, NodeState::Unpacted);
    /// ```
    #[must_use]
    pub fn with_ignored(mut self, ignored: bool) -> Self {
        self.ignored = ignored;
        self
    }

    #[must_use]
    pub fn is_ignored(&self) -> bool {
        self.ignored
    }

    /// ```
    /// use warlock_engine::{Node, NodeState};
    ///
    /// let engine = Node::new("repo/engine", "repo/engine/WARLOCK.md", NodeState::PactedFresh)
    ///     .with_scope(Some("data-plane".to_owned()));
    /// assert_eq!(engine.scope.as_deref(), Some("data-plane"));
    /// // Still an ordinary fresh node: a scope gates nothing and colours nothing.
    /// assert_eq!(engine.state, NodeState::PactedFresh);
    /// assert_eq!(engine.with_scope(None).scope, None);
    /// ```
    #[must_use]
    pub fn with_scope(mut self, scope: Option<String>) -> Self {
        self.scope = scope;
        self
    }

    #[must_use]
    pub fn is_leaf(&self) -> bool {
        self.children.is_empty()
    }
}

pub trait IntoDocument {
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

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Tree {
    pub root: Node,
}

impl Tree {
    #[must_use]
    pub fn new(root: Node) -> Self {
        Self { root }
    }

    #[must_use]
    pub fn root_path(&self) -> &Path {
        &self.root.path
    }

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

#[derive(Debug, Clone)]
pub struct DepthFirst<'a> {
    stack: Vec<(&'a Node, usize)>,
}

impl<'a> DepthFirst<'a> {
    fn new(root: &'a Node) -> Self {
        Self {
            stack: vec![(root, 0)],
        }
    }
}

impl<'a> Iterator for DepthFirst<'a> {
    type Item = (&'a Node, usize);

    fn next(&mut self) -> Option<Self::Item> {
        let (node, depth) = self.stack.pop()?;
        self.stack
            .extend(node.children.iter().rev().map(|child| (child, depth + 1)));
        Some((node, depth))
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct StateCounts {
    pub unpacted: usize,
    pub pacted_stale: usize,
    pub pacted_fresh: usize,
}

impl StateCounts {
    #[must_use]
    pub const fn get(&self, state: NodeState) -> usize {
        match state {
            NodeState::Unpacted => self.unpacted,
            NodeState::PactedStale => self.pacted_stale,
            NodeState::PactedFresh => self.pacted_fresh,
        }
    }

    #[must_use]
    pub const fn total(&self) -> usize {
        self.unpacted + self.pacted_stale + self.pacted_fresh
    }

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
    fn a_node_is_covered_until_something_says_otherwise() {
        let node = Node::new("a", "a/WARLOCK.md", NodeState::Unpacted);
        assert!(
            !node.is_ignored(),
            "only a loader that read the rules can say a directory is excluded",
        );

        let excluded = node.clone().with_ignored(true);
        assert!(excluded.is_ignored());
        assert_eq!(excluded.ignored, excluded.is_ignored());
        assert_eq!(
            excluded.state, node.state,
            "being excluded is not a state: the node is unpacted either way",
        );
        assert_eq!(excluded.document, node.document);
        assert_eq!(excluded.children, node.children);
        assert_eq!(excluded.files, node.files);
        assert!(
            !excluded.with_ignored(false).is_ignored(),
            "the setter sets rather than latches",
        );
    }

    #[test]
    fn a_node_carries_its_own_scope_and_only_a_loader_can_put_one_there() {
        let node = Node::new("a", "a/WARLOCK.md", NodeState::PactedFresh);
        assert_eq!(
            node.scope, None,
            "a scope lives on a manifest entry, so only a loader that read one \
             can say a directory carries it",
        );

        let scoped = node.clone().with_scope(Some("data-plane".to_owned()));
        assert_eq!(scoped.scope.as_deref(), Some("data-plane"));
        assert_eq!(
            scoped.state, node.state,
            "a scope is a label, not a state: the node is fresh either way",
        );
        assert_eq!(scoped.document, node.document);
        assert_eq!(scoped.children, node.children);
        assert_eq!(scoped.files, node.files);
        assert_eq!(
            scoped.with_scope(None).scope,
            None,
            "the setter sets rather than latches",
        );
    }

    #[test]
    fn a_node_written_before_the_scope_existed_reads_back_unscoped() {
        use serde_test::{Token, assert_de_tokens};

        // The round trip below, minus `scope` entirely: a tree serialised by a
        // build that had no such field. It must still deserialise, and the
        // missing fact must read as "no scope" rather than fail.
        assert_de_tokens(
            &Node::new("repo", "repo/WARLOCK.md", NodeState::PactedFresh),
            &[
                Token::Struct {
                    name: "Node",
                    len: 6,
                },
                Token::Str("path"),
                Token::Str("repo"),
                Token::Str("document"),
                Token::Some,
                Token::Str("repo/WARLOCK.md"),
                Token::Str("state"),
                Token::UnitVariant {
                    name: "NodeState",
                    variant: "PactedFresh",
                },
                Token::Str("ignored"),
                Token::Bool(false),
                Token::Str("children"),
                Token::Seq { len: Some(0) },
                Token::SeqEnd,
                Token::Str("files"),
                Token::Seq { len: Some(0) },
                Token::SeqEnd,
                Token::StructEnd,
            ],
        );
    }

    #[test]
    fn a_node_written_before_the_flag_existed_reads_back_as_covered() {
        use serde_test::{Token, assert_de_tokens};

        // The same tokens as the round trip below, minus `ignored` entirely:
        // a tree serialised by an older build. It must still deserialise, and
        // the missing fact must read as "not excluded" rather than fail.
        assert_de_tokens(
            &Node::new("repo", "repo/WARLOCK.md", NodeState::PactedFresh),
            &[
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
                    variant: "PactedFresh",
                },
                Token::Str("children"),
                Token::Seq { len: Some(0) },
                Token::SeqEnd,
                Token::Str("files"),
                Token::Seq { len: Some(0) },
                Token::SeqEnd,
                Token::StructEnd,
            ],
        );
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
                .with_scope(Some("data-plane".to_owned()))
                .with_children([Node::new("repo/docs", None, NodeState::Unpacted)
                    .with_ignored(true)
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
                    len: 7,
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
                Token::Str("ignored"),
                Token::Bool(false),
                Token::Str("scope"),
                Token::Some,
                Token::Str("data-plane"),
                Token::Str("children"),
                Token::Seq { len: Some(1) },
                Token::Struct {
                    name: "Node",
                    len: 7,
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
                Token::Str("ignored"),
                Token::Bool(true),
                Token::Str("scope"),
                Token::None,
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
