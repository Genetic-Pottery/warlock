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
//! Which of the screen's three places the keys drive is view state of the same
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
//! pact of the session. The account is one of the panel's two cards — the other
//! is the document last read — and the window onto each is view state exactly as
//! the tree's is: a height set per frame for the slot they share, and an offset
//! and one bit apiece saying whether that window is following the
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
use std::mem;
use std::path::{Path, PathBuf};
use std::time::Instant;

use warlock_engine::{IntoDocument, NodeState, StateCounts, Tree, to_manifest_path};

use crate::account::{Account, Line};
use crate::claude::Activity;
use crate::thread::{Ending, Thread};
use crate::wrap::rows as wrap_rows;

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
///
/// Whether a file row is its directory's own `WARLOCK.md` comes along for a
/// related reason: the fact is a comparison against the node's document, the tree
/// is gone by the time anything asks, and the only way to answer it later would
/// be to spell the file's name a second time. See [`Row::is_document`].
///
/// The scope written on the row's own pact entry comes along for the same reason
/// again: the renderer draws the label beside the name and has nothing but the
/// row in its hand. See [`Row::scope`].
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
    /// Whether this file row is the `WARLOCK.md` of the directory listing it —
    /// the one file in that listing Warlock wrote. Ask it with
    /// [`Row::is_document`], and set it with [`Row::with_document_row`].
    ///
    /// Never true on a directory row: a directory *has* a document (see
    /// [`Row::document`]) and is not one.
    ///
    /// Carried rather than worked out where it is wanted, for the reason every
    /// other fact on a row is carried: the answer is a comparison against
    /// [`warlock_engine::Node::document`], the tree the rows were flattened from
    /// is not kept, and the alternative — matching the row's file name against
    /// the literal `WARLOCK.md` — would be a second spelling of a name the engine
    /// owns and would call any stray `WARLOCK.md` a document even where the load
    /// found none.
    pub document_row: bool,
    /// Whether the repository's `.warlockignore` keeps this row's content out of
    /// Warlock, straight from [`warlock_engine::Node::is_ignored`]. Ask it with
    /// [`Row::is_ignored`], and set it with [`Row::with_ignored`].
    pub ignored: bool,
    /// The scope written on this row's *own* pact entry, straight from
    /// [`warlock_engine::Node::scope`], or `None` where it has none — which
    /// includes every unpacted directory, since a scope lives on a pact entry
    /// and an unpacted directory has none, and every file row, since a file has
    /// no entry either.
    ///
    /// This directory's scope and never an ancestor's. The label in the tree
    /// marks where a boundary *starts*, so a directory covered only by a scope
    /// written further up carries `None` here and draws nothing; the different
    /// question — which scope covers a given path — is
    /// [`warlock_engine::scope_covering`]'s, and nothing that fills this in may
    /// ask it, or the two answers would drift.
    ///
    /// It colours nothing and gates nothing: a scoped row is drawn in the same
    /// state colour it would have had unlabelled. Set it with
    /// [`Row::with_scope`].
    pub scope: Option<String>,
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
    ///
    /// Unscoped is the safe default for the same kind of reason: a scope is
    /// something somebody wrote on a pact entry, so a row nobody told about one
    /// has none to draw. Say otherwise with [`Row::with_scope`].
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
    ///
    /// The state is the only thing a file borrows from the directory holding it.
    /// The scope is not: a scope lives on a pact entry, a file has none, and the
    /// label in the tree marks the directory that owns the boundary rather than
    /// everything under it. So a file row is unscoped even inside a scoped
    /// directory, and there is no builder call here to make it otherwise.
    ///
    /// An ordinary file is the safe default in one more way: a row nobody told
    /// otherwise is not the holding directory's document, because deciding that
    /// takes the directory's [`warlock_engine::Node::document`] and this knows
    /// nothing of any node. Say otherwise with [`Row::with_document_row`].
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

    /// The same row, standing for the document of the directory listing it, or
    /// not.
    ///
    /// [`App::from_tree`] says this by comparing the file's path against the
    /// holding node's [`warlock_engine::Node::document`], and
    /// [`App::insert_file_row`] says it of every row it splices, since its caller
    /// hands over the document by construction. Between them that is the whole of
    /// how the fact reaches a row: nothing downstream re-derives it, and nothing
    /// downstream spells `WARLOCK.md`. It is a builder rather than an argument to
    /// [`Row::file`] so that a test can hand a row over without a tree, a loader
    /// or a disk behind it.
    #[must_use]
    pub const fn with_document_row(mut self, document_row: bool) -> Self {
        self.document_row = document_row;
        self
    }

    /// The same row, carrying — or no longer carrying — the scope written on its
    /// own pact entry.
    ///
    /// [`App::from_tree`] says this from [`warlock_engine::Node::scope`], and it
    /// is the whole of how the fact reaches the renderer. It is a builder rather
    /// than an argument to [`Row::new`] so that a test can hand
    /// [`App::from_rows`] a scoped row without a tree, a loader or a disk behind
    /// it.
    ///
    /// Pass this row's own scope and never an ancestor's: see [`Row::scope`].
    #[must_use]
    pub fn with_scope(mut self, scope: Option<String>) -> Self {
        self.scope = scope;
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

    /// Whether this row is the `WARLOCK.md` of the directory listing it: the one
    /// row under a directory that Warlock itself wrote.
    ///
    /// A fact carried from the load, never worked out here — see
    /// [`Row::document_row`] for why. True only on file rows, and on at most one
    /// file row per directory, since a node has at most one document.
    ///
    /// It says nothing about how the row is drawn: a document row takes its
    /// directory's colour like every other file row, and has no colour, shade,
    /// marker or label of its own.
    #[must_use]
    pub const fn is_document(&self) -> bool {
        self.document_row
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
/// Private, and what comes out of it comes out finished: the yes-or-no of
/// [`App::is_in_flight`], the worded line of [`App::pact_line`], and the
/// already-spelled [`RunHeader`] of [`App::run_header`]. The path is answered
/// against or spelled, never handed back, so there is no second place the
/// directory's wording could be decided. A renderer needs to know which row of
/// the tree is the one being worked, and asking about a path it already holds
/// settles that without learning how the run is spelled.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct InFlight {
    /// The directory the pact is working now.
    path: PathBuf,
    /// Which directory of the run this is, counting from one.
    position: usize,
    /// The furthest position this run has reached: `position` at its high-water
    /// mark, which is what a bar is filled to.
    ///
    /// It is state on the record rather than something worked out when a frame
    /// is drawn, because "furthest so far" is a fact about the run's history and
    /// a draw sees only the run's present. It is seeded from the position the
    /// run starts at and only ever rises, so a directory reported out of order —
    /// a late event, a re-send, an engine that counts a retry backwards — cannot
    /// take a fill that has already been drawn back down. It goes with the rest
    /// of the record on [`App::clear_pact_in_flight`], so the next run starts its
    /// fraction again rather than inheriting this one's.
    ///
    /// `position` is left exactly as the caller said it, because the footer
    /// reports what is happening now: see [`App::pact_line`].
    reached: usize,
    /// How many directories the whole run covers.
    total: usize,
    /// Whether the run is a pact or a refresh, which decides the verb
    /// [`App::pact_line`] words the line with and nothing else.
    run: Run,
}

/// The run in flight as a header states it: which run it is, the directory it is
/// working spelled for the tree on screen, and how far through the run that
/// directory is.
///
/// A snapshot, made when it is asked for and thrown away after — see
/// [`App::run_header`]. Nothing keeps one, and nothing outside this module can
/// make one: the fields are read-only, so the only way to a header is a run
/// actually being in flight.
///
/// It exists because a header needs the *parts* where the footer needs a
/// sentence. [`App::pact_line`] words one line and hands over the words;
/// a header sets the directory in one place and a bar filled to
/// `position/total` in another, and cannot take those out of a sentence again.
/// So the parts come out here, already spelled — `directory` is
/// [`App::label_for`]'s spelling, decided at the moment the header is asked for
/// and against the tree that is on screen then, which is the same rule the
/// footer's line is spelled by and the same single speller.
///
/// `position` is the run's high-water mark rather than the last position
/// reported, so a fill drawn from it never goes backwards within one run: see
/// `InFlight::reached`. `total` is the engine's own count of the directories the
/// run plans to visit and does not move for the length of it, so
/// `position/total` is the whole of what a bar is filled to — there is nothing
/// here to estimate a remaining time from, and nothing that moves when a clock
/// moves.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RunHeader {
    run: Run,
    directory: String,
    position: usize,
    total: usize,
}

impl RunHeader {
    /// Whether the run being reported is a pact or a refresh.
    #[must_use]
    pub const fn run(&self) -> Run {
        self.run
    }

    /// The directory being worked, spelled relative to the root of the tree on
    /// screen the way every other label in the front end is: see
    /// [`App::label_for`].
    #[must_use]
    pub fn directory(&self) -> &str {
        &self.directory
    }

    /// Which directory of the run is being worked, counting from one — at the
    /// furthest the run has got, so it never goes backwards.
    #[must_use]
    pub const fn position(&self) -> usize {
        self.position
    }

    /// How many directories the whole run covers, which does not move for the
    /// length of it.
    #[must_use]
    pub const fn total(&self) -> usize {
        self.total
    }
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

/// Which of the screen's three places the keys are driving.
///
/// The screen is a tree column, a panel beside it, and the composer at the foot
/// of the panel's column; a key that moves a selection has to be about one of
/// them, and a key that is a letter has to be either a command or a character
/// somebody typed. This says which. Three variants and no fourth: the footer
/// runs the width of the screen and is nobody's to drive, so there is nothing
/// else focus could land on.
///
/// It is deliberately not a general "which widget has the cursor" — nothing here
/// is a widget, and the one cursor there is sits at the end of the composer's
/// draft by construction. It is one piece of view state, cycled by one key, read
/// by the renderer to decide which border is lit and by [`App`] to decide
/// whether a keystroke is a command about a pane or a character for the draft:
/// see [`App::toggle_focus`] and [`App::focus`].
///
/// [`Focus::Composer`] is the one variant that is not always available: the
/// composer is drawn under the conversation and under neither of the other two
/// cards, and focus must never sit on a field nobody can see. That rule lives on
/// the app rather than here, because it is a fact about which card is showing —
/// see [`App::composer_showable`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum Focus {
    /// The tree column. The movement keys move its selection, which is what
    /// they have always done, and this is where a freshly built [`App`] starts:
    /// the tree is what warlock opens on and the thing there is anything to do
    /// with yet.
    #[default]
    Tree,
    /// The panel beside it. The same movement keys scroll the panel's window
    /// over whichever of its two cards is showing — the account of the pact or
    /// the document last read, see [`App::panel_scroll_offset`] — and the tree's
    /// selection stays exactly where the reader left it.
    Panel,
    /// The composer under the panel. The keyboard is the draft's while focus is
    /// here: a letter is the letter and not the command it spells, and a
    /// movement key moves nothing at all, because neither the tree's selection
    /// nor the panel's window is what the reader is pointed at. See
    /// [`Composer`](crate::Composer).
    Composer,
}

impl Focus {
    /// The next place round the cycle: what [`App::toggle_focus`] moves to.
    ///
    /// Written as a method on the enum rather than as arithmetic on an index
    /// somewhere, so that "focus is one of these places" stays the thing the
    /// type says and a fourth place would be a compile error here rather than a
    /// silent wrong answer.
    ///
    /// The order is the order they sit on screen: the tree, then the panel, then
    /// the composer beneath it, then round to the tree again. Whether the
    /// composer is a place the cycle can *stop* is not this method's business —
    /// it is a fact about which card the panel is showing, and [`App`] answers
    /// it.
    #[must_use]
    pub const fn next(self) -> Self {
        match self {
            Self::Tree => Self::Panel,
            Self::Panel => Self::Composer,
            Self::Composer => Self::Tree,
        }
    }

    /// Whether a movement key moves the tree's selection: whether this focus is
    /// the one driving the tree column.
    ///
    /// The single place the rule is written down, so every movement method asks
    /// the same question rather than each of them matching on the enum in its
    /// own way. `false` does not mean the key does something else here — at the
    /// panel it scrolls the panel's window, at the composer it does nothing
    /// whatever. See [`Focus::drives_the_panel`] and `App::movement`.
    ///
    /// Spelled out arm by arm rather than as a `matches!`, so that a variant
    /// added later has to be answered for here instead of quietly falling
    /// through to `false`.
    #[must_use]
    pub const fn drives_the_tree(self) -> bool {
        match self {
            Self::Tree => true,
            Self::Panel | Self::Composer => false,
        }
    }

    /// Whether a movement key scrolls the panel's window: the same question of
    /// the other pane that a movement key can be about.
    ///
    /// The pair of them is not one boolean, because there are three places focus
    /// can be and only two of them a movement key means anything at: the
    /// composer answers `false` to both, which is exactly how "a movement key at
    /// the composer moves nothing" is written down once. Arm by arm for
    /// [`Focus::drives_the_tree`]'s reason.
    #[must_use]
    pub const fn drives_the_panel(self) -> bool {
        match self {
            Self::Panel => true,
            Self::Tree | Self::Composer => false,
        }
    }
}

/// What this machine holds for the repository on screen, as the header states
/// it.
///
/// A **scope** is a fact about a directory, committed inside the repository; a
/// **sigil** is what one person on one machine holds, recorded by `warlock
/// config` at `<home>/.warlock/<project>/config.toml` and never inside a
/// repository. Nothing here matches the one against the other — this is a
/// statement, not a rule — and nothing here reads a disk either: whoever loaded
/// the config turns what they found into one of these three values and hands it
/// over with [`Chrome::with_sigils`], exactly as the header's text is worded
/// once by [`Chrome::of`].
///
/// Three variants because there are exactly three things the header can
/// honestly say, and the middle one is why this is not an `Option<Vec<String>>`:
/// a config that is there and will not parse must never look like a machine
/// that holds nothing, since the two mean opposite things about what is on disk.
/// The absent file and the empty set are deliberately the *same* value, on the
/// other hand — both are "nothing is held", and a reader who has never run
/// `warlock config` should see the header they have always seen.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub enum Sigils {
    /// Nothing is held: no config file, an empty set in one, or no home
    /// directory to look in at all. The header says nothing whatever about
    /// sigils in this state, which is what makes it the default — an app nobody
    /// has told is an app with nothing to state.
    #[default]
    Nothing,
    /// The sigils held, in the order the config lists them. Never empty: an
    /// empty set is [`Sigils::Nothing`], which is what [`Sigils::held`] is for.
    Held(Vec<String>),
    /// The config is there and could not be read or understood. Said out loud,
    /// so broken is never drawn as absent.
    Unknown,
}

impl Sigils {
    /// What was read out of the config, with the empty set folded into
    /// [`Sigils::Nothing`].
    ///
    /// The one constructor for a holding, so the "never empty" invariant on
    /// [`Sigils::Held`] is kept by construction rather than by every caller
    /// remembering it. It is the loader's natural shape too: the engine hands
    /// back a list, and an empty list is a machine that holds nothing.
    #[must_use]
    pub fn held(sigils: impl IntoIterator<Item = impl Into<String>>) -> Self {
        let sigils: Vec<String> = sigils.into_iter().map(Into::into).collect();
        if sigils.is_empty() {
            return Self::Nothing;
        }
        Self::Held(sigils)
    }

    /// The sigils held, as the matcher takes them.
    ///
    /// The one place the three states are flattened into the two the boundary
    /// question has, so what each one means to a refusal is written down once
    /// rather than decided at each key that can refuse.
    ///
    /// [`Sigils::Nothing`] is the empty slice, which opens no scoped directory:
    /// a sigil is what opens a scope, so holding none opens none. It still opens
    /// every *unscoped* directory, because that permissiveness lives on the
    /// directory rather than here — see
    /// [`scope_opens_to`](warlock_engine::scope_opens_to), where the asymmetry is
    /// argued. A machine that has never run `warlock config` is therefore refused
    /// by a scoped repository until somebody records what it holds, which is the
    /// onboarding this vocabulary was designed around rather than a failure mode.
    ///
    /// [`Sigils::Unknown`] is the empty slice **too**, and that is a decision
    /// rather than a fallthrough. A config that will not parse leaves warlock
    /// unable to establish that anything is held, and "nobody told me what you
    /// hold" is already the same answer as "what you hold does not match" — so
    /// refusing is the consistent reading rather than a second rule. The state is
    /// not swallowed on the way: the header says `holding unknown` out loud for
    /// as long as it lasts ([`Sigils::line`]), so a machine refused for a broken
    /// file is told which of the two it is without pressing anything, and the fix
    /// is to repair the file rather than to guess at it here.
    #[must_use]
    pub fn as_slice(&self) -> &[String] {
        match self {
            Self::Nothing | Self::Unknown => &[],
            Self::Held(sigils) => sigils,
        }
    }

