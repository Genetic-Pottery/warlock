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
use crate::selection::{Position, Selection};

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
    // How far through the current directory's per-file passes the run is, once
    // it has told us. `None` until the first file, and again for every
    // directory that pays for no file at all — a refresh where every line came
    // off the page unchanged never reports one, and the bar falls back to
    // counting whole directories, which is what it counted before.
    files: Option<(usize, usize)>,
}

impl InFlight {
    // What is finished, which is the node being worked less itself. The header,
    // its bar and the footer line all count this way, so the three cannot
    // disagree about how far along a run is.
    const fn done(&self) -> usize {
        self.position.saturating_sub(1)
    }
}

/// The run in flight, as the header draws it.
///
/// [`RunHeader::position`] is the furthest node this run has reached, which is
/// the one it is working now — it is what the `(3/12)` on the header counts, so
/// it is 1 the moment the first directory starts.
///
/// [`RunHeader::completed`] is what the bar fills to, and it is that number
/// less the one in flight. A directory being described is not a directory
/// described: counting it would draw a full bar over a run that has finished
/// nothing, which is what a single-directory pact used to look like from the
/// keystroke to the document.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RunHeader {
    run: Run,
    directory: String,
    position: usize,
    total: usize,
    files: Option<(usize, usize)>,
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

    /// How many directories this run has finished: [`Self::position`] less the
    /// one being worked. Zero before anything is done, and never `total` — the
    /// header belongs to a run in flight, and a run with nothing left in flight
    /// has no header to draw.
    #[must_use]
    pub const fn completed(&self) -> usize {
        self.position.saturating_sub(1)
    }

    /// Files described of files this directory is paying for, straight from
    /// the engine's `Observer::describing`. `None` before the first file of a
    /// directory, and for a directory that pays for none.
    #[must_use]
    pub const fn files(&self) -> Option<(usize, usize)> {
        self.files
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
///
/// `highlight` is positions in the panel's thread rather than anything the tree
/// holds, so it goes wherever the panel goes and nowhere else: carried across a
/// reload, kept by [`App::restore_from`] over the view it rolls back to.
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
    // How many times the footer has been told something, which is what lets the
    // session time a message out: the same sentence said twice is two sayings,
    // and a reader who has just copied the same number of characters again is
    // owed the full ten seconds rather than what was left of the first.
    said: u64,
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
                said: 0,
                in_flight: None,
                pact_refused: false,
                mouse_captured: false,
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
    pub const fn said(&self) -> u64 {
        self.status.said
    }

    /// Said long enough ago that the footer would be claiming a moment that has
    /// passed. Not a keystroke's business — [`App::forget_last_keystroke`] is
    /// what a movement clears — so it is separate from that and from the run
    /// line, which outlives any number of messages.
    pub fn forget_message(&mut self) {
        self.status.message = None;
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
            // Cleared rather than carried: this is a new directory, and the
            // fraction that was on screen belonged to the last one.
            files: None,
        });
    }

    /// Where the directory in flight is through its own files, for the bar to
    /// fill between one directory and the next.
    ///
    /// Dropped when no run is in flight rather than remembered against the next
    /// one: a fraction of a directory nobody is working is not a fact about
    /// anything. Monotonic within a directory because the engine counts up and
    /// `set_run_in_flight` clears this on the way in, so nothing here has to
    /// guard a fraction going backwards the way `reached` does.
    pub fn set_files_in_flight(&mut self, position: usize, total: usize) {
        if let Some(in_flight) = self.status.in_flight.as_mut() {
            in_flight.files = Some((position, total));
        }
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
                // Finished, not started, the same as the bar and the header:
                // see `RunHeader::completed`.
                Run::Pact => pacting_message(&label, in_flight.done(), in_flight.total),
                Run::Refresh => refreshing_message(&label, in_flight.done(), in_flight.total),
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
            files: in_flight.files,
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
        // The highlight is drawn on the thread card, so a card taking its place
        // would leave it standing over text it was never measured against.
        self.clear_selection();
        self.rescue_focus();
    }

    #[must_use]
    pub const fn panel(&self) -> &Panel {
        &self.panel
    }

    pub const fn panel_mut(&mut self) -> &mut Panel {
        &mut self.panel
    }

    /// What the reader has highlighted in the thread card, if anything.
    #[must_use]
    pub fn selection(&self) -> Option<Selection> {
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
    pub fn start_selection(&mut self, at: Position) {
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
    pub fn extend_selection(&mut self, to: Position) {
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

    pub fn clear_selection(&mut self) {
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

    /// Rolls the view back to an earlier copy — but keeps the live panel, since
    /// an account or a conversation is a record of what happened and rolling it
    /// back would discard it at the moment the reader turned to read it.
    pub fn restore_from(&mut self, view: Self) {
        let panel = mem::take(&mut self.panel);
        // Positions in the thread on the panel that stayed, so the copy's idea
        // of what was highlighted is about a conversation that is not the one
        // being kept.
        let highlight = self.highlight;
        *self = view;
        self.panel = panel;
        self.highlight = highlight;
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
        let mut kept: Cow<'_, [Row]> = if self.viewpoint.show_files {
            Cow::Borrowed(&self.all_rows)
        } else {
            Cow::Owned(node_rows(&self.all_rows))
        };
        if self.viewpoint.pacted_only {
            kept = Cow::Owned(pacted_rows(&kept));
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
                self.say(message);
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
        if !toggle.pacted {
            self.say(left_on_disk_message(&self.label_for(&toggle.path)));
        }
    }

    // For the keys that refuse a file row outright. `view_target` deliberately
    // does not come through here: `v` hands a file row back rather than
    // refusing it, so routing it here would turn the one key that reads a file
    // into a key that says a file cannot be read.
    fn selected_directory(&mut self) -> Option<(PathBuf, NodeState)> {
        let row = self.rows.get(self.selected)?;
        let path = row.path.clone();
        let state = row.state;

        if row.is_file() {
            self.say(file_row_message(&self.label_for(&path)));
            return None;
        }
        Some((path, state))
    }

    pub fn refresh(&mut self) -> Option<PathBuf> {
        let (path, state) = self.selected_directory()?;
        match state {
            NodeState::Unpacted => {
                self.say(unpacted_message(&self.label_for(&path)));
                None
            }
            NodeState::PactedFresh => {
                self.say(already_fresh_message(&self.label_for(&path)));
                None
            }
            NodeState::PactedStale => {
                self.status.message = None;
                Some(path)
            }
        }
    }

    pub fn scope_target(&mut self) -> Option<PathBuf> {
        let (path, state) = self.selected_directory()?;
        match state {
            NodeState::Unpacted => {
                self.say(unpacted_scope_message(&self.label_for(&path)));
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
        self.say(document.map_or_else(
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
    let after = directory + 1;
    let before_it = rows
        .iter()
        .skip(after)
        .take_while(|row| row.is_file() && row.depth == depth && row.path.as_path() <= path)
        .count();
    after + before_it
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
        counts: tree.counts(),
        selected: 0,
        scroll_offset: 0,
        viewpoint: viewpoint.clone(),
        status: status.clone(),
        panel: panel.clone(),
        highlight: *highlight,
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
    row.path.starts_with(root) && !(state.is_pacted() && row.is_ignored())
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

fn pacting_message(label: &str, done: usize, total: usize) -> String {
    format!("pacting {label} ({done}/{total})")
}

fn refreshing_message(label: &str, done: usize, total: usize) -> String {
    format!("refreshing {label} ({done}/{total})")
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
#[path = "tests/app.rs"]
mod tests;
