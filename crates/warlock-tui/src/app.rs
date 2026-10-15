//! What the front end holds between keystrokes: a plain value, no terminal.
//!
//! The engine hands over a [`Tree`]; the screen wants a list of lines. So
//! `walk_of` flattens the depth-first walk once into `all_rows` — every node,
//! plus a row per file each node lists — and `reflow` derives the drawn `rows`
//! from it by three filters in a fixed order: files, then scoped-only, then
//! collapsed. `all_rows` is never filtered in place, so every view toggle is
//! the same re-derivation and turning one off puts the rest back untouched.
//!
//! Two consequences worth knowing before editing anything here. First, what the
//! reader has selected and which directories are collapsed are remembered as
//! *paths*, never as row indices: `reflow` and [`reseat_on`] both rebuild the
//! row list, and an index into the old list names a different node in the new
//! one. Second, `reflow` is the only thing that turns `all_rows` into `rows`.

use std::borrow::Cow;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use warlock_engine::{Tree, to_manifest_path};

use crate::panel::Panel;
use crate::selection::{Position, Selection};

/// One line of the flattened tree.
///
/// A row owns its path rather than borrowing from the [`Tree`], so [`App`] is a
/// self-contained value with no lifetimes threading through the event loop, and
/// a key handler can refuse a keystroke from the row alone without going back
/// to the tree for a second copy of the same fact.
///
/// Most rows stand for a node. The rest stand for one of the files a node lists
/// (see [`Row::file`]): drawn like any other row, and nothing like one
/// otherwise — it holds nothing and is scoped by nothing of its own.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct Row {
    pub depth: usize,
    pub path: PathBuf,
    /// Whether a scope covers the row: its own directory's or an ancestor's. A
    /// file row carries its directory's, which is what makes a file take its
    /// directory's colour.
    pub covered: bool,
    pub file: bool,
    pub scope: Option<String>,
}

impl Row {
    #[must_use]
    pub(crate) fn new(depth: usize, path: impl Into<PathBuf>, covered: bool) -> Self {
        Self {
            depth,
            path: path.into(),
            covered,
            file: false,
            scope: None,
        }
    }

    #[must_use]
    pub(crate) fn file(depth: usize, path: impl Into<PathBuf>, covered: bool) -> Self {
        Self {
            file: true,
            ..Self::new(depth, path, covered)
        }
    }

    #[must_use]
    pub(crate) fn with_scope(mut self, scope: Option<String>) -> Self {
        self.scope = scope;
        self
    }

    #[must_use]
    pub(crate) const fn is_file(&self) -> bool {
        self.file
    }
}

// The board work the workflow strip lights, each on a value of its own in the
// loop.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Working {
    Push,
    Draft,
    Pull,
}

/// Which of the screen's three places the movement keys drive.
///
/// [`Focus::next`] cycles through all three unconditionally; whether the
/// composer is reachable is [`App::toggle_focus`]'s question, not this type's,
/// because only the app can see the panel. The pointer methods
/// ([`App::select_row`], [`App::scroll_panel_down`] and their neighbours) name
/// their pane instead and never consult the focus at all.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub(crate) enum Focus {
    #[default]
    Tree,
    Panel,
    Composer,
}

impl Focus {
    #[must_use]
    pub(crate) const fn next(self) -> Self {
        match self {
            Self::Tree => Self::Panel,
            Self::Panel => Self::Composer,
            Self::Composer => Self::Tree,
        }
    }

    #[must_use]
    pub(crate) const fn drives_the_tree(self) -> bool {
        match self {
            Self::Tree => true,
            Self::Panel | Self::Composer => false,
        }
    }

    #[must_use]
    pub(crate) const fn drives_the_panel(self) -> bool {
        match self {
            Self::Panel => true,
            Self::Tree | Self::Composer => false,
        }
    }
}

/// Which sigils the machine holds.
///
/// `Nothing` and `Unknown` are not the same answer and must not be merged:
/// one is a config that holds nothing, the other a config that would not read,
/// and drawing broken as absent tells a reader they hold nothing when warlock
/// could not find out. [`Sigils::held`] folds an empty list into `Nothing`, so
/// `Held` is never empty.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub(crate) enum Sigils {
    #[default]
    Nothing,
    Held(Vec<String>),
    Unknown,
}