    /// What the header has to say about what is held, or `None` when it has
    /// nothing to say.
    ///
    /// `None` is [`Sigils::Nothing`] and is the whole of the promise that a
    /// machine holding nothing gets the header it always had: there is no
    /// wording for it, not even an empty one to be joined onto the line with a
    /// separator.
    ///
    /// The wording is `warlock config`'s own, to the letter: the same word
    /// "holding", the same backticked sigils in the order the config lists
    /// them, and the same "unknown" for a config that would not read. The
    /// subcommand that sets these and the header that states them are
    /// describing one fact, and two wordings for one fact is one too many.
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

/// What the header line states: which tree is on screen, and what this machine
/// holds for the repository it came out of.
///
/// Both are resolved once, by whoever loaded the app, and neither can change
/// under a running warlock: the tree's root and the repository above it are
/// fixed for the session, and a sigil is written by `warlock config` with
/// warlock not running. So this is not app state and is deliberately not a field
/// on [`App`] — it is handed to [`draw`](crate::draw) beside the two windows
/// that are drawn over the frame, for the reason those are:
/// a value the app has never heard of is a value no keystroke, run or reload can
/// be suspected of having changed.
///
/// It used to be two fields on [`App`], which meant every reload had to carry
/// them and then recompute the header anyway — `reseat_on` copied the header
/// across and its one caller immediately overwrote it. Neither move is needed
/// when the fact never moves.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Chrome {
    header: String,
    sigils: Sigils,
}

impl Chrome {
    /// The header for the tree rooted at `root` inside the repository at
    /// `repo_root`, holding nothing.
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
    /// of the values above it all the way down to the renderer. A `root` that
    /// does not sit inside `repo_root`, or that is not UTF-8, cannot be
    /// described relatively at all and falls back to `root` printed lossily: a
    /// header is a label, and failing to draw one is no reason to fail to draw
    /// the tree.
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

    /// The same header, stating `sigils` beside it.
    ///
    /// The other half of the one line: the header says which tree is on screen,
    /// and this says what this machine holds for the repository it came out of.
    /// Set once, by whoever loaded the app — a sigil is written by `warlock
    /// config`, on the ordinary screen, with warlock not running.
    ///
    /// Takes the value rather than a path or a home directory: reading the
    /// config is the caller's, so this type keeps no filesystem and the three
    /// states it can be in are three values a test can write down. See
    /// [`Sigils`] for why there are three of them, and note that
    /// [`Sigils::Nothing`] leaves the header byte for byte the line it would
    /// have been if this had never been called.
    ///
    /// It states them and nothing else: no row is coloured, filtered, sorted or
    /// re-ordered by what is held, nothing is refused for it, and no key acts on
    /// it. Matching a sigil against a scope is not this slice's, and it would not
    /// be this type's when it is.
    #[must_use]
    pub fn with_sigils(mut self, sigils: Sigils) -> Self {
        self.sigils = sigils;
        self
    }

    /// The header line: what tree is on screen, as [`Chrome::of`] worded it.
    ///
    /// The repository identity alone. What this machine *holds* is stated on the
    /// same line and is kept apart from it here, because the header states the
    /// two in order of importance and drops the second when the pane is too
    /// narrow for both: see [`Chrome::sigils`].
    #[must_use]
    pub fn header(&self) -> &str {
        &self.header
    }

    /// What this machine holds for the repository on screen, as
    /// [`Chrome::with_sigils`] was told it.
    #[must_use]
    pub const fn sigils(&self) -> &Sigils {
        &self.sigils
    }
}

/// The front end's state: the flattened tree, where the selection sits, the
/// slice of rows on screen, and the three groups of view state beside them.
///
/// Nine fields, six of them about the tree and three of them values of their
/// own. The split is by *lifetime*, and it is the thing this type is arranged
/// around, because the one operation that has to know it — [`reseat_on`], which
/// puts a reader back on a tree that has just been read again — used to be
/// twenty assignments that named each field by hand and could not be checked.
/// The three groups are `Viewpoint`, what the reader has done to the view;
/// `Status`, what the footer is saying right now; and `Panel`, the two cards of
/// the slot beside the tree with a window onto each. Each carries whole across a
/// reload, so there is nothing inside them for that function to forget.
///
/// What is *not* here is the header line, and its absence is deliberate: see
/// [`Chrome`]. Both halves of it are resolved once and cannot change while
/// warlock runs, so an app rebuilt on every reload has no business holding
/// either, and the renderer is handed one directly.
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
/// `collapsible` is the paths of the rows that have something under them *in
/// this view* — which is not what the tree says, because the file toggle and the
/// pacted-only filter both change what a row holds without the tree moving. It
/// is derived beside `rows`, in the one pass that already knows: see
/// `drawn_rows`, and [`App::can_collapse`] for what reads it. Kept as paths
/// rather than as a flag on [`Row`] so that a row stays a fact about the tree
/// and two rows for the same node compare equal whatever is filtered.
///
/// The tally is the engine's own [`StateCounts`], carried along rather than
/// recomputed: counting states is the engine's job, and a renderer that adds
/// up its rows itself is a second implementation of that job waiting to
/// disagree with the first.
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
/// Those two are also the two that a re-seat cannot simply carry, which is why
/// they sit out here rather than in `Viewpoint` with the rest of the reader's
/// state: an index names whichever node now sits at that position, so the
/// selection has to travel by path and be looked up again in the new rows.
///
/// Holds an [`Account`], which holds an [`f64`] cost, so it is [`PartialEq`] and
/// not [`Eq`].
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

/// What the reader has done to the view, and the only part of an [`App`] that
/// survives a reload whole.
///
/// The five facts here move together because they have one lifetime: they are
/// set by a keystroke, they are true until another keystroke changes them, and
/// nothing about a tree being read again touches any of them. That is what lets
/// [`reseat_on`] carry them as one value rather than as five assignments it has
/// to remember to make — and what makes a sixth of them one field here rather
/// than a field, a copy and a test that nobody writes.
///
/// The two facts that are *not* here are `selected` and `scroll_offset`, and
/// they are missing on purpose. Both have to be re-derived against the new rows
/// rather than carried: the selection is carried by *path* and looked up again,
/// because an index names whichever node now sits at that position, and the
/// offset is restored and then put back in range by `rescroll`. They are the
/// exception, so they are visibly the exception — see [`reseat_on`].
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
/// `focus` is which of the screen's three places the keys are driving, and it is
/// here rather than in the event loop for the reason everything else here is:
/// it is view state that changes what a keystroke does, and the rule it decides
/// — that a movement key moves the tree's selection while the tree has the focus,
/// scrolls the panel while the panel has it, and moves nothing at all while the
/// composer has it — is a rule about [`App`]'s methods, testable with nothing
/// attached to stdout. It starts on the tree, which is the pane warlock opens
/// on. See [`Focus`].
///
/// The variant is the whole of what the composer keeps here. The draft itself is
/// deliberately *not* a field on [`App`]: [`App::restore_from`] puts a pre-run
/// copy of the app back after a run that recorded nothing and keeps only the
/// panel, so a draft stored anywhere but the panel group would die every time
/// somebody pacted a clean directory.
#[derive(Debug, Clone, Default, PartialEq)]
struct Viewpoint {
    collapsed: BTreeSet<PathBuf>,
    pacted_only: bool,
    show_files: bool,
    viewport_height: usize,
    focus: Focus,
}

/// What the footer is saying right now: the line about the last keystroke, the
/// run in flight, and the two flags that word them.
///
/// One lifetime again, and it is the shortest of the three: everything here
/// belongs to the keystroke just pressed or to the run going on behind it, and
/// every movement method empties the message as it goes. It is the group a run
/// that ends with nothing recorded *rolls back* — see `restore` in
/// `mod@crate::pacting` — which is the whole reason it is kept apart from the
/// panel beside it.
///
/// `mouse_captured` is here for its lifetime rather than its ownership: it is
/// not the app's own fact — the terminal is the binary's to switch, and the loop
/// tells the app what it did every frame — but it changes at the same rate as
/// everything around it and is read by the same footer, in `keys_line`. Being in
/// this group means a rolled-back run installs the flag as it was before the
/// run; that is what happened before this type existed too, and it is put right
/// by the loop's next frame, which sets it unconditionally.
///
/// The message is the one line the app has to say about the keystroke just
/// pressed — why a pact was refused, or whatever the caller put there. It is
/// finished text, and it lives here rather than in the caller's hand because it
/// is display state like everything else around it: the renderer draws whatever
/// is in it, and every method that moves the selection empties it, so a message
/// lasts exactly until the next keystroke.
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
/// `pact_refused` is the one keystroke that has nowhere else to go: the pact key
/// pressed while `in_flight` is already there. It is a flag rather than a
/// message because the message line is exactly what a pact in flight has taken —
/// a sentence put in the message would be the one sentence nobody could read —
/// and it is a flag rather than a fourth footer line because the footer is a
/// fixed three. So it is a bit that changes how `in_flight` is worded, and
/// nothing more: see [`App::set_pact_refused`] and [`App::pact_line`]. It
/// belongs to the keystroke that set it, so it goes the way the message does,
/// on the next one.
#[derive(Debug, Clone, Default, PartialEq)]
struct Status {
    message: Option<String>,
    in_flight: Option<InFlight>,
    summarising: Option<Summarising>,
    pact_refused: bool,
    mouse_captured: bool,
}

/// One slot, three cards: the account of the run, the conversation somebody is
/// having, and the document somebody asked to read, with a window apiece and
/// one bit saying which of them is on screen.
///
/// The group that survives everything. A reload carries it, because the tree is
/// read again *because* a run finished and dropping it would wipe the record at
/// the moment the reader turned to read it; and a run that ends with nothing
/// recorded carries it too, because an account is not a claim about the tree and
/// has no business being rolled back with one. That second rule used to be
/// `App::take_account_from`, a method whose whole purpose was to reach into a
/// live app and steal back the four fields that must not roll back. It is a
/// field move now.
///
/// `account`, `thread` and `document` are the cards, and there are exactly
/// three: no list, no stack, no history and no fourth card, so "which card is
/// showing" is [`Showing`]'s three variants and nothing anybody can grow. Each
/// card is either empty or filled — filled by [`App::start_account`],
/// [`App::start_turn`] and [`App::show_document`] respectively — and a filled
/// card stays filled: reading a file does not throw the account away, a pact
/// starting does not throw the document away, and neither of them empties the
/// conversation. That is the whole point of the slot being three cards rather
/// than one thing at a time: a reader with a document up keeps it while a run
/// goes on filling the card behind it, and the answer to a question they asked
/// a minute ago is still there when they swap back to it.
///
/// A run is written to exactly one of the cards — its own — however many of the
/// three are filled and whichever one the reader is looking at (see
/// [`App::start_account`] and [`App::write_run`]). A conversation carrying the
/// same run would be one run written twice on one screen, and a card swapped
/// away under the reader would be worse: what is showing is theirs.
///
/// A card being filled is not the same as it having lines. An app that has never
/// run a pact has nothing to draw in the panel at all, not even a heading, while
/// an account that has started and recorded nothing yet is a run under way — and
/// the difference between the two is a blank panel and one that has started. A
/// second pact starts a second account rather than appending to the first: one
/// pact, one account. The thread is the one card that is the other way round —
/// one session, one conversation, so a second question goes under the first
/// rather than in place of it. See `Card::accrue`.
///
/// `showing` is which card the panel draws, and it moves for exactly three
/// reasons: the view key bringing a document to the front ([`App::show_document`]),
/// a message submitted below bringing the thread to it ([`App::start_turn`]), and
/// the swap key. A run never moves it — a run fills its own card wherever the
/// reader is looking — and neither does a document being read again under the
/// reader after `$EDITOR` rewrote it ([`App::refill_document`]), nor anything a
/// turn already under way reports.
///
/// Each card carries its own `offset` and `follows`, which is what lets the
/// account go on following the newest line while the document is up and lets the
/// reader come back to the line they left. `height` and `width` are the two things all
/// three cards share, because there is one panel and it is as tall and as wide as the
/// frame says. The width is only ever *used* by two of them — a document is
/// wrapped to it and so is an answer, where an account is cut to it by the
/// renderer instead (see [`mod@crate::wrap`]) — but it is a fact about the panel
/// rather than about any one card, and it sits beside the height for that reason.
/// Together they are to the panel what `viewport_height` and `scroll_offset` are
/// to the tree — with one difference, which is the flag. The tree's window is
/// dragged about by a selection; the panel has no selection, so a card's window
/// is either pinned to the newest line or parked where the reader put it, and
/// `follows` says which. While it is set, `offset` is not read at all: the
/// offset is the end of the card, worked out from the line count at the moment
/// it is asked for, so appending a line moves the window without anybody having
/// to tell the window that a line was appended. See
/// [`App::panel_scroll_offset`].
///
/// `mode` is which register the conversation is in — see [`Mode`] — and it is
/// in this group rather than beside `focus` for the reason everything else here
/// is: it has to survive a run that ended with nothing recorded. A reader who
/// has spent ten turns converging on a brief and then pacts a clean directory
/// would otherwise find the conversation back in chat mode because a *pact*
/// rolled back, which is a run reaching into a conversation it has nothing to
/// do with. It is a fact about the card and not about the slot: the mode is
/// what the thread is, whichever of the three cards is being looked at.
#[derive(Debug, Clone, Default, PartialEq)]
struct Panel {
    account: Card<Account>,
    thread: Card<Thread>,
    document: Card<Vec<Line>>,
    showing: Showing,
    mode: Mode,
    height: usize,
    width: usize,
}

/// Which of the panel's three cards is on screen.
///
/// Three variants and no fourth, for [`Focus`]'s reason: the panel holds an
/// account, a conversation and a document, so "which card is showing" is one of
/// three named things, not an index into a list somebody could grow. There is no
/// `Nothing` here — a slot with an empty card showing draws warlock's mark,
/// which is a fact about the card rather than a fourth thing to be showing.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
enum Showing {
    /// The account of the pact running now, or of the last one to run. Reached
    /// only once a pact has run: an empty one is a card about nothing, and the
    /// swap key steps over it.
    Account,
    /// The conversation: every question typed below and what came back, and the
    /// card the panel opens on.
    ///
    /// The one card that is always somewhere to be. It is where the field is
    /// drawn, so it is where a session starts and where the swap key can always
    /// go back to — empty, it draws warlock's mark over a field with nothing
    /// typed in it yet, which is the whole of what an app that has just started
    /// has to say.
    #[default]
    Thread,
    /// The document last read. Reached only once [`App::show_document`] has put
    /// one there, and only by the swap key after that.
    Document,
}

impl Showing {
    /// The card after this one: what [`App::swap_card`] shows next.
    ///
    /// [`Focus::next`] for the slot, and written the same way and for the same
    /// reason — an arm per card, so "the panel is showing one of these" stays
    /// the thing the type says and a fourth card is a compile error here rather
    /// than a swap that quietly went nowhere. The order is the order the reader
    /// comes to them in: the conversation they start on, the run they asked for,
    /// then the file it wrote.
    const fn next(self) -> Self {
        match self {
            Self::Thread => Self::Account,
            Self::Account => Self::Document,
            Self::Document => Self::Thread,
        }
    }
}

/// Which register the conversation is in: questions about the repository, or a
/// conversation converging on a document.
///
/// Two variants and no third. A mode here is not a second system prompt and not
/// a second session — it is a state warlock holds plus one ordinary turn sent
/// into the conversation already in progress (see
/// [`brief_instruction`](crate::brief_instruction) and
/// [`CHAT_INSTRUCTION`](crate::CHAT_INSTRUCTION)) — so what the app keeps is
/// only this word, and everything the word changes is said out loud somewhere
/// else: which instruction a command sends, how hard the turn is asked to think
/// (see [`BRIEF_EFFORT`](crate::BRIEF_EFFORT)), and which model is asked (see
/// [`BRIEF_MODEL`](crate::BRIEF_MODEL)).
///
/// What it does *not* change is the card. Nothing on the thread is cleared,
/// hidden or reordered by a mode change, the session is the same session, the
/// system prompt is the same string, and the tool grant is byte-identical.
///
/// It is drawn in exactly one place: the panel's border title, where the thread
/// already says which card it is (see `draw_panel` in [`mod@crate::ui`]). Not on
/// the run header's row, which is the fixed row a pact takes over the panel and
/// would collide with a brief started while a run is in flight, and not on a row
/// of the card, which would be a row of the conversation spent saying what the
/// border was already able to say for nothing.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum Mode {
    /// Questions about the repository, answered one at a time: the register
    /// warlock opens in and the one it goes back to on `/chat`.
    #[default]
    Chat,
    /// The conversation is aimed at an artifact, and the model's job is to argue
    /// toward a decision about it rather than to answer as asked. Entered by
    /// `/brief` from the composer, and by nothing else — no key, no click and no
    /// tree action puts a conversation in this mode.
    Brief,
}

/// One card of the panel: what it holds, if anything, and where its window sits
/// over it.
///
/// Generic over what is on the card because the window rule is not: an offset,
/// a follow flag and a height decide where a list of lines is cut, whether that
/// list is an account being written or a file that was read once. What differs
/// is only how the lines are counted and produced, which is [`Shown`]'s two
/// implementations.
///
/// `held` is `None` for a card nothing has filled yet, and that is not the same
/// as a card holding an empty list: an unfilled card is a panel with nothing to
/// say, and an empty account or an empty file is something having happened. See
/// [`App::has_panel_content`].
#[derive(Debug, Clone, PartialEq)]
struct Card<T> {
    held: Option<T>,
    offset: usize,
    follows: bool,
}

