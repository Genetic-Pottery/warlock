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
//! Showing the files inside a directory is the third flag of that same shape.
//! The files a node lists are flattened into rows alongside the nodes, once,
//! when the app is built; the flag decides whether those rows are drawn. Doing
//! it that way rather than splicing files in when the flag goes on is what lets
//! the flag go off again without the tree: the rows were never thrown away, so
//! turning files off is the same filter-and-re-flatten every other view change
//! goes through. A file row is a row for something that is not a node — no
//! document, no children, and the state of the directory holding it, because
//! the colour of a file is the colour of its module — so the operations that
//! act on nodes refuse it rather than half-working on it.
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
//! Which of the screen's two panes the keys drive is view state of the same
//! kind, and lives here for the same reason: it is a fact about what the reader
//! is looking at, it changes what a keystroke does, and a rule about keystrokes
//! that only an event loop with a terminal attached could demonstrate is a rule
//! nobody can test. See [`Focus`].
//!
//! A pointer asks for the same moves in a different grammar, and gets methods of
//! its own rather than a mode on the keys': it names a row instead of a
//! direction, and it names the pane it is over instead of accepting whichever
//! pane the keys are driving. So [`App::select_row`], [`App::select_next_by`],
//! [`App::select_previous_by`], [`App::scroll_panel_down`] and
//! [`App::scroll_panel_up`] never consult the focus, while landing in the same
//! places the keys land — a wheel notch over the tree is three presses of the
//! down key, a wheel notch over the panel is three of them at the panel, follow
//! rule and all. Which pointer landed where is somebody else's arithmetic; this
//! type is told the answer.
//!
//! A pact in flight is the same kind of thing said over a longer span. A subtree
//! pact is minutes of work happening somewhere else, so the app holds which
//! directory that work is on and how far down the list it has got — set and
//! cleared by whoever is running it, since only they know — and the renderer
//! draws a line from it. It is deliberately not a message: a message belongs to
//! the last keystroke and the next keystroke takes it down, while a pact goes on
//! running whatever the reader presses, so the two are separate fields and
//! movement clears only the one that belongs to a keystroke.
//!
//! What that pact has been *doing* is the same kind of thing again, held one
//! field along: the [`Account`] of the run, or nothing at all before the first
//! pact of the session. The account is the panel's contents and the panel's
//! window onto it is view state exactly as the tree's is — a height set per
//! frame, an offset, and one bit saying whether the window is following the
//! newest line. Following is not a mode the app has to be reminded of on every
//! appended line: while the flag is on, the offset *is* the end of the account,
//! computed when it is asked for, so a line arriving during a redraw pins itself
//! to the bottom by arithmetic rather than by a hook somebody could forget to
//! call. Scrolling up turns the flag off and the window stops where the reader
//! left it; scrolling back to the end turns it on again, because being at the
//! end is the whole of what following means.
//!
//! Nothing here touches a terminal: it is a plain data structure with plain
//! methods, so every rule about how the selection moves is testable with
//! nothing attached to stdout.

use std::borrow::Cow;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::Instant;

use warlock_engine::{IntoDocument, NodeState, StateCounts, Tree, to_manifest_path};

use crate::account::{Account, Line};

/// One line of the flattened tree: what to draw, how far to indent it, and
/// which colour it takes.
///
/// A row owns its path rather than borrowing from the tree, so [`App`] is a
/// self-contained value that can be built, moved and asserted on without
/// lifetimes threading through the event loop.
///
/// The document comes along for the same reason: pacting a node needs one, and
/// the row is what the key handler has in its hand when the key is pressed.
/// Fetching it back out of the tree at that moment would mean keeping the tree
/// alongside the rows and looking a path up in it, which is two sources for one
/// fact.
///
/// The child count comes along for a third reason of the same shape: a
/// directory with no children can be neither collapsed nor expanded, and a row
/// that could not say so would send the renderer back to the tree to find out
/// whether to draw a marker.
///
/// Most rows stand for a node. A row can also stand for one of the files a node
/// lists — see [`Row::file`] — which is drawn like any other row and is nothing
/// like one otherwise: it documents nothing, contains nothing, and is counted
/// nowhere.
///
/// Whether a `.warlockignore` keeps the row's content out comes along for the
/// same reason the document does: the pact key has to refuse such a row, and
/// [`App`] touches no filesystem, so the fact has to be here by the time the key
/// is pressed. See [`Row::is_ignored`].
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Row {
    /// How deep the node sits: `0` for the root, `1` for its children.
    pub depth: usize,
    /// The path of the node this row stands for, exactly as the engine stores
    /// it.
    pub path: PathBuf,
    /// The `WARLOCK.md` documenting the node, straight from
    /// [`warlock_engine::Node`], or `None` for an ordinary directory that has
    /// no documentation yet.
    pub document: Option<PathBuf>,
    /// What Warlock knows about the node, which is what colours the row.
    pub state: NodeState,
    /// How many child *nodes* the node has in the tree — not how many are
    /// drawn, which is none of them while the node is collapsed, and not how
    /// many rows hang under it, which counts the files it lists as well.
    ///
    /// A fact about the tree, and only that. Whether collapsing this row would
    /// hide anything is a question about the view — the file toggle and the
    /// pacted-only filter both change the answer without the tree moving — so it
    /// is [`App::can_collapse`] that decides it, not this.
    pub children: usize,
    /// Whether the row stands for a file rather than a node. See [`Row::file`],
    /// and ask it with [`Row::is_file`] rather than reading this: what the flag
    /// *means* is the interesting part.
    pub file: bool,
    /// Whether the repository's `.warlockignore` keeps this row's content out of
    /// Warlock, straight from [`warlock_engine::Node::is_ignored`]. Ask it with
    /// [`Row::is_ignored`], and set it with [`Row::with_ignored`].
    pub ignored: bool,
}

impl Row {
    /// A row for a childless node at `path`, documented by `document`, sitting
    /// at `depth`, in `state`.
    ///
    /// `document` takes whatever [`warlock_engine::Node::new`] takes: anything
    /// path-like for a node that has one, or `None` for a directory that has
    /// no documentation yet.
    ///
    /// Childless is the safe default rather than the common case: a row that
    /// claims children it does not have is a row the collapse key hides
    /// nothing with. Say otherwise with [`Row::with_child_count`].
    ///
    /// Covered by Warlock is the safe default in the same way: a row wrongly
    /// claiming a `.warlockignore` keeps it out is a row the pact key refuses
    /// for a reason nobody wrote down. Say otherwise with
    /// [`Row::with_ignored`].
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
            ignored: false,
        }
    }

    /// A row for the file at `path`, sitting at `depth`, in `state`.
    ///
    /// A file is not a node: it has no document of its own, no children, and no
    /// state the engine ever decided for it. What it has is the state of the
    /// directory holding it, handed over here by the caller doing the
    /// flattening, because the design doc's rule is that a file takes its
    /// module's colour — the colour says which module the file belongs to, not
    /// something about the file. That copy is the reason
    /// [`App::set_subtree_state`] writes a new state onto a directory's file
    /// rows as well as onto the directory: a copy nobody updates is a colour
    /// that goes quietly stale.
    ///
    /// `depth` is the caller's too, and is one deeper than the directory's, so
    /// the file indents under it.
    #[must_use]
    pub fn file(depth: usize, path: impl Into<PathBuf>, state: NodeState) -> Self {
        Self {
            file: true,
            ..Self::new(depth, path, None, state)
        }
    }

    /// The same row, standing for a node with `children` children of its own.
    #[must_use]
    pub const fn with_child_count(mut self, children: usize) -> Self {
        self.children = children;
        self
    }

    /// The same row, with the repository's `.warlockignore` keeping its content
    /// out of Warlock, or not.
    ///
    /// [`App::from_tree`] says this from [`warlock_engine::Node::is_ignored`],
    /// and it is the whole of how the fact reaches [`App::toggle_pact`]. It is a
    /// builder rather than an argument to [`Row::new`] so that a test can hand a
    /// row over without a tree, a loader or a disk behind it.
    #[must_use]
    pub const fn with_ignored(mut self, ignored: bool) -> Self {
        self.ignored = ignored;
        self
    }

    /// Whether the node has child nodes in the tree.
    ///
    /// Not the question the collapse key asks — see [`App::can_collapse`]. A
    /// directory holding nothing but files has no children by this and still
    /// has rows under it when the file toggle is on; a directory whose children
    /// the pacted-only filter has taken away has children by this and nothing
    /// under it on screen.
    #[must_use]
    pub const fn has_children(&self) -> bool {
        self.children > 0
    }

    /// Whether the row stands for a file rather than for a node.
    ///
    /// The one bit that tells a file row from a childless, undocumented
    /// directory, which is otherwise the same set of fields. Everything that
    /// acts on a node — pacting, above all — asks this first.
    #[must_use]
    pub const fn is_file(&self) -> bool {
        self.file
    }

    /// Whether a `.warlockignore` in the repository keeps this row's content out
    /// of Warlock.
    ///
    /// A fact carried from the load, never worked out here: this reads a stored
    /// flag and opens nothing, which is what lets [`App::toggle_pact`] refuse
    /// such a row without a filesystem under it.
    ///
    /// It says nothing about how the row is drawn. An excluded directory loads
    /// as [`NodeState::Unpacted`] and is gray like any other unpacted directory
    /// — gray already means outside Warlock's management — so there is no
    /// colour, shade or marker of its own for this.
    #[must_use]
    pub const fn is_ignored(&self) -> bool {
        self.ignored
    }
}

/// What a pact toggle asked for, for the caller that has to carry it out.
///
/// [`App::toggle_pact`] moves the colours on screen; the documents and the
/// manifest are somebody else's, so this says which directory the key was
/// pressed on and which way it went, and lets that somebody act on it.
///
/// There is no document in here, and no list of directories either. A pact
/// covers the directory and everything below it, and the documents are the pact
/// operation's own output rather than something the front end finds and hands
/// over: the one directory the user chose is the whole of what a caller needs
/// to run that operation over the subtree, or to undo it.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PactToggle {
    /// The directory the key was pressed on, whose subtree the toggle covers.
    pub path: PathBuf,
    /// Whether the subtree is pacted *now*, after the toggle: `true` means the
    /// pact operation should run over it, `false` that its entries should go.
    pub pacted: bool,
}

/// Which kind of run is in flight: a pact over a whole subtree, or a refresh
/// over the stale parts of one.
///
/// The two runs are the same run in every way the app cares about — one worker,
/// one channel, one account, one cancel, one line on the footer — and differ in
/// exactly one: the verb that line is worded with. So this is a kind on the one
/// in-flight record rather than a second in-flight state, and everything that
/// asks whether something is running ([`App::is_pacting`], [`App::is_in_flight`],
/// [`App::in_flight_covers`]) goes on asking it without knowing which kind it
/// got.
///
/// It is public because the caller starting the run is the only one who knows
/// which it started: see [`App::set_run_in_flight`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Run {
    /// A pact: every directory in the subtree is described, whatever state it
    /// was in.
    Pact,
    /// A refresh: only the stale directories of the subtree are described, and
    /// the fraction counts those rather than all of them.
    Refresh,
}

/// A pact running somewhere else, as far as the screen is concerned: the
/// directory being worked now, where it sits in the run, and which kind of run
/// it is.
///
/// The directory is kept as the path the caller was handed, not as finished
/// text, so the label is spelled relative to the root of the tree *on screen*
/// when it is drawn — see [`App::pact_line`]. `position` is one-based and
/// counts directories, so it reads as `(3/12)` beside a `total` that does not
/// move for the length of the run.
///
/// Private, and the only thing given out of it is the yes-or-no of
/// [`App::is_in_flight`]: what a caller can otherwise do with it is put it
/// there, take it away, and ask for the line it makes. The path is answered
/// against, never handed back, and the position and total are not given out at
/// all — handing the parts back would be a second place the wording could be
/// decided. A renderer needs to know which row of the tree is the one being
/// worked, and asking about a path it already holds settles that without
/// learning how the run is spelled.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct InFlight {
    /// The directory the pact is working now.
    path: PathBuf,
    /// Which directory of the run this is, counting from one.
    position: usize,
    /// How many directories the whole run covers.
    total: usize,
    /// Whether the run is a pact or a refresh, which decides the verb
    /// [`App::pact_line`] words the line with and nothing else.
    run: Run,
}

/// A summarising pass running inside the directory a pact is working: the file
/// it is about, and which pass of how many it is.
///
/// An over-cap file is read in chunks and summarised a chunk at a time, so a
/// single directory can be a dozen model passes over one file, minutes long, with
/// nothing on [`InFlight`] moving for the whole of it. This is what turns that
/// silence into a fraction that advances. `part` is one-based and `parts` is the
/// number of passes that file costs — the engine's own counting, see
/// `Observer::summarising` — so it reads as `(2/5)` beside a `parts` that does
/// not move for the length of that file.
///
/// The file is kept as the path the caller was handed rather than as finished
/// text, for the same reason [`InFlight`] keeps its directory that way: the label
/// is spelled relative to the root of the tree *on screen* at draw time, by
/// [`App::pact_line`].
///
/// Private, and nothing at all is given out of it — not even the yes-or-no
/// [`InFlight`] answers. A caller can put it there, and it goes when the
/// directory changes or the run ends; the only thing it does is add a clause to
/// the one line the app words.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct Summarising {
    /// The file the pass about to run is over.
    path: PathBuf,
    /// Which pass over that file this is, counting from one.
    part: usize,
    /// How many passes that file costs in total.
    parts: usize,
}

