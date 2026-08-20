//! What the front end holds between keystrokes.
//!
//! The tree is a tree, but the screen is a list of lines, so the app state
//! keeps the engine's depth-first walk flattened into [`Row`]s and remembers
//! which one is selected. Flattening here, rather than in the renderer, means
//! the renderer and the key handler agree on what "the next row" is without
//! either of them walking the tree again.
//!
//! The whole walk is kept, though, not only the part on screen: collapsing a
//! directory re-runs that flattening with the directory's descendants filtered
//! out, and expanding it re-runs the same flattening with them back, at the
//! depths and in the order the engine gave them. Which directories are
//! collapsed is remembered as node *paths* rather than row indices, because an
//! index means nothing once the row list has been rebuilt — and rebuilding it
//! is exactly what a reloaded tree does.
//!
//! Narrowing the view to what Warlock manages is the same kind of thing, and
//! goes through the same re-flattening: a pacted-only flag that drops every row
//! that is neither pacted nor on the way to something pacted. It is a filter
//! over the walk, not a rule about what the tree contains — the tree keeps every
//! node it was loaded with, the flag says which of them are worth drawing right
//! now, and turning it off puts the rest back untouched.
//!
//! A tree taller than the terminal does not fit, so the app also remembers
//! which slice of those rows is on screen: a scroll offset, kept in step with
//! the selection by every method that moves it, and computed by one pure
//! function (`scroll_offset_for`) that the tests can drive directly.
//!
//! It also holds the one line it has to say about the last keystroke — why a
//! pact was refused, say. That wording is state like everything else here, set
//! by whatever refused and dropped by the next movement, so the renderer draws
//! it without knowing what happened and the key handler never has to explain
//! itself.
//!
//! Nothing here touches a terminal: it is a plain data structure with plain
//! methods, so every rule about how the selection moves is testable with
//! nothing attached to stdout.

use std::borrow::Cow;
use std::collections::BTreeSet;
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
///
/// The child count comes along for a third reason of the same shape: a
/// directory with no children can be neither collapsed nor expanded, and a row
/// that could not say so would send the renderer back to the tree to find out
/// whether to draw a marker.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Row {
    /// How deep the node sits: `0` for the root, `1` for its children.
    pub depth: usize,
    /// The path of the node this row stands for, exactly as the engine stores
    /// it.
    pub path: PathBuf,
    /// The README documenting the node, straight from [`warlock_engine::Node`],
    /// or `None` for an ordinary directory that has no documentation yet.
    pub readme: Option<PathBuf>,
    /// What Warlock knows about the node, which is what colours the row.
    pub state: NodeState,
    /// How many children the node has in the tree — not how many are drawn,
    /// which is none of them while the node is collapsed. `0` for a leaf, and
    /// a leaf is what [`App::toggle_collapsed`] refuses to toggle.
    pub children: usize,
}

impl Row {
    /// A row for a childless node at `path`, documented by `readme`, sitting at
    /// `depth`, in `state`.
    ///
    /// `readme` takes whatever [`warlock_engine::Node::new`] takes: anything
    /// path-like for a node that has one, or `None` for a directory that has
    /// no documentation yet.
    ///
    /// Childless is the safe default rather than the common case: a row that
    /// claims children it does not have is a row the collapse key hides
    /// nothing with. Say otherwise with [`Row::with_child_count`].
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
            children: 0,
        }
    }

    /// The same row, standing for a node with `children` children of its own.
    #[must_use]
    pub const fn with_child_count(mut self, children: usize) -> Self {
        self.children = children;
        self
    }

    /// Whether the node has children, and so whether collapsing it would hide
    /// anything.
    #[must_use]
    pub const fn has_children(&self) -> bool {
        self.children > 0
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

/// The front end's state: the flattened tree, which of it is collapsed, the
/// selected row, the slice of rows on screen, the tally the footer shows and
/// the header line naming what is being shown.
///
/// `all_rows` is the engine's whole walk as it was when the app was built, and
/// nothing but a reload changes it; `rows` is what is actually drawn, rebuilt
/// from `all_rows` every time the collapsed set changes, with the descendants
/// of every collapsed node filtered out. Keeping both means collapsing is
/// reversible without a second walk of a tree the app no longer holds, and that
/// a row hidden under a collapsed parent keeps its depth and its place in the
/// order for when the parent opens again.
///
/// `collapsed` holds node paths, never row indices: an index names a different
/// node the moment the row list changes length, and the row list is rebuilt
/// both by every collapse and by every reload of the tree.
///
/// `pacted_only` is the second input to that rebuild: with it set, only the
/// pacted nodes and the ancestors leading to them are drawn. It is view state
/// and nothing else — no node is dropped from `all_rows`, no state is changed,
/// nothing is written anywhere — so [`App::toggle_pacted_only`] is reversible in
/// the strongest sense: the rows come back exactly as they were, at the same
/// depths, in the same order, still collapsed wherever they were collapsed.
///
/// `selected` is kept in range by construction and by every method that moves
/// it, so [`App::selected_row`] is `None` only when there are no rows at all.
/// It is kept *meaningful* by `reflow`, which puts it back on the node it was
/// on after the row list changes, or on that node's nearest drawn ancestor when
/// collapsing has taken it off screen.
///
/// `scroll_offset` is kept in step with `selected` by those same methods, so
/// the selected row is always inside the window the renderer draws — see
/// [`App::scroll_offset`]. It is derived state, never moved on its own: there
/// is no "scroll without moving the selection" here, because a selection that
/// has scrolled off screen is a selection the next keystroke moves invisibly.
///
/// The tally is the engine's own [`StateCounts`], carried along rather than
/// recomputed: counting states is the engine's job, and a renderer that adds
/// up its rows itself is a second implementation of that job waiting to
/// disagree with the first.
///
/// The header is carried as finished text for the same reason: the renderer
/// draws app state and nothing else, so it never has to know what a repository
/// root is or ask the engine where this tree came from.
///
/// The message is the one line the app has to say about the keystroke just
/// pressed — why a pact was refused, or whatever the caller put there. It is
/// finished text for the same reason the header is, and it lives here rather
/// than in the caller's hand because it is display state like everything else
/// around it: the renderer draws whatever is in it, and every method that moves
/// the selection empties it, so a message lasts exactly until the next
/// keystroke.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct App {
    all_rows: Vec<Row>,
    rows: Vec<Row>,
    collapsed: BTreeSet<PathBuf>,
    pacted_only: bool,
    selected: usize,
    scroll_offset: usize,
    viewport_height: usize,
    counts: StateCounts,
    header: String,
    message: Option<String>,
}

impl App {
    /// The app state for `tree`, with everything expanded and the first row
    /// selected.
    ///
    /// This is the only place the tree's shape is read: the whole walk is
    /// flattened here, and every later question about the tree's shape — which
    /// nodes hang under a collapsed one, whether a node has children to hide at
    /// all — is answered from the rows this produces rather than from a tree
    /// the app would otherwise have to keep. The front end gets its tree by
    /// calling the engine's constructor and hands it straight here; it never
    /// learns where the tree came from.
    ///
    /// Nothing starts collapsed, which is what makes a freshly launched app
    /// draw every node the walk yields. Carrying a previous run's collapsed
    /// directories onto a reloaded tree is [`App::with_collapsed`]'s job.
    #[must_use]
    pub fn from_tree(tree: &Tree) -> Self {
        Self::from_rows(
            tree.walk()
                .map(|(node, depth)| {
                    Row::new(depth, node.path.clone(), node.readme.clone(), node.state)
                        .with_child_count(node.children.len())
                })
                .collect(),
        )
        .with_counts(tree.counts())
    }