/// An empty card whose window is at the top and following nothing: what both
/// cards of a freshly built [`App`] are.
///
/// Written out rather than derived because a derived `Default` would demand one
/// of `T` as well, and [`Account`] has none: an account is a run that started at
/// some instant, and there is no default instant.
impl<T> Default for Card<T> {
    fn default() -> Self {
        Self {
            held: None,
            offset: 0,
            follows: false,
        }
    }
}

impl<T: Shown> Card<T> {
    /// Put `held` on the card, with its window at the top and `follows` saying
    /// whether it is pinned to the newest line.
    ///
    /// Whatever the card was holding goes: one pact is one account, and one
    /// press of the view key is one document.
    fn place(&mut self, held: T, follows: bool) {
        self.held = Some(held);
        self.offset = 0;
        self.follows = follows;
    }

    /// What is on the card, ready to be added to, filling an empty card first
    /// and pinning its window to the newest line.
    ///
    /// The append path, and deliberately not a second [`Card::place`]: `place`
    /// drops what the card held, which is right for a run and for a read and
    /// wrong for a conversation. One session is one thread, so a question asked
    /// ten minutes in goes under the nine before it rather than in place of
    /// them, and only the first of them fills the card at all.
    ///
    /// It follows, every time. Somebody who has just asked something is asking
    /// to see the answer, so the window goes back to the newest line even if
    /// they had scrolled up to re-read an older turn — the one thing that moves
    /// a card's window without the reader having pressed a movement key, and it
    /// is their own keystroke that does it.
    fn accrue(&mut self) -> &mut T
    where
        T: Default,
    {
        self.follows = true;
        self.held.get_or_insert_with(T::default)
    }

    /// How many rows the card draws as at `width`, or none at all while nothing
    /// has filled it.
    ///
    /// A count of rows on screen rather than of lines held: a document line too
    /// long for the width is drawn in several rows, and this is what the window
    /// is cut out of, so it is the wrapped count or the arithmetic below it
    /// would be about a screen nobody is looking at.
    fn line_count(&self, width: usize) -> usize {
        self.held.as_ref().map_or(0, |held| held.line_count(width))
    }

    /// Which row of the card is drawn at the top row of a window `height` rows
    /// tall and `width` columns wide.
    fn scroll_offset(&self, height: usize, width: usize) -> usize {
        panel_offset_for(self.line_count(width), height, self.offset, self.follows)
    }

    /// The `height` rows the card's window covers, with every clock measured
    /// against `now`.
    fn window(&self, height: usize, width: usize, now: Instant) -> Vec<Line> {
        self.held.as_ref().map_or_else(Vec::new, |held| {
            held.window(self.scroll_offset(height, width), height, width, now)
        })
    }

    /// How many of the card's rows sit below a window `height` rows tall.
    fn lines_below(&self, height: usize, width: usize) -> usize {
        self.line_count(width)
            .saturating_sub(self.scroll_offset(height, width) + height)
    }

    /// Park the card's window at `offset`, or as near to it as the card's own
    /// length allows, and say whether that is still following.
    fn scroll_to(&mut self, offset: usize, height: usize, width: usize) {
        // Where the end is, asked of the one function that decides it, so that
        // "as far down as this card goes" means the same thing to a keystroke as
        // it does to the frame being drawn.
        let end = panel_offset_for(self.line_count(width), height, 0, true);
        self.offset = offset.min(end);
        self.follows = self.offset == end;
    }
}

/// What a card can hold: something with lines the panel can count and cut a
/// window out of.
///
/// The three implementations are the three cards — an [`Account`], which words
/// its own lines and has clocks in them, a [`Thread`], which has both clocks and
/// prose, and a document, which is the lines themselves. A document has no
/// clock, so `now` reaches only the other two; the panel draws one list whichever
/// card is up, and which list it is is this trait's business rather than the
/// renderer's.
trait Shown {
    /// How many rows this draws as in a panel `width` columns wide.
    fn line_count(&self, width: usize) -> usize;

    /// The `height` rows starting at `offset` in a panel `width` columns wide,
    /// with any clocks measured against `now`.
    fn window(&self, offset: usize, height: usize, width: usize, now: Instant) -> Vec<Line>;
}

/// An account is wrapped like everything else the panel draws: a pass that
/// reports a long tool detail, or fails with a whole sentence of somebody else's
/// stderr, has that line broken across the rows it needs rather than cut off at
/// the edge (see [`mod@crate::wrap`]). What separates one thing that happened
/// from the next is then the clock in front of it, since the count of rows is no
/// longer the count of events.
///
/// The count is taken from the account's own start, which is the one instant an
/// account that has rows can always name. `now` decides what a clock *says*
/// rather than whether it is a row — with the one exception that a clock going
/// from `9:59` to `10:00` takes a column off the line beside it, which can move
/// a line that was exactly one column inside the width onto a second row. The
/// cost of that is one row of scrollback being a frame behind, which the next
/// frame settles.
impl Shown for Account {
    fn line_count(&self, width: usize) -> usize {
        self.lines(self.started())
            .iter()
            .map(|line| rows_of(line, width).len())
            .sum()
    }

    fn window(&self, offset: usize, height: usize, width: usize, now: Instant) -> Vec<Line> {
        self.lines(now)
            .iter()
            .flat_map(|line| rows_of(line, width))
            .skip(offset)
            .take(height)
            .collect()
    }
}

/// A thread is every kind of line the panel has in one card — a question, the
/// account's own work lines, prose — and every one of them goes through the same
/// `rows_of` at the panel's width, so a terminal dragged narrower re-flows the
/// turn a reader is looking at rather than asking the model again. What differs
/// between them is only where a continuation row sits: under the marker for a
/// question, under the clock for a work line, flush left for the answer.
impl Shown for Thread {
    fn line_count(&self, width: usize) -> usize {
        // Counted from the rows themselves rather than from a second formula
        // over the turns, so the count and the window cannot come to disagree
        // about what a width does. Any instant answers: `now` decides what a
        // clock *says* and never whether it is a row, and nothing else in a
        // turn moves — so the first entry's own instant, the one a thread that
        // has rows can always name, is as good as the frame's.
        self.started().map_or(0, |started| {
            self.lines(started)
                .iter()
                .map(|line| rows_of(line, width).len())
                .sum()
        })
    }

    fn window(&self, offset: usize, height: usize, width: usize, now: Instant) -> Vec<Line> {
        self.lines(now)
            .iter()
            .flat_map(|line| rows_of(line, width))
            .skip(offset)
            .take(height)
            .collect()
    }
}

/// A document is its lines, already worded — the file's own lines, and the one
/// sentence about a read the cap cut short, which is the only line in it the
/// file did not write. [`App`] never holds the path it came from and never opens
/// anything: what reaches it is text, from whoever did the reading.
///
/// It is the card the width reaches, and a line of it is drawn in as many rows
/// as the width needs — see [`mod@crate::wrap`] for why a file is wrapped where
/// an account is cut. The lines held are the file's own either way: wrapping
/// happens on the way to the screen, at whatever width the frame is, so a
/// terminal made narrower re-flows the document a reader is looking at rather
/// than re-reading it.
impl Shown for Vec<Line> {
    fn line_count(&self, width: usize) -> usize {
        self.iter().map(|line| rows_of(line, width).len()).sum()
    }

    fn window(&self, offset: usize, height: usize, width: usize, _now: Instant) -> Vec<Line> {
        self.iter()
            .flat_map(|line| rows_of(line, width))
            .skip(offset)
            .take(height)
            .collect()
    }
}

/// The rows one line of a card draws as at `width`.
///
/// One row wherever the line fits, and the rows it needs where it does not: the
/// wrap module's answer, not this one's, so that what the app counts and what
/// the renderer draws are the same rows from the same code. See
/// [`rows`](crate::wrap::rows), which is also where the shape of a continuation
/// row is decided.
fn rows_of(line: &Line, width: usize) -> Vec<Line> {
    wrap_rows(line, width)
}

impl Panel {
    /// Which row of the showing card is drawn at the panel's top row.
    fn scroll_offset(&self) -> usize {
        match self.showing {
            Showing::Account => self.account.scroll_offset(self.height, self.width),
            Showing::Thread => self.thread.scroll_offset(self.height, self.width),
            Showing::Document => self.document.scroll_offset(self.height, self.width),
        }
    }

    /// The rows the panel draws now, from the showing card.
    fn window(&self, now: Instant) -> Vec<Line> {
        match self.showing {
            Showing::Account => self.account.window(self.height, self.width, now),
            Showing::Thread => self.thread.window(self.height, self.width, now),
            Showing::Document => self.document.window(self.height, self.width, now),
        }
    }

    /// How many of the showing card's rows sit below the panel.
    fn lines_below(&self) -> usize {
        match self.showing {
            Showing::Account => self.account.lines_below(self.height, self.width),
            Showing::Thread => self.thread.lines_below(self.height, self.width),
            Showing::Document => self.document.lines_below(self.height, self.width),
        }
    }

    /// Move the showing card's window, and only that card's: the other two keep
    /// the line the reader left them on, and an account or a thread left
    /// following goes on following.
    fn scroll_to(&mut self, offset: usize) {
        let (height, width) = (self.height, self.width);
        match self.showing {
            Showing::Account => self.account.scroll_to(offset, height, width),
            Showing::Thread => self.thread.scroll_to(offset, height, width),
            Showing::Document => self.document.scroll_to(offset, height, width),
        }
    }
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
    ///
    /// Each row is told the scope written on its own node the same way, from the
    /// node's own field — never an ancestor's, and never by asking
    /// [`warlock_engine::scope_covering`], because the label in the tree marks
    /// where a boundary starts. The file rows are told nothing: a file has no
    /// pact entry to write a scope on, so unlike the state and the exclusion
    /// flag there is nothing of the directory's to copy down. See [`Row::scope`].
    #[must_use]
    pub fn from_tree(tree: &Tree) -> Self {
        Self::from_rows(walk_of(tree)).with_counts(tree.counts())
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
    /// Nothing here says anything about the header line: neither half of it is
    /// app state, and an app carries no [`Chrome`] at all — see that type for
    /// why a fact resolved once and fixed for the session has no business being
    /// rebuilt on every reload.
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
    /// All three of the panel's cards are empty: no pact has run this session,
    /// nobody has asked anything and no file has been read, so the panel has
    /// nothing whatever to draw, which is a different state from a run that has
    /// started and done nothing yet. See [`App::start_account`]. The
    /// conversation is the card showing — it is the one card that is always
    /// somewhere to be, and the one the field is drawn under — and all three
    /// windows start the way the tree's does, no height and no offset, since
    /// nothing has been drawn.
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
                summarising: None,
                pact_refused: false,
                mouse_captured: false,
            },
            panel: Panel {
                account: Card {
                    held: None,
                    offset: 0,
                    follows: false,
                },
                thread: Card {
                    held: None,
                    offset: 0,
                    follows: false,
                },
                document: Card {
                    held: None,
                    offset: 0,
                    follows: false,
                },
                showing: Showing::Thread,
                mode: Mode::Chat,
                height: 0,
                width: 0,
            },
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
        self.viewpoint.collapsed = collapsed.into_iter().map(Into::into).collect();
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

    /// What the app has to say about the last keystroke, or `None` when it has
    /// nothing to say.
    ///
    /// Set by whatever refused to do something — [`App::toggle_pact`] on a file
    /// row — or by something that did it and had news about it, or by the
    /// caller through [`App::set_message`], and emptied by the next movement, so
    /// what is here always belongs to the keystroke just pressed.
    #[must_use]
    pub fn message(&self) -> Option<&str> {
        self.status.message.as_deref()
    }

