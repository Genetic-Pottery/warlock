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

use std::path::{Path, PathBuf};

use warlock_engine::{IntoReadme, NodeState, StateCounts, Tree, to_manifest_path};

/// What the header says when the tree is rooted at the repository root itself.
///
/// The relative path of the root against itself is `"."`, which reads as
/// nothing much on a header line, and an empty line reads as a bug. The
/// brackets keep it from being mistaken for a directory of that name.
const REPOSITORY_ROOT_LABEL: &str = "(repository root)";

/// One line of the flattened tree: what to draw, how far to indent it, and
/// which colour it takes.
///
/// A row owns its path rather than borrowing from the tree, so [`App`] is a
/// self-contained value that can be built, moved and asserted on without
/// lifetimes threading through the event loop.
///
/// The README comes along for the same reason: pacting a node needs one, and
/// the row is what the key handler has in its hand when the key is pressed.
/// Fetching it back out of the tree at that moment would mean keeping the tree
/// alongside the rows and looking a path up in it, which is two sources for one
/// fact.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Row {
    /// How deep the node sits: `0` for the root, `1` for its children.
    pub depth: usize,
    /// The path of the node this row stands for, exactly as the engine stores
    /// it.
    pub path: PathBuf,
    /// The README documenting the node, straight from [`warlock_engine::Node`],
    /// or `None` for a connector directory that has none of its own.
    pub readme: Option<PathBuf>,
    /// What Warlock knows about the node, which is what colours the row.
    pub state: NodeState,
}

impl Row {
    /// A row for a node at `path`, documented by `readme`, sitting at `depth`,
    /// in `state`.
    ///
    /// `readme` takes whatever [`warlock_engine::Node::new`] takes: anything
    /// path-like for a node that has one, or `None` for a connector that does
    /// not.
    #[must_use]
    pub fn new(
        depth: usize,
        path: impl Into<PathBuf>,
        readme: impl IntoReadme,
        state: NodeState,
    ) -> Self {
        Self {
            depth,
            path: path.into(),
            readme: readme.into_readme(),
            state,
        }
    }
}

/// What a pact toggle changed, for the caller that has to write it down.
///
/// [`App::toggle_pact`] flips the state on screen; the manifest is somebody
/// else's file, so this says what happened and lets that somebody act on it.
/// A node with no README cannot be pacted at all, so `readme` is not optional
/// here — a toggle that produced one of these is a toggle of a real module.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PactToggle {
    /// The directory whose pact was toggled.
    pub path: PathBuf,
    /// The README documenting it, which a manifest entry needs.
    pub readme: PathBuf,
    /// Whether the node is pacted *now*, after the toggle: `true` means an
    /// entry should be written for it, `false` that its entry should go.
    pub pacted: bool,
}