    /// The app state for an already-flattened list of rows, with everything
    /// expanded, the first row selected and an all-zero tally.
    ///
    /// A [`Tree`] always has a root and so is never empty; this constructor is
    /// how the no-rows case is reachable at all, in tests and in any future
    /// caller that filters the tree down to nothing. Zero counts are the
    /// truth for that empty case; any caller passing rows should say what they
    /// tally to with [`App::with_counts`].
    ///
    /// The header starts empty: an app nobody has told where its tree came
    /// from has nothing honest to put there. See [`App::with_scope`].
    ///
    /// The viewport starts at zero rows tall and the window at the top, which
    /// is what is true of an app that has never been drawn: nothing is on
    /// screen yet, so nothing has been scrolled past. See
    /// [`App::set_viewport_height`].
    ///
    /// There is no message either: an app that has answered no keystroke yet
    /// has nothing to say about one. See [`App::message`].
    ///
    /// Nothing is collapsed and the pacted-only filter is off, so the rows
    /// handed over are exactly the rows drawn. They are kept a second time as the unfiltered list a later
    /// collapse re-filters and a later expand restores from; a caller that
    /// wants collapsing to hide anything has to say which rows have children,
    /// with [`Row::with_child_count`], because a bare list of rows is the one
    /// input here that does not come from a tree.
    #[must_use]
    pub fn from_rows(rows: Vec<Row>) -> Self {
        Self {
            rows: rows.clone(),
            all_rows: rows,
            collapsed: BTreeSet::new(),
            pacted_only: false,
            selected: 0,
            scroll_offset: 0,
            viewport_height: 0,
            counts: StateCounts::default(),
            header: String::new(),
            message: None,
        }
    }

