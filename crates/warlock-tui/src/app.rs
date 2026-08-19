//! What the front end holds between keystrokes.
//!
//! The tree is a tree, but the screen is a list of lines, so the app state
//! keeps the engine's depth-first walk already flattened into [`Row`]s and
//! remembers which one is selected. Flattening once, up front, means the
//! renderer and the key handler agree on what "the next row" is without either
//! of them walking the tree again.
//!
//! Nothing here touches a terminal: it is a plain data structure with plain
//! methods, so every rule about how the selection moves is testable with
//! nothing attached to stdout.

use std::path::PathBuf;

use warlock_engine::{NodeState, StateCounts, Tree};

/// One line of the flattened tree: what to draw, how far to indent it, and
/// which colour it takes.
///
/// A row owns its path rather than borrowing from the tree, so [`App`] is a
/// self-contained value that can be built, moved and asserted on without
/// lifetimes threading through the event loop.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Row {
    /// How deep the node sits: `0` for the root, `1` for its children.
    pub depth: usize,
    /// The path of the node this row stands for, exactly as the engine stores
    /// it.
    pub path: PathBuf,
    /// What Warlock knows about the node, which is what colours the row.
    pub state: NodeState,
}

impl Row {
    /// A row for a node at `path`, sitting at `depth`, in `state`.
    #[must_use]
    pub fn new(depth: usize, path: impl Into<PathBuf>, state: NodeState) -> Self {
        Self {
            depth,
            path: path.into(),
            state,
        }
    }
}

/// The front end's state: the flattened tree, the selected row and the tally
/// the footer shows.
///
/// `selected` is kept in range by construction and by every method that moves
/// it, so [`App::selected_row`] is `None` only when there are no rows at all.
///
/// The tally is the engine's own [`StateCounts`], carried along rather than
/// recomputed: counting states is the engine's job, and a renderer that adds
/// up its rows itself is a second implementation of that job waiting to
/// disagree with the first.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct App {
    rows: Vec<Row>,
    selected: usize,
    counts: StateCounts,
}

impl App {
    /// The app state for `tree`, with the first row selected.
    ///
    /// This is the only place the tree's shape is read. The front end gets its
    /// tree by calling the engine's constructor and hands it straight here; it
    /// never learns where the tree came from.
    #[must_use]
    pub fn from_tree(tree: &Tree) -> Self {
        Self::from_rows(
            tree.walk()
                .map(|(node, depth)| Row::new(depth, node.path.clone(), node.state))
                .collect(),
        )
        .with_counts(tree.counts())
    }

    /// The app state for an already-flattened list of rows, with the first row
    /// selected and an all-zero tally.
    ///
    /// A [`Tree`] always has a root and so is never empty; this constructor is
    /// how the no-rows case is reachable at all, in tests and in any future
    /// caller that filters the tree down to nothing. Zero counts are the
    /// truth for that empty case; any caller passing rows should say what they
    /// tally to with [`App::with_counts`].
    #[must_use]
    pub fn from_rows(rows: Vec<Row>) -> Self {
        Self {
            rows,
            selected: 0,
            counts: StateCounts::default(),
        }
    }

    /// The same app state, reporting `counts` in its footer.
    ///
    /// Takes the engine's tally as a value instead of deriving one, so the
    /// numbers on screen are the engine's numbers.
    #[must_use]
    pub const fn with_counts(mut self, counts: StateCounts) -> Self {
        self.counts = counts;
        self
    }

    /// Every row, in the order they are drawn.
    #[must_use]
    pub fn rows(&self) -> &[Row] {
        &self.rows
    }

    /// How many nodes sit in each state, as the engine counted them.
    #[must_use]
    pub const fn counts(&self) -> StateCounts {
        self.counts
    }

    /// Whether there is nothing to draw and nothing to select.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// Where the selection sits in [`App::rows`].
    ///
    /// Meaningless when there are no rows, where it stays `0`; ask
    /// [`App::selected_row`] instead if you need the row itself.
    #[must_use]
    pub const fn selected(&self) -> usize {
        self.selected
    }

    /// The selected row, or `None` when there are no rows.
    #[must_use]
    pub fn selected_row(&self) -> Option<&Row> {
        self.rows.get(self.selected)
    }