/// The front end's state: the flattened tree, the selected row, the tally the
/// footer shows and the header line naming what is being shown.
///
/// `selected` is kept in range by construction and by every method that moves
/// it, so [`App::selected_row`] is `None` only when there are no rows at all.
///
/// The tally is the engine's own [`StateCounts`], carried along rather than
/// recomputed: counting states is the engine's job, and a renderer that adds
/// up its rows itself is a second implementation of that job waiting to
/// disagree with the first.
///
/// The header is carried as finished text for the same reason: the renderer
/// draws app state and nothing else, so it never has to know what a repository
/// root is or ask the engine where this tree came from.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct App {
    rows: Vec<Row>,
    selected: usize,
    counts: StateCounts,
    header: String,
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
                .map(|(node, depth)| {
                    Row::new(depth, node.path.clone(), node.readme.clone(), node.state)
                })
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
    ///
    /// The header starts empty: an app nobody has told where its tree came
    /// from has nothing honest to put there. See [`App::with_scope`].
    #[must_use]
    pub fn from_rows(rows: Vec<Row>) -> Self {
        Self {
            rows,
            selected: 0,
            counts: StateCounts::default(),
            header: String::new(),
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

    /// The same app state, with a header naming the tree rooted at `root`
    /// inside the repository at `repo_root`.
    ///
    /// The text is `root` relative to `repo_root` in forward slashes — the
    /// engine's own manifest spelling, so the header and the manifest name a
    /// module the same way on every platform — except that `repo_root` itself
    /// is named `(repository root)` rather than left as the bare `"."` that
    /// relative spelling would give.
    ///
    /// The caller resolving the pair is where the filesystem is touched; this
    /// only formats what it was handed, which keeps the header a pure function
    /// of app state all the way down to the renderer. A `root` that does not
    /// sit inside `repo_root`, or that is not UTF-8, cannot be described
    /// relatively at all and falls back to `root` printed lossily: a header is
    /// a label, and failing to draw one is no reason to fail to draw the tree.
    #[must_use]
    pub fn with_scope(mut self, repo_root: impl AsRef<Path>, root: impl AsRef<Path>) -> Self {
        let root = root.as_ref();
        self.header = match to_manifest_path(repo_root, root) {
            Ok(relative) if relative == "." => REPOSITORY_ROOT_LABEL.to_owned(),
            Ok(relative) => relative,
            Err(_) => root.display().to_string(),
        };
        self
    }

    /// The header line: what tree is on screen, as [`App::with_scope`] worded
    /// it, or empty for an app that was never told.
    #[must_use]
    pub fn header(&self) -> &str {
        &self.header
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

    /// Bring the selected node under Warlock's management, or take it back out
    /// again, and say what changed.
    ///
    /// An unpacted node becomes [`NodeState::PactedStale`], never fresh: a pact
    /// with no granted hash was never judged, and unjudged *is* stale. A pacted
    /// node, stale or fresh, becomes [`NodeState::Unpacted`] — dropping a pact
    /// drops whatever was granted with it.
    ///
    /// The tally moves with the row, one node out of the old state's field and
    /// into the new one, so [`App::counts`] keeps describing [`App::rows`] and
    /// [`StateCounts::total`] does not budge. Nothing recounts the rows: the
    /// counts are the engine's numbers, kept current rather than re-derived.
    ///
    /// Returns what the caller needs to edit the manifest with, or `None` when
    /// nothing was toggled — an app with no rows, or a selected node with no
    /// README. A directory without a README is not a module (it is a connector,
    /// kept only because modules sit below it) and a manifest entry has nowhere
    /// to point without one, so such a node is refused outright: no state
    /// changes, no count changes, nothing to write.
    ///
    /// Writing the manifest is the caller's job. This is app state and touches
    /// no file.
    pub fn toggle_pact(&mut self) -> Option<PactToggle> {
        let row = self.rows.get_mut(self.selected)?;
        let readme = row.readme.clone()?;

        let was = row.state;
        let now = match was {
            NodeState::Unpacted => NodeState::PactedStale,
            NodeState::PactedStale | NodeState::PactedFresh => NodeState::Unpacted,
        };
        row.state = now;
        let path = row.path.clone();

        // Both halves of the move happen together or neither does. An app told
        // rows but never told a tally (see `App::from_rows`) holds zeroes that
        // never described those rows, and nudging one field up while the other
        // cannot come down would turn a tally that is merely absent into one
        // that counts a node that is not there.
        let old = count_mut(&mut self.counts, was);
        if let Some(fewer) = old.checked_sub(1) {
            *old = fewer;
            *count_mut(&mut self.counts, now) += 1;
        }

        Some(PactToggle {
            path,
            readme,
            pacted: now.is_pacted(),
        })
    }
}

/// The field of `counts` holding the tally for `state`.
///
/// [`StateCounts`]' fields are public but its own accessor for this is not, so
/// the match lives here instead. Written as a match on the enum rather than a
/// lookup, so a fourth state would fail to compile rather than quietly go
/// uncounted.
fn count_mut(counts: &mut StateCounts, state: NodeState) -> &mut usize {
    match state {
        NodeState::Unpacted => &mut counts.unpacted,
        NodeState::PactedStale => &mut counts.pacted_stale,
        NodeState::PactedFresh => &mut counts.pacted_fresh,
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use warlock_engine::{Node, NodeState, StateCounts, Tree};

    use super::{App, PactToggle, REPOSITORY_ROOT_LABEL, Row};
    use crate::fixture;

    /// Three rows, one per state, standing in for a flattened tree without
    /// dragging a `Tree` into tests that are only about the selection.
    fn three_rows() -> Vec<Row> {
        vec![
            Row::new(0, "repo", "repo/README.md", NodeState::PactedStale),
            Row::new(
                1,
                "repo/crates",
                "repo/crates/README.md",
                NodeState::PactedFresh,
            ),
            Row::new(
                1,
                "repo/assets",
                "repo/assets/README.md",
                NodeState::Unpacted,
            ),
        ]
    }

    /// The tally the rows actually add up to, counted here rather than asked
    /// of the app: a test that recomputes is the only way to catch the app's
    /// carried counts drifting from the rows they claim to describe.
    fn tally(app: &App) -> StateCounts {
        let mut counts = StateCounts::default();
        for row in app.rows() {
            match row.state {
                NodeState::Unpacted => counts.unpacted += 1,
                NodeState::PactedStale => counts.pacted_stale += 1,
                NodeState::PactedFresh => counts.pacted_fresh += 1,
            }
        }
        counts
    }

    /// The app for the shared fixture, selecting the row for `path`.
    fn app_selecting(path: &str) -> App {
        let mut app = App::from_tree(&fixture::tree());
        while app.selected_row().expect("the fixture has rows").path != Path::new(path) {
            let before = app.selected();
            app.select_next();
            assert_ne!(app.selected(), before, "no row for {path}");
        }
        app
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
    fn a_scope_below_the_repository_root_is_named_relative_to_it() {
        let app = App::from_rows(three_rows()).with_scope(
            Path::new("/repo"),
            Path::new("/repo").join("crates").join("engine"),
        );

        // Forward slashes even where the separator is a backslash: this is the
        // engine's manifest spelling.
        assert_eq!(app.header(), "crates/engine");
    }

    #[test]
    fn the_repository_root_itself_is_named_rather_than_left_as_a_dot() {
        let app = App::from_rows(three_rows()).with_scope("/repo", "/repo");

        assert_eq!(app.header(), REPOSITORY_ROOT_LABEL);
        assert_ne!(app.header(), ".");
        assert!(!app.header().is_empty());
    }

    #[test]
    fn a_relative_scope_is_taken_to_be_relative_to_the_repository_root_already() {
        let app = App::from_rows(three_rows()).with_scope("/repo", "docs/adr");

        assert_eq!(app.header(), "docs/adr");
    }

    #[test]
    fn a_root_outside_the_repository_falls_back_to_printing_itself() {
        let app = App::from_rows(three_rows()).with_scope("/repo", "/elsewhere/docs");

        // Not describable relative to the repository root, but a header is a
        // label: it says what it can rather than going blank.
        assert_eq!(app.header(), "/elsewhere/docs");
    }

    #[test]
    fn an_app_that_was_never_told_its_scope_has_no_header() {
        assert_eq!(App::from_rows(three_rows()).header(), "");
        assert_eq!(App::from_tree(&fixture::tree()).header(), "");
    }

    #[test]
    fn a_scope_changes_nothing_but_the_header() {
        let app = App::from_tree(&fixture::tree());

        let scoped = app.clone().with_scope("/repo", "/repo/crates");

        assert_eq!(scoped.rows(), app.rows());
        assert_eq!(scoped.counts(), app.counts());
        assert_eq!(scoped.selected(), app.selected());
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

    #[test]
    fn flattening_a_tree_carries_each_nodes_readme() {
        let tree = fixture::tree();

        let app = App::from_tree(&tree);

        for row in app.rows() {
            let node = tree.find(&row.path).expect("row came from the tree");
            assert_eq!(row.readme, node.readme, "readme for {}", row.path.display());
        }
        // Including the connector, whose README is honestly absent.
        assert!(
            app.rows()
                .iter()
                .any(|row| row.readme.is_none() && row.path == Path::new("warlock/crates"))
        );
    }

    #[test]
    fn pacting_an_unpacted_node_makes_it_stale_and_pacting_again_undoes_it() {
        let mut app = app_selecting("warlock/assets");

        let pacted = app.toggle_pact().expect("assets has a README");

        assert_eq!(
            pacted,
            PactToggle {
                path: PathBuf::from("warlock/assets"),
                readme: PathBuf::from("warlock/assets/README.md"),
                pacted: true,
            }
        );
        // Never fresh: a pact with no granted hash was never judged.
        assert_eq!(
            app.selected_row().map(|row| row.state),
            Some(NodeState::PactedStale)
        );

        let unpacted = app.toggle_pact().expect("assets still has a README");

        assert_eq!(
            unpacted,
            PactToggle {
                path: PathBuf::from("warlock/assets"),
                readme: PathBuf::from("warlock/assets/README.md"),
                pacted: false,
            }
        );
        assert_eq!(
            app.selected_row().map(|row| row.state),
            Some(NodeState::Unpacted)
        );
        assert_eq!(app.rows(), App::from_tree(&fixture::tree()).rows());
    }

    #[test]
    fn unpacting_a_fresh_node_drops_it_all_the_way_out() {
        let mut app = app_selecting("warlock/crates/engine");

        let toggled = app.toggle_pact().expect("engine has a README");

        // Fresh goes straight to unpacted: the grant goes with the pact.
        assert!(!toggled.pacted);
        assert_eq!(
            app.selected_row().map(|row| row.state),
            Some(NodeState::Unpacted)
        );
    }

    #[test]
    fn the_counts_move_with_the_rows_and_the_total_never_changes() {
        let mut app = App::from_tree(&fixture::tree());
        let total = app.counts().total();

        assert_eq!(app.counts(), tally(&app));
        // Every row in turn, twice over, so each state is both entered and left.
        for _ in 0..2 {
            for index in 0..app.rows().len() {
                while app.selected() != index {
                    app.select_next();
                }
                app.toggle_pact();
                assert_eq!(app.counts(), tally(&app), "after toggling row {index}");
                assert_eq!(app.counts().total(), total);
            }
            for _ in 0..app.rows().len() {
                app.select_previous();
            }
        }
    }

    #[test]
    fn a_connector_with_no_readme_cannot_be_pacted() {
        let mut app = app_selecting("warlock/crates");
        let before = app.clone();

        assert_eq!(app.toggle_pact(), None);

        assert_eq!(app, before);
        assert_eq!(
            app.selected_row().map(|row| row.state),
            Some(NodeState::Unpacted)
        );
        assert_eq!(app.counts(), tally(&app));
    }

    #[test]
    fn a_root_with_no_readme_cannot_be_pacted_either() {
        let mut app = App::from_rows(vec![Row::new(0, "repo", None, NodeState::Unpacted)])
            .with_counts(StateCounts {
                unpacted: 1,
                ..StateCounts::default()
            });
        let before = app.clone();

        assert_eq!(app.toggle_pact(), None);

        assert_eq!(app, before);
    }

    #[test]
    fn toggling_a_pact_on_an_empty_app_is_a_no_op() {
        let mut app = App::from_rows(Vec::new());

        assert_eq!(app.toggle_pact(), None);

        assert!(app.is_empty());
        assert_eq!(app.counts(), StateCounts::default());
    }
}