/// Which of the screen's two panes the keys are driving.
///
/// The screen is a tree column and a panel beside it, and a key that moves a
/// selection has to be about one of them; this says which. Two variants and no
/// third: the footer runs the width of the screen and is nobody's to drive, so
/// there is nothing else focus could land on.
///
/// It is deliberately not a general "which widget has the cursor" — nothing here
/// is a widget and there is no cursor. It is one bit of view state, toggled by
/// one key, read by the renderer to decide which border is lit and by [`App`] to
/// decide which pane a movement key is about: see [`App::toggle_focus`] and
/// [`App::focus`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum Focus {
    /// The tree column. The movement keys move its selection, which is what
    /// they have always done, and this is where a freshly built [`App`] starts:
    /// the tree is what warlock opens on and the thing there is anything to do
    /// with yet.
    #[default]
    Tree,
    /// The panel beside it. The same movement keys scroll the panel's window
    /// over the account of the pact — see [`App::panel_scroll_offset`] — and the
    /// tree's selection stays exactly where the reader left it.
    Panel,
}

impl Focus {
    /// The other pane: what [`App::toggle_focus`] moves to.
    ///
    /// Written as a method on the two-variant enum rather than as a `!` on a
    /// boolean somewhere, so that "focus is one of these places" stays the thing
    /// the type says and a third pane would be a compile error here rather than
    /// a silent wrong answer.
    #[must_use]
    pub const fn other(self) -> Self {
        match self {
            Self::Tree => Self::Panel,
            Self::Panel => Self::Tree,
        }
    }

    /// Whether a movement key moves the tree's selection: whether this focus is
    /// the one driving the tree column.
    ///
    /// The single place the rule is written down, so every movement method asks
    /// the same question rather than each of them matching on the enum in its
    /// own way. `false` does not mean the key does nothing — it means the key is
    /// the panel's, and scrolls the panel's window instead. See `App::movement`.
    #[must_use]
    pub const fn drives_the_tree(self) -> bool {
        matches!(self, Self::Tree)
    }
}