    /// Say `message` until the next keystroke moves the selection.
    ///
    /// For the caller's own sentences — a manifest that would not write is the
    /// caller's news, not the app's — so that there is one place a line reaches
    /// the screen from rather than two. Replaces whatever was there: only the
    /// latest keystroke has anything to report.
    pub fn set_message(&mut self, message: impl Into<String>) {
        self.status.message = Some(message.into());
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
    ///
    /// This is also where the run's high-water mark is kept up: a call with a
    /// `position` lower than one already seen in this run leaves the furthest
    /// position where it was, so a header filled from it never draws a smaller
    /// fraction than it has already drawn — see [`App::run_header`]. The footer
    /// is not affected either way, because it reports the position it was just
    /// handed. A run that has been cleared has no history to keep, so the next
    /// one starts its fraction again.
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
        self.status.summarising = None;
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
        self.status.summarising = Some(Summarising {
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
    /// The run's high-water position goes with it too, because it is a field of
    /// the record being dropped rather than something kept beside it: this is
    /// the boundary between one run and the next, so a second run reports 1 of
    /// 12 rather than inheriting the first run's 12 of 12. See
    /// [`App::run_header`].
    ///
    /// A no-op when no pact was in flight.
    pub fn clear_pact_in_flight(&mut self) {
        self.status.in_flight = None;
        self.status.summarising = None;
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
        self.status.pact_refused = true;
    }

    /// Whether a pact is running now, as last set by
    /// [`App::set_pact_in_flight`].
    ///
    /// For a renderer deciding which keys to advertise, and for a key handler
    /// deciding what Esc means. It is display state and nothing more: it is
    /// whatever the caller last said, not something the app went and checked.
    #[must_use]
    pub const fn is_pacting(&self) -> bool {
        self.status.in_flight.is_some()
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
        self.status
            .in_flight
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
        self.status.in_flight.as_ref().is_some_and(|in_flight| {
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
        self.status.in_flight.as_ref().map(|in_flight| {
            let label = self.label_for(&in_flight.path);
            let mut line = match in_flight.run {
                Run::Pact => pacting_message(&label, in_flight.position, in_flight.total),
                Run::Refresh => refreshing_message(&label, in_flight.position, in_flight.total),
            };
            if let Some(summarising) = self.status.summarising.as_ref() {
                line = summarising_message(
                    &line,
                    &self.label_for(&summarising.path),
                    summarising.part,
                    summarising.parts,
                );
            }
            if self.status.pact_refused {
                already_running_message(&line)
            } else {
                line
            }
        })
    }

    /// The run in flight in parts rather than in words — which run, which
    /// directory, how far through — or `None` when no run is running.
    ///
    /// What [`App::pact_line`] is to the footer, this is to the panel's header:
    /// the same one in-flight record, read for the same frame, given out as the
    /// pieces a header sets in separate places instead of as a sentence. See
    /// [`RunHeader`] for what each piece is worth.
    ///
    /// Nothing new is measured for it. Every part comes off the record
    /// [`App::set_run_in_flight`] already keeps — which is why there is no new
    /// event, no new observer call and nothing to change in the engine — and the
    /// directory is spelled here, at the moment it is asked for, by the same
    /// [`App::label_for`] the footer uses, so a header and a footer drawn in one
    /// frame cannot spell one directory two ways.
    ///
    /// It reads no clock and takes none: two frames drawn at two instants with
    /// no event in between come back with the same header, because the fraction
    /// is the run's own counting and moves only when the run says it has moved.
    ///
    /// `None` before the first run and after [`App::clear_pact_in_flight`], so a
    /// caller needs no separate [`App::is_pacting`] check — and so the rows a
    /// header would take are the account's again the moment a run is over.
    #[must_use]
    pub fn run_header(&self) -> Option<RunHeader> {
        self.status.in_flight.as_ref().map(|in_flight| RunHeader {
            run: in_flight.run,
            directory: self.label_for(&in_flight.path),
            position: in_flight.reached,
            total: in_flight.total,
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
    ///
    /// A document in the panel stays exactly where it is, and stays showing. The
    /// panel is three cards in one slot: this one fills the account's card,
    /// wherever the reader happens to be looking, because which card is on
    /// screen is the reader's and a run that took the slot would take a document
    /// out of their hands mid-sentence. The account goes on being written behind
    /// the document, following its own newest line, and is there the moment they
    /// swap back.
    ///
    /// The conversation is not touched, and that is the rule rather than an
    /// omission. A run has a card of its own — this one — and a conversation
    /// that also carried it would be the same run written twice on one screen,
    /// arriving in the middle of whatever the reader was reading. The thread is
    /// what somebody typed and what came back; a pact is the account's, whole,
    /// one swap away.
    ///
    /// Which card is showing moves in one case and one only: when the card on
    /// screen has nothing on it. A reader watching a conversation keeps it, and
    /// a reader reading a file keeps that — taking either away is taking away
    /// something they chose to look at — but a reader who pressed the pact key
    /// on a session that has said nothing is looking at warlock's mark, and the
    /// run they just asked for is worth more than that. Nothing is lost either
    /// way: an empty card has nothing to come back to, and the swap key reaches
    /// it in one press once it does.
    ///
    /// The focus is rescued with it, because the account is a card with no field
    /// under it (see [`App::composer_showable`]): a keyboard pointed at a
    /// composer that has just stopped being drawn goes to the panel, exactly as
    /// it does when a document takes the field away.
    pub fn start_account(&mut self, at: Instant) {
        if !self.has_panel_content() {
            self.panel.showing = Showing::Account;
            self.rescue_focus();
        }
        self.panel.account.place(Account::new(at), true);
    }

    /// Do `write` to the account of the run in flight.
    ///
    /// The one way a run's events reach the panel. It takes a closure rather
    /// than the event because [`Account`] already knows how to word every line a
    /// run will ever have — opening a directory's section, recording an activity
    /// or a summarising pass, closing a section with its outcome, finishing the
    /// run — and a method here per kind of event would be a second place a run's
    /// line could come to be spelled.
    ///
    /// Does nothing when there is no account to write to, which is a run nobody
    /// started through [`App::start_account`]: a test driving events straight
    /// down the channel. Dropping the line is the honest way to fail.
    pub fn write_run(&mut self, write: impl FnOnce(&mut Account)) {
        if let Some(account) = self.panel.account.held.as_mut() {
            write(account);
        }
    }

    /// Put the lines of a file on the panel's document card, from its first
    /// line, and show it.
    ///
    /// The read happened somewhere else. What arrives here is text — the file's
    /// own lines, in order — and a yes-or-no about whether the cap cut the read
    /// short; never a path, because a path is something that would have to be
    /// opened later and [`App`] opens nothing. Whoever pressed the key did the
    /// reading, worded any failure on [`App::message`], and calls this only when
    /// there is something to show.
    ///
    /// `cut` adds one line under the last of the file's own, saying so. It is the
    /// only line in the panel a document did not write, and it is added here
    /// rather than by the reader because the words are the screen's: the engine
    /// hands over the fact and nothing else.
    ///
    /// The document's window goes to the top and does not follow. A file is read
    /// from its first line — a document pinned to its own last line would be a
    /// log — and the follow rule that keeps a live account's newest line on the
    /// bottom row has nothing to be true of here, since nothing is appended to a
    /// file that has been read.
    ///
    /// The account's card is left alone, lines, window and all: this is the one
    /// thing that shows the document card, and showing it puts the account
    /// behind it rather than throwing it out. A run under way goes on filling
    /// that card, and a swap comes back to it where it was.
    ///
    /// Not a keystroke's whole answer: it neither says anything nor takes down
    /// what the last keystroke said, exactly as [`App::start_account`] does not.
    ///
    /// The one other way lines reach this card is [`App::refill_document`],
    /// which is this method minus the last two lines of it: a file read again
    /// because something changed it under the reader does not get to decide what
    /// they are looking at, and so cannot hide the composer either.
    ///
    /// The document card hides the composer, so a reader who was typing when
    /// they pressed the view key has the focus moved onto the panel — see
    /// `rescue_focus`. Their draft is untouched: it is not kept here, and the
    /// field it is kept in is not emptied by anything on screen changing.
    pub fn show_document(&mut self, lines: impl IntoIterator<Item = impl Into<String>>, cut: bool) {
        self.refill_document(lines, cut);
        self.panel.showing = Showing::Document;
        self.rescue_focus();
    }

    /// Put the lines of a file on the document card again, leaving which card is
    /// showing exactly as it was.
    ///
    /// [`App::show_document`] without the one thing the view key does, and the
    /// two are one method plus a line for that reason: `v` is a reader asking to
    /// look at a file, so it brings the file to the front; this is the file
    /// somebody has just edited being read again underneath them, and a panel
    /// that flipped to the document because a `WARLOCK.md` was saved would take
    /// the account of a run out of the reader's hands without their having
    /// pressed anything. So the bit that says which card is drawn is not touched
    /// here at all: a document showing stays showing, an account showing stays
    /// showing, and the card behind is filled either way.
    ///
    /// *Which* file the card holds is not known here and is deliberately not kept
    /// here: what arrives is text and a `cut`, never a path, exactly as
    /// [`App::show_document`] documents — [`App`] opens nothing, so a path on it
    /// would be a path something later had to open. Whoever pressed the key knows
    /// which file it read, and it is that caller who decides this is the same
    /// file and calls this rather than leaving the card alone.
    ///
    /// The window goes back to the top and follows nothing, which is
    /// [`App::show_document`]'s rule and not a second one. The reader's line is
    /// deliberately not kept: the file has been rewritten under them, so line
    /// forty of the file they were reading is not line forty of the file that is
    /// there now, and parking the window at a number would point it at a line
    /// nobody chose. The top of the file is somewhere they can see they are.
    ///
    /// Says nothing and takes down nothing the last keystroke said, for
    /// [`App::show_document`]'s reason: it is not a keystroke's whole answer.
    pub fn refill_document(
        &mut self,
        lines: impl IntoIterator<Item = impl Into<String>>,
        cut: bool,
    ) {
        let mut lines: Vec<Line> = lines
            .into_iter()
            .map(|text| Line::Text { text: text.into() })
            .collect();
        if cut {
            lines.push(Line::Text {
                text: cut_at_cap_message(),
            });
        }
        self.panel.document.place(lines, false);
    }

    /// Put `message` on the thread as a new turn asked at `at`, and show it.
    ///
    /// What a submitted composer comes to. The message is the reader's own text,
    /// exactly as they typed it and never a path or a prompt this type built:
    /// [`App`] runs nothing and asks nobody, so whoever took the draft starts the
    /// worker themselves and reports back through the three methods below.
    ///
    /// This is the only way an app comes to have a turn in its thread at all, and
    /// one of the three things that decide which card is drawn: somebody who has
    /// just asked a question is looking for the answer, so the conversation comes
    /// to the front and its window goes to the newest line. The card accumulates
    /// rather than being replaced — one session, one conversation, so the turn
    /// goes under every turn before it — which is `Card::accrue`'s whole reason
    /// for existing beside `Card::place`.
    ///
    /// The other two cards are left exactly as they are, lines and windows and
    /// all: a run goes on filling the account behind the conversation, and a
    /// document read an hour ago is still on the line the reader left it on.
    /// The field is drawn under the conversation, so a question asked from a
    /// field that was already on screen leaves the focus exactly where it was.
    ///
    /// Not a keystroke's whole answer: it says nothing and takes down nothing
    /// the last keystroke said, exactly as [`App::start_account`] does not.
    pub fn start_turn(&mut self, message: impl Into<String>, at: Instant) {
        self.panel.thread.accrue().ask(message, at);
        self.panel.showing = Showing::Thread;
    }

    /// Put one line of warlock's own on the thread at `at`, and show it.
    ///
    /// What warlock says for itself — a draft refused, and later a file written
    /// or a document gone stale — as against a turn, which is something
    /// somebody asked a model. It brings the conversation to the front exactly
    /// as [`App::start_turn`] does, and for the same reason: the line is an
    /// answer to what the reader just did, and an answer on a card they are not
    /// looking at is not an answer. The card accumulates, so the note goes under
    /// everything already said rather than replacing it.
    ///
    /// One unclocked row, and no turn is opened, closed or frozen by it — see
    /// [`Thread::note`]. Not a keystroke's whole answer either: it says nothing
    /// on the footer and takes down nothing that is there.
    pub fn note(&mut self, text: impl Into<String>, at: Instant) {
        self.panel.thread.accrue().note(text, at);
        self.panel.showing = Showing::Thread;
    }

    /// Record what the live turn was seen doing at `at`.
    ///
    /// The tool calls, the thinking and the writing, as they arrive off whatever
    /// channel the turn is reporting over — one clocked line each, in the
    /// account's own words, with no tool result and no fragment of the answer
    /// among them. See [`Thread::record`], which decides all of that; this only
    /// finds the card.
    ///
    /// A no-op when nothing has been asked yet, and when the live turn is over
    /// already — the same rule, one level down, that keeps a late event from
    /// contradicting a line the reader can already see.
    pub fn record_turn(&mut self, activity: &Activity, at: Instant) {
        if let Some(thread) = self.panel.thread.held.as_mut() {
            thread.record(activity, at);
        }
    }

    /// Land `answer` on the live turn at `at` and close it.
    ///
    /// The one piece of prose warlock draws, kept whole and unwrapped here and
    /// wrapped to the panel's width on the way to the screen. A model that
    /// finished with nothing to say ends the turn instead of answering it — see
    /// [`Thread::answer`] — so a caller with an empty answer in hand needs no
    /// arm of its own.
    ///
    /// A no-op with no live turn, for [`App::record_turn`]'s reason.
    pub fn answer_turn(&mut self, answer: impl Into<String>, at: Instant) {
        if let Some(thread) = self.panel.thread.held.as_mut() {
            thread.answer(answer, at);
        }
    }

    /// Close the live turn at `at` with the one line `ending` makes.
    ///
    /// The cancel and every failure come through here, and every one of them is
    /// one line under whatever had already arrived: a turn stopped after two
    /// tool calls still shows those two tool calls. Nothing is returned and
    /// nothing is thrown: a turn that could not be run is a row in the panel,
    /// which is why [`Ending`] is a value and not an error.
    ///
    /// A no-op with no live turn, for [`App::record_turn`]'s reason, and on a
    /// turn that has ended already — the first ending wins, because it is the
    /// one on screen.
    pub fn end_turn(&mut self, ending: &Ending, at: Instant) {
        if let Some(thread) = self.panel.thread.held.as_mut() {
            thread.end(ending, at);
        }
    }

    /// The conversation this session has had, or `None` before the first
    /// question.
    ///
    /// Reaches the thread card whichever card is showing, exactly as
    /// [`App::account`] reaches the account: a turn goes on being answered while
    /// the reader looks at a run, and whether one is in flight is a fact about
    /// the conversation rather than about what is on screen. Read-only — the
    /// four methods above are the whole of how a turn is written — and it is
    /// what a test asks when it wants the turns themselves rather than the rows
    /// the panel would draw them in.
    #[must_use]
    pub const fn thread(&self) -> Option<&Thread> {
        self.panel.thread.held.as_ref()
    }

    /// Show the next card of the panel: the account, then the thread, then the
    /// document, then the account again.
    ///
    /// The whole of what the swap key does, and the only thing besides
    /// [`App::show_document`] and [`App::start_turn`] that decides which card is
    /// on screen — which is what makes a document survive a pact starting,
    /// finishing, failing or being cancelled underneath it. A cycle rather than
    /// three "show the account" / "show the thread" / "show the document" calls,
    /// for [`App::toggle_focus`]'s reason: there are three cards and one key, and
    /// a cycle cannot be asked for a fourth.
    ///
    /// A card nothing has filled is stepped over rather than shown, so the key
    /// never spends a press on warlock's mark: a session that has read no file
    /// swaps between the conversation and the run, and one that has run no pact
    /// swaps between the conversation and the file. The conversation is the
    /// exception and is never stepped over, empty or not — it is where the panel
    /// opens and the one card the composer is drawn under, so it is always the
    /// way back to a screen the reader can type on.
    ///
    /// Nothing else moves, with one exception. The focus stays on the pane it
    /// was on — unless it was on the composer and this is the swap that hides
    /// it, and then it lands on the panel as it does for
    /// [`App::show_document`]; see `rescue_focus`. The tree's
    /// selection and window stay where the reader left them, each card keeps its
    /// own window — so scrolling a card, swapping away and swapping back lands
    /// on the line they left, while an account or a thread left following goes on
    /// following and shows the newest line of whatever reported while the
    /// document was up — and the last keystroke's message stays exactly as it
    /// was. A swap that worked says nothing: the panel is now drawing another
    /// card, which the reader can see, and a footer line announcing it would only
    /// push aside something they had not finished reading.
    ///
    /// The one thing it says is the refusal, and there is exactly one press that
    /// comes to it: the conversation showing on a session where no pact has run
    /// and no file has been read, so both of the cards the key would reach are
    /// cards about nothing (see `stops_on`). The panel stays where it is and the
    /// footer names the key that would make one of them — the shape every
    /// refusal here takes: a fact about what is there, and then the keystroke
    /// that helps.
    ///
    /// Reads no file and asks the engine nothing: the cards are already in hand,
    /// and this only picks which of them is drawn.
    pub fn swap_card(&mut self) {
        let Some(card) = self.next_card() else {
            self.set_message(no_document_message());
            return;
        };
        self.panel.showing = card;
        self.rescue_focus();
    }

    /// The card a swap from the one showing lands on, or `None` when there is
    /// nowhere to go.
    ///
    /// Round the cycle, stopping at the first card the swap will stop on. Two
    /// steps and no loop: [`Showing::next`] is a three-way cycle, so the two
    /// cards after this one are the other two, and a `find` over them is the
    /// whole search.
    fn next_card(&self) -> Option<Showing> {
        let next = self.panel.showing.next();
        [next, next.next()]
            .into_iter()
            .find(|&card| self.stops_on(card))
    }

    /// Whether the swap key stops on `card`.
    ///
    /// The conversation always, and the other two only once there is something
    /// on them. The thread is where the field is and where the panel opens, so
    /// it is always somewhere to go back to — a reader who has read a file, or
    /// watched a pact, is one press from the place they can type. The account
    /// and the document are worth a press only once a pact has run or a file has
    /// been read: a press that landed on either of them empty would spend itself
    /// showing warlock's mark, which is what the reader can already see on a
    /// conversation nobody has started.
    ///
    /// So a swap is refused only from the thread on a session where nothing has
    /// run and nothing has been read, and [`App::swap_card`] says so on the
    /// footer rather than moving.
    const fn stops_on(&self, card: Showing) -> bool {
        match card {
            Showing::Thread => true,
            Showing::Account => self.has_account(),
            Showing::Document => self.has_document(),
        }
    }

    /// Whether a pact has run this session, and so whether the panel has an
    /// account card to draw.
    ///
    /// `false` until [`App::start_account`] and `true` from then on, whichever
    /// card is showing: an account that has finished is still an account, and an
    /// account behind a document is still an account. It is a question about
    /// what the panel *holds*, not about what is on screen.
    #[must_use]
    pub const fn has_account(&self) -> bool {
        self.panel.account.held.is_some()
    }

    /// Whether anybody has asked anything this session, and so whether the panel
    /// has a thread card to draw.
    ///
    /// `false` until the first [`App::start_turn`] and `true` from then on. A
    /// conversation is not started again, so this goes from `false` to `true`
    /// once and never back: a second question goes under the first.
    ///
    /// A run does not make it true. [`App::start_account`] appends a turn to a
    /// conversation that is already there and fills no card that is not, so a
    /// session that has pacted all morning and typed nothing still answers
    /// `false` here — and the swap key goes on stepping over the thread rather
    /// than spending a press on a history nobody has.
    #[must_use]
    pub const fn has_thread(&self) -> bool {
        self.panel.thread.held.is_some()
    }

    /// Whether a file somebody asked to read is on the panel's document card.
    ///
    /// The same question of the third card, and all three are freely `true` at
    /// once: the slot holds three cards, so a run started under a document fills
    /// one without emptying the others. `false` until the first
    /// [`App::show_document`] of the session.
    #[must_use]
    pub const fn has_document(&self) -> bool {
        self.panel.document.held.is_some()
    }

    /// Whether the card the panel is showing has anything to draw.
    ///
    /// What the renderer asks before it draws warlock's mark instead: the mark is
    /// what is there while the showing card is empty, and a card somebody has
    /// filled is something having happened. Asked of the showing card and not of
    /// the slot, because a document held behind an account nobody has started is
    /// not on screen and cannot be what the panel draws.
    #[must_use]
    pub const fn has_panel_content(&self) -> bool {
        match self.panel.showing {
            Showing::Account => self.has_account(),
            Showing::Thread => self.has_thread(),
            Showing::Document => self.has_document(),
        }
    }

    /// Whether the card on screen is the conversation.
    ///
    /// The one question the renderer asks about *which* card is showing, and it
    /// is asked for one reason: a reader must be able to tell the thread from
    /// the account without reading the rows, so the panel says which card it is
    /// on its own edge (see [`draw_panel`](crate::ui)). A run's card and a
    /// file's card each say what they are in every row they draw — a directory
    /// heading and a clock, or the file's own text — where a conversation could
    /// be either at a glance, and the answer to that is a word on the border
    /// rather than a fourth thing colour is made to mean.
    ///
    /// About the slot and not about the thread: a session with ten turns in it
    /// answers `false` here while the reader is looking at the account.
    #[must_use]
    pub const fn showing_thread(&self) -> bool {
        matches!(self.panel.showing, Showing::Thread)
    }

    /// Which register the conversation is in: [`Mode::Chat`] until somebody
    /// types `/brief`.
    ///
    /// A fact about the conversation and not about what is on screen: an app
    /// showing the account is still in brief mode, exactly as one showing the
    /// account still has a thread. The renderer asks it to word the panel's
    /// title, and whoever is about to send a turn asks it to decide how hard the
    /// turn thinks; nothing else reads it.
    #[must_use]
    pub const fn mode(&self) -> Mode {
        self.panel.mode
    }

    /// Put the conversation in `mode`, and say whether that changed anything.
    ///
    /// The answer is what a caller needs and cannot easily get back afterwards:
    /// a mode *change* is worth one note in the thread, where `/brief` typed in
    /// brief mode is a re-send that has nothing new to say about the register.
    /// So the comparison happens here, once, rather than in each caller against
    /// a copy of [`App::mode`] it had to remember to take first.
    ///
    /// It sets one word and touches nothing else. The card showing does not
    /// move, no turn is started, ended or reordered, nothing is written into the
    /// thread and the run header is not consulted — the note and the turn are
    /// the caller's, and this is only the state they are about.
    pub fn set_mode(&mut self, mode: Mode) -> bool {
        let changed = self.panel.mode != mode;
        self.panel.mode = mode;
        changed
    }

    /// The account of the pact running now, or of the last one to run, or `None`
    /// before the first pact of the session.
    ///
    /// Reaches the account card whichever card is showing: a run appends to its
    /// own card while the reader looks at a document, and the pulse in the tree
    /// is a fact about the run rather than about what is on screen.
    #[must_use]
    pub const fn account(&self) -> Option<&Account> {
        self.panel.account.held.as_ref()
    }

    /// The same account, to record what the run has just been seen doing.
    ///
    /// The account words its own lines — see [`Account`] — so this hands it over
    /// rather than wrapping each of its five mutators in a method here that
    /// would only forward. Nothing needs doing afterwards: while the account's
    /// card is following, its window is the end of the account computed on
    /// demand, so a line recorded through here is on screen at the bottom of the
    /// next frame without this type being told about it.
    ///
    /// `None` before the first pact, where there is nothing to record against,
    /// and only then: a run reports into its own card while a document is up,
    /// and finds the account it left there.
    ///
    /// This card and no other, which is also the whole of where a run goes: what
    /// a run reports reaches the panel through [`App::write_run`], and this is
    /// the card it writes.
    pub const fn account_mut(&mut self) -> Option<&mut Account> {
        self.panel.account.held.as_mut()
    }

    /// Put `view` back in place of this app, keeping this app's panel.
    ///
    /// The one move a run that ended with nothing recorded needs, and the whole
    /// of it. `view` is the copy taken before the run started, so it holds the
    /// rows, the colours and the selection the manifest on disk still says are
    /// true; what it cannot hold is the account of the run that has just
    /// happened, because it predates it. An account is not a claim about the
    /// tree and has no business being rolled back with one — and the run that
    /// ends this way is exactly the one a reader most wants to see the end of.
    ///
    /// So the panel stays and everything else goes back. This used to be
    /// `take_account_from`, a method that reached into the live app from the old
    /// one to steal four fields back out of it; with the panel a value of its
    /// own it is one move, and the four fields are not enumerated anywhere.
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

    /// How many lines of whatever the panel holds fit in it, as last set by
    /// [`App::set_panel_height`].
    #[must_use]
    pub const fn panel_height(&self) -> usize {
        self.panel.height
    }

    /// Tell the app how many lines fit in the panel.
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
        self.panel.height = usize::from(height);
    }

    /// How many columns wide the panel is, as last set by
    /// [`App::set_panel_width`].
    ///
    /// `0` for a panel nothing has measured, which is not a width to wrap a
    /// document at: see [`App::set_panel_width`].
    #[must_use]
    pub const fn panel_width(&self) -> usize {
        self.panel.width
    }

    /// Tell the app how many columns wide the panel is.
    ///
    /// [`App::set_panel_height`]'s counterpart, for the same reason and safe to
    /// call every frame in the same way: only the layout knows the width, and a
    /// line of a document is drawn in as many rows as that width needs, so how
    /// many rows the window is cut out of depends on it exactly as the size of
    /// the window depends on the height.
    ///
    /// Nothing is brought back into line afterwards, and here that is worth
    /// saying out loud: a terminal made narrower gives a document more rows than
    /// it had, so the offset the reader parked at is a row further up the file
    /// than it was. The alternative is remembering which line of the file the
    /// top row came from and re-deriving the offset from it on every resize,
    /// which is a second window to keep in step for something a reader sees once
    /// per drag of a terminal's corner. The offset is clamped when it is read,
    /// so what a resize can cost is a reader's place, never a panel scrolled off
    /// the end of what it holds.
    ///
    /// A width of `0` — a panel nobody has measured — wraps nothing: the
    /// document's lines are drawn as they are and cut to the width by the
    /// renderer, which is what the panel did before it was ever wrapped. Every
    /// frame measures, so that is the state of an app between being built and
    /// being drawn, and of a test that only cares about the height.
    pub fn set_panel_width(&mut self, width: u16) {
        self.panel.width = usize::from(width);
    }

    /// Whether the showing card's window is following the newest line of what is
    /// on it.
    ///
    /// `true` while that window sits at the end, which is where a fresh account
    /// starts and where the end-of-list movement key puts it back. `false` from
    /// the moment the reader scrolls up, until they scroll back down to the end
    /// or ask for it outright. See [`App::select_last`].
    ///
    /// `false` for a document from the moment it arrives, wherever its window
    /// is: nothing is ever appended to a file that has been read, so there is no
    /// newest line to be pinned to.
    ///
    /// The card that is not showing keeps its own answer, and it is not this
    /// one: an account left following goes on following behind a document, which
    /// is what puts the newest line of a run on screen when the reader swaps
    /// back to it.
    #[must_use]
    pub const fn panel_follows(&self) -> bool {
        match self.panel.showing {
            Showing::Account => self.panel.account.follows,
            Showing::Thread => self.panel.thread.follows,
            Showing::Document => self.panel.document.follows,
        }
    }

    /// Which line of the showing card is drawn at the panel's top row.
    ///
    /// Derived rather than stored while that card is following: the answer is
    /// then the end of the account, which changes every time a line is
    /// appended, and computing it here is what keeps the newest line pinned to
    /// the bottom without anything having to be recomputed as the run reports.
    /// Parked, it is where the reader left the window, clamped to what the
    /// card's length allows — which is where a document starts, since a file is
    /// read from its first line and nothing follows it.
    ///
    /// `0` for an app whose showing card is empty, and for a panel shorter than
    /// the account it holds is the first line of the last screenful.
    #[must_use]
    pub fn panel_scroll_offset(&self) -> usize {
        self.panel.scroll_offset()
    }

    /// The lines the panel draws now, with every clock measured against `now`.
    ///
    /// The window [`App::panel_scroll_offset`] describes, [`App::panel_height`]
    /// lines of it, taken from the showing card, and empty for an app whose
    /// showing card is empty. `now` is the caller's, because the newest line of a
    /// live section counts up between events and the only thing that knows what
    /// the time is when a frame is drawn is whoever is drawing it. A document has
    /// no clock in it and so does not move with `now` at all.
    #[must_use]
    pub fn panel_lines(&self, now: Instant) -> Vec<Line> {
        self.panel.window(now)
    }

    /// How many lines of the showing card sit below the panel's window.
    ///
    /// `0` while the panel is showing the end of that card, which is what an
    /// indicator saying how far back the reader has scrolled is switched off by,
    /// and `0` for an app whose showing card is empty. Independent of `now`:
    /// the clocks move, the number of lines does not.
    ///
    /// A panel nobody has measured draws nothing, so the whole of it is below
    /// it — which no frame ever sees, since a frame that asks this has just told
    /// the app how tall the panel it is drawing is.
    #[must_use]
    pub fn panel_lines_below(&self) -> usize {
        self.panel.lines_below()
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
        &self.viewpoint.collapsed
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
        self.viewpoint.collapsed.contains(path.as_ref())
    }

    /// Whether the row at `index` in [`App::rows`] has anything under it in the
    /// view as it stands, and so whether collapsing it would hide something.
    ///
    /// This is the question the collapse key asks, and it is a question about
    /// the view rather than about the tree. A directory holding nothing but
    /// files has no children in the tree and has rows under it whenever the file
    /// toggle is on — and a documented one has its document row under it even
    /// when the toggle is off; a directory whose children the pacted-only filter
    /// has taken away has children in the tree and nothing under it. Asking
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
        self.viewpoint.pacted_only
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
        self.viewpoint.pacted_only = !self.viewpoint.pacted_only;
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
        self.viewpoint.show_files
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
        self.viewpoint.show_files = !self.viewpoint.show_files;
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
        self.viewpoint.viewport_height
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
        self.viewpoint.viewport_height = usize::from(height);
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
        self.status.mouse_captured
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
        self.status.mouse_captured = captured;
    }

    /// Which place the keys are driving: the tree column, the panel beside it,
    /// or the composer under the panel.
    ///
    /// [`Focus::Tree`] for a freshly built app, which is the pane warlock opens
    /// on. For the renderer, deciding which border to light and where to put the
    /// cursor, for the event loop, deciding whether a letter is a command or a
    /// character, and for the movement methods below, deciding whether they mean
    /// anything.
    #[must_use]
    pub const fn focus(&self) -> Focus {
        self.viewpoint.focus
    }

    /// Whether the composer is a thing on screen: whether it is drawn, and so
    /// whether focus is allowed to land on it.
    ///
    /// One question with two readers. The renderer asks it to decide whether to
    /// cut rows off the bottom of the panel's column at all, and the event loop
    /// asks it to decide whether a keystroke can be the composer's. It is the
    /// same answer for both, and it is here rather than in either of them
    /// because it is a fact about which card the panel is showing.
    ///
    /// The thread and nothing else. The field writes into the conversation, so
    /// it belongs under the conversation: a document showing gives those rows
    /// back to the file, and an account showing gives them back to the run. A
    /// field under a run is a field about the wrong card — there is nothing on
    /// the account a sentence could be added to — and a reader who wants to ask
    /// something is one swap from the card that takes it.
    ///
    /// The rule is about the *card*, not about the draft. A composer holding
    /// nothing is still showable — that is the one empty row somebody types the
    /// first character into — and a draft somebody typed is not thrown away by
    /// the card that hides it.
    #[must_use]
    pub const fn composer_showable(&self) -> bool {
        match self.panel.showing {
            Showing::Thread => true,
            Showing::Account | Showing::Document => false,
        }
    }

    /// Move the focus one place round the cycle: tree, panel, composer, tree.
    ///
    /// The whole of what the focus key does. It is a cycle rather than three
    /// "focus the tree" / "focus the panel" / "focus the composer" calls because
    /// there are three places and one key, and a cycle cannot be asked for a
    /// fourth.
    ///
    /// The composer is skipped while it is not on screen — see
    /// [`App::composer_showable`] — so a reader with a document up tabs between
    /// the tree and the panel and never lands on a field that is not there. One
    /// skip is enough and there is no loop here: the composer is the only place
    /// that can be unavailable, and the place after it is always the tree.
    ///
    /// Nothing else moves: the selection stays on the row it was on, the windows
    /// stay where they were, the draft stays exactly as it was typed, and the
    /// last keystroke's message stays up. Focus changes what the *next* key
    /// means and says nothing itself, so there is nothing here for a message to
    /// report and nothing that would make a message stale.
    pub const fn toggle_focus(&mut self) {
        let next = self.viewpoint.focus.next();
        self.viewpoint.focus = match next {
            Focus::Composer if !self.composer_showable() => next.next(),
            Focus::Tree | Focus::Panel | Focus::Composer => next,
        };
    }

    /// Put the focus on `focus`, wherever it was.
    ///
    /// What a pointer can ask for and a key cannot. The focus key knows only
    /// "the next one", which is the whole of what one key over three places can
    /// mean; a click names the pane it landed in, and naming the pane that
    /// already has the focus has to leave it there rather than move off it —
    /// which is exactly what assigning the value it already holds does.
    ///
    /// Asking for the composer while it is not on screen puts the focus on the
    /// panel instead, which is the same rule [`App::toggle_focus`] keeps and the
    /// same one [`App::show_document`] applies to focus that was already there:
    /// nothing may point the keyboard at a field nobody can see.
    ///
    /// Nothing else moves, for the reason [`App::toggle_focus`] moves nothing
    /// else: focus changes what the *next* movement means and says nothing
    /// itself, so the selection, both windows, the draft and the last
    /// keystroke's message are none of its business.
    pub const fn set_focus(&mut self, focus: Focus) {
        self.viewpoint.focus = focus;
        self.rescue_focus();
    }

    /// Move the focus off the composer if the composer is not on screen.
    ///
    /// The one rescue, called by everything that can put the document card up
    /// under a focused composer — [`App::show_document`], [`App::swap_card`] and
    /// [`App::restore_from`] — and by [`App::set_focus`], which can be handed the
    /// composer outright. Written once here rather than at each of those, so
    /// "focus never sits on a hidden field" is one sentence in one place.
    ///
    /// It lands on the panel, not on the tree: the document that hid the
    /// composer is drawn in the panel, so the panel is where the reader is
    /// looking and the movement keys they press next are about the thing they
    /// just asked for. Nothing else moves, and nothing is said — a rescue is not
    /// a keystroke's answer.
    const fn rescue_focus(&mut self) {
        match self.viewpoint.focus {
            Focus::Composer if !self.composer_showable() => {
                self.viewpoint.focus = Focus::Panel;
            }
            Focus::Tree | Focus::Panel | Focus::Composer => {}
        }
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
    ///
    /// A movement at the composer does neither, because there is no third thing
    /// to move: the draft has no window and its cursor is at its end by
    /// construction, so `j` at the composer is the letter j and never reaches
    /// here at all. If one does — a wheel, a chord, anything routed here while
    /// the keyboard is the draft's — it is a no-op rather than a selection
    /// moving under a reader who is typing.
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
            let offset = panel(self, self.panel_scroll_offset());
            self.scroll_panel_to(offset);
        }
    }

    /// Park the showing card's window at `offset`, or as near to it as that card
    /// allows, and say whether that is still following.
    ///
    /// The one place the follow flag is decided by a keystroke, and it is
    /// decided by where the window ended up rather than by which key was
    /// pressed: following the newest line *is* sitting at the end of the card,
    /// so scrolling up breaks it and scrolling back down restores it, with no key
    /// having to mean "and start following again" as well as what it already
    /// means.
    ///
    /// The card that is not showing does not move. A key is about what is on
    /// screen, and a reader scrolling a document has said nothing whatever about
    /// where the account behind it should sit.
    fn scroll_panel_to(&mut self, offset: usize) {
        self.panel.scroll_to(offset);
    }

    /// How many rows one page key moves the tree by: a windowful, or a single
    /// row for an app whose window nobody has measured yet.
    const fn page(&self) -> usize {
        if self.viewpoint.viewport_height == 0 {
            1
        } else {
            self.viewpoint.viewport_height
        }
    }

    /// How many lines one page key moves the panel by, on the same rule.
    const fn panel_page(&self) -> usize {
        if self.panel.height == 0 {
            1
        } else {
            self.panel.height
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
            self.viewpoint.viewport_height,
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
    /// nodes and, under each documented one, the single file row that is its own
    /// `WARLOCK.md` — see [`node_rows`], which is the only pass that reads what a
    /// row *is*. Both passes below reason by depth, and a surviving document row
    /// is one they are right about without being told: it carries its directory's
    /// state, so pactedness keeps it wherever it keeps that directory for being
    /// pacted and never lets it rescue an ancestor; and it sits one level under
    /// its directory, so collapsing takes it away with everything else of that
    /// directory's. Every other file row is gone, and would have been a row those
    /// passes had to reason around for no reason.
    ///
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
        self.status.message = None;
        self.status.pact_refused = false;
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
    ///
    /// Public because a refusal worded outside [`App`] still has to name its row
    /// the way every refusal worded inside it does. The scope boundary is the
    /// one such refusal: sigils live on [`Chrome`], which is deliberately not a
    /// field here, so the app cannot word that message itself and must at least
    /// lend its spelling of the path.
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
        if !self.viewpoint.collapsed.remove(&path) {
            self.viewpoint.collapsed.insert(path);
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
            self.status.message = Some(file_row_message(&self.label_for(&path)));
            return None;
        }
        if ignored {
            self.status.message = Some(ignored_row_message(&self.label_for(&path)));
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
        self.status.message = (!pacted).then(|| left_on_disk_message(&self.label_for(&path)));

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

    /// Say which directory the scope key would set a scope on, and say why not
    /// when there is none.
    ///
    /// The scope key's half of what [`App::toggle_pact`] is for the pact key and
    /// [`App::refresh`] is for the refresh key: it decides what the press means
    /// on the row the selection is on, words any refusal, and hands the
    /// directory back for whoever actually opens the prompt and writes the
    /// manifest. `None` means nothing should open.
    ///
    /// A scope is a fact recorded against a pact — an entry in
    /// `.warlock/pacts.toml` — so the rows that have one to set are exactly the
    /// pacted ones, fresh or stale alike: whether a pact's grants still hold has
    /// nothing to do with which team the module belongs to. Two rows are
    /// refused, and each says so through [`App::message`]:
    ///
    /// - a file row, in [`App::toggle_pact`]'s own words, because the reason is
    ///   the same one — a file is part of a module rather than being one, and it
    ///   is the module that carries the pact a scope hangs off;
    /// - a directory that is not pacted, which points at the pact key: there is
    ///   no entry to record a scope against until one exists, so `p` is the key
    ///   that would help. A directory a `.warlockignore` keeps out reads as
    ///   unpacted here like any other, and is refused for that reason without
    ///   this having to know which reason made it so.
    ///
    /// A press that goes through changes *nothing whatever*, and that includes
    /// the message line: unlike the other two keys this one does not clear it,
    /// because opening the prompt and dismissing it with Esc must leave the app
    /// exactly as it was found — the reader who opened a window and closed it
    /// again did not answer the keystroke that put the last line up. Nothing is
    /// repainted, no tally moves, no selection moves and no run is touched
    /// either.
    ///
    /// Whether a run is already in flight is not asked here, exactly as it is
    /// not asked by the other two keys: that refusal is worded on the progress
    /// line rather than in the message (see [`App::set_pact_refused`]), so it
    /// belongs to the caller that knows about the worker. Nothing here reads the
    /// manifest either — what scope the directory carries now is read from the
    /// manifest by the caller that opens the prompt, never off a [`Row`] — and
    /// nothing here opens, spawns or writes anything: this is app state.
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

    /// Say which file the view key would read, and say why not when there is
    /// none.
    ///
    /// [`App::scope_target`]'s shape for the other key: it decides what the press
    /// means on the row the selection is on, words any refusal, and hands the
    /// file back for whoever actually opens it and puts its lines in the panel
    /// with [`App::show_document`]. `None` means nothing should be read.
    ///
    /// A file row is the yes, and it is the only one. Warlock reads a file, and
    /// the design doc's rule that a file has no state of its own is not bent
    /// here: a `WARLOCK.md` is an ordinary file row, so the document of a pacted
    /// directory is read by pressing the key on the document's own row, in
    /// whatever state or colour that row is drawn.
    ///
    /// A directory is the no, and it is refused in the terms the row it is on
    /// makes available, each said through [`App::message`]:
    ///
    /// - a directory that has a `WARLOCK.md` is one keystroke away from what the
    ///   reader wanted, so the refusal names that document — the row beneath the
    ///   directory — rather than only saying no;
    /// - a directory with no `WARLOCK.md` has nothing to read at all, so the
    ///   refusal names `p`, exactly as [`unpacted_scope_message`] does: a pact is
    ///   what would write the document this key would then read.
    ///
    /// A press that goes through changes *nothing whatever*, [`App::scope_target`]
    /// fashion, and that includes the message line and the panel: the reading has
    /// not happened yet, and a panel cleared here would blank on a read that then
    /// failed. Nothing is repainted, no tally moves, no selection moves and no run
    /// is touched.
    ///
    /// Nothing here opens, reads or writes anything: this is app state, and the
    /// path handed back is the row's own — the only filesystem fact in the answer
    /// is the one the walk already put on the row.
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
    /// Content the repository's `.warlockignore` keeps out is the one thing a
    /// pacted `state` does not reach — see [`moves_with_subtree`] for why the
    /// two directions differ. It is why this method takes a whole subtree and
    /// still cannot paint a pact onto a directory no pact will ever cover.
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
    /// The row it splices is the directory's *document*, and is marked as one
    /// ([`Row::is_document`]). That is the contract rather than a guess: the one
    /// caller is the pact observer, which passes the path the pass has just
    /// written on the engine's word that it wrote it, so there is nothing to
    /// compare and no `WARLOCK.md` to spell here. Nor could it be worked out from
    /// the holder row, whose [`Row::document`] is the *pre-run* tree's and is
    /// `None` for exactly the directory this method exists for: the one the run
    /// has this moment documented for the first time. So a caller that hands over
    /// some other file is not listing a file, it is saying that file is the
    /// document now, and gets a document row.
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
/// header, the focus, the message, the pact in flight if there is one, and both
/// of the panel's cards with the window onto each and which of them is showing.
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

/// The engine's whole walk of `tree`, flattened into the row list an [`App`]
/// keeps as `all_rows`.
///
/// Pulled out of [`App::from_tree`] so that [`reseat_on`] has the same thing to
/// build from without going through a whole [`App`] it would then have to take
/// apart again: the tree-derived half of a re-seat is this list and the tally
/// beside it, and nothing else.
///
/// Each row is told whether a `.warlockignore` keeps its content out, from the
/// node's own flag, and the file rows are told the same as the directory
/// listing them — the rules exclude a directory's content along with it, so a
/// file row saying otherwise would be a second answer to one question. It
/// changes no colour and hides no row: an excluded directory keeps its row and
/// its gray, and the flag is there for [`App::toggle_pact`] to refuse a press on
/// it without asking the filesystem anything.
///
/// Each row is told the scope written on its own node the same way, from the
/// node's own field — never an ancestor's, and never by asking
/// [`warlock_engine::scope_covering`], because the label in the tree marks where
/// a boundary starts. The file rows are told nothing: a file has no pact entry
/// to write a scope on, so unlike the state and the exclusion flag there is
/// nothing of the directory's to copy down. See [`Row::scope`].
///
/// Each file row is also told whether it is the holding directory's own
/// document, by comparing its whole path against that node's
/// [`warlock_engine::Node::document`] — whole paths, not file names, and the
/// node's own document rather than any name the walk happened to list. A node
/// has at most one document, so at most one file row under a directory is told
/// yes, and a directory whose load found no document has none among its files
/// however they are spelled. That field is presence-on-disk, so a `WARLOCK.md` an
/// un-pact left behind still compares equal and still gives a document row: the
/// tree says what is there. See [`Row::is_document`].
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

/// Which of `all` the view keeps when the file toggle is off: every node, and
/// under each documented one the single file row that is its own `WARLOCK.md`.
///
/// Every other file row goes. The document stays because it is the one file
/// Warlock wrote and the whole point of the tool, and a directory that showed no
/// sign of it would leave the reader nothing to land on and read; it is drawn as
/// the plain file row `f` already produces for it, with no colour, shade, marker
/// or label of its own. A node has at most one document, so a directory gains at
/// most one row, and an undocumented one gains none.
///
/// The one filter here that is about what a row *is* rather than about where it
/// sits, which is why it runs before the other two. What they are owed is not
/// quite a list of nodes any more, so it is worth saying why neither minds the
/// document rows that survive. A document row carries its directory's state (see
/// [`walk_of`]), so under [`pacted_rows`] it is kept exactly where its directory
/// is kept for being pacted, and it can never rescue an ancestor that its own
/// directory would not have rescued — while a leftover document under a gray
/// directory kept only as the way in to something pacted is dropped, like any
/// other unpacted row that is nobody's way in. And it sits directly under its
/// directory, one level deeper, so under [`drawn_rows`] it is a descendant of
/// that directory and of nothing else: collapsing the directory takes it away
/// with the rest, and it makes a documented directory with no children hold
/// something, which is exactly what [`App::can_collapse`] should say of it.
///
/// Pure, and deliberately free of [`App`]: rows in, rows out.
fn node_rows(all: &[Row]) -> Vec<Row> {
    all.iter()
        .filter(|row| !row.is_file() || row.is_document())
        .cloned()
        .collect()
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
        if moves_with_subtree(row, path, state) {
            row.state = state;
        }
    }
}

/// Whether `row` moves when the subtree at `root` is put into `state`.
///
/// Being in the subtree is most of the answer, and for every ordinary row it is
/// the whole of it. The exception is the row the repository's `.warlockignore`
/// keeps out, which moves in one direction only, because the two engine walks
/// underneath the two directions do not agree about it:
///
/// - **Into a pacted state, it does not move.** `pactable_directories` reads
///   `.warlockignore` and leaves excluded content out of the walk, so no pass
///   runs there, no `WARLOCK.md` is written and no entry is recorded. Painting
///   such a row yellow — or, when the grants land, green — would promise a pact
///   that the run has already decided not to make, and the promise would stand
///   until the reload at the end quietly took it back. This is the same refusal
///   [`App::toggle_pact`] makes when the excluded row is the one *selected*; it
///   belongs here as well, because a subtree paints rows the selection never
///   touched.
/// - **Into [`NodeState::Unpacted`], it moves like anything else.**
///   `unpact_subtree` is manifest arithmetic and drops every entry at or below
///   the directory without asking what the ignore rules say, so an excluded
///   directory carrying a pact from before it was excluded loses it with the
///   rest. Skipping it here would leave that row coloured for a pact that is no
///   longer recorded.
///
/// So the rule is not "excluded rows never move" but "excluded rows are never
/// pacted", which is the same thing the rest of Warlock says: a directory the
/// repository keeps out can always stop being managed, and can never start.
///
/// File rows are included in this. A file row carries the ignore flag of the
/// directory holding it (see [`Row::file`] and [`App::from_tree`]), so a file
/// inside excluded content is held back exactly as its directory is, and is not
/// left drawn in a colour its module never took.
fn moves_with_subtree(row: &Row, root: &Path, state: NodeState) -> bool {
    in_subtree(&row.path, root) && !(state.is_pacted() && row.is_ignored())
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

/// What the app says when the scope key is pressed on a directory that is not
/// pacted, naming it as `label`.
///
/// [`unpacted_message`]'s sibling, and deliberately not that same sentence: a
/// scope is recorded against a manifest entry, so what a pact would give this
/// directory is something to *scope* rather than something to refresh, and a
/// refusal that promised the wrong thing would send the reader looking for it.
/// The front half is shared word for word, because it is the same fact about the
/// same row, and it names the key that would help for the same reason — the
/// reader is one keystroke from what they wanted.
///
/// This is the answer for a directory a `.warlockignore` keeps out as well, for
/// [`unpacted_message`]'s reason: such a directory reads as unpacted, which is
/// what it is as far as the manifest goes, and `p` is still the key that would
/// change it.
fn unpacted_scope_message(label: &str) -> String {
    format!("{label} is not pacted — press p to pact it, and there will be a pact to scope")
}

/// What the app says when the view key is pressed on a directory that has a
/// `WARLOCK.md`, naming the directory as `label` and its document as `document`.
///
/// The refusal names the row that would have worked, because it is directly
/// beneath this one and the reader is one keystroke from what they asked for. A
/// directory is not a thing there is text of — the text is in the document — and
/// this is the whole of how that is told, in the shape [`unpacted_scope_message`]
/// established: the fact about the row, then the key or the row that would help.
fn directory_view_message(label: &str, document: &str) -> String {
    format!("{label} is a directory — press v on {document}, the row beneath it, to read it")
}

/// What the app says when the view key is pressed on a directory with no
/// `WARLOCK.md`, naming it as `label`.
///
/// [`directory_view_message`] with nothing to point at, so it points at the key
/// that would make something to point at: a pact is what writes a document, and
/// a directory with none has nothing whatever to read. Named `p` for
/// [`unpacted_message`]'s reason — the reader is one keystroke from the thing
/// they wanted to exist, and the two keys sit next to each other in the footer.
///
/// This is the answer for an unpacted directory and for a pacted one whose
/// document has not been written yet alike: what decides the wording is whether
/// there is a document to read, which is the question the key asks.
fn undocumented_view_message(label: &str) -> String {
    format!(
        "{label} is a directory with no WARLOCK.md — press p to pact it, and there will be a document to read"
    )
}

/// The one line a document gets that the file did not write: that the read
/// stopped at the cap and the file goes on past it.
///
/// Worded here rather than by whoever did the reading, because the engine hands
/// over the cut as a fact and the words on a screen are the screen's. It names
/// no size: the cap is a number the reader cannot do anything with, while "there
/// is more of this file than you are looking at" is the whole of what they need
/// to know before judging what they are reading.
fn cut_at_cap_message() -> String {
    "— cut here: the file goes on past this line, and Warlock reads no further".to_owned()
}

/// What the app says when the swap key is pressed on a session with nothing but
/// the conversation in the panel.
///
/// The slot holds three cards and two of them are empty, so there is nowhere to
/// swap to: said out loud rather than swallowed, because a key that did nothing
/// and reported nothing is indistinguishable from a key that is broken. In the
/// shape [`undocumented_view_message`] and `scoping::no_pact_message` share —
/// the fact about what is there, then the key that would make the thing the
/// reader asked for — and it names `v` rather than `p`, since a file is a
/// keystroke away where a pact is a run.
fn no_document_message() -> String {
    "nothing has been read this session — press v on a file row, and there will be a document to swap to".to_owned()
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
        Account, App, Chrome, Focus, Line, Mode, PactToggle, Row, Run, Showing, Sigils,
        cut_at_cap_message, no_document_message, panel_offset_for, reseat_on, scroll_offset_for,
    };
    use crate::account::Outcome;
    use crate::claude::Activity;
    use crate::fixture;
    use crate::thread::Ending;

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
    ///
    /// Node rows only. The counts are a count of directories, and a file row —
    /// including the document row the default view now draws under a documented
    /// directory — carries its directory's state rather than one of its own, so
    /// counting it would count that directory twice.
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

    /// Rows by path and depth, for a test comparing two views that are two
    /// accounts of the same rows rather than the same value — a mid-run tree and
    /// the tree the reload after it finds. Takes a list rather than an app so
    /// that the walk behind the screen can be compared as well as the screen.
    fn paths_and_depths(rows: &[Row]) -> Vec<(String, usize)> {
        rows.iter()
            .map(|row| (row.path.to_string_lossy().into_owned(), row.depth))
            .collect()
    }

    /// The drawn rows that stand for directories, by path: what `drawn` used to
    /// give back before the default view started drawing a document row under
    /// each documented directory, for the tests that are about the nodes.
    fn node_paths(app: &App) -> Vec<String> {
        app.rows()
            .iter()
            .filter(|row| !row.is_file())
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
    ///
    /// Every node, and under each documented one its own `WARLOCK.md` — the
    /// default view hides the files a directory merely holds, not the one
    /// Warlock wrote. `crates/` has no document in this fixture and so draws
    /// nothing under it.
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

    /// The fixture under the pacted-only filter: the two pacted leaves, the
    /// pacted root, and the undocumented `crates/` that is the only way down to
    /// them, each documented one still followed by its own `WARLOCK.md`.
    /// `assets/` is unpacted and has nothing pacted below it, so it and the
    /// document row under it both go — a document row carries its directory's
    /// state, so it is kept and dropped with the directory it belongs to.
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

    /// [`whole_fixture`] one load later: [`fixture::tree_after_a_run`] drawn in
    /// the default view, where the `WARLOCK.md` the run wrote under `crates/`
    /// gives that directory a document row it did not have before.
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

    /// The default view with `crates/` collapsed: the two modules under it go,
    /// and so do their document rows, which are descendants of `crates/` like
    /// anything else one level under a directory it holds.
    fn collapsed_over_crates() -> Vec<String> {
        vec![
            "warlock".to_owned(),
            "warlock/WARLOCK.md".to_owned(),
            "warlock/crates".to_owned(),
            "warlock/assets".to_owned(),
            "warlock/assets/WARLOCK.md".to_owned(),
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

    /// Every drawn row by path, depth and whether it stands for a file: the
    /// shape of the flattening, rather than only its order.
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

    /// A scoped directory holding a file, an unscoped pacted child and an
    /// unpacted one: every way a row can come out of the flatten with or without
    /// a scope, in one tree.
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

    /// The mode is one word and it is only a word: the card showing, the turns
    /// on the thread and the run header are exactly what they were, and a run in
    /// flight neither blocks the change nor is disturbed by it.
    #[test]
    fn setting_the_mode_changes_the_mode_and_nothing_else() {
        let base = Instant::now();
        let mut app = App::from_rows(rooted_rows());
        app.start_turn("what does this do", base);
        app.answer_turn("it walks the tree", at(base, 1));
        app.set_run_in_flight(Run::Pact, Path::new("/repo").join("crates"), 1, 4);

        let before = app.panel_lines(at(base, 2));
        let header = app.run_header().expect("a run in flight has a header");

        // Chat until somebody says otherwise, and the change is reported.
        assert_eq!(app.mode(), Mode::Chat);
        assert!(app.set_mode(Mode::Brief), "chat to brief is a change");
        assert_eq!(app.mode(), Mode::Brief);

        // Nothing else moved: same rows in the same order, same card, same
        // header — the mode wrote no note and started no turn of its own.
        assert_eq!(app.panel_lines(at(base, 2)), before);
        assert!(app.showing_thread(), "the mode moved the card showing");
        let after = app.run_header().expect("the mode took the header down");
        assert_eq!(after.run(), header.run());
        assert_eq!(after.directory(), header.directory());
        assert_eq!(after.position(), header.position());
        assert_eq!(after.total(), header.total());

        // Setting the mode it is already in is no change, and says so: that is
        // how a re-sent instruction knows to add no second note.
        assert!(!app.set_mode(Mode::Brief), "brief to brief is no change");
        assert_eq!(app.mode(), Mode::Brief);
        assert!(app.set_mode(Mode::Chat), "brief to chat is a change");
        assert_eq!(app.panel_lines(at(base, 2)), before);
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
        app.set_focus(Focus::Panel);
        app
    }

    /// The same app with the keyboard on the composer instead: mid-tree for
    /// [`panel_focused`]'s reason, and on the conversation, which is the card
    /// the field is drawn under and the card a session opens on.
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
        assert!(app.composer_showable(), "an app opens with room to type");

        // The file gives the panel those rows back, and so does the run: a
        // field under either is a field about the wrong card.
        app.show_document(["a line of a file"], false);
        assert!(!app.composer_showable(), "the document card hid nothing");

        app.start_account(base);
        assert_eq!(
            app.panel.showing,
            Showing::Document,
            "the run took the panel"
        );
        assert!(!app.composer_showable());

        app.swap_card();
        assert_eq!(app.panel.showing, Showing::Thread);
        assert!(app.composer_showable(), "the conversation gave it back");

        app.swap_card();
        assert_eq!(app.panel.showing, Showing::Account);
        assert!(!app.composer_showable(), "the run drew a field of its own");
    }

    #[test]
    fn a_run_takes_the_panel_only_from_a_card_with_nothing_on_it() {
        let base = Instant::now();

        // Nothing said and nothing read: the panel is warlock's mark, and the
        // run the reader just asked for is worth more than that.
        let mut app = App::from_rows(three_rows());
        app.set_panel_height(PANEL);
        app.start_account(base);
        assert_eq!(app.panel.showing, Showing::Account);

        // A conversation on screen is something they chose to look at, and a
        // run started behind it leaves it exactly where it is.
        let mut app = App::from_rows(three_rows());
        app.set_panel_height(PANEL);
        ask_and_answer(&mut app, base);
        app.start_account(at(base, 10));
        assert_eq!(app.panel.showing, Showing::Thread);

        // And so is a file.
        let mut app = App::from_rows(three_rows());
        app.set_panel_height(PANEL);
        app.show_document(document_lines(), false);
        app.start_account(at(base, 10));
        assert_eq!(app.panel.showing, Showing::Document);
    }

    #[test]
    fn a_run_taking_an_empty_panel_takes_the_keyboard_off_the_field_with_it() {
        // The account draws no field, so a keyboard left pointing at one would
        // be pointing at nothing — the same rescue a document does.
        let mut app = App::from_rows(three_rows());
        app.set_panel_height(PANEL);
        app.set_focus(Focus::Composer);
        assert_eq!(app.focus(), Focus::Composer);

        app.start_account(Instant::now());

        assert_eq!(app.panel.showing, Showing::Account);
        assert!(!app.composer_showable());
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

        assert!(!app.composer_showable(), "the panel came back with the app");
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
                app.panel_scroll_offset(),
                before.panel_scroll_offset(),
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
            unfocused.set_focus(focused.focus());

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
                .account()
                .map(Account::line_count)
                .expect("a run has started");
            assert_eq!(lines, 12);
            if in_flight {
                app.set_run_in_flight(Run::Pact, "crates/engine", 2, 5);
            }
            app.set_panel_height(u16::try_from(window).expect("a window this small"));

            // Following the newest line: the window is the last screenful of
            // the window it was given, and nothing is below it however many
            // rows the header took.
            assert!(app.panel_follows(), "{name}");
            assert_eq!(
                app.panel_scroll_offset(),
                panel_offset_for(lines, window, 0, true),
                "{name}"
            );
            assert_eq!(app.panel_scroll_offset(), lines - window, "{name}");
            assert_eq!(app.panel_lines_below(), 0, "{name}");
            assert_eq!(app.panel_lines(at(base, 99)).len(), window, "{name}");

            // Parked at the first line: what is below is everything the shorter
            // window does not cover, counted against that window rather than
            // against the one the panel would have had with no header on it.
            app.select_first();
            assert!(!app.panel_follows(), "{name}");
            assert_eq!(
                app.panel_scroll_offset(),
                panel_offset_for(lines, window, 0, false),
                "{name}"
            );
            assert_eq!(app.panel_lines_below(), lines - window, "{name}");
            below_at_top.push(app.panel_lines_below());

            // Parked in the middle: what is above the window, what is drawn in
            // it and what is below it come to the account, so the header's row
            // is neither counted twice nor lost between the three.
            app.select_next();
            app.select_next();
            let offset = app.panel_scroll_offset();
            assert_eq!(offset, 2, "{name}");
            assert_eq!(
                offset,
                panel_offset_for(lines, window, offset, false),
                "{name}"
            );
            assert_eq!(
                offset + app.panel_lines(at(base, 99)).len() + app.panel_lines_below(),
                lines,
                "{name}"
            );

            // And the count is off by exactly nothing: a line at a time down
            // reaches the end of the account after that many presses, and the
            // window is following again when it gets there.
            let below = app.panel_lines_below();
            for step in 1..=below {
                app.select_next();
                assert_eq!(app.panel_lines_below(), below - step, "{name}, {step} down");
            }
            assert!(app.panel_follows(), "{name}");

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

    /// The lines of a small file, as whoever read it would hand them over.
    fn document_lines() -> Vec<String> {
        (0..5).map(|line| format!("line {line}")).collect()
    }

    #[test]
    fn a_document_is_drawn_from_its_first_line_in_the_order_it_was_given() {
        let now = Instant::now();
        let mut app = App::from_tree(&fixture::tree());
        app.set_panel_height(PANEL);

        app.show_document(document_lines(), false);

        // The window is the top of the file: a file is read from its first line,
        // and nothing is appended to it for the panel to follow.
        assert!(app.has_document());
        assert!(!app.panel_follows());
        assert_eq!(app.panel_scroll_offset(), 0);
        assert_eq!(panel_text(&app, now), ["line 0", "line 1", "line 2"]);
        // And the count agrees with what was handed over: five lines, three of
        // them on screen.
        assert_eq!(app.panel_lines_below(), 5 - usize::from(PANEL));

        // A panel tall enough for the whole file draws the whole file, in order,
        // with nothing below it.
        app.set_panel_height(9);
        assert_eq!(panel_text(&app, now), document_lines());
        assert_eq!(app.panel_lines_below(), 0);
    }

    #[test]
    fn a_read_the_cap_cut_short_says_so_under_the_last_line_it_got() {
        let now = Instant::now();
        let mut app = App::from_tree(&fixture::tree());
        app.set_panel_height(9);

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

    /// A document with one line in it far too long for [`NARROW`], and two short
    /// ones either side of it, so a test can say which rows came from wrapping.
    fn a_long_line() -> Vec<String> {
        [
            "# The engine",
            "It walks the tree and writes what it finds.",
            "done",
        ]
        .map(str::to_owned)
        .to_vec()
    }

    /// A panel narrow enough that the long line above needs three rows of it.
    const NARROW: u16 = 18;

    #[test]
    fn a_document_line_wider_than_the_panel_is_drawn_in_as_many_rows_as_it_needs() {
        let now = Instant::now();
        let mut app = App::from_tree(&fixture::tree());
        app.set_panel_height(9);
        app.set_panel_width(NARROW);

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
        assert_eq!(app.panel_lines_below(), 0);
    }

    #[test]
    fn a_panel_nobody_has_measured_wraps_nothing() {
        // What an app is between being built and being drawn: every frame tells
        // it the width, and until one has, a line is the row it arrived as.
        let now = Instant::now();
        let mut app = App::from_tree(&fixture::tree());
        app.set_panel_height(9);

        app.show_document(a_long_line(), false);

        assert_eq!(app.panel_width(), 0);
        assert_eq!(panel_text(&app, now), a_long_line());
    }

    #[test]
    fn the_rows_a_document_draws_as_follow_the_width_the_panel_was_last_told() {
        // The reader drags the terminal narrower with a file up: the document
        // re-flows to the width of the frame it is drawn in, from the lines it
        // has always held, without anything being read again.
        let now = Instant::now();
        let mut app = App::from_tree(&fixture::tree());
        app.set_panel_height(9);
        app.set_panel_width(80);
        app.show_document(a_long_line(), false);
        assert_eq!(panel_text(&app, now), a_long_line());

        app.set_panel_width(NARROW);

        assert_eq!(panel_text(&app, now).len(), 5);
        // And wider again is the document it was: nothing was lost on the way
        // through the narrow panel, because the lines held are the file's.
        app.set_panel_width(80);
        assert_eq!(panel_text(&app, now), a_long_line());
    }

    #[test]
    fn the_panel_scrolls_by_rows_of_a_wrapped_document_rather_than_by_its_lines() {
        let now = Instant::now();
        let mut app = App::from_tree(&fixture::tree());
        app.set_panel_height(2);
        app.set_panel_width(NARROW);
        app.show_document(a_long_line(), false);

        // Five rows in a panel two tall: what is below the window is counted in
        // rows, so a reader scrolling past a wrapped line scrolls through it.
        assert_eq!(app.panel_lines_below(), 3);
        assert_eq!(panel_text(&app, now), ["# The engine", "It walks the tree"]);

        app.scroll_panel_down(1);

        assert_eq!(app.panel_scroll_offset(), 1);
        assert_eq!(
            panel_text(&app, now),
            ["It walks the tree", "and writes what it"]
        );
        assert_eq!(app.panel_lines_below(), 2);
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
        app.set_panel_height(9);
        let account = app.account_mut().expect("a run has just started");
        account.record(
            &Activity::Tool {
                name: "Read".to_owned(),
                detail: Some("crates/warlock-engine/src/pact.rs".to_owned()),
            },
            at(base, 1),
        );

        // Nobody has measured the panel yet: two lines, two rows.
        assert_eq!(app.panel_width(), 0);
        assert_eq!(panel_text(&app, at(base, 9)).len(), 2);

        app.set_panel_width(NARROW);

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
        assert_eq!(app.panel_lines_below(), 0);

        // And wider again is the account it always was: what a card holds is
        // what happened, never the rows some width once broke it into.
        app.set_panel_width(80);
        assert_eq!(panel_text(&app, at(base, 9)).len(), 2);
    }

    /// Where the account's card would be if the reader swapped to it: the line
    /// at the top of its window, and whether it is still following its newest
    /// one.
    ///
    /// Asked of the card rather than of [`App::panel_scroll_offset`], because
    /// the point of every test below is what the card that is *not* showing is
    /// doing.
    fn account_window(app: &App) -> (usize, bool) {
        (
            app.panel
                .account
                .scroll_offset(app.panel.height, app.panel.width),
            app.panel.account.follows,
        )
    }

    /// The same of the thread's card.
    fn thread_window(app: &App) -> (usize, bool) {
        (
            app.panel
                .thread
                .scroll_offset(app.panel.height, app.panel.width),
            app.panel.thread.follows,
        )
    }

    /// The same of the document's card.
    fn document_window(app: &App) -> (usize, bool) {
        (
            app.panel
                .document
                .scroll_offset(app.panel.height, app.panel.width),
            app.panel.document.follows,
        )
    }

    /// Every line the document's card holds, whole, whatever the panel's window
    /// is over and whichever card is showing.
    ///
    /// What a snapshot is asserted with: [`panel_text`] is the window a reader
    /// would see, and this is the card underneath it, so a test can say that the
    /// lines themselves never changed rather than that a cut of them looked the
    /// same.
    fn document_text(app: &App) -> Vec<String> {
        app.panel
            .document
            .held
            .iter()
            .flatten()
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
        assert!(app.has_account());

        app.show_document(document_lines(), false);

        // Two cards in one slot: the document is what is drawn, and the account
        // is still there behind it — held, handed out by the getters, and with
        // its own window exactly where it was.
        assert!(app.has_document());
        assert!(app.has_account());
        assert_eq!(app.account().map(Account::line_count), Some(10));
        assert!(app.account_mut().is_some());
        assert!(app.has_panel_content());
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
        assert!(app.has_document());
        assert_eq!(panel_text(&app, at(base, 100)), shown);
        assert_eq!(app.panel_scroll_offset(), 0);
        assert!(!app.panel_follows());
        assert!(app.has_account());
        assert_eq!(app.account().map(Account::line_count), Some(0));
        assert_eq!(account_window(&app), (0, true));

        // And the run goes on reporting into that card while the reader reads,
        // with its window following the newest line for when they swap to it.
        let account = app.account_mut().expect("the run has its own card");
        account.open_section("crates/tui", at(base, 100));
        for line in 0..9 {
            account.record(&Activity::Thinking, at(base, 101 + line));
        }
        assert_eq!(app.account().map(Account::line_count), Some(2));
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
        assert_eq!(app.panel_scroll_offset(), 1);
        assert_eq!(account_window(&app), (1, false));

        // Including when the movement takes the document to its own end, which
        // is the one thing that sets a follow flag.
        app.select_last();
        assert_eq!(document_window(&app), (5 - usize::from(PANEL), true));
        assert!(app.panel_follows());
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
                app.panel_scroll_offset(),
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
        app.set_panel_height(PANEL);

        app.show_document(Vec::<String>::new(), false);

        // An empty file read is not the same as nothing having happened: the
        // panel is holding it, so the mark does not come back.
        assert!(app.has_document());
        assert!(app.has_panel_content());
        assert_eq!(app.panel_lines(now), Vec::new());
        assert_eq!(app.panel_lines_below(), 0);
        assert_eq!(app.panel_scroll_offset(), 0);
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
        assert_eq!(app.panel.showing, Showing::Thread);
        assert!(!app.has_panel_content(), "nothing has been asked");

        app.swap_card();
        assert_eq!(panel_text(&app, at(base, 9)), account);
        assert!(app.has_panel_content());

        app.swap_card();
        assert_eq!(panel_text(&app, at(base, 9)), document);
        assert!(app.has_panel_content());
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
        before.panel.showing = Showing::Thread;
        assert_eq!(app, before, "the swap moved something other than the card");
    }

    /// The file the tests below re-read, as whoever read it again would hand it
    /// over: the same document rewritten by somebody else's editor, so that a
    /// card holding the new lines cannot be mistaken for one holding the old.
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
        assert_eq!(app.panel.showing, Showing::Account);
        let account = panel_text(&app, at(base, 9));

        app.refill_document(rewritten_lines(), false);

        // The panel is exactly where they left it. The new lines are on the card
        // behind it, waiting for the swap they will ask for themselves — a file
        // being saved in an editor is not a reason to take a run off the screen.
        assert_eq!(panel_text(&app, at(base, 9)), account);
        assert!(app.has_document());
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
        assert!(app.panel_follows());
        let parked = account_window(&app);

        app.refill_document(rewritten_lines(), false);

        // Still the card on screen — nothing about which card is showing moved —
        // and showing the new file from its first line: line five of what was
        // there is not line five of what is there now.
        assert_eq!(panel_text(&app, at(base, 9)), rewritten_lines());
        assert_eq!(app.panel_scroll_offset(), 0);
        assert!(!app.panel_follows());
        // And the account behind it is where it was, still following its own
        // newest line.
        assert_eq!(account_window(&app), parked);
    }

    #[test]
    fn a_re_read_the_cap_cut_short_says_so_exactly_as_the_first_read_did() {
        let now = Instant::now();
        let mut app = App::from_tree(&fixture::tree());
        app.set_panel_height(9);
        app.show_document(document_lines(), false);

        app.refill_document(document_lines(), true);

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

        app.refill_document(rewritten_lines(), false);

        // The whole of what changed is what is on the one card: the account, the
        // focus, the selection, the tree's window, the bit saying which card is
        // drawn and the footer are all the app's own clone, untouched.
        before.panel.document.place(
            rewritten_lines()
                .into_iter()
                .map(|text| Line::Text { text })
                .collect(),
            false,
        );
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
        assert_eq!(app.panel.showing, Showing::Account);
        assert_eq!(app.panel_scroll_offset(), parked.0);
        assert_eq!(panel_text(&app, at(base, 9))[0], "Read line 0");

        // And back: the document is on the line they left it on.
        app.swap_card();
        assert_eq!(document_window(&app), left);
        assert_eq!(app.panel_scroll_offset(), left.0);
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
        assert!(app.panel_follows());
        assert_eq!(drawn.last().expect("the account has lines"), "Read line 11");

        // Parked: the reader put that window where it is, and lines arriving
        // behind a document do not take it off the line they chose.
        let mut app = app_pacting(9, base);
        app.select_first();
        app.show_document(document_lines(), false);
        record_lines(&mut app, 9..12, base);

        app.swap_card();
        app.swap_card();
        assert!(!app.panel_follows());
        assert_eq!(app.panel_scroll_offset(), 0);
        assert_eq!(panel_text(&app, at(base, 20))[0], "crates/engine");
    }

    /// Record one numbered tool line per second of `lines` on `app`'s account,
    /// the way [`app_pacting`] fills one, so a test can go on writing a run that
    /// started before it.
    fn record_lines(app: &mut App, lines: std::ops::Range<u64>, base: Instant) {
        let account = app.account_mut().expect("a run has started");
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
        app.set_panel_height(9);
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
        app.account_mut()
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
            app.account().map(Account::line_count),
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
        app.set_panel_height(PANEL);
        app.set_message("something the last keystroke said");
        let mut before = app.clone();

        app.swap_card();

        // The mark stays up, the conversation stays showing, and the refusal is
        // about the document because the document is the card a keystroke can
        // make. The message is the whole of what the press changed.
        let message = app.message().expect("a swap with nothing read is refused");
        assert_eq!(message, no_document_message(), "{message}");
        assert!(message.contains("press v"), "{message}");
        assert_eq!(app.panel.showing, Showing::Thread);
        assert!(!app.has_panel_content());
        before.set_message(message);
        assert_eq!(app, before, "the refusal moved something else");
    }

    /// What the thread tests ask.
    const QUESTION: &str = "what does the engine do?";
    /// What comes back: prose, and long enough that [`NARROW`] has to break it
    /// over three rows — the same sentence [`a_long_line`] wraps, so the two
    /// tests agree about what wrapping does.
    const ANSWER: &str = "It walks the tree and writes what it finds.";

    /// Put one whole turn on `app`'s thread: the question a second past `base`,
    /// three tool calls under it, then the answer.
    ///
    /// Five rows, so a [`PANEL`]-tall window over it has a top and a bottom that
    /// are not the same row, and every row says which one it is.
    fn ask_and_answer(app: &mut App, base: Instant) {
        app.start_turn(QUESTION, at(base, 1));
        for line in 0..3 {
            app.record_turn(
                &Activity::Tool {
                    name: "Grep".to_owned(),
                    detail: Some(format!("thread line {line}")),
                },
                at(base, line + 2),
            );
        }
        app.answer_turn(ANSWER, at(base, 5));
    }

    #[test]
    fn the_swap_key_goes_round_all_three_cards_and_leaves_none_unreachable() {
        let base = Instant::now();
        let mut app = app_pacting(9, base);
        let account = panel_text(&app, at(base, 9));

        // A question brings the thread to the front by itself, the way a read
        // brings the document; both cards are filled behind whatever is drawn.
        ask_and_answer(&mut app, base);
        assert!(app.has_thread());
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
        assert!(!app.has_document());

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
        app.set_panel_height(PANEL);
        ask_and_answer(&mut app, base);
        let thread = panel_text(&app, at(base, 9));
        assert!(!app.has_account(), "no pact has run this session");

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
        app.set_panel_height(PANEL);
        app.show_document(document_lines(), false);
        assert!(!app.composer_showable());

        app.swap_card();

        assert!(app.composer_showable());
        assert!(!app.has_panel_content(), "the account is the empty one");
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
        assert!(app.has_account());
        assert!(app.has_thread());
        assert!(app.has_document());
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
        assert!(!app.panel_follows());
        assert_eq!(app.panel_scroll_offset(), 0);
        app.swap_card();
        assert!(app.panel_follows());
        assert_eq!(app.panel_scroll_offset(), 2);
        app.swap_card();
        assert!(!app.panel_follows());
        assert_eq!(app.panel_scroll_offset(), 0);

        // A turn already under way goes on being answered behind a document,
        // and the document does not move an inch while it is. Asking brings the
        // conversation to the front, so the file is two presses back.
        app.start_turn("and how long does it take?", at(base, 6));
        assert_eq!(app.panel.showing, Showing::Thread);
        app.swap_card();
        app.swap_card();
        assert_eq!(app.panel.showing, Showing::Document);
        assert_eq!(
            panel_text(&app, at(base, 9)),
            ["line 0", "line 1", "line 2"]
        );
        app.answer_turn("About a second.", at(base, 7));
        assert_eq!(
            panel_text(&app, at(base, 9)),
            ["line 0", "line 1", "line 2"]
        );

        // And the thread is at its newest row the moment the reader swaps back
        // to it, exactly as a run left following is.
        app.swap_card();
        assert_eq!(app.panel.showing, Showing::Thread);
        assert!(app.panel_follows());
        assert_eq!(
            panel_text(&app, at(base, 9)).last().map(String::as_str),
            Some("About a second.")
        );
    }

    #[test]
    fn every_line_wider_than_the_panel_is_drawn_in_as_many_rows_as_it_needs() {
        let base = Instant::now();
        let mut app = app_pacting(11, base);
        app.set_panel_height(11);
        ask_and_answer(&mut app, base);

        // Nobody has measured the panel yet, so nothing is wrapped: five lines,
        // five rows.
        assert_eq!(app.panel_width(), 0);
        assert_eq!(panel_text(&app, at(base, 5)).len(), 5);

        app.set_panel_width(NARROW);

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
        assert_eq!(app.panel_lines_below(), 0);

        // The window is cut out of those rows and not out of the lines: a panel
        // three tall over eleven rows shows the last three and has eight above
        // them, and a reader who goes back to the top has those eight below.
        app.set_panel_height(PANEL);
        assert_eq!(app.panel_scroll_offset(), 8);
        assert_eq!(app.panel_lines_below(), 0);
        app.select_first();
        assert_eq!(app.panel_lines_below(), 8);

        // And wider again is the answer it always was — the lines held are the
        // model's own, so a terminal dragged about re-flows what is on screen
        // rather than asking anything again.
        app.set_panel_height(9);
        app.set_panel_width(80);
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
        assert!(!app.has_thread());
        assert!(app.thread().is_none());
        app.record_turn(&Activity::Thinking, base);
        app.answer_turn("nobody asked", base);
        assert!(!app.has_thread());

        ask_and_answer(&mut app, base);
        app.start_turn("and what did that cost?", at(base, 6));
        app.record_turn(&Activity::Cost { usd: 0.02 }, at(base, 7));
        app.end_turn(&Ending::Cancelled, at(base, 8));

        // One session, one conversation: the second turn is under the first and
        // the first is exactly as it was answered.
        let thread = app.thread().expect("a question has been asked");
        assert_eq!(thread.turns().len(), 2);
        assert_eq!(thread.turns()[0].message(), QUESTION);
        assert_eq!(thread.turns()[0].answer(), Some(ANSWER));
        assert_eq!(thread.turns()[1].message(), "and what did that cost?");
        assert_eq!(thread.turns()[1].ending(), Some(&Ending::Cancelled));

        // A second ending is the first one still: the line on screen wins.
        app.end_turn(&Ending::NothingSaid, at(base, 9));
        let thread = app.thread().expect("a question has been asked");
        assert_eq!(thread.turns()[1].ending(), Some(&Ending::Cancelled));

        // And the run behind all of it never heard a word: a conversation is
        // not a pact.
        assert_eq!(app.account().map(Account::line_count), Some(10));
    }

    #[test]
    fn a_note_brings_the_conversation_forward_and_costs_nobody_a_turn() {
        const REFUSED: &str = "commands are /brief, /write and /chat, and take nothing after them";

        let base = Instant::now();
        let mut app = app_pacting(9, base);
        // A file on screen, and not one question asked: the note has to make
        // the card as well as land on it.
        app.show_document(document_lines(), false);
        assert_eq!(app.panel.showing, Showing::Document);
        assert!(!app.has_thread());

        app.note(REFUSED, base);

        // The conversation comes to the front the way a question brings it: the
        // line answers what the reader just typed, and an answer on a card they
        // are not looking at is not an answer.
        assert_eq!(app.panel.showing, Showing::Thread);
        assert!(app.has_thread());
        assert_eq!(panel_text(&app, at(base, 9)), [REFUSED]);

        // And it is warlock's own line, not a turn: nobody was asked anything,
        // so nothing is in flight and the composer is free.
        let thread = app.thread().expect("the note made the card");
        assert!(thread.turns().is_empty());
        assert!(thread.in_flight().is_none());
        assert_eq!(thread.line_count(), 1);

        // The card accumulates, as it does for a question: what is said next
        // goes under the note rather than in place of it.
        ask_and_answer(&mut app, base);
        app.note("wrote docs/brief.md", at(base, 6));

        let lines = app
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
        assert_eq!(app.thread().map(|thread| thread.turns().len()), Some(1));

        // The other two cards are exactly as they were left, lines and all.
        assert!(app.has_account());
        assert!(app.has_document());
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
        assert!(reseated.has_thread());
        assert_eq!(
            reseated.thread().map(|thread| thread.turns().len()),
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
        assert!(app.has_thread());
        assert_eq!(app.thread().map(|thread| thread.turns().len()), Some(1));
        assert_eq!(panel_text(&app, at(base, 9)), thread);
    }

    /// A whole run driven through `app` from `at(base, from)`, event by event
    /// and through the one call a run's events go through: the account starts,
    /// one directory is worked and says what it wrote, and the run finishes.
    ///
    /// Six rows of account — a heading, three activities, an outcome and a
    /// summary — so a window [`PANEL`] rows tall has a top and a bottom that are
    /// nowhere near each other, and every row says which one it is.
    fn run_a_pact(app: &mut App, base: Instant, from: u64) {
        app.start_account(at(base, from));
        app.write_run(|account| account.open_section("crates/tui", at(base, from)));
        for line in 0..3 {
            app.write_run(|account| {
                account.record(
                    &Activity::Tool {
                        name: "Read".to_owned(),
                        detail: Some(format!("run line {line}")),
                    },
                    at(base, from + line + 1),
                );
            });
        }
        app.write_run(|account| {
            account.close_section(
                &Outcome::Wrote {
                    document: PathBuf::from("crates/tui").join("WARLOCK.md"),
                    bytes: 120,
                },
                at(base, from + 4),
            );
        });
        app.write_run(|account| account.finish(at(base, from + 5)));
    }

    #[test]
    fn a_run_started_on_the_thread_puts_nothing_in_the_conversation() {
        let base = Instant::now();
        let mut app = app_pacting(9, base);
        ask_and_answer(&mut app, base);
        assert!(app.showing_thread());
        let asked = panel_text(&app, at(base, 9));

        run_a_pact(&mut app, base, 10);

        // The card the reader was on is the card they are on, drawing exactly
        // what it drew: a pact has a card of its own, and a conversation that
        // also carried it would be the same run written twice on one screen.
        assert!(app.showing_thread(), "the run took the panel");
        assert_eq!(panel_text(&app, at(base, 20)), asked);
        let thread = app.thread().expect("a question has been asked");
        assert_eq!(thread.turns().len(), 1, "the run took a turn");
        assert_eq!(thread.turns()[0].message(), QUESTION);
        assert_eq!(thread.turns()[0].answer(), Some(ANSWER));
        // And nothing is in flight, so the field is the reader's: a run is not
        // a question and does not mute the composer.
        assert!(thread.in_flight().is_none());

        // The run is all on its own card, whole.
        assert_eq!(app.account().map(Account::line_count), Some(6));
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
        app.write_run(|account| account.open_section("crates/tui", at(base, 10)));
        let mut seen = Vec::new();
        for (line, now) in [(0, 11), (1, 12), (2, 13)] {
            app.write_run(|account| {
                account.record(
                    &Activity::Tool {
                        name: "Read".to_owned(),
                        detail: Some(format!("run line {line}")),
                    },
                    at(base, now),
                );
            });
            let card = app.account().expect("the run started its own card");
            seen.push(card.line_count());
            assert_eq!(panel_text(&app, at(base, 20)), asked);
        }
        // Something did arrive on each of the three passes, so the equality
        // above is about a card that was moving.
        assert_eq!(seen, [2, 3, 4]);

        // And the outcome and the summary are the same story: the account says
        // how the run went, and the thread still says what was asked.
        app.write_run(|account| {
            account.close_section(
                &Outcome::Refused {
                    reason: "the model would not".to_owned(),
                },
                at(base, 14),
            );
            account.finish(at(base, 15));
        });
        let card = app.account().expect("the run started its own card");
        assert!(
            card.lines(at(base, 30))
                .iter()
                .any(|line| matches!(line, Line::Summary { .. })),
            "the run never said how it went"
        );
        assert_eq!(panel_text(&app, at(base, 30)), asked);
        assert_eq!(
            app.thread().map(|thread| thread.turns().len()),
            Some(1),
            "the run took a turn of the conversation"
        );
    }

    #[test]
    fn a_run_with_no_conversation_behind_it_conjures_no_thread() {
        let base = Instant::now();
        let mut app = app_pacting(9, base);
        assert!(!app.has_thread());

        run_a_pact(&mut app, base, 10);

        // Nothing typed, no conversation held: a run fills its own card and
        // nothing else, and it took the panel because the panel had nothing on
        // it — the mark is not worth more than the run the reader just asked
        // for.
        assert!(!app.has_thread());
        assert!(app.thread().is_none());
        assert_eq!(app.panel.showing, Showing::Account);
        assert_eq!(app.account().map(Account::line_count), Some(6));

        // The conversation is still a card to swap to, empty or not: it is
        // where the field is, so a reader who wants to ask something about the
        // run they are watching is one press away from being able to.
        app.swap_card();
        assert_eq!(app.panel.showing, Showing::Thread);
        assert!(!app.has_panel_content(), "nothing has been asked");
        assert!(app.composer_showable(), "the field came with it");
        assert!(app.message().is_none(), "a swap that worked said something");

        // With no document read the key goes between those two and is never
        // refused: there is always somewhere to go.
        app.swap_card();
        assert_eq!(app.panel.showing, Showing::Account);
        app.swap_card();
        assert_eq!(app.panel.showing, Showing::Thread);
        assert!(app.message().is_none(), "a swap that worked said something");

        // With a document read the cycle is all three, in order.
        app.show_document(document_lines(), false);
        assert_eq!(app.panel.showing, Showing::Document);
        app.swap_card();
        assert_eq!(app.panel.showing, Showing::Thread);
        app.swap_card();
        assert_eq!(app.panel.showing, Showing::Account);
        app.swap_card();
        assert_eq!(app.panel.showing, Showing::Document);
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
        assert_eq!(app.panel.showing, Showing::Document);
        assert_eq!(panel_text(&app, at(base, 20)), document);
        assert_eq!(app.account().map(Account::line_count), Some(6));
        assert_eq!(
            app.thread().map(|thread| thread.turns().len()),
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
        assert_eq!(app.panel.showing, Showing::Account);
        assert_eq!(app.account().map(Account::line_count), Some(6));
        assert_eq!(account_window(&app), (6 - usize::from(PANEL), true));
        assert_eq!(panel_text(&app, at(base, 20))[0], "Read run line 2");

        app.swap_card();
        assert_eq!(app.panel.showing, Showing::Thread);
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
        assert!(app.showing_thread());
        assert_eq!(panel_text(&app, at(base, 20)), thread);
        assert_eq!(app.account().map(Account::line_count), Some(6));
        let held = app.thread().expect("a question has been asked");
        assert_eq!(held.turns().len(), 1);
        assert_eq!(held.turns()[0].answer(), Some(ANSWER));
    }

    #[test]
    fn a_run_under_way_when_a_question_is_asked_files_nothing_under_it() {
        let base = Instant::now();
        let mut app = app_pacting(9, base);

        // The run is under way, and then somebody asks something.
        app.start_account(at(base, 10));
        app.start_turn(QUESTION, at(base, 11));
        app.write_run(|account| account.open_section("crates/tui", at(base, 12)));
        app.write_run(|account| account.finish(at(base, 13)));

        // The run has a card and the question has a turn: a line filed under
        // the question would put a pact's work under somebody's sentence.
        assert_eq!(app.account().map(Account::line_count), Some(3));
        let thread = app.thread().expect("a question has been asked");
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
        let turns = app.thread().map(|thread| thread.turns().len());

        // A late event — a worker that reported after its outcome landed — is
        // dropped by the account, which is finished and says so on screen. The
        // conversation hears nothing either, as it heard nothing about the rest
        // of the run.
        app.write_run(|account| account.record(&Activity::Thinking, at(base, 21)));

        assert_eq!(app.thread().map(|thread| thread.turns().len()), turns);
        assert_eq!(panel_text(&app, at(base, 30)), thread);
        assert_eq!(app.account().map(Account::line_count), Some(6));
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
            assert!(!app.has_panel_content(), "{path} drew something");
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
            assert!(!app.has_panel_content(), "{path} drew something");
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
        assert!(!app.has_panel_content(), "the refusal drew something");
    }

    #[test]
    fn an_app_with_no_rows_views_nothing() {
        let mut app = App::from_rows(Vec::new());

        assert_eq!(app.view_target(), None);
        assert_eq!(app.message(), None);
        assert!(!app.has_panel_content());
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
        assert_eq!(reseated.panel.showing, Showing::Document);
        assert!(reseated.has_document());
        assert_eq!(panel_text(&reseated, at(base, 9)), showing);
        assert_eq!(document_window(&reseated), document_window(&app));
        // And the account is intact behind it, lines, window and all.
        assert!(reseated.has_account());
        assert_eq!(reseated.account(), app.account());
        assert_eq!(account_window(&reseated), account_window(&app));
        // The card is a snapshot: the reload rewrote none of its lines, and the
        // swap still comes back to the whole of what was read.
        assert_eq!(document_text(&reseated), document_lines());
        let mut reseated = reseated;
        for _ in 0..3 {
            reseated.swap_card();
        }
        assert_eq!(reseated.panel.showing, Showing::Document);
        assert_eq!(document_text(&reseated), document_lines());
        assert_eq!(panel_text(&reseated, at(base, 9)), showing);
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

    /// The fixture with its files shown, which is the view every file an
    /// insertion names is visible in.
    fn app_with_files() -> App {
        let mut app = App::from_tree(&fixture::tree());
        app.toggle_files();
        app
    }

    /// `app` with the row for `path` calling itself the document of the
    /// directory holding it, in the walk and in the drawn rows both.
    ///
    /// The one field a load and an insertion disagree about, and they disagree on
    /// purpose. A load reads the tree, where `index.html` is an ordinary file
    /// nobody documented anything with; an insertion is the pact observer's word
    /// that the file it names is the document the pass has just written, which is
    /// a fact no tree the app can see holds yet (see `App::insert_file_row`). So
    /// a reloaded app is brought into line by marking the row the insertion
    /// marked, rather than by leaving that difference unasserted.
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