impl Sigils {
    #[must_use]
    pub(crate) fn held(sigils: impl IntoIterator<Item = impl Into<String>>) -> Self {
        let sigils: Vec<String> = sigils.into_iter().map(Into::into).collect();
        if sigils.is_empty() {
            return Self::Nothing;
        }
        Self::Held(sigils)
    }

    #[must_use]
    pub(crate) fn as_slice(&self) -> &[String] {
        match self {
            Self::Nothing | Self::Unknown => &[],
            Self::Held(sigils) => sigils,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub(crate) struct Held {
    pub(crate) sigils: Sigils,
    pub(crate) base: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Chrome {
    header: String,
    held: Held,
    // Kept apart from the base rather than resolved into one name, so
    // `/branch clear` has the remote's default to fall back to without asking
    // `git` again.
    remote_head: Option<String>,
}

impl Chrome {
    /// The header is empty for a root that *is* the repo root, so the title bar
    /// draws no redundant "." — and falls back to the absolute path when `root`
    /// turns out not to be under `repo_root` at all.
    #[must_use]
    pub(crate) fn of(repo_root: impl AsRef<Path>, root: impl AsRef<Path>) -> Self {
        let root = root.as_ref();
        let header = match to_manifest_path(repo_root, root) {
            Ok(relative) if relative == "." => String::new(),
            Ok(relative) => relative,
            Err(_) => root.display().to_string(),
        };
        Self {
            header,
            held: Held::default(),
            remote_head: None,
        }
    }

    #[must_use]
    pub(crate) fn with_held(mut self, held: Held) -> Self {
        self.held = held;
        self
    }

    #[must_use]
    pub(crate) fn header(&self) -> &str {
        &self.header
    }

    #[must_use]
    pub(crate) const fn sigils(&self) -> &Sigils {
        &self.held.sigils
    }

    #[must_use]
    pub(crate) fn with_remote_head(mut self, remote_head: Option<String>) -> Self {
        self.remote_head = remote_head;
        self
    }

    pub(crate) fn set_held(&mut self, held: Held) {
        self.held = held;
    }

    #[must_use]
    pub(crate) fn remote_head(&self) -> Option<&str> {
        self.remote_head.as_deref()
    }

    #[must_use]
    pub(crate) fn target(&self) -> Target<'_> {
        Target::of(self.held.base.as_deref(), self.remote_head())
    }
}

/// The branch a pull starts from and opens its pull request against.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Target<'a> {
    Set(&'a str),
    Default(&'a str),
    Unknown,
}

impl<'a> Target<'a> {
    #[must_use]
    pub(crate) const fn of(base: Option<&'a str>, remote_head: Option<&'a str>) -> Self {
        match (base, remote_head) {
            (Some(base), _) => Self::Set(base),
            (None, Some(head)) => Self::Default(head),
            (None, None) => Self::Unknown,
        }
    }

    #[must_use]
    pub(crate) fn sentence(self) -> String {
        match self {
            Self::Set(base) => format!("`{base}`"),
            Self::Default(head) => format!("the remote's default branch, `{head}`"),
            Self::Unknown => "the remote's default branch".to_owned(),
        }
    }
}

/// The whole of the front end's state, and none of its plumbing.
///
/// `all_rows` is the flattening of the tree and `rows` is the part of it being
/// drawn; `selected` and `scroll_offset` index `rows`, so anything that
/// rebuilds `rows` owes both of them a re-derivation. The three groups below
/// are grouped so that [`reseat_on`] can carry them across a reload by moving
/// three fields rather than by copying twenty.
///
/// `highlight` is positions in the panel's thread rather than anything the tree
/// holds, so it goes wherever the panel goes and nowhere else.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct App {
    all_rows: Vec<Row>,
    rows: Vec<Row>,
    collapsible: BTreeSet<PathBuf>,
    selected: usize,
    scroll_offset: usize,
    viewpoint: Viewpoint,
    status: Status,
    panel: Panel,
    highlight: Option<Highlight>,
}

/// What the reader has highlighted in the thread card, and where the drag making
/// it began.
///
/// The anchor is kept because it cannot be recovered from the selection:
/// [`Selection::new`] puts its two positions in reading order, so a drag that
/// has gone up past where it started is indistinguishable from one that has not,
/// and extending from the wrong end would grow the selection the wrong way.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Highlight {
    anchor: Position,
    selection: Selection,
}

// What the reader is looking at rather than what is being looked at, which is
// why a reload carries this whole and rebuilds everything above it.
#[derive(Debug, Clone, Default, PartialEq)]
struct Viewpoint {
    collapsed: BTreeSet<PathBuf>,
    scoped_only: bool,
    show_files: bool,
    viewport_height: usize,
    focus: Focus,
}

#[derive(Debug, Clone, Default, PartialEq)]
struct Status {
    // Belongs to the last keystroke: set by whatever refused it, dropped by the
    // next movement (`forget_last_keystroke`).
    message: Option<String>,
    // How many times the footer has been told something, which is what lets the
    // session time a message out: the same sentence said twice is two sayings,
    // and a reader who has just copied the same number of characters again is
    // owed the full ten seconds rather than what was left of the first.
    said: u64,
    mouse_captured: bool,
    working: Option<Working>,
}

impl App {
    #[must_use]
    pub(crate) fn from_tree(tree: &Tree) -> Self {
        Self::from_rows(walk_of(tree))
    }

    #[must_use]
    pub(crate) fn from_rows(rows: Vec<Row>) -> Self {
        // Every field named, and no `..Default::default()` anywhere in this
        // file: a struct literal that names all of them is what turns a
        // twenty-third field into a compile error here and in [`reseat_on`],
        // rather than into a field somebody forgot to carry. See the note on
        // [`reseat_on`].
        let mut app = Self {
            rows: rows.clone(),
            all_rows: rows,
            collapsible: BTreeSet::new(),
            selected: 0,
            scroll_offset: 0,
            viewpoint: Viewpoint {
                collapsed: BTreeSet::new(),
                scoped_only: false,
                show_files: false,
                viewport_height: 0,
                focus: Focus::Tree,
            },
            status: Status {
                message: None,
                said: 0,
                mouse_captured: false,
                working: None,
            },
            panel: Panel::default(),
            highlight: None,
        };
        // The rows handed over may hold file rows, which the file toggle starts
        // off over, so the drawn list is derived rather than assumed even here.
        app.reflow();
        app
    }

    #[must_use]
    #[cfg(test)]
    pub(crate) fn with_collapsed(
        mut self,
        collapsed: impl IntoIterator<Item = impl Into<PathBuf>>,
    ) -> Self {
        self.viewpoint.collapsed = collapsed.into_iter().map(Into::into).collect();
        self.reflow();
        self
    }

    #[must_use]
    pub(crate) fn message(&self) -> Option<&str> {
        self.status.message.as_deref()
    }

    pub(crate) fn set_message(&mut self, message: impl Into<String>) {
        self.say(message.into());
    }

    /// The one door onto the footer's line, so that nothing sets it without
    /// counting: a saying nobody counted is one the session's timer reads as
    /// the saying before it, and it goes off the screen early.
    fn say(&mut self, message: String) {
        self.status.message = Some(message);
        self.status.said = self.status.said.saturating_add(1);
    }

    /// The count of sayings, whatever they were. Paired with [`App::message`] by
    /// the session, which holds the clock this side has none of.
    #[must_use]
    pub(crate) const fn said(&self) -> u64 {
        self.status.said
    }

    /// Said long enough ago that the footer would be claiming a moment that has
    /// passed. Not a keystroke's business — [`App::forget_last_keystroke`] is
    /// what a movement clears — so it is separate from that.
    pub(crate) fn forget_message(&mut self) {
        self.status.message = None;
    }

    pub(crate) fn show_document(
        &mut self,
        lines: impl IntoIterator<Item = impl Into<String>>,
        cut: bool,
    ) {
        self.panel.show_document(lines, cut);
        self.rescue_focus();
    }

    pub(crate) fn swap_card(&mut self) {
        let Some(card) = self.panel.next_card() else {
            self.set_message(no_document_message());
            return;
        };
        self.panel.show(card);
        // The highlight is drawn on the thread card, so a card taking its place
        // would leave it standing over text it was never measured against.
        self.clear_selection();
        self.rescue_focus();
    }

    #[must_use]
    pub(crate) const fn panel(&self) -> &Panel {
        &self.panel
    }

    pub(crate) const fn panel_mut(&mut self) -> &mut Panel {
        &mut self.panel
    }

    /// What the reader has highlighted in the thread card, if anything.
    #[must_use]
    pub(crate) fn selection(&self) -> Option<Selection> {
        self.highlight.map(|highlight| highlight.selection)
    }

    /// Anchors a selection at `at` and drops whatever was highlighted before.
    ///
    /// It covers nothing until [`App::extend_selection`] moves its far end,
    /// which is what makes a press nobody dragged from copy no text rather than
    /// the character under the pointer.
    ///
    /// Does nothing when there is no conversation, since a position is an offset
    /// into a thread's text and there is none to snap it against.
    pub(crate) fn start_selection(&mut self, at: Position) {
        let Some(selection) = self.selection_between(at, at) else {
            return;
        };
        self.highlight = Some(Highlight {
            anchor: at,
            selection,
        });
    }

    /// Moves the far end of the selection to `to`, leaving the anchor where the
    /// press put it. Does nothing until a press has put one there: a drag with
    /// no anchor behind it has no second position to make a selection from.
    pub(crate) fn extend_selection(&mut self, to: Position) {
        let Some(highlight) = self.highlight else {
            return;
        };
        let Some(selection) = self.selection_between(highlight.anchor, to) else {
            return;
        };
        self.highlight = Some(Highlight {
            anchor: highlight.anchor,
            selection,
        });
    }

    pub(crate) fn clear_selection(&mut self) {
        self.highlight = None;
    }

    // Both positions go through `Selection::new` every time rather than being
    // snapped once and stored: it is the only thing that puts them on a char
    // boundary and in reading order, and a position that skipped it would be
    // sliced with by everything in `selection.rs`.
    fn selection_between(&self, anchor: Position, to: Position) -> Option<Selection> {
        let thread = self.panel.thread()?;
        Some(Selection::new(&thread.pieces(), anchor, to))
    }

    #[must_use]
    pub(crate) fn rows(&self) -> &[Row] {
        &self.rows
    }

    #[must_use]
    #[cfg(test)]
    pub(crate) const fn collapsed(&self) -> &BTreeSet<PathBuf> {
        &self.viewpoint.collapsed
    }

    #[must_use]
    pub(crate) fn is_collapsed(&self, path: impl AsRef<Path>) -> bool {
        self.viewpoint.collapsed.contains(path.as_ref())
    }

    /// Asks whether the row has anything *drawn* under it, not whether the node
    /// has children: a directory whose children are all filtered away collapses
    /// onto nothing, so the marker is not offered. `collapsible` is rebuilt by
    /// `drawn_rows` on every `reflow` for that reason.
    #[must_use]
    pub(crate) fn can_collapse(&self, index: usize) -> bool {
        self.rows
            .get(index)
            .is_some_and(|row| self.collapsible.contains(&row.path))
    }

    #[must_use]
    #[cfg(test)]
    pub(crate) const fn scoped_only(&self) -> bool {
        self.viewpoint.scoped_only
    }

    pub(crate) fn toggle_scoped_only(&mut self) {
        self.viewpoint.scoped_only = !self.viewpoint.scoped_only;
        self.forget_last_keystroke();
        self.reflow();
    }

    #[must_use]
    #[cfg(test)]
    pub(crate) const fn show_files(&self) -> bool {
        self.viewpoint.show_files
    }

    pub(crate) fn toggle_files(&mut self) {
        self.viewpoint.show_files = !self.viewpoint.show_files;
        self.forget_last_keystroke();
        self.reflow();
    }

    /// Directories a scope covers, then directories none does, across the whole
    /// tree whatever is drawn: a footer that counted only what the filters left
    /// would change its numbers when the reader toggled a view.
    #[must_use]
    pub(crate) fn coverage(&self) -> (usize, usize) {
        let (mut scoped, mut unscoped) = (0, 0);
        for row in self.all_rows.iter().filter(|row| !row.is_file()) {
            if row.covered {
                scoped += 1;
            } else {
                unscoped += 1;
            }
        }
        (scoped, unscoped)
    }

    #[must_use]
    #[cfg(test)]
    pub(crate) fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    #[must_use]
    pub(crate) const fn selected(&self) -> usize {
        self.selected
    }

    #[must_use]
    pub(crate) fn selected_row(&self) -> Option<&Row> {
        self.rows.get(self.selected)
    }

    #[must_use]
    pub(crate) const fn scroll_offset(&self) -> usize {
        self.scroll_offset
    }

    #[must_use]
    #[cfg(test)]
    pub(crate) const fn viewport_height(&self) -> usize {
        self.viewpoint.viewport_height
    }

    pub(crate) fn set_viewport_height(&mut self, height: u16) {
        self.viewpoint.viewport_height = usize::from(height);
        self.rescroll();
    }

    pub(crate) const fn set_mouse_captured(&mut self, captured: bool) {
        self.status.mouse_captured = captured;
    }

    // Told once a round by the loop, which holds the three values that know,
    // rather than at each edge: a strip that had to be told when work stopped
    // is one a missed edge leaves lit.
    pub(crate) const fn set_working(&mut self, working: Option<Working>) {
        self.status.working = working;
    }

    #[must_use]
    pub(crate) const fn working(&self) -> Option<Working> {
        self.status.working
    }

    #[must_use]
    pub(crate) const fn focus(&self) -> Focus {
        self.viewpoint.focus
    }

    pub(crate) const fn toggle_focus(&mut self) {
        let next = self.viewpoint.focus.next();
        self.viewpoint.focus = match next {
            Focus::Composer if !self.panel().composer_showable() => next.next(),
            Focus::Tree | Focus::Panel | Focus::Composer => next,
        };
    }

    pub(crate) const fn set_focus(&mut self, focus: Focus) {
        self.viewpoint.focus = focus;
        self.rescue_focus();
    }

    // The composer can stop being showable under a focus already resting on it
    // — a card swap, a document arriving — so every path that sets a focus or
    // changes what the panel shows ends here rather than trusting the focus it
    // was handed.
    const fn rescue_focus(&mut self) {
        match self.viewpoint.focus {
            Focus::Composer if !self.panel().composer_showable() => {
                self.viewpoint.focus = Focus::Panel;
            }
            Focus::Tree | Focus::Panel | Focus::Composer => {}
        }
    }

    pub(crate) fn select_previous(&mut self) {
        self.movement(
            |app| app.selected.saturating_sub(1),
            |_, offset| offset.saturating_sub(1),
        );
    }

    pub(crate) fn select_next(&mut self) {
        self.movement(
            |app| {
                let last = app.rows.len().saturating_sub(1);
                app.selected.saturating_add(1).min(last)
            },
            |_, offset| offset.saturating_add(1),
        );
    }

    pub(crate) fn select_page_up(&mut self) {
        self.movement(
            |app| app.selected.saturating_sub(app.page()),
            |app, offset| offset.saturating_sub(app.panel.page()),
        );
    }

    pub(crate) fn select_page_down(&mut self) {
        self.movement(
            |app| {
                let last = app.rows.len().saturating_sub(1);
                app.selected.saturating_add(app.page()).min(last)
            },
            |app, offset| offset.saturating_add(app.panel.page()),
        );
    }

    pub(crate) fn select_first(&mut self) {
        self.movement(|_| 0, |_, _| 0);
    }

    pub(crate) fn select_last(&mut self) {
        // `usize::MAX` rather than the panel's last line: the panel clamps, and
        // asking it for its length here would be a second place that knows how
        // long a card is.
        self.movement(|app| app.rows.len().saturating_sub(1), |_, _| usize::MAX);
    }

    // The pointer family, from here to `scroll_panel_up`. These never consult
    // the focus — the pointer names the pane it is over — and so must not be
    // routed through `movement`.
    pub(crate) fn select_row(&mut self, index: usize) {
        if index >= self.rows.len() {
            return;
        }
        self.selected = index;
        self.moved();
    }

    pub(crate) fn select_next_by(&mut self, rows: usize) {
        let last = self.rows.len().saturating_sub(1);
        self.selected = self.selected.saturating_add(rows).min(last);
        self.moved();
    }

    pub(crate) fn select_previous_by(&mut self, rows: usize) {
        self.selected = self.selected.saturating_sub(rows);
        self.moved();
    }

    pub(crate) fn scroll_panel_down(&mut self, lines: usize) {
        self.panel
            .scroll_to(self.panel().scroll_offset().saturating_add(lines));
    }

    pub(crate) fn scroll_panel_up(&mut self, lines: usize) {
        self.panel
            .scroll_to(self.panel().scroll_offset().saturating_sub(lines));
    }

    // One key, two meanings: the caller supplies what the keystroke means to the
    // tree and what it means to the panel, and the focus picks. Neither runs
    // when the composer has the focus, which is how a movement key reaches the
    // text field without also moving the selection behind it.
    fn movement(
        &mut self,
        tree: impl FnOnce(&Self) -> usize,
        panel: impl FnOnce(&Self, usize) -> usize,
    ) {
        let focus = self.viewpoint.focus;
        if focus.drives_the_tree() {
            self.selected = tree(self);
            self.moved();
        } else if focus.drives_the_panel() {
            let offset = panel(self, self.panel().scroll_offset());
            self.panel.scroll_to(offset);
        }
    }

    const fn page(&self) -> usize {
        if self.viewpoint.viewport_height == 0 {
            1
        } else {
            self.viewpoint.viewport_height
        }
    }

    fn rescroll(&mut self) {
        self.scroll_offset = scroll_offset_for(
            self.rows.len(),
            self.viewpoint.viewport_height,
            self.selected,
            self.scroll_offset,
        );
    }

    fn reflow(&mut self) {
        let selected = self.rows.get(self.selected).map(|row| row.path.clone());
        let mut kept: Cow<'_, [Row]> = if self.viewpoint.show_files {
            Cow::Borrowed(&self.all_rows)
        } else {
            Cow::Owned(node_rows(&self.all_rows))
        };
        if self.viewpoint.scoped_only {
            kept = Cow::Owned(scoped_rows(&kept));
        }
        (self.rows, self.collapsible) = drawn_rows(&kept, &self.viewpoint.collapsed);
        self.selected = selected
            .and_then(|path| index_for(&self.rows, &path))
            .unwrap_or(0);
        self.rescroll();
    }

    fn moved(&mut self) {
        self.forget_last_keystroke();
        self.rescroll();
    }

    fn forget_last_keystroke(&mut self) {
        self.status.message = None;
    }

    #[must_use]
    pub(crate) fn label_for(&self, path: &Path) -> String {
        self.rows.first().map_or_else(
            || path.display().to_string(),
            |root| label(&root.path, path),
        )
    }

    pub(crate) fn toggle_collapsed(&mut self) {
        if !self.can_collapse(self.selected) {
            return;
        }
        let Some(row) = self.rows.get(self.selected) else {
            return;
        };

        let path = row.path.clone();
        if !self.viewpoint.collapsed.remove(&path) {
            self.viewpoint.collapsed.insert(path);
        }
        self.forget_last_keystroke();
        self.reflow();
    }

    pub(crate) fn scope_target(&mut self) -> Option<PathBuf> {
        let row = self.rows.get(self.selected)?;
        let path = row.path.clone();

        if row.is_file() {
            self.say(file_row_message(&self.label_for(&path)));
            return None;
        }
        Some(path)
    }

    pub(crate) fn view_target(&mut self) -> Option<PathBuf> {
        let row = self.rows.get(self.selected)?;
        let path = row.path.clone();

        if row.is_file() {
            return Some(path);
        }
        self.say(directory_view_message(&self.label_for(&path)));
        None
    }
}

#[must_use]
pub(crate) fn reseat_on(view: App, tree: &Tree) -> App {
    // Taken before anything is rebuilt, because it is the one fact about the
    // old view that the new rows cannot be asked for.
    let selected = view.selected_row().map(|row| row.path.clone());

    // Destructured rather than read field by field, and deliberately with no
    // `..`: this function is the one place that decides what survives a reload,
    // and a pattern that names every field is what makes adding a tenth one a
    // compile error here rather than a fact that quietly stops being carried.
    let App {
        // From the tree, every time. Whatever the old view held for these is a
        // description of a tree that has just been replaced.
        all_rows: _,
        rows: _,
        collapsible: _,
        // Re-derived below against the new rows rather than carried as they
        // stand: an index names whichever node now sits at that position, so
        // the selection travels by path, and the offset is put back only to be
        // brought into range by `rescroll`.
        selected: _,
        scroll_offset,
        // Carried whole. Three values, three moves — and nothing to forget
        // inside them, because a field added to any of the three is carried by
        // the move that already exists. The highlight is a fourth because it is
        // positions in the thread on the panel beside it: a tree read again off
        // disk is not a word of the conversation changed.
        viewpoint,
        status,
        panel,
        highlight,
    } = view;

    let mut reseated = App {
        all_rows: walk_of(tree),
        // Both derived by the `reflow` below, which is the one thing that knows
        // how the filters and the collapsed set turn a walk into a row list.
        rows: Vec::new(),
        collapsible: BTreeSet::new(),
        selected: 0,
        scroll_offset: 0,
        viewpoint,
        status,
        panel,
        highlight,
    };

    // Re-filter first, so the selection is looked up in the rows that will
    // actually be drawn rather than in the whole walk: what a hidden node falls
    // back to depends on what is on screen around it.
    reseated.reflow();
    reseated.selected = selected
        .and_then(|path| index_for(&reseated.rows, &path))
        .unwrap_or(0);
    // `reflow` has just scrolled the window to suit its own guess at the
    // selection; the offset the reader left is the one the window rule is owed,
    // so it goes back before that rule is applied to where the selection really
    // landed. A window that still holds the selection does not move at all.
    reseated.scroll_offset = scroll_offset;
    reseated.rescroll();

    reseated
}

fn walk_of(tree: &Tree) -> Vec<Row> {
    let mut rows = Vec::new();
    for (node, depth) in tree.walk() {
        let covered = node.is_covered();
        rows.push(Row::new(depth, node.path.clone(), covered).with_scope(node.scope.clone()));
        rows.extend(
            node.files
                .iter()
                .map(|file| Row::file(depth + 1, file.clone(), covered)),
        );
    }
    rows
}

fn node_rows(all: &[Row]) -> Vec<Row> {
    all.iter().filter(|row| !row.is_file()).cloned().collect()
}

fn drawn_rows(all: &[Row], collapsed: &BTreeSet<PathBuf>) -> (Vec<Row>, BTreeSet<PathBuf>) {
    let mut drawn = Vec::with_capacity(all.len());
    let mut collapsible = BTreeSet::new();
    // The depth of the collapsed node whose descendants are being skipped, if
    // any are.
    let mut hiding: Option<usize> = None;

    for (index, row) in all.iter().enumerate() {
        let holds = all
            .get(index + 1)
            .is_some_and(|next| next.depth > row.depth);
        if holds {
            collapsible.insert(row.path.clone());
        }
        if hiding.is_some_and(|depth| row.depth > depth) {
            continue;
        }
        hiding = (holds && collapsed.contains(&row.path)).then_some(row.depth);
        drawn.push(row.clone());
    }
    (drawn, collapsible)
}

fn scoped_rows(all: &[Row]) -> Vec<Row> {
    let mut kept = Vec::with_capacity(all.len());
    // The depth of the last row kept, which is still waiting for an ancestor,
    // if anything is.
    let mut needed: Option<usize> = None;

    for row in all.iter().rev() {
        if row.covered || needed.is_some_and(|depth| row.depth < depth) {
            needed = Some(row.depth);
            kept.push(row.clone());
        }
    }
    kept.reverse();
    kept
}

fn index_for(rows: &[Row], path: &Path) -> Option<usize> {
    rows.iter()
        .position(|row| row.path == path)
        .or_else(|| rows.iter().rposition(|row| path.starts_with(&row.path)))
}

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

pub(crate) fn label(root: &Path, directory: &Path) -> String {
    match to_manifest_path(root, directory) {
        Ok(relative) if relative != "." => relative,
        _ => directory.display().to_string(),
    }
}

fn directory_view_message(label: &str) -> String {
    format!("{label} is a directory — press v on one of its files to read it")
}

pub(crate) fn cut_at_cap_message() -> String {
    "— cut here: the file goes on past this line, and Warlock reads no further".to_owned()
}

fn no_document_message() -> String {
    "nothing has been read this session — press v on a file row, and there will be a document to swap to".to_owned()
}

fn file_row_message(label: &str) -> String {
    format!("{label} is a file — scopes are drawn on the directory holding it, not on a file")
}

#[cfg(test)]
#[path = "tests/app.rs"]
mod tests;