/// The front end's state: the flattened tree, which of it is collapsed, the
/// selected row, the slice of rows on screen, the tally the footer shows and
/// the header line naming what is being shown.
///
/// `all_rows` is the engine's whole walk as it was when the app was built. No
/// view change touches it, and a reload replaces it whole; the only thing that
/// writes into it is news a reload cannot be waited for — a state a running pact
/// has earned ([`App::set_subtree_state`]) and a document it has just written
/// ([`App::insert_file_row`]). `rows` is what is actually drawn, rebuilt
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
/// `show_files` is the third, and the only one that can put a row on screen
/// rather than take one off: `all_rows` holds a row for every file the tree
/// listed as well as one for every node, and with the flag off — which is how an
/// app starts — every one of those file rows is filtered out before anything
/// else looks at the list. Keeping them in `all_rows` rather than splicing them
/// in when the flag goes on is what makes the flag reversible without the tree,
/// exactly as `collapsed` is.
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
///
/// `focus` is which of the screen's two panes the keys are driving, and it is
/// here rather than in the event loop for the reason everything else here is:
/// it is view state that changes what a keystroke does, and the rule it decides
/// — that a movement key moves the tree's selection while the tree has the focus
/// and scrolls the panel while the panel has it — is a rule about this type's
/// methods, testable with nothing attached to stdout. It starts on the tree,
/// which is the pane warlock opens on. See [`Focus`].
///
/// `account` is what the pact running now, or the last one to run, has been seen
/// doing. It is `None` until the first pact of the session, which is not the
/// same as an empty account: an app that has never run a pact has nothing to
/// draw in the panel at all, not even a heading, and the difference between "no
/// account" and "an account with no lines yet" is the difference between a blank
/// panel and one that has started. [`App::start_account`] is what turns the one
/// into the other, and a second pact starts a second account rather than
/// appending to the first: one pact, one account.
///
/// `panel_height`, `panel_offset` and `panel_follows` are that account's window,
/// and are to the panel what `viewport_height` and `scroll_offset` are to the
/// tree — with one difference, which is the flag. The tree's window is dragged
/// about by a selection; the panel has no selection, so its window is either
/// pinned to the newest line or parked where the reader put it, and
/// `panel_follows` says which. While it is set, `panel_offset` is not read at
/// all: the offset is the end of the account, worked out from the line count at
/// the moment it is asked for, so appending a line moves the window without
/// anybody having to tell the window that a line was appended. See
/// [`App::panel_scroll_offset`].
///
/// `mouse_captured` is whether the terminal is reporting its mouse, and it is the
/// one field here that is not the app's own: the terminal is the binary's to
/// switch, so the loop that switches it tells the app what it did, every frame,
/// the way it tells it the two window heights. It is here for the footer's sake
/// alone — the keys line names the `m` key by what the *next* press of it does,
/// so it has to know what this press left behind — and nothing in this type reads
/// it. `false` for a freshly built app, which is the truth about one nobody has
/// put on a terminal yet. See [`App::set_mouse_captured`].
///
/// `in_flight` is the pact running now, if one is, and is the one piece of state
/// here that no keystroke touches: it is put there and taken away by whoever is
/// running the pact — see [`App::set_pact_in_flight`] — because the app cannot
/// see a background thread and the thread cannot see a screen. It outlasts
/// keystrokes for that reason, and takes the message line while it is there:
/// see [`App::pact_line`].
///
/// `summarising` is the pass over one big file happening inside that directory,
/// if one is, and is set and cleared by the same caller for the same reason —
/// see [`App::set_pact_summarising`]. It hangs off `in_flight` rather than
/// standing beside it: it is wording added to that one line, and it goes whenever
/// `in_flight` moves or goes, so no file is ever named under a directory the run
/// has left.
///
/// `collapsible` is the paths of the rows that have something under them *in
/// this view* — which is not what the tree says, because the file toggle and the
/// pacted-only filter both change what a row holds without the tree moving. It
/// is derived beside `rows`, in the one pass that already knows: see
/// [`drawn_rows`], and [`App::can_collapse`] for what reads it. Kept as paths
/// rather than as a flag on [`Row`] so that a row stays a fact about the tree
/// and two rows for the same node compare equal whatever is filtered.
///
/// `pact_refused` is the one keystroke that has nowhere else to go: the pact key
/// pressed while `in_flight` is already there. It is a flag rather than a
/// message because the message line is exactly what a pact in flight has taken —
/// a sentence put in `message` would be the one sentence nobody could read — and
/// it is a flag rather than a fourth footer line because the footer is a fixed
/// three. So it is a bit that changes how `in_flight` is worded, and nothing
/// more: see [`App::set_pact_refused`] and [`App::pact_line`]. It belongs to the
/// keystroke that set it, so it goes the way `message` does, on the next one.
/// Holds an [`Account`], which holds an [`f64`] cost, so it is [`PartialEq`] and
/// not [`Eq`].
#[derive(Debug, Clone, Default, PartialEq)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "the five flags here are independent facts about the view — files \
              shown, pacted-only, panel following, pact refused, mouse captured \
              — and every one of the thirty-two combinations is a state the \
              screen can honestly be in. Folding them into a state machine would \
              invent an ordering between things that have none, and cost the \
              derives a bool keeps free"
)]
pub struct App {
    all_rows: Vec<Row>,
    rows: Vec<Row>,
    collapsed: BTreeSet<PathBuf>,
    collapsible: BTreeSet<PathBuf>,
    pacted_only: bool,
    show_files: bool,
    selected: usize,
    scroll_offset: usize,
    viewport_height: usize,
    counts: StateCounts,
    header: String,
    message: Option<String>,
    in_flight: Option<InFlight>,
    summarising: Option<Summarising>,
    pact_refused: bool,
    focus: Focus,
    account: Option<Account>,
    panel_height: usize,
    panel_offset: usize,
    panel_follows: bool,
    mouse_captured: bool,
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
    ///
    /// The files each node lists are flattened in here too, each one a
    /// [`Row::file`] straight after the row for the directory holding it, one
    /// level deeper and in the state that directory is in. They are not drawn:
    /// the file toggle starts off (see [`App::toggle_files`]), so a freshly
    /// built app draws the nodes and nothing else. Reading the listing once,
    /// here, is what lets the toggle be a filter later rather than a second
    /// visit to a tree the app does not keep.
    ///
    /// Each row is told whether a `.warlockignore` keeps its content out, from
    /// the node's own flag, and the file rows are told the same as the directory
    /// listing them — the rules exclude a directory's content along with it, so
    /// a file row saying otherwise would be a second answer to one question. It
    /// changes no colour and hides no row: an excluded directory keeps its row
    /// and its gray, and the flag is there for [`App::toggle_pact`] to refuse a
    /// press on it without asking the filesystem anything.
    #[must_use]
    pub fn from_tree(tree: &Tree) -> Self {
        let mut rows = Vec::new();
        for (node, depth) in tree.walk() {
            rows.push(
                Row::new(depth, node.path.clone(), node.document.clone(), node.state)
                    .with_child_count(node.children.len())
                    .with_ignored(node.is_ignored()),
            );
            rows.extend(node.files.iter().map(|file| {
                Row::file(depth + 1, file.clone(), node.state).with_ignored(node.is_ignored())
            }));
        }
        Self::from_rows(rows).with_counts(tree.counts())
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
    /// has nothing to say about one. See [`App::message`]. Nor is a pact in
    /// flight — nobody has started one — see [`App::set_pact_in_flight`] — and
    /// so nothing has been refused for one running either, see
    /// [`App::set_pact_refused`].
    ///
    /// And there is no account: no pact has run this session, so the panel has
    /// nothing whatever to draw, which is a different state from a run that has
    /// started and done nothing yet. See [`App::start_account`]. Its window
    /// starts the way the tree's does — no height, no offset — since nothing has
    /// been drawn.
    ///
    /// The tree has the focus, so the movement keys move its selection from the
    /// first keystroke on. See [`App::focus`].
    ///
    /// The mouse is not captured, which is what is true of an app that is on no
    /// terminal: whoever puts one under it says so, every frame, and until they
    /// do the footer names the `m` key by what it does on a terminal reporting
    /// nothing. See [`App::set_mouse_captured`].
    ///
    /// Nothing is collapsed and the pacted-only filter is off, so the rows
    /// handed over are exactly the rows drawn — unless some of them are file
    /// rows, which the file toggle starts off over. They are kept a second time
    /// as the unfiltered list a later collapse re-filters and a later expand
    /// restores from; a caller that wants collapsing to hide anything has to
    /// say which rows have children, with [`Row::with_child_count`], because a
    /// bare list of rows is the one input here that does not come from a tree.
    #[must_use]
    pub fn from_rows(rows: Vec<Row>) -> Self {
        let mut app = Self {
            rows: rows.clone(),
            all_rows: rows,
            collapsed: BTreeSet::new(),
            collapsible: BTreeSet::new(),
            pacted_only: false,
            show_files: false,
            selected: 0,
            scroll_offset: 0,
            viewport_height: 0,
            counts: StateCounts::default(),
            header: String::new(),
            message: None,
            in_flight: None,
            summarising: None,
            pact_refused: false,
            focus: Focus::Tree,
            account: None,
            panel_height: 0,
            panel_offset: 0,
            panel_follows: false,
            mouse_captured: false,
        };
        // The rows handed over may hold file rows, which the file toggle starts
        // off over, so the drawn list is derived rather than assumed even here.
        app.reflow();
        app
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
    /// module the same way on every platform.
    ///
    /// A tree rooted at `repo_root` itself gets no header at all. The line is
    /// there to say *which part* of the repository is on screen, which is a
    /// thing worth saying only when it is not the whole of it: at the root there
    /// is no part, the relative spelling would be a bare `"."`, and any wording
    /// for it says out loud what the root row underneath already shows. So the
    /// line goes blank and keeps its row, because a header that appeared and
    /// disappeared would move every tree row up and down with it.
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
            Ok(relative) if relative == "." => String::new(),
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
    /// Set by whatever refused to do something — [`App::toggle_pact`] on a file
    /// row — or by something that did it and had news about it, or by the
    /// caller through [`App::set_message`], and emptied by the next movement, so
    /// what is here always belongs to the keystroke just pressed.
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

    /// Say that a pact is working the directory at `path`, which is directory
    /// `position` of `total`.
    ///
    /// The app runs no pact and can see none: a subtree pact happens on another
    /// thread, over minutes, and the only thing here that could know how it is
    /// going is whoever started it. So this is the caller's to set as the run
    /// advances — once per directory, with the same `total` throughout — and the
    /// caller's to take away with [`App::clear_pact_in_flight`] when the run
    /// ends, however it ends.
    ///
    /// What lands on screen is [`App::pact_line`]; `position` counts from one,
    /// because the line is read by a person rather than indexed.
    ///
    /// Not a keystroke, so it says nothing and takes nothing down: the message
    /// the last keystroke left is still the last keystroke's, and is still there
    /// when the pact is over. Movement does not undo this either — a pact
    /// carries on being in flight however much the reader scrolls.
    ///
    /// Takes down whatever [`App::set_pact_summarising`] last said: the file
    /// being summarised belonged to the directory the run has just left, so a
    /// new directory never inherits the last one's file.
    pub fn set_pact_in_flight(&mut self, path: impl Into<PathBuf>, position: usize, total: usize) {
        self.set_run_in_flight(Run::Pact, path, position, total);
    }

    /// Say that the run of kind `run` is working the directory at `path`, which
    /// is directory `position` of `total`.
    ///
    /// [`App::set_pact_in_flight`] with the kind said out loud, for the caller
    /// driving a refresh — which is the same run reported the same way, over the
    /// same channel, in the same account, with one word of the footer different:
    /// see [`Run`]. Everything the doc comment on [`App::set_pact_in_flight`]
    /// says holds here, kind and all, because that method is this one with
    /// [`Run::Pact`] filled in.
    ///
    /// The kind rides on the same single in-flight record as the directory and
    /// the fraction, so a run cannot end up half pact and half refresh, and
    /// [`App::clear_pact_in_flight`] takes the kind away with everything else.
    pub fn set_run_in_flight(
        &mut self,
        run: Run,
        path: impl Into<PathBuf>,
        position: usize,
        total: usize,
    ) {
        self.in_flight = Some(InFlight {
            path: path.into(),
            position,
            total,
            run,
        });
        self.summarising = None;
    }

    /// Say that a summarising pass over the file at `path` is running — pass
    /// `part` of `parts` — inside the directory the pact is working.
    ///
    /// A file too big for one request is read in chunks and summarised a chunk at
    /// a time, so one directory of the run can be a dozen model passes over a
    /// single file. Without this the footer would sit unchanged for the whole of
    /// it, which is the one thing the progress line exists to prevent; with it,
    /// the line names the file and how far through it the run is. `part` counts
    /// from one and `parts` is how many passes that file costs — the engine's own
    /// counting, which a caller passes straight through rather than deriving.
    ///
    /// The caller's to set, as [`App::set_pact_in_flight`] is and for the same
    /// reason: the passes happen on another thread. It is not the caller's to
    /// take away one by one, though — moving the run on with
    /// [`App::set_pact_in_flight`] clears it, and so does
    /// [`App::clear_pact_in_flight`], so there is no way for a file to outlive
    /// the directory it was found in.
    ///
    /// Says nothing on its own: it adds a clause to [`App::pact_line`], which
    /// exists only while a pact is in flight. Not a keystroke, so it says nothing
    /// and takes nothing down.
    pub fn set_pact_summarising(&mut self, path: impl Into<PathBuf>, part: usize, parts: usize) {
        self.summarising = Some(Summarising {
            path: path.into(),
            part,
            parts,
        });
    }

    /// Say that no pact is running any more.
    ///
    /// The other half of [`App::set_pact_in_flight`], for the end of a run
    /// whether it finished, failed or was cancelled: the line describes work
    /// happening now, so it has to go when the work stops, and only the caller
    /// knows that it has. Leaves the message alone, so whatever the caller says
    /// about how the run went is on screen the moment the progress line is off
    /// it.
    ///
    /// Takes the summarising pass down with it — see
    /// [`App::set_pact_summarising`] — so no chunk wording survives the end of a
    /// run.
    ///
    /// A no-op when no pact was in flight.
    pub fn clear_pact_in_flight(&mut self) {
        self.in_flight = None;
        self.summarising = None;
    }

    /// Say that the pact key was pressed while a pact was already running, so
    /// that the press is answered rather than swallowed.
    ///
    /// The caller refuses the press — nothing starts, nothing toggles — and
    /// says here that it did. Deliberately not [`App::set_message`]: the
    /// message line is the very thing a pact in flight has taken (see
    /// [`App::pact_line`]), so a sentence left there would be the one sentence
    /// the reader could not see, and it would then turn up minutes later when
    /// the run ended, long after the key that earned it. Deliberately not a
    /// fourth footer line either — the footer is three lines at fixed heights.
    /// What it does instead is re-word the line the reader is already watching,
    /// as a suffix on [`App::pact_line`].
    ///
    /// A flag and not a count: a second, third or fourth press says the same
    /// thing, so setting this again changes nothing. It says nothing at all
    /// while no pact is in flight, since there is no line for it to be a suffix
    /// on and no press it could have refused.
    ///
    /// This one *is* a keystroke, so it goes the way a message goes: the next
    /// keystroke takes it down. Progress events do not — a tick landing a
    /// fraction of a second after the press would otherwise wipe the answer
    /// before it was read.
    pub fn set_pact_refused(&mut self) {
        self.pact_refused = true;
    }

    /// Whether a pact is running now, as last set by
    /// [`App::set_pact_in_flight`].
    ///
    /// For a renderer deciding which keys to advertise, and for a key handler
    /// deciding what Esc means. It is display state and nothing more: it is
    /// whatever the caller last said, not something the app went and checked.
    #[must_use]
    pub const fn is_pacting(&self) -> bool {
        self.in_flight.is_some()
    }

    /// Whether `path` is the directory the pact is working now.
    ///
    /// The narrow question a renderer asks: given a row it is about to draw,
    /// is this the one the run is inside? Answering it here rather than handing
    /// the path out keeps [`InFlight`] private, and keeps the comparison one
    /// exact path against one exact path — no ancestors, no descendants, and
    /// nothing about the file rows beneath the directory.
    ///
    /// `false` when no pact is running, so a caller needs no separate
    /// [`App::is_pacting`] check.
    #[must_use]
    pub fn is_in_flight(&self, path: &Path) -> bool {
        self.in_flight
            .as_ref()
            .is_some_and(|in_flight| in_flight.path == path)
    }

    /// Whether the pass in flight covers `row`: the directory being worked, or
    /// a file that directory holds.
    ///
    /// The pulse's question, wider than [`App::is_in_flight`] by exactly the
    /// file rows. A directory's pass reads the files directly inside it, so
    /// while it runs those rows are the work on screen and flash with it. Its
    /// child *directories* are not covered — each has a pass of its own,
    /// already finished by the time the parent's runs, and a row that is done
    /// has no business flashing — and nor are files deeper down, which belong
    /// to those passes. Ancestors and siblings are as untouched as ever.
    ///
    /// `false` when no pact is running, exactly as [`App::is_in_flight`] is.
    #[must_use]
    pub fn in_flight_covers(&self, row: &Row) -> bool {
        self.in_flight.as_ref().is_some_and(|in_flight| {
            in_flight.path == row.path
                || (row.is_file() && row.path.parent() == Some(in_flight.path.as_path()))
        })
    }

    /// The line describing the run in flight — `pacting crates/engine (3/12)`,
    /// or `refreshing crates/engine (3/7)` for a refresh — or `None` when no run
    /// is running.
    ///
    /// The verb is the one the caller said the run was started by — see [`Run`]
    /// and [`App::set_run_in_flight`] — and it is the only difference between the
    /// two lines: the directory, the fraction, the summarising clause and the
    /// refusal suffix are worded the same way for both, because a refresh is a
    /// run in flight in every way that matters here.
    ///
    /// The directory is named relative to the root of the tree on screen, in the
    /// engine's own manifest spelling, for the reason every other label here is:
    /// an absolute path spends the footer on the part the reader already knows.
    /// It is worded here rather than by the caller so that the app is the one
    /// place a footer line is decided, and spelled at draw time rather than when
    /// it was set so that it is spelled against the tree that is on screen now.
    ///
    /// This takes the message line when there is one to take: while a pact runs,
    /// what is happening to the reader's repository right now outranks a
    /// sentence about a keystroke, and the run is the thing Esc is about to act
    /// on. The message underneath is not thrown away — [`App::message`] still
    /// holds it, and it appears when the run ends — which is what makes the
    /// precedence a display rule rather than a loss of state.
    ///
    /// While a summarising pass is running inside that directory the line says so
    /// too — `pacting crates/engine (3/12) — summarising Cargo.lock (2/5)` — so
    /// the minutes a single big file costs are minutes of a fraction advancing
    /// rather than of a screen that has not changed: see
    /// [`App::set_pact_summarising`]. Still one line, and the directory and its
    /// fraction still come first, because they are what the rest of the run is
    /// measured in. The file is named relative to the root of the tree at draw
    /// time exactly as the directory is.
    ///
    /// A press of the pact key refused because this run is already going adds
    /// `— already running` to the end of it, rather than taking a line of its
    /// own: see [`App::set_pact_refused`]. It is a suffix and goes last — after
    /// the summarising clause as well — so that a narrow terminal cuts the answer
    /// to a key just pressed rather than the fraction, which is the part that
    /// says Warlock has not hung. The line is rebuilt every frame, so a progress
    /// event arriving after the press re-words it around the new directory and
    /// position and carries the suffix along.
    #[must_use]
    pub fn pact_line(&self) -> Option<String> {
        self.in_flight.as_ref().map(|in_flight| {
            let label = self.label_for(&in_flight.path);
            let mut line = match in_flight.run {
                Run::Pact => pacting_message(&label, in_flight.position, in_flight.total),
                Run::Refresh => refreshing_message(&label, in_flight.position, in_flight.total),
            };
            if let Some(summarising) = self.summarising.as_ref() {
                line = summarising_message(
                    &line,
                    &self.label_for(&summarising.path),
                    summarising.part,
                    summarising.parts,
                );
            }
            if self.pact_refused {
                already_running_message(&line)
            } else {
                line
            }
        })
    }

    /// Begin the account of a pact starting at `at`, throwing away whatever the
    /// last one left.
    ///
    /// One pact, one account. A second run does not append to the first: the
    /// panel is a record of what is happening now, and a reader who has to
    /// scroll past a finished run to find the live one has been handed a log
    /// rather than a report. So this clears the sections, the lines and the
    /// summary of the previous run outright, and puts the window back at the top
    /// following the newest line — which for an account with nothing in it is
    /// the same place.
    ///
    /// This is also the only way an app comes to have an account at all: before
    /// the first call there is none, and the panel draws nothing whatever rather
    /// than an empty frame around a run that has not happened. See
    /// [`App::has_account`].
    ///
    /// Not a keystroke — the pact key reaches this by way of whoever starts the
    /// run — so it neither says anything nor takes down what the last keystroke
    /// said.
    pub fn start_account(&mut self, at: Instant) {
        self.account = Some(Account::new(at));
        self.panel_offset = 0;
        self.panel_follows = true;
    }

    /// Whether a pact has run this session, and so whether the panel has
    /// anything at all to draw.
    ///
    /// `false` until [`App::start_account`], and `true` for ever after: an
    /// account that has finished is still an account, and it stays on screen to
    /// be read once the run that made it is over.
    #[must_use]
    pub const fn has_account(&self) -> bool {
        self.account.is_some()
    }

    /// The account of the pact running now, or of the last one to run, or `None`
    /// before the first pact of the session.
    #[must_use]
    pub const fn account(&self) -> Option<&Account> {
        self.account.as_ref()
    }

    /// The same account, to record what the run has just been seen doing.
    ///
    /// The account words its own lines — see [`Account`] — so this hands it over
    /// rather than wrapping each of its five mutators in a method here that
    /// would only forward. Nothing needs doing afterwards: while the panel is
    /// following, its window is the end of the account computed on demand, so a
    /// line recorded through here is on screen at the bottom of the next frame
    /// without this type being told about it.
    ///
    /// `None` before the first pact, where there is nothing to record against.
    pub const fn account_mut(&mut self) -> Option<&mut Account> {
        self.account.as_mut()
    }

    /// Take `view`'s account and the panel's window over from it, leaving `view`
    /// with none.
    ///
    /// For the one caller that puts an older copy of the app back in place of
    /// the live one: a run that ended with nothing recorded restores the rows,
    /// the colours and the selection to what the manifest on disk still says,
    /// and that copy was taken before the run started, so it knows nothing of
    /// what the run then did. The account is not a claim about the tree and has
    /// no business being rolled back with it — it is the record of a run that
    /// really happened, and the run that ends this way is exactly the one a
    /// reader most wants to see the end of.
    ///
    /// Moved rather than copied because the app it comes from is on its way out;
    /// an account is every line of a run that may have taken minutes, and there
    /// is no reason to duplicate it on the way past.
    pub fn take_account_from(&mut self, view: &mut Self) {
        self.account = view.account.take();
        self.panel_height = view.panel_height;
        self.panel_offset = view.panel_offset;
        self.panel_follows = view.panel_follows;
    }

    /// How many lines of account fit in the panel, as last set by
    /// [`App::set_panel_height`].
    #[must_use]
    pub const fn panel_height(&self) -> usize {
        self.panel_height
    }

    /// Tell the app how many lines of account fit in the panel.
    ///
    /// The panel's [`App::set_viewport_height`], and a field for the same reason:
    /// only the layout knows the height, only the frame knows when it changed,
    /// and a height passed per call would let two callers disagree about the size
    /// of one window. Safe to call every frame.
    ///
    /// Nothing has to be brought back into line afterwards. The offset is
    /// clamped when it is read, and a window that was following is still
    /// following, so a terminal that has just been made shorter or taller still
    /// shows the newest line at the bottom.
    pub fn set_panel_height(&mut self, height: u16) {
        self.panel_height = usize::from(height);
    }

    /// Whether the panel is following the newest line of the account.
    ///
    /// `true` while the window sits at the end, which is where a fresh account
    /// starts and where the end-of-list movement key puts it back. `false` from
    /// the moment the reader scrolls up, until they scroll back down to the end
    /// or ask for it outright. See [`App::select_last`].
    #[must_use]
    pub const fn panel_follows(&self) -> bool {
        self.panel_follows
    }

    /// Which line of the account is drawn at the top of the panel.
    ///
    /// Derived rather than stored while the panel is following: the answer is
    /// then the end of the account, which changes every time a line is
    /// appended, and computing it here is what keeps the newest line pinned to
    /// the bottom without anything having to be recomputed as the run reports.
    /// Parked, it is where the reader left the window, clamped to what the
    /// account's length allows.
    ///
    /// `0` for an app with no account, and for a panel shorter than the account
    /// it holds is the first line of the last screenful.
    #[must_use]
    pub fn panel_scroll_offset(&self) -> usize {
        panel_offset_for(
            self.account_line_count(),
            self.panel_height,
            self.panel_offset,
            self.panel_follows,
        )
    }

    /// The lines the panel draws now, with every clock measured against `now`.
    ///
    /// The window [`App::panel_scroll_offset`] describes, [`App::panel_height`]
    /// lines of it, and empty for an app with no account. `now` is the caller's,
    /// because the newest line of a live section counts up between events and the
    /// only thing that knows what the time is when a frame is drawn is whoever is
    /// drawing it.
    #[must_use]
    pub fn panel_lines(&self, now: Instant) -> Vec<Line> {
        self.account.as_ref().map_or_else(Vec::new, |account| {
            account.window(self.panel_scroll_offset(), self.panel_height, now)
        })
    }

    /// How many lines of the account sit below the panel's window.
    ///
    /// `0` while the panel is showing the end of the account, which is what an
    /// indicator saying how far back the reader has scrolled is switched off by,
    /// and `0` for an app with no account at all. Independent of `now`: the
    /// clocks move, the number of lines does not.
    ///
    /// A panel nobody has measured draws nothing, so the whole account is below
    /// it — which no frame ever sees, since a frame that asks this has just told
    /// the app how tall the panel it is drawing is.
    #[must_use]
    pub fn panel_lines_below(&self) -> usize {
        self.account_line_count()
            .saturating_sub(self.panel_scroll_offset() + self.panel_height)
    }

    /// How many lines the account draws as, or none where there is no account.
    fn account_line_count(&self) -> usize {
        self.account.as_ref().map_or(0, Account::line_count)
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
    /// Answers for a node holding nothing too, where it means only that the path
    /// is in the set: there is nothing to hide, so nothing is hidden. A renderer
    /// deciding which marker to draw wants this *and* [`App::can_collapse`] —
    /// collapsed, expanded-over-something and holding-nothing are three cases,
    /// and this answers one bit of them.
    #[must_use]
    pub fn is_collapsed(&self, path: impl AsRef<Path>) -> bool {
        self.collapsed.contains(path.as_ref())
    }

    /// Whether the row at `index` in [`App::rows`] has anything under it in the
    /// view as it stands, and so whether collapsing it would hide something.
    ///
    /// This is the question the collapse key asks, and it is a question about
    /// the view rather than about the tree. A directory holding nothing but
    /// files has no children in the tree and has rows under it whenever the file
    /// toggle is on; a directory whose children the pacted-only filter has taken
    /// away has children in the tree and nothing under it on screen. Asking
    /// [`Row::children`] instead gets both of those wrong, in opposite
    /// directions — the first as a key that silently does nothing, the second as
    /// a marker promising something to unfold.
    ///
    /// Answered from the set `reflow` derived beside the rows, so it costs a
    /// lookup and it is the same answer the drawn rows were filtered with. An
    /// `index` past the end is not a row and holds nothing.
    #[must_use]
    pub fn can_collapse(&self, index: usize) -> bool {
        self.rows
            .get(index)
            .is_some_and(|row| self.collapsible.contains(&row.path))
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
        self.forget_last_keystroke();
        self.reflow();
    }

    /// Whether the files inside each directory are drawn as well as the
    /// directories themselves.
    ///
    /// `false` for a freshly built app: the tree is a tree of modules, and the
    /// files are detail asked for by [`App::toggle_files`] rather than the
    /// first thing a reader is shown.
    #[must_use]
    pub const fn show_files(&self) -> bool {
        self.show_files
    }

    /// Show the files inside each directory, or hide them again.
    ///
    /// Shown, every file a node listed gets a row directly under the row for
    /// the directory holding it, one level deeper, in that directory's state
    /// and so in that module's colour. The order is the walk's: the files of a
    /// directory come in the order the tree listed them, before the rows for
    /// that directory's subdirectories.
    ///
    /// A file row is drawn and nothing else. It is no module, so
    /// [`App::toggle_pact`] refuses it; it contains nothing, so collapsing it
    /// hides nothing; and it is no node, so [`App::counts`] does not move by a
    /// single one when this is toggled either way — the footer counts modules,
    /// and a module has the same files whether or not they are on screen.
    ///
    /// It composes with the other two view flags the way they compose with each
    /// other, and in that order: the file rows are filtered out first, so with
    /// files hidden the pacted-only pass and the collapsed set see exactly the
    /// list of nodes they saw before files existed. With files shown, a file
    /// under a collapsed directory is hidden with it, and — because a file row
    /// carries its directory's state — a file under a pacted directory survives
    /// the pacted-only filter while one under a directory that is merely on the
    /// way to something pacted does not.
    ///
    /// Clears the last keystroke's message, like every other key that does
    /// something.
    pub fn toggle_files(&mut self) {
        self.show_files = !self.show_files;
        self.forget_last_keystroke();
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

    /// Whether the terminal is reporting its mouse, as last set by
    /// [`App::set_mouse_captured`].
    ///
    /// For the footer and nobody else: the keys line names the `m` key by what
    /// the next press of it will do, which is the one thing on screen that
    /// depends on this. Nothing in here gates on it — while capture is off the
    /// pointer's events never arrive, so there is no second door to lock.
    #[must_use]
    pub const fn mouse_captured(&self) -> bool {
        self.mouse_captured
    }

    /// Tell the app whether the terminal is reporting its mouse.
    ///
    /// A mirror rather than a switch: turning capture on and off is the
    /// binary's, because it is a sequence written to a terminal this type cannot
    /// see, and the app is told what was done so the footer can say it. Safe to
    /// call every frame, and meant to be — told the same way the two window
    /// heights are, from the one place that knows, so a copy of the app taken
    /// before a pact and put back after one cannot leave the keys line naming a
    /// state the terminal is no longer in.
    pub const fn set_mouse_captured(&mut self, captured: bool) {
        self.mouse_captured = captured;
    }

    /// Which pane the keys are driving: the tree column, or the panel beside
    /// it.
    ///
    /// [`Focus::Tree`] for a freshly built app, which is the pane warlock opens
    /// on. For the renderer, deciding which border to light, and for the
    /// movement methods below, deciding whether they mean anything.
    #[must_use]
    pub const fn focus(&self) -> Focus {
        self.focus
    }

    /// Move the focus to the other pane.
    ///
    /// The whole of what the focus key does. It is a toggle rather than a pair
    /// of "focus the tree" / "focus the panel" calls because there are two panes
    /// and one key, and a toggle cannot be asked for a third.
    ///
    /// Nothing else moves: the selection stays on the row it was on, the window
    /// stays where it was, and the last keystroke's message stays up. Focus
    /// changes what the *next* key means and says nothing itself, so there is
    /// nothing here for a message to report and nothing that would make a
    /// message stale.
    pub const fn toggle_focus(&mut self) {
        self.focus = self.focus.other();
    }

    /// Put the focus on `focus`, wherever it was.
    ///
    /// What a pointer can ask for and a key cannot. The focus key knows only
    /// "the other one", which is the whole of what one key over two panes can
    /// mean; a click names the pane it landed in, and naming the pane that
    /// already has the focus has to leave it there rather than toggle away from
    /// it — which is exactly what assigning the value it already holds does.
    ///
    /// Nothing else moves, for the reason [`App::toggle_focus`] moves nothing
    /// else: focus changes what the *next* movement means and says nothing
    /// itself, so the selection, both windows and the last keystroke's message
    /// are none of its business.
    pub const fn set_focus(&mut self, focus: Focus) {
        self.focus = focus;
    }

    /// Move up one line: the selection while the tree has the focus, the
    /// panel's window while the panel has it.
    ///
    /// It clamps rather than wrapping: an unnoticed wrap at the top of a long
    /// tree throws the reader to the bottom of it, and the arrow key is for
    /// stepping, not teleporting. A no-op when there are no rows, or — at the
    /// panel — when the window is already at the top of the account.
    pub fn select_previous(&mut self) {
        self.movement(
            |app| app.selected.saturating_sub(1),
            |_, offset| offset.saturating_sub(1),
        );
    }

    /// Move down one line: the selection at the tree, the panel's window at the
    /// panel.
    ///
    /// Clamps for the same reason [`App::select_previous`] does. A no-op when
    /// there are no rows, or when the panel's window is already at the end —
    /// where, being at the end, it goes back to following the newest line.
    pub fn select_next(&mut self) {
        self.movement(
            |app| {
                let last = app.rows.len().saturating_sub(1);
                app.selected.saturating_add(1).min(last)
            },
            |_, offset| offset.saturating_add(1),
        );
    }

    /// Move one screenful up: the selection at the tree, the panel's window at
    /// the panel.
    ///
    /// A screenful is the focused pane's own height — [`App::viewport_height`]
    /// or [`App::panel_height`] — so the row that was at the top of the window is
    /// roughly the one at the bottom afterwards: paging by the window's own
    /// height is what makes reading a long list a sequence of screens rather
    /// than a slide.
    ///
    /// A pane that has never been drawn has no height to page by, and a key
    /// that does nothing at all reads as a broken key, so the step never falls
    /// below one line.
    pub fn select_page_up(&mut self) {
        self.movement(
            |app| app.selected.saturating_sub(app.page()),
            |app, offset| offset.saturating_sub(app.panel_page()),
        );
    }

    /// Move one screenful down: the selection at the tree, the panel's window at
    /// the panel.
    ///
    /// The mirror of [`App::select_page_up`], down to the one-line floor for a
    /// pane that has not been drawn.
    pub fn select_page_down(&mut self) {
        self.movement(
            |app| {
                let last = app.rows.len().saturating_sub(1);
                app.selected.saturating_add(app.page()).min(last)
            },
            |app, offset| offset.saturating_add(app.panel_page()),
        );
    }

    /// Go to the beginning: the first row of the tree, or the first line of the
    /// account.
    ///
    /// At the panel this is the start of the run, which for an account of any
    /// length means the window is no longer following the newest line.
    pub fn select_first(&mut self) {
        self.movement(|_| 0, |_, _| 0);
    }

    /// Go to the end: the last row of the tree, or the newest line of the
    /// account.
    ///
    /// At the panel this is what returns a scrolled-back reader to live. The
    /// window goes to the end of the account *and* starts following again, so
    /// the lines that arrive afterwards go on moving it — which is the whole
    /// difference between the end of the account and the end of the account as
    /// it stood when the key was pressed. It is the movement key the pane
    /// already has rather than a key of its own: "go to the end" and "follow the
    /// end" are the same instruction to a list that is still being written.
    pub fn select_last(&mut self) {
        self.movement(|app| app.rows.len().saturating_sub(1), |_, _| usize::MAX);
    }

    /// Select the row at `index` in [`App::rows`], wherever the focus is.
    ///
    /// What a click on a tree row asks for. A key names a direction and lets
    /// the app work out where that lands; a pointer names the row outright, and
    /// this is the only way to say so. It goes through the same path an
    /// ordinary movement key goes through all the same — the last keystroke's
    /// message comes down and the window is brought back into line with the
    /// selection — because a row reached by pointer is the same selection as a
    /// row reached by pressing `j` at it, and everything downstream should be
    /// unable to tell which one happened.
    ///
    /// An `index` no row stands for is refused outright rather than clamped to
    /// the nearest one: what counts rows here is a layout answering a screen
    /// point, and a point below the last row of a half-full tree is a point on
    /// nothing at all — not a roundabout way of asking for the last row. So this
    /// is a complete no-op there, message included, and on an app with no rows.
    ///
    /// The focus is neither read nor written. Which pane the keys are driving
    /// says nothing about which row a pointer landed on, and a caller that wants
    /// the click to move the focus as well says so with [`App::set_focus`].
    pub fn select_row(&mut self, index: usize) {
        if index >= self.rows.len() {
            return;
        }
        self.selected = index;
        self.moved();
    }

    /// Move the selection `rows` rows down the tree, wherever the focus is.
    ///
    /// Exactly where `rows` presses of [`App::select_next`] at a focused tree
    /// would leave it, clamped at the last row, reached by arithmetic rather
    /// than by a loop. The wheel's, and the reason it does not go through
    /// `movement` the way the keys do: the pointer is over the tree column, so
    /// the tree column is what moves, however the keys happen to be pointed at
    /// the time.
    ///
    /// It scrolls nothing on its own. The tree pane has no window of its own to
    /// scroll — the window is derived from the selection by `rescroll`, as it is
    /// for every key — so a notch of the wheel here is three rows of selection
    /// and the window comes along behind it.
    ///
    /// A no-op on an app with no rows, and clears the last keystroke's message
    /// exactly as the key it stands in for does.
    pub fn select_next_by(&mut self, rows: usize) {
        let last = self.rows.len().saturating_sub(1);
        self.selected = self.selected.saturating_add(rows).min(last);
        self.moved();
    }

    /// Move the selection `rows` rows up the tree, wherever the focus is.
    ///
    /// The mirror of [`App::select_next_by`], clamping at the first row: what
    /// `rows` presses of [`App::select_previous`] at a focused tree would leave
    /// behind.
    pub fn select_previous_by(&mut self, rows: usize) {
        self.selected = self.selected.saturating_sub(rows);
        self.moved();
    }

    /// Scroll the panel's window `lines` lines towards the newest line of the
    /// account, wherever the focus is.
    ///
    /// The panel's half of the same wheel, and focus-free for the same reason:
    /// the pointer is over the panel, so it is the panel that scrolls. Where it
    /// lands is `scroll_panel_to`'s to decide, which is what keeps one rule
    /// about the end of the account rather than two — the window stops at the
    /// end however many lines were asked for, and a window that has arrived at
    /// the end is following the newest line again, with nothing here having to
    /// mean "and go live" as well as what it already means.
    ///
    /// Says nothing and takes nothing down. Nothing in the tree column has
    /// moved, and the line explaining the last keystroke belongs to the tree —
    /// the same reading a movement key at the panel takes, see `movement`.
    pub fn scroll_panel_down(&mut self, lines: usize) {
        self.scroll_panel_to(self.panel_scroll_offset().saturating_add(lines));
    }

    /// Scroll the panel's window `lines` lines back towards the start of the
    /// account, wherever the focus is.
    ///
    /// The mirror of [`App::scroll_panel_down`], stopping at the first line. Any
    /// movement off the end stops the panel following the newest line, so the
    /// lines that arrive afterwards leave the window where the reader put it —
    /// which is the whole of what scrolling back through a live log is for.
    pub fn scroll_panel_up(&mut self, lines: usize) {
        self.scroll_panel_to(self.panel_scroll_offset().saturating_sub(lines));
    }

    /// Carry out a movement key: `tree` says where the selection lands, `panel`
    /// says where the panel's window lands, and the focus decides which of them
    /// is asked.
    ///
    /// Every movement method goes through here, so the rule that a movement key
    /// drives whichever pane has the focus is written once rather than six times
    /// — and so a seventh movement method cannot be added without it. Each method
    /// hands over where to land as a function of the app, because that is the
    /// only part of a movement that differs between them; the clamping is
    /// theirs, since what "one row up" clamps to is not what "one screenful
    /// down" clamps to. The panel's is handed the offset as well as the app,
    /// because the panel has no selection to work from and its window is
    /// computed rather than stored.
    ///
    /// A movement at the tree clears the last keystroke's message, as every key
    /// that does something does. A movement at the panel does not: nothing in
    /// the tree column has moved, the reader is looking somewhere else entirely,
    /// and sweeping away the line explaining what the last key did would be the
    /// panel answering for the tree.
    fn movement(
        &mut self,
        tree: impl FnOnce(&Self) -> usize,
        panel: impl FnOnce(&Self, usize) -> usize,
    ) {
        if self.focus.drives_the_tree() {
            self.selected = tree(self);
            self.moved();
        } else {
            let offset = panel(self, self.panel_scroll_offset());
            self.scroll_panel_to(offset);
        }
    }

    /// Park the panel's window at `offset`, or as near to it as the account
    /// allows, and say whether that is still following.
    ///
    /// The one place the follow flag is decided by a keystroke, and it is
    /// decided by where the window ended up rather than by which key was
    /// pressed: following the newest line *is* sitting at the end of the
    /// account, so scrolling up breaks it and scrolling back down restores it,
    /// with no key having to mean "and start following again" as well as what it
    /// already means.
    fn scroll_panel_to(&mut self, offset: usize) {
        // Where the end is, asked of the one function that decides it, so that
        // "as far down as the account goes" means the same thing to a keystroke
        // as it does to the frame being drawn.
        let end = panel_offset_for(self.account_line_count(), self.panel_height, 0, true);
        self.panel_offset = offset.min(end);
        self.panel_follows = self.panel_offset == end;
    }

    /// How many rows one page key moves the tree by: a windowful, or a single
    /// row for an app whose window nobody has measured yet.
    const fn page(&self) -> usize {
        if self.viewport_height == 0 {
            1
        } else {
            self.viewport_height
        }
    }

    /// How many lines one page key moves the panel by, on the same rule.
    const fn panel_page(&self) -> usize {
        if self.panel_height == 0 {
            1
        } else {
            self.panel_height
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

    /// Rebuild the drawn rows from the whole walk, the file and pacted-only
    /// flags and the collapsed set, and put the selection and the window back
    /// where they belong.
    ///
    /// Every change to what is drawn ends here, and it is the only place
    /// `rows` is written: the drawn list is derived from `all_rows`,
    /// `show_files`, `pacted_only` and `collapsed` and nothing else, so there is
    /// no state to get out of step with them.
    ///
    /// The three filters are applied in that order and never as one pass. Files
    /// go first, so that with the toggle off the two passes below it see the
    /// list of nodes and only that: both of them reason by depth, and a file row
    /// sits deeper than the directory holding it, so a hidden file left in the
    /// list would be a row those passes had to reason around for no reason.
    /// Then pactedness: what is pacted, and what is on the way to something
    /// pacted, is read off the whole walk, so a directory that is collapsed over
    /// the only pacted node below it still earns its row; deciding pactedness
    /// from the already-collapsed list would make collapsing a directory delete
    /// it from a narrowed view, which is the one thing collapsing must never do.
    ///
    /// The selection is carried by path rather than by index, because the index
    /// it sat at meant a row that may not exist any more. A selection whose
    /// node is still drawn stays exactly where it is, however many rows above
    /// it have gone; one whose node has just been hidden — collapsed over or
    /// filtered away, it makes no difference here — lands on the nearest
    /// ancestor still drawn, and on the first row when not even that survives.
    fn reflow(&mut self) {
        let selected = self.rows.get(self.selected).map(|row| row.path.clone());
        let kept: Cow<'_, [Row]> = if self.show_files {
            Cow::Borrowed(&self.all_rows)
        } else {
            Cow::Owned(node_rows(&self.all_rows))
        };
        let kept: Cow<'_, [Row]> = if self.pacted_only {
            Cow::Owned(pacted_rows(&kept))
        } else {
            kept
        };
        (self.rows, self.collapsible) = drawn_rows(&kept, &self.collapsed);
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
        self.forget_last_keystroke();
        self.rescroll();
    }

    /// A keystroke has just done something: forget what the one before it left
    /// on the footer, whether that was a message or a refusal.
    ///
    /// The two go together because they are one thing said two ways — what the
    /// app has to say about the key just pressed — and a reader who has moved on
    /// to the next key is owed neither. Which is why the refusal recorded by
    /// [`App::set_pact_refused`] is taken down here rather than by the next
    /// progress event: the run advancing is not a keystroke, and a tick arriving
    /// a fraction of a second after the press would wipe the answer before it
    /// could be read.
    fn forget_last_keystroke(&mut self) {
        self.message = None;
        self.pact_refused = false;
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
    /// A no-op, and deliberately a *complete* no-op, on a row with nothing under
    /// it in this view — see [`App::can_collapse`], which is the question, and
    /// not the tree's child count, which is not. Nothing to hide means nothing
    /// collapses, nothing is recorded, and last keystroke's message is left on
    /// screen rather than swept away by a key that did nothing. A no-op on an
    /// app with no rows too.
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
        if !self.can_collapse(self.selected) {
            return;
        }
        let Some(row) = self.rows.get(self.selected) else {
            return;
        };

        let path = row.path.clone();
        if !self.collapsed.remove(&path) {
            self.collapsed.insert(path);
        }
        self.forget_last_keystroke();
        self.reflow();
    }

    /// Bring the selected directory and everything below it under Warlock's
    /// management, or take the lot back out again, and say what was asked for.
    ///
    /// A pact covers a subtree, so both directions do: the selected directory's
    /// row, every row below it and every file row inside any of them move
    /// together, through [`App::set_subtree_state`].
    ///
    /// An unpacted subtree becomes [`NodeState::PactedStale`], never fresh: a
    /// pact with no granted hash was never judged, and unjudged *is* stale. The
    /// caller that runs the pact and lands the grants says so afterwards, with
    /// [`App::set_subtree_state`] again. A pacted subtree, stale or fresh,
    /// becomes [`NodeState::Unpacted`] — dropping a pact drops whatever was
    /// granted with it.
    ///
    /// A directory with no `WARLOCK.md` is pacted like any other: writing that
    /// document is what the pact operation *does*, so having none yet is the
    /// ordinary case rather than a reason to refuse. Two rows are refused, and
    /// they are the only ones that are:
    ///
    /// - a file row, because a pact is made with a module, and a file is part of
    ///   one rather than being one;
    /// - a row the repository's `.warlockignore` keeps out, because the pact
    ///   walks would find nothing there to describe — the engine excludes such a
    ///   directory from `pactable_directories` whether or not this refuses — so a
    ///   press that went through would paint a subtree that no run would ever
    ///   make good on. The rules are the repository author's, so the refusal
    ///   names the file rather than Warlock's own judgement.
    ///
    /// So `None` comes back for an app with no rows, for a selected file and for
    /// excluded content, and nothing moves in any of the three cases.
    ///
    /// Being excluded is read off the row (see [`Row::is_ignored`]), which read
    /// it off the node the load put it on. Nothing here opens anything: this
    /// method is pure, and a filesystem call in it would be a filesystem under
    /// every test of it.
    ///
    /// A refusal sets [`App::message`] to say so, and the return value stays a
    /// bare `Option`: the wording is display state, it belongs here with the
    /// rest of the display state, and a caller that had to translate an outcome
    /// into a sentence would be a second place deciding what a refused row
    /// means.
    /// Un-pacting sets a message of its own — the documents stay on disk, and a
    /// subtree that has just gone grey should say that the writing survived it.
    /// Pacting clears the message instead: whatever the last keystroke said,
    /// this one did something.
    ///
    /// Writing documents and saving the manifest are the caller's job. This is
    /// app state and touches no file.
    pub fn toggle_pact(&mut self) -> Option<PactToggle> {
        let row = self.rows.get(self.selected)?;
        let path = row.path.clone();
        let ignored = row.is_ignored();
        if row.is_file() {
            // Answered as a file first: that is what the row *is*, and it is the
            // answer whether or not the rules also keep it out.
            self.message = Some(file_row_message(&self.label_for(&path)));
            return None;
        }
        if ignored {
            self.message = Some(ignored_row_message(&self.label_for(&path)));
            return None;
        }

        let pacted = !row.state.is_pacted();
        self.set_subtree_state(
            &path,
            if pacted {
                NodeState::PactedStale
            } else {
                NodeState::Unpacted
            },
        );
        self.message = (!pacted).then(|| left_on_disk_message(&self.label_for(&path)));

        Some(PactToggle { path, pacted })
    }

    /// Ask for the selected directory's stale parts to be described again, and
    /// say why not when there is nothing to ask for.
    ///
    /// The refresh key's half of what [`App::toggle_pact`] is for the pact key:
    /// it decides what the press means on the row the selection is on, words any
    /// refusal, and hands the directory back for whoever actually runs the pass.
    /// The path that comes back is the root of the subtree to refresh — the run
    /// covers it and everything below it, exactly as a pact does — and `None`
    /// means nothing should start.
    ///
    /// Three rows are refused, and each says so through [`App::message`]:
    ///
    /// - a file row, in [`App::toggle_pact`]'s own words, because the reason is
    ///   the same one — a file is part of a module rather than being one, so
    ///   there is no subtree here to describe;
    /// - a directory that is pacted and *fresh*, because a refresh describes the
    ///   stale directories of a subtree and this one has none, so the honest
    ///   answer is that the work is already done;
    /// - a directory that is not pacted, which points at the pact key: a refresh
    ///   re-describes an existing pact and cannot make one, so `p` is the key
    ///   that would help. A directory a `.warlockignore` keeps out reads as
    ///   unpacted here like any other, and is refused for that reason without
    ///   this having to know which reason made it so.
    ///
    /// A refusal moves nothing else whatever: no subtree is repainted, no tally
    /// moves, no account is started and no run is touched. A press that goes
    /// through clears the message instead — whatever the last keystroke said,
    /// this one did something — and the run's own line takes the footer from
    /// there.
    ///
    /// Whether a run is already in flight is not asked here. That refusal is
    /// worded on the progress line rather than in the message (see
    /// [`App::set_pact_refused`]), so it belongs to the caller that knows about
    /// the worker, exactly as it does for the pact key. Nothing here spawns
    /// anything, reads a file or starts an account either: this is app state,
    /// and the pass is somebody else's.
    pub fn refresh(&mut self) -> Option<PathBuf> {
        let row = self.rows.get(self.selected)?;
        let path = row.path.clone();
        let state = row.state;

        if row.is_file() {
            self.message = Some(file_row_message(&self.label_for(&path)));
            return None;
        }
        match state {
            NodeState::Unpacted => {
                self.message = Some(unpacted_message(&self.label_for(&path)));
                None
            }
            NodeState::PactedFresh => {
                self.message = Some(already_fresh_message(&self.label_for(&path)));
                None
            }
            NodeState::PactedStale => {
                self.message = None;
                Some(path)
            }
        }
    }

    /// Put the directory at `path`, every directory below it and every file
    /// inside any of them into `state`.
    ///
    /// This is how a whole subtree changes colour at once, and it is the only
    /// way any state moves after the tree was loaded: [`App::toggle_pact`] goes
    /// through it, and so does a caller with news about how a pact actually
    /// went — a subtree pacted and granted is [`NodeState::PactedFresh`], and
    /// only whoever ran it knows that.
    ///
    /// Both lists move together: the drawn rows, and the whole walk behind them
    /// that the next collapse or filter rebuilds those rows from. Writing only
    /// the drawn ones would make the new colour last exactly until something
    /// was collapsed and then quietly revert.
    ///
    /// The file rows move with the directories holding them. A file row carries
    /// a copy of its directory's state so that it can be drawn in its module's
    /// colour, and a copy nobody updates is a file drawn in the colour its
    /// module used to be.
    ///
    /// The tally moves too, one node out of each old state's field and into
    /// `state`, so [`App::counts`] keeps describing [`App::rows`] and
    /// [`StateCounts::total`] does not budge. Nothing recounts the rows: the
    /// counts are the engine's numbers, kept current rather than re-derived.
    /// Files are counted nowhere, and so move nothing.
    ///
    /// A `path` no row stands for changes nothing at all — a subtree that is
    /// not on screen has no colour to move — and neither does a `path` whose
    /// rows are in `state` already.
    ///
    /// Says nothing: this is not a keystroke, so it neither sets a message nor
    /// clears the one the last keystroke left. It does not re-filter the drawn
    /// rows either, which is what lets a subtree that has just been un-pacted
    /// stay on screen under the pacted-only filter until the next keystroke
    /// rebuilds the view.
    pub fn set_subtree_state(&mut self, path: impl AsRef<Path>, state: NodeState) {
        let path = path.as_ref();

        // The tally first, off the unfiltered list, so that every node is
        // counted once whether or not it is drawn — and before the states are
        // written, since what moves out of a field is what each node is now.
        for row in &self.all_rows {
            if row.is_file() || row.state == state || !in_subtree(&row.path, path) {
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

    /// Splice a row for the file at `path` in under the directory holding it,
    /// where a fresh load would have put it.
    ///
    /// A pact writes a `WARLOCK.md` beside each directory as its pass delivers,
    /// and the row for that document has to appear then rather than when the run
    /// ends. The tree the app was built from is not held and the tree on disk is
    /// mid-run — the manifest is still the pre-pact one — so the row cannot be
    /// re-derived: re-reading would repaint every row from stale state and wipe
    /// the colours the run has already earned. So the one row that is news is
    /// written in, and nothing else moves.
    ///
    /// It is written in at the place [`App::from_tree`] would have flattened it
    /// to: among that directory's file rows in path order — the order the
    /// engine's loader sorts a node's listing into — at one depth deeper than
    /// the directory, and so before the rows for any of its subdirectories. A
    /// mid-run tree and the reload that follows the run therefore agree.
    ///
    /// The row carries the directory's state *now*, and its `.warlockignore`
    /// flag, for the reason [`Row::file`] gives: a file is drawn in its module's
    /// colour, and the module's colour is whatever the run has just made it.
    ///
    /// Both lists move, the way [`App::set_subtree_state`] moves both: into the
    /// whole walk, so the row survives the next collapse or filter rebuild
    /// rather than vanishing, and into the drawn rows when the view is one this
    /// file is drawn in. It re-filters nothing, again like `set_subtree_state` —
    /// the drawn half is decided by asking what this view already shows, not by
    /// rebuilding it.
    ///
    /// Nothing else moves at all. A `path` already in the walk is inserted
    /// nowhere, so a re-pact of an already documented directory leaves exactly
    /// one row for it; a `path` whose directory has no row is not news about
    /// anything on screen and changes nothing. [`App::counts`] never moves,
    /// because files are counted nowhere. The selection keeps naming the row it
    /// named, and the window keeps its rows, both of which mean an index that
    /// shifts when a row lands above it. And no message is set or cleared: this
    /// is not a keystroke.
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
        let row = Row::file(depth, path, holder.state).with_ignored(holder.is_ignored());
        let at = file_row_position(&self.all_rows, index, depth, path);
        self.all_rows.insert(at, row.clone());

        // The drawn half, which is conditional: `rows` holds file rows only
        // while the toggle is on, holds a file under a pacted directory only
        // under the pacted-only filter, and holds nothing at all under a
        // directory that is collapsed or filtered away. Asking those three
        // questions is what keeps this insertion and the next `reflow` from
        // disagreeing about what is on screen.
        if !self.show_files
            || (self.pacted_only && !row.state.is_pacted())
            || self.collapsed.contains(directory)
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

/// Where the row for the file at `path` goes in `rows`, given that the
/// directory holding it has the row at `directory` and that its file rows sit at
/// `depth`.
///
/// The engine's loader sorts each node's listing (`directory.files.sort()`), and
/// [`App::from_tree`] flattens that listing straight after the directory's own
/// row, so the file rows of a directory are the run of rows following it that
/// are files at `depth`, in path order. The answer is the first of them that
/// sorts after `path`, or the end of that run — which is where the rows for the
/// directory's subdirectories begin, and so is before all of them.
///
/// Paths are compared as [`Path`] compares them, component by component, which
/// is the ordering `sort` on a [`Vec<PathBuf>`] uses: one ordering, so a spliced
/// row and a reloaded one land in the same place.
///
/// Pure, and deliberately free of [`App`]: rows and a path in, an index out.
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

/// The view `view` is showing, re-seated on `tree`: the rows and the tally of
/// the tree just handed over, under the selection, the collapsed set, the
/// filters, the window and the footer the reader already had.
///
/// An [`App`] reads a tree exactly once, in [`App::from_tree`], and answers
/// every later question from the rows that produced — so a tree that has
/// changed since then can reach the screen only as a *new* app. Building that
/// new app is the easy half. The half that matters is this one: a front end
/// that built it and left it as [`App::from_tree`] made it would answer every
/// re-read by expanding everything the reader had collapsed, dropping their
/// filters, throwing the selection to the first row and scrolling to the top.
/// That is worse than never re-reading at all, because it happens exactly when
/// the reader was watching something.
///
/// So the view is carried and only the tree is replaced. What comes from
/// `tree`: every row, every state on one, and [`App::counts`]. What comes from
/// `view`: which node is selected, which directories are collapsed, the
/// pacted-only and file flags, the scroll offset and the viewport height, the
/// header, the focus, the message, the pact in flight if there is one, and the
/// account of the pact with the panel's window onto it.
///
/// The account has to be carried for the plainest of reasons: the tree is read
/// again *because* a pact has just finished, so a re-seat that dropped it would
/// wipe the record of the run at the exact moment the run was over and the
/// reader turned to read it.
///
/// The selection and the collapsed set are carried by *path*, never by row
/// index: an index names whichever node now sits at that position, which after
/// a re-read is any node at all. A selected path the new tree no longer has
/// falls back to its nearest surviving ancestor — the deepest part of the way
/// to it that is still drawn, which is where the node the reader was looking at
/// went — and only falls to the first row when not even an ancestor of it
/// survived. A collapsed path the new tree has no node for is carried
/// untouched and hides nothing, exactly as [`App::with_collapsed`] documents:
/// a directory that goes and comes back should come back as the reader left it.
///
/// Nothing here knows why the tree was re-read. It is a function of two values
/// — a view and a tree — so it runs no pact, waits on no thread, spawns
/// nothing, reads no file and triggers no re-read of its own: whoever loaded
/// `tree` decided when to, and this only puts the reader back on top of it.
#[must_use]
pub fn reseat_on(view: &App, tree: &Tree) -> App {
    // Taken before anything is rebuilt, because it is the one fact about the
    // old view that the new rows cannot be asked for.
    let selected = view.selected_row().map(|row| row.path.clone());

    let mut reseated = App::from_tree(tree);
    reseated.collapsed.clone_from(&view.collapsed);
    reseated.pacted_only = view.pacted_only;
    reseated.show_files = view.show_files;
    reseated.viewport_height = view.viewport_height;
    reseated.header.clone_from(&view.header);
    reseated.message.clone_from(&view.message);
    reseated.in_flight.clone_from(&view.in_flight);
    // With the pass running inside it, for the same reason: the run did not stop
    // because a tree was re-read, so the file being paid for right now is still
    // being paid for and the line should not lose it for a frame.
    reseated.summarising.clone_from(&view.summarising);
    // Carried, like the message beside it and for the same reason: a tree being
    // re-read is not a keystroke, so it is not the thing that answers one. A
    // reader whose press was refused a frame ago should still be reading the
    // answer, and the run it refers to has been carried over too.
    reseated.pact_refused = view.pact_refused;
    reseated.focus = view.focus;
    reseated.account.clone_from(&view.account);
    reseated.panel_height = view.panel_height;
    reseated.panel_offset = view.panel_offset;
    reseated.panel_follows = view.panel_follows;

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
    reseated.scroll_offset = view.scroll_offset;
    reseated.rescroll();

    reseated
}

/// Which of `all` stand for nodes: the walk with every file row dropped, which
/// is what the view shows when the file toggle is off.
///
/// The one filter here that is about what a row *is* rather than about where it
/// sits, which is why it runs before the other two: the passes below reason
/// about depth alone, and they are owed a list in which every depth belongs to
/// a node.
///
/// Pure, and deliberately free of [`App`]: rows in, rows out.
fn node_rows(all: &[Row]) -> Vec<Row> {
    all.iter().filter(|row| !row.is_file()).cloned().collect()
}

/// Put every row inside the subtree at `path` into `state`: the directory
/// itself, every directory below it, and every file in any of them.
///
/// A pact covers a subtree, so a colour does too. The directories and their
/// files move together because they are one fact drawn more than once: a file
/// takes its module's colour, so the state on a file row is the state of the
/// directory holding it and has no other source to be refreshed from.
///
/// Pure, and deliberately free of [`App`]: rows in, rows painted.
fn paint_subtree(rows: &mut [Row], path: &Path, state: NodeState) {
    for row in rows {
        if in_subtree(&row.path, path) {
            row.state = state;
        }
    }
}

/// Whether `path` is the subtree rooted at `root`, or something inside it —
/// a directory below it, or a file in any of them.
///
/// Ancestry is a path prefix taken component by component, the way
/// [`Path::starts_with`] takes it and the way the engine's paths nest, so
/// `crates-old` is no part of the subtree at `crates` however much of the name
/// it shares.
fn in_subtree(path: &Path, root: &Path) -> bool {
    path.starts_with(root)
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
/// A collapsed node with nothing under it hides nothing. It cannot arrive
/// through [`App::toggle_collapsed`], which refuses such a node, but it can
/// arrive through [`App::with_collapsed`] carrying a set from a tree whose shape
/// has since changed, or through the file toggle taking away the only rows a
/// directory had.
///
/// The second value out is the other half of the same reading: the paths of the
/// rows that hold something *here*, collapsed or not. `all` is this view with
/// nothing collapsed, so a row holds something exactly when the row after it is
/// deeper than it — the same depth comparison the skipping runs on, asked one
/// row ahead instead of one row behind. It is worked out in this pass because
/// this is the only point where it can be: before it the answer is the tree's
/// rather than the view's, and after it a collapsed row's descendants are gone
/// and nothing is left to count.
///
/// Pure, and deliberately free of [`App`]: rows in, rows and paths out.
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

/// Where the panel's window onto an account of `lines` lines should start, given
/// a window `viewport` lines tall, a window parked at `offset`, and whether it is
/// `following` the newest line.
///
/// The rule the tree's `scroll_offset_for` cannot be: the tree's window is
/// dragged about by a selection and moves as little as it can, while the panel's
/// is either at the end of a list that is still being written or exactly where
/// the reader left it. So there is no minimum-movement case here, and no
/// selection either — two inputs decide it. Following, the answer is the last
/// screenful, whatever the account's length is *now*, which is what pins the
/// newest line to the bottom row as lines arrive. Parked, the answer is the
/// reader's own offset, clamped to what the account allows so that a window can
/// never hang past the end.
///
/// A window at least as tall as the account has nothing to scroll: everything is
/// on screen, so the top is the end and following it is standing still. A window
/// no lines tall — which is what a panel nobody has drawn has — has no screen to
/// scroll at all, and the honest offset for it is the top, exactly as the tree's
/// rule says of a viewport of zero rows.
///
/// Pure, and deliberately free of [`App`]: it takes three numbers and a flag and
/// returns one number, so every edge of it is testable with no account, no tree
/// and no terminal.
fn panel_offset_for(lines: usize, viewport: usize, offset: usize, following: bool) -> usize {
    if viewport == 0 {
        return 0;
    }
    let end = lines.saturating_sub(viewport);
    if following { end } else { offset.min(end) }
}

/// What the app says when a subtree has just been un-pacted, naming the
/// directory it was rooted at as `label`.
///
/// Un-pacting drops the manifest entries for a directory and everything below
/// it, and nothing else: the `WARLOCK.md` files those pacts were written in
/// stay exactly where they are. That is worth a line, because the subtree has
/// just gone grey, and grey is the colour of a directory Warlock knows nothing
/// about — a reader could easily take a whole subtree turning that colour for
/// the writing having been thrown away.
fn left_on_disk_message(label: &str) -> String {
    format!(
        "{label} is no longer pacted — every WARLOCK.md in it was left on disk, \
         untouched"
    )
}

/// What the app says while a pact is working the directory named `label`, which
/// is directory `position` of `total`.
///
/// A present participle and a fraction, and nothing else. The verb is the one
/// the product's own key is named after, so the line reads as the `p` key still
/// going rather than as a report about something; the fraction is what turns a
/// screen that has not changed in two minutes from a hung Warlock into a working
/// one, which is the whole reason the line exists.
fn pacting_message(label: &str, position: usize, total: usize) -> String {
    format!("pacting {label} ({position}/{total})")
}

/// What the app says while a refresh is working the directory named `label`,
/// which is directory `position` of `total`.
///
/// [`pacting_message`] with the other verb, and shaped identically down to the
/// fraction, because the reader is watching the same kind of work: a pass per
/// directory, minutes at a time, with a number that has to move. The verb is the
/// one the `r` key is named after, so the line says which key is still going —
/// which is the one thing a reader cannot tell from the shape alone.
///
/// The fraction counts the directories this run will visit, which for a refresh
/// is the stale ones rather than all of them: a refresh of a forty-directory
/// subtree with seven stale directories counts to seven. That is the engine's
/// counting, passed straight through, exactly as [`pacting_message`]'s is.
fn refreshing_message(label: &str, position: usize, total: usize) -> String {
    format!("refreshing {label} ({position}/{total})")
}

/// What the app says when the refresh key is pressed on a pacted directory that
/// is already fresh, naming it as `label`.
///
/// A refresh describes the stale directories under the one it is pointed at, so
/// a subtree with none is a run with nothing in it: starting one would spend
/// minutes and money re-describing content that already holds. Worded as news
/// rather than as an error, because it is the good outcome — the reader asked
/// whether anything needed doing and the answer is no — and said out loud rather
/// than silently ignored, so that a key that starts a long run on some rows and
/// nothing on others always says which it just did.
fn already_fresh_message(label: &str) -> String {
    format!("{label} is already fresh — there is nothing under it to describe again")
}

/// What the app says when the refresh key is pressed on a directory that is not
/// pacted, naming it as `label`.
///
/// A refresh re-describes an existing pact; it cannot make one, and a directory
/// with no pact has no grant to have gone stale against. So the refusal names
/// the key that would help rather than merely saying no — the reader is one
/// keystroke from what they wanted, and the two keys sit next to each other in
/// the footer.
///
/// This is the answer for a directory a `.warlockignore` keeps out as well. Such
/// a directory reads as unpacted, which is exactly what it is as far as the
/// manifest goes, and the sentence stays true: `p` is still the key that
/// would change it.
fn unpacted_message(label: &str) -> String {
    format!("{label} is not pacted — press p to pact it, and there will be something to refresh")
}

/// What the app says while a summarising pass over the file named `label` is
/// running inside the directory `pacting` — a line from [`pacting_message`] or
/// [`refreshing_message`] — is about: that same line with
/// `— summarising <file> (part/parts)` after it.
///
/// A clause on the run's line rather than a line of its own, because the footer
/// is a fixed three lines and because this *is* the run: it is where the minutes
/// are going right now. The directory and its fraction keep the front of the line
/// — they are what the whole run is measured in, and the file is a detail inside
/// one of their steps — and the second fraction is what moves while the first one
/// cannot, which is the difference between a big file being paid for and a hung
/// Warlock. Named with the same participle as the pass itself, so the two
/// fractions read as one sentence about one piece of work.
fn summarising_message(pacting: &str, label: &str, part: usize, parts: usize) -> String {
    format!("{pacting} — summarising {label} ({part}/{parts})")
}

/// What the app says when the pact or refresh key is pressed while `pacting` — a
/// line from [`pacting_message`] or [`refreshing_message`] — is already on the
/// footer: that same line with `— already running` on the end.
///
/// The answer to the press is worded as a suffix rather than as a line of its
/// own because the run's line is where the reader is already looking, and
/// because it answers the press in the run's own terms: the reason nothing
/// started is the thing the rest of the line is describing. It goes last so that
/// a terminal too narrow for the whole of it cuts the answer and keeps the
/// fraction.
fn already_running_message(pacting: &str) -> String {
    format!("{pacting} — already running")
}

/// What the app says when the pact key is pressed on a file, naming it as
/// `label`.
///
/// The one refusal the pact key has left, and the one it should have: a pact is
/// an agreement about a module — the document it is written in and the
/// directory it covers — and the files are what the module is made of, so the
/// answer is to point at the directory holding this one. Said out loud rather
/// than silently ignored: a key that does nothing on some rows and something on
/// others has to say which it just did.
fn file_row_message(label: &str) -> String {
    format!("{label} is a file — pacts are made with the directory holding it, not with a file")
}

/// What the app says when the pact key is pressed on a directory the
/// repository's `.warlockignore` keeps out, naming it as `label`.
///
/// The pact key's other refusal, worded in the same shape and for the same
/// reason: a key that does nothing on some rows and something on others has to
/// say which it just did. It names `.warlockignore` because the rule is the
/// repository's own and is written down in a file the reader can open and edit —
/// this is not Warlock deciding the directory is uninteresting, and the sentence
/// should send them to the place where that decision lives rather than sound
/// like a verdict.
///
/// The row stays gray and stays where it is. Being kept out is not a fourth
/// state and gets no colour of its own: gray already means outside Warlock's
/// management, and this sentence is the whole of how the difference is told.
fn ignored_row_message(label: &str) -> String {
    format!(
        "{label} is kept out by .warlockignore — Warlock covers nothing in there, so there is nothing to pact"
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
    use std::time::{Duration, Instant};

    use warlock_engine::{Node, NodeState, StateCounts, Tree};

    use super::{
        Account, App, Focus, Line, PactToggle, Row, Run, panel_offset_for, reseat_on,
        scroll_offset_for,
    };
    use crate::claude::Activity;
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

    /// Three rows rooted at an absolute path, for the tests about how a
    /// directory is named on the footer.
    ///
    /// [`three_rows`] and the shared fixture are both rooted at relative paths,
    /// which the engine's manifest spelling takes to be relative to the root
    /// already and hands straight back — so neither of them can show that a
    /// label really is cut down to its place under the tree's root.
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
                    format!("repo/module{index}/WARLOCK.md"),
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

    /// The drawn rows, by path and state, for a test that is about which rows
    /// changed colour and which were left alone.
    fn states(app: &App) -> Vec<(&str, NodeState)> {
        app.rows()
            .iter()
            .map(|row| (row.path.to_str().expect("ascii path"), row.state))
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

    /// The whole fixture with its files shown: every node, each one followed by
    /// the files it lists, one level deeper.
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

    /// The app for the shared fixture, selecting the row for `path`.
    fn app_selecting(path: &str) -> App {
        select(App::from_tree(&fixture::tree()), path)
    }

    /// The app for the shared fixture with its files shown, selecting the row
    /// for `path` — which may be a file.
    fn app_with_files_selecting(path: &str) -> App {
        let mut app = App::from_tree(&fixture::tree());
        app.toggle_files();
        select(app, path)
    }

    /// `app` with the row for `path` selected, reached by stepping down to it.
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
    fn the_repository_root_itself_gets_no_header_rather_than_a_dot_or_a_label() {
        let app = App::from_rows(three_rows()).with_scope("/repo", "/repo");

        // The header says which part of the repository is on screen. The whole
        // of it is not a part, so there is nothing for the line to say — and the
        // bare "." the relative spelling would give is not an answer, it is the
        // question left unanswered.
        assert_eq!(app.header(), "");
        assert_ne!(app.header(), ".");
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
    fn flattening_a_tree_carries_each_nodes_document() {
        let tree = fixture::tree();

        let app = App::from_tree(&tree);

        for row in app.rows() {
            let node = tree.find(&row.path).expect("row came from the tree");
            assert_eq!(
                row.document,
                node.document,
                "document for {}",
                row.path.display()
            );
        }
        // Including `crates/`, whose document is honestly absent.
        assert!(
            app.rows()
                .iter()
                .any(|row| row.document.is_none() && row.path == Path::new("warlock/crates"))
        );
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
        // nothing outside the subtree.
        assert_eq!(
            states(&app),
            [
                ("warlock", NodeState::PactedStale),
                ("warlock/crates", NodeState::PactedStale),
                ("warlock/crates/engine", NodeState::PactedStale),
                ("warlock/crates/tui", NodeState::PactedStale),
                ("warlock/assets", NodeState::Unpacted),
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
                ("warlock/crates", NodeState::PactedFresh),
                ("warlock/crates/engine", NodeState::PactedFresh),
                ("warlock/crates/tui", NodeState::PactedFresh),
                ("warlock/assets", NodeState::Unpacted),
            ]
        );
        assert_eq!(app.counts(), tally(&app));
        assert_eq!(app.counts().total(), 5);
    }

    #[test]
    fn a_subtree_state_reaches_the_rows_no_view_is_drawing() {
        let mut app = App::from_tree(&fixture::tree()).with_collapsed(["warlock/crates"]);
        assert_eq!(drawn(&app), ["warlock", "warlock/crates", "warlock/assets"]);

        app.set_subtree_state("warlock/crates", NodeState::PactedFresh);
        app.select_next();
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
        assert!(!app.has_account());
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

    /// Three rows with a `.warlockignore` keeping the middle one out: a covered
    /// sibling on either side, so a refusal that repainted the whole tree, or
    /// repainted nothing because it refused every press, would both be caught.
    ///
    /// Rows rather than a tree, and a hand-written flag rather than a load: the
    /// point of carrying the fact on the row is that no filesystem is needed to
    /// answer the key, and a test that built the fixture off disk could not show
    /// that.
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
        assert!(!app.has_account());
        assert!(!app.is_pacting());
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
        app.set_pact_summarising(
            Path::new("/repo").join("crates").join("warlock-engine"),
            2,
            5,
        );
        app.set_pact_refused();

        // The summarising clause and the refusal are worded exactly as they are
        // for a pact, and the refusal still goes last.
        assert_eq!(
            app.pact_line().as_deref(),
            Some(
                "refreshing crates/warlock-engine (3/7) — summarising \
                 crates/warlock-engine (2/5) — already running"
            )
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
    fn a_summarising_pass_names_the_file_and_its_part_beside_the_directory() {
        let mut app = App::from_rows(rooted_rows());
        app.set_pact_in_flight(Path::new("/repo").join("crates"), 3, 12);

        app.set_pact_summarising(Path::new("/repo").join("crates").join("Cargo.lock"), 2, 5);

        // One line: the directory and where the run is in it first, because that
        // is what the whole run is measured in, then the file being paid for now
        // and how far through it the passes are. Both named relative to the root
        // of the tree on screen, in the manifest spelling.
        assert_eq!(
            app.pact_line().as_deref(),
            Some("pacting crates (3/12) — summarising crates/Cargo.lock (2/5)")
        );

        // And the second fraction moves while the first one cannot, which is the
        // whole point of it.
        app.set_pact_summarising(Path::new("/repo").join("crates").join("Cargo.lock"), 3, 5);
        assert_eq!(
            app.pact_line().as_deref(),
            Some("pacting crates (3/12) — summarising crates/Cargo.lock (3/5)")
        );
    }

    #[test]
    fn the_next_directory_does_not_inherit_the_last_ones_summarising_file() {
        let mut app = App::from_rows(rooted_rows());
        app.set_pact_in_flight(Path::new("/repo").join("crates"), 3, 12);
        app.set_pact_summarising(Path::new("/repo").join("crates").join("Cargo.lock"), 2, 5);

        app.set_pact_in_flight(
            Path::new("/repo").join("crates").join("warlock-engine"),
            4,
            12,
        );

        // The file belonged to the directory the run has just left, so it goes
        // with it: the footer never names a file under a directory that is not
        // the one holding it.
        assert_eq!(
            app.pact_line().as_deref(),
            Some("pacting crates/warlock-engine (4/12)")
        );
    }

    #[test]
    fn no_summarising_wording_survives_the_end_of_a_run() {
        let mut app = App::from_rows(rooted_rows());
        app.set_pact_in_flight(Path::new("/repo").join("crates"), 3, 12);
        app.set_pact_summarising(Path::new("/repo").join("crates").join("Cargo.lock"), 2, 5);

        app.clear_pact_in_flight();

        assert_eq!(app.pact_line(), None);
        // And nothing of it comes back with the next run either.
        app.set_pact_in_flight(Path::new("/repo").join("crates"), 1, 2);
        assert_eq!(app.pact_line().as_deref(), Some("pacting crates (1/2)"));
    }

    #[test]
    fn a_summarising_pass_says_nothing_while_no_pact_is_in_flight() {
        let mut app = App::from_rows(rooted_rows());

        app.set_pact_summarising(Path::new("/repo").join("crates").join("Cargo.lock"), 2, 5);

        // There is no line for it to be a clause on: the file is a detail inside
        // a directory of a run, and outside a run it describes nothing.
        assert_eq!(app.pact_line(), None);
    }

    #[test]
    fn the_already_running_suffix_still_goes_last_with_a_summarising_pass() {
        let mut app = App::from_rows(rooted_rows());
        app.set_pact_in_flight(Path::new("/repo").join("crates"), 3, 12);
        app.set_pact_summarising(Path::new("/repo").join("crates").join("Cargo.lock"), 2, 5);

        app.set_pact_refused();

        // The answer to a key just pressed is still the last thing on the line,
        // so a terminal too narrow for all of it cuts that before it cuts either
        // fraction.
        assert_eq!(
            app.pact_line().as_deref(),
            Some("pacting crates (3/12) — summarising crates/Cargo.lock (2/5) — already running")
        );
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

    /// Whether the window sits over rows that exist: it may start at the top of
    /// a tree shorter than itself, but it must never hang off the end of one.
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
    fn a_fresh_app_draws_the_directories_and_not_the_files_in_them() {
        let app = App::from_tree(&fixture::tree());

        assert!(!app.show_files());
        assert_eq!(drawn(&app), whole_fixture());
        assert!(app.rows().iter().all(|row| !row.is_file()));
        // Not because the fixture has no files: they are held, and hidden.
        assert!(
            fixture::tree()
                .walk()
                .any(|(node, _)| !node.files.is_empty())
        );
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
        // `warlock/assets` has no child directories and lists two files, so
        // whether anything is under it is the file toggle's answer and not the
        // tree's. Asking the tree — `Row::children`, which is 0 here — is what
        // used to leave the key doing nothing at all on such a row.
        let mut app = select(App::from_tree(&fixture::tree()), "warlock/assets");
        let selected = app.selected();

        assert!(!app.can_collapse(selected), "nothing is drawn under it");
        assert!(app.selected_row().is_some_and(|row| !row.has_children()));

        app.toggle_collapsed();
        assert!(app.collapsed().is_empty(), "and the key refuses it");

        app.toggle_files();
        let selected = app.selected();

        assert!(app.can_collapse(selected), "now two file rows are");
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
        let mut app = app_with_files_selecting("warlock/crates/engine/WARLOCK.md");

        app.toggle_files();

        // The file's row is gone, and what it went behind is the directory that
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

    /// Every method a movement key reaches, named so a failure says which one
    /// broke the rule. The same six [`every_movement_clears_a_message`] drives,
    /// and the whole of what focus is allowed to redirect from the tree's
    /// selection to the panel's window.
    const MOVEMENTS: [(&str, Movement); 6] = [
        ("select_previous", App::select_previous),
        ("select_next", App::select_next),
        ("select_page_up", App::select_page_up),
        ("select_page_down", App::select_page_down),
        ("select_first", App::select_first),
        ("select_last", App::select_last),
    ];

    /// An app of [`MANY`] rows scrolled to the middle of them, with the panel
    /// focused. Mid-tree on purpose: a selection at either end could sit still
    /// under half the movement keys for reasons that have nothing to do with
    /// focus.
    fn panel_focused() -> App {
        let mut app = scrolled_to(MANY / 2);
        app.toggle_focus();
        app
    }

    #[test]
    fn a_fresh_app_has_the_tree_focused() {
        assert_eq!(App::from_rows(three_rows()).focus(), Focus::Tree);
        assert_eq!(App::from_tree(&fixture::tree()).focus(), Focus::Tree);
        assert_eq!(App::default().focus(), Focus::Tree);
    }

    #[test]
    fn the_focus_key_moves_to_the_panel_and_back() {
        let mut app = App::from_rows(three_rows());

        app.toggle_focus();
        assert_eq!(app.focus(), Focus::Panel);

        app.toggle_focus();
        assert_eq!(app.focus(), Focus::Tree);
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
            app.toggle_focus();
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
        app.toggle_focus();
        assert_eq!(app, before, "the focus key changed something else");
    }

    /// Everything that is not a movement key, run over an app with each focus in
    /// turn: the two apps must end up as one, focus aside.
    ///
    /// Each case is a name and something to do to an app. Written as one list
    /// rather than a test each, because what is being asserted is the same
    /// sentence six times over — focus decides what the *movement* keys mean and
    /// nothing else about this type.
    #[test]
    fn nothing_but_a_movement_key_cares_which_pane_has_the_focus() {
        type Change = fn(&mut App);
        let changes: [(&str, Change); 9] = [
            ("toggle_collapsed", App::toggle_collapsed),
            ("toggle_pacted_only", App::toggle_pacted_only),
            ("toggle_files", App::toggle_files),
            ("toggle_pact", |app| {
                app.toggle_pact();
            }),
            ("set_pact_in_flight", |app| {
                app.set_pact_in_flight("warlock/crates", 2, 5);
            }),
            ("set_pact_summarising", |app| {
                app.set_pact_in_flight("warlock/crates", 2, 5);
                app.set_pact_summarising("warlock/crates/Cargo.lock", 2, 5);
            }),
            ("clear_pact_in_flight", |app| {
                app.set_pact_in_flight("warlock/crates", 2, 5);
                app.set_pact_summarising("warlock/crates/Cargo.lock", 2, 5);
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
            unfocused.toggle_focus();

            assert_eq!(unfocused, focused, "{name} depends on the focus");
        }
    }

    /// How many lines of account the panel tests give the panel room for. Small
    /// enough that an account of a dozen lines has a top and a bottom that are
    /// nowhere near each other.
    const PANEL: u16 = 3;

    /// The instant `seconds` after `base`, so a run can be driven through an
    /// account without anything reading a clock. The same helper
    /// [`crate::account`]'s own tests use, for the same reason.
    fn at(base: Instant, seconds: u64) -> Instant {
        base + Duration::from_secs(seconds)
    }

    /// An app with the panel focused, [`PANEL`] lines of panel, and an account of
    /// one section holding `lines` activity lines — so `lines + 1` drawable
    /// lines, the heading included.
    ///
    /// The lines are numbered tool calls rather than bare thinking, so a test can
    /// say which line it is looking at from the text alone.
    fn app_pacting(lines: usize, base: Instant) -> App {
        let mut app = App::from_rows(three_rows());
        app.set_panel_height(PANEL);
        app.toggle_focus();
        app.start_account(base);

        let account = app.account_mut().expect("a run has just started");
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

    /// What the panel is drawing, as plain text, so a test asserts on the window
    /// a reader would see rather than on an offset.
    fn panel_text(app: &App, now: Instant) -> Vec<String> {
        app.panel_lines(now)
            .into_iter()
            .map(|line| match line {
                Line::Directory { path } => path.display().to_string(),
                Line::Clocked { text, .. } | Line::Summary { text } => text,
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
            app.set_panel_height(PANEL);

            assert!(!app.has_account());
            assert_eq!(app.account(), None);
            assert_eq!(app.panel_lines(now), Vec::new());
            assert_eq!(app.panel_lines_below(), 0);
            assert_eq!(app.panel_scroll_offset(), 0);

            // Not even a movement key can make a panel with no account say
            // something.
            app.toggle_focus();
            for (_, movement) in MOVEMENTS {
                movement(&mut app);
            }
            assert!(!app.has_account());
            assert_eq!(app.panel_lines(now), Vec::new());
        }
    }

    #[test]
    fn a_second_pact_starts_the_account_again_from_empty() {
        let base = Instant::now();
        let mut app = app_pacting(9, base);
        assert_eq!(app.account().map(Account::line_count), Some(10));

        app.start_account(at(base, 100));

        assert!(app.has_account());
        assert_eq!(app.account().map(Account::line_count), Some(0));
        assert_eq!(app.panel_lines(at(base, 100)), Vec::new());
        assert_eq!(app.panel_scroll_offset(), 0);
        assert!(app.panel_follows());
    }

    #[test]
    fn a_second_pact_puts_a_scrolled_back_panel_at_the_top_of_the_new_run() {
        let base = Instant::now();
        let mut app = app_pacting(9, base);
        app.select_first();
        assert!(!app.panel_follows());

        app.start_account(at(base, 100));
        app.account_mut()
            .expect("a run has just started")
            .open_section("crates/tui", at(base, 100));

        assert!(app.panel_follows());
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
        assert_eq!(app.panel_scroll_offset(), 0);
        assert_eq!(
            panel_text(&app, at(base, 3)),
            ["crates/engine", "Read line 0", "Read line 1"],
        );

        // Longer than the panel: the window is the last screenful, and moves as
        // each line arrives without anybody telling it to. Numbered tools
        // rather than thinking, because a stretch of thinking is one line
        // however often it is reported and this test needs several.
        for line in 2..6 {
            app.account_mut().expect("a run is under way").record(
                &Activity::Tool {
                    name: "Read".to_owned(),
                    detail: Some(format!("line {line}")),
                },
                at(base, line + 1),
            );
        }

        assert!(app.panel_follows());
        assert_eq!(app.panel_scroll_offset(), 7 - usize::from(PANEL));
        assert_eq!(app.panel_lines_below(), 0);
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

        assert!(!app.panel_follows());
        let parked = panel_text(&app, at(base, 9));
        assert_eq!(parked, ["Read line 5", "Read line 6", "Read line 7"]);

        // And the lines that arrive afterwards leave the window where it is.
        for line in 9..20 {
            app.account_mut()
                .expect("a run is under way")
                .record(&Activity::Thinking, at(base, line + 1));
        }

        assert_eq!(panel_text(&app, at(base, 30)), parked);
        assert_eq!(app.panel_scroll_offset(), 6);
    }

    #[test]
    fn the_end_of_list_key_puts_the_panel_back_on_live() {
        let base = Instant::now();
        let mut app = app_pacting(9, base);
        app.select_first();
        assert!(!app.panel_follows());
        assert_eq!(app.panel_scroll_offset(), 0);

        app.select_last();

        assert!(app.panel_follows());
        assert_eq!(
            panel_text(&app, at(base, 9)),
            ["Read line 6", "Read line 7", "Read line 8"],
        );

        // Live means live: the next line to arrive moves the window again.
        app.account_mut()
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
        assert!(!app.panel_follows());

        app.select_next();
        app.select_next();
        assert!(!app.panel_follows());

        app.select_next();

        assert!(app.panel_follows());
        assert_eq!(app.panel_lines_below(), 0);
    }

    #[test]
    fn the_panel_says_how_many_lines_are_below_the_view() {
        let base = Instant::now();
        let mut app = app_pacting(9, base);

        // Ten lines, three of them on screen at the end: nothing below.
        assert_eq!(app.panel_lines_below(), 0);

        app.select_first();
        assert_eq!(app.panel_lines_below(), 10 - usize::from(PANEL));

        app.select_next();
        assert_eq!(app.panel_lines_below(), 10 - usize::from(PANEL) - 1);

        app.select_last();
        assert_eq!(app.panel_lines_below(), 0);
    }

    #[test]
    fn a_panel_nobody_has_measured_has_nothing_to_scroll() {
        let base = Instant::now();
        let mut app = app_pacting(9, base);
        app.set_panel_height(0);

        // No height means no window: nothing is drawn, so the honest offset is
        // the top however long the account is, and no movement key can push it
        // off one — the same reading the tree's rule takes of a viewport of
        // zero rows.
        for (name, movement) in MOVEMENTS {
            movement(&mut app);
            assert_eq!(
                app.panel_scroll_offset(),
                0,
                "{name} moved a panel of no height"
            );
            assert_eq!(app.panel_lines(at(base, 9)), Vec::new(), "{name}");
            // Nothing is drawn, so the whole account is below what is drawn.
            assert_eq!(app.panel_lines_below(), 10, "{name}");
        }

        // And the window is measurable again the moment somebody measures it.
        app.set_panel_height(PANEL);
        assert_eq!(app.panel_scroll_offset(), 10 - usize::from(PANEL));
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

    /// How many rows or lines the tests below move by where they stand in for
    /// one notch of the wheel. Nothing in this module decides that number —
    /// whoever reads the pointer does — but a step of more than one is what
    /// makes clamping worth asserting on, and three is what will be asked for.
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
                wheeled.panel_scroll_offset(),
                stepped.panel_scroll_offset(),
                "{lines} up"
            );
            assert_eq!(
                wheeled.panel_follows(),
                stepped.panel_follows(),
                "{lines} up"
            );

            for _ in 0..lines {
                stepped.select_next();
            }
            wheeled.scroll_panel_down(lines);
            assert_eq!(
                wheeled.panel_scroll_offset(),
                stepped.panel_scroll_offset(),
                "{lines} back down"
            );
            assert_eq!(
                wheeled.panel_follows(),
                stepped.panel_follows(),
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
        assert!(app.panel_follows());

        app.scroll_panel_up(NOTCH);

        assert!(!app.panel_follows());
        assert_eq!(
            panel_text(&app, at(base, 9)),
            ["Read line 3", "Read line 4", "Read line 5"],
        );

        // Parked means parked: the lines that arrive next leave it where it is.
        app.account_mut()
            .expect("a run is under way")
            .record(&Activity::Thinking, at(base, 10));
        assert_eq!(app.panel_scroll_offset(), 10 - usize::from(PANEL) - NOTCH);
        assert!(!app.panel_follows());

        // And scrolling back to the end is live again, with nothing having to
        // say so.
        app.scroll_panel_down(MANY);

        assert!(app.panel_follows());
        assert_eq!(app.panel_lines_below(), 0);
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

        assert_eq!(reseated.account(), app.account());
        assert_eq!(reseated.panel_height(), usize::from(PANEL));
        assert_eq!(reseated.panel_scroll_offset(), 0);
        assert!(!reseated.panel_follows());
        assert_eq!(panel_text(&reseated, at(base, 9)), before);
    }

    /// The fixture's shape with `warlock/crates/tui` gone, and its files with
    /// it: what a re-seat meets when the node the selection was sitting on is
    /// not in the new tree at all.
    ///
    /// Hand-written rather than loaded, like every other tree these tests use,
    /// so a re-seat is driven by two values and needs no repository, no pact and
    /// no `claude` on the path.
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

    /// The path of the selected row, for the re-seat tests, which are about
    /// which node the selection is on rather than which index it sits at.
    fn selected_path(app: &App) -> Option<&Path> {
        app.selected_row().map(|row| row.path.as_path())
    }

    #[test]
    fn a_re_seat_takes_its_rows_its_states_and_its_tally_from_the_new_tree() {
        let app = App::from_tree(&fixture::tree());

        let reseated = reseat_on(&app, &fixture::tree_after_a_run());

        // The same five nodes, and the one the run worked on carrying the
        // document it wrote, in the state the new tree gives it.
        assert_eq!(drawn(&reseated), whole_fixture());
        let crates = &reseated.rows()[1];
        assert_eq!(crates.path, PathBuf::from("warlock/crates"));
        assert_eq!(
            crates.document,
            Some(PathBuf::from("warlock/crates/WARLOCK.md")),
            "the document the run wrote never reached the row"
        );
        assert_eq!(crates.state, NodeState::PactedFresh);
        // The old app still says what it always said: a re-seat builds a new
        // value rather than editing the one it was handed.
        assert_eq!(app.rows()[1].document, None);
        assert_eq!(app.rows()[1].state, NodeState::Unpacted);

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
        let mut app = App::from_tree(&fixture::tree()).with_scope("/repo", "/repo/warlock");
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
        app.set_pact_summarising("warlock/crates/Cargo.lock", 2, 5);
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

    /// The fixture as a later load would find it once the file at `path` had
    /// been written: listed on the directory holding it, in the sorted order the
    /// loader's `files.sort()` leaves a listing in.
    ///
    /// This is what an insertion is measured against. A test that asserted a
    /// hand-written order would be asserting the same guess twice; comparing
    /// against a tree built the way `from_tree` reads one is asserting that a
    /// mid-run splice and the reload after the run agree.
    fn tree_listing(path: &str) -> Tree {
        let mut tree = fixture::tree();
        assert!(
            list_file(&mut tree.root, Path::new(path)),
            "no node for the directory holding {path}"
        );
        tree
    }

    /// List `path` on whichever node under `node` is the directory holding it,
    /// keeping the listing sorted. Whether one was found.
    fn list_file(node: &mut Node, path: &Path) -> bool {
        if Some(node.path.as_path()) == path.parent() {
            node.files.push(path.to_path_buf());
            node.files.sort();
            return true;
        }
        node.children.iter_mut().any(|child| list_file(child, path))
    }

    /// The fixture with its files shown, which is the view an insertion is
    /// visible in at all.
    fn app_with_files() -> App {
        let mut app = App::from_tree(&fixture::tree());
        app.toggle_files();
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
        // whole walk behind the drawn rows are the load's as well.
        assert_eq!(app, expected);
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
        assert_eq!(app, expected);
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
    fn with_files_hidden_an_insertion_shows_nothing_until_the_toggle() {
        let mut app = App::from_tree(&fixture::tree());
        let before = app.clone();

        app.insert_file_row("warlock/assets/index.html");

        assert_eq!(app.rows(), before.rows());
        assert_eq!(drawn(&app), whole_fixture());

        app.toggle_files();

        let mut expected = App::from_tree(&tree_listing("warlock/assets/index.html"));
        expected.toggle_files();
        assert_eq!(drawn(&app), drawn(&expected));
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
