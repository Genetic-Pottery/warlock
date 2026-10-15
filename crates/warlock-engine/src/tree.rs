use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Node {
    pub path: PathBuf,
    // The scope this directory's own row carries, and never an ancestor's: a
    // node says which boundary starts at it.
    #[serde(default)]
    pub scope: Option<String>,
    // The scope that governs this directory, its own or the nearest
    // ancestor's: what decides whether warlock may work here at all.
    #[serde(default)]
    pub covering: Option<String>,
    pub children: Vec<Node>,
    pub files: Vec<PathBuf>,
}

impl Node {
    /// ```
    /// use warlock_engine::Node;
    ///
    /// let module = Node::new("repo/docs");
    /// assert!(module.files.is_empty());
    /// assert_eq!(module.scope, None);
    /// assert_eq!(module.covering, None);
    /// ```
    #[must_use]
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            scope: None,
            covering: None,
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
    /// use warlock_engine::Node;
    ///
    /// let module = Node::new("repo/docs")
    ///     .with_files(["repo/docs/guide.md", "repo/docs/adr.md"].map(std::path::PathBuf::from));
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
    /// use warlock_engine::Node;
    ///
    /// let engine = Node::new("repo/engine").with_scope(Some("data-plane".to_owned()));
    /// assert_eq!(engine.scope.as_deref(), Some("data-plane"));
    /// // Its own scope governs it.
    /// assert_eq!(engine.covering.as_deref(), Some("data-plane"));
    /// assert_eq!(engine.with_scope(None).scope, None);
    /// ```
    #[must_use]
    pub fn with_scope(mut self, scope: Option<String>) -> Self {
        self.covering.clone_from(&scope);
        self.scope = scope;
        self
    }

    /// ```
    /// use warlock_engine::Node;
    ///
    /// let src = Node::new("repo/engine/src").with_covering(Some("data-plane".to_owned()));
    /// assert_eq!(src.scope, None);
    /// assert!(src.is_covered());
    /// ```
    #[must_use]
    pub fn with_covering(mut self, covering: Option<String>) -> Self {
        self.covering = covering;
        self
    }

    #[must_use]
    pub fn is_covered(&self) -> bool {
        self.covering.is_some()
    }

    #[must_use]
    pub fn is_leaf(&self) -> bool {
        self.children.is_empty()
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
    /// use warlock_engine::{Node, Tree};
    ///
    /// let tree = Tree::new(
    ///     Node::new("repo")
    ///         .with_children([Node::new("repo/docs").with_scope(Some("docs".to_owned()))]),
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
    /// use warlock_engine::{Node, Tree};
    ///
    /// let tree = Tree::new(
    ///     Node::new("repo")
    ///         .with_children([Node::new("repo/docs").with_scope(Some("docs".to_owned()))]),
    /// );
    ///
    /// let found = tree.find("repo/docs").expect("docs is in the tree");
    /// assert_eq!(found.scope.as_deref(), Some("docs"));
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

#[cfg(test)]
#[path = "tests/tree.rs"]
mod tests;
