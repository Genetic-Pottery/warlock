//! What the front end holds between keystrokes: a plain value, no terminal.
//!
//! The engine hands over a [`Tree`]; the screen wants a list of lines. So
//! `walk_of` flattens the depth-first walk once into `all_rows` — every node,
//! plus a row per file each node lists — and `reflow` derives the drawn `rows`
//! from it by three filters in a fixed order: files, then pacted-only, then
//! collapsed. `all_rows` is never filtered in place, so every view toggle is
//! the same re-derivation and turning one off puts the rest back untouched.
//!
//! Two consequences worth knowing before editing anything here. First, what the
//! reader has selected and which directories are collapsed are remembered as
//! *paths*, never as row indices: `reflow` and [`reseat_on`] both rebuild the
//! row list, and an index into the old list names a different node in the new
//! one. Second, `reflow` is meant to be the only thing that turns `all_rows`
//! into `rows`; `App::insert_file_row` is the single exception, and it has to
//! ask the same three filters in the same order or the spliced row and the next
//! `reflow` will disagree about what is on screen.

use std::borrow::Cow;
use std::collections::BTreeSet;
use std::mem;
use std::path::{Path, PathBuf};
use std::time::Instant;

use warlock_engine::{IntoDocument, NodeState, StateCounts, Tree, to_manifest_path};

use crate::panel::{Panel, Showing};

/// One line of the flattened tree.
///
/// A row owns its path and document rather than borrowing from the [`Tree`], so
/// [`App`] is a self-contained value with no lifetimes threading through the
/// event loop, and a key handler can refuse a keystroke from the row alone
/// without going back to the tree for a second copy of the same fact.
///
/// Most rows stand for a node. The rest stand for one of the files a node lists
/// (see [`Row::file`]): drawn like any other row, and nothing like one
/// otherwise — it documents nothing, holds nothing, and is counted nowhere.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Row {
    pub depth: usize,
    pub path: PathBuf,
    pub document: Option<PathBuf>,
    /// A file row carries the state of the directory holding it, which is what
    /// makes a file take its module's colour.
    pub state: NodeState,
    pub children: usize,
    pub file: bool,
    /// Set on the one file row that is its directory's own `WARLOCK.md`. Kept
    /// on the row because the comparison against the node's document cannot be
    /// made again once the tree is gone.
    pub document_row: bool,
    pub ignored: bool,
    pub scope: Option<String>,
}

impl Row {
    #[must_use]
    pub fn new(
        depth: usize,
        path: impl Into<PathBuf>,
        document: impl IntoDocument,
        state: NodeState,
    ) -> Self {
        Self {
            depth,
            path: path.into(),
            document: document.into_document(),
            state,
            children: 0,
            file: false,
            document_row: false,
            ignored: false,
            scope: None,
        }
    }

    #[must_use]
    pub fn file(depth: usize, path: impl Into<PathBuf>, state: NodeState) -> Self {
        Self {
            file: true,
            ..Self::new(depth, path, None, state)
        }
    }

    #[must_use]
    pub const fn with_child_count(mut self, children: usize) -> Self {
        self.children = children;
        self
    }

    #[must_use]
    pub const fn with_ignored(mut self, ignored: bool) -> Self {
        self.ignored = ignored;
        self
    }

    #[must_use]
    pub const fn with_document_row(mut self, document_row: bool) -> Self {
        self.document_row = document_row;
        self
    }

    #[must_use]
    pub fn with_scope(mut self, scope: Option<String>) -> Self {
        self.scope = scope;
        self
    }

    #[must_use]
    pub const fn has_children(&self) -> bool {
        self.children > 0
    }

    #[must_use]
    pub const fn is_file(&self) -> bool {
        self.file
    }

    #[must_use]
    pub const fn is_document(&self) -> bool {
        self.document_row
    }

    #[must_use]
    pub const fn is_ignored(&self) -> bool {
        self.ignored
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PactToggle {
    pub path: PathBuf,
    pub pacted: bool,
}

/// What the pact key would do, asked without doing it.
///
/// [`App::toggle_pact`] is this decision plus its effect; keeping the decision
/// separate is what lets a caller show the refusal — or count the nodes a pact
/// would reach — without moving any state.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum PactIntent {
    Toggles(PactToggle),
    Refused(String),
    NoRow,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Run {
    Pact,
    Refresh,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct InFlight {
    path: PathBuf,
    position: usize,
    // Highest `position` this run has been told about. Nodes are not
    // necessarily reported in order, so the header counts by this and the
    // message line by `position`; a header that could go backwards would read
    // as work being undone.
    reached: usize,
    total: usize,
    run: Run,
}

/// The run in flight, as the header draws it. [`RunHeader::position`] is the
/// furthest node this run has reached, not the one it is on.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RunHeader {
    run: Run,
    directory: String,
    position: usize,
    total: usize,
}

impl RunHeader {
    #[must_use]
    pub const fn run(&self) -> Run {
        self.run
    }

    #[must_use]
    pub fn directory(&self) -> &str {
        &self.directory
    }

    #[must_use]
    pub const fn position(&self) -> usize {
        self.position
    }

    #[must_use]
    pub const fn total(&self) -> usize {
        self.total
    }
}

/// Which of the screen's three places the movement keys drive.
///
/// [`Focus::next`] cycles through all three unconditionally; whether the
/// composer is reachable is [`App::toggle_focus`]'s question, not this type's,
/// because only the app can see the panel. The pointer methods
/// ([`App::select_row`], [`App::scroll_panel_down`] and their neighbours) name
/// their pane instead and never consult the focus at all.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum Focus {
    #[default]
    Tree,
    Panel,
    Composer,
}

impl Focus {
    #[must_use]
    pub const fn next(self) -> Self {
        match self {
            Self::Tree => Self::Panel,
            Self::Panel => Self::Composer,
            Self::Composer => Self::Tree,
        }
    }

    #[must_use]
    pub const fn drives_the_tree(self) -> bool {
        match self {
            Self::Tree => true,
            Self::Panel | Self::Composer => false,
        }
    }

    #[must_use]
    pub const fn drives_the_panel(self) -> bool {
        match self {
            Self::Panel => true,
            Self::Tree | Self::Composer => false,
        }
    }
}

/// Which sigils the machine holds, for the one line the chrome draws about it.
///
/// `Nothing` and `Unknown` are not the same answer and must not be merged:
/// holding no sigil draws no line at all, while failing to find out draws
/// "holding unknown". [`Sigils::held`] folds an empty list into `Nothing`, so
/// `Held` is never empty and the line is never a dangling "holding".
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub enum Sigils {
    #[default]
    Nothing,
    Held(Vec<String>),
    Unknown,
}

impl Sigils {
    #[must_use]
    pub fn held(sigils: impl IntoIterator<Item = impl Into<String>>) -> Self {
        let sigils: Vec<String> = sigils.into_iter().map(Into::into).collect();
        if sigils.is_empty() {
            return Self::Nothing;
        }
        Self::Held(sigils)
    }

    #[must_use]
    pub fn as_slice(&self) -> &[String] {
        match self {
            Self::Nothing | Self::Unknown => &[],
            Self::Held(sigils) => sigils,
        }
    }

    #[must_use]
    pub fn line(&self) -> Option<String> {
        match self {
            Self::Nothing => None,
            Self::Held(sigils) => Some(format!(
                "holding {}",
                sigils
                    .iter()
                    .map(|sigil| format!("`{sigil}`"))
                    .collect::<Vec<_>>()
                    .join(", ")
            )),
            Self::Unknown => Some("holding unknown".to_owned()),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Chrome {
    header: String,
    sigils: Sigils,
}

impl Chrome {
    /// The header is empty for a root that *is* the repo root, so the title bar
    /// draws no redundant "." — and falls back to the absolute path when `root`
    /// turns out not to be under `repo_root` at all.
    #[must_use]
    pub fn of(repo_root: impl AsRef<Path>, root: impl AsRef<Path>) -> Self {
        let root = root.as_ref();
        let header = match to_manifest_path(repo_root, root) {
            Ok(relative) if relative == "." => String::new(),
            Ok(relative) => relative,
            Err(_) => root.display().to_string(),
        };
        Self {
            header,
            sigils: Sigils::Nothing,
        }
    }

    #[must_use]
    pub fn with_sigils(mut self, sigils: Sigils) -> Self {
        self.sigils = sigils;
        self
    }

    #[must_use]
    pub fn header(&self) -> &str {
        &self.header
    }

    #[must_use]
    pub const fn sigils(&self) -> &Sigils {
        &self.sigils
    }
}

/// The whole of the front end's state, and none of its plumbing.
///
/// `all_rows` is the flattening of the tree and `rows` is the part of it being
/// drawn; `selected` and `scroll_offset` index `rows`, so anything that
/// rebuilds `rows` owes both of them a re-derivation. The three groups below
/// are grouped so that [`reseat_on`] can carry them across a reload by moving
/// three fields rather than by copying twenty.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct App {
    all_rows: Vec<Row>,
    rows: Vec<Row>,
    collapsible: BTreeSet<PathBuf>,
    counts: StateCounts,
    selected: usize,
    scroll_offset: usize,
    viewpoint: Viewpoint,
    status: Status,
    panel: Panel,
}

// What the reader is looking at rather than what is being looked at, which is
// why a reload carries this whole and rebuilds everything above it.
#[derive(Debug, Clone, Default, PartialEq)]
struct Viewpoint {
    collapsed: BTreeSet<PathBuf>,
    pacted_only: bool,
    show_files: bool,
    viewport_height: usize,
    focus: Focus,
}

#[derive(Debug, Clone, Default, PartialEq)]
struct Status {
    // Belongs to the last keystroke: set by whatever refused it, dropped by the
    // next movement (`forget_last_keystroke`). Deliberately separate from
    // `in_flight`, which outlives any number of keystrokes and must survive
    // them.
    message: Option<String>,
    in_flight: Option<InFlight>,
    pact_refused: bool,
    mouse_captured: bool,
}

impl App {
    #[must_use]
    pub fn from_tree(tree: &Tree) -> Self {
        Self::from_rows(walk_of(tree)).with_counts(tree.counts())
    }

    /// Builds an app from rows alone, leaving the tally at zero. Tests use it;
    /// [`App::from_tree`] is the path that also has counts to give. A zero tally
    /// never described these rows, which is why [`App::set_subtree_state`]
    /// declines to move it rather than counting up from nothing.
    #[must_use]
    pub fn from_rows(rows: Vec<Row>) -> Self {
        // Every field named, and no `..Default::default()` anywhere in this
        // file: a struct literal that names all of them is what turns a
        // twenty-third field into a compile error here and in [`reseat_on`],
        // rather than into a field somebody forgot to carry. See the note on
        // [`reseat_on`].
        let mut app = Self {
            rows: rows.clone(),
            all_rows: rows,
            collapsible: BTreeSet::new(),
            counts: StateCounts::default(),
            selected: 0,
            scroll_offset: 0,
            viewpoint: Viewpoint {
                collapsed: BTreeSet::new(),
                pacted_only: false,
                show_files: false,
                viewport_height: 0,
                focus: Focus::Tree,
            },
            status: Status {
                message: None,
                in_flight: None,
                pact_refused: false,
                mouse_captured: false,
            },
            panel: Panel::default(),
        };
        // The rows handed over may hold file rows, which the file toggle starts
        // off over, so the drawn list is derived rather than assumed even here.
        app.reflow();
        app
    }

    #[must_use]
    pub fn with_collapsed(
        mut self,
        collapsed: impl IntoIterator<Item = impl Into<PathBuf>>,
    ) -> Self {
        self.viewpoint.collapsed = collapsed.into_iter().map(Into::into).collect();
        self.reflow();
        self
    }

    #[must_use]
    pub const fn with_counts(mut self, counts: StateCounts) -> Self {
        self.counts = counts;
        self
    }

    #[must_use]
    pub fn message(&self) -> Option<&str> {
        self.status.message.as_deref()
    }

    pub fn set_message(&mut self, message: impl Into<String>) {
        self.status.message = Some(message.into());
    }

    pub fn set_pact_in_flight(&mut self, path: impl Into<PathBuf>, position: usize, total: usize) {
        self.set_run_in_flight(Run::Pact, path, position, total);
    }

    pub fn set_run_in_flight(
        &mut self,
        run: Run,
        path: impl Into<PathBuf>,
        position: usize,
        total: usize,
    ) {
        let reached = self
            .status
            .in_flight
            .as_ref()
            .map_or(position, |before| before.reached.max(position));
        self.status.in_flight = Some(InFlight {
            path: path.into(),
            position,
            reached,
            total,
            run,
        });
    }

    pub fn clear_pact_in_flight(&mut self) {
        self.status.in_flight = None;
    }

    pub fn set_pact_refused(&mut self) {
        self.status.pact_refused = true;
    }

    #[must_use]
    pub const fn is_pacting(&self) -> bool {
        self.status.in_flight.is_some()
    }

    #[must_use]
    pub fn is_in_flight(&self, path: &Path) -> bool {
        self.status
            .in_flight
            .as_ref()
            .is_some_and(|in_flight| in_flight.path == path)
    }

    /// True for the directory being run *and* for the file rows directly inside
    /// it, so a run marks the rows a reader can see it working on. Only one
    /// level down: a file under a subdirectory belongs to that subdirectory's
    /// own turn.
    #[must_use]
    pub fn in_flight_covers(&self, row: &Row) -> bool {
        self.status.in_flight.as_ref().is_some_and(|in_flight| {
            in_flight.path == row.path
                || (row.is_file() && row.path.parent() == Some(in_flight.path.as_path()))
        })
    }

    #[must_use]
    pub fn pact_line(&self) -> Option<String> {
        self.status.in_flight.as_ref().map(|in_flight| {
            let label = self.label_for(&in_flight.path);
            let line = match in_flight.run {
                Run::Pact => pacting_message(&label, in_flight.position, in_flight.total),
                Run::Refresh => refreshing_message(&label, in_flight.position, in_flight.total),
            };
            if self.status.pact_refused {
                already_running_message(&line)
            } else {
                line
            }
        })
    }

    #[must_use]
    pub fn run_header(&self) -> Option<RunHeader> {
        self.status.in_flight.as_ref().map(|in_flight| RunHeader {
            run: in_flight.run,
            directory: self.label_for(&in_flight.path),
            position: in_flight.reached,
            total: in_flight.total,
        })
    }

    /// A run switches the panel to its account only when there is nothing there
    /// to lose. A reader part-way through a document keeps it: the account has
    /// been opened either way and the swap key reaches it.
    pub fn start_account(&mut self, at: Instant) {
        if !self.panel.has_content() {
            self.panel.show(Showing::Account);
            self.rescue_focus();
        }
        self.panel.open_account(at);
    }

    pub fn show_document(&mut self, lines: impl IntoIterator<Item = impl Into<String>>, cut: bool) {
        self.panel.show_document(lines, cut);
        self.rescue_focus();
    }

    pub fn swap_card(&mut self) {
        let Some(card) = self.panel.next_card() else {
            self.set_message(no_document_message());
            return;
        };
        self.panel.show(card);
        self.rescue_focus();
    }

    #[must_use]
    pub const fn panel(&self) -> &Panel {
        &self.panel
    }

    pub const fn panel_mut(&mut self) -> &mut Panel {
        &mut self.panel
    }

    /// Rolls the view back to an earlier copy — but keeps the live panel, since
    /// an account or a conversation is a record of what happened and rolling it
    /// back would discard it at the moment the reader turned to read it.
    pub fn restore_from(&mut self, view: Self) {
        let panel = mem::take(&mut self.panel);
        *self = view;
        self.panel = panel;
        // The focus comes from `view` and the card showing comes from the panel
        // that stayed, so this is the one place the two can arrive out of step:
        // a copy taken with the composer focused, put back over a panel that has
        // a document up since. See `rescue_focus`.
        self.rescue_focus();
    }

    #[must_use]
    pub fn rows(&self) -> &[Row] {
        &self.rows
    }

    #[must_use]
    pub const fn collapsed(&self) -> &BTreeSet<PathBuf> {
        &self.viewpoint.collapsed
    }

    #[must_use]
    pub fn is_collapsed(&self, path: impl AsRef<Path>) -> bool {
        self.viewpoint.collapsed.contains(path.as_ref())
    }

    /// Asks whether the row has anything *drawn* under it, not whether the node
    /// has children: a directory whose children are all filtered away collapses
    /// onto nothing, so the marker is not offered. `collapsible` is rebuilt by
    /// `drawn_rows` on every `reflow` for that reason.
    #[must_use]
    pub fn can_collapse(&self, index: usize) -> bool {
        self.rows
            .get(index)
            .is_some_and(|row| self.collapsible.contains(&row.path))
    }

    #[must_use]
    pub const fn pacted_only(&self) -> bool {
        self.viewpoint.pacted_only
    }

    pub fn toggle_pacted_only(&mut self) {
        self.viewpoint.pacted_only = !self.viewpoint.pacted_only;
        self.forget_last_keystroke();
        self.reflow();
    }

    #[must_use]
    pub const fn show_files(&self) -> bool {
        self.viewpoint.show_files
    }

    pub fn toggle_files(&mut self) {
        self.viewpoint.show_files = !self.viewpoint.show_files;
        self.forget_last_keystroke();
        self.reflow();
    }

    #[must_use]
    pub const fn counts(&self) -> StateCounts {
        self.counts
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    #[must_use]
    pub const fn selected(&self) -> usize {
        self.selected
    }

    #[must_use]
    pub fn selected_row(&self) -> Option<&Row> {
        self.rows.get(self.selected)
    }

    #[must_use]
    pub const fn scroll_offset(&self) -> usize {
        self.scroll_offset
    }

    #[must_use]
    pub const fn viewport_height(&self) -> usize {
        self.viewpoint.viewport_height
    }

    pub fn set_viewport_height(&mut self, height: u16) {
        self.viewpoint.viewport_height = usize::from(height);
        self.rescroll();
    }

    #[must_use]
    pub const fn mouse_captured(&self) -> bool {
        self.status.mouse_captured
    }

    pub const fn set_mouse_captured(&mut self, captured: bool) {
        self.status.mouse_captured = captured;
    }

    #[must_use]
    pub const fn focus(&self) -> Focus {
        self.viewpoint.focus
    }

    pub const fn toggle_focus(&mut self) {
        let next = self.viewpoint.focus.next();
        self.viewpoint.focus = match next {
            Focus::Composer if !self.panel().composer_showable() => next.next(),
            Focus::Tree | Focus::Panel | Focus::Composer => next,
        };
    }

    pub const fn set_focus(&mut self, focus: Focus) {
        self.viewpoint.focus = focus;
        self.rescue_focus();
    }

    // The composer can stop being showable under a focus already resting on it
    // — a card swap, a document arriving, a view restored over a live panel — so
    // every path that sets a focus or changes what the panel shows ends here
    // rather than trusting the focus it was handed.
    const fn rescue_focus(&mut self) {
        match self.viewpoint.focus {
            Focus::Composer if !self.panel().composer_showable() => {
                self.viewpoint.focus = Focus::Panel;
            }
            Focus::Tree | Focus::Panel | Focus::Composer => {}
        }
    }

    pub fn select_previous(&mut self) {
        self.movement(
            |app| app.selected.saturating_sub(1),
            |_, offset| offset.saturating_sub(1),
        );
    }

    pub fn select_next(&mut self) {
        self.movement(
            |app| {
                let last = app.rows.len().saturating_sub(1);
                app.selected.saturating_add(1).min(last)
            },
            |_, offset| offset.saturating_add(1),
        );
    }

    pub fn select_page_up(&mut self) {
        self.movement(
            |app| app.selected.saturating_sub(app.page()),
            |app, offset| offset.saturating_sub(app.panel_page()),
        );
    }

    pub fn select_page_down(&mut self) {
        self.movement(
            |app| {
                let last = app.rows.len().saturating_sub(1);
                app.selected.saturating_add(app.page()).min(last)
            },
            |app, offset| offset.saturating_add(app.panel_page()),
        );
    }

    pub fn select_first(&mut self) {
        self.movement(|_| 0, |_, _| 0);
    }

    pub fn select_last(&mut self) {
        // `usize::MAX` rather than the panel's last line: the panel clamps, and
        // asking it for its length here would be a second place that knows how
        // long a card is.
        self.movement(|app| app.rows.len().saturating_sub(1), |_, _| usize::MAX);
    }

    // The pointer family, from here to `scroll_panel_up`. These never consult
    // the focus — the pointer names the pane it is over — and so must not be
    // routed through `movement`.
    pub fn select_row(&mut self, index: usize) {
        if index >= self.rows.len() {
            return;
        }
        self.selected = index;
        self.moved();
    }

    pub fn select_next_by(&mut self, rows: usize) {
        let last = self.rows.len().saturating_sub(1);
        self.selected = self.selected.saturating_add(rows).min(last);
        self.moved();
    }

    pub fn select_previous_by(&mut self, rows: usize) {
        self.selected = self.selected.saturating_sub(rows);
        self.moved();
    }

    pub fn scroll_panel_down(&mut self, lines: usize) {
        self.scroll_panel_to(self.panel().scroll_offset().saturating_add(lines));
    }

    pub fn scroll_panel_up(&mut self, lines: usize) {
        self.scroll_panel_to(self.panel().scroll_offset().saturating_sub(lines));
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
            self.scroll_panel_to(offset);
        }
    }

    fn scroll_panel_to(&mut self, offset: usize) {
        self.panel.scroll_to(offset);
    }

    const fn page(&self) -> usize {
        if self.viewpoint.viewport_height == 0 {
            1
        } else {
            self.viewpoint.viewport_height
        }
    }

    const fn panel_page(&self) -> usize {
        self.panel.page()
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
        let kept: Cow<'_, [Row]> = if self.viewpoint.show_files {
            Cow::Borrowed(&self.all_rows)
        } else {
            Cow::Owned(node_rows(&self.all_rows))
        };
        let kept: Cow<'_, [Row]> = if self.viewpoint.pacted_only {
            Cow::Owned(pacted_rows(&kept))
        } else {
            kept
        };
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
        self.status.pact_refused = false;
    }

    #[must_use]
    pub fn label_for(&self, path: &Path) -> String {
        match self
            .rows
            .first()
            .map(|root| to_manifest_path(&root.path, path))
        {
            Some(Ok(relative)) if relative != "." => relative,
            _ => path.display().to_string(),
        }
    }