    /// Move the selection one row up, stopping at the first row.
    ///
    /// It clamps rather than wrapping: an unnoticed wrap at the top of a long
    /// tree throws the reader to the bottom of it, and the arrow key is for
    /// stepping, not teleporting. A no-op when there are no rows.
    pub fn select_previous(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    /// Move the selection one row down, stopping at the last row.
    ///
    /// Clamps for the same reason [`App::select_previous`] does. A no-op when
    /// there are no rows.
    pub fn select_next(&mut self) {
        let last = self.rows.len().saturating_sub(1);
        self.selected = self.selected.saturating_add(1).min(last);
    }
}

#[cfg(test)]
mod tests {
    use warlock_engine::{Node, NodeState, StateCounts, Tree};

    use super::{App, Row};
    use crate::fixture;

    /// Three rows, one per state, standing in for a flattened tree without
    /// dragging a `Tree` into tests that are only about the selection.
    fn three_rows() -> Vec<Row> {
        vec![
            Row::new(0, "repo", NodeState::PactedStale),
            Row::new(1, "repo/crates", NodeState::PactedFresh),
            Row::new(1, "repo/assets", NodeState::Unpacted),
        ]
    }

    #[test]
    fn flattening_a_tree_keeps_depth_first_order_and_depth() {
        let tree = Tree::new(
            Node::new("repo", "repo/README.md", NodeState::PactedStale).with_children([
                Node::new(
                    "repo/crates",
                    "repo/crates/README.md",
                    NodeState::PactedFresh,
                )
                .with_children([Node::new(
                    "repo/crates/engine",
                    "repo/crates/engine/README.md",
                    NodeState::PactedFresh,
                )]),
                Node::new("repo/assets", "repo/assets/README.md", NodeState::Unpacted),
            ]),
        );

        let app = App::from_tree(&tree);

        let seen: Vec<(usize, &str)> = app
            .rows()
            .iter()
            .map(|row| (row.depth, row.path.to_str().expect("ascii path")))
            .collect();
        assert_eq!(
            seen,
            [
                (0, "repo"),
                (1, "repo/crates"),
                (2, "repo/crates/engine"),
                (1, "repo/assets"),
            ]
        );
    }

    #[test]
    fn flattening_a_tree_keeps_every_node_and_its_state() {
        let tree = fixture::tree();

        let app = App::from_tree(&tree);

        assert_eq!(app.rows().len(), tree.counts().total());
        for row in app.rows() {
            let node = tree.find(&row.path).expect("row came from the tree");
            assert_eq!(row.state, node.state);
        }
    }

    #[test]
    fn an_app_carries_the_trees_own_counts() {
        let tree = fixture::tree();

        let app = App::from_tree(&tree);

        assert_eq!(app.counts(), tree.counts());
        assert_eq!(app.counts().total(), app.rows().len());
    }

    #[test]
    fn an_app_built_from_bare_rows_counts_nothing_until_told() {
        let counts = fixture::tree().counts();

        let app = App::from_rows(three_rows());

        assert_eq!(app.counts(), StateCounts::default());
        assert_eq!(app.with_counts(counts).counts(), counts);
    }

    #[test]
    fn a_fresh_app_selects_the_first_row() {
        let app = App::from_rows(three_rows());

        assert_eq!(app.selected(), 0);
        assert_eq!(app.selected_row(), Some(&three_rows()[0]));
    }

    #[test]
    fn selecting_down_then_up_moves_one_row_at_a_time() {
        let mut app = App::from_rows(three_rows());

        app.select_next();
        assert_eq!(app.selected(), 1);
        app.select_next();
        assert_eq!(app.selected(), 2);
        app.select_previous();
        assert_eq!(app.selected(), 1);
        assert_eq!(app.selected_row(), Some(&three_rows()[1]));
    }

    #[test]
    fn selecting_up_at_the_first_row_stays_put_and_does_not_wrap() {
        let mut app = App::from_rows(three_rows());

        app.select_previous();
        app.select_previous();

        assert_eq!(app.selected(), 0);
    }

    #[test]
    fn selecting_down_at_the_last_row_stays_put_and_does_not_wrap() {
        let mut app = App::from_rows(three_rows());
        let last = three_rows().len() - 1;

        for _ in 0..three_rows().len() + 2 {
            app.select_next();
        }

        assert_eq!(app.selected(), last);
    }

    #[test]
    fn moving_the_selection_on_an_empty_app_is_a_no_op() {
        let mut app = App::from_rows(Vec::new());

        app.select_next();
        app.select_previous();

        assert!(app.is_empty());
        assert_eq!(app.selected(), 0);
        assert_eq!(app.selected_row(), None);
    }
}
