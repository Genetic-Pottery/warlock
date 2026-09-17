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
#[path = "tests/tree.rs"]
mod tests;