    pub fn toggle_collapsed(&mut self) {
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

    pub fn toggle_pact(&mut self) -> Option<PactToggle> {
        match self.pact_intent() {
            PactIntent::Toggles(toggle) => {
                self.apply_toggle(&toggle);
                Some(toggle)
            }
            PactIntent::Refused(message) => {
                self.status.message = Some(message);
                None
            }
            PactIntent::NoRow => None,
        }
    }

    #[must_use]
    pub fn pact_intent(&self) -> PactIntent {
        let Some(row) = self.rows.get(self.selected) else {
            return PactIntent::NoRow;
        };
        let path = row.path.clone();
        if row.is_file() {
            return PactIntent::Refused(file_row_message(&self.label_for(&path)));
        }
        if row.is_ignored() {
            return PactIntent::Refused(ignored_row_message(&self.label_for(&path)));
        }
        PactIntent::Toggles(PactToggle {
            pacted: !row.state.is_pacted(),
            path,
        })
    }

    #[must_use]
    pub fn pact_reach(&self) -> Option<PactToggle> {
        let row = self.rows.get(self.selected)?;
        (!row.is_file()).then(|| PactToggle {
            path: row.path.clone(),
            pacted: !row.state.is_pacted(),
        })
    }

    pub fn apply_toggle(&mut self, toggle: &PactToggle) {
        self.set_subtree_state(
            &toggle.path,
            if toggle.pacted {
                NodeState::PactedStale
            } else {
                NodeState::Unpacted
            },
        );
        self.status.message =
            (!toggle.pacted).then(|| left_on_disk_message(&self.label_for(&toggle.path)));
    }

    pub fn refresh(&mut self) -> Option<PathBuf> {
        let row = self.rows.get(self.selected)?;
        let path = row.path.clone();
        let state = row.state;

        if row.is_file() {
            self.status.message = Some(file_row_message(&self.label_for(&path)));
            return None;
        }
        match state {
            NodeState::Unpacted => {
                self.status.message = Some(unpacted_message(&self.label_for(&path)));
                None
            }
            NodeState::PactedFresh => {
                self.status.message = Some(already_fresh_message(&self.label_for(&path)));
                None
            }
            NodeState::PactedStale => {
                self.status.message = None;
                Some(path)
            }
        }
    }

    pub fn scope_target(&mut self) -> Option<PathBuf> {
        let row = self.rows.get(self.selected)?;
        let path = row.path.clone();
        let state = row.state;

        if row.is_file() {
            self.status.message = Some(file_row_message(&self.label_for(&path)));
            return None;
        }
        match state {
            NodeState::Unpacted => {
                self.status.message = Some(unpacted_scope_message(&self.label_for(&path)));
                None
            }
            // Fresh or stale, the pact is there and so is the entry the scope
            // is written on. Not a line of state is touched on the way out.
            NodeState::PactedFresh | NodeState::PactedStale => Some(path),
        }
    }

    pub fn view_target(&mut self) -> Option<PathBuf> {
        let row = self.rows.get(self.selected)?;
        let path = row.path.clone();
        let document = row.document.clone();

        if row.is_file() {
            return Some(path);
        }
        self.status.message = Some(document.map_or_else(
            || undocumented_view_message(&self.label_for(&path)),
            |document| directory_view_message(&self.label_for(&path), &self.label_for(&document)),
        ));
        None
    }

    pub fn set_subtree_state(&mut self, path: impl AsRef<Path>, state: NodeState) {
        let path = path.as_ref();

        // The tally first, off the unfiltered list, so that every node is
        // counted once whether or not it is drawn — and before the states are
        // written, since what moves out of a field is what each node is now.
        for row in &self.all_rows {
            if row.is_file() || row.state == state || !moves_with_subtree(row, path, state) {
                continue;
            }
            // Both halves of the move happen together or neither does. An app
            // told rows but never told a tally (see `App::from_rows`) holds
            // zeroes that never described those rows, and nudging one field up
            // while the other cannot come down would turn a tally that is
            // merely absent into one that counts a node that is not there.
            let old = count_mut(&mut self.counts, row.state);
            if let Some(fewer) = old.checked_sub(1) {
                *old = fewer;
                *count_mut(&mut self.counts, state) += 1;
            }
        }

        paint_subtree(&mut self.all_rows, path, state);
        paint_subtree(&mut self.rows, path, state);
    }

    pub fn insert_file_row(&mut self, path: impl AsRef<Path>) {
        let path = path.as_ref();
        // One row per path, so a second delivery for a directory already
        // holding its document is not news.
        if self.all_rows.iter().any(|row| row.path == path) {
            return;
        }
        let Some(directory) = path.parent() else {
            return;
        };
        // The directory's own row, never a file row that happens to be spelled
        // like one: a file holds nothing.
        let Some(index) = self
            .all_rows
            .iter()
            .position(|row| !row.is_file() && row.path == directory)
        else {
            return;
        };

        let holder = &self.all_rows[index];
        let depth = holder.depth + 1;
        let row = Row::file(depth, path, holder.state)
            .with_ignored(holder.is_ignored())
            .with_document_row(true);
        let at = file_row_position(&self.all_rows, index, depth, path);
        self.all_rows.insert(at, row.clone());

        // The drawn half, which is conditional: `rows` holds an ordinary file
        // row only while the toggle is on — but it holds a *document* row either
        // way, because `node_rows` keeps each directory's own `WARLOCK.md` when
        // files are hidden, so the row spliced here is drawn then and there in
        // the view warlock opens on rather than waiting for `f`. It holds a file
        // under an unpacted directory not at all under the pacted-only filter,
        // and holds nothing at all under a directory that is collapsed or
        // filtered away. Asking those three questions in the shape the three
        // filters ask them is what keeps this insertion and the next `reflow`
        // from disagreeing about what is on screen.
        if (!self.viewpoint.show_files && !row.is_document())
            || (self.viewpoint.pacted_only && !row.state.is_pacted())
            || self.viewpoint.collapsed.contains(directory)
        {
            return;
        }
        // A directory whose own row is not drawn — filtered away, or under
        // something collapsed — has nothing on screen for this to go under.
        let Some(index) = self
            .rows
            .iter()
            .position(|drawn| !drawn.is_file() && drawn.path == directory)
        else {
            return;
        };
        let at = file_row_position(&self.rows, index, depth, path);
        self.rows.insert(at, row);
        // `collapsible` is deliberately left alone. A directory that held
        // nothing and has just gained its first drawn row is collapsible now and
        // will not be said to be until the next `reflow`, which any keystroke
        // that changes the view runs; the cost of being wrong is a collapse key
        // that does nothing on one row for one frame, and the fix would be a
        // second place the drawn-and-collapsed rule is written down.

        // Both indices are into `rows`, so a row landing at or above one of them
        // moves it: the selection would otherwise name the row below the one it
        // named, and the window would slide up a line for a reason the reader
        // did nothing to cause.
        if at <= self.selected {
            self.selected += 1;
        }
        if at <= self.scroll_offset {
            self.scroll_offset += 1;
        }
        self.rescroll();
    }
}

fn file_row_position(rows: &[Row], directory: usize, depth: usize, path: &Path) -> usize {
    let mut at = directory + 1;
    for row in rows.iter().skip(directory + 1) {
        if !row.is_file() || row.depth != depth || row.path.as_path() > path {
            break;
        }
        at += 1;
    }
    at
}

#[must_use]
pub fn reseat_on(view: &App, tree: &Tree) -> App {
    // Taken before anything is rebuilt, because it is the one fact about the
    // old view that the new rows cannot be asked for.
    let selected = view.selected_row().map(|row| row.path.clone());

    // Destructured rather than read field by field, and deliberately with no
    // `..`: this function is the one place that decides what survives a reload,
    // and a pattern that names every field is what makes adding a tenth one a
    // compile error here rather than a fact that quietly stops being carried.
    // That used to be twenty assignments nobody could check, and the proof it
    // could not be checked is that the header was among them — carried here and
    // then immediately recomputed by the only caller.
    let App {
        // From the tree, every time. Whatever the old view held for these is a
        // description of a tree that has just been replaced.
        all_rows: _,
        rows: _,
        collapsible: _,
        counts: _,
        // Re-derived below against the new rows rather than carried as they
        // stand: an index names whichever node now sits at that position, so
        // the selection travels by path, and the offset is put back only to be
        // brought into range by `rescroll`.
        selected: _,
        scroll_offset,
        // Carried whole. Three values, three moves — and nothing to forget
        // inside them, because a field added to any of the three is carried by
        // the move that already exists.
        viewpoint,
        status,
        panel,
    } = view;

    let mut reseated = App {
        all_rows: walk_of(tree),
        // Both derived by the `reflow` below, which is the one thing that knows
        // how the filters and the collapsed set turn a walk into a row list.
        rows: Vec::new(),
        collapsible: BTreeSet::new(),
        counts: tree.counts(),
        selected: 0,
        scroll_offset: 0,
        viewpoint: viewpoint.clone(),
        status: status.clone(),
        panel: panel.clone(),
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
    reseated.scroll_offset = *scroll_offset;
    reseated.rescroll();

    reseated
}

fn walk_of(tree: &Tree) -> Vec<Row> {
    let mut rows = Vec::new();
    for (node, depth) in tree.walk() {
        rows.push(
            Row::new(depth, node.path.clone(), node.document.clone(), node.state)
                .with_child_count(node.children.len())
                .with_ignored(node.is_ignored())
                .with_scope(node.scope.clone()),
        );
        rows.extend(node.files.iter().map(|file| {
            Row::file(depth + 1, file.clone(), node.state)
                .with_ignored(node.is_ignored())
                .with_document_row(node.document.as_deref() == Some(file.as_path()))
        }));
    }
    rows
}

fn node_rows(all: &[Row]) -> Vec<Row> {
    all.iter()
        .filter(|row| !row.is_file() || row.is_document())
        .cloned()
        .collect()
}

fn paint_subtree(rows: &mut [Row], path: &Path, state: NodeState) {
    for row in rows {
        if moves_with_subtree(row, path, state) {
            row.state = state;
        }
    }
}

fn moves_with_subtree(row: &Row, root: &Path, state: NodeState) -> bool {
    in_subtree(&row.path, root) && !(state.is_pacted() && row.is_ignored())
}

fn in_subtree(path: &Path, root: &Path) -> bool {
    path.starts_with(root)
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

fn left_on_disk_message(label: &str) -> String {
    format!(
        "{label} is no longer pacted — every WARLOCK.md in it was left on disk, \
         untouched"
    )
}

fn pacting_message(label: &str, position: usize, total: usize) -> String {
    format!("pacting {label} ({position}/{total})")
}

fn refreshing_message(label: &str, position: usize, total: usize) -> String {
    format!("refreshing {label} ({position}/{total})")
}

fn already_fresh_message(label: &str) -> String {
    format!("{label} is already fresh — there is nothing under it to describe again")
}

fn unpacted_message(label: &str) -> String {
    format!("{label} is not pacted — press p to pact it, and there will be something to refresh")
}

fn unpacted_scope_message(label: &str) -> String {
    format!("{label} is not pacted — press p to pact it, and there will be a pact to scope")
}

fn directory_view_message(label: &str, document: &str) -> String {
    format!("{label} is a directory — press v on {document}, the row beneath it, to read it")
}

fn undocumented_view_message(label: &str) -> String {
    format!(
        "{label} is a directory with no WARLOCK.md — press p to pact it, and there will be a document to read"
    )
}

pub(crate) fn cut_at_cap_message() -> String {
    "— cut here: the file goes on past this line, and Warlock reads no further".to_owned()
}

fn no_document_message() -> String {
    "nothing has been read this session — press v on a file row, and there will be a document to swap to".to_owned()
}

fn already_running_message(pacting: &str) -> String {
    format!("{pacting} — already running")
}

fn file_row_message(label: &str) -> String {
    format!("{label} is a file — pacts are made with the directory holding it, not with a file")
}

fn ignored_row_message(label: &str) -> String {
    format!(
        "{label} is kept out by .warlockignore — Warlock covers nothing in there, so there is nothing to pact"
    )
}

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
    use std::time::{Duration, Instant};

    use warlock_engine::{Node, NodeState, StateCounts, Tree};

    use super::{
        App, Chrome, Focus, PactIntent, PactToggle, Row, Run, Showing, Sigils, cut_at_cap_message,
        ignored_row_message, no_document_message, reseat_on, scroll_offset_for,
    };
    use crate::account::{Account, Line, Outcome};
    use crate::claude::Activity;
    use crate::fixture;
    use crate::panel::Mode;
    use crate::panel::panel_offset_for;
    use crate::thread::Ending;

    const MANY: usize = 20;
    const WINDOW: u16 = 5;

    type Movement = fn(&mut App);

    fn three_rows() -> Vec<Row> {
        vec![
            Row::new(0, "repo", "repo/WARLOCK.md", NodeState::PactedStale),
            Row::new(
                1,
                "repo/crates",
                "repo/crates/WARLOCK.md",
                NodeState::PactedFresh,
            ),
            Row::new(
                1,
                "repo/assets",
                "repo/assets/WARLOCK.md",
                NodeState::Unpacted,
            ),
        ]
    }

    fn rooted_rows() -> Vec<Row> {
        vec![
            Row::new(0, "/repo", "/repo/WARLOCK.md", NodeState::PactedStale).with_child_count(1),
            Row::new(
                1,
                "/repo/crates",
                "/repo/crates/WARLOCK.md",
                NodeState::PactedStale,
            )
            .with_child_count(1),
            Row::new(
                2,
                "/repo/crates/warlock-engine",
                "/repo/crates/warlock-engine/WARLOCK.md",
                NodeState::PactedFresh,
            ),
        ]
    }

    fn tally(app: &App) -> StateCounts {
        let mut counts = StateCounts::default();
        for row in app.rows().iter().filter(|row| !row.is_file()) {
            match row.state {
                NodeState::Unpacted => counts.unpacted += 1,
                NodeState::PactedStale => counts.pacted_stale += 1,
                NodeState::PactedFresh => counts.pacted_fresh += 1,
            }
        }
        counts
    }

    fn many_rows(count: usize) -> Vec<Row> {
        (0..count)
            .map(|index| {
                Row::new(
                    1,
                    format!("repo/module{index}"),
                    format!("repo/module{index}/WARLOCK.md"),
                    NodeState::Unpacted,
                )
            })
            .collect()
    }

    fn scrolled_to(selected: usize) -> App {
        let mut app = App::from_rows(many_rows(MANY));
        app.set_viewport_height(WINDOW);
        while app.selected() < selected {
            app.select_next();
        }
        app
    }

    fn drawn(app: &App) -> Vec<String> {
        app.rows()
            .iter()
            .map(|row| row.path.to_string_lossy().into_owned())
            .collect()
    }

    fn paths_and_depths(rows: &[Row]) -> Vec<(String, usize)> {
        rows.iter()
            .map(|row| (row.path.to_string_lossy().into_owned(), row.depth))
            .collect()
    }

    fn node_paths(app: &App) -> Vec<String> {
        app.rows()
            .iter()
            .filter(|row| !row.is_file())
            .map(|row| row.path.to_string_lossy().into_owned())
            .collect()
    }

    fn states(app: &App) -> Vec<(&str, NodeState)> {
        app.rows()
            .iter()
            .map(|row| (row.path.to_str().expect("ascii path"), row.state))
            .collect()
    }

    fn whole_fixture() -> Vec<String> {
        vec![
            "warlock".to_owned(),
            "warlock/WARLOCK.md".to_owned(),
            "warlock/crates".to_owned(),
            "warlock/crates/engine".to_owned(),
            "warlock/crates/engine/WARLOCK.md".to_owned(),
            "warlock/crates/tui".to_owned(),
            "warlock/crates/tui/WARLOCK.md".to_owned(),
            "warlock/assets".to_owned(),
            "warlock/assets/WARLOCK.md".to_owned(),
        ]
    }

    fn pacted_fixture() -> Vec<String> {
        vec![
            "warlock".to_owned(),
            "warlock/WARLOCK.md".to_owned(),
            "warlock/crates".to_owned(),
            "warlock/crates/engine".to_owned(),
            "warlock/crates/engine/WARLOCK.md".to_owned(),
            "warlock/crates/tui".to_owned(),
            "warlock/crates/tui/WARLOCK.md".to_owned(),
        ]
    }

    fn whole_fixture_after_a_run() -> Vec<String> {
        vec![
            "warlock".to_owned(),
            "warlock/WARLOCK.md".to_owned(),
            "warlock/crates".to_owned(),
            "warlock/crates/WARLOCK.md".to_owned(),
            "warlock/crates/engine".to_owned(),
            "warlock/crates/engine/WARLOCK.md".to_owned(),
            "warlock/crates/tui".to_owned(),
            "warlock/crates/tui/WARLOCK.md".to_owned(),
            "warlock/assets".to_owned(),
            "warlock/assets/WARLOCK.md".to_owned(),
        ]
    }

    fn collapsed_over_crates() -> Vec<String> {
        vec![
            "warlock".to_owned(),
            "warlock/WARLOCK.md".to_owned(),
            "warlock/crates".to_owned(),
            "warlock/assets".to_owned(),
            "warlock/assets/WARLOCK.md".to_owned(),
        ]
    }

    fn whole_fixture_with_files() -> Vec<String> {
        vec![
            "warlock".to_owned(),
            "warlock/README.md".to_owned(),
            "warlock/WARLOCK.md".to_owned(),
            "warlock/crates".to_owned(),
            "warlock/crates/engine".to_owned(),
            "warlock/crates/engine/Cargo.toml".to_owned(),
            "warlock/crates/engine/WARLOCK.md".to_owned(),
            "warlock/crates/tui".to_owned(),
            "warlock/crates/tui/WARLOCK.md".to_owned(),
            "warlock/assets".to_owned(),
            "warlock/assets/WARLOCK.md".to_owned(),
            "warlock/assets/logo.svg".to_owned(),
        ]
    }

    fn app_selecting(path: &str) -> App {
        select(App::from_tree(&fixture::tree()), path)
    }

    fn app_with_files_selecting(path: &str) -> App {
        let mut app = App::from_tree(&fixture::tree());
        app.toggle_files();
        select(app, path)
    }

    fn select(mut app: App, path: &str) -> App {
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
            Node::new("repo", "repo/WARLOCK.md", NodeState::PactedStale).with_children([
                Node::new(
                    "repo/crates",
                    "repo/crates/WARLOCK.md",
                    NodeState::PactedFresh,
                )
                .with_children([Node::new(
                    "repo/crates/engine",
                    "repo/crates/engine/WARLOCK.md",
                    NodeState::PactedFresh,
                )]),
                Node::new("repo/assets", "repo/assets/WARLOCK.md", NodeState::Unpacted),
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

        // The default view draws a document row under each documented
        // directory too, so the nodes are the rows that are not files.
        assert_eq!(node_paths(&app).len(), tree.counts().total());
        for row in app.rows().iter().filter(|row| !row.is_file()) {
            let node = tree.find(&row.path).expect("row came from the tree");
            assert_eq!(row.state, node.state);
        }
    }

    #[test]
    fn an_app_carries_the_trees_own_counts() {
        let tree = fixture::tree();

        let app = App::from_tree(&tree);

        assert_eq!(app.counts(), tree.counts());
        // Directories are what is counted, and the document rows beside them
        // are not: see [`tally`].
        assert_eq!(app.counts().total(), node_paths(&app).len());
    }

    #[test]
    fn an_app_built_from_bare_rows_counts_nothing_until_told() {
        let counts = fixture::tree().counts();

        let app = App::from_rows(three_rows());

        assert_eq!(app.counts(), StateCounts::default());
        assert_eq!(app.with_counts(counts).counts(), counts);
    }

    #[test]
    fn a_root_below_the_repository_root_is_named_relative_to_it() {
        let chrome = Chrome::of(
            Path::new("/repo"),
            Path::new("/repo").join("crates").join("engine"),
        );

        // Forward slashes even where the separator is a backslash: this is the
        // engine's manifest spelling.
        assert_eq!(chrome.header(), "crates/engine");
    }

    #[test]
    fn the_repository_root_itself_gets_no_header_rather_than_a_dot_or_a_label() {
        let chrome = Chrome::of("/repo", "/repo");

        // The header says which part of the repository is on screen. The whole
        // of it is not a part, so there is nothing for the line to say — and the
        // bare "." the relative spelling would give is not an answer, it is the
        // question left unanswered.
        assert_eq!(chrome.header(), "");
        assert_ne!(chrome.header(), ".");
    }

    #[test]
    fn a_relative_root_is_taken_to_be_relative_to_the_repository_root_already() {
        assert_eq!(Chrome::of("/repo", "docs/adr").header(), "docs/adr");
    }

    #[test]
    fn a_root_outside_the_repository_falls_back_to_printing_itself() {
        // Not describable relative to the repository root, but a header is a
        // label: it says what it can rather than going blank.
        assert_eq!(
            Chrome::of("/repo", "/elsewhere/docs").header(),
            "/elsewhere/docs"
        );
    }

    #[test]
    fn a_chrome_nobody_built_states_nothing_at_all() {
        let chrome = Chrome::default();

        assert_eq!(chrome.header(), "");
        assert_eq!(chrome.sigils(), &Sigils::Nothing);
    }

    #[test]
    fn the_header_is_not_app_state_and_no_app_carries_one() {
        // The structural claim this type exists for, and the reason there is no
        // `App::header` to assert against: both halves of the header are
        // resolved once and cannot change while warlock runs, so an app rebuilt
        // on every reload has no business holding either. What is left to check
        // is that building one costs an app nothing — a `Chrome` is a function
        // of two paths and a config, and there is no `App` anywhere in it.
        let chrome = Chrome::of("/repo", "/repo/crates").with_sigils(Sigils::held(["billing"]));

        assert_eq!(chrome.header(), "crates");
        assert_eq!(chrome.sigils(), &Sigils::Held(vec!["billing".to_owned()]));
    }

    #[test]
    fn a_chrome_that_was_never_told_what_is_held_states_nothing() {
        // The default, and the state a reader who has never run `warlock
        // config` is in: there is no wording for it at all, so there is nothing
        // for the header to join onto the line it already had.
        let chrome = Chrome::of("/repo", "/repo/crates");

        assert_eq!(chrome.sigils(), &Sigils::Nothing);
        assert_eq!(chrome.sigils().line(), None);
    }

    #[test]
    fn the_sigils_held_are_listed_in_the_order_the_config_has_them() {
        let chrome = Chrome::default().with_sigils(Sigils::held(["billing", "web"]));

        assert_eq!(
            chrome.sigils(),
            &Sigils::Held(vec!["billing".to_owned(), "web".to_owned()])
        );
        assert_eq!(
            chrome.sigils().line(),
            Some("holding `billing`, `web`".into())
        );
    }

    #[test]
    fn a_missing_config_and_an_empty_set_are_the_same_nothing() {
        // Two ways of holding nothing, and the header must not tell them apart:
        // a machine that cleared its sigils and one that never had any hold the
        // same nothing.
        let empty: [String; 0] = [];

        assert_eq!(Sigils::held(empty), Sigils::Nothing);
        assert_eq!(Sigils::held(Vec::<String>::new()).line(), None);
    }

    #[test]
    fn a_config_that_would_not_read_is_said_out_loud_rather_than_looking_absent() {
        // The whole of why this is not an `Option<Vec<String>>`: broken and
        // absent mean opposite things about what is on disk, so they must not
        // draw the same line.
        let unknown = Sigils::Unknown;

        assert_eq!(unknown.line(), Some("holding unknown".into()));
        assert_ne!(unknown.line(), Sigils::Nothing.line());
        assert_ne!(unknown.line(), Sigils::held(["billing"]).line());
    }

    #[test]
    fn holding_nothing_leaves_the_header_byte_for_byte_what_it_was() {
        // The promise the three states are arranged around: a reader who never
        // runs `warlock config` sees the header they have always seen, in both
        // spellings of nothing and whether or not anything was ever set.
        let told = Chrome::of("/repo", "/repo/crates/engine");
        let before = told.header().to_owned();

        for nothing in [Sigils::Nothing, Sigils::held(Vec::<String>::new())] {
            let chrome = told.clone().with_sigils(nothing);

            assert_eq!(chrome.header(), before);
            assert_eq!(chrome.sigils().line(), None);
        }
    }

    #[test]
    fn what_is_held_changes_nothing_but_what_the_header_states() {
        // The identity half of the header is not touched by a holding. That no
        // *row* is coloured, filtered, sorted or re-ordered by one used to be
        // asserted here too, over an `App`; it is now true by construction,
        // because a `Chrome` has no rows to reach.
        let chrome = Chrome::of("/repo", "/repo/crates");

        let holding = chrome.clone().with_sigils(Sigils::held(["billing"]));

        assert_eq!(holding.header(), chrome.header());
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
    fn flattening_a_tree_carries_each_nodes_document() {
        let tree = fixture::tree();

        let app = App::from_tree(&tree);

        for row in app.rows().iter().filter(|row| !row.is_file()) {
            let node = tree.find(&row.path).expect("row came from the tree");
            assert_eq!(
                row.document,
                node.document,
                "document for {}",
                row.path.display()
            );
        }
        // The document rows drawn beside them stand for a file and so document
        // nothing themselves — a directory *has* a document and is not one.
        assert!(
            app.rows()
                .iter()
                .filter(|row| row.is_file())
                .all(|row| row.document.is_none() && row.is_document())
        );
        // Including `crates/`, whose document is honestly absent.
        assert!(
            app.rows()
                .iter()
                .any(|row| row.document.is_none() && row.path == Path::new("warlock/crates"))
        );
    }

    fn shape(app: &App) -> Vec<(String, usize, bool)> {
        app.rows()
            .iter()
            .map(|row| {
                (
                    row.path.to_string_lossy().into_owned(),
                    row.depth,
                    row.is_file(),
                )
            })
            .collect()
    }

    #[test]
    fn showing_files_draws_every_file_once_under_its_directory_at_one_more_depth() {
        let mut app = App::from_tree(&fixture::tree());
        app.toggle_files();

        // Pinned whole: every file the fixture lists — the documents among them
        // — once each, in path order, directly after the directory listing it
        // and one level deeper. Nothing about documents may move this.
        assert_eq!(
            shape(&app),
            [
                ("warlock".to_owned(), 0, false),
                ("warlock/README.md".to_owned(), 1, true),
                ("warlock/WARLOCK.md".to_owned(), 1, true),
                ("warlock/crates".to_owned(), 1, false),
                ("warlock/crates/engine".to_owned(), 2, false),
                ("warlock/crates/engine/Cargo.toml".to_owned(), 3, true),
                ("warlock/crates/engine/WARLOCK.md".to_owned(), 3, true),
                ("warlock/crates/tui".to_owned(), 2, false),
                ("warlock/crates/tui/WARLOCK.md".to_owned(), 3, true),
                ("warlock/assets".to_owned(), 1, false),
                ("warlock/assets/WARLOCK.md".to_owned(), 2, true),
                ("warlock/assets/logo.svg".to_owned(), 2, true),
            ]
        );
        // And no path drawn twice, which the list above pins only as long as
        // somebody reads it carefully.
        let mut paths = drawn(&app);
        let drawn_count = paths.len();
        paths.sort();
        paths.dedup();
        assert_eq!(paths.len(), drawn_count);
    }

    #[test]
    fn only_a_directorys_own_document_is_flagged_as_one() {
        let mut app = App::from_tree(&fixture::tree());
        app.toggle_files();

        let flagged: Vec<(String, bool)> = app
            .rows()
            .iter()
            .map(|row| (row.path.to_string_lossy().into_owned(), row.is_document()))
            .collect();

        assert_eq!(
            flagged,
            [
                ("warlock".to_owned(), false),
                // A README documents nothing as far as the tree is concerned.
                ("warlock/README.md".to_owned(), false),
                ("warlock/WARLOCK.md".to_owned(), true),
                // No document loaded, so nothing under it could be one.
                ("warlock/crates".to_owned(), false),
                ("warlock/crates/engine".to_owned(), false),
                ("warlock/crates/engine/Cargo.toml".to_owned(), false),
                ("warlock/crates/engine/WARLOCK.md".to_owned(), true),
                ("warlock/crates/tui".to_owned(), false),
                ("warlock/crates/tui/WARLOCK.md".to_owned(), true),
                ("warlock/assets".to_owned(), false),
                // Unpacted and documented: presence on disk, not pactedness, is
                // what the flag follows.
                ("warlock/assets/WARLOCK.md".to_owned(), true),
                ("warlock/assets/logo.svg".to_owned(), false),
            ]
        );
        // Said again as the two rules it stands for: a directory is never a
        // document, and each documented directory has exactly one.
        assert!(
            app.rows()
                .iter()
                .all(|row| !row.is_document() || row.is_file())
        );
        for (node, _) in fixture::tree().walk() {
            let documents = app
                .rows()
                .iter()
                .filter(|row| row.is_document() && row.path.parent() == Some(node.path.as_path()))
                .count();
            assert_eq!(documents, usize::from(node.document.is_some()));
        }
    }

    #[test]
    fn a_row_handed_over_without_a_tree_is_no_document() {
        // The safe default: `from_rows` tests hand rows over with no node
        // behind them, and a row nobody told about a document has none.
        assert!(!Row::file(1, "repo/WARLOCK.md", NodeState::PactedFresh).is_document());
        assert!(
            Row::file(1, "repo/WARLOCK.md", NodeState::PactedFresh)
                .with_document_row(true)
                .is_document()
        );
        assert!(!Row::new(0, "repo", "repo/WARLOCK.md", NodeState::PactedFresh).is_document());
    }

    #[test]
    fn pacting_an_unpacted_node_makes_it_stale_and_pacting_again_undoes_it() {
        let mut app = app_selecting("warlock/assets");

        let pacted = app.toggle_pact().expect("a directory can be pacted");

        assert_eq!(
            pacted,
            PactToggle {
                path: PathBuf::from("warlock/assets"),
                pacted: true,
            }
        );
        // Never fresh: a pact with no granted hash was never judged.
        assert_eq!(
            app.selected_row().map(|row| row.state),
            Some(NodeState::PactedStale)
        );

        let unpacted = app.toggle_pact().expect("a directory can be un-pacted");

        assert_eq!(
            unpacted,
            PactToggle {
                path: PathBuf::from("warlock/assets"),
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

        let toggled = app.toggle_pact().expect("a directory can be un-pacted");

        // Fresh goes straight to unpacted: the grant goes with the pact.
        assert!(!toggled.pacted);
        assert_eq!(
            app.selected_row().map(|row| row.state),
            Some(NodeState::Unpacted)
        );
    }

    #[test]
    fn a_directory_with_no_document_is_pacted_like_any_other() {
        // `crates/` has no `WARLOCK.md` in the fixture, which is exactly the
        // case the pact operation exists to fix: it writes one.
        let mut app = app_selecting("warlock/crates");
        assert_eq!(
            app.selected_row().and_then(|row| row.document.clone()),
            None
        );

        let pacted = app.toggle_pact().expect("an undocumented directory pacts");

        assert_eq!(
            pacted,
            PactToggle {
                path: PathBuf::from("warlock/crates"),
                pacted: true,
            }
        );
        assert_eq!(
            app.selected_row().map(|row| row.state),
            Some(NodeState::PactedStale)
        );
        assert_eq!(app.message(), None);
        assert_eq!(app.counts(), tally(&app));
    }

    #[test]
    fn pacting_a_directory_pacts_everything_below_it() {
        let mut app = app_selecting("warlock/crates");

        app.toggle_pact().expect("a directory can be pacted");

        // The directory the key was pressed on and both modules under it, and
        // nothing outside the subtree. The document rows take their
        // directory's colour, here as everywhere.
        assert_eq!(
            states(&app),
            [
                ("warlock", NodeState::PactedStale),
                ("warlock/WARLOCK.md", NodeState::PactedStale),
                ("warlock/crates", NodeState::PactedStale),
                ("warlock/crates/engine", NodeState::PactedStale),
                ("warlock/crates/engine/WARLOCK.md", NodeState::PactedStale),
                ("warlock/crates/tui", NodeState::PactedStale),
                ("warlock/crates/tui/WARLOCK.md", NodeState::PactedStale),
                ("warlock/assets", NodeState::Unpacted),
                ("warlock/assets/WARLOCK.md", NodeState::Unpacted),
            ]
        );
        assert_eq!(app.counts(), tally(&app));
        assert_eq!(app.counts().total(), 5);
    }

    #[test]
    fn un_pacting_a_directory_greys_the_whole_subtree() {
        // The root, so the subtree is the whole tree and holds all three states
        // on the way in.
        let mut app = app_selecting("warlock");

        let toggled = app.toggle_pact().expect("a directory can be un-pacted");

        assert!(!toggled.pacted);
        assert!(
            app.rows()
                .iter()
                .all(|row| row.state == NodeState::Unpacted),
            "{:?}",
            states(&app)
        );
        assert_eq!(app.counts(), tally(&app));
        assert_eq!(app.counts().unpacted, 5);
        assert_eq!(app.counts().total(), 5);
    }

    #[test]
    fn un_pacting_says_the_documents_were_left_on_disk() {
        let mut app = app_selecting("warlock/crates/tui");

        app.toggle_pact().expect("a directory can be un-pacted");

        let message = app.message().expect("un-pacting says what it left behind");
        assert!(message.starts_with("warlock/crates/tui"), "{message}");
        assert!(message.contains("left on disk"), "{message}");
    }

    #[test]
    fn a_subtree_can_be_put_into_a_state_the_toggle_never_reaches() {
        // What a caller that has really pacted a subtree — documents written,
        // hashes granted — says afterwards. The toggle itself never reaches
        // fresh, because it grants nothing.
        let mut app = App::from_tree(&fixture::tree());

        app.set_subtree_state("warlock/crates", NodeState::PactedFresh);

        assert_eq!(
            states(&app),
            [
                ("warlock", NodeState::PactedStale),
                ("warlock/WARLOCK.md", NodeState::PactedStale),
                ("warlock/crates", NodeState::PactedFresh),
                ("warlock/crates/engine", NodeState::PactedFresh),
                ("warlock/crates/engine/WARLOCK.md", NodeState::PactedFresh),
                ("warlock/crates/tui", NodeState::PactedFresh),
                ("warlock/crates/tui/WARLOCK.md", NodeState::PactedFresh),
                ("warlock/assets", NodeState::Unpacted),
                ("warlock/assets/WARLOCK.md", NodeState::Unpacted),
            ]
        );
        assert_eq!(app.counts(), tally(&app));
        assert_eq!(app.counts().total(), 5);
    }

    #[test]
    fn a_subtree_state_reaches_the_rows_no_view_is_drawing() {
        let mut app = App::from_tree(&fixture::tree()).with_collapsed(["warlock/crates"]);
        assert_eq!(drawn(&app), collapsed_over_crates());

        app.set_subtree_state("warlock/crates", NodeState::PactedFresh);
        let mut app = select(app, "warlock/crates");
        app.toggle_collapsed();

        // Hidden under a collapsed directory while the state was set, and still
        // coloured by it when the directory opens again.
        let engine = app
            .rows()
            .iter()
            .find(|row| row.path == Path::new("warlock/crates/engine"))
            .expect("expanding brought it back");
        assert_eq!(engine.state, NodeState::PactedFresh);
        assert_eq!(app.counts(), tally(&app));
    }

    #[test]
    fn a_subtree_state_for_a_path_no_row_stands_for_changes_nothing() {
        let mut app = App::from_tree(&fixture::tree());
        let before = app.clone();

        app.set_subtree_state("warlock/crates-old", NodeState::PactedFresh);

        // A sibling is not a descendant, however much of its name it shares,
        // and a directory the tree has never heard of is nothing at all.
        assert_eq!(app.rows(), before.rows());
        assert_eq!(app.counts(), before.counts());
    }

    #[test]
    fn a_subtree_state_leaves_the_message_alone() {
        let mut app = App::from_tree(&fixture::tree());
        app.set_message("something the caller said");

        app.set_subtree_state("warlock", NodeState::PactedFresh);

        // Not a keystroke: it neither says anything nor takes down what the
        // keystroke it belongs to put up.
        assert_eq!(app.message(), Some("something the caller said"));
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
    fn the_next_keystroke_clears_the_message() {
        let mut app = app_with_files_selecting("warlock/assets/logo.svg");
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
    fn a_pact_that_goes_through_leaves_no_refusal_behind() {
        let mut app = app_with_files_selecting("warlock/assets/logo.svg");
        assert_eq!(app.toggle_pact(), None);
        assert!(app.message().is_some());

        // Onto the directory holding it, which the movement clears the message
        // for, and then a toggle that works, which must not put one back.
        app.select_previous();
        app.select_previous();
        assert_eq!(
            app.selected_row().map(|row| row.path.clone()),
            Some(PathBuf::from("warlock/assets"))
        );
        app.toggle_pact().expect("a directory can be pacted");

        assert_eq!(app.message(), None);
    }

    #[test]
    fn a_root_with_no_document_is_pacted_and_named_as_it_stands() {
        let mut app = App::from_rows(vec![Row::new(0, "repo", None, NodeState::PactedStale)])
            .with_counts(StateCounts {
                pacted_stale: 1,
                ..StateCounts::default()
            });

        let toggled = app.toggle_pact().expect("the root can be un-pacted");

        assert!(!toggled.pacted);
        assert_eq!(app.counts(), tally(&app));
        // The root cannot be named relative to itself, so it is named as it
        // stands rather than as the `"."` that relative spelling would give.
        let message = app.message().expect("un-pacting says what it left behind");
        assert!(message.starts_with("repo is no longer pacted"), "{message}");
    }

    #[test]
    fn refreshing_a_stale_directory_hands_its_subtree_back_and_says_nothing() {
        let mut app = app_selecting("warlock/crates/tui");
        app.set_message("something the last keystroke said");
        let before = app.clone();

        let asked = app.refresh();

        // The root of the subtree to describe again, for whoever runs the pass.
        assert_eq!(asked, Some(PathBuf::from("warlock/crates/tui")));
        // A press that does something says nothing, and does nothing else here:
        // the states, the tally, the selection and the panel are the run's to
        // move once it is under way, not this key's.
        assert_eq!(app.message(), None);
        assert_eq!(app.rows(), before.rows());
        assert_eq!(app.counts(), before.counts());
        assert!(!app.panel().has_account());
        assert!(!app.is_pacting());
    }

    #[test]
    fn a_file_row_is_refused_in_the_pact_keys_own_words() {
        let mut refreshed = app_with_files_selecting("warlock/assets/logo.svg");
        let mut pacted = app_with_files_selecting("warlock/assets/logo.svg");

        assert_eq!(refreshed.refresh(), None);
        assert_eq!(pacted.toggle_pact(), None);

        // The same refusal, because it is the same reason: a file is part of a
        // module rather than being one, so neither key has a subtree here.
        let message = refreshed.message().expect("a file row is refused out loud");
        assert_eq!(Some(message), pacted.message());
        assert!(
            message.starts_with("warlock/assets/logo.svg is a file"),
            "{message}"
        );
    }

    fn rows_with_one_kept_out() -> Vec<Row> {
        vec![
            Row::new(0, "repo", "repo/WARLOCK.md", NodeState::PactedStale).with_child_count(2),
            // Kept out by the rules: unpacted like any other unpacted directory,
            // and carrying no state of its own for being excluded.
            Row::new(1, "repo/notes", None, NodeState::Unpacted).with_ignored(true),
            Row::new(
                1,
                "repo/crates",
                "repo/crates/WARLOCK.md",
                NodeState::Unpacted,
            ),
        ]
    }

    #[test]
    fn a_row_kept_out_by_the_ignore_file_is_refused_and_nothing_moves() {
        let mut app = select(App::from_rows(rows_with_one_kept_out()), "repo/notes").with_counts(
            StateCounts {
                unpacted: 2,
                pacted_stale: 1,
                ..StateCounts::default()
            },
        );
        let mut before = app.clone();
        let states: Vec<NodeState> = app.rows().iter().map(|row| row.state).collect();

        assert_eq!(app.toggle_pact(), None, "an excluded row started something");

        // The rule is the repository author's and is written down in a file
        // they can open, so the refusal names it rather than sounding like a
        // verdict of Warlock's own.
        let message = app.message().expect("an excluded row is refused out loud");
        assert!(message.contains(".warlockignore"), "{message}");
        assert!(message.starts_with("repo/notes is kept out"), "{message}");
        // No state moved: not the excluded row's, not its siblings', not the
        // root's above it.
        assert_eq!(
            app.rows().iter().map(|row| row.state).collect::<Vec<_>>(),
            states
        );
        assert_eq!(app.counts(), before.counts());
        // And the message is the whole of what the press changed: no subtree
        // repainted, no selection moved, no account started, no run in flight.
        before.set_message(message);
        assert_eq!(app, before, "refusing an excluded row moved something else");
        assert!(!app.panel().has_account());
        assert!(!app.is_pacting());
    }

    #[test]
    fn asking_what_the_pact_key_means_changes_nothing_at_all() {
        // The whole point of splitting the question off the press: a caller
        // that has to decline before anything moves can ask, and asking is
        // free of consequence — no subtree repainted, no message written, no
        // selection moved.
        for (row, expected) in [
            (
                "repo/crates",
                PactIntent::Toggles(PactToggle {
                    path: PathBuf::from("repo/crates"),
                    pacted: true,
                }),
            ),
            (
                "repo/notes",
                PactIntent::Refused(ignored_row_message("repo/notes")),
            ),
        ] {
            let app = select(App::from_rows(rows_with_one_kept_out()), row);
            let before = app.clone();

            assert_eq!(app.pact_intent(), expected, "{row}");
            assert_eq!(app.pact_intent(), expected, "{row}: asking twice differs");
            assert_eq!(app, before, "{row}: asking moved something");
            assert_eq!(app.message(), None, "{row}: asking said something");
        }
    }

    #[test]
    fn the_reach_of_a_press_answers_for_a_row_its_meaning_refuses() {
        // The two questions are deliberately not the same one. Whether a press
        // would lose a scope boundary is settled before the repository's own
        // rules are applied to it, so an excluded row has a reach even though
        // it has no meaning.
        let app = select(App::from_rows(rows_with_one_kept_out()), "repo/notes");
        assert!(matches!(app.pact_intent(), PactIntent::Refused(_)));
        assert_eq!(
            app.pact_reach(),
            Some(PactToggle {
                path: PathBuf::from("repo/notes"),
                pacted: true,
            }),
            "an excluded row still says which way a press would go"
        );

        // A file row is no module either way: there is no subtree to reach.
        let mut app = App::from_rows(vec![
            Row::new(0, "repo", "repo/WARLOCK.md", NodeState::PactedFresh),
            Row::file(1, "repo/lib.rs", NodeState::PactedFresh),
        ]);
        // File rows are detail asked for, so they have to be on screen before
        // one can be selected.
        app.toggle_files();
        let app = select(app, "repo/lib.rs");
        assert!(matches!(app.pact_intent(), PactIntent::Refused(_)));
        assert_eq!(app.pact_reach(), None);
    }

    #[test]
    fn a_covered_sibling_of_an_excluded_row_pacts_as_usual() {
        let mut app = select(App::from_rows(rows_with_one_kept_out()), "repo/crates");

        let toggled = app
            .toggle_pact()
            .expect("a covered directory can be pacted");

        // The refusal is about the row the rules name, not about the tree it is
        // in, and a press that goes through says nothing.
        assert_eq!(
            toggled,
            PactToggle {
                path: PathBuf::from("repo/crates"),
                pacted: true,
            }
        );
        assert_eq!(app.message(), None);
    }

    #[test]
    fn pacting_a_subtree_leaves_an_excluded_row_out_of_the_pact() {
        // The press the reader actually makes: `p` on a directory *above* the
        // excluded one, which never asks `toggle_pact`'s refusal anything. The
        // fixture's root is pacted already, so the first press takes the subtree
        // out and the second is the one under test.
        let mut app =
            select(App::from_rows(rows_with_one_kept_out()), "repo").with_counts(StateCounts {
                unpacted: 2,
                pacted_stale: 1,
                ..StateCounts::default()
            });
        app.toggle_pact().expect("the root can be un-pacted");

        let toggled = app.toggle_pact().expect("and pacted again");

        assert!(toggled.pacted);
        // The engine's pact walk reads `.warlockignore` and will never reach
        // `repo/notes`, so painting it yellow would promise a document that no
        // run is going to write.
        assert_eq!(
            states(&app),
            [
                ("repo", NodeState::PactedStale),
                ("repo/notes", NodeState::Unpacted),
                ("repo/crates", NodeState::PactedStale),
            ]
        );
        assert_eq!(app.counts(), tally(&app));
        assert_eq!(app.counts().unpacted, 1);
        assert_eq!(app.counts().total(), 3);
    }

    #[test]
    fn a_granted_subtree_leaves_an_excluded_row_out_too() {
        // The other half of the same rule, and the one a run reaches: the
        // caller with grants in hand says so over the whole subtree, and green
        // is a claim about a document that `repo/notes` has no more of than
        // yellow was.
        let mut app = App::from_rows(rows_with_one_kept_out());

        app.set_subtree_state("repo", NodeState::PactedFresh);

        assert_eq!(
            states(&app),
            [
                ("repo", NodeState::PactedFresh),
                ("repo/notes", NodeState::Unpacted),
                ("repo/crates", NodeState::PactedFresh),
            ]
        );
    }

    #[test]
    fn an_excluded_row_still_follows_its_subtree_out_of_a_pact() {
        // The direction that does move it. `unpact_subtree` is manifest
        // arithmetic and drops every entry at or below the directory without
        // asking the ignore rules anything, so a directory excluded after it was
        // pacted loses its entry with the rest — and a row held back here would
        // keep a colour the manifest no longer has anything to say for.
        let mut rows = rows_with_one_kept_out();
        rows[1].state = NodeState::PactedFresh;
        let mut app = App::from_rows(rows);

        app.set_subtree_state("repo", NodeState::Unpacted);

        assert_eq!(
            states(&app),
            [
                ("repo", NodeState::Unpacted),
                ("repo/notes", NodeState::Unpacted),
                ("repo/crates", NodeState::Unpacted),
            ]
        );
    }

    #[test]
    fn a_file_inside_excluded_content_is_held_back_with_its_directory() {
        // A file row carries its directory's ignore flag precisely so that it
        // is not left drawn in a colour its module never took.
        let tree = Tree::new(
            Node::new("repo", "repo/WARLOCK.md", NodeState::Unpacted)
                .with_files([PathBuf::from("repo/WARLOCK.md")])
                .with_children([Node::new("repo/notes", None, NodeState::Unpacted)
                    .with_files([PathBuf::from("repo/notes/plan.md")])
                    .with_ignored(true)]),
        );
        let mut app = App::from_tree(&tree);
        app.toggle_files();

        app.set_subtree_state("repo", NodeState::PactedStale);

        assert_eq!(
            states(&app),
            [
                ("repo", NodeState::PactedStale),
                ("repo/WARLOCK.md", NodeState::PactedStale),
                ("repo/notes", NodeState::Unpacted),
                ("repo/notes/plan.md", NodeState::Unpacted),
            ]
        );
    }

    #[test]
    fn an_excluded_row_is_drawn_in_no_colour_of_its_own() {
        let app = App::from_rows(rows_with_one_kept_out());
        let excluded = app
            .rows()
            .iter()
            .find(|row| row.is_ignored())
            .expect("the fixture keeps one row out");

        // Gray already means outside Warlock's management, and being kept out is
        // not a fourth state: the flag changes nothing the renderer reads.
        assert_eq!(excluded.state, NodeState::Unpacted);
        assert_eq!(
            *excluded,
            Row::new(1, "repo/notes", None, NodeState::Unpacted).with_ignored(true)
        );
    }

    #[test]
    fn flattening_a_tree_carries_the_ignore_flag_onto_the_rows() {
        let tree = Tree::new(
            Node::new("repo", "repo/WARLOCK.md", NodeState::PactedStale).with_children([
                Node::new("repo/notes", None, NodeState::Unpacted)
                    .with_files([PathBuf::from("repo/notes/plan.md")])
                    .with_ignored(true),
                Node::new("repo/crates", None, NodeState::Unpacted),
            ]),
        );
        let mut app = App::from_tree(&tree);
        app.toggle_files();

        let flagged: Vec<(String, bool)> = app
            .rows()
            .iter()
            .map(|row| (row.path.to_string_lossy().into_owned(), row.is_ignored()))
            .collect();
        assert_eq!(
            flagged,
            [
                ("repo".to_owned(), false),
                ("repo/notes".to_owned(), true),
                // The rules exclude a directory's content along with it, so the
                // file listed inside it is told what the directory was told.
                ("repo/notes/plan.md".to_owned(), true),
                ("repo/crates".to_owned(), false),
            ]
        );

        // And the flag is enough on its own to refuse the key, with no tree,
        // loader or filesystem behind the app by the time it is pressed.
        let mut app = select(app, "repo/notes");
        assert_eq!(app.toggle_pact(), None);
        assert!(
            app.message()
                .expect("an excluded row is refused")
                .contains(".warlockignore")
        );
    }

    fn tree_with_one_scoped_directory() -> Tree {
        Tree::new(
            Node::new("repo", "repo/WARLOCK.md", NodeState::PactedStale)
                .with_scope(Some("tui-team".to_owned()))
                .with_files([PathBuf::from("repo/README.md")])
                .with_children([
                    // Pacted, and covered by the scope written above it, which is
                    // not the same as carrying one.
                    Node::new(
                        "repo/crates",
                        "repo/crates/WARLOCK.md",
                        NodeState::PactedFresh,
                    ),
                    // Nobody pacted it, so there is no entry a scope could be
                    // written on.
                    Node::new("repo/notes", None, NodeState::Unpacted),
                ]),
        )
    }

    #[test]
    fn flattening_a_tree_carries_each_nodes_own_scope_onto_its_row() {
        let mut app = App::from_tree(&tree_with_one_scoped_directory());
        app.toggle_files();

        let scoped: Vec<(String, Option<&str>)> = app
            .rows()
            .iter()
            .map(|row| {
                (
                    row.path.to_string_lossy().into_owned(),
                    row.scope.as_deref(),
                )
            })
            .collect();
        assert_eq!(
            scoped,
            [
                // The directory the scope was written on, and the only row of
                // the four with one.
                ("repo".to_owned(), Some("tui-team")),
                // A file has no pact entry, so it takes no scope from the
                // directory listing it — unlike the state and the ignore flag,
                // which it does take.
                ("repo/README.md".to_owned(), None),
                // Covered by the scope above it and carrying none: the label
                // marks where a boundary starts, so nothing is inherited down.
                ("repo/crates".to_owned(), None),
                // Unpacted, so there is no entry to have written one on.
                ("repo/notes".to_owned(), None),
            ]
        );
    }

    #[test]
    fn a_scope_on_a_row_moves_no_state_and_needs_no_tree() {
        let plain = Row::new(0, "repo", "repo/WARLOCK.md", NodeState::PactedStale);
        let scoped = plain.clone().with_scope(Some("tui-team".to_owned()));

        // A row nobody told about a scope has none: the fact comes from a pact
        // entry somebody wrote, never from the row's own shape.
        assert_eq!(plain.scope, None);
        assert_eq!(scoped.scope.as_deref(), Some("tui-team"));
        // A scope is a label, not a state: nothing else about the row moves, so
        // a scoped row is drawn in the colour it would have had unlabelled.
        assert_eq!(scoped.state, plain.state);
        assert_eq!(scoped.clone().with_scope(None), plain);

        // And the builder is enough on its own: an app can be handed a scoped
        // row with no tree, loader or disk behind it.
        let app = App::from_rows(vec![scoped]);
        assert_eq!(app.rows()[0].scope.as_deref(), Some("tui-team"));
    }

    #[test]
    fn a_fresh_directory_is_refused_and_nothing_else_moves() {
        let mut app = app_selecting("warlock/crates/engine");
        let mut before = app.clone();

        assert_eq!(app.refresh(), None);

        let message = app.message().expect("a fresh directory is refused");
        assert!(
            message.starts_with("warlock/crates/engine is already fresh"),
            "{message}"
        );
        // The message is the whole of what the press changed: no subtree
        // repainted, no tally moved, no account started, no run in flight.
        before.set_message(message);
        assert_eq!(app, before, "refusing a fresh row moved something else");
    }

    #[test]
    fn an_unpacted_directory_is_refused_by_naming_the_pact_key() {
        // Two ways a directory comes to be unpacted: one the loader found no
        // document for, and one that has a document and no manifest entry —
        // which is how a directory a `.warlockignore` keeps out reads here. The
        // refusal does not care which, because the answer is the same.
        for path in ["warlock/crates", "warlock/assets"] {
            let mut app = app_selecting(path);
            let mut before = app.clone();

            assert_eq!(app.refresh(), None, "{path} started something");

            let message = app.message().expect("an unpacted row is refused");
            assert!(
                message.starts_with(&format!("{path} is not pacted")),
                "{message}"
            );
            // The key that would help is named, since the reader is one
            // keystroke away from what they asked for.
            assert!(message.contains("press p to pact it"), "{message}");
            before.set_message(message);
            assert_eq!(app, before, "refusing {path} moved something else");
        }
    }

    #[test]
    fn an_app_with_no_rows_refreshes_nothing() {
        assert_eq!(App::from_rows(Vec::new()).refresh(), None);
        assert_eq!(App::from_rows(Vec::new()).message(), None);
    }

    #[test]
    fn scoping_a_pacted_directory_hands_it_back_and_changes_nothing_at_all() {
        // Stale and fresh alike: whether the grants still hold has nothing to
        // do with which team the module belongs to.
        for path in ["warlock/crates/tui", "warlock/crates/engine"] {
            let mut app = app_selecting(path);
            app.set_message("something the last keystroke said");
            let before = app.clone();

            let asked = app.scope_target();

            // The directory whose entry the prompt will be opened on.
            assert_eq!(asked, Some(PathBuf::from(path)), "{path}");
            // And nothing whatever moved — including the message line, which
            // this key alone leaves standing: a prompt opened and dismissed
            // with Esc has to leave the app exactly as it was found.
            assert_eq!(app, before, "an accepted press moved something on {path}");
        }
    }

    #[test]
    fn a_file_row_is_refused_by_the_scope_key_in_the_pact_keys_own_words() {
        let mut scoped = app_with_files_selecting("warlock/assets/logo.svg");
        let mut pacted = app_with_files_selecting("warlock/assets/logo.svg");

        assert_eq!(scoped.scope_target(), None);
        assert_eq!(pacted.toggle_pact(), None);

        // The same refusal, because it is the same reason: a file is part of a
        // module rather than being one, and the module is what carries the pact
        // a scope hangs off.
        let message = scoped.message().expect("a file row is refused out loud");
        assert_eq!(Some(message), pacted.message());
        assert!(
            message.starts_with("warlock/assets/logo.svg is a file"),
            "{message}"
        );
    }

    #[test]
    fn an_unpacted_directory_is_refused_by_the_scope_key_naming_the_pact_key() {
        // The two ways a directory comes to be unpacted, as in the refresh
        // key's test: one the loader found no document for, and one with a
        // document and no manifest entry.
        for path in ["warlock/crates", "warlock/assets"] {
            let mut app = app_selecting(path);
            let mut before = app.clone();

            assert_eq!(app.scope_target(), None, "{path} opened something");

            let message = app.message().expect("an unpacted row is refused");
            assert!(
                message.starts_with(&format!("{path} is not pacted")),
                "{message}"
            );
            // The key that would help is named, as it is on the refresh key —
            // but what a pact would leave behind is worded for this key: a pact
            // to scope, not something to refresh.
            assert!(message.contains("press p to pact it"), "{message}");
            assert!(
                message.ends_with("there will be a pact to scope"),
                "{message}"
            );
            // The message is the whole of what the press changed.
            before.set_message(message);
            assert_eq!(app, before, "refusing {path} moved something else");
        }
    }

    #[test]
    fn a_row_kept_out_by_the_ignore_file_is_refused_by_the_scope_key_as_unpacted() {
        let mut app = select(App::from_rows(rows_with_one_kept_out()), "repo/notes").with_counts(
            StateCounts {
                unpacted: 2,
                pacted_stale: 1,
                ..StateCounts::default()
            },
        );
        let mut before = app.clone();

        assert_eq!(app.scope_target(), None, "an excluded row opened something");

        // Refused for having no pact rather than for being kept out: as far as
        // the manifest goes those are the same fact, and the sentence stays
        // true either way.
        let message = app.message().expect("an excluded row is refused out loud");
        assert!(message.starts_with("repo/notes is not pacted"), "{message}");
        assert!(message.contains("press p to pact it"), "{message}");
        before.set_message(message);
        assert_eq!(app, before, "refusing an excluded row moved something else");
    }

    #[test]
    fn an_app_with_no_rows_scopes_nothing() {
        let mut app = App::from_rows(Vec::new());

        assert_eq!(app.scope_target(), None);
        assert_eq!(app.message(), None);
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
    fn a_pact_in_flight_names_the_directory_relative_to_the_root_with_its_place() {
        let mut app = App::from_rows(rooted_rows());

        app.set_pact_in_flight(
            Path::new("/repo").join("crates").join("warlock-engine"),
            3,
            12,
        );

        assert!(app.is_pacting());
        // Relative to the tree's own root, in the engine's forward-slash
        // manifest spelling whatever the platform's separator is, with a
        // one-based place in the run beside it.
        assert_eq!(
            app.pact_line().as_deref(),
            Some("pacting crates/warlock-engine (3/12)")
        );
    }

    #[test]
    fn the_root_of_the_tree_is_named_as_it_stands_in_a_pact_line() {
        let mut app = App::from_rows(rooted_rows());

        app.set_pact_in_flight("/repo", 1, 5);

        // The root cannot be named relative to itself, so it is named as it
        // stands rather than as the `"."` relative spelling would give — the
        // same rule every other label here follows.
        assert_eq!(app.pact_line().as_deref(), Some("pacting /repo (1/5)"));
    }

    #[test]
    fn the_pact_line_moves_as_the_caller_advances_the_run() {
        let mut app = App::from_rows(rooted_rows());
        let mut said = Vec::new();

        for (position, path) in ["/repo", "/repo/crates", "/repo/crates/warlock-engine"]
            .into_iter()
            .enumerate()
        {
            app.set_pact_in_flight(path, position + 1, 3);
            said.push(app.pact_line().expect("a pact is in flight"));
        }

        assert_eq!(
            said,
            [
                "pacting /repo (1/3)",
                "pacting crates (2/3)",
                "pacting crates/warlock-engine (3/3)",
            ]
        );
    }

    #[test]
    fn the_progress_line_takes_its_verb_from_the_kind_of_run() {
        for (run, said) in [
            (Run::Pact, "pacting crates/warlock-engine (3/12)"),
            (Run::Refresh, "refreshing crates/warlock-engine (3/12)"),
        ] {
            let mut app = App::from_rows(rooted_rows());

            app.set_run_in_flight(
                run,
                Path::new("/repo").join("crates").join("warlock-engine"),
                3,
                12,
            );

            // One word apart: the directory, the fraction and the shape of the
            // line are the same, because it is the same kind of work.
            assert_eq!(app.pact_line().as_deref(), Some(said), "{run:?}");
            // And a refresh is a run in flight for everything else that asks.
            assert!(app.is_pacting(), "{run:?}");
            assert!(
                app.is_in_flight(&Path::new("/repo").join("crates").join("warlock-engine")),
                "{run:?}"
            );
        }
    }

    #[test]
    fn the_pact_key_starts_the_pacting_verb_and_nothing_has_to_say_so() {
        let mut app = App::from_rows(rooted_rows());

        app.set_pact_in_flight("/repo", 1, 5);

        // `set_pact_in_flight` is `set_run_in_flight` with the kind filled in,
        // so every caller that predates the refresh goes on wording pacts.
        assert_eq!(app.pact_line().as_deref(), Some("pacting /repo (1/5)"));
    }

    #[test]
    fn a_refresh_counts_the_stale_directories_it_was_given_and_keeps_its_clauses() {
        let mut app = App::from_rows(rooted_rows());

        // Seven of the subtree's forty directories are stale, which is the
        // engine's counting for the refresh and is passed straight through.
        app.set_run_in_flight(
            Run::Refresh,
            Path::new("/repo").join("crates").join("warlock-engine"),
            3,
            7,
        );
        app.set_pact_refused();

        // The refusal is worded exactly as it is for a pact, and still goes
        // last.
        assert_eq!(
            app.pact_line().as_deref(),
            Some("refreshing crates/warlock-engine (3/7) — already running")
        );

        // The run moving on re-words the line around the new directory and
        // carries the refusal along, refresh or not.
        app.set_run_in_flight(Run::Refresh, Path::new("/repo").join("crates"), 4, 7);
        assert_eq!(
            app.pact_line().as_deref(),
            Some("refreshing crates (4/7) — already running")
        );

        // And the end of the run takes the whole line down, kind and all.
        app.clear_pact_in_flight();
        assert!(!app.is_pacting());
        assert_eq!(app.pact_line(), None);
    }

    #[test]
    fn an_app_with_no_pact_running_has_no_pact_line() {
        let mut app = App::from_tree(&fixture::tree());
        assert!(!app.is_pacting());
        assert_eq!(app.pact_line(), None);
        assert_eq!(App::from_rows(three_rows()).pact_line(), None);

        app.set_pact_in_flight("warlock/crates", 2, 4);
        app.clear_pact_in_flight();

        // And the run being over is the caller's to say, whichever way it ended.
        assert!(!app.is_pacting());
        assert_eq!(app.pact_line(), None);
        // Clearing one that was never there changes nothing.
        app.clear_pact_in_flight();
        assert_eq!(app.pact_line(), None);
    }

    #[test]
    fn an_app_with_no_run_in_flight_has_no_run_header() {
        let mut app = App::from_rows(rooted_rows());

        // Before the first run there is nothing to head: a panel with no run
        // draws no header and keeps those rows for the account.
        assert!(app.run_header().is_none());

        app.set_pact_in_flight(Path::new("/repo").join("crates"), 2, 4);
        assert!(app.run_header().is_some());

        // And the run being over takes the header down with the footer's line,
        // however the run ended.
        app.clear_pact_in_flight();
        assert!(app.run_header().is_none());
        assert_eq!(app.pact_line(), None);
    }

    #[test]
    fn setting_the_mode_changes_the_mode_and_nothing_else() {
        let base = Instant::now();
        let mut app = App::from_rows(rooted_rows());
        app.panel_mut().start_turn("what does this do", base);
        app.panel_mut()
            .answer_turn("it walks the tree", at(base, 1));
        app.set_run_in_flight(Run::Pact, Path::new("/repo").join("crates"), 1, 4);

        let before = app.panel().window(at(base, 2));
        let header = app.run_header().expect("a run in flight has a header");

        // Chat until somebody says otherwise, and the change is reported.
        assert_eq!(app.panel().mode(), Mode::Chat);
        assert!(
            app.panel_mut().set_mode(Mode::Brief),
            "chat to brief is a change"
        );
        assert_eq!(app.panel().mode(), Mode::Brief);

        // Nothing else moved: same rows in the same order, same card, same
        // header — the mode wrote no note and started no turn of its own.
        assert_eq!(app.panel().window(at(base, 2)), before);
        assert!(
            app.panel().showing_thread(),
            "the mode moved the card showing"
        );
        let after = app.run_header().expect("the mode took the header down");
        assert_eq!(after.run(), header.run());
        assert_eq!(after.directory(), header.directory());
        assert_eq!(after.position(), header.position());
        assert_eq!(after.total(), header.total());

        // Setting the mode it is already in is no change, and says so: that is
        // how a re-sent instruction knows to add no second note.
        assert!(
            !app.panel_mut().set_mode(Mode::Brief),
            "brief to brief is no change"
        );
        assert_eq!(app.panel().mode(), Mode::Brief);
        assert!(
            app.panel_mut().set_mode(Mode::Chat),
            "brief to chat is a change"
        );
        assert_eq!(app.panel().window(at(base, 2)), before);
    }

    #[test]
    fn the_run_header_states_the_kind_the_directory_and_the_fraction() {
        for run in [Run::Pact, Run::Refresh] {
            let mut app = App::from_rows(rooted_rows());

            app.set_run_in_flight(
                run,
                Path::new("/repo").join("crates").join("warlock-engine"),
                3,
                12,
            );

            let header = app.run_header().expect("a run in flight has a header");
            // The kind is the caller's, said back unchanged: the header is the
            // one place that has to know a refresh from a pact without reading
            // the footer's wording for it.
            assert_eq!(header.run(), run, "{run:?}");
            assert_eq!(header.directory(), "crates/warlock-engine", "{run:?}");
            assert_eq!(header.position(), 3, "{run:?}");
            assert_eq!(header.total(), 12, "{run:?}");
        }
    }

    #[test]
    fn the_run_headers_directory_is_spelled_the_way_the_footer_spells_it() {
        let mut app = App::from_rows(rooted_rows());

        app.set_pact_in_flight(
            Path::new("/repo").join("crates").join("warlock-engine"),
            3,
            12,
        );

        // One speller, so the header and the footer drawn in the same frame
        // cannot name one directory two ways.
        let header = app.run_header().expect("a run in flight has a header");
        assert_eq!(header.directory(), "crates/warlock-engine");
        assert_eq!(
            app.pact_line().as_deref(),
            Some("pacting crates/warlock-engine (3/12)")
        );

        // The root of the tree on screen is what it is spelled against, and a
        // path that is not under that root is printed as it stands rather than
        // as nothing at all.
        app.set_pact_in_flight(Path::new("/elsewhere").join("notes"), 4, 12);
        let header = app.run_header().expect("a run in flight has a header");
        assert_eq!(
            header.directory(),
            Path::new("/elsewhere/notes").display().to_string()
        );
        assert!(
            app.pact_line()
                .expect("a run in flight has a line")
                .contains(header.directory()),
            "the footer spelled it some other way"
        );
    }

    #[test]
    fn the_run_headers_position_never_goes_backwards_within_one_run() {
        let mut app = App::from_rows(rooted_rows());

        app.set_pact_in_flight(Path::new("/repo").join("crates"), 3, 12);
        assert_eq!(
            app.run_header()
                .expect("a run in flight has a header")
                .position(),
            3
        );

        // A position lower than one already seen leaves the fill where it is: a
        // bar that goes backwards mid-run is a bar that is reporting the event
        // order rather than the progress.
        app.set_pact_in_flight(Path::new("/repo").join("crates"), 2, 12);
        let header = app.run_header().expect("a run in flight has a header");
        assert_eq!(header.position(), 3);
        assert_eq!(header.total(), 12);
        // The footer is untouched by the high-water mark and goes on reporting
        // the directory and position it was just handed.
        assert_eq!(app.pact_line().as_deref(), Some("pacting crates (2/12)"));

        // And it goes on rising the moment the run gets past where it had been.
        app.set_pact_in_flight(
            Path::new("/repo").join("crates").join("warlock-engine"),
            5,
            12,
        );
        assert_eq!(
            app.run_header()
                .expect("a run in flight has a header")
                .position(),
            5
        );
    }

    #[test]
    fn a_fresh_run_starts_the_headers_fraction_again() {
        let mut app = App::from_rows(rooted_rows());

        app.set_pact_in_flight(Path::new("/repo").join("crates"), 12, 12);
        app.clear_pact_in_flight();

        // The high-water mark went with the record that held it, so run two is
        // one of twelve rather than a bar that starts full.
        app.set_run_in_flight(Run::Refresh, Path::new("/repo").join("crates"), 1, 12);
        let header = app.run_header().expect("a run in flight has a header");
        assert_eq!(header.run(), Run::Refresh);
        assert_eq!(header.position(), 1);
        assert_eq!(header.total(), 12);
    }

    #[test]
    fn the_app_says_which_row_is_the_one_being_worked() {
        let mut app = App::from_rows(rooted_rows());
        let engine = Path::new("/repo").join("crates").join("warlock-engine");

        // Nothing is in flight, so no row is.
        assert!(!app.is_in_flight(&engine));

        app.set_pact_in_flight(engine.clone(), 3, 12);

        // Exactly the one directory: not the root above it, not the parent, and
        // not anything beneath it.
        assert!(app.is_in_flight(&engine));
        assert!(!app.is_in_flight(Path::new("/repo")));
        assert!(!app.is_in_flight(&Path::new("/repo").join("crates")));
        assert!(!app.is_in_flight(&engine.join("src")));

        // And it follows the run to the next directory.
        app.set_pact_in_flight(Path::new("/repo").join("crates"), 4, 12);
        assert!(!app.is_in_flight(&engine));
        assert!(app.is_in_flight(&Path::new("/repo").join("crates")));
    }

    #[test]
    fn the_pass_in_flight_covers_its_directory_and_the_files_it_holds() {
        let mut app = App::from_rows(rooted_rows());
        let engine = Path::new("/repo").join("crates").join("warlock-engine");
        let covered = |app: &App, row: &Row| app.in_flight_covers(row);

        // Nothing is in flight, so nothing is covered.
        let directory = Row::new(2, engine.clone(), None::<PathBuf>, NodeState::PactedStale);
        assert!(!covered(&app, &directory));

        app.set_pact_in_flight(engine.clone(), 3, 12);

        // The directory being worked, and the files directly inside it: they
        // are what its pass is reading.
        assert!(covered(&app, &directory));
        assert!(covered(
            &app,
            &Row::file(3, engine.join("Cargo.toml"), NodeState::PactedStale)
        ));

        // Not a child directory — its pass already ran — and not a file that
        // belongs to that pass, however the paths nest.
        assert!(!covered(
            &app,
            &Row::new(
                3,
                engine.join("src"),
                None::<PathBuf>,
                NodeState::PactedStale
            )
        ));
        assert!(!covered(
            &app,
            &Row::file(4, engine.join("src").join("lib.rs"), NodeState::PactedStale)
        ));
        // And not the parent, nor a file beside the directory rather than in
        // it.
        assert!(!covered(
            &app,
            &Row::new(
                1,
                Path::new("/repo").join("crates"),
                None::<PathBuf>,
                NodeState::PactedStale
            )
        ));
        assert!(!covered(
            &app,
            &Row::file(
                2,
                Path::new("/repo").join("crates").join("README.md"),
                NodeState::PactedStale
            )
        ));
    }

    #[test]
    fn no_row_is_in_flight_once_the_run_is_over() {
        let mut app = App::from_rows(rooted_rows());
        let engine = Path::new("/repo").join("crates").join("warlock-engine");

        app.set_pact_in_flight(engine.clone(), 3, 12);
        app.clear_pact_in_flight();

        // However the run ended, the row it was on goes back to being an
        // ordinary row on the next frame.
        assert!(!app.is_pacting());
        assert!(!app.is_in_flight(&engine));
    }

    #[test]
    fn a_pact_in_flight_takes_the_message_line_and_hands_it_back() {
        let mut app = App::from_tree(&fixture::tree());
        app.set_message("something the caller said");

        app.set_pact_in_flight("warlock/crates", 2, 4);

        // The progress line outranks the message while the run is on, and the
        // message is kept rather than dropped: starting a pact is not a
        // keystroke, so it neither says anything nor takes anything down.
        assert_eq!(
            app.pact_line().as_deref(),
            Some("pacting warlock/crates (2/4)")
        );
        assert_eq!(app.message(), Some("something the caller said"));

        app.clear_pact_in_flight();

        assert_eq!(app.pact_line(), None);
        assert_eq!(app.message(), Some("something the caller said"));
    }

    #[test]
    fn a_keystroke_clears_a_message_and_leaves_the_pact_in_flight_alone() {
        let keystrokes: [(&str, Movement); 9] = [
            ("select_next", App::select_next),
            ("select_previous", App::select_previous),
            ("select_page_down", App::select_page_down),
            ("select_page_up", App::select_page_up),
            ("select_first", App::select_first),
            ("select_last", App::select_last),
            ("toggle_collapsed", App::toggle_collapsed),
            ("toggle_pacted_only", App::toggle_pacted_only),
            ("toggle_files", App::toggle_files),
        ];

        for (name, keystroke) in keystrokes {
            // `warlock/crates` has children, so the collapse key is a keystroke
            // that does something here rather than a no-op.
            let mut app = app_selecting("warlock/crates");
            app.set_message("something to forget");
            app.set_pact_in_flight("warlock/crates/engine", 3, 12);

            keystroke(&mut app);

            assert_eq!(app.message(), None, "{name} left the message behind");
            // The pact goes on running however much the reader scrolls, so the
            // line describing it goes on being true.
            assert!(app.is_pacting(), "{name} stopped the pact");
            assert_eq!(
                app.pact_line().as_deref(),
                Some("pacting warlock/crates/engine (3/12)"),
                "{name} blanked the line for a pact that is still running"
            );
        }
    }

    #[test]
    fn a_press_refused_by_a_running_pact_is_said_on_the_end_of_the_pact_line() {
        let mut app = app_selecting("warlock/crates");
        app.set_pact_in_flight("warlock/crates/engine", 3, 12);

        app.set_pact_refused();

        assert_eq!(
            app.pact_line().as_deref(),
            Some("pacting warlock/crates/engine (3/12) — already running")
        );

        // A second, third and fourth press say the same thing, because there is
        // nothing further to say.
        let after_one = app.clone();
        app.set_pact_refused();
        app.set_pact_refused();
        app.set_pact_refused();
        assert_eq!(app, after_one, "pressing again changed something");
    }

    #[test]
    fn the_run_moving_on_re_words_the_refusal_and_keeps_it() {
        let mut app = app_selecting("warlock/crates");
        app.set_pact_in_flight("warlock/crates/engine", 3, 12);
        app.set_pact_refused();

        // The tick that lands a moment after the press: it says where the run is
        // now, and says nothing about the keystroke either way.
        app.set_pact_in_flight("warlock/crates/tui", 4, 12);

        assert_eq!(
            app.pact_line().as_deref(),
            Some("pacting warlock/crates/tui (4/12) — already running")
        );
    }

    #[test]
    fn a_keystroke_takes_the_refusal_down_the_way_it_takes_a_message_down() {
        let keystrokes: [(&str, Movement); 9] = [
            ("select_next", App::select_next),
            ("select_previous", App::select_previous),
            ("select_page_down", App::select_page_down),
            ("select_page_up", App::select_page_up),
            ("select_first", App::select_first),
            ("select_last", App::select_last),
            ("toggle_collapsed", App::toggle_collapsed),
            ("toggle_pacted_only", App::toggle_pacted_only),
            ("toggle_files", App::toggle_files),
        ];

        for (name, keystroke) in keystrokes {
            let mut app = app_selecting("warlock/crates");
            app.set_pact_in_flight("warlock/crates/engine", 3, 12);
            app.set_pact_refused();

            keystroke(&mut app);

            // The refusal belonged to the key that earned it; the run it was
            // about goes on, and its line goes back to being about the run.
            assert_eq!(
                app.pact_line().as_deref(),
                Some("pacting warlock/crates/engine (3/12)"),
                "{name} left the refusal on the line"
            );
        }
    }

    #[test]
    fn a_refused_press_leaves_the_message_where_it_was() {
        let mut app = App::from_tree(&fixture::tree());
        app.set_message("something the caller said");
        app.set_pact_in_flight("warlock/crates", 2, 4);

        app.set_pact_refused();

        // The refusal is a bit of wording on the progress line and nothing else:
        // it is not routed through the message, so the message is still the last
        // keystroke's.
        assert_eq!(
            app.pact_line().as_deref(),
            Some("pacting warlock/crates (2/4) — already running")
        );
        assert_eq!(app.message(), Some("something the caller said"));

        app.clear_pact_in_flight();

        // And when the run ends the message is handed back untouched, with no
        // sign of the refusal anywhere: the run it was about is over.
        assert_eq!(app.pact_line(), None);
        assert_eq!(app.message(), Some("something the caller said"));
    }

    #[test]
    fn a_refusal_says_nothing_while_no_pact_is_running() {
        let mut app = App::from_tree(&fixture::tree());
        app.set_message("something the caller said");

        app.set_pact_refused();

        // Nothing to be a suffix on, so nothing is said — and the pact key with
        // no run in flight goes on refusing the way it always did, through the
        // message.
        assert_eq!(app.pact_line(), None);
        assert_eq!(app.message(), Some("something the caller said"));
    }

    #[test]
    fn a_fresh_app_has_nothing_collapsed_and_draws_the_whole_walk() {
        let app = App::from_tree(&fixture::tree());

        assert!(app.collapsed().is_empty());
        assert_eq!(drawn(&app), whole_fixture());
        // And it knows which of those rows could be collapsed at all. The
        // document rows drawn beside them stand for files, which the tree knows
        // nothing about and which hold nothing.
        for row in app.rows().iter().filter(|row| !row.is_file()) {
            assert_eq!(
                row.children,
                children_in_fixture(&row.path),
                "child count for {}",
                row.path.display()
            );
            assert_eq!(row.has_children(), row.children > 0);
        }
    }

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

        assert_eq!(drawn(&app), collapsed_over_crates());
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

        assert_eq!(drawn(&app), collapsed_over_crates());
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
        assert_eq!(app.selected(), 7);

        let app = app.with_collapsed(["warlock/crates"]);

        // Four rows fewer above it — two modules and the document row under
        // each — and the same node under it.
        assert_eq!(app.selected(), 3);
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
        assert_eq!(drawn(&rebuilt), collapsed_over_crates());
    }

    #[test]
    fn a_collapsed_path_the_new_tree_has_no_node_for_hides_nothing() {
        let app = App::from_tree(&fixture::tree()).with_collapsed(["warlock/gone"]);

        assert_eq!(drawn(&app), whole_fixture());
        // Kept all the same: a directory that comes back should come back shut.
        assert!(app.is_collapsed("warlock/gone"));
    }

    #[test]
    fn toggling_a_node_with_nothing_under_it_changes_nothing_at_all() {
        // Undocumented as well as childless: every leaf in the shared fixture
        // has a document, and the default view draws that document under it, so
        // a leaf there is a row with something under it now.
        let mut app = select(
            App::from_rows(vec![
                Row::new(0, "repo", "repo/WARLOCK.md", NodeState::PactedStale).with_child_count(1),
                Row::new(1, "repo/crates", None, NodeState::Unpacted),
            ]),
            "repo/crates",
        );
        app.set_message("something from the last keystroke");
        let before = app.clone();

        app.toggle_collapsed();

        // Including the message: a key that did nothing should not look like a
        // key that did something.
        assert_eq!(app, before);
        assert!(app.collapsed().is_empty());
        assert_eq!(drawn(&app), ["repo", "repo/crates"]);
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
        app.toggle_pact().expect("tui has a document");
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

    fn window_is_in_range(app: &App) -> bool {
        app.scroll_offset() <= app.rows().len().saturating_sub(app.viewport_height())
    }

    #[test]
    fn a_collapse_clears_the_last_keystrokes_message() {
        // The root, which has both something to say when it is un-pacted and
        // children to collapse.
        let mut app = app_selecting("warlock");
        app.toggle_pact().expect("a directory can be un-pacted");
        assert!(app.message().is_some());

        app.toggle_collapsed();

        assert_eq!(app.message(), None);
    }

    #[test]
    fn collapsing_a_directory_leaves_a_sibling_whose_name_it_prefixes_alone() {
        let tree = Tree::new(
            Node::new("repo", "repo/WARLOCK.md", NodeState::PactedStale).with_children([
                Node::new("repo/crates", None, NodeState::Unpacted).with_children([Node::new(
                    "repo/crates/engine",
                    "repo/crates/engine/WARLOCK.md",
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
            Row::new(0, "repo", "repo/WARLOCK.md", NodeState::PactedStale).with_child_count(1),
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
                .filter(|row| !row.is_file() && row.state.is_pacted())
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
        // drawn as their way in, and each document row still sits one level
        // under the directory that owns it: the filter narrows the view, it does
        // not reparent anything.
        assert_eq!(
            seen,
            [
                (0, "warlock"),
                (1, "warlock/WARLOCK.md"),
                (1, "warlock/crates"),
                (2, "warlock/crates/engine"),
                (3, "warlock/crates/engine/WARLOCK.md"),
                (2, "warlock/crates/tui"),
                (3, "warlock/crates/tui/WARLOCK.md"),
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

        // Down to the last row the narrowed view has — `tui`'s document — and
        // then up onto the node holding it, which the wide view puts somewhere
        // else entirely.
        app.select_last();
        assert_eq!(
            app.selected_row().map(|row| row.path.clone()),
            Some(PathBuf::from("warlock/crates/tui/WARLOCK.md"))
        );
        app.select_previous();
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

        // Two rows fewer on screen — `assets` and the document row that goes
        // with it — and the same five nodes tallied: the footer describes the
        // tree, not the view.
        assert_eq!(drawn(&app), pacted_fixture());
        assert_eq!(node_paths(&app).len(), 4);
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
        assert_eq!(drawn(&app), collapsed_over_crates());

        app.toggle_pacted_only();

        // `crates` is unpacted and its descendants are hidden, but it is still
        // the way to them, so it keeps its row: what survives the filter is read
        // off the whole walk, not off what collapsing left drawn.
        assert_eq!(
            drawn(&app),
            ["warlock", "warlock/WARLOCK.md", "warlock/crates"]
        );
        assert!(app.is_collapsed("warlock/crates"));

        app.toggle_pacted_only();

        assert!(app.is_collapsed("warlock/crates"));
        assert_eq!(drawn(&app), collapsed_over_crates());

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
            Row::new(0, "repo", "repo/WARLOCK.md", NodeState::Unpacted).with_child_count(1),
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
        app.toggle_pact().expect("assets has a document");

        app.toggle_pacted_only();

        // Newly pacted, so newly worth drawing — and the document row under it
        // comes back with it, carrying the state that saved them both: the
        // filter reads the states as they are now, not as the tree was loaded
        // with them.
        assert_eq!(
            drawn(&app),
            [
                "warlock",
                "warlock/WARLOCK.md",
                "warlock/crates",
                "warlock/crates/engine",
                "warlock/crates/engine/WARLOCK.md",
                "warlock/crates/tui",
                "warlock/crates/tui/WARLOCK.md",
                "warlock/assets",
                "warlock/assets/WARLOCK.md",
            ]
        );
    }

    #[test]
    fn an_unpacted_subtree_goes_whole_while_its_unpacted_siblings_way_in_stays() {
        let tree = Tree::new(Node::new("repo", None, NodeState::Unpacted).with_children([
            Node::new("repo/kept", None, NodeState::Unpacted).with_children([Node::new(
                "repo/kept/deep",
                "repo/kept/deep/WARLOCK.md",
                NodeState::PactedFresh,
            )]),
            Node::new("repo/gone", None, NodeState::Unpacted).with_children([Node::new(
                "repo/gone/deep",
                "repo/gone/deep/WARLOCK.md",
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
        let mut app = app_with_files_selecting("warlock/assets/logo.svg");
        assert_eq!(app.toggle_pact(), None);
        assert!(app.message().is_some());

        app.toggle_pacted_only();

        assert_eq!(app.message(), None);
    }

    #[test]
    fn a_fresh_app_draws_the_directories_and_no_file_but_each_ones_document() {
        let app = App::from_tree(&fixture::tree());

        assert!(!app.show_files());
        assert_eq!(drawn(&app), whole_fixture());
        // Every file row drawn is the document of the directory above it, and
        // no other file the fixture lists is anywhere on screen.
        assert!(app.rows().iter().filter(|row| row.is_file()).count() > 0);
        assert!(
            app.rows()
                .iter()
                .filter(|row| row.is_file())
                .all(Row::is_document)
        );
        for held in ["warlock/README.md", "warlock/assets/logo.svg"] {
            assert!(
                !drawn(&app).iter().any(|path| path == held),
                "{held} is held by a directory, not written by Warlock"
            );
        }
        // And they are hidden rather than absent: the fixture holds them.
        assert!(
            fixture::tree()
                .walk()
                .any(|(node, _)| !node.files.is_empty())
        );
    }

    #[test]
    fn a_documented_directory_draws_its_document_beneath_it_and_an_undocumented_one_draws_nothing()
    {
        // A documented root, an undocumented directory under it, and a
        // documented one a `.warlockignore` excludes.
        let tree = Tree::new(
            Node::new("repo", "repo/WARLOCK.md", NodeState::PactedFresh)
                .with_files([
                    PathBuf::from("repo/README.md"),
                    PathBuf::from("repo/WARLOCK.md"),
                ])
                .with_children([
                    Node::new("repo/crates", None, NodeState::Unpacted),
                    Node::new("repo/vendor", "repo/vendor/WARLOCK.md", NodeState::Unpacted)
                        .with_files([PathBuf::from("repo/vendor/WARLOCK.md")])
                        .with_ignored(true),
                ]),
        );

        let app = App::from_tree(&tree);

        assert!(!app.show_files());
        assert_eq!(
            drawn(&app),
            [
                "repo",
                "repo/WARLOCK.md",
                // Nothing under `crates`, which has no document to draw.
                "repo/crates",
                "repo/vendor",
                "repo/vendor/WARLOCK.md",
            ]
        );
        // Exactly the row `f` draws for the same file: a file row, one level
        // under its directory, carrying that directory's state and its
        // exclusion flag, and documenting nothing itself.
        for (directory, document) in [(0, 1), (3, 4)] {
            let directory = app.rows()[directory].clone();
            let document = &app.rows()[document];
            assert_eq!(document.path, directory.path.join("WARLOCK.md"));
            assert_eq!(
                Some(document.path.as_path()),
                directory.document.as_deref(),
                "the row is the document the node names"
            );
            assert_eq!(document.depth, directory.depth + 1);
            assert!(document.is_file());
            assert!(document.is_document());
            assert_eq!(document.state, directory.state);
            assert_eq!(document.is_ignored(), directory.is_ignored());
            assert_eq!(document.document, None);
            assert!(!document.has_children());
        }
        assert!(app.rows()[4].is_ignored(), "the excluded one is excluded");

        // And it is the same row, field for field, as the one the file toggle
        // shows: hiding the files changes which rows are kept, not what a row is.
        let mut shown = App::from_tree(&tree);
        shown.toggle_files();
        for row in app.rows().iter().filter(|row| row.is_file()) {
            assert!(
                shown.rows().contains(row),
                "{} is not the row `f` draws",
                row.path.display()
            );
        }
    }

    #[test]
    fn the_filter_keeps_a_pacted_directorys_document_and_drops_a_leftover_one() {
        // `repo/left` is gray with a `WARLOCK.md` an un-pact left on disk, and
        // is drawn only as the way down to the pacted node below it.
        let tree = Tree::new(
            Node::new("repo", None, NodeState::Unpacted).with_children([Node::new(
                "repo/left",
                "repo/left/WARLOCK.md",
                NodeState::Unpacted,
            )
            .with_files([PathBuf::from("repo/left/WARLOCK.md")])
            .with_children([Node::new(
                "repo/left/deep",
                "repo/left/deep/WARLOCK.md",
                NodeState::PactedFresh,
            )
            .with_files([PathBuf::from("repo/left/deep/WARLOCK.md")])])]),
        );
        let mut app = App::from_tree(&tree);
        assert_eq!(
            drawn(&app),
            [
                "repo",
                "repo/left",
                "repo/left/WARLOCK.md",
                "repo/left/deep",
                "repo/left/deep/WARLOCK.md",
            ]
        );

        app.toggle_pacted_only();

        // The pacted directory keeps its document; the gray one keeps its own
        // row as the way in and loses the leftover document, which carries that
        // gray state and so has nothing to save it — and could not have saved
        // the directory above it either.
        assert_eq!(
            drawn(&app),
            [
                "repo",
                "repo/left",
                "repo/left/deep",
                "repo/left/deep/WARLOCK.md"
            ]
        );
    }

    #[test]
    fn collapsing_a_documented_directory_takes_its_document_row_with_it() {
        let mut app = app_selecting("warlock/crates/engine");

        app.toggle_collapsed();

        // The directory keeps its row and its document goes with the rest of
        // what was under it, because that is where the document row sits.
        assert_eq!(
            drawn(&app),
            [
                "warlock",
                "warlock/WARLOCK.md",
                "warlock/crates",
                "warlock/crates/engine",
                "warlock/crates/tui",
                "warlock/crates/tui/WARLOCK.md",
                "warlock/assets",
                "warlock/assets/WARLOCK.md",
            ]
        );

        app.toggle_collapsed();

        assert_eq!(drawn(&app), whole_fixture());
    }

    #[test]
    fn a_documented_childless_directory_collapses_with_the_files_hidden() {
        // `warlock/assets` has no child directories, and the tree still says
        // so — but the default view now draws its document under it, and a row
        // with something under it is a row the collapse key has work to do on.
        let mut app = app_selecting("warlock/assets");

        assert!(app.selected_row().is_some_and(|row| !row.has_children()));
        assert!(app.can_collapse(app.selected()));

        app.toggle_collapsed();

        // Everything but the last row: `assets` keeps its place and its
        // document is hidden under it.
        assert_eq!(drawn(&app), whole_fixture()[..8]);
        assert!(app.is_collapsed("warlock/assets"));

        app.toggle_collapsed();

        assert_eq!(drawn(&app), whole_fixture());
    }

    #[test]
    fn showing_files_puts_each_one_under_its_directory_one_level_deeper() {
        let mut app = App::from_tree(&fixture::tree());

        app.toggle_files();

        assert!(app.show_files());
        assert_eq!(drawn(&app), whole_fixture_with_files());
        let seen: Vec<(usize, &str, bool)> = app
            .rows()
            .iter()
            .map(|row| {
                (
                    row.depth,
                    row.path.to_str().expect("ascii path"),
                    row.is_file(),
                )
            })
            .collect();
        assert_eq!(
            seen,
            [
                (0, "warlock", false),
                (1, "warlock/README.md", true),
                (1, "warlock/WARLOCK.md", true),
                (1, "warlock/crates", false),
                (2, "warlock/crates/engine", false),
                (3, "warlock/crates/engine/Cargo.toml", true),
                (3, "warlock/crates/engine/WARLOCK.md", true),
                (2, "warlock/crates/tui", false),
                (3, "warlock/crates/tui/WARLOCK.md", true),
                (1, "warlock/assets", false),
                (2, "warlock/assets/WARLOCK.md", true),
                (2, "warlock/assets/logo.svg", true),
            ]
        );
        // Every file row sits one level under the directory that listed it.
        for (index, row) in app.rows().iter().enumerate() {
            if !row.is_file() {
                continue;
            }
            let parent = app.rows()[..index]
                .iter()
                .rposition(|other| !other.is_file())
                .map(|at| &app.rows()[at]);
            let parent = parent.expect("a file row always follows a directory row");
            assert_eq!(row.path.parent(), Some(parent.path.as_path()));
            assert_eq!(row.depth, parent.depth + 1);
        }
    }

    #[test]
    fn hiding_the_files_again_puts_the_app_back_as_it_was() {
        let before = App::from_tree(&fixture::tree());
        let mut app = before.clone();

        app.toggle_files();
        app.toggle_files();

        assert!(!app.show_files());
        assert_eq!(drawn(&app), whole_fixture());
        assert_eq!(app, before);
    }

    #[test]
    fn a_file_row_documents_nothing_contains_nothing_and_takes_its_modules_state() {
        let tree = fixture::tree();
        let mut app = App::from_tree(&tree);

        app.toggle_files();

        for row in app.rows().iter().filter(|row| row.is_file()) {
            assert_eq!(
                row.document,
                None,
                "{} claims a document",
                row.path.display()
            );
            assert_eq!(row.children, 0);
            assert!(!row.has_children());
            let directory = row.path.parent().expect("a file sits in a directory");
            let node = tree.find(directory).expect("the file came from that node");
            assert_eq!(
                row.state,
                node.state,
                "{} is not its module's colour",
                row.path.display()
            );
        }
        // Including files under two different states, so the copy is a copy of
        // the right node rather than of any node.
        assert!(
            app.rows()
                .iter()
                .any(|row| row.is_file() && row.state == NodeState::PactedFresh)
        );
        assert!(
            app.rows()
                .iter()
                .any(|row| row.is_file() && row.state == NodeState::Unpacted)
        );
    }

    #[test]
    fn pacting_a_directory_recolours_the_files_in_it() {
        let mut app = app_with_files_selecting("warlock/assets");

        app.toggle_pact().expect("assets has a document");

        // The directory's own files move with it; a file of another directory
        // does not.
        let state_of = |app: &App, path: &str| {
            app.rows()
                .iter()
                .find(|row| row.path == Path::new(path))
                .map(|row| row.state)
        };
        assert_eq!(
            state_of(&app, "warlock/assets"),
            Some(NodeState::PactedStale)
        );
        assert_eq!(
            state_of(&app, "warlock/assets/WARLOCK.md"),
            Some(NodeState::PactedStale)
        );
        assert_eq!(
            state_of(&app, "warlock/assets/logo.svg"),
            Some(NodeState::PactedStale)
        );
        assert_eq!(
            state_of(&app, "warlock/crates/tui/WARLOCK.md"),
            Some(NodeState::PactedStale),
        );
        assert_eq!(
            state_of(&app, "warlock/crates/engine/WARLOCK.md"),
            Some(NodeState::PactedFresh)
        );

        // And in the list behind the drawn one, or the colour would revert the
        // moment anything re-filtered the rows.
        app.toggle_files();
        app.toggle_files();
        assert_eq!(
            state_of(&app, "warlock/assets/logo.svg"),
            Some(NodeState::PactedStale)
        );
    }

    #[test]
    fn a_file_cannot_be_pacted_and_the_refusal_says_why() {
        let mut app = app_with_files_selecting("warlock/assets/logo.svg");
        let before = app.clone();

        assert_eq!(app.toggle_pact(), None);

        // Everything but the message is exactly as it was: no state change, no
        // count change, and nothing for the caller to write down.
        assert_eq!(app.rows(), before.rows());
        assert_eq!(app.counts(), before.counts());
        assert_eq!(app.selected(), before.selected());
        let message = app.message().expect("a refusal says why");
        assert!(
            message.starts_with("warlock/assets/logo.svg is a file"),
            "{message}"
        );
        // Its own wording, not the missing-document one, though a file has no
        // document either.
        assert!(!message.contains("no WARLOCK.md"), "{message}");
        assert!(message.contains("directory"), "{message}");
    }

    #[test]
    fn a_file_under_a_documented_directory_is_refused_all_the_same() {
        // The document of a documented module, which is the row most likely
        // to be mistaken for the module itself.
        let mut app = app_with_files_selecting("warlock/crates/tui/WARLOCK.md");
        let before = app.clone();

        assert_eq!(app.toggle_pact(), None);

        assert_eq!(app.rows(), before.rows());
        assert_eq!(app.counts(), before.counts());
        assert!(app.message().is_some());
    }

    #[test]
    fn showing_files_leaves_the_engines_tally_alone() {
        let tree = fixture::tree();
        let mut app = App::from_tree(&tree);
        let before = app.counts();
        assert_eq!(before, tally(&app));

        app.toggle_files();

        // More rows on screen and not one more node: the footer counts modules.
        assert!(app.rows().len() > tree.counts().total());
        assert_eq!(app.counts(), before);
        assert_eq!(app.counts(), tree.counts());

        app.toggle_files();

        assert_eq!(app.counts(), before);
        assert_eq!(app.counts(), tally(&app));
    }

    #[test]
    fn pacting_with_files_shown_still_moves_exactly_one_node_in_the_tally() {
        let mut app = app_with_files_selecting("warlock/assets");
        let before = app.counts();

        app.toggle_pact().expect("assets has a document");

        assert_eq!(app.counts().total(), before.total());
        assert_eq!(app.counts().unpacted, before.unpacted - 1);
        assert_eq!(app.counts().pacted_stale, before.pacted_stale + 1);
    }

    #[test]
    fn collapsing_a_directory_hides_its_files_with_it_and_expanding_puts_them_back() {
        let mut leaf = app_with_files_selecting("warlock/assets");

        leaf.toggle_collapsed();

        // `assets` has no child directories, and with the file toggle on it has
        // rows under it all the same. The key hides exactly those: what
        // collapsing asks is what is drawn under the row, not what the tree
        // calls a child.
        assert_eq!(
            drawn(&leaf),
            [
                "warlock",
                "warlock/README.md",
                "warlock/WARLOCK.md",
                "warlock/crates",
                "warlock/crates/engine",
                "warlock/crates/engine/Cargo.toml",
                "warlock/crates/engine/WARLOCK.md",
                "warlock/crates/tui",
                "warlock/crates/tui/WARLOCK.md",
                "warlock/assets",
            ]
        );
        assert_eq!(leaf.collapsed().len(), 1);

        leaf.toggle_collapsed();
        assert_eq!(drawn(&leaf), whole_fixture_with_files());
        assert!(leaf.collapsed().is_empty());

        // A directory with children, on the other hand, takes its own files and
        // its descendants' with it.
        let mut app = app_with_files_selecting("warlock/crates");
        let before = app.rows().to_vec();
        app.toggle_collapsed();

        assert_eq!(
            drawn(&app),
            [
                "warlock",
                "warlock/README.md",
                "warlock/WARLOCK.md",
                "warlock/crates",
                "warlock/assets",
                "warlock/assets/WARLOCK.md",
                "warlock/assets/logo.svg",
            ]
        );

        app.toggle_collapsed();

        // Byte for byte what was there before: same rows, same order, same
        // depths, same states.
        assert_eq!(app.rows(), before);
    }

    #[test]
    fn a_directory_holding_only_files_collapses_exactly_when_the_files_are_shown() {
        // `repo/assets` has no child directories, no document and lists one
        // file, so whether anything is under it is the file toggle's answer and
        // not the tree's. Asking the tree — `Row::children`, which is 0 here —
        // is what used to leave the key doing nothing at all on such a row.
        let tree = Tree::new(
            Node::new("repo", "repo/WARLOCK.md", NodeState::PactedStale)
                .with_children([Node::new("repo/assets", None, NodeState::Unpacted)
                    .with_files([PathBuf::from("repo/assets/logo.svg")])]),
        );
        let mut app = select(App::from_tree(&tree), "repo/assets");
        let selected = app.selected();

        assert!(!app.can_collapse(selected), "nothing is drawn under it");
        assert!(app.selected_row().is_some_and(|row| !row.has_children()));

        app.toggle_collapsed();
        assert!(app.collapsed().is_empty(), "and the key refuses it");

        app.toggle_files();
        let selected = app.selected();

        assert!(app.can_collapse(selected), "now a file row is");
        app.toggle_collapsed();
        assert_eq!(
            app.collapsed().len(),
            1,
            "and the key hides them, on a row the tree still calls childless"
        );
    }

    #[test]
    fn a_directory_the_filter_has_emptied_does_not_collapse() {
        // The mirror of the case above: `repo/crates` has a child in the tree,
        // and the pacted-only view takes it away. A row with nothing under it is
        // a row with nothing to hide, whichever filter emptied it.
        let mut app = App::from_rows(vec![
            Row::new(0, "repo", "repo/WARLOCK.md", NodeState::PactedStale).with_child_count(1),
            Row::new(1, "repo/crates", None, NodeState::Unpacted).with_child_count(1),
            Row::new(2, "repo/crates/tui", None, NodeState::Unpacted),
        ]);

        assert!(app.can_collapse(0), "the whole walk is drawn");

        app.toggle_pacted_only();

        assert_eq!(drawn(&app), ["repo"], "the unpacted pair is filtered out");
        assert!(!app.can_collapse(0), "so the root holds nothing");
        assert!(
            app.selected_row().is_some_and(Row::has_children),
            "though the tree still says it has a child"
        );

        app.toggle_collapsed();
        assert!(app.collapsed().is_empty());
    }

    #[test]
    fn collapsing_the_root_hides_every_file_in_the_tree() {
        let mut app = app_with_files_selecting("warlock");

        app.toggle_collapsed();

        assert_eq!(drawn(&app), ["warlock"]);

        app.toggle_collapsed();

        assert_eq!(drawn(&app), whole_fixture_with_files());
    }

    #[test]
    fn hiding_the_files_under_the_selection_lands_it_on_the_directory() {
        let mut app = app_with_files_selecting("warlock/crates/engine/Cargo.toml");

        app.toggle_files();

        // The file's row is gone — it is an ordinary file, not the document the
        // hidden view keeps — and what it went behind is the directory that
        // held it.
        assert_eq!(
            app.selected_row().map(|row| row.path.clone()),
            Some(PathBuf::from("warlock/crates/engine"))
        );
        assert!(app.selected() < app.rows().len());
    }

    #[test]
    fn the_filter_keeps_the_files_of_pacted_directories_and_no_others() {
        let mut app = App::from_tree(&fixture::tree());
        app.toggle_files();

        app.toggle_pacted_only();

        // `crates/` survives as the way down to the pacted modules, but nothing
        // of it is pacted, so it brings no files with it — and it has none.
        // `assets/` is unpacted and goes whole, its files with it.
        assert_eq!(
            drawn(&app),
            [
                "warlock",
                "warlock/README.md",
                "warlock/WARLOCK.md",
                "warlock/crates",
                "warlock/crates/engine",
                "warlock/crates/engine/Cargo.toml",
                "warlock/crates/engine/WARLOCK.md",
                "warlock/crates/tui",
                "warlock/crates/tui/WARLOCK.md",
            ]
        );

        app.toggle_files();

        // And with the files hidden again the narrowed view is exactly the one
        // it was before files existed.
        assert_eq!(drawn(&app), pacted_fixture());
    }

    #[test]
    fn toggling_the_files_clears_the_last_keystrokes_message() {
        let mut app = app_with_files_selecting("warlock/assets/logo.svg");
        assert_eq!(app.toggle_pact(), None);
        assert!(app.message().is_some());

        app.toggle_files();

        assert_eq!(app.message(), None);
    }

    #[test]
    fn showing_files_leaves_the_window_in_range_with_the_selection_in_it() {
        let mut app = app_selecting("warlock/assets");
        app.set_viewport_height(2);

        app.toggle_files();

        assert!(window_is_in_range(&app));
        assert!(selection_is_on_screen(&app));

        app.toggle_files();

        assert!(window_is_in_range(&app));
        assert!(selection_is_on_screen(&app));
    }

    #[test]
    fn a_bare_list_of_rows_can_hold_files_too_and_starts_with_them_hidden() {
        let app = App::from_rows(vec![
            Row::new(0, "repo", "repo/WARLOCK.md", NodeState::PactedStale).with_child_count(1),
            Row::file(1, "repo/WARLOCK.md", NodeState::PactedStale),
            Row::new(1, "repo/crates", None, NodeState::Unpacted),
        ]);

        assert!(!app.show_files());
        assert_eq!(drawn(&app), ["repo", "repo/crates"]);

        let mut app = app;
        app.toggle_files();
        assert_eq!(drawn(&app), ["repo", "repo/WARLOCK.md", "repo/crates"]);
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

    const MOVEMENTS: [(&str, Movement); 6] = [
        ("select_previous", App::select_previous),
        ("select_next", App::select_next),
        ("select_page_up", App::select_page_up),
        ("select_page_down", App::select_page_down),
        ("select_first", App::select_first),
        ("select_last", App::select_last),
    ];

    fn panel_focused() -> App {
        let mut app = scrolled_to(MANY / 2);
        app.set_focus(Focus::Panel);
        app
    }

    fn composer_focused() -> App {
        let mut app = scrolled_to(MANY / 2);
        app.set_focus(Focus::Composer);
        assert_eq!(
            app.focus(),
            Focus::Composer,
            "the composer refused the keys"
        );
        app
    }

    #[test]
    fn a_fresh_app_has_the_tree_focused() {
        assert_eq!(App::from_rows(three_rows()).focus(), Focus::Tree);
        assert_eq!(App::from_tree(&fixture::tree()).focus(), Focus::Tree);
        assert_eq!(App::default().focus(), Focus::Tree);
    }

    #[test]
    fn the_focus_key_walks_the_tree_the_panel_and_the_composer_and_comes_back() {
        let mut app = App::from_rows(three_rows());

        app.toggle_focus();
        assert_eq!(app.focus(), Focus::Panel);

        app.toggle_focus();
        assert_eq!(app.focus(), Focus::Composer);

        app.toggle_focus();
        assert_eq!(app.focus(), Focus::Tree, "the cycle did not come home");

        // And round again, so the cycle is the cycle rather than three
        // one-off answers.
        for expected in [Focus::Panel, Focus::Composer, Focus::Tree] {
            app.toggle_focus();
            assert_eq!(app.focus(), expected);
        }
    }

    #[test]
    fn the_composer_is_showable_only_while_the_conversation_is_the_card_up() {
        let base = Instant::now();
        let mut app = App::from_rows(three_rows());
        assert!(
            app.panel().composer_showable(),
            "an app opens with room to type"
        );

        // The file gives the panel those rows back, and so does the run: a
        // field under either is a field about the wrong card.
        app.show_document(["a line of a file"], false);
        assert!(
            !app.panel().composer_showable(),
            "the document card hid nothing"
        );

        app.start_account(base);
        assert_eq!(
            app.panel.showing(),
            Showing::Document,
            "the run took the panel"
        );
        assert!(!app.panel().composer_showable());

        app.swap_card();
        assert_eq!(app.panel.showing(), Showing::Thread);
        assert!(
            app.panel().composer_showable(),
            "the conversation gave it back"
        );

        app.swap_card();
        assert_eq!(app.panel.showing(), Showing::Account);
        assert!(
            !app.panel().composer_showable(),
            "the run drew a field of its own"
        );
    }

    #[test]
    fn a_run_takes_the_panel_only_from_a_card_with_nothing_on_it() {
        let base = Instant::now();

        // Nothing said and nothing read: the panel is warlock's mark, and the
        // run the reader just asked for is worth more than that.
        let mut app = App::from_rows(three_rows());
        app.panel_mut().set_height(PANEL);
        app.start_account(base);
        assert_eq!(app.panel.showing(), Showing::Account);

        // A conversation on screen is something they chose to look at, and a
        // run started behind it leaves it exactly where it is.
        let mut app = App::from_rows(three_rows());
        app.panel_mut().set_height(PANEL);
        ask_and_answer(&mut app, base);
        app.start_account(at(base, 10));
        assert_eq!(app.panel.showing(), Showing::Thread);

        // And so is a file.
        let mut app = App::from_rows(three_rows());
        app.panel_mut().set_height(PANEL);
        app.show_document(document_lines(), false);
        app.start_account(at(base, 10));
        assert_eq!(app.panel.showing(), Showing::Document);
    }

    #[test]
    fn a_run_taking_an_empty_panel_takes_the_keyboard_off_the_field_with_it() {
        // The account draws no field, so a keyboard left pointing at one would
        // be pointing at nothing — the same rescue a document does.
        let mut app = App::from_rows(three_rows());
        app.panel_mut().set_height(PANEL);
        app.set_focus(Focus::Composer);
        assert_eq!(app.focus(), Focus::Composer);

        app.start_account(Instant::now());

        assert_eq!(app.panel.showing(), Showing::Account);
        assert!(!app.panel().composer_showable());
        assert_eq!(
            app.focus(),
            Focus::Panel,
            "the keys stayed on a hidden field"
        );
    }

    #[test]
    fn the_focus_key_skips_the_composer_while_the_document_card_is_up() {
        let mut app = App::from_rows(three_rows());
        app.show_document(["a line of a file"], false);

        // Two places and no third: the field is not on screen, so tabbing
        // through it would point the keyboard at nothing.
        for expected in [Focus::Panel, Focus::Tree, Focus::Panel, Focus::Tree] {
            app.toggle_focus();
            assert_eq!(app.focus(), expected, "the cycle stopped at a hidden field");
        }
    }

    #[test]
    fn showing_the_document_takes_the_focus_off_the_composer() {
        let mut app = composer_focused();

        app.show_document(["a line of a file"], false);

        // The panel, because the document that hid the composer is drawn there:
        // the next movement key is about the thing the reader just asked for.
        assert_eq!(
            app.focus(),
            Focus::Panel,
            "the keys stayed on a hidden field"
        );
    }

    #[test]
    fn swapping_to_the_document_takes_the_focus_off_the_composer() {
        let mut app = composer_focused();
        // A document to swap to, read while the tree had the keys, then the
        // keyboard put back on the composer over the conversation.
        app.show_document(["a line of a file"], false);
        app.swap_card();
        app.set_focus(Focus::Composer);
        assert_eq!(app.focus(), Focus::Composer, "the thread card allows it");

        app.swap_card();

        assert_eq!(app.focus(), Focus::Panel, "the swap left the keys nowhere");
    }

    #[test]
    fn a_click_cannot_put_the_keys_on_a_composer_that_is_not_on_screen() {
        let mut app = App::from_rows(three_rows());
        app.show_document(["a line of a file"], false);

        app.set_focus(Focus::Composer);

        assert_eq!(app.focus(), Focus::Panel, "a hidden field took the keys");
    }

    #[test]
    fn a_run_rolled_back_under_a_document_leaves_the_keys_somewhere_they_can_be() {
        // The copy is taken with the composer focused; the run puts a document
        // up behind it and then records nothing, so the pre-run focus meets a
        // panel that has moved on.
        let mut app = composer_focused();
        let before = app.clone();
        app.show_document(["a line of a file"], false);

        app.restore_from(before);

        assert!(
            !app.panel().composer_showable(),
            "the panel came back with the app"
        );
        assert_eq!(
            app.focus(),
            Focus::Panel,
            "the keys came back to a hidden field"
        );
    }

    #[test]
    fn no_movement_key_moves_anything_while_the_composer_has_the_focus() {
        // The keyboard is the draft's, so a movement key is a character and
        // never arrives here; if one does, it moves neither pane. An account of
        // more lines than the panel is tall and a selection in the middle of the
        // tree, so both windows have somewhere to go if anything lets them.
        for (name, movement) in MOVEMENTS {
            let base = Instant::now();
            let mut app = app_pacting(9, base);
            // The conversation, which is the card the field is drawn under, with
            // enough on it that the panel's window has somewhere to go.
            ask_and_answer(&mut app, base);
            app.scroll_panel_up(3);
            app.set_focus(Focus::Tree);
            app.select_next();
            app.set_focus(Focus::Composer);
            assert_eq!(app.focus(), Focus::Composer, "the thread card allows it");
            let before = app.clone();

            movement(&mut app);

            assert_eq!(
                app.selected(),
                before.selected(),
                "{name} moved the selection"
            );
            assert_eq!(
                app.scroll_offset(),
                before.scroll_offset(),
                "{name} scrolled"
            );
            assert_eq!(
                app.panel().scroll_offset(),
                before.panel().scroll_offset(),
                "{name} moved the panel's window"
            );
            assert_eq!(app.focus(), Focus::Composer, "{name} moved the focus");
            assert_eq!(app, before, "{name} changed something at the composer");
        }
    }

    #[test]
    fn no_movement_key_moves_the_tree_while_the_panel_has_the_focus() {
        for (name, movement) in MOVEMENTS {
            let mut app = panel_focused();
            let (selected, offset) = (app.selected(), app.scroll_offset());

            movement(&mut app);

            assert_eq!(app.selected(), selected, "{name} moved the selection");
            assert_eq!(app.scroll_offset(), offset, "{name} moved the window");
            assert_eq!(app.focus(), Focus::Panel, "{name} moved the focus");
        }
    }

    #[test]
    fn a_movement_key_at_the_panel_leaves_the_last_keystrokes_message_up() {
        // The key moved the panel's window, which is not the tree column and has
        // nothing to report about the last keystroke; sweeping the line away
        // would take down the explanation of the key that did something there.
        for (name, movement) in MOVEMENTS {
            let mut app = panel_focused();
            app.set_message("something to keep");

            movement(&mut app);

            assert_eq!(app.message(), Some("something to keep"), "{name} said it");
        }
    }

    #[test]
    fn the_movement_keys_move_again_once_the_tree_has_the_focus_back() {
        for (name, movement) in MOVEMENTS {
            let mut app = panel_focused();
            let mut expected = scrolled_to(MANY / 2);
            movement(&mut expected);

            movement(&mut app);
            // Round the rest of the cycle, past the composer, to the tree.
            app.toggle_focus();
            app.toggle_focus();
            assert_eq!(app.focus(), Focus::Tree, "the cycle went somewhere else");
            movement(&mut app);

            assert_eq!(app.selected(), expected.selected(), "{name} moved oddly");
            assert_eq!(app.scroll_offset(), expected.scroll_offset(), "{name}");
        }
    }

    #[test]
    fn moving_the_focus_moves_nothing_else() {
        let mut app = scrolled_to(MANY / 2);
        app.set_message("something to keep");
        let before = app.clone();

        app.toggle_focus();

        assert_eq!(app.selected(), before.selected());
        assert_eq!(app.scroll_offset(), before.scroll_offset());
        assert_eq!(app.message(), Some("something to keep"));
        // All the way round rather than back the way it came, so the whole cycle
        // is covered by the comparison.
        app.toggle_focus();
        app.toggle_focus();
        assert_eq!(app, before, "the focus key changed something else");
    }

    #[test]
    fn nothing_but_a_movement_key_cares_which_pane_has_the_focus() {
        type Change = fn(&mut App);
        let changes: [(&str, Change); 8] = [
            ("toggle_collapsed", App::toggle_collapsed),
            ("toggle_pacted_only", App::toggle_pacted_only),
            ("toggle_files", App::toggle_files),
            ("toggle_pact", |app| {
                app.toggle_pact();
            }),
            ("set_pact_in_flight", |app| {
                app.set_pact_in_flight("warlock/crates", 2, 5);
            }),
            ("clear_pact_in_flight", |app| {
                app.set_pact_in_flight("warlock/crates", 2, 5);
                app.clear_pact_in_flight();
            }),
            ("set_subtree_state", |app| {
                app.set_subtree_state("warlock/crates", NodeState::PactedFresh);
            }),
            ("set_viewport_height", |app| app.set_viewport_height(2)),
        ];

        for (name, change) in changes {
            let mut focused = app_selecting("warlock/crates");
            let mut unfocused = app_selecting("warlock/crates");
            unfocused.toggle_focus();

            change(&mut focused);
            change(&mut unfocused);
            // Put the focus back rather than exempting the field, so the
            // comparison covers every other field there is.
            unfocused.set_focus(focused.focus());

            assert_eq!(unfocused, focused, "{name} depends on the focus");
        }
    }

    const PANEL: u16 = 3;

    fn at(base: Instant, seconds: u64) -> Instant {
        base + Duration::from_secs(seconds)
    }

    fn app_pacting(lines: usize, base: Instant) -> App {
        let mut app = App::from_rows(three_rows());
        app.panel_mut().set_height(PANEL);
        app.toggle_focus();
        app.start_account(base);

        let account = app
            .panel_mut()
            .account_mut()
            .expect("a run has just started");
        account.open_section("crates/engine", base);
        for line in 0..lines {
            account.record(
                &Activity::Tool {
                    name: "Read".to_owned(),
                    detail: Some(format!("line {line}")),
                },
                at(base, line as u64 + 1),
            );
        }
        app
    }

    fn panel_text(app: &App, now: Instant) -> Vec<String> {
        app.panel()
            .window(now)
            .into_iter()
            .map(|line| match line {
                Line::Directory { path } => path.display().to_string(),
                // A document's line is its own text and nothing else, which is
                // exactly what a test asserting on what is drawn wants back.
                Line::Clocked { text, .. }
                | Line::Summary { text }
                | Line::Text { text }
                | Line::Said { text }
                | Line::Note { text }
                | Line::Wrapped { text, .. } => text,
            })
            .collect()
    }

    #[test]
    fn an_app_that_has_run_no_pact_has_no_account_and_draws_no_panel() {
        let now = Instant::now();

        for app in [
            App::default(),
            App::from_rows(three_rows()),
            App::from_tree(&fixture::tree()),
        ] {
            let mut app = app;
            app.panel_mut().set_height(PANEL);

            assert!(!app.panel().has_account());
            assert_eq!(app.panel().account(), None);
            assert_eq!(app.panel().window(now), Vec::new());
            assert_eq!(app.panel().lines_below(), 0);
            assert_eq!(app.panel().scroll_offset(), 0);

            // Not even a movement key can make a panel with no account say
            // something.
            app.toggle_focus();
            for (_, movement) in MOVEMENTS {
                movement(&mut app);
            }
            assert!(!app.panel().has_account());
            assert_eq!(app.panel().window(now), Vec::new());
        }
    }

    #[test]
    fn a_second_pact_starts_the_account_again_from_empty() {
        let base = Instant::now();
        let mut app = app_pacting(9, base);
        assert_eq!(app.panel().account().map(Account::line_count), Some(10));

        app.start_account(at(base, 100));

        assert!(app.panel().has_account());
        assert_eq!(app.panel().account().map(Account::line_count), Some(0));
        assert_eq!(app.panel().window(at(base, 100)), Vec::new());
        assert_eq!(app.panel().scroll_offset(), 0);
        assert!(app.panel().follows());
    }

    #[test]
    fn a_second_pact_puts_a_scrolled_back_panel_at_the_top_of_the_new_run() {
        let base = Instant::now();
        let mut app = app_pacting(9, base);
        app.select_first();
        assert!(!app.panel().follows());

        app.start_account(at(base, 100));
        app.panel_mut()
            .account_mut()
            .expect("a run has just started")
            .open_section("crates/tui", at(base, 100));

        assert!(app.panel().follows());
        assert_eq!(
            panel_text(&app, at(base, 100)),
            ["crates/tui".to_owned(), "waiting".to_owned()]
        );
    }

    #[test]
    fn the_panel_keeps_the_newest_line_on_the_bottom_row_while_it_follows() {
        let base = Instant::now();
        let mut app = app_pacting(2, base);

        // Shorter than the panel: everything is on screen and there is nothing
        // to scroll.
        assert_eq!(app.panel().scroll_offset(), 0);
        assert_eq!(
            panel_text(&app, at(base, 3)),
            ["crates/engine", "Read line 0", "Read line 1"],
        );

        // Longer than the panel: the window is the last screenful, and moves as
        // each line arrives without anybody telling it to. Numbered tools
        // rather than thinking, because a stretch of thinking is one line
        // however often it is reported and this test needs several.
        for line in 2..6 {
            app.panel_mut()
                .account_mut()
                .expect("a run is under way")
                .record(
                    &Activity::Tool {
                        name: "Read".to_owned(),
                        detail: Some(format!("line {line}")),
                    },
                    at(base, line + 1),
                );
        }

        assert!(app.panel().follows());
        assert_eq!(app.panel().scroll_offset(), 7 - usize::from(PANEL));
        assert_eq!(app.panel().lines_below(), 0);
        assert_eq!(
            panel_text(&app, at(base, 7)),
            ["Read line 3", "Read line 4", "Read line 5"],
        );
    }

    #[test]
    fn scrolling_up_in_the_panel_stops_it_following() {
        let base = Instant::now();
        let mut app = app_pacting(9, base);
        assert_eq!(
            panel_text(&app, at(base, 9)).first().unwrap(),
            "Read line 6"
        );

        app.select_previous();

        assert!(!app.panel().follows());
        let parked = panel_text(&app, at(base, 9));
        assert_eq!(parked, ["Read line 5", "Read line 6", "Read line 7"]);

        // And the lines that arrive afterwards leave the window where it is.
        for line in 9..20 {
            app.panel_mut()
                .account_mut()
                .expect("a run is under way")
                .record(&Activity::Thinking, at(base, line + 1));
        }

        assert_eq!(panel_text(&app, at(base, 30)), parked);
        assert_eq!(app.panel().scroll_offset(), 6);
    }

    #[test]
    fn the_end_of_list_key_puts_the_panel_back_on_live() {
        let base = Instant::now();
        let mut app = app_pacting(9, base);
        app.select_first();
        assert!(!app.panel().follows());
        assert_eq!(app.panel().scroll_offset(), 0);

        app.select_last();

        assert!(app.panel().follows());
        assert_eq!(
            panel_text(&app, at(base, 9)),
            ["Read line 6", "Read line 7", "Read line 8"],
        );

        // Live means live: the next line to arrive moves the window again.
        app.panel_mut()
            .account_mut()
            .expect("a run is under way")
            .record(&Activity::Thinking, at(base, 10));

        assert_eq!(
            panel_text(&app, at(base, 10)),
            ["Read line 7", "Read line 8", "thinking"],
        );
    }

    #[test]
    fn scrolling_back_down_to_the_end_is_following_again() {
        let base = Instant::now();
        let mut app = app_pacting(9, base);

        app.select_page_up();
        assert!(!app.panel().follows());

        app.select_next();
        app.select_next();
        assert!(!app.panel().follows());

        app.select_next();

        assert!(app.panel().follows());
        assert_eq!(app.panel().lines_below(), 0);
    }

    #[test]
    fn the_panel_says_how_many_lines_are_below_the_view() {
        let base = Instant::now();
        let mut app = app_pacting(9, base);

        // Ten lines, three of them on screen at the end: nothing below.
        assert_eq!(app.panel().lines_below(), 0);

        app.select_first();
        assert_eq!(app.panel().lines_below(), 10 - usize::from(PANEL));

        app.select_next();
        assert_eq!(app.panel().lines_below(), 10 - usize::from(PANEL) - 1);

        app.select_last();
        assert_eq!(app.panel().lines_below(), 0);
    }

    #[test]
    fn a_panel_nobody_has_measured_has_nothing_to_scroll() {
        let base = Instant::now();
        let mut app = app_pacting(9, base);
        app.panel_mut().set_height(0);

        // No height means no window: nothing is drawn, so the honest offset is
        // the top however long the account is, and no movement key can push it
        // off one — the same reading the tree's rule takes of a viewport of
        // zero rows.
        for (name, movement) in MOVEMENTS {
            movement(&mut app);
            assert_eq!(
                app.panel().scroll_offset(),
                0,
                "{name} moved a panel of no height"
            );
            assert_eq!(app.panel().window(at(base, 9)), Vec::new(), "{name}");
            // Nothing is drawn, so the whole account is below what is drawn.
            assert_eq!(app.panel().lines_below(), 10, "{name}");
        }

        // And the window is measurable again the moment somebody measures it.
        app.panel_mut().set_height(PANEL);
        assert_eq!(app.panel().scroll_offset(), 10 - usize::from(PANEL));
    }

    #[test]
    fn the_panel_window_rule_follows_the_end_or_stays_where_it_was_put() {
        // Following: the last screenful, whatever the account's length.
        assert_eq!(panel_offset_for(10, 3, 0, true), 7);
        assert_eq!(panel_offset_for(11, 3, 99, true), 8);
        // Parked: the reader's own offset, clamped to what is there.
        assert_eq!(panel_offset_for(10, 3, 4, false), 4);
        assert_eq!(panel_offset_for(10, 3, 99, false), 7);
        // An account that fits, and a panel nobody has measured, are both the
        // top — and following one is the same as being parked at it.
        for following in [true, false] {
            assert_eq!(panel_offset_for(3, 3, 2, following), 0);
            assert_eq!(panel_offset_for(0, 3, 2, following), 0);
            assert_eq!(panel_offset_for(10, 0, 2, following), 0);
        }
    }

    #[test]
    fn the_run_headers_row_costs_the_window_one_line_and_the_scrollback_counts_it() {
        // What a panel has inside its border at one fixed terminal size, and
        // what a frame with a run in flight leaves the account underneath the
        // header. How many rows a header really takes is [`crate::ui`]'s
        // `RUN_HEADER_HEIGHT` and is asserted over there; one is enough here,
        // where the question is what a shorter window does, so nothing is
        // measured and no frame is drawn.
        const WHOLE: usize = 6;
        const HEADER: usize = 1;

        let base = Instant::now();
        let mut below_at_top = Vec::new();
        for (name, window, in_flight) in [
            ("with no run in flight", WHOLE, false),
            ("under a run's header", WHOLE - HEADER, true),
        ] {
            // The same account either way — twelve lines, the section heading
            // included — so the only difference between the two passes is the
            // row the header took off the window.
            let mut app = app_pacting(11, base);
            let lines = app
                .panel()
                .account()
                .map(Account::line_count)
                .expect("a run has started");
            assert_eq!(lines, 12);
            if in_flight {
                app.set_run_in_flight(Run::Pact, "crates/engine", 2, 5);
            }
            app.panel_mut()
                .set_height(u16::try_from(window).expect("a window this small"));

            // Following the newest line: the window is the last screenful of
            // the window it was given, and nothing is below it however many
            // rows the header took.
            assert!(app.panel().follows(), "{name}");
            assert_eq!(
                app.panel().scroll_offset(),
                panel_offset_for(lines, window, 0, true),
                "{name}"
            );
            assert_eq!(app.panel().scroll_offset(), lines - window, "{name}");
            assert_eq!(app.panel().lines_below(), 0, "{name}");
            assert_eq!(app.panel().window(at(base, 99)).len(), window, "{name}");

            // Parked at the first line: what is below is everything the shorter
            // window does not cover, counted against that window rather than
            // against the one the panel would have had with no header on it.
            app.select_first();
            assert!(!app.panel().follows(), "{name}");
            assert_eq!(
                app.panel().scroll_offset(),
                panel_offset_for(lines, window, 0, false),
                "{name}"
            );
            assert_eq!(app.panel().lines_below(), lines - window, "{name}");
            below_at_top.push(app.panel().lines_below());

            // Parked in the middle: what is above the window, what is drawn in
            // it and what is below it come to the account, so the header's row
            // is neither counted twice nor lost between the three.
            app.select_next();
            app.select_next();
            let offset = app.panel().scroll_offset();
            assert_eq!(offset, 2, "{name}");
            assert_eq!(
                offset,
                panel_offset_for(lines, window, offset, false),
                "{name}"
            );
            assert_eq!(
                offset + app.panel().window(at(base, 99)).len() + app.panel().lines_below(),
                lines,
                "{name}"
            );

            // And the count is off by exactly nothing: a line at a time down
            // reaches the end of the account after that many presses, and the
            // window is following again when it gets there.
            let below = app.panel().lines_below();
            for step in 1..=below {
                app.select_next();
                assert_eq!(
                    app.panel().lines_below(),
                    below - step,
                    "{name}, {step} down"
                );
            }
            assert!(app.panel().follows(), "{name}");

            // An offset past the end is clamped to the last screenful of the
            // window there is, header or no header.
            assert_eq!(
                panel_offset_for(lines, window, lines * 2, false),
                lines - window,
                "{name}"
            );
        }

        // The whole of the difference is the header's row: the same account
        // parked at the same line has exactly one more line below a window that
        // has paid for a header, and never two.
        assert_eq!(below_at_top[1], below_at_top[0] + HEADER);
    }

    fn document_lines() -> Vec<String> {
        (0..5).map(|line| format!("line {line}")).collect()
    }

    #[test]
    fn a_document_is_drawn_from_its_first_line_in_the_order_it_was_given() {
        let now = Instant::now();
        let mut app = App::from_tree(&fixture::tree());
        app.panel_mut().set_height(PANEL);

        app.show_document(document_lines(), false);

        // The window is the top of the file: a file is read from its first line,
        // and nothing is appended to it for the panel to follow.
        assert!(app.panel().has_document());
        assert!(!app.panel().follows());
        assert_eq!(app.panel().scroll_offset(), 0);
        assert_eq!(panel_text(&app, now), ["line 0", "line 1", "line 2"]);
        // And the count agrees with what was handed over: five lines, three of
        // them on screen.
        assert_eq!(app.panel().lines_below(), 5 - usize::from(PANEL));

        // A panel tall enough for the whole file draws the whole file, in order,
        // with nothing below it.
        app.panel_mut().set_height(9);
        assert_eq!(panel_text(&app, now), document_lines());
        assert_eq!(app.panel().lines_below(), 0);
    }

    #[test]
    fn a_read_the_cap_cut_short_says_so_under_the_last_line_it_got() {
        let now = Instant::now();
        let mut app = App::from_tree(&fixture::tree());
        app.panel_mut().set_height(9);

        app.show_document(document_lines(), true);

        // One line more than the file's own, and it is the last one: what the
        // reader is looking at stops here and the file does not.
        let drawn = panel_text(&app, now);
        assert_eq!(drawn.len(), document_lines().len() + 1);
        assert_eq!(drawn[..5], document_lines()[..]);
        let cut = drawn.last().expect("a cut read says so");
        assert!(cut.contains("cut"), "{cut}");
        assert_eq!(cut, &cut_at_cap_message());

        // A read that fitted says nothing at all: the line is about the cut and
        // not about the reading.
        app.show_document(document_lines(), false);
        assert_eq!(panel_text(&app, now), document_lines());
    }

    fn a_long_line() -> Vec<String> {
        [
            "# The engine",
            "It walks the tree and writes what it finds.",
            "done",
        ]
        .map(str::to_owned)
        .to_vec()
    }

    const NARROW: u16 = 18;

    #[test]
    fn a_document_line_wider_than_the_panel_is_drawn_in_as_many_rows_as_it_needs() {
        let now = Instant::now();
        let mut app = App::from_tree(&fixture::tree());
        app.panel_mut().set_height(9);
        app.panel_mut().set_width(NARROW);

        app.show_document(a_long_line(), false);

        // Three lines, five rows: the long one broken at spaces, the short ones
        // exactly as they were, and the whole of the file's text on screen.
        assert_eq!(
            panel_text(&app, now),
            [
                "# The engine",
                "It walks the tree",
                "and writes what it",
                "finds.",
                "done",
            ]
        );
        assert_eq!(app.panel().lines_below(), 0);
    }

    #[test]
    fn a_panel_nobody_has_measured_wraps_nothing() {
        // What an app is between being built and being drawn: every frame tells
        // it the width, and until one has, a line is the row it arrived as.
        let now = Instant::now();
        let mut app = App::from_tree(&fixture::tree());
        app.panel_mut().set_height(9);

        app.show_document(a_long_line(), false);

        assert_eq!(app.panel().width(), 0);
        assert_eq!(panel_text(&app, now), a_long_line());
    }

    #[test]
    fn the_rows_a_document_draws_as_follow_the_width_the_panel_was_last_told() {
        // The reader drags the terminal narrower with a file up: the document
        // re-flows to the width of the frame it is drawn in, from the lines it
        // has always held, without anything being read again.
        let now = Instant::now();
        let mut app = App::from_tree(&fixture::tree());
        app.panel_mut().set_height(9);
        app.panel_mut().set_width(80);
        app.show_document(a_long_line(), false);
        assert_eq!(panel_text(&app, now), a_long_line());

        app.panel_mut().set_width(NARROW);

        assert_eq!(panel_text(&app, now).len(), 5);
        // And wider again is the document it was: nothing was lost on the way
        // through the narrow panel, because the lines held are the file's.
        app.panel_mut().set_width(80);
        assert_eq!(panel_text(&app, now), a_long_line());
    }

    #[test]
    fn the_panel_scrolls_by_rows_of_a_wrapped_document_rather_than_by_its_lines() {
        let now = Instant::now();
        let mut app = App::from_tree(&fixture::tree());
        app.panel_mut().set_height(2);
        app.panel_mut().set_width(NARROW);
        app.show_document(a_long_line(), false);

        // Five rows in a panel two tall: what is below the window is counted in
        // rows, so a reader scrolling past a wrapped line scrolls through it.
        assert_eq!(app.panel().lines_below(), 3);
        assert_eq!(panel_text(&app, now), ["# The engine", "It walks the tree"]);

        app.scroll_panel_down(1);

        assert_eq!(app.panel().scroll_offset(), 1);
        assert_eq!(
            panel_text(&app, now),
            ["It walks the tree", "and writes what it"]
        );
        assert_eq!(app.panel().lines_below(), 2);
    }

    #[test]
    fn an_account_line_too_long_for_the_panel_is_broken_under_its_own_clock() {
        // The panel's other card, at a width its lines do not fit in. A pass
        // that reported a long path — or failed with a sentence of somebody
        // else's stderr in it — is worth reading to the end, so the line is
        // broken into the rows it needs, in the column it started in. See
        // [`mod@crate::wrap`].
        let base = Instant::now();
        let mut app = app_pacting(0, base);
        app.panel_mut().set_height(9);
        let account = app
            .panel_mut()
            .account_mut()
            .expect("a run has just started");
        account.record(
            &Activity::Tool {
                name: "Read".to_owned(),
                detail: Some("crates/warlock-engine/src/pact.rs".to_owned()),
            },
            at(base, 1),
        );

        // Nobody has measured the panel yet: two lines, two rows.
        assert_eq!(app.panel().width(), 0);
        assert_eq!(panel_text(&app, at(base, 9)).len(), 2);

        app.panel_mut().set_width(NARROW);

        // Five rows, and the whole of the path on screen. The clock's own
        // columns are blank under it rather than repeated, so a row with
        // nothing in that column is the row above it still going.
        assert_eq!(
            panel_text(&app, at(base, 9)),
            [
                "crates/engine",
                "Read",
                "       crates/warl",
                "       ock-engine/",
                "       src/pact.rs",
            ]
        );
        assert_eq!(app.panel().lines_below(), 0);

        // And wider again is the account it always was: what a card holds is
        // what happened, never the rows some width once broke it into.
        app.panel_mut().set_width(80);
        assert_eq!(panel_text(&app, at(base, 9)).len(), 2);
    }

    fn account_window(app: &App) -> (usize, bool) {
        app.panel.window_of(Showing::Account)
    }

    fn thread_window(app: &App) -> (usize, bool) {
        app.panel.window_of(Showing::Thread)
    }

    fn document_window(app: &App) -> (usize, bool) {
        app.panel.window_of(Showing::Document)
    }

    fn document_text(app: &App) -> Vec<String> {
        app.panel
            .document_lines()
            .iter()
            .map(|line| match line {
                Line::Directory { path } => path.display().to_string(),
                Line::Clocked { text, .. }
                | Line::Summary { text }
                | Line::Text { text }
                | Line::Said { text }
                | Line::Note { text }
                | Line::Wrapped { text, .. } => text.clone(),
            })
            .collect()
    }

    #[test]
    fn a_document_shows_over_the_account_rather_than_taking_the_panel_from_it() {
        let base = Instant::now();
        let mut app = app_pacting(9, base);
        assert!(app.panel().has_account());

        app.show_document(document_lines(), false);

        // Two cards in one slot: the document is what is drawn, and the account
        // is still there behind it — held, handed out by the getters, and with
        // its own window exactly where it was.
        assert!(app.panel().has_document());
        assert!(app.panel().has_account());
        assert_eq!(app.panel().account().map(Account::line_count), Some(10));
        assert!(app.panel_mut().account_mut().is_some());
        assert!(app.panel().has_content());
        assert_eq!(panel_text(&app, at(base, 9))[0], "line 0");
        assert_eq!(account_window(&app), (10 - usize::from(PANEL), true));
    }

    #[test]
    fn a_pact_started_under_a_document_fills_the_card_behind_it() {
        let base = Instant::now();
        let mut app = app_pacting(9, base);
        app.show_document(document_lines(), false);
        let shown = panel_text(&app, at(base, 9));

        app.start_account(at(base, 100));

        // The run took nothing: the document is still the card on screen, and
        // the account started behind it on its own terms — empty, at the top,
        // following.
        assert!(app.panel().has_document());
        assert_eq!(panel_text(&app, at(base, 100)), shown);
        assert_eq!(app.panel().scroll_offset(), 0);
        assert!(!app.panel().follows());
        assert!(app.panel().has_account());
        assert_eq!(app.panel().account().map(Account::line_count), Some(0));
        assert_eq!(account_window(&app), (0, true));

        // And the run goes on reporting into that card while the reader reads,
        // with its window following the newest line for when they swap to it.
        let account = app
            .panel_mut()
            .account_mut()
            .expect("the run has its own card");
        account.open_section("crates/tui", at(base, 100));
        for line in 0..9 {
            account.record(&Activity::Thinking, at(base, 101 + line));
        }
        assert_eq!(app.panel().account().map(Account::line_count), Some(2));
        assert_eq!(panel_text(&app, at(base, 120)), shown);
        assert_eq!(account_window(&app), (0, true));
    }

    #[test]
    fn each_card_keeps_a_window_of_its_own() {
        let base = Instant::now();
        let mut app = app_pacting(9, base);

        // The account parked six lines down, and then a document over it.
        app.select_first();
        app.select_next();
        assert_eq!(account_window(&app), (1, false));

        app.show_document(document_lines(), false);
        assert_eq!(document_window(&app), (0, false));

        // Moving the document's window moves the document's window: the account
        // is where the reader left it, still parked.
        app.select_next();
        assert_eq!(document_window(&app), (1, false));
        assert_eq!(app.panel().scroll_offset(), 1);
        assert_eq!(account_window(&app), (1, false));

        // Including when the movement takes the document to its own end, which
        // is the one thing that sets a follow flag.
        app.select_last();
        assert_eq!(document_window(&app), (5 - usize::from(PANEL), true));
        assert!(app.panel().follows());
        assert_eq!(account_window(&app), (1, false));
    }

    #[test]
    fn a_movement_at_the_panel_moves_only_the_card_that_is_showing() {
        let base = Instant::now();

        for (name, movement) in MOVEMENTS {
            let mut app = app_pacting(9, base);
            app.select_first();
            app.show_document(document_lines(), false);
            let parked = account_window(&app);

            movement(&mut app);

            assert_eq!(account_window(&app), parked, "{name} moved the account");
            assert_eq!(
                app.panel().scroll_offset(),
                document_window(&app).0,
                "{name} scrolled something other than the document"
            );
        }

        // And the wheel, which names the pane rather than accepting the focus,
        // reads the same way.
        for lines in [1, 3, 99] {
            let mut app = app_pacting(9, base);
            app.select_first();
            app.show_document(document_lines(), false);
            let parked = account_window(&app);

            app.scroll_panel_down(lines);
            assert_eq!(account_window(&app), parked, "a wheel moved the account");
            app.scroll_panel_up(lines);
            assert_eq!(account_window(&app), parked, "a wheel moved the account");
            assert_eq!(document_window(&app), (0, false));
        }
    }

    #[test]
    fn an_empty_document_is_still_something_the_panel_is_holding() {
        let now = Instant::now();
        let mut app = App::from_tree(&fixture::tree());
        app.panel_mut().set_height(PANEL);

        app.show_document(Vec::<String>::new(), false);

        // An empty file read is not the same as nothing having happened: the
        // panel is holding it, so the mark does not come back.
        assert!(app.panel().has_document());
        assert!(app.panel().has_content());
        assert_eq!(app.panel().window(now), Vec::new());
        assert_eq!(app.panel().lines_below(), 0);
        assert_eq!(app.panel().scroll_offset(), 0);
    }

    #[test]
    fn the_swap_shows_the_other_card_and_the_next_swap_shows_it_back() {
        let base = Instant::now();
        let mut app = app_pacting(9, base);
        let account = panel_text(&app, at(base, 9));

        app.show_document(document_lines(), false);
        let document = panel_text(&app, at(base, 9));
        assert_ne!(document, account, "the two cards draw the same thing");

        // Round the cycle, because a swap that only went one way would strand a
        // reader on whichever card they were not reading. The conversation sits
        // between the two — always a stop, empty or not, because it is where the
        // field is — so the file is two presses from the run either way.
        app.swap_card();
        assert_eq!(app.panel.showing(), Showing::Thread);
        assert!(!app.panel().has_content(), "nothing has been asked");

        app.swap_card();
        assert_eq!(panel_text(&app, at(base, 9)), account);
        assert!(app.panel().has_content());

        app.swap_card();
        assert_eq!(panel_text(&app, at(base, 9)), document);
        assert!(app.panel().has_content());
    }

    #[test]
    fn a_swap_moves_nothing_but_which_card_is_showing() {
        let base = Instant::now();
        let mut app = app_pacting(9, base);
        app.show_document(document_lines(), false);
        // A window of the document's own, then the focus and a selection of the
        // tree's own, then something on the footer: everything a swap could
        // disturb, put somewhere a default would not be.
        app.select_next();
        app.toggle_focus();
        app.select_next();
        app.set_message("something the last keystroke said");
        let mut before = app.clone();

        app.swap_card();

        // The whole of what changed is the bit saying which card is drawn: the
        // focus, the selection, both windows and the footer are the app's own
        // clone, untouched — and nothing announces the swap, because the reader
        // can see it.
        before.panel.show(Showing::Thread);
        assert_eq!(app, before, "the swap moved something other than the card");
    }

    fn rewritten_lines() -> Vec<String> {
        ["# Rewritten", "by somebody else"]
            .map(str::to_owned)
            .to_vec()
    }

    #[test]
    fn a_re_read_fills_the_document_card_without_bringing_it_to_the_front() {
        let base = Instant::now();
        let mut app = app_pacting(9, base);
        // A document read once and then left: the reader is back on the
        // account, which is where a run they are watching puts them. Two presses
        // from the file, since the conversation sits between them.
        app.show_document(document_lines(), false);
        app.swap_card();
        app.swap_card();
        assert_eq!(app.panel.showing(), Showing::Account);
        let account = panel_text(&app, at(base, 9));

        app.panel_mut().refill_document(rewritten_lines(), false);

        // The panel is exactly where they left it. The new lines are on the card
        // behind it, waiting for the swap they will ask for themselves — a file
        // being saved in an editor is not a reason to take a run off the screen.
        assert_eq!(panel_text(&app, at(base, 9)), account);
        assert!(app.panel().has_document());
        assert_eq!(document_text(&app), rewritten_lines());

        app.swap_card();
        assert_eq!(panel_text(&app, at(base, 9)), rewritten_lines());
    }

    #[test]
    fn a_re_read_under_a_showing_document_leaves_it_showing_from_its_first_line() {
        let base = Instant::now();
        let mut app = app_pacting(9, base);
        app.show_document(document_lines(), false);
        // Parked at the end of the file that was there before, which is the
        // window a re-read has to decide what to do with.
        app.select_last();
        assert!(app.panel().follows());
        let parked = account_window(&app);

        app.panel_mut().refill_document(rewritten_lines(), false);

        // Still the card on screen — nothing about which card is showing moved —
        // and showing the new file from its first line: line five of what was
        // there is not line five of what is there now.
        assert_eq!(panel_text(&app, at(base, 9)), rewritten_lines());
        assert_eq!(app.panel().scroll_offset(), 0);
        assert!(!app.panel().follows());
        // And the account behind it is where it was, still following its own
        // newest line.
        assert_eq!(account_window(&app), parked);
    }

    #[test]
    fn a_re_read_the_cap_cut_short_says_so_exactly_as_the_first_read_did() {
        let now = Instant::now();
        let mut app = App::from_tree(&fixture::tree());
        app.panel_mut().set_height(9);
        app.show_document(document_lines(), false);

        app.panel_mut().refill_document(document_lines(), true);

        // The one line a document did not write, added here for the same reason
        // it is added to a first read: the words are the screen's.
        let drawn = panel_text(&app, now);
        assert_eq!(drawn.len(), document_lines().len() + 1);
        assert_eq!(drawn.last(), Some(&cut_at_cap_message()));
    }

    #[test]
    fn a_re_read_moves_nothing_but_the_document_card() {
        let base = Instant::now();
        let mut app = app_pacting(9, base);
        app.show_document(document_lines(), false);
        // A window of the document's own, the focus and a selection of the
        // tree's own, and something on the footer: everything a re-read could
        // disturb, put somewhere a default would not be.
        app.select_next();
        app.toggle_focus();
        app.select_next();
        app.set_message("something the last keystroke said");
        let mut before = app.clone();

        app.panel_mut().refill_document(rewritten_lines(), false);

        // The whole of what changed is what is on the one card: the account, the
        // focus, the selection, the tree's window, the bit saying which card is
        // drawn and the footer are all the app's own clone, untouched.
        before.panel.refill_document(rewritten_lines(), false);
        assert_eq!(
            app, before,
            "the re-read moved something other than the card"
        );
    }

    #[test]
    fn each_cards_window_survives_a_swap_away_and_back() {
        let base = Instant::now();
        let mut app = app_pacting(9, base);

        // The account parked a line down from the top, and a document over it
        // parked a line down from its own.
        app.select_first();
        app.select_next();
        let parked = account_window(&app);
        assert_eq!(parked, (1, false));

        app.show_document(document_lines(), false);
        app.select_next();
        let left = document_window(&app);
        assert_eq!(left, (1, false));

        // Away, past the empty conversation: the account's window is the one the
        // reader left it on, not the top and not the end.
        app.swap_card();
        app.swap_card();
        assert_eq!(app.panel.showing(), Showing::Account);
        assert_eq!(app.panel().scroll_offset(), parked.0);
        assert_eq!(panel_text(&app, at(base, 9))[0], "Read line 0");

        // And back: the document is on the line they left it on.
        app.swap_card();
        assert_eq!(document_window(&app), left);
        assert_eq!(app.panel().scroll_offset(), left.0);
        assert_eq!(
            panel_text(&app, at(base, 9)),
            ["line 1", "line 2", "line 3"]
        );
    }

    #[test]
    fn the_account_shows_its_newest_line_on_return_unless_it_was_parked() {
        let base = Instant::now();

        // Following: a run that went on reporting while the document was up is
        // at its newest line the moment the reader swaps to it.
        let mut app = app_pacting(9, base);
        app.show_document(document_lines(), false);
        record_lines(&mut app, 9..12, base);

        app.swap_card();
        app.swap_card();
        let drawn = panel_text(&app, at(base, 20));
        assert!(app.panel().follows());
        assert_eq!(drawn.last().expect("the account has lines"), "Read line 11");

        // Parked: the reader put that window where it is, and lines arriving
        // behind a document do not take it off the line they chose.
        let mut app = app_pacting(9, base);
        app.select_first();
        app.show_document(document_lines(), false);
        record_lines(&mut app, 9..12, base);

        app.swap_card();
        app.swap_card();
        assert!(!app.panel().follows());
        assert_eq!(app.panel().scroll_offset(), 0);
        assert_eq!(panel_text(&app, at(base, 20))[0], "crates/engine");
    }

    fn record_lines(app: &mut App, lines: std::ops::Range<u64>, base: Instant) {
        let account = app.panel_mut().account_mut().expect("a run has started");
        for line in lines {
            account.record(
                &Activity::Tool {
                    name: "Read".to_owned(),
                    detail: Some(format!("line {line}")),
                },
                at(base, line + 1),
            );
        }
    }

    #[test]
    fn the_document_card_is_a_snapshot_of_what_was_read_and_nothing_rewrites_it() {
        let base = Instant::now();
        let mut app = app_pacting(9, base);
        app.panel_mut().set_height(9);
        app.show_document(document_lines(), false);
        assert_eq!(document_text(&app), document_lines());

        // Everything a session does that is not another read: the reader walks
        // the tree, reshapes it, scrolls both cards, swaps between them, and a
        // second run starts and reports behind the document all the while.
        app.toggle_focus();
        app.select_next();
        app.select_last();
        app.toggle_collapsed();
        app.toggle_files();
        app.toggle_pacted_only();
        app.toggle_focus();
        app.select_next();
        app.swap_card();
        app.select_next();
        app.swap_card();
        app.swap_card();
        app.start_account(at(base, 100));
        app.panel_mut()
            .account_mut()
            .expect("a second run has started")
            .open_section("crates/tui", at(base, 100));
        record_lines(&mut app, 0..4, at(base, 100));
        app.set_message("something the last keystroke said");

        // The card holds the lines it was handed, in the order it was handed
        // them: it does not follow the selection, it is not appended to by the
        // run, and no window the reader moved has edited what it is a window
        // over.
        assert_eq!(document_text(&app), document_lines());
        assert_eq!(panel_text(&app, at(base, 200)), document_lines());
        assert_eq!(
            app.panel().account().map(Account::line_count),
            Some(5),
            "the run behind the document recorded nothing"
        );

        // The only thing that writes the card is another read, which replaces it
        // whole rather than adding to it.
        app.show_document(["a line of another file"], false);
        assert_eq!(document_text(&app), ["a line of another file"]);
    }

    #[test]
    fn a_swap_with_nothing_but_the_conversation_is_refused_naming_the_view_key() {
        // The only press that has nowhere to go. The conversation is always a
        // stop, so a swap is refused exactly when it is the only card there is:
        // no pact has run and no file has been read, and the two cards the key
        // would reach are both cards about nothing.
        let mut app = App::from_rows(three_rows());
        app.panel_mut().set_height(PANEL);
        app.set_message("something the last keystroke said");
        let mut before = app.clone();

        app.swap_card();

        // The mark stays up, the conversation stays showing, and the refusal is
        // about the document because the document is the card a keystroke can
        // make. The message is the whole of what the press changed.
        let message = app.message().expect("a swap with nothing read is refused");
        assert_eq!(message, no_document_message(), "{message}");
        assert!(message.contains("press v"), "{message}");
        assert_eq!(app.panel.showing(), Showing::Thread);
        assert!(!app.panel().has_content());
        before.set_message(message);
        assert_eq!(app, before, "the refusal moved something else");
    }

    const QUESTION: &str = "what does the engine do?";
    const ANSWER: &str = "It walks the tree and writes what it finds.";

    fn ask_and_answer(app: &mut App, base: Instant) {
        app.panel_mut().start_turn(QUESTION, at(base, 1));
        for line in 0..3 {
            app.panel_mut().record_turn(
                &Activity::Tool {
                    name: "Grep".to_owned(),
                    detail: Some(format!("thread line {line}")),
                },
                at(base, line + 2),
            );
        }
        app.panel_mut().answer_turn(ANSWER, at(base, 5));
    }

    #[test]
    fn the_swap_key_goes_round_all_three_cards_and_leaves_none_unreachable() {
        let base = Instant::now();
        let mut app = app_pacting(9, base);
        let account = panel_text(&app, at(base, 9));

        // A question brings the thread to the front by itself, the way a read
        // brings the document; both cards are filled behind whatever is drawn.
        ask_and_answer(&mut app, base);
        assert!(app.panel().has_thread());
        let thread = panel_text(&app, at(base, 9));
        app.show_document(document_lines(), false);
        let document = panel_text(&app, at(base, 9));

        // Three cards drawing three different things, so a swap landing on the
        // wrong one could not pass for the right one.
        assert_ne!(thread, account, "the run and the conversation draw alike");
        assert_ne!(document, thread, "the conversation and the file draw alike");
        assert_ne!(document, account, "the run and the file draw alike");

        // Round twice from the document: the cycle is a cycle, every card is
        // reachable from every other, and none of the three is skipped.
        let mut seen = Vec::new();
        for _ in 0..6 {
            app.swap_card();
            seen.push(panel_text(&app, at(base, 9)));
        }
        assert_eq!(
            seen,
            [
                thread.clone(),
                account.clone(),
                document.clone(),
                thread,
                account,
                document,
            ]
        );
        // And nothing was said about any of it: the reader can see which card
        // they are on.
        assert!(app.message().is_none(), "a swap that worked said something");
    }

    #[test]
    fn a_swap_with_a_thread_and_no_document_goes_between_those_two() {
        let base = Instant::now();
        let mut app = app_pacting(9, base);
        let account = panel_text(&app, at(base, 9));
        ask_and_answer(&mut app, base);
        let thread = panel_text(&app, at(base, 9));
        assert!(!app.panel().has_document());

        // The empty card is stepped over rather than shown, so the key never
        // spends a press on warlock's mark — and it is never refused either,
        // because there is somewhere to go.
        app.swap_card();
        assert_eq!(panel_text(&app, at(base, 9)), account);
        app.swap_card();
        assert_eq!(panel_text(&app, at(base, 9)), thread);
        assert!(app.message().is_none(), "a swap that worked said something");
    }

    #[test]
    fn a_swap_out_of_a_document_lands_on_the_conversation_rather_than_an_empty_account() {
        // The session this is most of: somebody has been talking to the panel,
        // no pact has run, and they open a file. The swap back has to be the
        // conversation they left — an empty account here would take their chat
        // off the screen and give them warlock's mark, which is the one thing
        // on this screen that says nothing.
        let base = Instant::now();
        let mut app = App::from_rows(three_rows());
        app.panel_mut().set_height(PANEL);
        ask_and_answer(&mut app, base);
        let thread = panel_text(&app, at(base, 9));
        assert!(!app.panel().has_account(), "no pact has run this session");

        app.show_document(document_lines(), false);
        let document = panel_text(&app, at(base, 9));
        assert_ne!(document, thread, "the file and the conversation draw alike");

        app.swap_card();

        assert_eq!(panel_text(&app, at(base, 9)), thread);
        assert!(app.message().is_none(), "a swap that worked said something");

        // And back to the file: a cycle of the two cards that have something on
        // them, with the empty one stepped over in both directions.
        app.swap_card();
        assert_eq!(panel_text(&app, at(base, 9)), document);
        app.swap_card();
        assert_eq!(panel_text(&app, at(base, 9)), thread);
    }

    #[test]
    fn a_swap_out_of_a_document_with_nothing_else_lands_where_the_field_is() {
        // The other half of that rule, and why the empty account is not simply
        // skipped: a document hides the composer, so the way back out of one
        // has to land on a card the field is drawn under. With no conversation
        // to go to, the empty account is the only one there is — and a reader
        // who has read a file on a fresh session can still type.
        let mut app = App::from_rows(three_rows());
        app.panel_mut().set_height(PANEL);
        app.show_document(document_lines(), false);
        assert!(!app.panel().composer_showable());

        app.swap_card();

        assert!(app.panel().composer_showable());
        assert!(!app.panel().has_content(), "the account is the empty one");
        assert!(app.message().is_none(), "a swap that worked said something");
    }

    #[test]
    fn showing_one_card_throws_neither_of_the_other_two_away() {
        let base = Instant::now();
        let mut app = app_pacting(9, base);

        // The account parked a line down from its top, then a thread over it
        // parked a line down from its own, then a document over that.
        app.select_first();
        app.select_next();
        assert_eq!(account_window(&app), (1, false));

        ask_and_answer(&mut app, base);
        app.select_first();
        app.select_next();
        assert_eq!(thread_window(&app), (1, false));

        app.show_document(document_lines(), false);
        app.select_next();
        assert_eq!(document_window(&app), (1, false));

        // Every card is still filled and every window is still where it was
        // left: showing one card neither empties the others nor moves them.
        assert!(app.panel().has_account());
        assert!(app.panel().has_thread());
        assert!(app.panel().has_document());
        assert_eq!(account_window(&app), (1, false));
        assert_eq!(thread_window(&app), (1, false));
        assert_eq!(document_window(&app), (1, false));

        // And round the cycle: each card draws from the line the reader left it
        // on rather than from its top or its end.
        app.swap_card();
        assert_eq!(panel_text(&app, at(base, 9))[0], "Grep thread line 0");
        app.swap_card();
        assert_eq!(panel_text(&app, at(base, 9))[0], "Read line 0");
        app.swap_card();
        assert_eq!(
            panel_text(&app, at(base, 9)),
            ["line 1", "line 2", "line 3"]
        );
    }

    #[test]
    fn each_of_the_three_cards_keeps_its_own_follow_flag() {
        let base = Instant::now();
        let mut app = app_pacting(9, base);
        // The account parked, the thread left following, the document at the
        // top of a file it was never following: three cards, three answers.
        app.select_first();
        ask_and_answer(&mut app, base);
        app.show_document(document_lines(), false);

        assert_eq!(account_window(&app), (0, false));
        assert_eq!(thread_window(&app), (2, true));
        assert_eq!(document_window(&app), (0, false));

        // Whichever card is showing is the one the panel answers about, and the
        // other two go on being what they were. The document first, then round
        // the cycle: the conversation it left following, then the account it
        // left parked.
        assert!(!app.panel().follows());
        assert_eq!(app.panel().scroll_offset(), 0);
        app.swap_card();
        assert!(app.panel().follows());
        assert_eq!(app.panel().scroll_offset(), 2);
        app.swap_card();
        assert!(!app.panel().follows());
        assert_eq!(app.panel().scroll_offset(), 0);

        // A turn already under way goes on being answered behind a document,
        // and the document does not move an inch while it is. Asking brings the
        // conversation to the front, so the file is two presses back.
        app.panel_mut()
            .start_turn("and how long does it take?", at(base, 6));
        assert_eq!(app.panel.showing(), Showing::Thread);
        app.swap_card();
        app.swap_card();
        assert_eq!(app.panel.showing(), Showing::Document);
        assert_eq!(
            panel_text(&app, at(base, 9)),
            ["line 0", "line 1", "line 2"]
        );
        app.panel_mut().answer_turn("About a second.", at(base, 7));
        assert_eq!(
            panel_text(&app, at(base, 9)),
            ["line 0", "line 1", "line 2"]
        );

        // And the thread is at its newest row the moment the reader swaps back
        // to it, exactly as a run left following is.
        app.swap_card();
        assert_eq!(app.panel.showing(), Showing::Thread);
        assert!(app.panel().follows());
        assert_eq!(
            panel_text(&app, at(base, 9)).last().map(String::as_str),
            Some("About a second.")
        );
    }

    #[test]
    fn every_line_wider_than_the_panel_is_drawn_in_as_many_rows_as_it_needs() {
        let base = Instant::now();
        let mut app = app_pacting(11, base);
        app.panel_mut().set_height(11);
        ask_and_answer(&mut app, base);

        // Nobody has measured the panel yet, so nothing is wrapped: five lines,
        // five rows.
        assert_eq!(app.panel().width(), 0);
        assert_eq!(panel_text(&app, at(base, 5)).len(), 5);

        app.panel_mut().set_width(NARROW);

        // Eleven rows now, and nothing has run off the edge. The answer is
        // prose and is broken at spaces into three; the question is broken
        // under its own marker and each work line under its own clock, so what
        // continues a row sits in the column the row started in.
        assert_eq!(
            panel_text(&app, at(base, 5)),
            [
                "what does the",
                "  engine do?",
                "Grep thread",
                "       line 0",
                "Grep thread",
                "       line 1",
                "Grep thread",
                "       line 2",
                "It walks the tree",
                "and writes what it",
                "finds.",
            ]
        );
        assert_eq!(app.panel().lines_below(), 0);

        // The window is cut out of those rows and not out of the lines: a panel
        // three tall over eleven rows shows the last three and has eight above
        // them, and a reader who goes back to the top has those eight below.
        app.panel_mut().set_height(PANEL);
        assert_eq!(app.panel().scroll_offset(), 8);
        assert_eq!(app.panel().lines_below(), 0);
        app.select_first();
        assert_eq!(app.panel().lines_below(), 8);

        // And wider again is the answer it always was — the lines held are the
        // model's own, so a terminal dragged about re-flows what is on screen
        // rather than asking anything again.
        app.panel_mut().set_height(9);
        app.panel_mut().set_width(80);
        let drawn = panel_text(&app, at(base, 5));
        assert_eq!(drawn.len(), 5);
        assert_eq!(drawn.last().map(String::as_str), Some(ANSWER));
    }

    #[test]
    fn a_second_question_goes_under_the_first_rather_than_in_place_of_it() {
        let base = Instant::now();
        let mut app = app_pacting(9, base);
        // Nothing asked yet: there is no thread at all, and what a turn would
        // have reported falls on the floor rather than making one.
        assert!(!app.panel().has_thread());
        assert!(app.panel().thread().is_none());
        app.panel_mut().record_turn(&Activity::Thinking, base);
        app.panel_mut().answer_turn("nobody asked", base);
        assert!(!app.panel().has_thread());

        ask_and_answer(&mut app, base);
        app.panel_mut()
            .start_turn("and what did that cost?", at(base, 6));
        app.panel_mut()
            .record_turn(&Activity::Cost { usd: 0.02 }, at(base, 7));
        app.panel_mut().end_turn(&Ending::Cancelled, at(base, 8));

        // One session, one conversation: the second turn is under the first and
        // the first is exactly as it was answered.
        let thread = app.panel().thread().expect("a question has been asked");
        assert_eq!(thread.turns().len(), 2);
        assert_eq!(thread.turns()[0].message(), QUESTION);
        assert_eq!(thread.turns()[0].answer(), Some(ANSWER));
        assert_eq!(thread.turns()[1].message(), "and what did that cost?");
        assert_eq!(thread.turns()[1].ending(), Some(&Ending::Cancelled));

        // A second ending is the first one still: the line on screen wins.
        app.panel_mut().end_turn(&Ending::NothingSaid, at(base, 9));
        let thread = app.panel().thread().expect("a question has been asked");
        assert_eq!(thread.turns()[1].ending(), Some(&Ending::Cancelled));

        // And the run behind all of it never heard a word: a conversation is
        // not a pact.
        assert_eq!(app.panel().account().map(Account::line_count), Some(10));
    }

    #[test]
    fn a_note_brings_the_conversation_forward_and_costs_nobody_a_turn() {
        const REFUSED: &str = "commands are /brief, /write and /chat, and take nothing after them";

        let base = Instant::now();
        let mut app = app_pacting(9, base);
        // A file on screen, and not one question asked: the note has to make
        // the card as well as land on it.
        app.show_document(document_lines(), false);
        assert_eq!(app.panel.showing(), Showing::Document);
        assert!(!app.panel().has_thread());

        app.panel_mut().note(REFUSED, base);

        // The conversation comes to the front the way a question brings it: the
        // line answers what the reader just typed, and an answer on a card they
        // are not looking at is not an answer.
        assert_eq!(app.panel.showing(), Showing::Thread);
        assert!(app.panel().has_thread());
        assert_eq!(panel_text(&app, at(base, 9)), [REFUSED]);

        // And it is warlock's own line, not a turn: nobody was asked anything,
        // so nothing is in flight and the composer is free.
        let thread = app.panel().thread().expect("the note made the card");
        assert!(thread.turns().is_empty());
        assert!(thread.in_flight().is_none());
        assert_eq!(thread.line_count(), 1);

        // The card accumulates, as it does for a question: what is said next
        // goes under the note rather than in place of it.
        ask_and_answer(&mut app, base);
        app.panel_mut().note("wrote docs/brief.md", at(base, 6));

        let lines = app
            .panel()
            .thread()
            .expect("a question has been asked")
            .lines(at(base, 9));
        assert_eq!(lines.len(), 1 + 5 + 1);
        assert_eq!(
            lines.first(),
            Some(&Line::Note {
                text: REFUSED.to_owned(),
            })
        );
        assert_eq!(
            lines.last(),
            Some(&Line::Note {
                text: "wrote docs/brief.md".to_owned(),
            })
        );
        assert_eq!(
            app.panel().thread().map(|thread| thread.turns().len()),
            Some(1)
        );

        // The other two cards are exactly as they were left, lines and all.
        assert!(app.panel().has_account());
        assert!(app.panel().has_document());
    }

    #[test]
    fn a_reload_carries_the_thread_over_with_the_rest_of_the_panel() {
        let base = Instant::now();
        let mut app = app_pacting(9, base);
        ask_and_answer(&mut app, base);
        let thread = panel_text(&app, at(base, 9));

        let reseated = reseat_on(&app, &fixture::tree_after_a_run());

        // The tree is read again *because* something happened, and a
        // conversation is not a claim about the tree: the card, the turn on it
        // and the card showing all come over.
        assert!(reseated.panel().has_thread());
        assert_eq!(
            reseated.panel().thread().map(|thread| thread.turns().len()),
            Some(1)
        );
        assert_eq!(panel_text(&reseated, at(base, 9)), thread);
    }

    #[test]
    fn a_run_rolled_back_leaves_the_conversation_where_it_was() {
        let base = Instant::now();
        let mut app = app_pacting(9, base);
        // The copy is taken before the run, so it predates every word of the
        // conversation held over it.
        let before = app.clone();
        ask_and_answer(&mut app, base);
        let thread = panel_text(&app, at(base, 9));

        app.restore_from(before);

        // The panel is the one thing a rollback keeps, and the thread is on it:
        // a question somebody asked is not a claim about the tree, and rolling
        // one back would take the answer off the screen of the reader who
        // wanted it.
        assert!(app.panel().has_thread());
        assert_eq!(
            app.panel().thread().map(|thread| thread.turns().len()),
            Some(1)
        );
        assert_eq!(panel_text(&app, at(base, 9)), thread);
    }

    fn run_a_pact(app: &mut App, base: Instant, from: u64) {
        app.start_account(at(base, from));
        app.panel_mut()
            .write_run(|account| account.open_section("crates/tui", at(base, from)));
        for line in 0..3 {
            app.panel_mut().write_run(|account| {
                account.record(
                    &Activity::Tool {
                        name: "Read".to_owned(),
                        detail: Some(format!("run line {line}")),
                    },
                    at(base, from + line + 1),
                );
            });
        }
        app.panel_mut().write_run(|account| {
            account.close_section(
                &Outcome::Wrote {
                    document: PathBuf::from("crates/tui").join("WARLOCK.md"),
                    bytes: 120,
                },
                at(base, from + 4),
            );
        });
        app.panel_mut()
            .write_run(|account| account.finish(at(base, from + 5)));
    }

    #[test]
    fn a_run_started_on_the_thread_puts_nothing_in_the_conversation() {
        let base = Instant::now();
        let mut app = app_pacting(9, base);
        ask_and_answer(&mut app, base);
        assert!(app.panel().showing_thread());
        let asked = panel_text(&app, at(base, 9));

        run_a_pact(&mut app, base, 10);

        // The card the reader was on is the card they are on, drawing exactly
        // what it drew: a pact has a card of its own, and a conversation that
        // also carried it would be the same run written twice on one screen.
        assert!(app.panel().showing_thread(), "the run took the panel");
        assert_eq!(panel_text(&app, at(base, 20)), asked);
        let thread = app.panel().thread().expect("a question has been asked");
        assert_eq!(thread.turns().len(), 1, "the run took a turn");
        assert_eq!(thread.turns()[0].message(), QUESTION);
        assert_eq!(thread.turns()[0].answer(), Some(ANSWER));
        // And nothing is in flight, so the field is the reader's: a run is not
        // a question and does not mute the composer.
        assert!(thread.in_flight().is_none());

        // The run is all on its own card, whole.
        assert_eq!(app.panel().account().map(Account::line_count), Some(6));
    }

    #[test]
    fn a_runs_events_reach_the_account_card_and_nowhere_else() {
        let base = Instant::now();
        let mut app = app_pacting(9, base);
        ask_and_answer(&mut app, base);
        let asked = panel_text(&app, at(base, 9));

        // Fed one event at a time, and after each of them the conversation is
        // the conversation: a run fills one card, as it happens, and the other
        // two are none of its business.
        app.start_account(at(base, 10));
        app.panel_mut()
            .write_run(|account| account.open_section("crates/tui", at(base, 10)));
        let mut seen = Vec::new();
        for (line, now) in [(0, 11), (1, 12), (2, 13)] {
            app.panel_mut().write_run(|account| {
                account.record(
                    &Activity::Tool {
                        name: "Read".to_owned(),
                        detail: Some(format!("run line {line}")),
                    },
                    at(base, now),
                );
            });
            let card = app.panel().account().expect("the run started its own card");
            seen.push(card.line_count());
            assert_eq!(panel_text(&app, at(base, 20)), asked);
        }
        // Something did arrive on each of the three passes, so the equality
        // above is about a card that was moving.
        assert_eq!(seen, [2, 3, 4]);

        // And the outcome and the summary are the same story: the account says
        // how the run went, and the thread still says what was asked.
        app.panel_mut().write_run(|account| {
            account.close_section(
                &Outcome::Refused {
                    reason: "the model would not".to_owned(),
                },
                at(base, 14),
            );
            account.finish(at(base, 15));
        });
        let card = app.panel().account().expect("the run started its own card");
        assert!(
            card.lines(at(base, 30))
                .iter()
                .any(|line| matches!(line, Line::Summary { .. })),
            "the run never said how it went"
        );
        assert_eq!(panel_text(&app, at(base, 30)), asked);
        assert_eq!(
            app.panel().thread().map(|thread| thread.turns().len()),
            Some(1),
            "the run took a turn of the conversation"
        );
    }

    #[test]
    fn a_run_with_no_conversation_behind_it_conjures_no_thread() {
        let base = Instant::now();
        let mut app = app_pacting(9, base);
        assert!(!app.panel().has_thread());

        run_a_pact(&mut app, base, 10);

        // Nothing typed, no conversation held: a run fills its own card and
        // nothing else, and it took the panel because the panel had nothing on
        // it — the mark is not worth more than the run the reader just asked
        // for.
        assert!(!app.panel().has_thread());
        assert!(app.panel().thread().is_none());
        assert_eq!(app.panel.showing(), Showing::Account);
        assert_eq!(app.panel().account().map(Account::line_count), Some(6));

        // The conversation is still a card to swap to, empty or not: it is
        // where the field is, so a reader who wants to ask something about the
        // run they are watching is one press away from being able to.
        app.swap_card();
        assert_eq!(app.panel.showing(), Showing::Thread);
        assert!(!app.panel().has_content(), "nothing has been asked");
        assert!(app.panel().composer_showable(), "the field came with it");
        assert!(app.message().is_none(), "a swap that worked said something");

        // With no document read the key goes between those two and is never
        // refused: there is always somewhere to go.
        app.swap_card();
        assert_eq!(app.panel.showing(), Showing::Account);
        app.swap_card();
        assert_eq!(app.panel.showing(), Showing::Thread);
        assert!(app.message().is_none(), "a swap that worked said something");

        // With a document read the cycle is all three, in order.
        app.show_document(document_lines(), false);
        assert_eq!(app.panel.showing(), Showing::Document);
        app.swap_card();
        assert_eq!(app.panel.showing(), Showing::Thread);
        app.swap_card();
        assert_eq!(app.panel.showing(), Showing::Account);
        app.swap_card();
        assert_eq!(app.panel.showing(), Showing::Document);
    }

    #[test]
    fn a_run_started_under_a_document_leaves_both_other_cards_alone() {
        let base = Instant::now();
        let mut app = app_pacting(9, base);
        ask_and_answer(&mut app, base);
        app.show_document(document_lines(), false);
        let document = panel_text(&app, at(base, 9));

        run_a_pact(&mut app, base, 10);

        // Which card is showing is the reader's: the file they were reading is
        // still what is drawn, and the run filled the one card it is about.
        assert_eq!(app.panel.showing(), Showing::Document);
        assert_eq!(panel_text(&app, at(base, 20)), document);
        assert_eq!(app.panel().account().map(Account::line_count), Some(6));
        assert_eq!(
            app.panel().thread().map(|thread| thread.turns().len()),
            Some(1),
            "the run took a turn of the conversation"
        );
    }

    #[test]
    fn the_account_is_a_swap_away_from_a_conversation_that_never_moved() {
        let base = Instant::now();
        let mut app = app_pacting(9, base);
        ask_and_answer(&mut app, base);
        app.select_first();
        app.select_next();
        assert_eq!(thread_window(&app), (1, false));
        let parked = panel_text(&app, at(base, 20));

        run_a_pact(&mut app, base, 10);

        // A run behind the conversation moves nothing on it: not its lines, not
        // its window, not where the reader parked it.
        assert_eq!(thread_window(&app), (1, false));
        assert_eq!(panel_text(&app, at(base, 20)), parked);

        // One press to the account, which the run filled, and one back. The
        // account is at its own window, following its own newest line: the
        // reader's place in the thread had nothing to do with it.
        app.swap_card();
        assert_eq!(app.panel.showing(), Showing::Account);
        assert_eq!(app.panel().account().map(Account::line_count), Some(6));
        assert_eq!(account_window(&app), (6 - usize::from(PANEL), true));
        assert_eq!(panel_text(&app, at(base, 20))[0], "Read run line 2");

        app.swap_card();
        assert_eq!(app.panel.showing(), Showing::Thread);
        // Each card keeps its own window: the reader comes back to the line
        // they left rather than to the end of a run that went on without them.
        assert_eq!(thread_window(&app), (1, false));
        assert_eq!(panel_text(&app, at(base, 20)), parked);
    }

    #[test]
    fn a_rollback_keeps_the_run_that_ended_and_the_conversation_beside_it() {
        let base = Instant::now();
        let mut app = app_pacting(9, base);
        ask_and_answer(&mut app, base);
        // The copy is taken when the key is pressed, before the run it is put
        // back for has recorded a word of itself.
        let before = app.clone();
        run_a_pact(&mut app, base, 10);
        let thread = panel_text(&app, at(base, 20));

        app.restore_from(before);

        // A run that ends with nothing recorded is exactly the run a reader
        // most wants to see the end of, and the panel is what a rollback keeps:
        // the account survives whole, and so does the conversation beside it.
        assert!(app.panel().showing_thread());
        assert_eq!(panel_text(&app, at(base, 20)), thread);
        assert_eq!(app.panel().account().map(Account::line_count), Some(6));
        let held = app.panel().thread().expect("a question has been asked");
        assert_eq!(held.turns().len(), 1);
        assert_eq!(held.turns()[0].answer(), Some(ANSWER));
    }

    #[test]
    fn a_run_under_way_when_a_question_is_asked_files_nothing_under_it() {
        let base = Instant::now();
        let mut app = app_pacting(9, base);

        // The run is under way, and then somebody asks something.
        app.start_account(at(base, 10));
        app.panel_mut().start_turn(QUESTION, at(base, 11));
        app.panel_mut()
            .write_run(|account| account.open_section("crates/tui", at(base, 12)));
        app.panel_mut()
            .write_run(|account| account.finish(at(base, 13)));

        // The run has a card and the question has a turn: a line filed under
        // the question would put a pact's work under somebody's sentence.
        assert_eq!(app.panel().account().map(Account::line_count), Some(3));
        let thread = app.panel().thread().expect("a question has been asked");
        assert_eq!(thread.turns().len(), 1);
        assert_eq!(thread.turns()[0].message(), QUESTION);
        assert_eq!(thread.turns()[0].answer(), None);
    }

    #[test]
    fn a_line_written_after_a_run_is_over_reaches_nothing_at_all() {
        let base = Instant::now();
        let mut app = app_pacting(9, base);
        ask_and_answer(&mut app, base);
        run_a_pact(&mut app, base, 10);
        let thread = panel_text(&app, at(base, 20));
        let turns = app.panel().thread().map(|thread| thread.turns().len());

        // A late event — a worker that reported after its outcome landed — is
        // dropped by the account, which is finished and says so on screen. The
        // conversation hears nothing either, as it heard nothing about the rest
        // of the run.
        app.panel_mut()
            .write_run(|account| account.record(&Activity::Thinking, at(base, 21)));

        assert_eq!(
            app.panel().thread().map(|thread| thread.turns().len()),
            turns
        );
        assert_eq!(panel_text(&app, at(base, 30)), thread);
        assert_eq!(app.panel().account().map(Account::line_count), Some(6));
    }

    #[test]
    fn a_file_row_hands_its_own_path_to_the_view_key_and_changes_nothing() {
        // A `WARLOCK.md` is an ordinary file row, so both kinds are the yes.
        for path in ["warlock/assets/logo.svg", "warlock/crates/tui/WARLOCK.md"] {
            let mut app = app_with_files_selecting(path);
            app.set_message("something the last keystroke said");
            let before = app.clone();

            let asked = app.view_target();

            assert_eq!(asked, Some(PathBuf::from(path)), "{path}");
            // Nothing moved — not the message, not the panel, nothing: the
            // reading has not happened yet.
            assert_eq!(app, before, "an accepted press moved something on {path}");
            assert!(!app.panel().has_content(), "{path} drew something");
        }
    }

    #[test]
    fn a_documented_directory_is_refused_by_the_view_key_naming_its_document() {
        // Pacted or not: what decides the wording is whether there is a document
        // to read, and `warlock/assets` has one without a manifest entry.
        for (path, document) in [
            ("warlock/crates/tui", "warlock/crates/tui/WARLOCK.md"),
            ("warlock/assets", "warlock/assets/WARLOCK.md"),
        ] {
            let mut app = app_selecting(path);
            let mut before = app.clone();

            assert_eq!(app.view_target(), None, "{path} was read");

            let message = app.message().expect("a directory row is refused");
            assert!(
                message.starts_with(&format!("{path} is a directory")),
                "{message}"
            );
            // The row that would have worked is named, because it is the next
            // one down.
            assert!(message.contains(document), "{message}");
            // The message is the whole of what the press changed, and the panel
            // is untouched.
            before.set_message(message);
            assert_eq!(app, before, "refusing {path} moved something else");
            assert!(!app.panel().has_content(), "{path} drew something");
        }
    }

    #[test]
    fn an_undocumented_directory_is_refused_by_the_view_key_naming_the_pact_key() {
        let mut app = app_selecting("warlock/crates");
        let mut before = app.clone();

        assert_eq!(app.view_target(), None, "an undocumented row was read");

        let message = app.message().expect("a directory row is refused");
        assert!(
            message.starts_with("warlock/crates is a directory"),
            "{message}"
        );
        // Nothing to point at, so it points at the key that would make
        // something to point at.
        assert!(message.contains("press p to pact it"), "{message}");
        before.set_message(message);
        assert_eq!(app, before, "refusing a directory moved something else");
        assert!(!app.panel().has_content(), "the refusal drew something");
    }

    #[test]
    fn an_app_with_no_rows_views_nothing() {
        let mut app = App::from_rows(Vec::new());

        assert_eq!(app.view_target(), None);
        assert_eq!(app.message(), None);
        assert!(!app.panel().has_content());
    }

    const NOTCH: usize = 3;

    #[test]
    fn selecting_a_row_lands_exactly_where_stepping_to_it_lands() {
        // Every row of the tree, reached both ways from the middle: a pointer
        // that landed on a row and a key pressed until it got there are the same
        // selection, the same window, and the same everything else.
        for index in 0..MANY {
            let mut stepped = scrolled_to(MANY / 2);
            while stepped.selected() < index {
                stepped.select_next();
            }
            while stepped.selected() > index {
                stepped.select_previous();
            }

            let mut picked = scrolled_to(MANY / 2);
            picked.select_row(index);

            assert_eq!(picked.selected(), index, "row {index} was not selected");
            assert_eq!(picked, stepped, "row {index} landed somewhere else");
        }
    }

    #[test]
    fn selecting_a_row_clears_the_last_keystrokes_message() {
        let mut app = scrolled_to(MANY / 2);
        app.set_message("something to sweep away");

        app.select_row(2);

        assert_eq!(app.message(), None);
    }

    #[test]
    fn a_row_the_tree_has_not_got_is_not_selected_at_all() {
        // A point below the last row of a half-full tree is a point on nothing,
        // so it is refused rather than clamped to the last row — message
        // included, since a key that did nothing explains nothing.
        let mut app = scrolled_to(MANY / 2);
        app.set_message("something to keep");
        let before = app.clone();

        for index in [MANY, MANY + 1, usize::MAX] {
            app.select_row(index);
            assert_eq!(app, before, "row {index} moved something");
        }

        let mut empty = App::from_rows(Vec::new());
        empty.set_viewport_height(WINDOW);
        empty.set_message("something to keep");
        let before = empty.clone();

        for index in [0, 1, usize::MAX] {
            empty.select_row(index);
            assert_eq!(empty, before, "row {index} moved an empty app");
        }
    }

    #[test]
    fn scrolling_the_tree_is_exactly_that_many_presses_of_the_movement_key() {
        for rows in [0, 1, NOTCH, usize::from(WINDOW), MANY, MANY * 2] {
            for start in [0, NOTCH, MANY / 2, MANY - 1] {
                let mut stepped = scrolled_to(start);
                let mut wheeled = scrolled_to(start);
                for _ in 0..rows {
                    stepped.select_next();
                }
                wheeled.select_next_by(rows);
                assert_eq!(wheeled, stepped, "{rows} down from {start}");

                let mut stepped = scrolled_to(start);
                let mut wheeled = scrolled_to(start);
                for _ in 0..rows {
                    stepped.select_previous();
                }
                wheeled.select_previous_by(rows);
                assert_eq!(wheeled, stepped, "{rows} up from {start}");
            }
        }
    }

    #[test]
    fn scrolling_the_tree_past_either_end_stops_there() {
        let mut app = scrolled_to(MANY / 2);

        app.select_next_by(MANY * 2);
        assert_eq!(app.selected(), MANY - 1);
        assert_eq!(app.scroll_offset(), MANY - usize::from(WINDOW));

        // And a notch past the end is the end again rather than a wrap.
        app.select_next_by(NOTCH);
        assert_eq!(app.selected(), MANY - 1);
        assert_eq!(app.scroll_offset(), MANY - usize::from(WINDOW));

        app.select_previous_by(MANY * 2);
        assert_eq!(app.selected(), 0);
        assert_eq!(app.scroll_offset(), 0);

        app.select_previous_by(NOTCH);
        assert_eq!(app.selected(), 0);
        assert_eq!(app.scroll_offset(), 0);
    }

    #[test]
    fn scrolling_a_tree_with_no_rows_is_a_no_op() {
        let mut app = App::from_rows(Vec::new());
        app.set_viewport_height(WINDOW);

        for rows in [0, 1, NOTCH, MANY] {
            app.select_next_by(rows);
            assert!(app.is_empty());
            assert_eq!(app.selected_row(), None);
            assert_eq!(app.selected(), 0, "{rows} down moved an empty app");
            assert_eq!(app.scroll_offset(), 0, "{rows} down scrolled it");

            app.select_previous_by(rows);
            assert_eq!(app.selected(), 0, "{rows} up moved an empty app");
            assert_eq!(app.scroll_offset(), 0, "{rows} up scrolled it");
        }
    }

    #[test]
    fn the_wheel_over_the_tree_does_not_consult_the_focus() {
        // The pointer is over the tree, so the tree moves — however the keys
        // happen to be pointed, and without the wheel taking the keys with it.
        for focus in [Focus::Tree, Focus::Panel] {
            let mut app = scrolled_to(MANY / 2);
            app.set_focus(focus);

            app.select_next_by(NOTCH);
            assert_eq!(app.selected(), MANY / 2 + NOTCH, "{focus:?} held it back");

            app.select_previous_by(NOTCH);
            assert_eq!(app.selected(), MANY / 2, "{focus:?} held it back");

            app.select_row(1);
            assert_eq!(app.selected(), 1, "{focus:?} refused the click");

            assert_eq!(app.focus(), focus, "the wheel moved the focus");
        }
    }

    #[test]
    fn the_wheel_over_the_panel_is_exactly_that_many_presses_at_the_panel() {
        let base = Instant::now();

        for lines in [0, 1, NOTCH, usize::from(PANEL), MANY] {
            // `app_pacting` leaves the panel focused, so the keys reach it; the
            // wheel is asked of an app whose keys are pointed at the tree.
            let mut stepped = app_pacting(9, base);
            let mut wheeled = app_pacting(9, base);
            wheeled.set_focus(Focus::Tree);
            for _ in 0..lines {
                stepped.select_previous();
            }
            wheeled.scroll_panel_up(lines);
            assert_eq!(
                wheeled.panel().scroll_offset(),
                stepped.panel().scroll_offset(),
                "{lines} up"
            );
            assert_eq!(
                wheeled.panel().follows(),
                stepped.panel().follows(),
                "{lines} up"
            );

            for _ in 0..lines {
                stepped.select_next();
            }
            wheeled.scroll_panel_down(lines);
            assert_eq!(
                wheeled.panel().scroll_offset(),
                stepped.panel().scroll_offset(),
                "{lines} back down"
            );
            assert_eq!(
                wheeled.panel().follows(),
                stepped.panel().follows(),
                "{lines} back down"
            );
        }
    }

    #[test]
    fn the_wheel_over_the_panel_breaks_follow_and_scrolling_back_restores_it() {
        let base = Instant::now();
        let mut app = app_pacting(9, base);
        // The keys are pointed at the tree throughout: the pointer is over the
        // panel, and that is the whole of what decides which pane scrolls.
        app.set_focus(Focus::Tree);
        let selected = app.selected();
        assert!(app.panel().follows());

        app.scroll_panel_up(NOTCH);

        assert!(!app.panel().follows());
        assert_eq!(
            panel_text(&app, at(base, 9)),
            ["Read line 3", "Read line 4", "Read line 5"],
        );

        // Parked means parked: the lines that arrive next leave it where it is.
        app.panel_mut()
            .account_mut()
            .expect("a run is under way")
            .record(&Activity::Thinking, at(base, 10));
        assert_eq!(app.panel().scroll_offset(), 10 - usize::from(PANEL) - NOTCH);
        assert!(!app.panel().follows());

        // And scrolling back to the end is live again, with nothing having to
        // say so.
        app.scroll_panel_down(MANY);

        assert!(app.panel().follows());
        assert_eq!(app.panel().lines_below(), 0);
        assert_eq!(
            panel_text(&app, at(base, 10)),
            ["Read line 7", "Read line 8", "thinking"],
        );
        assert_eq!(app.selected(), selected, "the panel moved the tree");
        assert_eq!(app.focus(), Focus::Tree, "the panel took the keys");
    }

    #[test]
    fn the_wheel_over_the_panel_leaves_the_last_keystrokes_message_up() {
        // The same reading a movement key at the panel takes: the line belongs
        // to the tree column, and nothing there has moved.
        let base = Instant::now();
        let mut app = app_pacting(9, base);
        app.set_focus(Focus::Tree);
        app.set_message("something to keep");

        app.scroll_panel_up(NOTCH);
        app.scroll_panel_down(1);

        assert_eq!(app.message(), Some("something to keep"));
    }

    #[test]
    fn setting_the_focus_to_the_pane_that_has_it_changes_nothing() {
        for focus in [Focus::Tree, Focus::Panel] {
            let mut app = scrolled_to(MANY / 2);
            app.set_message("something to keep");
            app.set_focus(focus);
            let before = app.clone();

            app.set_focus(focus);

            assert_eq!(app, before, "{focus:?} again moved something");
        }
    }

    #[test]
    fn setting_the_focus_moves_nothing_but_the_focus() {
        let mut app = scrolled_to(MANY / 2);
        app.set_message("something to keep");
        let before = app.clone();

        app.set_focus(Focus::Panel);

        assert_eq!(app.focus(), Focus::Panel);
        assert_eq!(app.selected(), before.selected());
        assert_eq!(app.scroll_offset(), before.scroll_offset());
        assert_eq!(app.message(), Some("something to keep"));

        // Put it back rather than exempting the field, so the comparison covers
        // every other field there is.
        app.set_focus(Focus::Tree);
        assert_eq!(app, before, "setting the focus changed something else");
    }

    #[test]
    fn none_of_the_pointers_moves_touches_the_tree_or_the_collapsed_set() {
        let mut app = app_selecting("warlock/crates");
        app.toggle_collapsed();
        let rows = drawn(&app);
        let collapsed = app.collapsed().clone();
        let counts = app.counts();

        app.select_row(0);
        app.select_next_by(NOTCH);
        app.select_previous_by(NOTCH);
        app.scroll_panel_up(NOTCH);
        app.scroll_panel_down(NOTCH);
        app.set_focus(Focus::Panel);
        app.set_focus(Focus::Tree);

        assert_eq!(drawn(&app), rows, "the pointer reshaped the tree");
        assert_eq!(
            app.collapsed(),
            &collapsed,
            "the pointer collapsed something"
        );
        assert_eq!(app.counts(), counts, "the pointer moved the tally");
    }

    #[test]
    fn a_re_seat_carries_the_account_and_the_panels_window() {
        let base = Instant::now();
        let mut app = app_pacting(9, base);
        app.select_first();
        let before = panel_text(&app, at(base, 9));

        let reseated = reseat_on(&app, &fixture::tree());

        assert_eq!(reseated.panel().account(), app.panel().account());
        assert_eq!(reseated.panel().height(), usize::from(PANEL));
        assert_eq!(reseated.panel().scroll_offset(), 0);
        assert!(!reseated.panel().follows());
        assert_eq!(panel_text(&reseated, at(base, 9)), before);
    }

    #[test]
    fn a_re_seat_under_a_document_leaves_it_showing_with_the_account_behind_it() {
        let base = Instant::now();
        let mut app = app_pacting(9, base);
        app.show_document(document_lines(), false);
        // A line down the document, so what carries is the reader's window and
        // not the top of the card, which a rebuild would arrive at by accident.
        app.select_next();
        let showing = panel_text(&app, at(base, 9));
        assert_eq!(document_window(&app), (1, false));

        // What the watcher does when something on disk changed: a new tree,
        // everything else carried.
        let reseated = reseat_on(&app, &fixture::tree_after_a_run());

        // The same card is on screen, on the same line of it. A reload that put
        // the account back would take a document out of the reader's hands at
        // exactly the moment they were reading it.
        assert_eq!(reseated.panel.showing(), Showing::Document);
        assert!(reseated.panel().has_document());
        assert_eq!(panel_text(&reseated, at(base, 9)), showing);
        assert_eq!(document_window(&reseated), document_window(&app));
        // And the account is intact behind it, lines, window and all.
        assert!(reseated.panel().has_account());
        assert_eq!(reseated.panel().account(), app.panel().account());
        assert_eq!(account_window(&reseated), account_window(&app));
        // The card is a snapshot: the reload rewrote none of its lines, and the
        // swap still comes back to the whole of what was read.
        assert_eq!(document_text(&reseated), document_lines());
        let mut reseated = reseated;
        for _ in 0..3 {
            reseated.swap_card();
        }
        assert_eq!(reseated.panel.showing(), Showing::Document);
        assert_eq!(document_text(&reseated), document_lines());
        assert_eq!(panel_text(&reseated, at(base, 9)), showing);
    }

    fn tree_without_the_tui_crate() -> Tree {
        Tree::new(
            Node::new("warlock", "warlock/WARLOCK.md", NodeState::PactedStale).with_children([
                Node::new("warlock/crates", None, NodeState::Unpacted).with_children([Node::new(
                    "warlock/crates/engine",
                    "warlock/crates/engine/WARLOCK.md",
                    NodeState::PactedFresh,
                )]),
                Node::new(
                    "warlock/assets",
                    "warlock/assets/WARLOCK.md",
                    NodeState::Unpacted,
                ),
            ]),
        )
    }

    fn selected_path(app: &App) -> Option<&Path> {
        app.selected_row().map(|row| row.path.as_path())
    }

    #[test]
    fn a_re_seat_takes_its_rows_its_states_and_its_tally_from_the_new_tree() {
        let app = App::from_tree(&fixture::tree());

        let reseated = reseat_on(&app, &fixture::tree_after_a_run());

        // The same five nodes, and the one the run worked on carrying the
        // document it wrote, in the state the new tree gives it — and drawing
        // that document beneath it, which is the row the whole view is for.
        assert_eq!(drawn(&reseated), whole_fixture_after_a_run());
        let crates = &reseated.rows()[2];
        assert_eq!(crates.path, PathBuf::from("warlock/crates"));
        assert_eq!(
            crates.document,
            Some(PathBuf::from("warlock/crates/WARLOCK.md")),
            "the document the run wrote never reached the row"
        );
        assert_eq!(crates.state, NodeState::PactedFresh);
        // The old app still says what it always said: a re-seat builds a new
        // value rather than editing the one it was handed.
        assert_eq!(app.rows()[2].path, PathBuf::from("warlock/crates"));
        assert_eq!(app.rows()[2].document, None);
        assert_eq!(app.rows()[2].state, NodeState::Unpacted);

        assert_eq!(reseated.counts(), fixture::tree_after_a_run().counts());
        assert_eq!(tally(&reseated), reseated.counts());
    }

    #[test]
    fn a_file_the_new_tree_lists_and_the_old_one_did_not_gets_a_row() {
        let mut app = App::from_tree(&fixture::tree());
        app.toggle_files();
        let written = "warlock/crates/WARLOCK.md".to_owned();
        assert!(!drawn(&app).contains(&written));

        let reseated = reseat_on(&app, &fixture::tree_after_a_run());

        assert!(reseated.show_files(), "the file toggle did not carry");
        assert!(
            drawn(&reseated).contains(&written),
            "the WARLOCK.md the run wrote is on disk and nowhere on screen"
        );
    }

    #[test]
    fn a_re_seat_keeps_the_selection_on_the_node_it_was_on() {
        let app = app_selecting("warlock/crates/tui");

        let reseated = reseat_on(&app, &fixture::tree_after_a_run());

        assert_eq!(
            selected_path(&reseated),
            Some(Path::new("warlock/crates/tui"))
        );
    }

    #[test]
    fn a_selection_the_new_tree_lost_lands_on_its_nearest_surviving_ancestor() {
        let app = app_selecting("warlock/crates/tui");

        let reseated = reseat_on(&app, &tree_without_the_tui_crate());

        // The directory the node was in, which is where it went — not the first
        // row, which is a different part of the tree entirely.
        assert_eq!(selected_path(&reseated), Some(Path::new("warlock/crates")));
        assert_ne!(reseated.selected(), 0);

        // The first row only when nothing on the way to the node survived at
        // all, which is the honest answer rather than a shortcut to it.
        let elsewhere = Tree::new(Node::new(
            "elsewhere",
            "elsewhere/WARLOCK.md",
            NodeState::Unpacted,
        ));
        let reseated = reseat_on(&app, &elsewhere);

        assert_eq!(reseated.selected(), 0);
        assert_eq!(selected_path(&reseated), Some(Path::new("elsewhere")));
    }

    #[test]
    fn a_re_seat_carries_the_collapsed_set_including_paths_the_new_tree_lacks() {
        let app =
            App::from_tree(&fixture::tree()).with_collapsed(["warlock/crates", "warlock/gone"]);

        let reseated = reseat_on(&app, &tree_without_the_tui_crate());

        // Both paths carried, the one the tree has no node for included: a
        // directory that comes back should come back shut.
        assert_eq!(reseated.collapsed(), app.collapsed());
        assert!(reseated.is_collapsed("warlock/gone"));
        // And it hides nothing, while the one the tree does have hides its
        // subtree in the new tree exactly as it did in the old one.
        assert_eq!(
            drawn(&reseated),
            ["warlock", "warlock/crates", "warlock/assets"]
        );
    }

    #[test]
    fn the_view_flags_and_the_window_survive_a_re_seat() {
        let mut app = App::from_tree(&fixture::tree());
        app.toggle_files();
        app.toggle_pacted_only();
        app.set_viewport_height(3);
        let mut app = select(app, "warlock/crates/tui");
        app.set_message("something from the last keystroke");
        app.set_pact_in_flight("warlock/crates", 2, 5);
        app.toggle_focus();

        let reseated = reseat_on(&app, &fixture::tree_after_a_run());

        assert!(reseated.show_files());
        assert!(reseated.pacted_only());
        assert_eq!(reseated.viewport_height(), 3);
        assert_eq!(reseated.focus(), Focus::Panel);
        assert_eq!(
            reseated.message(),
            Some("something from the last keystroke")
        );
        assert_eq!(reseated.pact_line(), app.pact_line());
        assert_eq!(selected_path(&reseated), selected_path(&app));
        // The window may have had to move by the rows the new tree added above
        // the selection, but never further than it had to, and never off the
        // end of the rows there are.
        assert!(window_is_in_range(&reseated));
        assert!(selection_is_on_screen(&reseated));
    }

    #[test]
    fn re_seating_on_a_tree_that_has_not_changed_leaves_the_view_exactly_as_it_was() {
        let mut app = App::from_tree(&fixture::tree());
        app.toggle_files();
        app.toggle_pacted_only();
        let mut app = select(app, "warlock/crates");
        app.toggle_collapsed();
        app.set_viewport_height(3);
        // Down to the bottom of the rows and back up into the middle of the
        // window, so the offset is one the window rule would not arrive at from
        // the top: a re-seat that rebuilt it instead of carrying it would put
        // the reader somewhere else and still look tidy.
        app.select_last();
        app.select_previous();
        app.select_previous();
        app.set_message("something from the last keystroke");
        app.set_pact_in_flight("warlock/crates", 2, 5);
        app.toggle_focus();
        // A view with something to lose in every field there is.
        assert!(app.scroll_offset() > 0);
        assert!(app.selected() < app.scroll_offset() + app.viewport_height() - 1);

        // What the binary does when a load turns up a tree nothing has happened
        // to: every field of the re-seated app is the field it carried, down to
        // the header, the scroll offset and the pact still in flight.
        let reseated = reseat_on(&app, &fixture::tree());

        assert_eq!(reseated, app);
    }

    fn tree_listing(path: &str) -> Tree {
        let mut tree = fixture::tree();
        assert!(
            list_file(&mut tree.root, Path::new(path)),
            "no node for the directory holding {path}"
        );
        tree
    }

    fn list_file(node: &mut Node, path: &Path) -> bool {
        if Some(node.path.as_path()) == path.parent() {
            node.files.push(path.to_path_buf());
            node.files.sort();
            return true;
        }
        node.children.iter_mut().any(|child| list_file(child, path))
    }

    fn app_with_files() -> App {
        let mut app = App::from_tree(&fixture::tree());
        app.toggle_files();
        app
    }

    fn as_the_document(mut app: App, path: &str) -> App {
        let path = Path::new(path);
        let mut found = false;
        for row in app.all_rows.iter_mut().chain(app.rows.iter_mut()) {
            if row.path == path {
                row.document_row = true;
                found = true;
            }
        }
        assert!(found, "no row for {}", path.display());
        app
    }

    #[test]
    fn inserting_a_file_row_lands_it_where_a_fresh_load_would_have_put_it() {
        // Sorts between `WARLOCK.md` and `logo.svg`, so neither appending nor
        // prepending would pass this.
        let mut app = app_with_files();

        app.insert_file_row("warlock/assets/index.html");

        let mut expected = App::from_tree(&tree_listing("warlock/assets/index.html"));
        expected.toggle_files();
        assert_eq!(drawn(&app), drawn(&expected));
        // Every field, not only the drawn paths: the depth, the state and the
        // whole walk behind the drawn rows are the load's as well — bar the
        // document flag, which an insertion decides for itself.
        assert_eq!(app, as_the_document(expected, "warlock/assets/index.html"));
    }

    #[test]
    fn an_inserted_file_row_goes_before_the_rows_for_its_directorys_children() {
        // `zzz.md` sorts after every file the root lists and after the name of
        // its child directory, so only the files-before-children rule of the
        // walk puts it in the right place.
        let mut app = app_with_files();

        app.insert_file_row("warlock/zzz.md");

        let mut expected = App::from_tree(&tree_listing("warlock/zzz.md"));
        expected.toggle_files();
        assert_eq!(app, as_the_document(expected, "warlock/zzz.md"));
        assert_eq!(
            &drawn(&app)[..4],
            [
                "warlock".to_owned(),
                "warlock/README.md".to_owned(),
                "warlock/WARLOCK.md".to_owned(),
                "warlock/zzz.md".to_owned(),
            ]
        );
        assert_eq!(drawn(&app)[4], "warlock/crates");
    }

    #[test]
    fn an_inserted_file_row_is_a_file_row_in_its_directorys_colour_now() {
        let mut app = app_with_files();
        // The colour a run has just painted, which is the whole point: the
        // state comes off the directory row as it stands, not off any tree.
        app.set_subtree_state("warlock/assets", NodeState::PactedFresh);

        app.insert_file_row("warlock/assets/WARLOCK.md");

        let row = app
            .rows()
            .iter()
            .find(|row| row.path == Path::new("warlock/assets/WARLOCK.md"))
            .expect("the inserted row is drawn");
        assert!(row.is_file());
        assert_eq!(row.depth, 2);
        assert_eq!(row.state, NodeState::PactedFresh);
        assert_eq!(row.document, None);
        assert_eq!(row.children, 0);
        assert!(!row.is_ignored());
    }

    #[test]
    fn an_inserted_file_row_carries_the_directorys_ignored_flag() {
        let tree = Tree::new(
            Node::new("repo", "repo/WARLOCK.md", NodeState::PactedStale).with_children([
                Node::new("repo/vendor", None, NodeState::Unpacted).with_ignored(true),
            ]),
        );
        let mut app = App::from_tree(&tree);
        app.toggle_files();

        app.insert_file_row("repo/vendor/WARLOCK.md");
        app.insert_file_row("repo/WARLOCK.md");

        let ignored = |app: &App, path: &str| {
            app.rows()
                .iter()
                .find(|row| row.path == Path::new(path))
                .map(Row::is_ignored)
        };
        assert_eq!(ignored(&app, "repo/vendor/WARLOCK.md"), Some(true));
        assert_eq!(ignored(&app, "repo/WARLOCK.md"), Some(false));
    }

    #[test]
    fn inserting_a_path_already_in_the_walk_changes_nothing() {
        let app = app_with_files();

        // A file the load already listed: a re-pact of a directory that was
        // documented before the run started.
        let mut again = app.clone();
        again.insert_file_row("warlock/assets/logo.svg");
        assert_eq!(again, app);

        // And a second delivery of a row this method itself put there.
        let mut twice = app.clone();
        twice.insert_file_row("warlock/assets/index.html");
        let once = twice.clone();
        twice.insert_file_row("warlock/assets/index.html");
        assert_eq!(twice, once);
        assert_eq!(
            twice
                .rows()
                .iter()
                .filter(|row| row.path == Path::new("warlock/assets/index.html"))
                .count(),
            1,
        );
    }

    #[test]
    fn inserting_under_a_directory_with_no_row_changes_nothing_at_all() {
        let app = app_with_files();

        for path in [
            // No node for the directory.
            "warlock/nowhere/WARLOCK.md",
            // Nor for a file's own path read as one.
            "warlock/assets/logo.svg/WARLOCK.md",
            // Nor above the root of the tree on screen.
            "elsewhere/WARLOCK.md",
            // And a path with no directory to be in.
            "WARLOCK.md",
        ] {
            let mut after = app.clone();
            after.insert_file_row(path);
            assert_eq!(after, app, "{path} moved something");
        }
    }

    #[test]
    fn inserting_a_file_row_leaves_the_tally_alone() {
        let mut app = app_with_files();
        let before = app.counts();
        assert_eq!(before, fixture::tree().counts());

        app.insert_file_row("warlock/assets/index.html");
        app.insert_file_row("warlock/crates/WARLOCK.md");

        // Files are counted nowhere, so not one field of the footer's tally
        // moves — not even the total.
        assert_eq!(app.counts(), before);
        assert_eq!(app.counts().total(), before.total());
    }

    #[test]
    fn an_inserted_file_row_survives_a_rebuild_of_the_drawn_rows() {
        let mut app = app_with_files();

        app.insert_file_row("warlock/assets/index.html");
        let after = drawn(&app);

        // The file toggle, which rebuilds the drawn rows from the whole walk:
        // a row written only into the drawn list would go here and never come
        // back.
        app.toggle_files();
        app.toggle_files();
        assert_eq!(drawn(&app), after);

        // And a collapse over it, which rebuilds them again.
        let mut app = select(app, "warlock/assets");
        app.toggle_collapsed();
        app.toggle_collapsed();
        assert_eq!(drawn(&app), after);
    }

    #[test]
    fn with_files_hidden_a_document_written_mid_run_is_drawn_at_once() {
        // The case the first clause of the guard exists for. `crates/` had no
        // document, a pass has this moment written one, and the reader is in the
        // default view — the one warlock opens on, where the files a directory
        // merely holds are hidden. The row goes in there and then rather than
        // waiting for `f` or for the reload at the end of the run.
        let mut app = App::from_tree(&fixture::tree());
        app.set_subtree_state("warlock/crates", NodeState::PactedFresh);

        app.insert_file_row("warlock/crates/WARLOCK.md");

        // What the reload at the end of that run draws, reached the way the
        // front end reaches it: the view carried onto the tree a second load
        // finds. Path and depth, because the two apps are two accounts of the
        // same screen and not the same value — the reload knows `crates/` has a
        // document and the mid-run tree does not.
        let reloaded = reseat_on(&app, &fixture::tree_after_a_run());
        assert_eq!(
            paths_and_depths(app.rows()),
            paths_and_depths(reloaded.rows())
        );
        // And the walk behind them, which is what the next rebuild of the drawn
        // rows will be made from: the row went in at the load's index there too,
        // among the files the load would have listed and not merely on screen.
        assert_eq!(
            paths_and_depths(&app.all_rows),
            paths_and_depths(&reloaded.all_rows)
        );
        assert_eq!(drawn(&app), whole_fixture_after_a_run());
        // Nothing else came with it: this is one row, not the file toggle.
        assert!(!app.show_files());
        assert!(!drawn(&app).contains(&"warlock/README.md".to_owned()));
    }

    #[test]
    fn an_insertion_is_the_documents_row_whatever_the_file_is_called() {
        // `insert_file_row` is not "the loader listed one more file". Its caller
        // is the pact observer, which passes the `WARLOCK.md` a pass has just
        // written, so the row is a document row by construction — no name is
        // compared, here or anywhere in this crate — and it is drawn with files
        // hidden like every other document row.
        let mut app = App::from_tree(&fixture::tree());

        app.insert_file_row("warlock/assets/index.html");

        let row = app
            .rows()
            .iter()
            .find(|row| row.path == Path::new("warlock/assets/index.html"))
            .expect("a document row is drawn with files hidden");
        assert!(row.is_document());
        assert!(row.is_file());
        assert_eq!(row.depth, 2);
        // And the whole of what the default view gained is that one row, in
        // path order among the rows `assets/` already had.
        let mut expected = whole_fixture();
        expected.push("warlock/assets/index.html".to_owned());
        assert_eq!(drawn(&app), expected);
    }

    #[test]
    fn an_insertion_under_a_collapsed_directory_waits_for_the_expand() {
        let mut app = select(app_with_files(), "warlock/assets");
        app.toggle_collapsed();
        let before = drawn(&app);

        app.insert_file_row("warlock/assets/index.html");

        assert_eq!(
            drawn(&app),
            before,
            "nothing is drawn under a collapsed row"
        );

        app.toggle_collapsed();

        let mut expected = App::from_tree(&tree_listing("warlock/assets/index.html"));
        expected.toggle_files();
        assert_eq!(drawn(&app), drawn(&expected));
    }

    #[test]
    fn the_filter_draws_an_inserted_file_only_under_a_pacted_directory() {
        let mut app = app_with_files();
        app.toggle_pacted_only();
        let before = drawn(&app);

        // `warlock/assets` is unpacted, so the filter would drop a file row
        // taking its colour, exactly as it drops the two it already has.
        app.insert_file_row("warlock/assets/index.html");
        assert_eq!(drawn(&app), before);

        // `warlock/crates/engine` is pacted, so the filter keeps its files.
        app.insert_file_row("warlock/crates/engine/index.html");

        let mut expected = App::from_tree(&tree_listing("warlock/crates/engine/index.html"));
        expected.toggle_files();
        expected.toggle_pacted_only();
        assert_eq!(drawn(&app), drawn(&expected));

        // Both rows are in the walk all the same: the filter is a view, and
        // widening it puts the one it dropped back.
        app.toggle_pacted_only();
        assert!(
            drawn(&app).contains(&"warlock/assets/index.html".to_owned()),
            "the walk kept the row the filter would not draw"
        );
    }

    #[test]
    fn an_insertion_above_the_selection_keeps_the_selection_on_its_row() {
        let mut app = app_with_files_selecting("warlock/assets/logo.svg");
        let selected = app.selected();

        // Sorts before the selected row, so the index it sits at is no longer
        // the index it belongs at.
        app.insert_file_row("warlock/assets/index.html");

        assert_eq!(
            app.selected_row().map(|row| row.path.clone()),
            Some(PathBuf::from("warlock/assets/logo.svg"))
        );
        assert_eq!(app.selected(), selected + 1);

        // A row landing below the selection moves nothing.
        let selected = app.selected();
        app.insert_file_row("warlock/assets/zebra.svg");
        assert_eq!(app.selected(), selected);
        assert_eq!(
            app.selected_row().map(|row| row.path.clone()),
            Some(PathBuf::from("warlock/assets/logo.svg"))
        );
        // And the message line is untouched throughout: this is not a keystroke.
        assert_eq!(app.message(), None);
    }

    #[test]
    fn an_insertion_above_the_window_keeps_the_same_rows_on_screen() {
        let mut app = app_with_files_selecting("warlock/assets/logo.svg");
        app.set_viewport_height(4);
        let offset = app.scroll_offset();
        assert!(offset > 0, "the window has scrolled off the top");
        let on_screen: Vec<String> = drawn(&app)[offset..offset + 4].to_vec();

        // Above the window, so every row on screen would slide down a line if
        // the offset were left where it was.
        app.insert_file_row("warlock/README.txt");

        let offset = app.scroll_offset();
        assert_eq!(drawn(&app)[offset..offset + 4], on_screen[..]);
        assert!(window_is_in_range(&app));
        assert!(selection_is_on_screen(&app));
    }
}