    /// The same app state, with every node in `collapsed` collapsed and the
    /// rows re-filtered to match.
    ///
    /// This is how a collapsed tree survives a reload. The app state is thrown
    /// away and rebuilt from the new tree whenever the tree is re-read, so
    /// something has to carry the view across the gap; paths carry, which is
    /// the whole reason [`App::collapsed`] hands back paths rather than the row
    /// indices they were pressed on. A path the new tree no longer has is kept
    /// and ignored — a directory that has come and gone and come back should
    /// find itself as the user left it, and the set is small enough that
    /// pruning it would cost more than carrying it.
    ///
    /// The rows are filtered on the way in rather than at the next keystroke,
    /// so the first frame drawn after a reload is already the collapsed one.
    #[must_use]
    pub fn with_collapsed(
        mut self,
        collapsed: impl IntoIterator<Item = impl Into<PathBuf>>,
    ) -> Self {
        self.collapsed = collapsed.into_iter().map(Into::into).collect();
        self.reflow();
        self
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

    /// What the app has to say about the last keystroke, or `None` when it has
    /// nothing to say.
    ///
    /// Set by whatever refused to do something — [`App::toggle_pact`] on a
    /// directory with no README — or by the caller through
    /// [`App::set_message`], and emptied by the next movement, so what is here
    /// always belongs to the keystroke just pressed.
    #[must_use]
    pub fn message(&self) -> Option<&str> {
        self.message.as_deref()
    }

    /// Say `message` until the next keystroke moves the selection.
    ///
    /// For the caller's own sentences — a manifest that would not write is the
    /// caller's news, not the app's — so that there is one place a line reaches
    /// the screen from rather than two. Replaces whatever was there: only the
    /// latest keystroke has anything to report.
    pub fn set_message(&mut self, message: impl Into<String>) {
        self.message = Some(message.into());
    }

    /// Every row that is drawn, in the order it is drawn: the engine's walk
    /// with the descendants of every collapsed node left out.
    ///
    /// Depths are the tree's own, not the drawn list's, so a row whose parent
    /// is two levels of collapsed directory above it still indents to where it
    /// belongs when those levels open again.
    #[must_use]
    pub fn rows(&self) -> &[Row] {
        &self.rows
    }

    /// Which nodes are collapsed, by path, in a fixed order.
    ///
    /// For handing to [`App::with_collapsed`] on a rebuild, and for a test to
    /// assert on. It is what the user pressed the key on, which is not
    /// necessarily what is on screen: a node under a collapsed parent can be
    /// in here and drawn nowhere.
    #[must_use]
    pub const fn collapsed(&self) -> &BTreeSet<PathBuf> {
        &self.collapsed
    }

    /// Whether the node at `path` is collapsed.
    ///
    /// Answers for a node with no children too, where it means only that the
    /// key was pressed on it: there was nothing to hide, so nothing is hidden.
    /// A renderer deciding which marker to draw wants this *and*
    /// [`Row::has_children`] — collapsed, expanded-with-children and childless
    /// are three cases, and this answers one bit of them.
    #[must_use]
    pub fn is_collapsed(&self, path: impl AsRef<Path>) -> bool {
        self.collapsed.contains(path.as_ref())
    }

    /// Whether the view is narrowed to the pacted part of the tree.
    ///
    /// `false` is the whole walk, which is what a freshly built app shows. See
    /// [`App::toggle_pacted_only`] for what `true` leaves on screen.
    #[must_use]
    pub const fn pacted_only(&self) -> bool {
        self.pacted_only
    }

    /// Narrow the view to the pacted part of the tree, or widen it back to the
    /// whole of it.
    ///
    /// Narrowed, the drawn rows are every node whose state
    /// [`NodeState::is_pacted`] calls pacted, plus every ancestor needed to
    /// reach one, and nothing else: an unpacted directory earns its row only by
    /// having something pacted somewhere below it. That is a filter over the
    /// walk and not a change to it — no node leaves [`App::rows`]' source, no
    /// state moves, [`App::counts`] still describes the whole tree, and nothing
    /// about the flag reaches a file.
    ///
    /// It composes with collapsing rather than replacing it. Which rows survive
    /// the filter is decided from the whole walk, so a directory that is only on
    /// the way to something pacted survives even while it is collapsed over that
    /// something; the collapse then hides its descendants as it always did. A
    /// directory collapsed before the filter went on is still collapsed after it
    /// comes off again, because the filter never touched the collapsed set.
    ///
    /// The selection is carried by path, so widening the view again finds the
    /// node that was selected while it was narrow. Narrowing over the selection
    /// lands it on the nearest still-drawn ancestor — the same rule collapsing
    /// over the selection follows, and for the same reason: that ancestor is the
    /// row the hidden node went behind. A node with no drawn ancestor at all,
    /// which is any node once nothing in the tree is pacted, falls back to the
    /// first row.
    ///
    /// Clears the last keystroke's message, like every other key that does
    /// something.
    pub fn toggle_pacted_only(&mut self) {
        self.pacted_only = !self.pacted_only;
        self.message = None;
        self.reflow();
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

    /// Which row of [`App::rows`] is drawn at the top of the tree area: the
    /// window onto the tree starts here and runs for [`App::viewport_height`]
    /// rows.
    ///
    /// `0` for a tree that fits on screen, and for an app that has not been
    /// drawn yet. The selection is always inside the window, so a renderer
    /// that honours this offset draws the selected row in every frame.
    #[must_use]
    pub const fn scroll_offset(&self) -> usize {
        self.scroll_offset
    }

    /// How many rows of tree fit on screen, as last set by
    /// [`App::set_viewport_height`].
    ///
    /// `0` until something tells it otherwise, which is honest for an app
    /// nobody has drawn.
    #[must_use]
    pub const fn viewport_height(&self) -> usize {
        self.viewport_height
    }

    /// Tell the app how many rows of tree fit on screen, and bring the window
    /// back into line with the selection.
    ///
    /// The height is a field set from outside rather than an argument to each
    /// movement method, because the only place that knows it is the layout —
    /// it is the height of the tree area, which is the terminal's height less
    /// the header and the footer — and the movement methods are called from
    /// the key handler, which lays nothing out. Passing it in per call would
    /// make every caller fetch a number from the renderer first, and would let
    /// two call sites disagree about the size of one window; keeping it here
    /// means the app is asked once per frame and every rule reads the same
    /// value.
    ///
    /// Takes the `u16` a terminal rectangle measures itself in, so the caller
    /// hands over `area.height` without converting; everything downstream
    /// counts rows in `usize` alongside the selection index.
    ///
    /// Safe to call every frame, including when the height has not changed:
    /// it recomputes the offset from the same rule as any movement, so a
    /// terminal that has just been made shorter scrolls the selection back
    /// into view instead of leaving it below the fold.
    pub fn set_viewport_height(&mut self, height: u16) {
        self.viewport_height = usize::from(height);
        self.rescroll();
    }

    /// Move the selection one row up, stopping at the first row.
    ///
    /// It clamps rather than wrapping: an unnoticed wrap at the top of a long
    /// tree throws the reader to the bottom of it, and the arrow key is for
    /// stepping, not teleporting. A no-op when there are no rows.
    pub fn select_previous(&mut self) {
        self.selected = self.selected.saturating_sub(1);
        self.moved();
    }

    /// Move the selection one row down, stopping at the last row.
    ///
    /// Clamps for the same reason [`App::select_previous`] does. A no-op when
    /// there are no rows.
    pub fn select_next(&mut self) {
        let last = self.rows.len().saturating_sub(1);
        self.selected = self.selected.saturating_add(1).min(last);
        self.moved();
    }

    /// Move the selection one screenful up, stopping at the first row.
    ///
    /// A screenful is [`App::viewport_height`] rows, so the row that was at
    /// the top of the window is roughly the one at the bottom afterwards:
    /// paging by the window's own height is what makes reading a long tree a
    /// sequence of screens rather than a slide.
    ///
    /// An app that has never been drawn has no height to page by, and a key
    /// that does nothing at all reads as a broken key, so the step never falls
    /// below one row. A no-op when there are no rows.
    pub fn select_page_up(&mut self) {
        self.selected = self.selected.saturating_sub(self.page());
        self.moved();
    }

    /// Move the selection one screenful down, stopping at the last row.
    ///
    /// The mirror of [`App::select_page_up`], down to the one-row floor for an
    /// app that has not been drawn. A no-op when there are no rows.
    pub fn select_page_down(&mut self) {
        let last = self.rows.len().saturating_sub(1);
        self.selected = self.selected.saturating_add(self.page()).min(last);
        self.moved();
    }

    /// Select the first row, scrolling the window back to the top of the tree.
    ///
    /// A no-op when there are no rows.
    pub fn select_first(&mut self) {
        self.selected = 0;
        self.moved();
    }

    /// Select the last row, scrolling the window to the bottom of the tree.
    ///
    /// A no-op when there are no rows.
    pub fn select_last(&mut self) {
        self.selected = self.rows.len().saturating_sub(1);
        self.moved();
    }

    /// How many rows one page key moves by: a windowful, or a single row for
    /// an app whose window nobody has measured yet.
    const fn page(&self) -> usize {
        if self.viewport_height == 0 {
            1
        } else {
            self.viewport_height
        }
    }

    /// Bring the window back into line with wherever the selection now is.
    ///
    /// Every method that moves the selection ends here, which is what keeps
    /// the two fields from ever describing different screens. The rule itself
    /// lives in `scroll_offset_for`, where it is a pure function of four
    /// numbers and can be tested without an `App` at all.
    fn rescroll(&mut self) {
        self.scroll_offset = scroll_offset_for(
            self.rows.len(),
            self.viewport_height,
            self.selected,
            self.scroll_offset,
        );
    }

    /// Rebuild the drawn rows from the whole walk, the pacted-only flag and the
    /// collapsed set, and put the selection and the window back where they
    /// belong.
    ///
    /// Every change to what is drawn ends here, and it is the only place
    /// `rows` is written: the drawn list is derived from `all_rows`,
    /// `pacted_only` and `collapsed` and nothing else, so there is no state to
    /// get out of step with them.
    ///
    /// The two filters are applied in that order and never as one pass. What is
    /// pacted, and what is on the way to something pacted, is read off the whole
    /// walk, so a directory that is collapsed over the only pacted node below it
    /// still earns its row; deciding pactedness from the already-collapsed list
    /// would make collapsing a directory delete it from a narrowed view, which
    /// is the one thing collapsing must never do.
    ///
    /// The selection is carried by path rather than by index, because the index
    /// it sat at meant a row that may not exist any more. A selection whose
    /// node is still drawn stays exactly where it is, however many rows above
    /// it have gone; one whose node has just been hidden — collapsed over or
    /// filtered away, it makes no difference here — lands on the nearest
    /// ancestor still drawn, and on the first row when not even that survives.
    fn reflow(&mut self) {
        let selected = self.rows.get(self.selected).map(|row| row.path.clone());
        let kept: Cow<'_, [Row]> = if self.pacted_only {
            Cow::Owned(pacted_rows(&self.all_rows))
        } else {
            Cow::Borrowed(&self.all_rows)
        };
        self.rows = drawn_rows(&kept, &self.collapsed);
        self.selected = selected
            .and_then(|path| index_for(&self.rows, &path))
            .unwrap_or(0);
        self.rescroll();
    }

    /// The selection has just moved: forget last keystroke's message and bring
    /// the window back into line with it.
    ///
    /// Every movement method ends here rather than in `rescroll` alone, so no
    /// caller has to remember to clear a message by hand — a message belongs to
    /// the keystroke that produced it, and the next one has moved on. Clearing
    /// lives here and not in `rescroll` because `App::set_viewport_height`
    /// rescrolls on every frame, including the frame that is about to draw the
    /// message.
    fn moved(&mut self) {
        self.message = None;
        self.rescroll();
    }

    /// How to name `path` in a message: relative to the root of the tree on
    /// screen, in the engine's own forward-slash manifest spelling.
    ///
    /// A message naming a directory by its absolute path spends most of a
    /// footer line on the part of it the reader already knows, and truncation
    /// then eats the part they do not. The root row's path is the root of the
    /// tree by construction, so relative spelling is available without keeping
    /// a second copy of it. A path that cannot be described relative to that
    /// root — including the root itself, which is `"."` — is printed as it
    /// stands: a label that says something odd beats a label that says nothing.
    fn label_for(&self, path: &Path) -> String {
        match self
            .rows
            .first()
            .map(|root| to_manifest_path(&root.path, path))
        {
            Some(Ok(relative)) if relative != "." => relative,
            _ => path.display().to_string(),
        }
    }

    /// Hide the selected node's descendants, or bring them back.
    ///
    /// The rows come back exactly as they were — same nodes, same order, same
    /// depths — because they were never thrown away: expanding re-filters the
    /// walk this app was built from rather than reconstructing anything.
    ///
    /// A no-op, and deliberately a *complete* no-op, on a node with no
    /// children: nothing to hide means nothing collapses, nothing is recorded,
    /// and last keystroke's message is left on screen rather than swept away by
    /// a key that did nothing. A no-op on an app with no rows too.
    ///
    /// Collapsing a directory the selection sits inside moves the selection
    /// onto that directory — see `reflow` — because a selection on a row that
    /// is not drawn is a selection the next keystroke moves invisibly. The
    /// window follows, so the selected row is on screen when the frame is next
    /// drawn whichever way the row count went.
    ///
    /// Collapsing a node under a *collapsed* node is allowed and draws nothing:
    /// what is recorded is the state of that node, and it takes effect when the
    /// node is on screen to take effect on.
    pub fn toggle_collapsed(&mut self) {
        let Some(row) = self.rows.get(self.selected) else {
            return;
        };
        if !row.has_children() {
            return;
        }

        let path = row.path.clone();
        if !self.collapsed.remove(&path) {
            self.collapsed.insert(path);
        }
        self.message = None;
        self.reflow();
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
    /// README. A directory with no documentation yet is not a module, and a
    /// manifest entry has nowhere to point without a README, so such a node is
    /// refused outright: no state changes, no count changes, nothing to write.
    ///
    /// A refusal sets [`App::message`] to say so, and the return value stays a
    /// bare `Option`: the wording is display state, it belongs here with the
    /// rest of the display state, and a caller that had to translate an outcome
    /// into a sentence would be a second place that decides what a missing
    /// README means. A toggle that went through clears the message instead —
    /// whatever the last keystroke said, this one did something.
    ///
    /// Writing the manifest is the caller's job. This is app state and touches
    /// no file.
    pub fn toggle_pact(&mut self) -> Option<PactToggle> {
        let row = self.rows.get(self.selected)?;
        let path = row.path.clone();
        let Some(readme) = row.readme.clone() else {
            self.message = Some(no_readme_message(&self.label_for(&path)));
            return None;
        };

        let row = &mut self.rows[self.selected];
        let was = row.state;
        let now = match was {
            NodeState::Unpacted => NodeState::PactedStale,
            NodeState::PactedStale | NodeState::PactedFresh => NodeState::Unpacted,
        };
        row.state = now;
        // And in the unfiltered list the next collapse rebuilds the drawn rows
        // from, or the new state would last exactly until something was
        // collapsed and then quietly revert.
        if let Some(row) = self.all_rows.iter_mut().find(|row| row.path == path) {
            row.state = now;
        }
        self.message = None;

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

/// Which of `all` are drawn, given that every node in `collapsed` is
/// collapsed: the same rows in the same order at the same depths, less the
/// descendants of any collapsed node.
///
/// The filtering is done on depth rather than on paths, which is what makes it
/// one pass. A walk is depth first and parents come before children, so a
/// node's descendants are exactly the rows following it that are deeper than
/// it, up to the first row that is not — no path comparisons, and no
/// assumptions about how a child's path is spelled relative to its parent's.
///
/// A collapsed node inside a collapsed node is skipped like any other
/// descendant, so its own state is remembered and does nothing until its
/// ancestor opens: expanding a directory you cannot see is allowed and draws
/// nothing, which is the only reading that lets the collapsed set be carried
/// whole across a reload.
///
/// A collapsed node with no children hides nothing. It cannot arrive through
/// [`App::toggle_collapsed`], which refuses such a node, but it can arrive
/// through [`App::with_collapsed`] carrying a set from a tree whose shape has
/// since changed.
///
/// Pure, and deliberately free of [`App`]: rows in, rows out.
fn drawn_rows(all: &[Row], collapsed: &BTreeSet<PathBuf>) -> Vec<Row> {
    let mut drawn = Vec::with_capacity(all.len());
    // The depth of the collapsed node whose descendants are being skipped, if
    // any are.
    let mut hiding: Option<usize> = None;

    for row in all {
        if hiding.is_some_and(|depth| row.depth > depth) {
            continue;
        }
        hiding = (row.has_children() && collapsed.contains(&row.path)).then_some(row.depth);
        drawn.push(row.clone());
    }
    drawn
}

/// Which of `all` the pacted-only view keeps: every pacted node, plus every
/// ancestor needed to reach one, in the order and at the depths they came in.
///
/// An unpacted node survives only as somebody's way in. That makes the rule a
/// question about what comes *after* a row rather than before it, which is why
/// this pass runs backwards: a walk is depth first, so the ancestors of a row
/// are the rows before it that are shallower than it, and going in reverse means
/// each row is met with the requirement its descendants have already left
/// behind. `needed` is that requirement — the depth of the last row kept, which
/// is still short of an ancestor — so a row is kept when it is pacted itself or
/// when it is shallower than that, and every row kept replaces the requirement
/// with its own depth. Forwards, the same rule would mean holding every unpacted
/// directory aside until its subtree had been read.
///
/// Replaces rather than lowers, because the requirement belongs to one branch at
/// a time: a pacted node met after a shallower one has already been kept still
/// needs its own way in, and a running minimum would decide that way in had
/// already been found.
///
/// Depth is the whole of it: no path comparisons, so nothing here assumes how a
/// child's path is spelled relative to its parent's, and one reversed pass
/// answers for a walk of any shape.
///
/// Nothing is dropped from the tree and no state is read but [`Row::state`].
/// Pure, and deliberately free of [`App`]: rows in, rows out.
fn pacted_rows(all: &[Row]) -> Vec<Row> {
    let mut kept = Vec::with_capacity(all.len());
    // The depth of the last row kept, which is still waiting for an ancestor,
    // if anything is.
    let mut needed: Option<usize> = None;

    for row in all.iter().rev() {
        if row.state.is_pacted() || needed.is_some_and(|depth| row.depth < depth) {
            needed = Some(row.depth);
            kept.push(row.clone());
        }
    }
    kept.reverse();
    kept
}

/// Where `path` sits in `rows`, or where the deepest drawn ancestor of it
/// sits, or `None` when neither is drawn.
///
/// The ancestor is the fallback because it is what hiding a row leaves behind: a
/// collapsed node's nearest drawn ancestor is the directory that was collapsed
/// over it, and a filtered-away node's is the deepest part of the way to it that
/// the filter had a reason to keep. Either way that is where the selection
/// belongs, and either way it is the first row when nothing on the way to the
/// node survived at all. Ancestry is a path prefix
/// — the engine's paths nest the way the tree does — taken component by
/// component, so `crates-old` is no ancestor of anything under `crates`; and
/// the last matching row is the deepest one, since a walk visits ancestors
/// before descendants.
fn index_for(rows: &[Row], path: &Path) -> Option<usize> {
    rows.iter()
        .position(|row| row.path == path)
        .or_else(|| rows.iter().rposition(|row| path.starts_with(&row.path)))
}

/// Where the window onto `rows` rows should start, given a window `viewport`
/// rows tall, a selection at `selected`, and a window starting at `offset` now.
///
/// The rule is minimum movement. If the selection is already inside the window,
/// the window does not move at all; if it has fallen off an edge, the window
/// moves by exactly the rows needed to put it back on that same edge, and no
/// further. It never recentres: a reader stepping down one row wants the tree
/// to hold still and the selection to move, not the other way about, and a
/// window that jumps by half a screen loses whatever the reader was comparing
/// the selected row against.
///
/// The offset it is handed is a starting point, not a promise: it is clamped
/// to what the current row count allows, so a tree that has shrunk (or a
/// terminal that has grown) closes the gap under the last row instead of
/// leaving the window hanging past the end.
///
/// Two cases return `0` outright. A window at least as tall as the tree has
/// nothing to scroll — everything is on screen, and any offset at all would
/// push rows off the top for no reason. A window no rows tall has no screen to
/// scroll: nothing is drawn, so the honest offset is the top.
///
/// Pure, and deliberately free of [`App`]: it takes four numbers and returns
/// one, so every edge of it is testable with no rows, no tree and no terminal.
fn scroll_offset_for(rows: usize, viewport: usize, selected: usize, offset: usize) -> usize {
    if viewport == 0 {
        return 0;
    }
    // `None` exactly when the whole tree fits, which includes the no-rows case.
    let Some(max_offset) = rows.checked_sub(viewport) else {
        return 0;
    };

    let offset = offset.min(max_offset);
    if selected < offset {
        // Off the top: put it on the first visible row.
        selected
    } else if selected >= offset + viewport {
        // Off the bottom: put it on the last visible row. The subtraction
        // cannot wrap, because `selected` is at least `viewport` here, and the
        // clamp only bites for a `selected` past the last row, which `App`
        // never has.
        (selected + 1 - viewport).min(max_offset)
    } else {
        offset
    }
}

/// What the app says when a pact is refused for want of a README, naming the
/// directory as `label`.
///
/// Worded as a capability that is not wired up yet rather than as a chore for
/// the reader, because that is what it is: a README is no part of what a pact
/// means. Under the real pact operation the AI reads the module and writes one,
/// and a refresh puts back a README somebody deleted — today's `p` is a
/// placeholder that only flips a colour and writes a manifest line, so it has
/// nothing to point an entry at. Telling the reader to go and write the file
/// themselves would teach them a rule that is about to stop being true.
fn no_readme_message(label: &str) -> String {
    format!(
        "{label} has no README — writing one is the pact operation's job, \
         and that is not wired up yet"
    )
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

    use super::{App, PactToggle, REPOSITORY_ROOT_LABEL, Row, scroll_offset_for};
    use crate::fixture;

    /// How many rows the scrolling tests work with, and how tall the window
    /// onto them is. A tree comfortably taller than its window, so there is a
    /// top edge and a bottom edge that are not the same row.
    const MANY: usize = 20;
    /// The window height those tests set, small enough to leave rows off both
    /// ends of it.
    const WINDOW: u16 = 5;

    /// One of [`App`]'s selection-moving methods, so a test can drive the whole
    /// set of them from a list rather than repeating itself six times.
    type Movement = fn(&mut App);

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

    /// `count` rows of nothing in particular, for the tests that care only
    /// about how many there are and which one is selected.
    fn many_rows(count: usize) -> Vec<Row> {
        (0..count)
            .map(|index| {
                Row::new(
                    1,
                    format!("repo/module{index}"),
                    format!("repo/module{index}/README.md"),
                    NodeState::Unpacted,
                )
            })
            .collect()
    }

    /// An app of [`MANY`] rows with a [`WINDOW`]-tall window, selecting
    /// `selected` — reached by stepping, so the offset is whatever ordinary
    /// movement leaves it as rather than something the test wrote directly.
    fn scrolled_to(selected: usize) -> App {
        let mut app = App::from_rows(many_rows(MANY));
        app.set_viewport_height(WINDOW);
        while app.selected() < selected {
            app.select_next();
        }
        app
    }

    /// The rows an app draws, by path, for a test that is about which rows are
    /// on screen rather than what is on them.
    fn drawn(app: &App) -> Vec<String> {
        app.rows()
            .iter()
            .map(|row| row.path.to_string_lossy().into_owned())
            .collect()
    }

    /// The whole fixture, in walk order: what an app with nothing collapsed
    /// draws.
    fn whole_fixture() -> Vec<String> {
        vec![
            "warlock".to_owned(),
            "warlock/crates".to_owned(),
            "warlock/crates/engine".to_owned(),
            "warlock/crates/tui".to_owned(),
            "warlock/assets".to_owned(),
        ]
    }

    /// The fixture under the pacted-only filter: the two pacted leaves, the
    /// pacted root, and the undocumented `crates/` that is the only way down to
    /// them. `assets/` is unpacted and has nothing pacted below it, so it goes.
    fn pacted_fixture() -> Vec<String> {
        vec![
            "warlock".to_owned(),
            "warlock/crates".to_owned(),
            "warlock/crates/engine".to_owned(),
            "warlock/crates/tui".to_owned(),
        ]
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
    fn a_window_at_least_as_tall_as_the_tree_never_scrolls() {
        for viewport in [MANY, MANY + 1, MANY * 3] {
            for selected in 0..MANY {
                assert_eq!(
                    scroll_offset_for(MANY, viewport, selected, 0),
                    0,
                    "row {selected} of {MANY} in a window {viewport} tall"
                );
                // Including from an offset left over from a shorter window:
                // a terminal that has just been made taller closes the gap.
                assert_eq!(scroll_offset_for(MANY, viewport, selected, 7), 0);
            }
        }
    }

    #[test]
    fn a_selection_at_either_edge_of_the_window_leaves_it_alone() {
        // Rows 4..=8 are on screen.
        let (rows, viewport, offset) = (MANY, 5, 4);

        // The top edge, the bottom edge, and a row in between.
        assert_eq!(scroll_offset_for(rows, viewport, 4, offset), 4);
        assert_eq!(scroll_offset_for(rows, viewport, 8, offset), 4);
        assert_eq!(scroll_offset_for(rows, viewport, 6, offset), 4);
    }

    #[test]
    fn a_selection_one_past_an_edge_scrolls_by_exactly_one_row() {
        let (rows, viewport, offset) = (MANY, 5, 4);

        // One above the top edge: the window follows by one, so the selection
        // lands on the first visible row rather than in the middle.
        assert_eq!(scroll_offset_for(rows, viewport, 3, offset), 3);
        // One below the bottom edge: by one the other way, landing on the
        // last visible row.
        assert_eq!(scroll_offset_for(rows, viewport, 9, offset), 5);
    }

    #[test]
    fn the_first_and_last_rows_pin_the_window_to_the_ends() {
        let (rows, viewport) = (MANY, 5);

        assert_eq!(scroll_offset_for(rows, viewport, 0, 9), 0);
        // The last row sits on the bottom edge, so the window stops with the
        // last row of the tree on the last row of the screen: no blank space
        // scrolled in under it.
        assert_eq!(
            scroll_offset_for(rows, viewport, MANY - 1, 0),
            MANY - viewport
        );
    }

    #[test]
    fn a_long_jump_puts_the_selection_on_an_edge_and_does_not_recentre() {
        let (rows, viewport) = (MANY, 5);

        // Half a screen either side of the selection would be 13; the rule is
        // minimum movement, so it is the bottom edge instead.
        assert_eq!(scroll_offset_for(rows, viewport, 15, 0), 11);
        // And the same jump upwards from the bottom lands on the top edge.
        assert_eq!(scroll_offset_for(rows, viewport, 2, 15), 2);
    }

    #[test]
    fn an_offset_the_rows_no_longer_allow_is_pulled_back() {
        // Six rows in a five-tall window can only be scrolled by one, however
        // far the offset was left when the tree was longer.
        assert_eq!(scroll_offset_for(6, 5, 5, 99), 1);
        assert_eq!(scroll_offset_for(6, 5, 1, 99), 1);
    }

    #[test]
    fn nothing_to_show_means_nothing_scrolled_past() {
        // No rows, and no window to put them in: both are the top.
        assert_eq!(scroll_offset_for(0, 5, 0, 3), 0);
        assert_eq!(scroll_offset_for(MANY, 0, 12, 3), 0);
    }

    /// Whether the selected row is inside the window the app would be drawn
    /// with, which is the property every movement has to preserve.
    fn selection_is_on_screen(app: &App) -> bool {
        let first = app.scroll_offset();
        (first..first + app.viewport_height()).contains(&app.selected())
    }

    #[test]
    fn an_app_nobody_has_drawn_has_its_window_at_the_top() {
        let app = App::from_rows(many_rows(MANY));

        assert_eq!(app.scroll_offset(), 0);
        assert_eq!(app.viewport_height(), 0);
    }

    #[test]
    fn stepping_down_off_the_bottom_scrolls_one_row_at_a_time() {
        let window = usize::from(WINDOW);
        let mut app = scrolled_to(0);

        // Down to the bottom edge: still nothing scrolled.
        for _ in 0..window - 1 {
            app.select_next();
        }
        assert_eq!(app.selected(), window - 1);
        assert_eq!(app.scroll_offset(), 0);

        app.select_next();
        assert_eq!(app.scroll_offset(), 1);
        app.select_next();
        assert_eq!(app.scroll_offset(), 2);
        assert!(selection_is_on_screen(&app));

        // And back up: the window holds still all the way to its top row —
        // rows 2..=6 are on screen, so only leaving row 2 moves it.
        assert_eq!(app.selected(), window + 1);
        for _ in 0..window - 1 {
            app.select_previous();
            assert_eq!(app.scroll_offset(), 2, "at row {}", app.selected());
        }
        assert_eq!(app.selected(), 2);

        app.select_previous();
        assert_eq!(app.scroll_offset(), 1);
        assert!(selection_is_on_screen(&app));
    }

    #[test]
    fn paging_moves_by_a_windowful_and_stops_at_the_ends() {
        let window = usize::from(WINDOW);
        let mut app = scrolled_to(0);

        app.select_page_down();
        assert_eq!(app.selected(), window);
        assert!(selection_is_on_screen(&app));
        app.select_page_down();
        assert_eq!(app.selected(), 2 * window);
        assert!(selection_is_on_screen(&app));

        app.select_page_up();
        assert_eq!(app.selected(), window);
        assert!(selection_is_on_screen(&app));

        for _ in 0..MANY {
            app.select_page_down();
        }
        assert_eq!(app.selected(), MANY - 1);
        assert_eq!(app.scroll_offset(), MANY - window);

        for _ in 0..MANY {
            app.select_page_up();
        }
        assert_eq!(app.selected(), 0);
        assert_eq!(app.scroll_offset(), 0);
    }

    #[test]
    fn paging_an_undrawn_app_still_moves_one_row() {
        // No window means no windowful; a key that did nothing at all would
        // read as a broken key.
        let mut app = App::from_rows(many_rows(MANY));

        app.select_page_down();

        assert_eq!(app.selected(), 1);
        app.select_page_up();
        assert_eq!(app.selected(), 0);
    }

    #[test]
    fn jumping_to_the_last_row_and_back_moves_the_window_with_it() {
        let window = usize::from(WINDOW);
        let mut app = scrolled_to(0);

        app.select_last();

        assert_eq!(app.selected(), MANY - 1);
        assert_eq!(app.scroll_offset(), MANY - window);
        assert!(selection_is_on_screen(&app));

        app.select_first();

        assert_eq!(app.selected(), 0);
        assert_eq!(app.scroll_offset(), 0);
        assert!(selection_is_on_screen(&app));
    }

    #[test]
    fn a_shorter_terminal_scrolls_the_selection_back_into_view() {
        let mut app = scrolled_to(MANY - 1);
        assert_eq!(app.scroll_offset(), MANY - usize::from(WINDOW));

        app.set_viewport_height(3);

        assert_eq!(app.viewport_height(), 3);
        assert_eq!(app.scroll_offset(), MANY - 3);
        assert!(selection_is_on_screen(&app));
    }

    #[test]
    fn a_terminal_taller_than_the_tree_scrolls_back_to_the_top() {
        let mut app = scrolled_to(MANY - 1);

        app.set_viewport_height(u16::try_from(MANY).expect("a small row count") + 1);

        assert_eq!(app.scroll_offset(), 0);
        assert_eq!(app.selected(), MANY - 1);
        assert!(selection_is_on_screen(&app));
    }

    #[test]
    fn every_step_through_the_tree_keeps_the_selection_on_screen() {
        let mut app = scrolled_to(0);

        for _ in 0..MANY + 2 {
            app.select_next();
            assert!(selection_is_on_screen(&app), "at row {}", app.selected());
        }
        for _ in 0..MANY + 2 {
            app.select_previous();
            assert!(selection_is_on_screen(&app), "at row {}", app.selected());
        }
    }

    #[test]
    fn scrolling_an_empty_app_is_a_no_op() {
        let mut app = App::from_rows(Vec::new());
        app.set_viewport_height(WINDOW);

        app.select_page_down();
        app.select_page_up();
        app.select_last();
        app.select_first();

        assert!(app.is_empty());
        assert_eq!(app.selected(), 0);
        assert_eq!(app.scroll_offset(), 0);
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
        // Including `crates/`, whose README is honestly absent.
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
    fn a_directory_with_no_readme_cannot_be_pacted() {
        let mut app = app_selecting("warlock/crates");
        let before = app.clone();

        assert_eq!(app.toggle_pact(), None);

        // Everything but the message is exactly as it was: no state change, no
        // count change, and nothing for the caller to write down. Compared
        // field by field rather than as whole apps, because the message is the
        // one thing a refusal is meant to change.
        assert_eq!(app.rows(), before.rows());
        assert_eq!(app.counts(), before.counts());
        assert_eq!(app.selected(), before.selected());
        assert_eq!(
            app.selected_row().map(|row| row.state),
            Some(NodeState::Unpacted)
        );
        assert_eq!(app.counts(), tally(&app));
    }

    #[test]
    fn refusing_to_pact_says_why_naming_the_directory() {
        let mut app = app_selecting("warlock/crates");

        assert_eq!(app.toggle_pact(), None);

        let message = app.message().expect("a refusal says why");
        assert!(
            message.starts_with("warlock/crates has no README"),
            "{message}"
        );
        // Framed as a capability that is not wired up yet, not as a file the
        // reader has to go and write.
        assert!(message.contains("not wired up yet"), "{message}");
    }

    #[test]
    fn the_next_keystroke_clears_the_message() {
        let mut app = app_selecting("warlock/crates");
        assert_eq!(app.toggle_pact(), None);
        assert!(app.message().is_some());

        app.select_next();

        assert_eq!(app.message(), None);
    }

    #[test]
    fn every_movement_clears_a_message() {
        let movements: [(&str, Movement); 6] = [
            ("select_next", App::select_next),
            ("select_previous", App::select_previous),
            ("select_page_down", App::select_page_down),
            ("select_page_up", App::select_page_up),
            ("select_first", App::select_first),
            ("select_last", App::select_last),
        ];

        for (name, movement) in movements {
            let mut app = app_selecting("warlock/crates");
            app.set_message("something to forget");

            movement(&mut app);

            assert_eq!(app.message(), None, "{name} left the message behind");
        }
    }

    #[test]
    fn a_toggle_that_goes_through_leaves_no_refusal_behind() {
        let mut app = app_selecting("warlock/crates");
        assert_eq!(app.toggle_pact(), None);
        assert!(app.message().is_some());

        // Onto a documented row, which the movement clears the message for, and
        // then a toggle that works, which must not put one back.
        while app
            .selected_row()
            .expect("the fixture has rows")
            .readme
            .is_none()
        {
            app.select_next();
        }
        app.toggle_pact().expect("a documented row can be pacted");

        assert_eq!(app.message(), None);
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

        assert_eq!(app.rows(), before.rows());
        assert_eq!(app.counts(), before.counts());
        // The root cannot be named relative to itself, so it is named as it
        // stands rather than as the `"."` that relative spelling would give.
        assert_eq!(
            app.message().map(|message| message
                .split(" has no README")
                .next()
                .expect("a split always yields a first part")
                .to_owned()),
            Some("repo".to_owned())
        );
    }

    #[test]
    fn an_app_with_nothing_to_say_says_nothing() {
        assert_eq!(App::from_rows(three_rows()).message(), None);
        assert_eq!(App::from_tree(&fixture::tree()).message(), None);
    }

    #[test]
    fn a_caller_can_put_its_own_line_up_and_the_next_move_takes_it_down() {
        let mut app = App::from_rows(three_rows());

        app.set_message("could not write the pact manifest");

        assert_eq!(app.message(), Some("could not write the pact manifest"));
        app.select_next();
        assert_eq!(app.message(), None);
    }

    #[test]
    fn a_fresh_app_has_nothing_collapsed_and_draws_the_whole_walk() {
        let app = App::from_tree(&fixture::tree());

        assert!(app.collapsed().is_empty());
        assert_eq!(drawn(&app), whole_fixture());
        // And it knows which of those rows could be collapsed at all.
        for row in app.rows() {
            assert_eq!(
                row.children,
                children_in_fixture(&row.path),
                "child count for {}",
                row.path.display()
            );
            assert_eq!(row.has_children(), row.children > 0);
        }
    }

    /// How many children the fixture's tree gives the node at `path`.
    fn children_in_fixture(path: &Path) -> usize {
        fixture::tree()
            .find(path)
            .expect("the row came from the fixture")
            .children
            .len()
    }

    #[test]
    fn collapsing_a_directory_hides_its_descendants_and_expanding_puts_them_back() {
        let mut app = app_selecting("warlock/crates");
        let before = app.rows().to_vec();

        app.toggle_collapsed();

        assert_eq!(drawn(&app), ["warlock", "warlock/crates", "warlock/assets"]);
        assert!(app.is_collapsed("warlock/crates"));
        // The directory itself keeps its place and the selection.
        assert_eq!(
            app.selected_row().map(|row| row.path.clone()),
            Some(PathBuf::from("warlock/crates"))
        );

        app.toggle_collapsed();

        // Byte for byte the rows that were there before: same nodes, same
        // order, same depths, same states.
        assert_eq!(app.rows(), before);
        assert!(app.collapsed().is_empty());
    }

    #[test]
    fn collapsing_over_the_selection_puts_it_on_the_collapsed_directory() {
        let before = App::from_tree(&fixture::tree()).rows().to_vec();

        // The collapse arrives from outside, which is the only way a directory
        // above the selection is collapsed: the key itself acts on the row
        // under the selection.
        let mut app = app_selecting("warlock/crates/tui").with_collapsed(["warlock/crates"]);

        assert_eq!(drawn(&app), ["warlock", "warlock/crates", "warlock/assets"]);
        assert_eq!(
            app.selected_row().map(|row| row.path.clone()),
            Some(PathBuf::from("warlock/crates"))
        );
        // Never an index that is not drawn.
        assert!(app.selected() < app.rows().len());

        app.toggle_collapsed();

        assert_eq!(app.rows(), before);
    }

    #[test]
    fn collapsing_the_root_leaves_the_root_alone_on_screen() {
        let mut app = app_selecting("warlock/crates/engine");

        app.select_first();
        app.toggle_collapsed();

        assert_eq!(drawn(&app), ["warlock"]);
        assert_eq!(app.selected(), 0);
        assert_eq!(
            app.selected_row().map(|row| row.path.clone()),
            Some(PathBuf::from("warlock"))
        );

        app.toggle_collapsed();

        assert_eq!(drawn(&app), whole_fixture());
    }

    #[test]
    fn expanding_under_a_collapsed_parent_draws_nothing_until_the_parent_opens() {
        // Both collapsed, so only the root is drawn.
        let app = App::from_tree(&fixture::tree()).with_collapsed(["warlock", "warlock/crates"]);
        assert_eq!(drawn(&app), ["warlock"]);

        // `crates` expands while it is nowhere on screen: recorded, and drawn
        // nowhere, because its parent is still shut.
        let mut app = app.with_collapsed(["warlock"]);
        assert_eq!(drawn(&app), ["warlock"]);
        assert!(!app.is_collapsed("warlock/crates"));

        // And the expansion was waiting for the root all along.
        app.toggle_collapsed();

        assert_eq!(drawn(&app), whole_fixture());
    }

    #[test]
    fn a_collapse_above_the_selection_leaves_it_on_the_same_node() {
        let app = app_selecting("warlock/assets");
        assert_eq!(app.selected(), 4);

        let app = app.with_collapsed(["warlock/crates"]);

        // Two rows fewer above it, and the same node under it.
        assert_eq!(app.selected(), 2);
        assert_eq!(
            app.selected_row().map(|row| row.path.clone()),
            Some(PathBuf::from("warlock/assets"))
        );
    }

    #[test]
    fn a_rebuilt_app_carrying_the_collapsed_set_hides_the_same_rows() {
        let mut app = app_selecting("warlock/crates");
        app.toggle_collapsed();

        // What the binary does when the tree is reloaded: new app state from a
        // fresh tree, the view carried across by path.
        let rebuilt = App::from_tree(&fixture::tree()).with_collapsed(app.collapsed());

        assert_eq!(rebuilt.rows(), app.rows());
        assert_eq!(rebuilt.collapsed(), app.collapsed());
        // Filtered on the way in, not at the next keystroke.
        assert_eq!(
            drawn(&rebuilt),
            ["warlock", "warlock/crates", "warlock/assets"]
        );
    }

    #[test]
    fn a_collapsed_path_the_new_tree_has_no_node_for_hides_nothing() {
        let app = App::from_tree(&fixture::tree()).with_collapsed(["warlock/gone"]);

        assert_eq!(drawn(&app), whole_fixture());
        // Kept all the same: a directory that comes back should come back shut.
        assert!(app.is_collapsed("warlock/gone"));
    }

    #[test]
    fn toggling_a_childless_node_changes_nothing_at_all() {
        let mut app = app_selecting("warlock/assets");
        app.set_message("something from the last keystroke");
        let before = app.clone();

        app.toggle_collapsed();

        // Including the message: a key that did nothing should not look like a
        // key that did something.
        assert_eq!(app, before);
        assert!(app.collapsed().is_empty());
        assert_eq!(drawn(&app), whole_fixture());
    }

    #[test]
    fn toggling_collapse_on_an_empty_app_is_a_no_op() {
        let mut app = App::from_rows(Vec::new());

        app.toggle_collapsed();

        assert!(app.is_empty());
        assert_eq!(app.selected(), 0);
        assert!(app.collapsed().is_empty());
    }

    #[test]
    fn collapsing_leaves_the_engines_tally_alone() {
        let tree = fixture::tree();
        let mut app = App::from_tree(&tree);

        app.select_first();
        app.toggle_collapsed();

        // Four of the five rows are hidden; the footer still describes the
        // whole tree, because that is what the engine counted.
        assert_eq!(app.rows().len(), 1);
        assert_eq!(app.counts(), tree.counts());
        assert_eq!(app.counts().total(), 5);
    }

    #[test]
    fn a_pact_survives_a_collapse_and_expand_of_the_directory_above_it() {
        let mut app = app_selecting("warlock/crates/tui");
        app.toggle_pact().expect("tui has a README");
        assert_eq!(
            app.selected_row().map(|row| row.state),
            Some(NodeState::Unpacted)
        );

        let mut app = app.with_collapsed(["warlock/crates"]);
        app.toggle_collapsed();

        let row = app
            .rows()
            .iter()
            .find(|row| row.path == Path::new("warlock/crates/tui"))
            .expect("expanding brought it back");
        assert_eq!(row.state, NodeState::Unpacted);
    }

    #[test]
    fn a_toggle_leaves_the_window_in_range_with_the_selection_in_it() {
        let mut app = app_selecting("warlock/assets");
        app.set_viewport_height(2);
        assert!(selection_is_on_screen(&app));

        // A collapse from above, shortening the tree under a window that was
        // scrolled to the bottom of it.
        let mut app = app.with_collapsed(["warlock/crates"]);

        assert!(window_is_in_range(&app));
        assert!(selection_is_on_screen(&app));

        // And the extreme of it: everything but the root gone.
        app.select_first();
        app.toggle_collapsed();

        assert_eq!(app.rows().len(), 1);
        assert_eq!(app.scroll_offset(), 0);
        assert!(window_is_in_range(&app));
        assert!(selection_is_on_screen(&app));

        app.toggle_collapsed();

        assert!(window_is_in_range(&app));
        assert!(selection_is_on_screen(&app));
    }

    /// Whether the window sits over rows that exist: it may start at the top of
    /// a tree shorter than itself, but it must never hang off the end of one.
    fn window_is_in_range(app: &App) -> bool {
        app.scroll_offset() <= app.rows().len().saturating_sub(app.viewport_height())
    }

    #[test]
    fn a_collapse_clears_the_last_keystrokes_message() {
        let mut app = app_selecting("warlock/crates");
        assert_eq!(app.toggle_pact(), None);
        assert!(app.message().is_some());

        app.toggle_collapsed();

        assert_eq!(app.message(), None);
    }

    #[test]
    fn collapsing_a_directory_leaves_a_sibling_whose_name_it_prefixes_alone() {
        let tree = Tree::new(
            Node::new("repo", "repo/README.md", NodeState::PactedStale).with_children([
                Node::new("repo/crates", None, NodeState::Unpacted).with_children([Node::new(
                    "repo/crates/engine",
                    "repo/crates/engine/README.md",
                    NodeState::PactedFresh,
                )]),
                Node::new("repo/crates-old", None, NodeState::Unpacted),
            ]),
        );
        let mut app = App::from_tree(&tree);

        app.select_next();
        app.toggle_collapsed();

        // A sibling is not a descendant, however much of its name it shares.
        assert_eq!(drawn(&app), ["repo", "repo/crates", "repo/crates-old"]);
    }

    #[test]
    fn rows_from_a_bare_list_collapse_only_where_they_claim_children() {
        let mut app = App::from_rows(vec![
            Row::new(0, "repo", "repo/README.md", NodeState::PactedStale).with_child_count(1),
            Row::new(1, "repo/crates", None, NodeState::Unpacted),
        ]);

        app.toggle_collapsed();
        assert_eq!(drawn(&app), ["repo"]);

        app.toggle_collapsed();
        assert_eq!(drawn(&app), ["repo", "repo/crates"]);
        // The child, which claims none of its own, is not collapsible.
        app.select_next();
        app.toggle_collapsed();
        assert!(app.collapsed().is_empty());
    }

    #[test]
    fn toggling_a_pact_on_an_empty_app_is_a_no_op() {
        let mut app = App::from_rows(Vec::new());

        assert_eq!(app.toggle_pact(), None);

        assert!(app.is_empty());
        assert_eq!(app.counts(), StateCounts::default());
    }

    #[test]
    fn a_fresh_app_draws_the_whole_tree_rather_than_only_the_pacted_part() {
        let app = App::from_tree(&fixture::tree());

        assert!(!app.pacted_only());
        assert_eq!(drawn(&app), whole_fixture());
    }

    #[test]
    fn the_filter_keeps_the_pacted_nodes_and_the_way_to_them_and_nothing_else() {
        let mut app = app_selecting("warlock/crates/tui");
        let before = app.clone();

        app.toggle_pacted_only();

        assert!(app.pacted_only());
        assert_eq!(drawn(&app), pacted_fixture());
        // Every drawn row is pacted or is on the way to one that is, and every
        // pacted node in the tree is drawn.
        for row in app.rows() {
            assert!(
                row.state.is_pacted()
                    || app
                        .rows()
                        .iter()
                        .any(|other| other.state.is_pacted() && other.path.starts_with(&row.path)),
                "{} is neither pacted nor the way to anything pacted",
                row.path.display()
            );
        }
        assert_eq!(
            app.rows()
                .iter()
                .filter(|row| row.state.is_pacted())
                .count(),
            fixture::tree()
                .walk()
                .filter(|(node, _)| node.state.is_pacted())
                .count()
        );

        app.toggle_pacted_only();

        // Widening again is the whole walk back, and — the selection having
        // survived the narrowing — an app indistinguishable from the one before.
        assert!(!app.pacted_only());
        assert_eq!(drawn(&app), whole_fixture());
        assert_eq!(app, before);
    }

    #[test]
    fn the_filter_keeps_the_depths_and_the_order_the_walk_gave() {
        let mut app = App::from_tree(&fixture::tree());

        app.toggle_pacted_only();

        let seen: Vec<(usize, &str)> = app
            .rows()
            .iter()
            .map(|row| (row.depth, row.path.to_str().expect("ascii path")))
            .collect();
        // `engine` and `tui` still sit at depth 2 under a `crates` that is only
        // drawn as their way in: the filter narrows the view, it does not
        // reparent anything.
        assert_eq!(
            seen,
            [
                (0, "warlock"),
                (1, "warlock/crates"),
                (2, "warlock/crates/engine"),
                (2, "warlock/crates/tui"),
            ]
        );
    }

    #[test]
    fn filtering_the_selected_node_away_lands_the_selection_on_a_drawn_row() {
        let mut app = app_selecting("warlock/assets");

        app.toggle_pacted_only();

        // `assets` is gone, and its nearest surviving ancestor is the root,
        // which is the first row.
        assert_eq!(app.selected(), 0);
        assert_eq!(
            app.selected_row().map(|row| row.path.clone()),
            Some(PathBuf::from("warlock"))
        );
        assert!(app.selected() < app.rows().len());
    }

    #[test]
    fn widening_the_view_keeps_the_selection_on_the_node_it_was_on() {
        let mut app = app_selecting("warlock/assets");
        app.toggle_pacted_only();

        // Down to the last row the narrowed view has, which is a node the wide
        // view puts somewhere else entirely.
        app.select_last();
        assert_eq!(
            app.selected_row().map(|row| row.path.clone()),
            Some(PathBuf::from("warlock/crates/tui"))
        );

        app.toggle_pacted_only();

        assert_eq!(
            app.selected_row().map(|row| row.path.clone()),
            Some(PathBuf::from("warlock/crates/tui"))
        );
        assert_eq!(drawn(&app), whole_fixture());
    }

    #[test]
    fn the_filter_leaves_the_engines_tally_alone() {
        let tree = fixture::tree();
        let mut app = App::from_tree(&tree);
        let before = app.counts();

        app.toggle_pacted_only();

        // A row fewer on screen, and the same five nodes tallied: the footer
        // describes the tree, not the view.
        assert_eq!(app.rows().len(), 4);
        assert_eq!(app.counts(), before);
        assert_eq!(app.counts(), tree.counts());
        assert_eq!(app.counts().total(), 5);

        app.toggle_pacted_only();

        assert_eq!(app.counts(), before);
        assert_eq!(app.counts(), tally(&app));
    }

    #[test]
    fn a_directory_collapsed_before_the_filter_is_still_collapsed_after_it() {
        let mut app = app_selecting("warlock/crates");
        app.toggle_collapsed();
        assert_eq!(drawn(&app), ["warlock", "warlock/crates", "warlock/assets"]);

        app.toggle_pacted_only();

        // `crates` is unpacted and its descendants are hidden, but it is still
        // the way to them, so it keeps its row: what survives the filter is read
        // off the whole walk, not off what collapsing left drawn.
        assert_eq!(drawn(&app), ["warlock", "warlock/crates"]);
        assert!(app.is_collapsed("warlock/crates"));

        app.toggle_pacted_only();

        assert!(app.is_collapsed("warlock/crates"));
        assert_eq!(drawn(&app), ["warlock", "warlock/crates", "warlock/assets"]);

        // And the collapse is still a collapse afterwards, not a filter
        // casualty: expanding puts the descendants back.
        app.toggle_collapsed();
        assert_eq!(drawn(&app), whole_fixture());
    }

    #[test]
    fn collapsing_under_the_filter_hides_descendants_as_it_always_did() {
        let mut app = App::from_tree(&fixture::tree());
        app.toggle_pacted_only();

        app.toggle_collapsed();

        // The root, collapsed over everything the filter kept.
        assert_eq!(drawn(&app), ["warlock"]);
        assert!(app.is_collapsed("warlock"));

        app.toggle_collapsed();
        assert_eq!(drawn(&app), pacted_fixture());
    }

    #[test]
    fn nothing_pacted_narrows_to_nothing_at_all() {
        let mut app = App::from_rows(vec![
            Row::new(0, "repo", "repo/README.md", NodeState::Unpacted).with_child_count(1),
            Row::new(1, "repo/crates", None, NodeState::Unpacted),
        ]);

        app.toggle_pacted_only();

        // No pacted node means no ancestors worth keeping either, and an app
        // with no rows selects nothing rather than an index off the end.
        assert!(app.is_empty());
        assert_eq!(app.selected(), 0);
        assert_eq!(app.selected_row(), None);

        app.toggle_pacted_only();
        assert_eq!(drawn(&app), ["repo", "repo/crates"]);
    }

    #[test]
    fn pacting_a_node_under_the_filter_brings_its_row_into_the_view_to_stay() {
        let mut app = app_selecting("warlock/assets");
        app.toggle_pact().expect("assets has a README");

        app.toggle_pacted_only();

        // Newly pacted, so newly worth drawing: the filter reads the states as
        // they are now, not as the tree was loaded with them.
        assert_eq!(
            drawn(&app),
            [
                "warlock",
                "warlock/crates",
                "warlock/crates/engine",
                "warlock/crates/tui",
                "warlock/assets",
            ]
        );
    }

    #[test]
    fn an_unpacted_subtree_goes_whole_while_its_unpacted_siblings_way_in_stays() {
        let tree = Tree::new(Node::new("repo", None, NodeState::Unpacted).with_children([
            Node::new("repo/kept", None, NodeState::Unpacted).with_children([Node::new(
                "repo/kept/deep",
                "repo/kept/deep/README.md",
                NodeState::PactedFresh,
            )]),
            Node::new("repo/gone", None, NodeState::Unpacted).with_children([Node::new(
                "repo/gone/deep",
                "repo/gone/deep/README.md",
                NodeState::Unpacted,
            )]),
        ]));
        let mut app = App::from_tree(&tree);

        app.toggle_pacted_only();

        // An unpacted root and an unpacted directory both survive as the way to
        // one pacted node; the sibling branch with nothing pacted in it goes
        // whole, parent and child together.
        assert_eq!(drawn(&app), ["repo", "repo/kept", "repo/kept/deep"]);
    }

    #[test]
    fn toggling_the_filter_clears_the_last_keystrokes_message() {
        let mut app = app_selecting("warlock/crates");
        assert_eq!(app.toggle_pact(), None);
        assert!(app.message().is_some());

        app.toggle_pacted_only();

        assert_eq!(app.message(), None);
    }

    #[test]
    fn narrowing_leaves_the_window_in_range_with_the_selection_in_it() {
        let mut app = app_selecting("warlock/assets");
        app.set_viewport_height(2);
        assert!(selection_is_on_screen(&app));

        app.toggle_pacted_only();

        assert!(window_is_in_range(&app));
        assert!(selection_is_on_screen(&app));

        app.toggle_pacted_only();

        assert!(window_is_in_range(&app));
        assert!(selection_is_on_screen(&app));
    }
}
