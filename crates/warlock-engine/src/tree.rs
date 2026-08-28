//! The shape of the tree.
//!
//! Section 5 of the design doc makes the project tree the interface: a tree of
//! module documents, each coloured by its [`NodeState`]. This module gives
//! that tree a type. It is pure shape and nothing else — building a tree from
//! a real directory belongs to [`load_tree`](crate::load_tree), and the state
//! on a node is a plain stored field, never computed here.
//!
//! [`Node::ignored`] is the same kind of thing: a plain stored fact, put there
//! by whoever built the node. It says the repository's own `.warlockignore`
//! excludes that directory — content Warlock is not about. It is deliberately
//! *not* a [`NodeState`] and not a colour: gray already means "outside
//! Warlock's management", an excluded directory is gray like any other unpacted
//! one, and there is no fourth colour. The flag exists so that a front end can
//! refuse to pact such a directory without asking the filesystem, since every
//! other fact about a node — its state, its document, its files — comes from
//! the load and this one should too.
//!
//! [`Node::scope`] is a third such fact: the boundary written on *this*
//! directory's own manifest entry, and never one inherited from an ancestor. A
//! renderer draws the directory that owns a scope, so the node has to know
//! whether the scope is its own; the question "which scope covers this path"
//! is a different one, and [`scope_covering`](crate::scope_covering) answers
//! it by walking upwards. Storing the inherited answer here would make the two
//! indistinguishable and put one boundary's name on every row below it.

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
    /// Whether the repository's own `.warlockignore` rules exclude this
    /// directory.
    ///
    /// A fact about the node, not a state and not a colour: an excluded
    /// directory is [`NodeState::Unpacted`] and draws gray like any other
    /// directory nobody has pacted. What it adds is *why* — this one cannot be
    /// pacted at all, because the repository said Warlock is not about it — so
    /// a front end can say so instead of starting a run that would produce
    /// nothing.
    ///
    /// Stored rather than derived, like [`state`](Node::state): asking the
    /// filesystem is the loader's job, and a tree is just values by the time
    /// anyone renders it. Defaulted on deserialisation, so a tree written
    /// before this field existed still reads back.
    #[serde(default)]
    pub ignored: bool,
    /// The scope written on this directory's own pact entry, or `None` where it
    /// has none — which includes every directory nobody pacted, since a scope
    /// is stored on an entry and a directory with no entry has nowhere to hold
    /// one.
    ///
    /// Its own and never an inherited one. A renderer draws the directory that
    /// carries a scope, so a row has to be able to say "this boundary starts
    /// here"; the different question — which scope covers a given path — is
    /// [`scope_covering`](crate::scope_covering)'s, and it walks upwards to
    /// answer it. Filling this in with an ancestor's scope would answer the
    /// second question badly in the place the first one is asked.
    ///
    /// A scope the validator refuses reads as `None`: it is reported as a
    /// problem by the loader and left on disk exactly as somebody wrote it, but
    /// for as long as it is not a scope the directory is unscoped. Stored
    /// rather than derived, like [`state`](Node::state), and defaulted on
    /// deserialisation so a tree written before this field existed still reads
    /// back.
    #[serde(default)]
    pub scope: Option<String>,
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
    /// The node is not [`ignored`](Node::ignored): a directory is content
    /// Warlock covers unless a rule says otherwise, and only a loader that
    /// consulted the rules can say otherwise — see [`Node::with_ignored`]. It
    /// carries no [`scope`](Node::scope) either, for the same reason: a scope
    /// lives on a manifest entry, and only a loader that read the manifest can
    /// put one here — see [`Node::with_scope`].
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
            ignored: false,
            scope: None,
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

    /// The same node marked — or unmarked — as excluded by the repository's
    /// `.warlockignore`, the companion to [`with_children`](Node::with_children)
    /// and [`with_files`](Node::with_files) for the fact only a loader can
    /// know.
    ///
    /// Nothing else about the node moves: it keeps its state, its document and
    /// its children, because being excluded is not a state and an excluded
    /// directory is a row in the tree like any other. Hiding it is not on
    /// offer — the reader is meant to see what the repository excluded.
    ///
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

    /// Whether the repository's `.warlockignore` excludes this directory.
    ///
    /// Reads [`ignored`](Node::ignored) and touches no filesystem, so a caller
    /// that must stay pure — an event loop deciding whether a keypress can pact
    /// this row — can ask it.
    #[must_use]
    pub fn is_ignored(&self) -> bool {
        self.ignored
    }

    /// The same node carrying — or no longer carrying — the scope written on
    /// its own manifest entry, the companion to
    /// [`with_ignored`](Node::with_ignored) for the other fact only a loader
    /// that read the manifest can know.
    ///
    /// This directory's scope and never an ancestor's: see
    /// [`scope`](Node::scope). Nothing else about the node moves — a scope is a
    /// label on a row, not a state, not a colour and not a gate — and `None`
    /// clears it rather than leaving what was there, so the setter sets rather
    /// than latches.
    ///
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
