//! What one frame looks like.
//!
//! The screen is a footer along the bottom and two panes above it: the panel on
//! the left, and on the right a bordered column carrying a header naming which
//! tree is on screen and the flattened tree itself, one node per line. The
//! footer runs the full width under both of them with the tally, the keys and
//! whatever the app has to say about the last keystroke — or about the pact it
//! is running, which takes that line and changes the one above it for as long as
//! it runs. [`draw`] takes a frame, the app state and the instant the frame is
//! being drawn at, and nothing else — no terminal setup, no globals, no clock of
//! its own, no reaching back into the engine — so what appears on screen is a
//! pure function of what the app state says and what time the caller says it is,
//! and a test can assert it against an in-memory buffer with no tty attached.
//!
//! The panel is the account of the pact that is running, or of the last one to
//! run: a heading naming each directory and, under it, one row per thing that
//! pass was seen doing, each with the elapsed clock of its own section. Before
//! the first pact of the session there is no account, and what it holds instead
//! is warlock's mark: the program's name in shaded blocks, compiled in, centred
//! and dim, and gone for good the moment a pact starts. Not a word of text goes
//! with it — a screen saying something before anything has happened would be
//! saying it about nothing, where a mark only says whose screen this is. A panel
//! with no room for the whole mark draws the bare border, which on a narrow
//! terminal is every panel there is. Nothing on any card runs off the
//! right-hand edge: a line too long for the panel is broken into the rows it
//! needs, under its own clock or marker, and none of that happens here — every
//! card arrives already in the rows its width needs (see [`mod@crate::wrap`]),
//! so this module draws the rows it is handed. See [`draw_panel`].
//!
//! Under the panel, in the panel's own column, is the composer: the draft
//! somebody is typing, in a bordered box of its own. It is one row tall while
//! there is nothing in it and grows a row per newline and per wrapped line up to
//! a cap, past which it scrolls within itself and keeps the cursor's row on
//! screen (see [`mod@crate::composer`]). Every row it takes is a row the panel
//! above it does not have — [`areas`] cuts one column into the two of them, and
//! [`panel_height`] and [`composer_height`] are that one cut read twice — and
//! while the panel is showing a document it is not drawn at all and the panel
//! has the column back. Its border is lit by [`pane_block`] like either pane's,
//! which is what makes three places for the keyboard readable as three.
//!
//! The tree area is a window onto the flattened rows: it draws the slice
//! starting at the app's scroll offset and running for as many rows as the area
//! is tall, so a tree taller than the terminal scrolls under a header and a
//! footer that stay where they are. The window is the app's, not the widget's —
//! see [`draw_tree`]. Which rows exist at all is the app's too: collapsing a
//! directory takes its descendants out of [`App::rows`], and whatever rows the
//! app holds are the rows drawn — this module only says which of them is
//! collapsed, with a marker on the line, and leads each one in with the guides
//! [`guide_prefixes`] works out for the window.
//!
//! A pointer is answered by measuring, not by remembering: [`hit_test`] takes a
//! screen column and row and says which of these areas they landed on — the
//! footer, a border, the header, a row of the tree's window, a line of the
//! panel's — off the very [`areas`] call the frame is cut by, so a click lands
//! on what the reader saw there. It is a function of three numbers and knows
//! nothing about the tree, the selection or the window's contents: what a row
//! offset means is the app's to say. Nothing here follows the pointer around,
//! and there is deliberately no hover.
//!
//! One thing is drawn over all of that, and only one: the gate on the way out.
//! When [`QuitConfirm`] is up, [`draw`] finishes the frame it would have drawn
//! anyway and then puts a small window in the middle of the terminal, with the
//! cells behind it cleared — see [`draw_confirm`]. The question is handed in
//! beside the app rather than read off it, because the app has never heard of it
//! (see [`mod@crate::confirm`]), and it is one parameter of the one drawing
//! function rather than a second entry point, so the binary still makes one call
//! per frame and a test still asserts one buffer.

use std::time::{Duration, Instant};

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Position, Rect, Size};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, List, ListItem, ListState, Padding, Paragraph};
use warlock_engine::{NodeState, scope};

// Renamed on the way in: `Line` here is ratatui's, the thing a row is drawn as,
// and the account's `Line` is what a row says. Both names are right where they
// live, and this module is the one place both are in scope.
use crate::account::{Account, Line as Entry};
use crate::app::{App, Chrome, Focus, Row, Run, RunHeader};
use crate::colour::{FOCUS_COLOUR, GUIDE_COLOUR, colour_for};
use crate::composer::Composer;
use crate::confirm::{Answer, QuitConfirm};
use crate::panel::Mode;
use crate::prompt::{ScopeField, ScopePrompt};
use crate::wrap::shape;

/// One level of nesting, per unit of the depth the engine's walk yields.
const INDENT: &str = "  ";

/// The stroke a guide is drawn with where the branch it stands for carries on
/// below this row: one per level of nesting, in the first of that level's
/// [`INDENT`] columns.
///
/// Unicode where the markers are held to ASCII, because ratatui already draws
/// this pane's border in box-drawing characters — a terminal that cannot manage
/// this one is already drawing a mess of the frame around it.
const GUIDE: &str = "│";

/// The guide on the row itself, where the directory holding it has more rows
/// under it after this one.
///
/// The tee and [`GUIDE_LAST`] are what make the guides say something the
/// verticals alone could not: where a subtree stops. Reading down a column of
/// plain bars, the eye has to count indents to find out whether the next row is
/// a sibling or an aunt; the corner says it in the glyph. Costing a
/// last-sibling pass to work out, which is why it is worked out once for the
/// drawn rows rather than asked per row — see [`guide_prefixes`].
///
/// Nothing reaches from the corner across to the row it belongs to, and the
/// horizontal that would is deliberately not drawn. [`EXPANDED_MARKER`] is a
/// hyphen sitting at the same height as such a line and two columns along from
/// it, so an arm would run into the marker on a directory row and stop short on
/// a file row — leaving the right-hand end of the line jumping a column back and
/// forth down a column of siblings. The rest of that level stays blank: nothing
/// can merge with a space.
const GUIDE_BRANCH: &str = "├";

/// The guide on the last row a directory holds, where the branch stops.
const GUIDE_LAST: &str = "└";

/// Drawn to the left of the selected row. The reversed highlight already says
/// where the selection is on any terminal with colour; the marker says it
/// again for the ones without.
const SELECTION_MARKER: &str = "> ";

/// Drawn on a directory whose children are hidden, between its indent and its
/// name.
///
/// Plus and minus rather than arrows or triangles: the selection marker is
/// already plain ASCII for the terminals and fonts that would make a mess of
/// anything else, and a marker that renders as a box on such a terminal says
/// less than no marker at all.
const COLLAPSED_MARKER: &str = "+ ";

/// Drawn on a directory whose children are on screen. See [`COLLAPSED_MARKER`].
const EXPANDED_MARKER: &str = "- ";

/// Drawn on a node with no children: nothing, in the width of a marker.
///
/// A node with nothing under it is neither collapsed nor expanded and says so
/// by carrying no marker — an empty directory must not look like one hiding
/// something. It still takes the marker's two columns, because a name that
/// slid left when a directory turned out to be empty would put the siblings of
/// one parent at two different indents and undo what the indent is for.
const NO_MARKER: &str = "  ";

/// Warlock's mark: the program's name set in shaded blocks, one row per line,
/// drawn in the panel while there is no account to put there.
///
/// The name rather than the logo of `assets/warlock-logo.png`. A `W` alone asks
/// the reader to already know whose `W` it is, and the panel it sits in is the
/// first thing a new reader looks at; the word answers the question the mark is
/// there to answer.
///
/// Compiled in as text. Nothing is read off the disk to draw it: an image on
/// disk would be a file the program had to find, a decoder to read it with and a
/// terminal that could show it, and the mark is the one thing on screen that
/// must be there before anything has happened. Being a constant also means it is
/// the same mark in a test buffer as on a terminal, which is how
/// [`mod@crate::ui`] asserts anything at all.
///
/// Block elements rather than plain ASCII, which is a raised ceiling and a
/// deliberate one. Everywhere else this crate holds itself to what
/// [`SELECTION_MARKER`] and [`COLLAPSED_MARKER`] are held to, and the highest it
/// goes is the box drawing ratatui already puts in the panes' borders and in
/// [`GUIDE`]. `█` and `▒` are one block along from that in Unicode and rather
/// older in practice — both are in the original IBM PC character set — so a
/// terminal that can draw a border can draw these. The shading is what the mark
/// is: a solid face with a lighter one falling away behind it, which is a
/// letterform no single-weight character set can carry.
///
/// A hundred columns wide, which is the size the shape needs and is wider than
/// this panel is on an ordinary terminal. It is drawn on a wide one and not on a
/// narrow one — see [`mark_area`], which draws it whole or not at all — and that
/// is the trade taken knowingly: warlock puts a file tree beside this panel, so
/// the terminal it is run in is a wide one or the tree has nowhere to be either.
const MARK: &[&str] = &[
    " █████   ███   █████   █████████   ███████████   █████          ███████      █████████  █████   ████",
    "▒▒███   ▒███  ▒▒███   ███▒▒▒▒▒███ ▒▒███▒▒▒▒▒███ ▒▒███         ███▒▒▒▒▒███   ███▒▒▒▒▒███▒▒███   ███▒",
    " ▒███   ▒███   ▒███  ▒███    ▒███  ▒███    ▒███  ▒███        ███     ▒▒███ ███     ▒▒▒  ▒███  ███",
    " ▒███   ▒███   ▒███  ▒███████████  ▒██████████   ▒███       ▒███      ▒███▒███          ▒███████",
    " ▒▒███  █████  ███   ▒███▒▒▒▒▒███  ▒███▒▒▒▒▒███  ▒███       ▒███      ▒███▒███          ▒███▒▒███",
    "  ▒▒▒█████▒█████▒    ▒███    ▒███  ▒███    ▒███  ▒███      █▒▒███     ███ ▒▒███     ███ ▒███ ▒▒███",
    "    ▒▒███ ▒▒███      █████   █████ █████   █████ ███████████ ▒▒▒███████▒   ▒▒█████████  █████ ▒▒████",
    "     ▒▒▒   ▒▒▒      ▒▒▒▒▒   ▒▒▒▒▒ ▒▒▒▒▒   ▒▒▒▒▒ ▒▒▒▒▒▒▒▒▒▒▒    ▒▒▒▒▒▒▒      ▒▒▒▒▒▒▒▒▒  ▒▒▒▒▒   ▒▒▒▒",
];

/// The clear space the mark wants on either side of it, and the least the panel
/// can be wider than the art and still draw it.
///
/// Two columns, so the mark reads as a mark inside a border rather than as
/// something wedged against it. Below that the panel draws its bare border and
/// no mark at all: the art is drawn whole or not drawn, since half a `W` is not
/// a smaller mark but a different and wrong one.
const MARK_MARGIN: u16 = 2;

/// The rows the mark wants spare on top of its own height, and the least the
/// panel can be taller than the art and still draw it.
///
/// One row over the whole height, where [`MARK_MARGIN`] is two columns a side: a
/// row costs about two columns' worth of screen, and rows are what a terminal is
/// short of first. On a panel with exactly that row spare it falls under the
/// mark, because the odd row of an uneven split always does — see [`mark_area`].
const MARK_MARGIN_ROWS: u16 = 1;

/// The one line naming the tree's root.
const HEADER_HEIGHT: u16 = 1;

/// What the run in flight takes off the top of the panel, inside the border and
/// above the window: the one line naming it, and one blank row under that.
///
/// The line itself is one row, like the tree's header, and for the same reason:
/// every row it takes is a row of the card the reader no longer has, and the run
/// says what it is doing in one line or it is not worth a second — see
/// [`run_header_line`].
///
/// The blank row is what makes it read as a header rather than as the first row
/// of whatever is under it. The header is bold and so are an account's headings,
/// a summary and a question, so with nothing between them a run's line and the
/// card's first line are two bold rows against each other and the eye has to
/// work out which is furniture. A row is a cheap price for that, and it is only
/// paid while a run is actually going.
const RUN_HEADER_HEIGHT: u16 = 2;

/// What the run header calls a pact.
///
/// The footer's own verb, because it is the same run said twice on one screen
/// and a header that called it something else would be a second name for one
/// thing. It is repeated here rather than borrowed because
/// [`App::pact_line`](crate::app::App::pact_line) hands over a finished
/// sentence and a header sets its parts in different places; what is shared is
/// the vocabulary, not the wording.
const PACTING_RUN: &str = "pacting";

/// What the run header calls a refresh: [`PACTING_RUN`]'s counterpart, and the
/// one word the two runs differ by, exactly as on the footer.
const REFRESHING_RUN: &str = "refreshing";

/// One column of the bar the run header ends with that the run has reached.
///
/// A full block, so the fill reads as a quantity at a glance and on a terminal
/// with no colour — the bar carries none, like everything else in the panel.
const BAR_FILLED: &str = "█";

/// One column of the run header's bar the run has not reached: the same block
/// shaded, so the bar's whole length is visible and the fill is read against it
/// rather than against the border.
const BAR_EMPTY: &str = "░";

/// What the run header's line and its bar are held apart by.
const BAR_GAP: &str = " ";

/// The fewest columns of bar worth drawing.
///
/// Below this the bar is dropped whole and the line keeps the columns, the way
/// the tree header drops its sigils: a bar three columns long cannot show a
/// fraction — every position in a run of any length would fill the same one or
/// two of them — so it would be furniture in place of the directory name it
/// crowded out.
const BAR_MIN_WIDTH: usize = 4;

/// What the header's two facts are joined by: which tree is on screen, and what
/// this machine holds for the repository it came out of.
///
/// A dash with a space either side rather than a run of blanks, because the two
/// are different kinds of fact — a path and a holding — and a gap alone would
/// read as one sentence that had drifted apart. Unicode, like [`ELLIPSIS`] and
/// the panel's arrow, and measured with [`display_width`] like everything else
/// on the line, so what it costs is what the backend will charge for it.
const HEADER_GAP: &str = " — ";

/// Drawn on the left of every panel line that sits under a section heading, so a
/// directory and the pass under it read as one block rather than as a list with
/// a path in the middle of it.
///
/// The tree's [`INDENT`] happens to be the same two columns and deliberately is
/// not reused: that one is a unit of a walk's depth, this one is a fixed inset
/// under a heading, and a change to either has nothing to say about the other.
pub(crate) const PANEL_INDENT: &str = "  ";

/// What the panel's top edge says while the conversation is the card on screen.
///
/// The one card that is named on the border, because it is the one card a reader
/// could otherwise mistake for another: an account says what it is in every row
/// it draws — a directory and a clock — and a document is the file's own text,
/// where a question and an answer are neither, and a reader who swapped one card
/// too far would have to read the prose to find out where they had landed. A
/// word costs nothing off the window: a title sits on the border row the panel
/// was drawing anyway, so the thread has exactly the rows the account has.
///
/// Padded a column each side so it does not sit against the corner, like
/// [`scrollback`]'s indicator on the opposite edge. No colour, for the reason
/// written out at [`draw_panel`]: bold is what a heading gets here.
///
/// What the title says while the conversation is in brief mode is
/// [`BRIEF_THREAD_TITLE`]; [`thread_title`] is the choice between the two.
const THREAD_TITLE: &str = " thread ";

/// The same title while the conversation is converging on a document: the card's
/// name, then the register it is in.
///
/// The whole of what brief mode says on screen. It is the card's own name with a
/// word added rather than a name of its own, because it is the same card and the
/// same conversation — every turn that was on it is still on it — in a state; a
/// title that read `brief` alone would be a fourth card that does not exist. The
/// mode is worth saying at all because it changes what the next thing typed will
/// do, and a reader who has scrolled back past the `/brief` turn has nothing
/// else on screen to tell them.
///
/// What it does *not* say is anything about an artifact — no "nothing written
/// yet", no path of a file that has been written. A title is what the card is,
/// and the state of a document is news, which belongs in the thread with the
/// turn that caused it and a clock beside it, not on a border that would go on
/// asserting it every frame for the rest of the session.
///
/// The separator is the middot [`crate::account`] uses between the parts of a
/// line, and it lands on the same border row: a brief-mode panel has exactly the
/// rows a chat-mode one has. On a panel too narrow for it, the border truncates
/// the title as it always did.
const BRIEF_THREAD_TITLE: &str = " thread · brief ";

/// What the panel's top edge says about the conversation, given the register it
/// is in.
///
/// The one place the title is decided, so the two strings above are the two
/// things a top edge can say and a third register would be a compile error here
/// rather than a card that silently kept the old name. Called at the one
/// `title_top` in [`draw_panel`] and nowhere else.
const fn thread_title(mode: Mode) -> &'static str {
    match mode {
        Mode::Chat => THREAD_TITLE,
        Mode::Brief => BRIEF_THREAD_TITLE,
    }
}

/// Drawn on the left of the reader's own words in the thread, so a question
/// reads as a question rather than as the first line of an answer.
///
/// The turn's own heading, and marked rather than indented for the same reason
/// [`PANEL_INDENT`] indents what sits under a heading: the question is what the
/// rest of the turn is about, and everything under it belongs to it. Not
/// [`SELECTION_MARKER`], which is the tree's `>` and says where the keyboard is
/// — nothing is selected in the panel — and not a colour, which is a node
/// state's.
pub(crate) const SAID_MARKER: &str = "› ";

/// Drawn on the left of warlock's own line in the thread, so a note reads as the
/// program talking rather than as a question, an answer or a pass at work.
///
/// A dot rather than [`SAID_MARKER`]'s arrow, because an arrow points at what
/// follows it and a note is not addressed to anybody: it is an aside beside the
/// conversation. Two columns like the question's marker and like
/// [`PANEL_INDENT`], so the three shapes of the card line up down the same
/// column instead of stepping in and out by a character. Plain rather than bold
/// — a note heads nothing, and bold is what the turn's own heading gets — which
/// is the second half of telling it from a question drawn with the same width in
/// front of it.
pub(crate) const NOTE_MARKER: &str = "· ";

/// What a truncated panel line ends with, in place of what was cut off.
///
/// One column rather than three dots, because the columns it takes are columns
/// taken off the text it is there to make room for. Unicode, like the scrollback
/// indicator's arrow: a terminal that cannot draw this cannot draw the panel's
/// arrow either, and the ASCII the tree's markers are held to is about a screen
/// full of them rather than about the odd line that ran long.
const ELLIPSIS: &str = "…";

/// The arrow the scrollback indicator leads with: down, because what it counts
/// is below the view.
const SCROLLBACK_ARROW: &str = "↓";

/// The key the scrollback indicator names, which is the one that returns a
/// scrolled-back panel to the newest line.
///
/// `G` and not a key of its own: it is the focused pane's ordinary end-of-list
/// movement, already on [`KEYS`], and a run that needed a new binding to get
/// back to live would be a run with a mode in it.
const LIVE_KEY: &str = "G";

/// The `k`/`j` key pair, which moves the selection one row.
const ROW_KEY: &str = "k/j: row";

/// The page keys, which move the selection one screenful.
const PAGE_KEYS: &str = "PgUp/PgDn";

/// The `g`/`G` pair, which goes to the top of the list or the bottom of it.
const ENDS_KEY: &str = "g/G: ends";

/// The space key, which hides the subtree under the selected row or shows it
/// again.
const FOLD_KEY: &str = "space: fold";

/// The `o` key, which hides everything warlock is not managing.
const PACTS_KEY: &str = "o: pacts";

/// The `f` key, which puts the files inside a directory on screen.
const FILES_KEY: &str = "f: files";

/// The `p` key, which makes a pact with the selected directory or gives one up.
const PACT_KEY: &str = "p: pact";

/// The `r` key, which runs a pass over what is stale.
const REFRESH_KEY: &str = "r: refresh";

/// The `s` key, which opens the scope prompt on a directory already pacted.
const SCOPE_KEY: &str = "s: scope";

/// The Tab key, which moves the focus on one place: the tree, then the panel,
/// then the composer under it, then round again.
///
/// `focus` and not `compose`, because that is what the key does — see
/// [`Focus::next`](crate::Focus::next) — and a name that said the composer would
/// be a name that lied on the two presses out of three that land somewhere else.
/// It is still the name that says the composer is reachable at all: the field is
/// drawn without the keyboard until this key hands it over.
const FOCUS_KEY: &str = "Tab: focus";

/// The Enter pair inside the composer: Enter offers the draft up, Alt+Enter puts
/// a newline in it.
///
/// One name for the two keys because they are one decision — this line ends the
/// message or this line does not — and a reader who is told about Enter without
/// being told about Alt+Enter has been told that a draft can only ever be one
/// line. Paired the way [`ROW_KEY`] and [`ENDS_KEY`] are, keys then effects in
/// the same order, so the slash reads across rather than down.
///
/// Shift+Enter is deliberately unnamed: [`compose_for`](crate::compose_for)
/// treats it as Enter because terminals disagree about whether they report it at
/// all, and a name for a keystroke half the readers of warlock cannot send is a
/// name that costs columns to mislead them.
const COMPOSE_KEYS: &str = "Enter/Alt+Enter: send/newline";

/// The Esc key inside the composer, which hands the keyboard back and leaves
/// what was typed exactly where it was.
///
/// The draft half of the name is the half that has to be there. Esc is the key
/// every other field in every other program throws work away with, and
/// [`compose_for`](crate::compose_for) does the opposite —
/// [`Composed::Leave`](crate::Composed::Leave) keeps the draft — so a reader who
/// is not told that is a reader who retypes a paragraph rather than pressing it.
///
/// Esc is on this line twice, here and inside [`QUIT_KEY`], and that is the
/// truth rather than a duplication: the same key leaves the field when the field
/// has the keyboard and leaves warlock when nothing does.
const LEAVE_KEY: &str = "Esc: leave, draft kept";

/// The leading slash inside the composer, which makes a draft a command instead
/// of a message.
///
/// One name for all three commands rather than one name each. The line is
/// already wider than an eighty-column terminal and every column of it costs
/// another key its place, so what the footer buys here is the shape of the
/// thing — a draft that starts with a slash is read as a command — and not the
/// vocabulary. The vocabulary is a keystroke away and already written:
/// [`submitted_for`](crate::submitted_for) refuses a slash that is not one of
/// the commands with a line that names `/brief`, `/write` and `/chat`, so a
/// reader who types `/` and guesses wrong is told all three by the thing they
/// typed at. Three names here would spend three times the columns to say that
/// worse, and would want widening again for every command that came after.
///
/// It is listed with the composer's names in [`KEYS`] and given up with them in
/// [`KEY_DROP_ORDER`], because it is a fact about the one field rather than
/// about the screen: a slash typed anywhere else is a slash, and a reader with
/// no cursor in the composer has nothing to do with this name.
const COMMAND_KEY: &str = "/: command";

/// The keys line of the footer, up to the one key whose name depends on what it
/// would do next: every key that does something, in the order [`keys_line`]
/// assembles them.
///
/// A sequence of names rather than one joined string, because the line is laid
/// out for the width the terminal actually has: [`laid_out_keys`] gives names up
/// whole until what is left fits, and a name buried inside a joined string is a
/// name it cannot give up.
///
/// The movement keys first and together, in the order a reader reaches for
/// them: one row, one screen, the whole tree. Then the three keys that move
/// nothing but change what there is to move through — space, which hides a
/// subtree, `o`, which hides everything Warlock is not managing, and `f`, which
/// is the one of the three that puts rows on screen rather than taking them
/// off — and only then the keys that change something. `s` comes after `p` and
/// `r` because it is the one of the three that needs a pact to already be
/// there: nothing on a row `p` has never been pressed on is scopeable.
///
/// The composer's four come last, together and in the order somebody meets
/// them: [`FOCUS_KEY`] reaches the field, [`COMPOSE_KEYS`] is what to press in
/// it, [`LEAVE_KEY`] is how to stop, and [`COMMAND_KEY`] is the one thing the
/// field does that a cursor does not explain. They sit at the end because they
/// are the keys of one pane rather than of the whole screen, and because that is
/// where a reader who has just looked down from a cursor finds them — next to
/// the other facts about what the terminal is doing rather than buried among the
/// tree's movement. Where they sit is not where they are given up: see
/// [`KEY_DROP_ORDER`], which loses all four before it loses anything else.
///
/// Every name here is as short as it can be and still be read: the line is
/// already wider than an eighty-column terminal, and a key nobody can see
/// because the line ran off the right-hand edge is a key nobody knows about.
/// That is why `o` is labelled with what it leaves on screen rather than with a
/// sentence about filtering, and `f` with what it shows rather than with a
/// sentence about a toggle.
///
/// The movement names are the shortest of the lot because they are the ones a
/// reader needs told once. `k/j: row` names the keys that have to be learnt and
/// leaves the arrows unnamed — they were the same sentence twice, and the
/// arrows are what a reader presses before reading anything — and `g/G: ends`
/// says where the pair go without spelling out which end is which, which the
/// keys' own order already implies. `PgUp/PgDn` carries no label at all for the
/// same reason: its label was the word already inside the keys' own names, and
/// a name that repeats itself is the cheapest kind of column to give up. The
/// three of them read as the granularities they are — a row, a page, the ends —
/// which is the order the group is in anyway.
///
/// Those shortenings are what paid for the keys that came later: `r: refresh`
/// was bought with `up/down k/j: move` and `g/G: first/last`, and `s: scope`
/// with `k/j: move` → `k/j: row`, `PgUp/PgDn: page` → `PgUp/PgDn`,
/// `space: collapse` → `space: fold` and `o: pacted` → `o: pacts` — twelve
/// columns for a twelve-column key. Those shortenings are still worth making,
/// but they are no longer the only thing standing between a narrow terminal and
/// the way out: see [`KEY_DROP_ORDER`].
const KEYS: &[&str] = &[
    ROW_KEY,
    PAGE_KEYS,
    ENDS_KEY,
    FOLD_KEY,
    PACTS_KEY,
    FILES_KEY,
    PACT_KEY,
    REFRESH_KEY,
    SCOPE_KEY,
    FOCUS_KEY,
    COMPOSE_KEYS,
    LEAVE_KEY,
    COMMAND_KEY,
];

/// What separates one key's name from the next on the keys line.
///
/// Wide enough that two names read as two keys rather than as one phrase, and
/// the same gap between every pair — including the two [`keys_line`] adds, which
/// are part of the same line and not an afterthought tacked onto the end of it.
const KEY_GAP: &str = "    ";

/// The `m` key's name while the terminal is reporting its mouse: what the next
/// press does, which is stop it.
///
/// Named by its effect rather than by its state — `m: mouse off` and not
/// `mouse: on` — because a line of keys is a line of things to press, and every
/// other name on it says what pressing does. It is short for the reason every
/// name here is short, and it sits next to quit rather than beside `f` and `o`:
/// those change what warlock draws, this changes what the terminal does with the
/// pointer, which is the same kind of fact as how to leave.
const MOUSE_OFF_KEY: &str = "m: mouse off";

/// The `m` key's name while capture is off: what the next press does, which is
/// start it again. See [`MOUSE_OFF_KEY`].
///
/// This is the wording that has to be right. Capture off is the state a reader
/// can be surprised by — the wheel does nothing, and the only way back is a key
/// whose name is the one thing on screen that says so.
const MOUSE_ON_KEY: &str = "m: mouse on";

/// The way out, last on the keys line because it is the last thing anybody needs
/// and the first thing they have to be able to find.
const QUIT_KEY: &str = "q/Esc/Ctrl-C: quit";

/// The `v` key's name, reserved in [`KEY_DROP_ORDER`] before the key exists.
///
/// Not on the line: naming a key warlock does not have would be the footer
/// lying. Its place in the order is here so that the slice which adds the key
/// adds a name to [`KEYS`] and nothing else.
const VIEW_KEY: &str = "v: view";

/// The `e` key's name, reserved the same way and for the same reason as
/// [`VIEW_KEY`].
const EDIT_KEY: &str = "e: edit";

/// The order the idle keys line gives its names up in, first dropped to last
/// kept.
///
/// A terminal too narrow for the whole line has to lose something, and this is
/// where which something is decided. Left to the right-hand edge the line loses
/// its tail, and its tail is [`QUIT_KEY`] — the one name a stuck reader is
/// looking for, gone on the eighty-column terminal where being stuck is most
/// likely. So the line is laid out for the width it has, by [`laid_out_keys`],
/// and gives up the names a reader is least likely to need told first.
///
/// The composer's four go before everything, because a reader looking at a
/// composer has a cursor in front of them saying it takes typing — the field is
/// the one thing on screen that explains itself, so its names are the cheapest
/// on the line. [`COMPOSE_KEYS`] goes first of the four: it is the widest name
/// here by some way, and Enter is the press everybody makes in a field without
/// being told. [`LEAVE_KEY`] next, because Esc is the other press everybody
/// makes, and because the way out is still spelled on the line inside
/// [`QUIT_KEY`]. [`COMMAND_KEY`] outlasts those two, because the slash is the
/// one thing about the field a cursor does not hint at and nobody presses by
/// accident. [`FOCUS_KEY`] is the last of the four kept even so: Tab is the only
/// one of them a reader needs *before* the cursor is theirs, and a composer
/// nobody can reach is a composer nobody types a command into either.
///
/// Movement goes next, all of it: `PgUp/PgDn`, then `g/G: ends`, then
/// `k/j: row`. The arrows are what a reader presses before reading anything at
/// all, and j/k and the page keys are the guesses anyone who has used a pager
/// already has — a movement key nobody names is a movement key most people find
/// anyway. The view toggles go next — `f: files`, `o: pacts`, `space: fold` —
/// because each of them is undone by pressing it again, so a reader who trips
/// over one is a single keystroke from the screen they had.
///
/// The verbs go late — `s: scope`, `r: refresh`, `p: pact` — because nobody
/// guesses p/r/s/v/e. They are the names that are only known if they are read,
/// so they are the last ones worth spending columns on, and `p` outlasts the
/// other two because a pact is the thing the other two are about.
///
/// The `m` key is in this order twice, and the asymmetry is the point.
/// [`MOUSE_OFF_KEY`] drops early, with the view toggles: while capture is on the
/// wheel works, and a reader who never learns it can be stopped has lost
/// nothing. [`MOUSE_ON_KEY`] is the last name kept before the way out, because
/// capture off is the state a reader can be surprised by — the wheel does
/// nothing, and this name is the only thing on screen that says how to get it
/// back.
///
/// [`QUIT_KEY`] is not in this order at all, which is how it is never dropped.
const KEY_DROP_ORDER: &[&str] = &[
    COMPOSE_KEYS,
    LEAVE_KEY,
    COMMAND_KEY,
    FOCUS_KEY,
    PAGE_KEYS,
    ENDS_KEY,
    ROW_KEY,
    FILES_KEY,
    PACTS_KEY,
    FOLD_KEY,
    MOUSE_OFF_KEY,
    EDIT_KEY,
    VIEW_KEY,
    SCOPE_KEY,
    REFRESH_KEY,
    PACT_KEY,
    MOUSE_ON_KEY,
];

/// `pieces` joined by [`KEY_GAP`] into a line of at most `width` columns, giving
/// names up in `drop_order` until what is left fits.
///
/// Arithmetic, and all of it here: measure, drop the next name the order names,
/// measure again. A name goes with its gap because the gap is what joins it to
/// its neighbour, and four columns of nothing between two keys would be the
/// dropped name still costing what it cost.
///
/// Whole names or nothing. Half a key's text is a key nobody can press, so a
/// name that does not fit is not abbreviated or ellipsised, it is left off, and
/// the reader is told fewer true things rather than one untrue one.
///
/// Measured with [`display_width`], which is what the backend will charge for
/// the row, rather than with `str::len`, which is only the same number while
/// every name is ASCII.
///
/// A name not in `drop_order` is never dropped. When only those are left and
/// they still do not fit, the line is cut to the width by [`clipped`] — which is
/// a terminal narrower than [`QUIT_KEY`] itself, or than [`CANCEL_KEY`] and the
/// way out together, where the start of a name is more use than a blank line.
fn laid_out_keys(width: usize, pieces: &[&str], drop_order: &[&str]) -> String {
    let mut kept: Vec<&str> = pieces.to_vec();
    let mut order = drop_order.iter();
    while display_width(&kept.join(KEY_GAP)) > width {
        let Some(name) = order.next() else {
            break;
        };
        if let Some(at) = kept.iter().position(|piece| piece == name) {
            kept.remove(at);
        }
    }

    clipped(&kept.join(KEY_GAP), width)
}

/// `text`, cut to `width` columns with nothing put in place of what was cut.
///
/// [`truncated`] with the [`ELLIPSIS`] left off, and the difference is what the
/// cut means. There it says a path goes on past the edge of the panel; here the
/// only thing ever cut is the tail of a keys line that has already given up
/// every name it is allowed to — [`QUIT_KEY`], or [`CANCEL_KEY`] and the way out
/// of a run — and a mark saying the key goes on would cost a column of the key
/// it was marking.
///
/// Cut on a character boundary, so this cannot panic on text that is not ASCII.
fn clipped(text: &str, width: usize) -> String {
    if display_width(text) <= width {
        return text.to_owned();
    }

    let mut taken = 0;
    let mut end = 0;
    for (index, character) in text.char_indices() {
        let next = index + character.len_utf8();
        let columns = display_width(&text[index..next]);
        if taken + columns > width {
            break;
        }
        taken += columns;
        end = next;
    }

    text[..end].to_owned()
}

/// The keys line for a terminal `width` columns wide that is reporting its mouse
/// or one that is not.
///
/// The only name that varies with the state is the `m` key's, and it varies the
/// way [`KEYS`] and [`PACTING_KEYS`] do: the line says what the next press will
/// do, so the state capture is in is read off the key that changes it rather
/// than announced on the message line — which the next keystroke would wipe,
/// while capture stays off until somebody presses `m` again.
///
/// What comes back is no wider than `width`, whichever name it picked and
/// however narrow the terminal: which names survive that is [`KEY_DROP_ORDER`]'s
/// business and fitting them is [`laid_out_keys`]'.
fn keys_line(mouse_captured: bool, width: usize) -> String {
    let mouse = if mouse_captured {
        MOUSE_OFF_KEY
    } else {
        MOUSE_ON_KEY
    };

    let mut pieces = KEYS.to_vec();
    pieces.push(mouse);
    pieces.push(QUIT_KEY);
    laid_out_keys(width, &pieces, KEY_DROP_ORDER)
}

/// The keys line while a pact is running: the same line's job, for the mode the
/// app is in while it works.
///
/// Esc is why this line exists. It means quit on [`KEYS`] and cancel here, and a
/// key that means two things has to say which one it means now; a run that
/// cannot be stopped by anyone who does not already know how is a run the reader
/// waits out.
///
/// Much shorter than [`KEYS`], and short on purpose. It names what a reader
/// reaches for while waiting — a look around the tree, a way to stop, a way out
/// — and leaves out the rest rather than restating a line they have been reading
/// since launch. Short also means it survives a narrow terminal whole, which
/// matters more here than there: this is the line that answers "how do I stop
/// this?", and an answer truncated off the right-hand edge is no answer.
///
/// Names rather than one joined string for the reason [`KEYS`] is: a terminal
/// narrow enough to lose part of even this line loses whole names, in
/// [`PACTING_KEY_DROP_ORDER`], rather than losing its right-hand end.
///
/// The names are longer than their counterparts on [`KEYS`] — the movement key
/// is spelled `up/down k/j: move` where the idle line says `k/j: row` — and can
/// afford to be, because there are only four of them. A reader who has just
/// started a run is reading this line for the first time; the idle line they
/// have had on screen since launch.
const PACTING_KEYS: &[&str] = &[MOVE_KEYS, COLLAPSE_KEY, CANCEL_KEY, PACTING_QUIT_KEY];

/// The movement keys while a pact runs, arrows and j/k named together.
///
/// Both halves named, unlike [`ROW_KEY`] on the idle line, because this line has
/// the columns for it: see [`PACTING_KEYS`].
const MOVE_KEYS: &str = "up/down k/j: move";

/// The space key while a pact runs: what it does, spelled out.
const COLLAPSE_KEY: &str = "space: collapse";

/// Esc while a pact runs, which is the whole reason this line differs from
/// [`KEYS`]: the key that means quit there means stop the run here.
const CANCEL_KEY: &str = "Esc: cancel";

/// The way out while a pact runs, with Esc left off it because Esc is spoken for
/// by [`CANCEL_KEY`].
const PACTING_QUIT_KEY: &str = "q/Ctrl-C: quit";

/// The order the pacting keys line gives its names up in, first dropped to last
/// kept.
///
/// [`KEY_DROP_ORDER`]'s reasoning, applied to a line with four names on it.
/// [`MOVE_KEYS`] goes first: the arrows and j/k are what a reader presses
/// without being told, and a look around the tree is the least of what they came
/// to this line for. [`COLLAPSE_KEY`] next, because folding a subtree is undone
/// by pressing the same key again.
///
/// [`CANCEL_KEY`] and [`PACTING_QUIT_KEY`] are not in this order at all, which is
/// how neither is ever dropped. This is the line that answers "how do I stop
/// this?" and both of its answers outlast every other name on it — the run's own
/// stop first, then the way out of warlock entirely.
const PACTING_KEY_DROP_ORDER: &[&str] = &[MOVE_KEYS, COLLAPSE_KEY];

/// The keys line for a terminal `width` columns wide while a pact is running.
///
/// [`keys_line`]'s job for [`PACTING_KEYS`], and the same [`laid_out_keys`] doing
/// it: nothing about this line varies with app state, so the width is the only
/// thing it is told.
fn pacting_keys_line(width: usize) -> String {
    laid_out_keys(width, PACTING_KEYS, PACTING_KEY_DROP_ORDER)
}

/// The tally line, the keys line and the message line.
///
/// The message line is there whether or not there is a message to put on it: a
/// footer that grew a line when the app had something to say would shove the
/// tree down a row and reflow the whole window on a keystroke that changed
/// nothing about the tree.
const FOOTER_HEIGHT: u16 = 3;

/// The share of the width the tree column asks for while the terminal is wide
/// enough to give it: a proportion, so a wide terminal spends its extra columns
/// on the panel rather than on a tree column that has nothing to do with them.
///
/// See [`areas`] for the whole rule.
const TREE_PERCENT: u16 = 30;

/// The fewest columns the tree column is cut to while the terminal can afford
/// it.
///
/// Roughly a name at four levels of indent, its markers and its gutter: below
/// this a tree column stops showing the tree and starts showing the left-hand
/// end of it. See [`areas`] for what happens when the terminal cannot afford
/// even this.
const TREE_MIN_WIDTH: u16 = 30;

/// The border every pane carries, on all four sides.
///
/// One row at the top and one at the bottom, one column each side: the tree's
/// rows and its header are drawn inside this, which is why [`tree_height`] has
/// to take it off the terminal's height as well as the header and the footer.
const BORDER_THICKNESS: u16 = 1;

/// How long the row of the directory a pact is working right now holds each of
/// its two colours before taking the other: half a second stale, half a second
/// fresh, over and over for as long as that pass runs. See [`pulse_colour`].
///
/// Half a second because of what it is measured against at either end. The event
/// loop wakes every 100 ms (`POLL_INTERVAL`), so a phase this long is redrawn
/// about five times before it turns over: the change lands within a tenth of a
/// second of when it is due, and the pulse never depends on a wakeup arriving at
/// a particular moment. Much shorter and it would start to alias against that
/// tick — a phase of two or three wakeups reads as a flicker whose rate depends
/// on how busy the loop was — and a row flashing several times a second beside
/// text somebody is reading is the kind of movement that has to be looked away
/// from. Much longer and it stops reading as movement at all: a row that holds
/// one colour for two seconds looks like a row that has settled on it, which is
/// the one thing this must not say. Half a second is slow enough to be calm and
/// fast enough that a pass of even a few seconds visibly pulses more than once.
const PULSE_PHASE: Duration = Duration::from_millis(500);

/// What the gate on the way out asks.
///
/// A question, in the words the answers answer: "Leave warlock?" is answered by
/// Yes and No without either of them having to be re-read as a verb. It names
/// the program rather than saying "quit?", because the reader who pressed Esc by
/// reflex is being told what the keystroke was about to do, and "leave" is what
/// the footer's own [`QUIT_KEY`] calls it in the other direction.
///
/// Nothing is said here about the keys that answer it. The two answers are on
/// screen and Enter takes the lit one; a line spelling out `y`/`n`/Esc would be
/// a second footer inside a window that exists to be read in one glance, and the
/// keys that work are the ones anybody would try. See [`mod@crate::confirm`] for
/// what those are.
const CONFIRM_QUESTION: &str = "Leave warlock?";

/// The left-hand answer, the one that leaves.
///
/// Padded a column each side so the highlight is a block around the word rather
/// than a word with its edges touching whatever is beside it: the lit answer is
/// drawn reversed, and reversing exactly three columns reads as a stain on the
/// text instead of as a button. The padding is part of the constant so that the
/// width the window is sized to (see [`confirm_size`]) is the width actually
/// drawn.
const CONFIRM_YES: &str = " Yes ";

/// The right-hand answer, the one the question opens on. See [`CONFIRM_YES`] for
/// the padding, and [`mod@crate::confirm`] for why the order of the two is
/// load-bearing: Left lights this one's neighbour and Right lights this one,
/// positionally, which is only true while Yes is drawn on the left.
const CONFIRM_NO: &str = " No ";

/// What sits between the two answers.
///
/// The footer's [`KEY_GAP`] is the same four columns and is deliberately not
/// reused: that one separates names on a line of keys, this one separates two
/// answers to one question, and a change to either has nothing to say about the
/// other.
const CONFIRM_ANSWER_GAP: &str = "    ";

/// The clear columns the question is given inside the window's border, each
/// side.
///
/// Three, where the panel's mark takes two: this window is a handful of columns
/// of text in the middle of a full screen, and the space around it is what makes
/// it read as something laid on top rather than as a box that happens to be
/// there.
const CONFIRM_MARGIN: u16 = 3;

/// The clear rows above the question and below the answers, inside the border.
///
/// One each, for the reason [`MARK_MARGIN_ROWS`] is one: a row costs about two
/// columns' worth of screen, and this window is drawn over a tree somebody was
/// reading.
const CONFIRM_MARGIN_ROWS: u16 = 1;

/// The lines of the window itself: the question, a blank, the answers.
///
/// The blank between them is not decoration. The question and the pair of
/// answers are two different things to read, and a reader who has already read
/// the question needs to find the highlight without their eye being caught by
/// the text above it.
const CONFIRM_LINES: u16 = 3;

/// How tall the whole window is: its lines, the rows kept clear around them, and
/// the border.
///
/// A number rather than something measured, because unlike the width there is
/// nothing to measure: [`CONFIRM_LINES`] is a fixed three. It is what
/// [`confirm_area`] clamps against a short terminal.
const CONFIRM_HEIGHT: u16 = CONFIRM_LINES + 2 * CONFIRM_MARGIN_ROWS + 2 * BORDER_THICKNESS;

/// What the scope window says it is about, in front of the directory it is
/// about.
///
/// "Scope for `crates/warlock-engine`" rather than a bare path, because a path
/// on its own in a window that appeared under somebody's hands says which
/// directory but not what is being asked of it — and this window is opened by a
/// single keystroke, which is exactly the way to arrive at one without having
/// meant to. The trailing space is part of the constant so that the width the
/// window is sized to (see [`scope_size`]) is the width actually drawn, the way
/// [`CONFIRM_YES`]'s padding is.
///
/// Handed to [`draw_scope`] rather than read inside it: the same window draws
/// the path prompt too, under [`PATH_HEADING`], and a window that named itself
/// would be a second window to keep in step with this one.
const SCOPE_HEADING: &str = "Scope for ";

/// What the path prompt's window says it is about, in front of what
/// [`ScopeField::directory`] carries — which is nothing, because that field
/// carries the whole heading.
///
/// Empty on purpose, and a constant rather than a bare `""` at the one call
/// site, so that the two windows are drawn from two pairs of words in the same
/// place rather than from one pair and a literal. The path prompt is opened by a
/// `/write` turn landing, not by a keystroke, so the heading has to come from
/// whoever knew what was being asked for; that is the binary, and the one string
/// a [`ScopeField`] already has for saying what it is asking about is its
/// directory (see [`mod@crate::prompt`]). Nothing here reads it as a path — this
/// window prints it, as it prints the module the scope prompt puts there.
const PATH_HEADING: &str = "";

/// The dim last line of the path prompt's window: what the two keys do.
///
/// [`scope::RULES`](warlock_engine::scope::RULES)'s place in the window, and deliberately not the same kind of
/// sentence. The scope prompt's line is the engine's, word for word, because
/// what a scope may say is the engine's to judge; what a path may say is judged
/// where the file is written, and this line does not attempt it. It says what
/// Enter and Esc come to, which is worth saying here for the reason nothing else
/// in warlock needs it said: this is the one window a reader arrives at without
/// having pressed a key for it, so the way out of it is not something they can
/// have just done.
const PATH_RULES: &str = "Enter writes the document, Esc writes nothing";

/// The clear columns the scope window's text is given inside its border, each
/// side: [`CONFIRM_MARGIN`], spelled as that rather than as a three.
///
/// Two windows laid over the same frame by the same program, inset by different
/// amounts, would read as two programs — and a number copied here would be free
/// to become a different number by nobody's decision.
const SCOPE_MARGIN: u16 = CONFIRM_MARGIN;

/// The clear rows above and below the scope window's text, inside its border:
/// [`CONFIRM_MARGIN_ROWS`], for the reason [`SCOPE_MARGIN`] is
/// [`CONFIRM_MARGIN`].
const SCOPE_MARGIN_ROWS: u16 = CONFIRM_MARGIN_ROWS;

/// The one column drawn after the text in the field, reversed, to say where the
/// next character will land.
///
/// Drawn rather than asked for. The terminal's own caret is hidden for every
/// frame warlock draws — a caret parked in the corner of a tree nobody is typing
/// into is a caret in the wrong place all session — and showing it for this one
/// window would mean putting it back for the frame after, on a terminal whose
/// blink and shape warlock does not control. A reversed blank is in the buffer,
/// so it is the same on every terminal and a test can find it.
///
/// A space rather than a block glyph, for the same reason: reversing a cell
/// lands wherever the palette lands, where `█` is a font's opinion of a full
/// block and sits a row too high in some of them.
const SCOPE_CURSOR: &str = " ";

/// The lines of the scope window: the heading, a blank, the field, the row the
/// broken rule goes in, and the rules a scope keeps.
///
/// Five whether or not a rule has been broken. The row under the field is kept
/// clear rather than closed up when [`ScopeField::rule`] is `None`, so a refused
/// submit puts a line on screen without moving the field out from under the
/// reader's eye — a window that grew a row on Enter would take the text they are
/// about to correct with it, and on a short terminal would take it a row closer
/// to being clipped. The blank under the heading is [`CONFIRM_LINES`]'s blank,
/// for the same reason: what the window is about and what is being typed are two
/// things to read.
///
/// Five for the path prompt as well, which is drawn by the same [`draw_scope`]
/// with the other pair of words in it: one window somebody types a line into,
/// asked twice, rather than two windows that could drift a row apart.
const SCOPE_LINES: u16 = 5;

/// How tall the whole scope window is: its lines, the rows kept clear around
/// them, and the border. [`CONFIRM_HEIGHT`]'s arithmetic, over
/// [`SCOPE_LINES`].
const SCOPE_HEIGHT: u16 = SCOPE_LINES + 2 * SCOPE_MARGIN_ROWS + 2 * BORDER_THICKNESS;

/// The caret drawn after the composer's draft: [`SCOPE_CURSOR`], spelled as that
/// rather than as another space.
///
/// The two fields are the only things in warlock somebody types into, and a
/// reader who has seen one of them has learnt what the caret looks like. Written
/// as the constant rather than copied so that a change to how warlock draws a
/// cursor is one change and not two that have to be remembered together.
const COMPOSER_CURSOR: &str = SCOPE_CURSOR;

/// The fewest rows a composer can be drawn in: one row for the draft, and the
/// border above and below it.
///
/// Below this the field is not drawn at all and the panel keeps the whole column
/// — see [`split_column`]. A box with a border and no row inside it would be two
/// rows spent saying there is somewhere to type and no room to type in it.
const COMPOSER_MIN_HEIGHT: u16 = 1 + 2 * BORDER_THICKNESS;

/// Draw the whole frame: the panel on the left, the tree column on the right,
/// the footer full width beneath both.
///
/// Pure in the sense that matters here — it reads `app` and writes `frame`,
/// touching no terminal state of its own.
///
/// The panes take what the footer leaves and so give their rows up first when
/// the terminal is short: on a screen with no room for everything, which nodes
/// are off the bottom matters less than still being told the tally and how to
/// get out.
///
/// `now` is the instant this frame is being drawn at, and it is the caller's
/// rather than read here: the newest line of the panel's live section counts up
/// against it, the row of the directory a pact is working pulses against it (see
/// [`pulse_colour`]), and a renderer that called [`Instant::now`] itself would be
/// a second clock for a test to fight. The event loop already redraws on a tick,
/// so handing it the instant it woke up at is all the ticking there is.
///
/// `confirm` is the gate on the way out, and it is handed in beside the app
/// because it is not part of the app: answering No has to leave the view exactly
/// as it was, and the cheapest guarantee of that is an app that never heard the
/// question (see [`mod@crate::confirm`]). While it is [`QuitConfirm::Closed`]
/// this draws the frame it has always drawn, cell for cell. While it is open the
/// same frame is drawn and then [`draw_confirm`] puts the question over the
/// middle of it — over, and not instead of, so the reader can still see what
/// they are about to leave, and so the footer keeps its three lines and its
/// wording whichever way the question is answered.
///
/// `scope` is the other question this frame can be carrying, handed in beside
/// the app for the same reason and drawn the same way: closed, it changes
/// nothing about the frame; open, [`draw_scope`] puts a window over the middle
/// of it with the directory being scoped, the field, and the rules a scope keeps
/// (see [`mod@crate::prompt`]). By reference rather than by value, unlike
/// `confirm`, because it is carrying a string somebody is typing and a renderer
/// has no business owning a copy of it.
///
/// `path` is the third window and the only one no keystroke opens: the path a
/// brief is about to be written to, up from the moment a `/write` turn answers
/// until Enter writes the file or Esc writes nothing. It is a [`ScopePrompt`]
/// like the one above it because it is the same field and the same editor with a
/// different question in it, and it is drawn by the same [`draw_scope`] with
/// [`PATH_HEADING`] and [`PATH_RULES`] in place of the scope's two sentences —
/// so the window a reader has already learnt to read is the window they get. The
/// heading itself rides in the field (see [`PATH_HEADING`]): this crate never
/// composed it and never reads it as a path.
///
/// The windows are drawn in that order, and the scope prompt last, so that a
/// frame somehow carrying more than one shows the one whose keys are live rather
/// than half of each — the order they are stacked in here is the order
/// [`press_for`](crate::input::press_for) consults them in, back to front. The
/// event loop never opens two — while any is up it consults that one instead of
/// the app, so the key that would open another never reaches anything — and this
/// is what that costs to be safe about it: two `if`s in the order the modes
/// stack.
///
/// `composer` is the draft somebody is typing under the panel, handed in beside
/// the app for the reason the two questions are: the loop owns it and the app
/// has never heard of it (see [`mod@crate::composer`]). Unlike them it is not
/// drawn *over* anything — it is a pane of its own, cut off the bottom of the
/// panel's column by [`areas`], so the panel above it loses exactly the rows it
/// takes. `None` is a frame with no composer at all, which is what a test that
/// is not about the field draws; and a `Some` handed in while the document card
/// has the panel is put back to `None` here (see [`on_screen`]), so the rule
/// about when the field is on screen is [`App::composer_showable`]'s and is
/// asked rather than repeated.
#[expect(
    clippy::too_many_arguments,
    reason = "one frame's worth of state, and the point of it is that the binary \
              draws a frame in one call: the three windows that can be over the \
              app are three parameters here rather than three entry points"
)]
pub fn draw(
    frame: &mut Frame<'_>,
    app: &App,
    chrome: &Chrome,
    now: Instant,
    confirm: QuitConfirm,
    scope: &ScopePrompt,
    path: &ScopePrompt,
    composer: Option<&Composer>,
) {
    let screen = frame.area();
    let composer = on_screen(app, composer);
    let Areas {
        panel,
        composer: field,
        tree,
        footer,
    } = areas(screen, composer);

    draw_panel(frame, panel, app, now);
    if let (Some(area), Some(composer)) = (field, composer) {
        // Two things have to be true for the field to be drawn as the place the
        // next character lands: the keys have to be pointed at it, and it has to
        // be taking them. A muted field is one a turn is being answered over
        // (see [`Composer::is_muted`]), and it is drawn exactly as a field
        // nobody is pointed at — dim, and with no caret — because that is what
        // is true of it. Muting only ever changes how the field is drawn: it is
        // still on screen, still the panel's missing rows, still holding every
        // character of the draft. What takes the field off the frame is the card
        // showing — it is drawn under the conversation and under neither of the
        // other two — which is [`on_screen`]'s question and not this one.
        let live = app.focus() == Focus::Composer && !composer.is_muted();
        draw_composer(frame, area, composer, live);
    }
    draw_tree_pane(frame, tree, app, chrome, now);
    draw_footer(frame, footer, app);

    if let Some(highlighted) = confirm.highlighted() {
        draw_confirm(frame, screen, highlighted);
    }
    if let Some(field) = path.field() {
        draw_scope(frame, screen, field, PATH_HEADING, PATH_RULES);
    }
    if let Some(field) = scope.field() {
        draw_scope(frame, screen, field, SCOPE_HEADING, scope::RULES);
    }
}

/// The areas one frame is cut into: the panel, the composer under it, the tree
/// column beside them, and the footer under everything.
///
/// Split out so that [`tree_height`] answers the same question [`draw`] does,
/// from the same call: a caller that told the app one height while the frame
/// used another would scroll by a window that is not on screen. The composer is
/// why that matters twice over — the rows it takes are rows off the panel, so
/// [`panel_height`] and [`composer_height`] have to be two answers from the one
/// cut rather than two opinions about the same column.
struct Areas {
    /// The left-hand pane, the majority of the width, drawn by [`draw_panel`].
    panel: Rect,
    /// The composer's own pane, under the panel and in the same column: `None`
    /// on a frame that has no composer on it, and on one with no room for even
    /// the smallest field. See [`split_column`].
    composer: Option<Rect>,
    /// The right-hand pane: border, header, tree rows.
    tree: Rect,
    /// Full width along the bottom, under both panes.
    footer: Rect,
}

/// Cut `area` into the footer and the two panes above it.
///
/// The footer is taken off the bottom first, at its fixed [`FOOTER_HEIGHT`] and
/// the full width: it is nobody's pane, it says what the keys do and what the
/// tally is, and both of those are about the screen rather than about either
/// side of it.
///
/// What is left is split left to right by a proportion with a floor, not by a
/// fixed column count:
///
/// * The tree column asks for [`TREE_PERCENT`] of the width, and never fewer
///   than [`TREE_MIN_WIDTH`] columns.
/// * It is capped at half the width, so the panel is never the smaller of the
///   two: it keeps whatever is left over, which on an odd width is the extra
///   column. Everywhere the terminal is wide enough for the floor and the
///   proportion both, that leaves the panel the clear majority.
/// * On a terminal too narrow for the floor to be honoured (under twice
///   [`TREE_MIN_WIDTH`]) the cap wins and the floor is given up: the two panes
///   halve what there is. The floor is the first thing dropped because the
///   alternative is a panel squeezed to nothing or a tree column wider than the
///   pane it was supposed to be a column beside; an even split is at least a
///   split both panes survive, and warlock is not usable at such a width
///   whichever way the columns are shared out.
///
/// So: 160 columns gives the tree 48 and the panel 112; 80 gives the tree its
/// floor of 30 and the panel 50; 40 gives the tree 20 and the panel 20; 41 gives
/// the tree 20 and the panel 21.
///
/// The panel's side of that is then cut again, top to bottom, and only then: the
/// composer is a pane under the panel and in the panel's column, so it is paid
/// for out of the panel's rows and out of nothing else — see [`split_column`].
/// The tree column and the footer never hear about it, and neither does the
/// width, which is why a document wrapped at [`panel_width`] is wrapped at the
/// width the composer is drawn at too.
fn areas(area: Rect, composer: Option<&Composer>) -> Areas {
    let [above, footer] =
        Layout::vertical([Constraint::Min(0), Constraint::Length(FOOTER_HEIGHT)]).areas(area);
    let column = tree_width(above.width);
    let [panel, tree] = Layout::horizontal([
        Constraint::Length(above.width - column),
        Constraint::Length(column),
    ])
    .areas(above);
    let (panel, composer) = split_column(panel, composer);

    Areas {
        panel,
        composer,
        tree,
        footer,
    }
}

/// Cut the composer's pane off the bottom of `column`, the panel's, and give
/// back what is left of the panel and what the composer got.
///
/// The composer asks for the rows its draft needs at the column's own inside
/// width — one when the draft is empty, one more per newline and per wrap, and
/// never more than [`COMPOSER_MAX_ROWS`], past which the field scrolls within
/// itself (see [`Composer::height`]) — plus the row its border costs top and
/// bottom. What it asks for is what it gets, and the panel above keeps the rest:
/// panel and composer together are the column, so the rows one gains are rows
/// the other lost and no row of the column goes unaccounted for.
///
/// Two things it does not get. It never takes the panel's own border with it:
/// the most it is given is the column less those two rows, so the panel is
/// squeezed to nothing before the composer is drawn in a column with no panel
/// left around it. And below [`COMPOSER_MIN_HEIGHT`] — a field with no row to
/// type on — it is not drawn at all and the panel keeps the whole column, which
/// is the same answer a frame with no composer gets. A border round nothing
/// would be furniture on the terminal that can least afford it.
fn split_column(column: Rect, composer: Option<&Composer>) -> (Rect, Option<Rect>) {
    let Some(composer) = composer else {
        return (column, None);
    };

    let wanted = composer
        .height(pane_inner(column).width)
        .saturating_add(2 * BORDER_THICKNESS);
    let rows = wanted.min(column.height.saturating_sub(2 * BORDER_THICKNESS));
    if rows < COMPOSER_MIN_HEIGHT {
        return (column, None);
    }

    let [panel, field] =
        Layout::vertical([Constraint::Min(0), Constraint::Length(rows)]).areas(column);

    (panel, Some(field))
}

/// The composer as a frame drawn over `app` has it: `Some` when the field is on
/// screen, `None` while the document card is showing.
///
/// One question asked in one place, because four things have to give the same
/// answer to it: what [`draw`] draws, what [`panel_height`] tells the app to
/// scroll the account by, what [`composer_height`] says that cost, and what
/// [`hit_test`] is pointing at. The rule itself is [`App::composer_showable`]'s
/// — the panel holds one card at a time and a document takes the whole column —
/// and it is asked here rather than stated again, so the frame agrees with the
/// app that is already keeping the keyboard off a hidden field (see
/// [`App::toggle_focus`]).
///
/// The app is borrowed only for the length of the call: what comes back borrows
/// the draft and nothing else, so a caller can measure the frame's composer and
/// still hand the app out mutably to draw it.
#[must_use]
pub fn composer_on_screen<'a>(app: &App, composer: &'a Composer) -> Option<&'a Composer> {
    app.composer_showable().then_some(composer)
}

/// [`composer_on_screen`] for a caller that may not have a field at all: the
/// same rule, over an `Option`.
///
/// [`draw`] takes the composer as an `Option` — a frame with no composer on it
/// is what a test not about the field draws — and filtering it again there is
/// belt and braces: whatever a caller hands in, a document card on the panel is
/// a frame with no field on it.
fn on_screen<'a>(app: &App, composer: Option<&'a Composer>) -> Option<&'a Composer> {
    composer.and_then(|field| composer_on_screen(app, field))
}

/// How many columns of `width` the tree column gets. The rule is [`areas`]'s and
/// is written out there.
fn tree_width(width: u16) -> u16 {
    let share = u32::from(width) * u32::from(TREE_PERCENT) / 100;
    let share = u16::try_from(share).unwrap_or(width);

    share.max(TREE_MIN_WIDTH).min(width / 2)
}

/// The inside of a pane: `area` less its border on all four sides.
///
/// Asked of the block itself rather than worked out here, so what is measured is
/// what the border widget will actually leave rather than a second opinion about
/// it. [`BORDER_THICKNESS`] is what that comes to, and the assertion is there so
/// that a border which ever stopped costing exactly that says so here — in a
/// test run — rather than by quietly drawing one row too many.
fn pane_inner(area: Rect) -> Rect {
    let inner = pane_block(false).inner(area);
    debug_assert_eq!(
        inner.height,
        area.height.saturating_sub(2 * BORDER_THICKNESS),
        "the pane border no longer costs {BORDER_THICKNESS} row top and bottom"
    );

    inner
}

/// The rows of `tree`, the tree pane's area, that the tree itself is drawn into:
/// what the border and the header leave.
///
/// Empty rather than negative on a pane with no room for either — a `Rect` of
/// zero height draws nothing, which is what a terminal that short should get.
fn tree_rows_area(tree: Rect) -> Rect {
    let [_header, rows] = Layout::vertical([Constraint::Length(HEADER_HEIGHT), Constraint::Min(0)])
        .areas(pane_inner(tree));

    rows
}

/// Cut the run's header off the top of `panel`, the panel pane's area, and give
/// back the row it got and the rows the account window keeps.
///
/// [`tree_rows_area`]'s counterpart, and the panel's one cut: the header is a
/// fixed line inside the border and above the window, so the window is what the
/// border and the header leave. Everything that has to agree about it comes
/// through here — what [`draw_panel`] draws, what [`panel_height`] tells the app
/// to scroll the account by, what [`run_header_height`] says that cost, and what
/// [`hit_test`] is pointing at — because a header measured in one place and
/// drawn from another is an account scrolled by rows it does not have.
///
/// `header` is the run in flight as the next frame has it, or `None` when no run
/// is running, which is when the window keeps the whole inside of the border. It
/// is the header's own value rather than a flag because the caller drawing the
/// row holds it anyway and a second question would be a second answer.
///
/// A header is only cut when a row is left over for the account under it. A
/// panel no taller than the header would otherwise be a header over nothing — a
/// run reporting its progress into a window with no room to report anything in —
/// so such a panel degrades to the bare account, exactly as it was before there
/// was a header to pay for.
fn panel_split(panel: Rect, header: Option<&RunHeader>) -> (Option<Rect>, Rect) {
    let inner = pane_inner(panel);
    if header.is_none() || inner.height <= RUN_HEADER_HEIGHT {
        return (None, inner);
    }

    let [header, rows] =
        Layout::vertical([Constraint::Length(RUN_HEADER_HEIGHT), Constraint::Min(0)]).areas(inner);

    (Some(header), rows)
}

/// The rows of `panel` the account's window is drawn into: what the border and
/// the run's header leave. The cut is [`panel_split`]'s and is written out
/// there.
fn panel_rows_area(panel: Rect, header: Option<&RunHeader>) -> Rect {
    panel_split(panel, header).1
}

/// How many rows of tree a terminal of `size` has room for, once the footer, the
/// tree pane's border and its header have taken theirs.
///
/// This is what [`App::set_viewport_height`] wants, and the only reason it is
/// public: the app's scroll offset is only right if it was computed against the
/// height the next frame actually gives the tree, and the layout is the one
/// thing that knows that height. The caller asks before it draws, so the offset
/// the frame reads was computed for the frame being drawn. It is measured off
/// the same [`areas`] call the frame is cut by, border included, so the answer
/// is the number of rows drawn rather than a count that happens to agree.
#[must_use]
pub fn tree_height(size: Size) -> u16 {
    tree_rows_area(areas(Rect::from(size), None).tree).height
}

/// How many lines of account a terminal of `size` has room for in the panel,
/// once the footer, the panel's own border, the composer under it and the run's
/// header above it have taken theirs.
///
/// [`tree_height`]'s counterpart, public for the same reason and measured the
/// same way: off the very [`areas`] call the frame is cut by and through the
/// very [`panel_rows_area`] cut the frame draws the window into, so the height
/// the app scrolls the panel's window by is the height the next frame draws it
/// at.
///
/// `composer` is the field the next frame will draw, or `None` for a frame with
/// no composer on it — which is what the panel gets back while the document card
/// is showing. `header` is the run the next frame will report, or `None` when no
/// run is in flight, which is when the account has those rows back too (see
/// [`App::run_header`]).
///
/// What the two of them cost is [`composer_height`] and [`run_header_height`],
/// and the three come to what the panel's column has inside it: a row is the
/// account's, the field's or the header's and never none of them.
#[must_use]
pub fn panel_height(size: Size, composer: Option<&Composer>, header: Option<&RunHeader>) -> u16 {
    panel_rows_area(areas(Rect::from(size), composer).panel, header).height
}

/// How many rows of `size`'s panel the run's header takes, and zero on a frame
/// with no run in flight — or one with no room for a header and an account both.
///
/// [`composer_height`]'s counterpart for the row above the window rather than
/// the pane below it, and measured off the same cut for the same reason:
/// `panel_height(size, composer, header) + run_header_height(size, composer,
/// header) + composer_height(size, composer)` is the height the panel's column
/// had inside it before either was paid for, at every terminal size.
#[must_use]
pub fn run_header_height(
    size: Size,
    composer: Option<&Composer>,
    header: Option<&RunHeader>,
) -> u16 {
    panel_split(areas(Rect::from(size), composer).panel, header)
        .0
        .map_or(0, |area| area.height)
}

/// How many rows of `size`'s panel column the composer takes, its border
/// included, and zero on a frame that has no composer on it.
///
/// [`panel_height`]'s other half and measured off the same [`areas`] call, which
/// is the whole point of it being here: the two are one cut of one column read
/// two ways, so `panel_height(size, composer) + composer_height(size, composer)`
/// is the height that column had inside it before there was a composer to pay
/// for — at every terminal size, including the short ones where the field is
/// squeezed or dropped altogether.
///
/// The border is counted in because the border is a row the column gave up. What
/// is left over inside it is the draft's, and how many rows the draft asked for
/// is [`Composer::height`]'s answer at [`panel_width`]'s width.
#[must_use]
pub fn composer_height(size: Size, composer: Option<&Composer>) -> u16 {
    areas(Rect::from(size), composer)
        .composer
        .map_or(0, |area| area.height)
}

/// How many columns wide the panel's contents are in a terminal of `size`, once
/// the panel's own border has taken its two.
///
/// [`panel_height`]'s counterpart and public for the counterpart of its reason:
/// a document is drawn in as many rows as its lines need at the panel's width
/// (see [`mod@crate::wrap`]), so the app can only say how many rows it holds if
/// it was told the width the next frame is about to draw them at. Measured off
/// the same [`areas`] call, so the width wrapped at is the width drawn at and a
/// row that fits one fits the other.
///
/// No composer is handed in and none is needed: the composer is cut off the
/// bottom of the panel's column and takes rows rather than columns, so the panel
/// is exactly as wide with a draft under it as without one — and the composer
/// itself is drawn at this very width, which is why [`Composer::height`] can be
/// asked for before the frame is cut.
#[must_use]
pub fn panel_width(size: Size) -> u16 {
    pane_inner(areas(Rect::from(size), None).panel).width
}

/// What is drawn at the point [`hit_test`] was asked about.
///
/// One variant per thing a pointer can be over, because the answers are acted on
/// differently and a caller that had to work out which was which from a pair of
/// numbers would be doing the layout's arithmetic a second time. The two that
/// carry an offset carry it from the top of their own window, not from the top
/// of the screen: what is *at* that offset is the app's business, since the app
/// owns both windows.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Hit {
    /// A point warlock does not draw: past the terminal's last column or last
    /// row. Nothing reports such a point, and a total answer beats an assumption
    /// that nothing ever will.
    Offscreen,
    /// The footer, whichever of its three lines. It is nobody's pane and takes
    /// no focus.
    Footer,
    /// A pane's border, either pane's, corners included — the columns and rows
    /// between the two panes' insides, which belong to neither.
    Border,
    /// The one line naming the tree, inside the tree pane's border and above its
    /// rows.
    TreeHeader,
    /// A row of the tree's window: `offset` rows below the first one on screen.
    ///
    /// A window offset and not a row of the tree. The app adds
    /// [`App::scroll_offset`] to get the row it means, and the app is also the
    /// one that knows whether it has that many rows: a window can be taller than
    /// the tree in it, and this says where the pointer is rather than what is
    /// under it.
    TreeRow {
        /// How many rows below the top of the tree's window the point is.
        offset: u16,
    },
    /// Inside the tree pane, below every row the window has room for.
    ///
    /// Asked of the layout rather than assumed away: the rows currently take
    /// everything the header leaves, so nothing lands here today, and a pane
    /// that ever grew a line of its own along the bottom would put points here
    /// rather than quietly hand out a row offset the tree's window does not
    /// have.
    TreeBelowRows,
    /// The one line naming the run in flight, inside the panel's border and
    /// above its window.
    ///
    /// [`Hit::TreeHeader`]'s counterpart and there for the same reason: the
    /// header is not a line of the account, so a point on it is not a line
    /// offset — answering with one would hand out the account's first line for a
    /// point drawn on a row the account does not own. Nothing lands here on a
    /// frame with no run in flight, because on such a frame no header is drawn
    /// and the window has the row.
    PanelHeader,
    /// A line of the panel's window: `offset` lines below the first one on
    /// screen. The whole of the window answers this, drawn on or not — the
    /// panel has no selection, so a point in it is a point in the panel.
    PanelLine {
        /// How many lines below the top of the panel's window the point is.
        offset: u16,
    },
    /// Inside the composer, under the panel and in the panel's column.
    ///
    /// No offset and nothing to count: the field has one cursor, it is always
    /// after the last character, and nothing on screen moves it — a row of a
    /// draft is not a place a reader can point at. What this says is that the
    /// point is in the field, which is the whole of what a pointer can mean
    /// there.
    ///
    /// A variant of its own rather than a [`Hit::PanelLine`] with a big offset:
    /// the composer's rows are rows the panel gave up, so a point on them is a
    /// point on a line the panel does not have, and answering with one would
    /// scroll a window the reader is not over.
    Composer,
}

/// What is drawn at column `column`, row `row` of a terminal of `size`.
///
/// The one place a screen point is turned into something warlock has a name
/// for, and the mouse's counterpart to [`tree_height`] and [`panel_height`]:
/// measured off the same [`areas`] call [`draw`] cuts the frame by, so what a
/// click lands on is what the reader saw at that point rather than what a second
/// opinion about the layout thinks is there.
///
/// A function of three numbers, the draft under the panel and the run reported
/// above it. No frame, no app state, no terminal — which is what lets the event
/// loop's answer to a click be tested with nothing attached to stdout, and what
/// keeps this file from needing to know what a row of the tree is.
///
/// `composer` is here for one reason: the rows it takes are rows the panel no
/// longer has, so a hit test that did not know about the draft would hand out
/// panel lines for points drawn on a field. It is the same `composer` the frame
/// was drawn with — `None` on a frame with no composer on it — and the caller
/// hands over the one the round measured, so the answer is about the frame the
/// reader is pointing at.
///
/// `header` is here for the same reason at the other end of the panel: the row
/// the run's header takes is a row the account gave up, so the line offsets are
/// counted from the top of the window the header left rather than from the top
/// of the border, and the header's own row answers [`Hit::PanelHeader`]. It is
/// the run the frame was drawn with, or `None` on a frame with no run in flight,
/// when the offsets are what they always were.
///
/// Every case is asked of a [`Rect`] the layout produced, so a terminal too
/// short for a tree row, too short for a header, or too short for anything but a
/// footer answers what it has rather than underflowing its way to a row that is
/// not there.
#[must_use]
pub fn hit_test(
    column: u16,
    row: u16,
    size: Size,
    composer: Option<&Composer>,
    header: Option<&RunHeader>,
) -> Hit {
    let point = Position::new(column, row);
    let screen = Rect::from(size);
    if !screen.contains(point) {
        return Hit::Offscreen;
    }

    let Areas {
        panel,
        composer,
        tree,
        footer,
    } = areas(screen, composer);
    if footer.contains(point) {
        return Hit::Footer;
    }

    if pane_inner(panel).contains(point) {
        let rows = panel_rows_area(panel, header);
        if row < rows.y {
            return Hit::PanelHeader;
        }
        return Hit::PanelLine {
            offset: row.saturating_sub(rows.y),
        };
    }

    if composer.is_some_and(|field| pane_inner(field).contains(point)) {
        return Hit::Composer;
    }

    let inside = pane_inner(tree);
    if inside.contains(point) {
        let rows = tree_rows_area(tree);
        if row < rows.y {
            return Hit::TreeHeader;
        }
        return if rows.contains(point) {
            Hit::TreeRow {
                offset: row.saturating_sub(rows.y),
            }
        } else {
            Hit::TreeBelowRows
        };
    }

    Hit::Border
}

/// The border a pane is drawn in, lit if it has the focus and dim if it has not.
///
/// A border and nothing else: no title, no padding. The lit border takes
/// [`FOCUS_COLOUR`], which is no node state's colour (see [`colour_for`]) — a
/// focused pane is not a state a node can be in, and a border that borrowed one
/// of the three colours would be a fourth thing those colours meant. The
/// unfocused border takes no colour at all, only [`Modifier::DIM`], so the
/// distinction reads on a terminal with no colour as well as on one with.
fn pane_block(focused: bool) -> Block<'static> {
    let style = if focused {
        Style::new().fg(FOCUS_COLOUR).add_modifier(Modifier::BOLD)
    } else {
        Style::new().add_modifier(Modifier::DIM)
    };

    Block::bordered().border_style(style)
}

/// Draw the panel: the window onto the card it is showing, one row per line,
/// inside its border.
///
/// One slot and three cards. The panel holds the account of the pact, the
/// conversation somebody is having and the document they asked to read, and
/// draws whichever of them is showing — which is the app's answer and never this
/// function's. [`App::panel_lines`] hands over the showing card's window and
/// [`App::panel_lines_below`] counts what is under it, so a swap (see
/// [`App::swap_card`]) changes what reaches the screen without changing a line of
/// the drawing. All three are drawn in the same border, at the same width, under
/// the same indicator: what mostly differs between them is what the lines say,
/// not how the panel says them.
///
/// Mostly, and not entirely, because a reader has to be able to tell which card
/// they are on. The thread is named on the top edge with [`thread_title`] and
/// the other two are not, which is one word on a row the border already owns —
/// and its rows say it a second time, since a question carries [`SAID_MARKER`]
/// and nothing on an account ever does (see [`panel_row`]). No colour does any
/// of this work: see the note at the end of this comment.
///
/// That name is computed rather than fixed, and the one thing it varies by is
/// the register the conversation is in ([`Mode`]): `thread` while questions are
/// being answered, `thread · brief` while it is converging on a document. It is
/// the only place on the screen the mode is said. Not the run header below it —
/// that row belongs to a pact, which can be started in either mode and would
/// collide with it — and not a row of the card, which the title deliberately
/// costs nothing of.
///
/// While no card has anything in it — before the first pact, the first question
/// and the first read — what this draws inside the border is [`MARK`], centred
/// and dim:
/// warlock's own `W` and not one word — no heading, no title, no welcome, no key
/// hints. A screen that said something before anything had happened would be
/// saying it about nothing; a screen carrying the program's mark is saying whose
/// screen it is, which is true before anything happens and stops being worth the
/// room the moment there is something to put there. [`App::has_panel_content`]
/// is the switch, and not the number of lines: an account that has started and
/// has nothing in it yet is a pact under way, and the mark does not come back
/// for it. A panel too small for the mark and its margins draws the bare border,
/// exactly as it always did.
///
/// With the account showing, every row is a line of it or the rest of one: a
/// section heading naming a directory, or one thing that pass was seen doing
/// with the elapsed clock of its own section in front of it, or the line the run
/// finished with. With the thread showing, every row is a line of the
/// conversation: a question somebody typed, one thing the model was seen doing
/// while it answered, or a row of the answer itself. With the document showing,
/// every row is a line of the file, from its first. Which rows those are is
/// [`App::panel_lines`]'s answer, window and all — the app owns the scrolling,
/// exactly as it owns the tree's — and this only words them and cuts them to the
/// width.
///
/// A [`Paragraph`] with no [`Wrap`](ratatui::widgets::Wrap): every line handed
/// over is one row, whatever is on it. Wrapping here would be the widget
/// deciding how many rows the panel holds, which is the app's answer — it is
/// what the window is cut out of and what the scrollback counts — so every card
/// arrives already broken into the rows its width needs (see
/// [`mod@crate::wrap`]). The count of rows on screen is therefore not the count
/// of things that happened; the clock in front of each of them is what says
/// where one ends and the next begins.
///
/// While the showing card's window is scrolled back, the bottom edge of the
/// border says how much of *that* card is below it and which key returns to
/// live. It goes on the border rather than on a row of its own, because a row of
/// its own would be a row taken off the card by the act of looking at it — and
/// it goes away the moment the window is back at the end, since an indicator
/// that always says `0 more` is furniture rather than information. It counts the
/// card on screen and not the slot: what is under the card behind is no part of
/// what the reader is scrolling through, so a swap changes the number on the
/// edge or takes it away.
///
/// While a run is in flight the top row inside the border is not the window at
/// all but the run's header — which run, which directory, how far through, and a
/// bar filled to that fraction (see [`draw_run_header`]). It is fixed there: the
/// window under it is a row shorter for as long as the run lasts, so scrolling
/// the account back moves the lines and leaves the header where it is. The cut
/// is [`panel_split`]'s, which is also what told the app how tall its window is,
/// so the header costs the account exactly the row it takes and no line is
/// scrolled past unseen. The moment the run is over there is no header, and the
/// window has the row back.
///
/// No colour anywhere in here. The three node-state colours are the tree's and
/// [`FOCUS_COLOUR`] is the border's; a fourth meaning for colour would cost both
/// of those their meaning. Bold, which is not a colour, is all the headings get
/// — the account's, the run's, the thread's title and a reader's question — and
/// dim, which is not one either, is all the mark and the indicator get. Telling
/// the cards apart is a word and a marker for that reason: a card that was
/// recognised by its colour would be a card nobody could recognise on a terminal
/// without one.
fn draw_panel(frame: &mut Frame<'_>, area: Rect, app: &App, now: Instant) {
    let below = app.panel_lines_below();
    let mut block = pane_block(app.focus() == Focus::Panel);
    if app.showing_thread() {
        block = block.title_top(Line::from(thread_title(app.mode())).bold());
    }
    if below > 0 {
        block = block.title_bottom(Line::from(scrollback(below)).right_aligned().dim());
    }

    let header = app.run_header();
    let (header_area, inner) = panel_split(area, header.as_ref());
    frame.render_widget(block, area);

    if let (Some(area), Some(header)) = (header_area, header.as_ref()) {
        draw_run_header(frame, area, header);
    }

    if !app.has_panel_content() {
        draw_mark(frame, inner);
        return;
    }

    let rows: Vec<Line<'static>> = app
        .panel_lines(now)
        .iter()
        .map(|line| panel_row(line, inner.width))
        .collect();
    frame.render_widget(Paragraph::new(rows), inner);
}

/// Draw the run's header: the one line [`run_header_line`] words for the width
/// there is.
///
/// Bold, like the account's own headings and like the tree's header, and for the
/// same reason: it is a heading rather than a thing that happened, and every
/// colour on this screen already means a node state. Nothing here reads a clock
/// and nothing is handed one — what the line says is what the run has said, so
/// two frames drawn at two instants with no event in between draw the same row.
fn draw_run_header(frame: &mut Frame<'_>, area: Rect, header: &RunHeader) {
    // Into the whole of the area it was given, which is the line's row and the
    // blank one under it (see [`RUN_HEADER_HEIGHT`]): one line drawn into two
    // rows leaves the second one as the border cleared it, which is the gap.
    frame.render_widget(
        Paragraph::new(Line::from(run_header_line(header, usize::from(area.width))).bold()),
        area,
    );
}

/// The run header's line for a panel `width` columns wide: which run is going,
/// the directory it is working, its position out of the run's total, and a bar
/// filled to that fraction in the columns left over.
///
/// Two facts of unequal standing, like the tree header's, and the same rule
/// settles them. The words are written out as they stand — the verb, the
/// directory as [`App::label_for`] spells it, and the fraction in the
/// parentheses the footer puts it in — and the bar is offered the room left over
/// after them and a column of gap. Below [`BAR_MIN_WIDTH`] there is no bar at
/// all rather than a stub of one, and the words keep the width; the whole row is
/// then cut once, by [`panel_row`]'s [`truncated`], so a directory too long for
/// the panel costs the line its tail rather than the row its shape.
///
/// The bar is `position/total` of the columns it was given, rounded down, and is
/// nothing else: no clock is read, no instant is taken, nothing is interpolated
/// between two fractions and nothing is estimated from them. Rounded down so
/// that a full bar means a run that has reached its last directory and never
/// merely one that is near it; and `position` is the run's high-water mark (see
/// [`RunHeader`]), so within one run the fill cannot fall back. A `total` of
/// none is a run with nothing to count, drawn empty rather than divided by.
fn run_header_line(header: &RunHeader, width: usize) -> String {
    let words = format!(
        "{} {} ({}/{})",
        run_word(header.run()),
        header.directory(),
        header.position(),
        header.total(),
    );

    let room = width
        .saturating_sub(display_width(&words))
        .saturating_sub(display_width(BAR_GAP));
    if room < BAR_MIN_WIDTH {
        return truncated(&words, width);
    }

    truncated(&format!("{words}{BAR_GAP}{}", bar(header, room)), width)
}

/// The word the header calls `run` by: the footer's verb for the same run.
const fn run_word(run: Run) -> &'static str {
    match run {
        Run::Pact => PACTING_RUN,
        Run::Refresh => REFRESHING_RUN,
    }
}

/// A bar `columns` wide, filled to `header`'s fraction of it. The rule is
/// [`run_header_line`]'s and is written out there.
fn bar(header: &RunHeader, columns: usize) -> String {
    let filled = header
        .position()
        .saturating_mul(columns)
        .checked_div(header.total())
        .unwrap_or(0)
        .min(columns);

    format!(
        "{}{}",
        BAR_FILLED.repeat(filled),
        BAR_EMPTY.repeat(columns - filled)
    )
}

/// Draw [`MARK`] in the middle of `inner`, or draw nothing if it does not fit.
///
/// Dim and with no foreground colour, like everything else in the panel: the
/// mark is what is there while nothing has happened, and something that had to
/// be looked past once the account arrived would have been drawn too loud.
fn draw_mark(frame: &mut Frame<'_>, inner: Rect) {
    let Some(area) = mark_area(inner) else {
        return;
    };

    let rows: Vec<Line<'static>> = MARK.iter().map(|row| Line::raw(*row)).collect();
    let style = Style::new().add_modifier(Modifier::DIM);
    frame.render_widget(Paragraph::new(rows).style(style), area);
}

/// Where [`MARK`] is drawn inside `inner`, or `None` when there is not the room
/// for the whole of it and its margins.
///
/// Centred on both axes. Where the rows left over do not halve evenly the spare
/// one goes below, so the mark sits a hair high — which is where the eye expects
/// the middle of a rectangle to be, and where a mark that sat a row low would
/// read as having slipped.
///
/// All of the art or none of it: there is no scaled variant and no second
/// smaller mark, because a mark that changed shape with the terminal would be
/// two marks and a reader would have to learn that both are warlock.
///
/// [`MARK`] is wide enough that "none of it" is the answer on an ordinary
/// terminal — see the size note there. This function is not where that is
/// decided and does not soften it: what it owns is that the panel falls back to
/// the bare border it drew before there was a mark at all, which is a panel with
/// nothing missing from it rather than a mark with something missing from it.
fn mark_area(inner: Rect) -> Option<Rect> {
    let width = MARK.iter().copied().map(display_width).max().unwrap_or(0);
    let width = u16::try_from(width).ok()?;
    let height = u16::try_from(MARK.len()).ok()?;

    if inner.width < width.saturating_add(2 * MARK_MARGIN)
        || inner.height < height.saturating_add(MARK_MARGIN_ROWS)
    {
        return None;
    }

    Some(Rect {
        x: inner.x + (inner.width - width) / 2,
        y: inner.y + (inner.height - height) / 2,
        width,
        height,
    })
}

/// Draw the composer: the tail of the draft, one row per row it wraps to, inside
/// a border lit exactly when the field is `live` — the keys pointed at it and it
/// taking them.
///
/// [`pane_block`] and no other border, so the field is lit and dimmed by the
/// rule the two panes above it already follow — three places the keys can be and
/// one lit border between them, which is what makes the focus readable at all
/// (see [`Focus`]).
///
/// `live` rather than `focused`, and that is the whole of what muting looks
/// like: while a turn is being answered the field keeps the keyboard and hears
/// nothing with it, so it is drawn as a field nobody is pointed at — dim border,
/// no caret — and lights again the moment the answer lands. It is drawn rather
/// than announced: there is no second wording for it and no placeholder in the
/// box, because a line of prose explaining a dim border would be warlock talking
/// about itself over the answer somebody is waiting for.
///
/// The rows are [`Composer::window`]'s: the *tail* of the draft, so the row the
/// cursor is on is the last row drawn and a draft past
/// [`COMPOSER_MAX_ROWS`](crate::COMPOSER_MAX_ROWS) scrolls within the field
/// rather than growing it. They are cut to the rows the border actually left, in
/// case a short terminal squeezed the box below what the draft asked for — from
/// the bottom again, and for the same reason.
///
/// The caret is a reversed [`COMPOSER_CURSOR`] after the last character, drawn
/// only while the field is live: it says where the next character lands, and
/// while the keys are somewhere else — or the field is not hearing them —
/// nothing is landing. It is dropped rather than wrapped when the last row
/// fills the width — a caret is not text, and a row of its own for a cursor
/// would take a row off the draft and move everything above it while somebody
/// is typing.
///
/// No colour and no prompt glyph. The panel above spends none, the draft is the
/// reader's own words, and a `>` in front of them would be a column off every
/// row of a field this narrow for the sake of saying what the lit border says.
fn draw_composer(frame: &mut Frame<'_>, area: Rect, composer: &Composer, live: bool) {
    let inner = pane_inner(area);
    frame.render_widget(pane_block(live), area);

    let rows = composer.window(inner.width);
    let from = rows.len().saturating_sub(usize::from(inner.height));
    let rows = &rows[from..];
    let room = rows
        .last()
        .is_some_and(|row| u16::try_from(display_width(row)).unwrap_or(u16::MAX) < inner.width);

    let mut lines: Vec<Line<'static>> = rows.iter().map(|row| Line::raw(row.clone())).collect();
    if live
        && room
        && let Some(last) = lines.last_mut()
    {
        last.push_span(Span::styled(
            COMPOSER_CURSOR,
            Style::new().add_modifier(Modifier::REVERSED),
        ));
    }

    frame.render_widget(Paragraph::new(lines), inner);
}

/// What the bottom edge of a scrolled-back panel says: how many lines are below
/// the view, and the key back to the newest one.
///
/// `↓ 214 more (G)`, padded a column each side so it does not sit against the
/// border's corner. Counted in lines rather than in screenfuls or in a
/// percentage, because a line is the thing the reader is scrolling past.
fn scrollback(below: usize) -> String {
    format!(" {SCROLLBACK_ARROW} {below} more ({LIVE_KEY}) ")
}

/// One line of the panel's contents as one row of it, cut to `width`.
///
/// What goes in front of a line and whether it is bold is [`shape`]'s answer,
/// not this function's, and that is the whole of why it is a call: the app wraps
/// a line at the width the prefix leaves it (see [`mod@crate::wrap`]), and a
/// second opinion here about what that prefix is would be a line broken at one
/// width and drawn at another. A heading is the directory's path, bold and flush
/// left; a clocked line is its elapsed time and what happened, indented under
/// the heading it belongs to; the summary is the run's last word, flush left and
/// bold like a heading because it is about the whole run rather than about any
/// one directory; a question carries [`SAID_MARKER`] and is bold like the
/// headings it stands among, since it is the heading of its turn.
///
/// A line of a document is the file's own text, flush left, unindented and
/// unstyled — nothing is added to it and nothing is taken off it. So is a
/// model's answer, because prose is prose, and it reaches here only on the
/// thread's card: nothing an [`Account`] holds is an [`Entry::Text`], so no
/// arrangement of runs and swaps can put a sentence of a model's on the account.
///
/// Everything arrives already broken into the rows this width needs, one row per
/// row the app counted, so on the frames the binary draws — which tell the app
/// the width first, every time — [`truncated`] takes nothing off anything. The
/// call stays because [`draw`] takes an [`App`] rather than a promise about one:
/// a caller that never measured the panel, or measured a different one, gets a
/// row inside the border rather than a row over it. See [`App::show_document`].
///
/// The row is built whole and cut once, rather than assembled from a styled
/// clock and a styled text: the width is a fact about the row, and two spans
/// each guessing at their share of it is how a line ends up one column too wide.
fn panel_row(line: &Entry, width: u16) -> Line<'static> {
    let shape = shape(line);
    let row = Line::from(truncated(
        &format!("{}{}", shape.prefix, shape.text),
        usize::from(width),
    ));
    if shape.heading { row.bold() } else { row }
}

/// `text`, cut to `width` columns with an [`ELLIPSIS`] where it was cut.
///
/// Columns, not bytes and not characters: a path with an accent in it takes
/// fewer columns than bytes and a CJK name takes more columns than characters,
/// and a row measured in either of the wrong ones is a row that overflows into
/// its neighbour or stops short of the edge. Measured with [`Line::width`],
/// which is the same measurement the terminal backend lays the row out with, so
/// what is cut here fits there exactly.
///
/// Cut on a character boundary, so this can never panic on a multi-byte path;
/// a character that is part of a longer grapheme cluster can still be separated
/// from what follows it, which costs a glyph its accent in the worst case and
/// never costs a row its shape.
fn truncated(text: &str, width: usize) -> String {
    if display_width(text) <= width {
        return text.to_owned();
    }
    if width == 0 {
        return String::new();
    }

    let budget = width.saturating_sub(display_width(ELLIPSIS));
    let mut taken = 0;
    let mut end = 0;
    for (index, character) in text.char_indices() {
        let next = index + character.len_utf8();
        let columns = display_width(&text[index..next]);
        if taken + columns > budget {
            break;
        }
        taken += columns;
        end = next;
    }

    format!("{}{ELLIPSIS}", &text[..end])
}

/// How many columns `text` takes on screen.
///
/// Asked of ratatui rather than worked out here, and asked of a borrowed span so
/// that measuring costs no allocation: the renderer's own measurement is the one
/// that decides whether a row fits, so a second opinion about it would only ever
/// be wrong.
pub(crate) fn display_width(text: &str) -> usize {
    Span::raw(text).width()
}

/// Draw the tree pane: its border, the header naming the tree inside the top of
/// it, and the window onto the rows under that.
///
/// `now` is only passed through: nothing about the border or the header moves
/// with the clock, and the rows under them do — see [`draw_tree`].
fn draw_tree_pane(frame: &mut Frame<'_>, area: Rect, app: &App, chrome: &Chrome, now: Instant) {
    let inner = pane_inner(area);
    frame.render_widget(pane_block(app.focus() == Focus::Tree), area);

    let [header_area, rows_area] =
        Layout::vertical([Constraint::Length(HEADER_HEIGHT), Constraint::Min(0)]).areas(inner);
    draw_header(frame, header_area, chrome);
    draw_tree(frame, rows_area, app, now);
}

/// Draw the header: which tree this is and what this machine holds for it, as
/// [`header_line`] composes the two for the width there is.
///
/// Bold rather than coloured, because every colour on this screen already
/// means a node state and the header is not a node — and a holding is not one
/// either, so it is drawn in the header's own weight rather than picking up a
/// colour or a mark of its own.
fn draw_header(frame: &mut Frame<'_>, area: Rect, chrome: &Chrome) {
    frame.render_widget(
        Paragraph::new(Line::from(header_line(chrome, usize::from(area.width))).bold()),
        area,
    );
}

/// The header's line for a pane `width` columns wide: the tree on screen, and
/// what this machine holds stated after it when there is room for both.
///
/// Two facts of unequal standing, and this function is the whole of the
/// inequality. The identity — the module [`Chrome::of`] worded, which is
/// the answer to "what am I looking at" — is written out as it stands, whatever
/// the width; the holding is offered the room left over and dropped entirely
/// when it does not fit. Dropped rather than cut, because half a set of sigils
/// is a claim about what is held that is not true, while no sigils at all is
/// the header this screen has always had.
///
/// So the decision is made here, on two strings, and never by cutting a joined
/// one: joining first and truncating after would spend the identity's columns on
/// the holding and end the line in an [`ELLIPSIS`] where the name of the module
/// used to be. Nothing is truncated here at all — an identity too long for the
/// pane is clipped by the widget exactly as it was before there were sigils.
///
/// [`Sigils::Nothing`](crate::app::Sigils::Nothing) has no wording
/// ([`Sigils::line`](crate::app::Sigils::line)), so a machine that
/// holds nothing gets the identity and nothing else, byte for byte the line it
/// got before this existed — including the empty one a tree rooted at the
/// repository root draws.
fn header_line(chrome: &Chrome, width: usize) -> String {
    let identity = chrome.header();
    let Some(holding) = chrome.sigils().line() else {
        return identity.to_owned();
    };

    let both = if identity.is_empty() {
        holding
    } else {
        format!("{identity}{HEADER_GAP}{holding}")
    };
    if display_width(&both) <= width {
        both
    } else {
        identity.to_owned()
    }
}

/// Draw the window onto the flattened tree — the rows from the app's scroll
/// offset that fit in `area` — one per line, with the selected one highlighted.
///
/// The window is applied here, by slicing the rows, rather than handed to the
/// widget as a scroll offset to interpret. A `List` given every row would scroll
/// itself to keep the selection visible, on its own rule, and there would then
/// be two answers to which rows are on screen — the app's and the widget's — of
/// which only the app's is the one the page keys move by. Slicing leaves the
/// widget nothing to scroll: it is handed at most `area.height` items, so what
/// it draws is exactly the window [`App::scroll_offset`] describes.
///
/// A selection outside that window cannot happen for an app told the height
/// this frame was laid out with, but if it ever does, nothing is highlighted
/// rather than the wrong row.
///
/// While a pact is running, the rows the pass covers are drawn in the pulsing
/// colour [`pulse_colour`] works out for this frame instead of their own state
/// colour: the row of the directory the run is inside right now, and the rows
/// of the files that directory holds — the very files the pass is reading. The
/// colour is computed once for the frame and then offered to exactly the rows
/// [`App::in_flight_covers`] says yes to; ancestors, siblings and child
/// *directories* — passes of their own, already finished — are never offered it
/// and keep the colour their state paints.
fn draw_tree(frame: &mut Frame<'_>, area: Rect, app: &App, now: Instant) {
    let first = app.scroll_offset().min(app.rows().len());
    let height = usize::from(area.height);
    let pulse = pulse_colour(app, now);
    let guides = guide_prefixes(app.rows(), first, height);

    // The columns a row's own text has to fit a scope label into: the drawn
    // width of the rows area, less the gutter `List` keeps for
    // `SELECTION_MARKER`. That gutter is subtracted for *every* row and not
    // only the selected one, even though the widget only writes the marker into
    // the selected row's copy of it, because this is the one width the whole
    // frame is measured against: were it counted per row, moving the selection
    // onto a row would take two columns off it and a label sitting on the
    // boundary would appear and disappear as the reader moved. A label that
    // comes and goes with the cursor reads as a fact about the tree changing,
    // which it is not.
    let text_width = usize::from(area.width).saturating_sub(display_width(SELECTION_MARKER));
    let items: Vec<ListItem<'_>> = app.rows()[first..]
        .iter()
        .take(height)
        .enumerate()
        .map(|(offset, row)| {
            ListItem::new(line(
                row,
                &guides[offset],
                app.can_collapse(first + offset),
                app.is_collapsed(&row.path),
                pulse.filter(|_| app.in_flight_covers(row)),
                text_width,
            ))
        })
        .collect();
    let list = List::new(items)
        .highlight_symbol(SELECTION_MARKER)
        .highlight_style(Style::new().add_modifier(Modifier::REVERSED | Modifier::BOLD));

    // The selection lives in the app state, counted from the top of the tree;
    // the widget counts from the top of its own items, which start at `first`.
    // This widget-local state is just how a `List` is told about it, and is
    // thrown away with the frame.
    let selected = app
        .selected_row()
        .map(|_| app.selected())
        .filter(|index| (first..first + height).contains(index))
        .map(|index| index - first);
    let mut state = ListState::default().with_selected(selected);
    frame.render_stateful_widget(list, area, &mut state);
}

/// One line of the tree: indented by depth, marked by whether its children are
/// hidden, named by its last path component, coloured by its state.
///
/// The indentation already spells out the ancestry, so repeating the full
/// path on every line would be noise; a path with no final component (a bare
/// root) falls back to printing itself, because a blank line is worse than a
/// long one.
///
/// The marker goes inside the indent, so it sits with the node it describes and
/// every sibling's name starts in the same column whichever marker it carries —
/// see [`NO_MARKER`]. Whether the row is collapsed is passed in rather than read
/// off the row: which nodes are collapsed is view state the app owns, and a
/// [`Row`] describes the tree, which knows nothing about it.
///
/// Two spans, and only two. The first is `guides`, worked out for the whole
/// window by [`guide_prefixes`] because it is the one thing about a row that
/// cannot be read off the row: it takes the rows after this one to know whether
/// a branch carries on. It is one [`INDENT`]-wide unit per level of nesting, so
/// the marker column and the first column of every name are exactly where they
/// were before the guides arrived. A depth-0 row draws no guide at all and its
/// span is empty. The second span is the marker and the name, which take the
/// row's state colour together: colour on this screen means node state and
/// nothing else, so a marker in a colour of its own would be a second thing
/// colour meant.
///
/// The guides take [`GUIDE_COLOUR`] and no modifier — not `DIM`, which is the
/// obvious way to ask for a quieter line. `DIM` is honoured inconsistently
/// across terminals, and a guide that vanishes on one and shouts on another is
/// worse than one dim colour everywhere, so the dimness is pinned into the
/// colour itself. They are a span of their own rather than part of the row's
/// text because they are not state: a guide in the row's colour would make depth
/// look like something the engine had decided.
///
/// `pulse` is the marker-and-name style overridden for this frame, and is `Some`
/// for at most one row of the tree: the directory a pact is working right now,
/// which alternates between the two pacted colours rather than sitting in the
/// stale one for the whole run (see [`pulse_colour`]). It never reaches the
/// guides, which say depth and have nothing to report about a run. It is still
/// only a foreground colour, so the selection — a `REVERSED`/`BOLD` modifier the
/// `List` applies over the whole row, guides included — reads exactly the same on
/// a pulsing row as on any other, and the pulse cannot move it.
///
/// `collapsible` is whether the row has anything under it *in this view*, which
/// is [`App::can_collapse`]'s answer and not the tree's child count. The marker
/// and the collapse key have to agree — a `+` on a row space refuses to open is
/// the screen making a promise the keyboard then breaks — so both ask the one
/// question, and neither asks [`Row::children`], which is a fact about the tree
/// that the file toggle and the pacted-only filter can each make untrue of the
/// screen.
///
/// A file row needs no case of its own here and deliberately does not get one.
/// Nothing is ever under a file, so it falls into [`NO_MARKER`] like any other
/// row holding nothing, and its depth is already one deeper than its
/// directory's, so it indents under it. Its colour is its directory's state,
/// copied onto the row when the tree was flattened (see [`Row::file`]), which is
/// how the design doc's rule that a file takes its module's colour arrives here
/// as an ordinary row with an ordinary colour.
///
/// A row carrying a scope of its own reads `<name> (<scope>)`, and the label
/// goes inside that same second span rather than beside it in one of its own:
/// name, space and parentheses are one run of text in one colour, so there is
/// nothing here for a fourth colour or a modifier of its own to creep into, and
/// the label cannot end up styled differently from the name it belongs to. The
/// row's own scope and nothing else appears — [`Row::scope`] is never an
/// ancestor's — so a file, a gray unpacted row and a directory covered only from
/// above all fall out of it being `None` without this asking what they are.
///
/// `width` is the columns the row's text has, and a label that does not fit in
/// them is dropped whole rather than cut: half a scope names a boundary that
/// does not exist, while no label at all is the row this screen drew before
/// scopes. The name is handed on untouched either way — it is never shortened to
/// make room — so a row whose label is dropped is byte for byte the row it
/// always was.
fn line(
    row: &Row,
    guides: &str,
    collapsible: bool,
    collapsed: bool,
    pulse: Option<Color>,
    width: usize,
) -> Line<'static> {
    let name = row
        .path
        .file_name()
        .unwrap_or(row.path.as_os_str())
        .to_string_lossy();
    let marker = match (collapsible, collapsed) {
        (false, _) => NO_MARKER,
        (true, true) => COLLAPSED_MARKER,
        (true, false) => EXPANDED_MARKER,
    };

    let mut text = format!("{marker}{name}");
    if let Some(scope) = &row.scope {
        let labelled = format!("{text} ({scope})");
        if display_width(guides) + display_width(&labelled) <= width {
            text = labelled;
        }
    }

    Line::from(vec![
        Span::styled(guides.to_owned(), GUIDE_COLOUR),
        Span::styled(text, pulse.unwrap_or(colour_for(row.state))),
    ])
}

/// The guide prefix for each of the `height` rows of `all` starting at `first`,
/// one string per drawn row, each [`INDENT`] wide per level of nesting.
///
/// A row's own guide is [`GUIDE_BRANCH`] or [`GUIDE_LAST`] according to whether
/// the directory holding it has another row after this one, and the levels
/// above it are [`GUIDE`] where that ancestor's branch carries on below and
/// blank where it has ended. Blank is the whole point of the exercise: a column
/// of unbroken verticals says a subtree is still open long after it has closed,
/// and the reader has to count indents to find out otherwise.
///
/// Sibling here means *drawn* sibling. The rows handed in are the window's own
/// list, after the collapse, the file toggle and the pacted-only filter have had
/// it, so a guide describes the tree on screen rather than the tree on disk —
/// which is the only one the reader can check it against. A walk is depth first
/// and parents come before children, so a row's next sibling is the next row at
/// its depth before any row shallower than it, and no path comparisons are
/// needed for any of this.
///
/// Two passes and no per-row search. The first runs backwards over everything
/// from the end to `first`, and is what makes this a function over the list
/// rather than a method on a row: whether a branch carries on is a fact about
/// the rows *after* it, and a row cannot answer it alone. The second runs
/// forwards from the root, carrying a stack of which ancestors are still open,
/// and starts at the root rather than at `first` because the row at the top of a
/// scrolled window inherits its verticals from ancestors above the window.
fn guide_prefixes(all: &[Row], first: usize, height: usize) -> Vec<String> {
    let last = all.len().min(first.saturating_add(height));
    if first >= last {
        return Vec::new();
    }

    // Backwards: a row is its parent's last drawn row unless a row at its own
    // depth follows it before anything shallower does. `open` says a depth has
    // been seen since the last row shallower than it, and a row shallower than a
    // depth closes it — every row under that depth belongs to a different parent.
    let mut open: Vec<bool> = Vec::new();
    let mut continues = vec![false; last];
    for (index, row) in all.iter().enumerate().rev() {
        if row.depth >= open.len() {
            open.resize(row.depth + 1, false);
        }
        if index < last {
            continues[index] = open[row.depth];
        }
        open.truncate(row.depth + 1);
        open[row.depth] = true;
    }

    // Forwards: `stack[level]` is whether the ancestor sitting at that level has
    // rows of its own still to come, which is the vertical the levels above a
    // row are drawn with.
    let mut stack: Vec<bool> = Vec::new();
    let mut prefixes = Vec::with_capacity(last - first);
    for (index, row) in all.iter().enumerate().take(last) {
        stack.truncate(row.depth);
        stack.resize(row.depth, false);
        if index >= first {
            // The root's own column is never drawn — a depth-0 row carries no
            // guide — so the verticals start one level in, and the row's own
            // level is the corner rather than a vertical.
            //
            // Every unit is one `INDENT` wide: a glyph in the first column and
            // blanks for the rest of the level, whichever glyph it is. Built
            // from `INDENT` rather than from two literal columns so that the
            // guides keep step with the indent if it ever changes width.
            let level = INDENT.chars().count();
            let mut prefix = String::new();
            for open in stack.iter().skip(1) {
                prefix.push_str(if *open { GUIDE } else { " " });
                prefix.push_str(&" ".repeat(level - 1));
            }
            if row.depth > 0 {
                prefix.push_str(if continues[index] {
                    GUIDE_BRANCH
                } else {
                    GUIDE_LAST
                });
                prefix.push_str(&" ".repeat(level - 1));
            }
            prefixes.push(prefix);
        }
        stack.push(continues[index]);
    }
    prefixes
}

/// The colour the row of the directory in flight takes this frame, or `None`
/// when no pact is running and every row is simply its own state's colour.
///
/// The pulse is not stored anywhere and nothing is stepped: it is a function of
/// how long the pass on screen has been going, so the phase is worked out afresh
/// on every frame from `now` and the instant the account's open section was
/// opened at. Whole [`PULSE_PHASE`]s since that instant, even or odd: even is
/// stale, odd is fresh. Measuring it against the *section*'s start rather than
/// the run's is what makes each directory's pulse begin on stale — the account
/// opens a section as each pass starts, so the phase resets under every
/// directory and the handover from one to the next is visible as a colour that
/// goes back to yellow.
///
/// Both colours come from [`colour_for`], the one place a state's colour is
/// decided, so a change to the palette moves the pulse with it and this file
/// names no colour of its own.
///
/// A pact in flight with no open section — the moment between the keypress and
/// the first progress event, or a run whose account was closed while the app
/// still thinks something is in flight — draws steady stale. That is the colour
/// the keypress already painted the subtree in, so such a row is never blank and
/// never unstyled; it simply sits in the colour it would have had anyway until
/// its section opens and the pulse has a start to measure from.
fn pulse_colour(app: &App, now: Instant) -> Option<Color> {
    if !app.is_pacting() {
        return None;
    }

    let fresh = app
        .account()
        .and_then(Account::open_section_started)
        .is_some_and(|started| {
            let phases =
                now.saturating_duration_since(started).as_millis() / PULSE_PHASE.as_millis();
            phases % 2 == 1
        });

    Some(colour_for(if fresh {
        NodeState::PactedFresh
    } else {
        NodeState::PactedStale
    }))
}

/// Draw the tally of nodes by state, the keys that do something, and the one
/// line the app has to say about the last keystroke.
///
/// The message goes last, nearest the bottom of the screen, so the tally and
/// the keys sit where they always have. It is dim and uncoloured like the keys:
/// every colour on this screen already means a node state, and a sentence about
/// a keystroke is not a node. With no message the line is drawn blank rather
/// than skipped — see [`FOOTER_HEIGHT`].
///
/// A pact in flight takes both of the lower two lines and adds neither: the
/// progress line goes on the message line ahead of any message (the precedence
/// is [`App::pact_line`]'s, decided with the rest of the display state rather
/// than here) and the keys line becomes [`pacting_keys_line`]'s. Same three lines,
/// same heights, same places — a footer that grew while a pact ran would reflow
/// the tree under it on a keystroke that changed nothing about the tree, and
/// would do it in the middle of the one operation the reader is watching.
///
/// The keys line is also where the terminal's mouse capture is reported, by
/// naming the key that changes it with what the next press of it will do: see
/// [`keys_line`] and [`App::mouse_captured`]. Not on the message line, because
/// capture being off outlasts the keystroke that turned it off while a message
/// lasts until the next one.
///
/// The `Paragraph` is given no `.wrap`, deliberately: a line too long for the
/// terminal is cut at the right-hand edge rather than folded onto the line
/// below, so the footer is three lines whatever the app has put on them and it
/// is always the end of a line that is lost. Which end is worth losing is the
/// app's business and not decided here — [`App::pact_line`] puts the part that
/// answers a keystroke last for exactly this reason.
///
/// The keys line is the one line that never reaches that edge: it is laid out
/// for `area.width` by [`keys_line`] or [`pacting_keys_line`], which give whole
/// names up in [`KEY_DROP_ORDER`] and [`PACTING_KEY_DROP_ORDER`] until what is
/// left fits. Cut at the edge instead, the name each would lose first is the way
/// out of warlock and the way out of the run.
fn draw_footer(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let area = footer_text_area(area);
    let counts = app.counts();
    let mut tally = Vec::new();
    for state in NodeState::ALL {
        if !tally.is_empty() {
            tally.push(Span::raw("  "));
        }
        tally.push(Span::styled(
            format!("{} {}", counts.get(state), noun(state)),
            colour_for(state),
        ));
    }

    let width = usize::from(area.width);
    let keys = Line::from(if app.is_pacting() {
        pacting_keys_line(width)
    } else {
        keys_line(app.mouse_captured(), width)
    })
    .dim();

    let message = Line::from(
        app.pact_line()
            .unwrap_or_else(|| app.message().unwrap_or_default().to_owned()),
    )
    .dim();
    frame.render_widget(Paragraph::new(vec![Line::from(tally), keys, message]), area);
}

/// Where the footer's own text goes inside the band it was given: in from each
/// edge by the width of a pane's border.
///
/// The footer is nobody's pane and draws no border, so left alone its lines
/// would start in column zero — the column the panel's border owns, one to the
/// left of every row of text above it. Three lines flush against the edge under
/// a screen of lines that are not reads as a slip rather than as a band of its
/// own, and the fix is a column, not a border: a box around the footer would
/// cost two rows and make the tally look like a fourth card.
///
/// Taken off both edges rather than only the left, so the keys line — which is
/// laid out for the width it is handed — gives its names up at the width it is
/// actually drawn in, and a long message is cut where the panes end rather than
/// a column past them.
///
/// A band too narrow to inset comes back with nothing in it, which on a
/// terminal one or two columns wide is every line there is anyway.
fn footer_text_area(footer: Rect) -> Rect {
    Rect {
        x: footer.x + BORDER_THICKNESS,
        width: footer.width.saturating_sub(2 * BORDER_THICKNESS),
        ..footer
    }
}

/// What a state is called in the footer.
///
/// A total `match` rather than a `Display` impl on the engine's enum: how a
/// state is worded on screen is the renderer's business, and the engine should
/// not have to grow API for it.
const fn noun(state: NodeState) -> &'static str {
    match state {
        NodeState::Unpacted => "unpacted",
        NodeState::PactedStale => "stale",
        NodeState::PactedFresh => "fresh",
    }
}

/// Draw the gate on the way out: the question in a small bordered window in the
/// middle of `screen`, with `highlighted` lit.
///
/// The cells behind it are cleared first. A window drawn straight over the frame
/// would keep whatever the panel or the tree had put in the columns its own text
/// does not reach — a path ending inside the border, half a row of guides under
/// the answers — and the reader would be asked a question with the screen it is
/// about still legible through it. [`Clear`] is ratatui's own widget for exactly
/// this, so no cell is blanked by hand.
///
/// The two answers are drawn Yes then No, in that order, because
/// [`mod@crate::confirm`] makes Left and Right positional against it. The lit one
/// takes [`FOCUS_COLOUR`] reversed and bold — the colour that already means
/// "this is what the keys are driving" on a pane border, and the modifiers the
/// tree's own selection is drawn with, so nothing new has to be learnt and no
/// fourth colour is spent. The other is dim, which is what the unfocused pane
/// border is: the pair reads on a terminal with no colour as well as on one with,
/// which matters more here than anywhere else on the screen, because the
/// highlight is the whole of what says which way Enter goes.
///
/// The window is clamped rather than skipped on a terminal with no room for it —
/// see [`confirm_area`]. This is deliberately the opposite rule to
/// [`mark_area`]'s draw-it-whole-or-not: a mark that is not drawn costs nothing,
/// where a question that is not drawn is a mode the reader is in with nothing on
/// screen to say so, and every key they press then goes somewhere they cannot
/// see.
fn draw_confirm(frame: &mut Frame<'_>, screen: Rect, highlighted: Answer) {
    let area = confirm_area(screen);
    let block = Block::bordered().padding(Padding::symmetric(CONFIRM_MARGIN, CONFIRM_MARGIN_ROWS));
    let inner = block.inner(area);

    frame.render_widget(Clear, area);
    frame.render_widget(block, area);
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(CONFIRM_QUESTION).centered(),
            Line::default(),
            answers_line(highlighted),
        ]),
        inner,
    );
}

/// The two answers as one line, with `highlighted` lit.
///
/// Three spans and a fixed order: the gap is a span of its own rather than
/// padding on either answer, so neither highlight can grow into the space
/// between them and the two blocks stay the same shape as each other.
fn answers_line(highlighted: Answer) -> Line<'static> {
    let lit = Style::new()
        .fg(FOCUS_COLOUR)
        .add_modifier(Modifier::REVERSED | Modifier::BOLD);
    let unlit = Style::new().add_modifier(Modifier::DIM);
    let style = |answer: Answer| if answer == highlighted { lit } else { unlit };

    Line::from(vec![
        Span::styled(CONFIRM_YES, style(Answer::Yes)),
        Span::raw(CONFIRM_ANSWER_GAP),
        Span::styled(CONFIRM_NO, style(Answer::No)),
    ])
    .centered()
}

/// Where the confirmation is drawn on a terminal of `screen`: centred, and cut
/// down to the screen if the screen is smaller than the window.
///
/// A name of its own for what [`centred`] says about [`confirm_size`], because
/// the question's own place on the frame is what the tests about it ask for; the
/// centring and the clamping are [`centred`]'s, and are the same for the scope
/// prompt.
fn confirm_area(screen: Rect) -> Rect {
    centred(screen, confirm_size())
}

/// Where a window of `size` is drawn on a terminal of `screen`: in the middle of
/// it, and cut down to the screen when the screen is the smaller of the two.
///
/// Both windows over the frame place themselves through here, so the quit
/// confirmation and the scope prompt land in the same place and behave the same
/// way on a terminal too small for either.
///
/// Centred on both axes, with the odd spare row falling below the window and the
/// odd spare column to its right, so it sits a hair high and a hair left — which
/// is where the eye reads the middle of a rectangle to be, and is the same
/// rounding [`mark_area`] takes.
///
/// Clamped, never skipped. A terminal too narrow or too short for the whole
/// window gets as much of it as there is room for: at that point the border is
/// cut into, then the padding, then the text itself, and at one column by one row
/// what is left is a single cell of border — which is still the screen saying
/// that something is being asked. The alternative is worse than ugly: a window
/// that declined to draw would leave the reader in a mode with no sign of it,
/// pressing keys that reach nothing they can see, and the only way out of it
/// would be the one keystroke they cannot know is wanted.
///
/// Every arithmetic step is saturating or is guarded by the [`Ord::min`] above
/// it, so no size of terminal — a zero-width [`Rect`] included — underflows its
/// way to a window somewhere off the screen.
fn centred(screen: Rect, size: Size) -> Rect {
    let Size { width, height } = size;
    let width = width.min(screen.width);
    let height = height.min(screen.height);

    Rect {
        x: screen.x + (screen.width - width) / 2,
        y: screen.y + (screen.height - height) / 2,
        width,
        height,
    }
}

/// How big the whole window wants to be: the wider of its two lines, the margins
/// and the border.
///
/// The width is measured off the text rather than written down as a number, the
/// way [`mark_area`] measures the art: the question and the answers are constants
/// a few lines apart, and a window sized by hand would be the thing that stopped
/// agreeing with them when one of them was reworded. [`display_width`] rather
/// than a byte count, so a question with anything but ASCII in it is still sized
/// in the columns the backend will lay it out in.
fn confirm_size() -> Size {
    let answers =
        display_width(CONFIRM_YES) + display_width(CONFIRM_ANSWER_GAP) + display_width(CONFIRM_NO);
    let widest = display_width(CONFIRM_QUESTION).max(answers);
    let width = u16::try_from(widest)
        .unwrap_or(u16::MAX)
        .saturating_add(2 * CONFIRM_MARGIN)
        .saturating_add(2 * BORDER_THICKNESS);

    Size::new(width, CONFIRM_HEIGHT)
}

/// Draw a prompt somebody types one line into: what it is about, what has been
/// typed into it, and the sentence under it, in a bordered window over the
/// middle of `screen`.
///
/// Two windows, one function. `heading` goes in front of what the field carries
/// and `rules` on the last line, so the caller says which prompt this is —
/// [`SCOPE_HEADING`] and [`scope::RULES`](warlock_engine::scope::RULES) for the scope, [`PATH_HEADING`] and
/// [`PATH_RULES`] for the path a brief is about to be written to. Nothing else
/// differs: same [`Clear`], same border, same margins, same five lines, same
/// clamping on a terminal with no room, because a reader who has answered one of
/// them has learnt the other.
///
/// [`draw_confirm`]'s shape, down to the [`Clear`] before the border and the
/// clamping rather than skipping on a terminal with no room — a mode with
/// nothing on screen to say so is the same trap whichever question is being
/// asked. What differs is that this window is sized off a value rather than off
/// constants, because part of what it shows is being typed: see [`scope_size`].
///
/// Left-aligned, where the confirmation is centred. The field is a line somebody
/// is adding characters to, and a centred field would slide half a column left
/// on every other keystroke, taking the cursor and the text already typed with
/// it. Everything above and below it is aligned to the same edge, so the window
/// reads as one block rather than as a centred heading over a left-hung field.
fn draw_scope(frame: &mut Frame<'_>, screen: Rect, field: &ScopeField, heading: &str, rules: &str) {
    let area = centred(screen, scope_size(field, heading, rules));
    let block = Block::bordered().padding(Padding::symmetric(SCOPE_MARGIN, SCOPE_MARGIN_ROWS));
    let inner = block.inner(area);

    frame.render_widget(Clear, area);
    frame.render_widget(block, area);
    frame.render_widget(Paragraph::new(scope_lines(field, heading, rules)), inner);
}

/// The [`SCOPE_LINES`] lines of one prompt's window, in the order they are
/// drawn.
///
/// What the field carries is bold against the plain `heading`: the heading is
/// the same words every time the window opens and the path is the one part of it
/// worth reading, and bold is what the tree already spends on the row the keys
/// are driving, so no new colour is invented for a window that is up for a few
/// seconds. The path prompt hands in an empty heading and its whole first line
/// is that bold field (see [`PATH_HEADING`]), which is the same rule and not an
/// exception to it: the words worth reading are the words drawn bold.
///
/// The cursor is a reversed [`SCOPE_CURSOR`] after the text and nowhere else,
/// which is not a decision made here: [`mod@crate::prompt`] appends and deletes
/// at the end and moves nothing, so the end of the text is where the next
/// character lands, by construction.
///
/// The rules line is dim, as the footer's keys are: it is there to be read once,
/// before anything is typed, and then to stop competing with the text. The
/// broken rule above it is not dimmed — it is the one thing that changed since
/// the last frame, and it is the reason the prompt is still up.
fn scope_lines<'a>(field: &'a ScopeField, heading: &'a str, rules: &'a str) -> Vec<Line<'a>> {
    vec![
        Line::from(vec![
            Span::raw(heading),
            Span::raw(field.directory()).bold(),
        ]),
        Line::default(),
        Line::from(vec![
            Span::raw(field.text()),
            Span::styled(SCOPE_CURSOR, Style::new().add_modifier(Modifier::REVERSED)),
        ]),
        Line::from(field.rule().unwrap_or_default()),
        Line::from(rules).dim(),
    ]
}

/// How big one prompt's window wants to be: its widest line, the margins and the
/// border.
///
/// Measured off the lines the way [`confirm_size`] is, and off the field as well
/// as off the two sentences handed in, because a path and a broken rule are as
/// much of the window as the heading is. In practice the rules line is the floor
/// and the window does not breathe as somebody types: [`scope::RULES`](warlock_engine::scope::RULES) is wider
/// than a directory that fits on a tree row and wider than any scope the engine
/// would accept, so the width only moves for something longer than it. The path
/// prompt is the one window that does breathe — a proposed path is about as long
/// as [`PATH_RULES`] — and it breathes at the pace a path is edited, which is a
/// character at a time and no faster than the scope window already moves when a
/// refusal lands.
///
/// [`scope::RULES`](warlock_engine::scope::RULES) is the engine's sentence rather than one written here, and
/// that is a rule rather than a convenience: a window that spelled out how long
/// a scope may be or which characters it may hold would be this crate judging a
/// scope, and there is one judge — see [`mod@crate::prompt`]. Taking the
/// sentence as a parameter is what keeps that true of a function that now sizes
/// two windows: the caller says which words, and this measures them.
fn scope_size(field: &ScopeField, heading: &str, rules: &str) -> Size {
    let heading = display_width(heading) + display_width(field.directory());
    let typed = display_width(field.text()) + display_width(SCOPE_CURSOR);
    let widest = heading
        .max(typed)
        .max(field.rule().map_or(0, display_width))
        .max(display_width(rules));
    let width = u16::try_from(widest)
        .unwrap_or(u16::MAX)
        .saturating_add(2 * SCOPE_MARGIN)
        .saturating_add(2 * BORDER_THICKNESS);

    Size::new(width, SCOPE_HEIGHT)
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::time::{Duration, Instant};

    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;
    use ratatui::layout::{Position, Rect, Size};
    use ratatui::style::{Color, Modifier};
    use ratatui::widgets::{Paragraph, Widget};
    use warlock_engine::{NodeState, scope};

    use super::{
        Areas, BAR_EMPTY, BAR_FILLED, BAR_MIN_WIDTH, BORDER_THICKNESS, BRIEF_THREAD_TITLE,
        CANCEL_KEY, COLLAPSE_KEY, COMMAND_KEY, COMPOSE_KEYS, COMPOSER_MIN_HEIGHT,
        CONFIRM_ANSWER_GAP, CONFIRM_HEIGHT, CONFIRM_LINES, CONFIRM_MARGIN, CONFIRM_MARGIN_ROWS,
        CONFIRM_NO, CONFIRM_QUESTION, CONFIRM_YES, ELLIPSIS, FOCUS_KEY, FOOTER_HEIGHT, GUIDE,
        GUIDE_BRANCH, GUIDE_LAST, HEADER_GAP, HEADER_HEIGHT, Hit, INDENT, KEY_DROP_ORDER, KEY_GAP,
        KEYS, LEAVE_KEY, LIVE_KEY, MARK, MARK_MARGIN, MARK_MARGIN_ROWS, MOUSE_OFF_KEY,
        MOUSE_ON_KEY, MOVE_KEYS, NO_MARKER, NOTE_MARKER, PACTING_KEYS, PACTING_QUIT_KEY,
        PACTING_RUN, PAGE_KEYS, PANEL_INDENT, PATH_HEADING, PATH_RULES, QUIT_KEY, REFRESHING_RUN,
        RUN_HEADER_HEIGHT, SAID_MARKER, SCOPE_CURSOR, SCOPE_HEADING, SCOPE_HEIGHT, SCOPE_LINES,
        SCOPE_MARGIN, SCOPE_MARGIN_ROWS, SCROLLBACK_ARROW, SELECTION_MARKER, THREAD_TITLE,
        TREE_MIN_WIDTH, TREE_PERCENT, areas, centred, composer_height, composer_on_screen,
        confirm_area, confirm_size, display_width, draw, footer_text_area, guide_prefixes,
        hit_test, keys_line, mark_area, pacting_keys_line, pane_inner, panel_height, panel_row,
        panel_width, run_header_height, scope_size, tree_height, tree_rows_area, tree_width,
        truncated,
    };
    use crate::COMPOSER_MAX_ROWS;
    use crate::account::{Line as Entry, Outcome};
    use crate::app::{App, Chrome, Focus, Row, Run, Sigils};
    use crate::claude::Activity;
    use crate::colour::{FOCUS_COLOUR, GUIDE_COLOUR, colour_for};
    use crate::composer::Composer;
    use crate::confirm::{Answer, QuitConfirm};
    use crate::fixture;
    use crate::panel::Mode;
    use crate::prompt::{ScopeField, ScopePrompt};

    /// How many rows the window tests work with: comfortably more than fit on
    /// the terminal they draw into.
    const MANY: usize = 20;

    /// The terminal most of these tests draw into.
    ///
    /// Wide enough that the tree column is well clear of its floor and every
    /// line of the fixture fits inside its border — what is under test here is
    /// what the rows say, not how a name survives a narrow column — and tall
    /// enough for a header, a footer, the pane borders and a handful of tree
    /// rows, and no taller.
    const WIDTH: u16 = 120;
    /// See [`WIDTH`].
    const HEIGHT: u16 = 10;

    /// The height every row of the frame's chrome takes off the tree: the
    /// footer, the tree pane's border top and bottom, and its header.
    const CHROME_HEIGHT: u16 = FOOTER_HEIGHT + 2 * BORDER_THICKNESS + HEADER_HEIGHT;

    /// A terminal wide enough for the whole of [`keys_line`], whatever it grows
    /// to: the footer test asserts that line for equality, and a line drawn onto
    /// a narrower terminal than it needs would be compared against its own
    /// truncation.
    const KEYS_WIDTH: u16 = 240;

    /// The terminal the footer has to survive: eighty columns, which is what a
    /// split window on a laptop is and has been the default width of a terminal
    /// for longer than any of this.
    ///
    /// Narrower than the whole keys line by some way, so the line drawn here is
    /// one that has given names up — and the point of the tests that draw at
    /// this width is which name it kept.
    const EIGHTY_COLUMNS: u16 = 80;

    /// Tall enough for the whole fixture with its files on screen, chrome
    /// included, so a file test asserts about rows rather than about where the
    /// window happened to stop.
    const FILES_HEIGHT: u16 = 20;

    /// Tall enough for the whole of the fixture's default view on screen,
    /// chrome included: its five directories, the four document rows drawn
    /// under the documented ones, and then some.
    ///
    /// Nine rows rather than five, because the view the app opens on draws each
    /// directory's own `WARLOCK.md` under it — see [`WHOLE_FIXTURE`].
    const FIXTURE_HEIGHT: u16 = 16;

    /// Every line the fixture's default view draws, in order, exactly as it
    /// reaches the screen: the selection's gutter, the guides, the collapse
    /// marker's two columns and the name.
    ///
    /// Nine lines for five directories. The four that carry a document draw it
    /// under them as the ordinary file row `f` has always produced — one indent
    /// deeper than the directory, no collapse marker, nothing added to the name
    /// — and `crates/`, which has no document, draws nothing under itself. The
    /// three directories that hold only a document are marked expanded here
    /// because there is now something under them to hide.
    const WHOLE_FIXTURE: [&str; 9] = [
        "> - warlock",
        "  ├   WARLOCK.md",
        "  ├ - crates",
        "  │ ├ - engine",
        "  │ │ └   WARLOCK.md",
        "  │ └ - tui",
        "  │   └   WARLOCK.md",
        "  └ - assets",
        "    └   WARLOCK.md",
    ];

    /// The narrowest terminal the mark is drawn on: a hundred and fifty-one
    /// columns gives the tree forty-five and the panel a hundred and six, whose
    /// inside is a hundred and four — the art's hundred columns and
    /// [`MARK_MARGIN`] either side of it, exactly and not a column over.
    ///
    /// A wide terminal, and knowingly: see [`MARK`]. What this constant is for
    /// is that the threshold is a number in a test rather than something
    /// discovered by resizing a window.
    const MARK_WIDTH: u16 = 151;

    /// One column narrower than [`MARK_WIDTH`]: the tree takes forty-five at
    /// both widths, so the whole of the missing column comes off the panel and
    /// its inside is a column short of the art and its margins.
    const BELOW_MARK_WIDTH: u16 = MARK_WIDTH - 1;

    /// The shortest terminal the mark is drawn on: fourteen rows less the footer
    /// and the panel's border leaves nine, the art's eight and the
    /// [`MARK_MARGIN_ROWS`] row it wants clear.
    const MARK_HEIGHT: u16 = 14;

    /// One row shorter than [`MARK_HEIGHT`], leaving the panel's inside the
    /// art's own eight rows with none to spare.
    const BELOW_MARK_HEIGHT: u16 = MARK_HEIGHT - 1;

    /// A terminal with room to spare for the mark in both directions, for the
    /// tests that are about something else happening on a frame the mark is on —
    /// where drawing at the threshold would make a failure read as a rounding
    /// error rather than as the thing under test.
    const MARK_ROOM_WIDTH: u16 = 170;

    /// The height half of [`MARK_ROOM_WIDTH`], with rows to spare over the art
    /// and its margin.
    const MARK_ROOM_HEIGHT: u16 = 20;

    /// The 80-column terminal, where the tree takes its floor of thirty and the
    /// panel the other fifty: the width the mark has to survive to be drawn on
    /// an ordinary terminal at all.
    const STANDARD_WIDTH: u16 = 80;

    /// A terminal with room on the header for both of its facts: the tree pane
    /// takes its thirty per cent, forty-eight columns, and the border leaves
    /// forty-six inside — comfortably more than the thirty-three the fixture's
    /// identity, the gap and two sigils come to.
    ///
    /// Room to spare, and deliberately: what these tests are about is that the
    /// holding is stated when it fits, so drawing at the threshold would make a
    /// failure read as a rounding error rather than as the thing under test.
    const HELD_WIDTH: u16 = 160;

    /// The 40-column terminal, where the two panes halve the width and the
    /// panel is twenty columns: too narrow for the mark by a long way, and the
    /// size that pins what the panel does instead.
    const NARROW_WIDTH: u16 = 40;

    /// How far apart the pulse tests draw their frames: one phase, after which
    /// the row in flight should have changed colour, and the two phases that
    /// make a whole cycle, after which it should be back where it started.
    ///
    /// Written as wall-clock durations rather than as multiples of the
    /// `PULSE_PHASE` under test, because "about half a second" is what was
    /// asked for: a phase that quietly grew to two seconds would still be
    /// self-consistent with itself, and would fail here.
    const PHASE: Duration = Duration::from_millis(500);
    /// See [`PHASE`]: two of them.
    const CYCLE: Duration = Duration::from_secs(1);

    /// `count` rows of nothing in particular, named so that a line on screen
    /// says which row of the tree it is.
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

    /// The scope the label tests write on a row, and the name of the row it is
    /// written on.
    ///
    /// A name of some length on purpose: the width the labelled row needs has to
    /// be clear of the floor [`TREE_MIN_WIDTH`] puts under the tree column, or
    /// the boundary test would be asking for a terminal no width produces.
    const SCOPED_NAME: &str = "warlock-terminal-ui";
    /// See [`SCOPED_NAME`].
    const SCOPE_TEAM: &str = "tui-team";
    /// What [`SCOPED_NAME`] scoped to [`SCOPE_TEAM`] reads on screen: the
    /// name, a space, and the scope in parentheses.
    const SCOPED_LABEL: &str = "warlock-terminal-ui (tui-team)";

    /// A terminal with room to spare for [`SCOPED_LABEL`]: a hundred and sixty
    /// columns gives the tree forty-eight, which is well over the thirty-four
    /// the labelled row needs.
    ///
    /// Room to spare on purpose — the label tests are about what the row says,
    /// and drawing them at the threshold would make a failure read as a
    /// rounding error rather than as the thing under test. The threshold itself
    /// is what [`terminal_width_for`] is for.
    const SCOPE_ROOM_WIDTH: u16 = 160;

    /// A root, a directory carrying `scope`, a pacted directory under that one
    /// with no scope of its own, and a file inside the scoped directory: the
    /// four rows the label has to tell apart, all in `state`.
    ///
    /// Built as rows rather than from a tree, so what is under test is the
    /// renderer reading [`Row::scope`] and not a loader filling it in. `scope`
    /// is `None` for the same rows with nothing written on them, which is the
    /// screen as it was before there were labels.
    fn labelled_rows(scope: Option<&str>, state: NodeState) -> Vec<Row> {
        vec![
            Row::new(0, "warlock", "warlock/WARLOCK.md", NodeState::Unpacted).with_child_count(1),
            Row::new(
                1,
                format!("warlock/{SCOPED_NAME}"),
                format!("warlock/{SCOPED_NAME}/WARLOCK.md"),
                state,
            )
            .with_scope(scope.map(str::to_owned))
            .with_child_count(2),
            Row::new(
                2,
                format!("warlock/{SCOPED_NAME}/widgets"),
                format!("warlock/{SCOPED_NAME}/widgets/WARLOCK.md"),
                state,
            ),
            Row::file(2, format!("warlock/{SCOPED_NAME}/ui.rs"), state),
        ]
    }

    /// The colour and the modifiers of every cell `needle` is drawn in on tree
    /// row `index`.
    ///
    /// Read off the buffer cell by cell rather than trusted to a span, because
    /// what the reader sees is the cells: a label that arrived in a second span
    /// with a colour or an emphasis of its own would pass an assertion about the
    /// text and fail here.
    fn styles_of(buffer: &Buffer, index: u16, needle: &str) -> Vec<(Color, Modifier)> {
        let area = rows_area(buffer);
        let start =
            u16::try_from(column_of(&tree_row(buffer, index), needle)).expect("a narrow terminal");
        let width = u16::try_from(display_width(needle)).expect("a short needle");
        (start..start + width)
            .map(|column| {
                let cell = &buffer[(area.x + column, area.y + index)];
                (cell.fg, cell.modifier)
            })
            .collect()
    }

    /// The narrowest terminal whose tree draws its rows into exactly
    /// `rows_width` columns.
    ///
    /// Searched for rather than worked out, so a boundary test asks for the
    /// width it means — the one the rows are measured against — without
    /// restating the layout's own arithmetic beside it.
    fn terminal_width_for(rows_width: u16) -> u16 {
        (2 * TREE_MIN_WIDTH..=400)
            .find(|width| {
                tree_rows_area(areas(Rect::new(0, 0, *width, HEIGHT), None).tree).width
                    == rows_width
            })
            .unwrap_or_else(|| panic!("no terminal gives the tree's rows {rows_width} columns"))
    }

    /// An app of [`MANY`] rows with `selected` selected, measured for a
    /// [`WIDTH`]×[`HEIGHT`] terminal exactly the way the binary measures one.
    ///
    /// The selection is reached by stepping, so the offset it is drawn with is
    /// whatever ordinary movement left behind rather than something the test
    /// wrote in by hand.
    fn tall_app(selected: usize) -> App {
        let mut app = App::from_rows(many_rows(MANY));
        app.set_viewport_height(tree_height(Size::new(WIDTH, HEIGHT)));
        while app.selected() < selected {
            app.select_next();
        }
        app
    }

    /// The instant `seconds` after `base`, so a run can be drawn at an instant
    /// the test chose and its clocks asserted for equality.
    fn at(base: Instant, seconds: u64) -> Instant {
        base + Duration::from_secs(seconds)
    }

    /// The `line`th activity of a pass, as a tool call numbered so it is its
    /// own line.
    ///
    /// The tests below that need a long account need it to be long, and a
    /// stretch of thinking is one line however often it is reported — so what
    /// fills a panel is a sequence of distinguishable activities rather than
    /// the same one repeated.
    fn numbered(line: usize) -> Activity {
        Activity::Tool {
            name: "Read".to_owned(),
            detail: Some(format!("line {line}")),
        }
    }

    /// An app with the fixture's tree and an account of a pact that started at
    /// `base`, measured for a `width`×`height` terminal exactly the way the
    /// binary measures one.
    ///
    /// Nothing has happened in the run yet: the caller opens the sections and
    /// records the activities it wants, at the instants it wants, so that what
    /// is on screen is what a run put there rather than something a fixture
    /// arranged.
    fn pacting_app(base: Instant, width: u16, height: u16) -> App {
        let mut app = App::from_tree(&fixture::tree());
        app.set_viewport_height(tree_height(Size::new(width, height)));
        app.set_panel_height(panel_height(Size::new(width, height), None, None));
        app.set_panel_width(panel_width(Size::new(width, height)));
        app.start_account(base);
        app
    }

    /// Where the panel's lines land in a buffer of this size: the whole inside
    /// of the panel's border, which is the account's window and — while a run is
    /// in flight — the run's header on the top row of it.
    fn panel_area(buffer: &Buffer) -> Rect {
        pane_inner(areas(buffer.area, None).panel)
    }

    /// The rows of `buffer` the panel is drawn into, as text, without the tree
    /// pane's border on the end of them.
    fn panel_rows(buffer: &Buffer) -> Vec<String> {
        let area = panel_area(buffer);
        (0..area.height)
            .map(|index| text_in(buffer, area, area.y + index))
            .collect()
    }

    /// The panel's bottom border row, as text: the edge the scrollback
    /// indicator is written on, border glyphs and all.
    fn panel_bottom_edge(buffer: &Buffer) -> String {
        let panel = areas(buffer.area, None).panel;

        text_in(buffer, panel, panel.y + panel.height - 1)
    }

    /// The panel's top border row, as text: the edge the thread's own name is
    /// written on, border glyphs and all.
    fn panel_top_edge(buffer: &Buffer) -> String {
        let panel = areas(buffer.area, None).panel;

        text_in(buffer, panel, panel.y)
    }

    /// Where the tree's rows land in a buffer of this size: inside the tree
    /// pane's border, under its header. Measured off the layout the frame was
    /// cut by, so a test asserts about where the tree is rather than about where
    /// it used to be.
    fn rows_area(buffer: &Buffer) -> Rect {
        tree_rows_area(areas(buffer.area, None).tree)
    }

    /// The tree pane's header line: one row, inside the border, above the rows.
    fn header_area(buffer: &Buffer) -> Rect {
        let inner = pane_inner(areas(buffer.area, None).tree);

        Rect {
            height: HEADER_HEIGHT.min(inner.height),
            ..inner
        }
    }

    /// The rows of `buffer` the tree is drawn into, as text: the window onto the
    /// tree, inside the tree pane's border and under its header.
    fn tree_rows(buffer: &Buffer) -> Vec<String> {
        let area = rows_area(buffer);
        (0..area.height)
            .map(|index| text_in(buffer, area, area.y + index))
            .collect()
    }

    /// Tree row `index` of the window, as text.
    fn tree_row(buffer: &Buffer, index: u16) -> String {
        let area = rows_area(buffer);

        text_in(buffer, area, area.y + index)
    }

    /// The tree under `crates`, holding `sigils`: the header line the tests
    /// about it draw, with both halves set.
    ///
    /// The holding is handed over as a value, as [`Chrome`](crate::app::Chrome)
    /// takes it — nothing here reads a config, and no test of this module goes
    /// anywhere near a home directory.
    fn held_chrome(sigils: Sigils) -> Chrome {
        Chrome::of("/repo", "/repo/crates").with_sigils(sigils)
    }

    /// The app the header tests draw under that line: the fixture, unchanged.
    ///
    /// A header says which tree is on screen and nothing about the rows, so
    /// every one of those tests draws the same app and varies only the
    /// [`Chrome`](crate::app::Chrome) beside it — which is the arrangement the
    /// type exists to make possible.
    fn header_app() -> App {
        App::from_tree(&fixture::tree())
    }

    /// The tree pane's header line, as text.
    fn header_text(buffer: &Buffer) -> String {
        let area = header_area(buffer);

        text_in(buffer, area, area.y)
    }

    /// The header line of `buffer`, and its footer block: the two that must not
    /// move when the tree scrolls.
    fn header_and_footer(buffer: &Buffer) -> (Vec<String>, Vec<String>) {
        let height = buffer.area.height;
        (
            vec![header_text(buffer)],
            (height - FOOTER_HEIGHT..height)
                .map(|y| row_text(buffer, y))
                .collect(),
        )
    }

    /// The line row `index` of [`many_rows`] is drawn as when `selected` is the
    /// selected row: the selection marker's gutter, the one level of depth's
    /// guide in the columns an indent takes, the blank a childless row carries
    /// where a collapse marker would go, then the name.
    ///
    /// [`many_rows`] is a flat run of siblings, so every one of them is a branch
    /// except the last, which is the corner. `count` is what says which that is.
    fn drawn_row(index: usize, selected: usize, count: usize) -> String {
        let gutter = if index == selected {
            SELECTION_MARKER.to_owned()
        } else {
            " ".repeat(SELECTION_MARKER.chars().count())
        };
        let corner = if index + 1 == count {
            GUIDE_LAST
        } else {
            GUIDE_BRANCH
        };
        format!("{gutter}{corner} {NO_MARKER}module{index}")
    }

    /// Draw `app` onto an in-memory terminal of the given size and hand back
    /// the buffer. No tty is involved, so this runs anywhere `cargo test` does.
    ///
    /// Drawn at this moment, which every test that is not about the panel's
    /// clocks can ignore: an app with no account draws the same frame whatever
    /// instant it is handed. The ones that do care use [`render_at`].
    fn render(app: &App, width: u16, height: u16) -> Buffer {
        render_at(app, width, height, Instant::now())
    }

    /// [`render`], at an instant the test chose, so a clock on screen can be
    /// asserted for equality rather than for looking about right.
    ///
    /// With the gate on the way out closed, which is every test that is not
    /// about the gate: the frame this draws is the frame warlock has always
    /// drawn.
    fn render_at(app: &App, width: u16, height: u16, now: Instant) -> Buffer {
        render_confirm(app, width, height, now, QuitConfirm::Closed)
    }

    /// [`render_at`], with the quit confirmation in whatever state the test is
    /// about, and the scope prompt closed — which is every test but the ones
    /// about the scope prompt itself.
    fn render_confirm(
        app: &App,
        width: u16,
        height: u16,
        now: Instant,
        confirm: QuitConfirm,
    ) -> Buffer {
        render_windows(
            app,
            &Chrome::default(),
            width,
            height,
            now,
            confirm,
            &ScopePrompt::Closed,
        )
    }

    /// [`render_at`], with the scope prompt in whatever state the test is about
    /// and the gate on the way out closed.
    fn render_scope(
        app: &App,
        width: u16,
        height: u16,
        now: Instant,
        scope: &ScopePrompt,
    ) -> Buffer {
        render_windows(
            app,
            &Chrome::default(),
            width,
            height,
            now,
            QuitConfirm::Closed,
            scope,
        )
    }

    /// [`render_at`], with the path prompt in whatever state the test is about
    /// and every other window closed: the frame a `/write` turn's answer leaves
    /// on screen.
    fn render_path(app: &App, width: u16, height: u16, now: Instant, path: &ScopePrompt) -> Buffer {
        render_all(
            app,
            &Chrome::default(),
            width,
            height,
            now,
            QuitConfirm::Closed,
            &ScopePrompt::Closed,
            path,
            None,
        )
    }

    /// [`render`], with a header line the test chose.
    ///
    /// The header is no longer app state, so the tests about it hand one in
    /// rather than building an app that carries one: see
    /// [`Chrome`](crate::app::Chrome). Every other test here draws through
    /// [`render`], which passes a default — an empty identity holding nothing,
    /// which is the line warlock drew before either half existed.
    fn render_chrome(app: &App, chrome: &Chrome, width: u16, height: u16) -> Buffer {
        render_windows(
            app,
            chrome,
            width,
            height,
            Instant::now(),
            QuitConfirm::Closed,
            &ScopePrompt::Closed,
        )
    }

    /// [`render`], with a composer under the panel: the frame the binary draws
    /// once there is a draft on screen.
    ///
    /// Every other helper here passes `None`, which is a frame with no field on
    /// it — the shape of every test written before there was a composer, and
    /// still the shape of a frame while the document card has the panel.
    fn render_composer(app: &App, composer: &Composer, width: u16, height: u16) -> Buffer {
        render_all(
            app,
            &Chrome::default(),
            width,
            height,
            Instant::now(),
            QuitConfirm::Closed,
            &ScopePrompt::Closed,
            &ScopePrompt::Closed,
            Some(composer),
        )
    }

    /// The one place a frame is actually drawn: the app, the instant, and the
    /// windows that can be over it.
    ///
    /// The path prompt is closed here, which is every test but the ones about
    /// the path prompt itself — they go through [`render_path`].
    fn render_windows(
        app: &App,
        chrome: &Chrome,
        width: u16,
        height: u16,
        now: Instant,
        confirm: QuitConfirm,
        scope: &ScopePrompt,
    ) -> Buffer {
        render_all(
            app,
            chrome,
            width,
            height,
            now,
            confirm,
            scope,
            &ScopePrompt::Closed,
            None,
        )
    }

    /// [`render_windows`] with the composer as well: everything one frame can
    /// have on it, and the only place a test attaches a terminal.
    #[expect(
        clippy::too_many_arguments,
        reason = "one frame's worth of state, and the point of it is that no \
                  test builds a frame any other way"
    )]
    fn render_all(
        app: &App,
        chrome: &Chrome,
        width: u16,
        height: u16,
        now: Instant,
        confirm: QuitConfirm,
        scope: &ScopePrompt,
        path: &ScopePrompt,
        composer: Option<&Composer>,
    ) -> Buffer {
        let mut terminal =
            Terminal::new(TestBackend::new(width, height)).expect("test backend never fails");
        terminal
            .draw(|frame| draw(frame, app, chrome, now, confirm, scope, path, composer))
            .expect("test backend never fails");
        terminal.backend().buffer().clone()
    }

    /// The text of one row of a buffer, full width, trailing blanks trimmed off.
    fn row_text(buffer: &Buffer, y: u16) -> String {
        text_in(buffer, buffer.area, y)
    }

    /// The text of row `y` of `buffer`, clipped to `area`'s columns, trailing
    /// blanks trimmed off: how a row of one pane is read without the other
    /// pane's border on the end of it.
    fn text_in(buffer: &Buffer, area: Rect, y: u16) -> String {
        let text: String = (area.x..area.x + area.width)
            .map(|x| buffer[(x, y)].symbol())
            .collect();
        text.trim_end().to_string()
    }

    /// Line `line` of the footer, read from the columns the footer's text is
    /// drawn in.
    ///
    /// The footer draws no border and its text is inset a column from each edge
    /// so that it begins where the panes' own rows do (see
    /// [`footer_text_area`]). A test reading the whole row would read that
    /// column too and compare every line against a leading blank.
    fn footer_line(buffer: &Buffer, line: u16) -> String {
        let band = Rect {
            x: buffer.area.x,
            y: buffer.area.height - FOOTER_HEIGHT,
            width: buffer.area.width,
            height: FOOTER_HEIGHT,
        };

        text_in(buffer, footer_text_area(band), band.y + line)
    }

    /// How many columns the footer's text has on a terminal `width` wide: the
    /// width the keys line is laid out for, which is what [`footer_text_area`]
    /// leaves after insetting a column at each edge.
    fn footer_width(width: u16) -> usize {
        usize::from(footer_text_area(Rect::new(0, 0, width, FOOTER_HEIGHT)).width)
    }

    /// Every row of a buffer, full width, as text.
    fn rows_text(buffer: &Buffer) -> Vec<String> {
        (0..buffer.area.height)
            .map(|y| row_text(buffer, y))
            .collect()
    }

    /// Every glyph a guide is drawn with: the verticals and the two corners.
    ///
    /// Named once here so that the helpers below ask "is this a guide?" rather
    /// than listing the three constants each time and drifting apart when a
    /// fourth arrives.
    const GUIDE_GLYPHS: [&str; 3] = [GUIDE, GUIDE_BRANCH, GUIDE_LAST];

    /// The foreground colour of tree row `index`'s first glyph of *text*:
    /// its marker, or the first letter of its name when it carries none.
    ///
    /// Two things to the left of that are skipped, and neither is the row's
    /// text. The gutter the selection marker lives in is drawn by the list
    /// itself and takes no state colour; the indent guides are drawn in
    /// [`GUIDE_COLOUR`] and say depth, not state, which is what
    /// [`guide_columns`] is for.
    fn first_glyph_colour(buffer: &Buffer, index: u16) -> Color {
        let area = rows_area(buffer);
        let gutter = u16::try_from(SELECTION_MARKER.chars().count()).expect("a two-char marker");
        (area.x + gutter..area.x + area.width)
            .map(|x| &buffer[(x, area.y + index)])
            .find(|cell| !cell.symbol().trim().is_empty() && !GUIDE_GLYPHS.contains(&cell.symbol()))
            .expect("row has a glyph on it")
            .fg
    }

    /// Which column of `line` `needle` starts in, counted in characters rather
    /// than bytes.
    ///
    /// [`GUIDE`] is three bytes and one column, so a byte offset compared across
    /// two rows of different depths says nothing about what lines up with what
    /// on screen.
    fn column_of(line: &str, needle: &str) -> usize {
        let byte = line
            .find(needle)
            .unwrap_or_else(|| panic!("{needle:?} is not on {line:?}"));

        line[..byte].chars().count()
    }

    /// The columns of tree row `index` that carry a guide stroke, counted from
    /// the left-hand edge of the tree's rows, the gutter included.
    ///
    /// Read off the buffer a column at a time rather than out of the row's text,
    /// because [`GUIDE`] is three bytes and a byte offset into a string is not a
    /// column on screen.
    fn guide_columns(buffer: &Buffer, index: u16) -> Vec<u16> {
        let area = rows_area(buffer);
        (0..area.width)
            .filter(|column| {
                GUIDE_GLYPHS.contains(&buffer[(area.x + column, area.y + index)].symbol())
            })
            .collect()
    }

    /// Which drawn row of `app` stands for `path`, as an index into the window
    /// [`tree_rows`] and [`first_glyph_colour`] read.
    ///
    /// The pulse tests name the directory or file they mean rather than
    /// counting the fixture's rows, so that they keep asserting about the same
    /// node if the fixture ever grows another one above it. Only sound while
    /// the whole tree is on screen, which the tests that use it check.
    fn row_index(app: &App, path: &str) -> u16 {
        let index = app
            .rows()
            .iter()
            .position(|row| row.path.as_path() == Path::new(path))
            .unwrap_or_else(|| panic!("{path} is not a row of this app"));

        u16::try_from(index).expect("the fixture tree is small")
    }

    /// `app` with the row for `path` selected, reached by stepping down to it
    /// the way the movement keys do.
    ///
    /// Named rather than counted, for the same reason [`row_index`] is: the
    /// default view draws a document row under each documented directory, so
    /// which press of `down` lands on which directory is a fact about the
    /// fixture that no test of this module is about.
    fn select(mut app: App, path: &str) -> App {
        while app.selected_row().expect("the app has rows").path != Path::new(path) {
            let before = app.selected();
            app.select_next();
            assert_ne!(app.selected(), before, "no row for {path}");
        }
        app
    }

    /// The rows of `buffer` — counted from the top of the screen — drawn with
    /// the selection's highlight.
    ///
    /// Read down the tree's own first column rather than the screen's, which is
    /// the panel's border now; still the whole height of the screen, so a header
    /// or footer row that lit up would show up here as well.
    fn highlighted_rows(buffer: &Buffer) -> Vec<u16> {
        let x = rows_area(buffer).x;
        (0..buffer.area.height)
            .filter(|&y| buffer[(x, y)].modifier.contains(Modifier::REVERSED))
            .collect()
    }

    /// How wide [`MARK`] is drawn: the widest of its rows, in columns.
    fn mark_width() -> usize {
        MARK.iter()
            .copied()
            .map(display_width)
            .max()
            .expect("the art has rows")
    }

    /// Assert that the panel of `buffer` is the whole of [`MARK`] and nothing
    /// else: the art centred inside the border, dim, in no colour of its own,
    /// on otherwise blank rows.
    ///
    /// Where the art lands is worked out here from [`MARK`] and the panel's
    /// inner area rather than asked of [`mark_area`], so that a mark which
    /// moved or was redrawn fails here rather than agreeing with itself. The
    /// art is compared against the constant for the same reason: a copy of it
    /// written out in a test would go on passing after the mark changed.
    fn assert_mark_drawn(buffer: &Buffer) {
        let inner = panel_area(buffer);
        let left = (usize::from(inner.width) - mark_width()) / 2;
        let top = (usize::from(inner.height) - MARK.len()) / 2;

        // One row of the art per row of the panel, indented to the middle, and
        // blank rows above and below it.
        let rows = panel_rows(buffer);
        let expected: Vec<String> = (0..rows.len())
            .map(
                |index| match index.checked_sub(top).and_then(|row| MARK.get(row)) {
                    Some(line) => format!("{}{line}", " ".repeat(left)),
                    None => String::new(),
                },
            )
            .collect();
        assert_eq!(rows, expected);

        // Sitting a hair high: the row an uneven split leaves over falls under
        // the mark rather than above it.
        let below = rows.len() - top - MARK.len();
        assert!(
            below >= top,
            "the mark has {top} rows above it and {below} below"
        );

        // And every cell of it is dim and uncoloured. Colour in this crate says
        // what state a node is in; the mark is not a node.
        for (row, line) in MARK.iter().enumerate() {
            let y = inner.y + u16::try_from(top + row).expect("a panel shorter than u16::MAX");
            for (column, glyph) in line.chars().enumerate() {
                let x =
                    inner.x + u16::try_from(left + column).expect("a panel narrower than u16::MAX");
                let cell = &buffer[(x, y)];
                assert_eq!(cell.symbol(), glyph.to_string(), "at ({x}, {y})");
                assert!(
                    cell.modifier.contains(Modifier::DIM),
                    "({x}, {y}) is not dim"
                );
                assert_eq!(cell.fg, Color::Reset, "({x}, {y}) carries a colour");
            }
        }
    }

    /// Assert that the panel of `buffer` is the bare border it drew before
    /// there was a mark: every cell inside it a space, the border whole on all
    /// four sides but for the card's own title, and nothing on its bottom edge.
    ///
    /// The title is the one thing the top edge is allowed to carry. The panel
    /// opens on the conversation, which is the card that says its name on the
    /// border (see [`THREAD_TITLE`]), and the blanks padding that name off the
    /// corner are cells of the edge that are not border — so the top edge is
    /// only held to being unbroken on the cards that name nothing.
    fn assert_bare_panel(buffer: &Buffer) {
        let panel = areas(buffer.area, None).panel;
        let inner = pane_inner(panel);
        assert!(inner.width > 0 && inner.height > 0, "{inner:?}");

        for y in inner.y..inner.y + inner.height {
            for x in inner.x..inner.x + inner.width {
                assert_eq!(
                    buffer[(x, y)].symbol(),
                    " ",
                    "the panel drew something at ({x}, {y})"
                );
            }
        }

        // The border is still there, on all four sides, carrying nothing of its
        // own: no title, and no scrollback indicator on a panel with nothing to
        // scroll.
        let top = panel_top_edge(buffer);
        let titled = top.contains(THREAD_TITLE.trim());
        for x in panel.x..panel.x + panel.width {
            // The title, and the blanks padding it, are what the top edge is
            // allowed to carry; a panel too narrow for the name carries none of
            // it and the edge is unbroken.
            if !titled {
                assert_ne!(buffer[(x, panel.y)].symbol(), " ", "the top edge at {x}");
            }
            assert_ne!(
                buffer[(x, panel.y + panel.height - 1)].symbol(),
                " ",
                "the bottom edge at {x}"
            );
        }
        for y in panel.y..panel.y + panel.height {
            assert_ne!(buffer[(panel.x, y)].symbol(), " ", "the left edge at {y}");
            assert_ne!(
                buffer[(panel.x + panel.width - 1, y)].symbol(),
                " ",
                "the right edge at {y}"
            );
        }
        let edge = panel_bottom_edge(buffer);
        assert!(!edge.contains(SCROLLBACK_ARROW), "{edge:?}");
        assert!(!edge.contains("more"), "{edge:?}");
    }

    /// Assert that no row of `buffer`'s panel carries any part of [`MARK`],
    /// however much else is drawn there.
    fn assert_no_mark(buffer: &Buffer) {
        for row in panel_rows(buffer) {
            for line in MARK {
                assert!(
                    !row.contains(line.trim()),
                    "the panel drew {row:?}, which carries the mark"
                );
            }
        }
    }

    #[test]
    fn every_node_gets_its_own_line_indented_by_depth_in_walk_order() {
        let app = App::from_tree(&fixture::tree());

        let buffer = render(&app, WIDTH, FIXTURE_HEIGHT);

        let drawn: Vec<String> = tree_rows(&buffer)
            .into_iter()
            .take(app.rows().len())
            .collect();
        // Every directory of the walk, in walk order, with the document row
        // each documented one draws under it: see [`WHOLE_FIXTURE`].
        assert_eq!(drawn, WHOLE_FIXTURE);
    }

    #[test]
    fn a_documented_directorys_document_is_drawn_under_it_like_any_other_file_row() {
        let tree = fixture::tree();
        let app = App::from_tree(&tree);

        let buffer = render(&app, WIDTH, FIXTURE_HEIGHT);

        // The whole default view, line by line: four `WARLOCK.md` rows drawn
        // by the same rule as any file row, and nothing under `crates/`, which
        // has no document to draw.
        let drawn: Vec<String> = tree_rows(&buffer)
            .into_iter()
            .take(app.rows().len())
            .collect();
        assert_eq!(drawn, WHOLE_FIXTURE);

        for directory in [
            "warlock",
            "warlock/crates/engine",
            "warlock/crates/tui",
            "warlock/assets",
        ] {
            let above = row_index(&app, directory);
            let index = row_index(&app, &format!("{directory}/WARLOCK.md"));
            // Directly beneath its directory, one indent deeper, with the
            // name in the column a childless sibling's would start in: the
            // marker's two blanks and no marker, because nothing is under a
            // file to hide.
            assert_eq!(index, above + 1, "{directory}");
            let line = tree_row(&buffer, index);
            let level = u16::try_from(INDENT.chars().count()).expect("a two-column indent");
            assert_eq!(
                column_of(&line, "WARLOCK.md"),
                column_of(&tree_row(&buffer, above), directory_name(directory))
                    + usize::from(level),
                "{directory}"
            );
            assert!(
                !line.contains('+') && !line.contains('-'),
                "the document row carries a collapse marker: {line:?}"
            );
            // Its directory's colour and no shade of its own, and the name is
            // the whole of what it says: no marker, no label, nothing to tell
            // it from the row `f` draws for the same file.
            let node = tree.find(directory).expect("the fixture has it");
            assert_eq!(
                first_glyph_colour(&buffer, index),
                colour_for(node.state),
                "{directory}"
            );
            assert_eq!(
                styles_of(&buffer, index, "WARLOCK.md"),
                vec![(colour_for(node.state), Modifier::empty()); "WARLOCK.md".len()],
                "{directory}"
            );
            assert!(line.trim_end().ends_with("WARLOCK.md"), "{line:?}");
        }
    }

    /// The last component of `path`: what a directory's row draws as.
    fn directory_name(path: &str) -> &str {
        path.rsplit('/')
            .next()
            .expect("a path has a last component")
    }

    #[test]
    fn a_nested_row_carries_one_guide_per_level_it_sits_under_and_a_root_row_none() {
        let app = App::from_tree(&fixture::tree());

        let buffer = render(&app, WIDTH, FIXTURE_HEIGHT);

        // The root is at depth 0, so there is nothing above it to lead it in.
        assert!(
            guide_columns(&buffer, 0).is_empty(),
            "a depth-0 row should draw no guide: {:?}",
            tree_row(&buffer, 0)
        );
        // `crates`, one level down: one guide — its own corner — in the first
        // column past the gutter the selection marker lives in, and the rest of
        // the level blank.
        let gutter = u16::try_from(SELECTION_MARKER.chars().count()).expect("a two-char marker");
        let level = u16::try_from(INDENT.chars().count()).expect("a two-column indent");
        let crates = row_index(&app, "warlock/crates");
        assert_eq!(guide_columns(&buffer, crates), [gutter]);
        // `engine`, two levels down: two guides, one level apart — the vertical
        // carrying `crates` on past it and then its own corner — and its name
        // still starts one marker past the last of them.
        let engine = row_index(&app, "warlock/crates/engine");
        assert_eq!(guide_columns(&buffer, engine), [gutter, gutter + level]);
        assert_eq!(
            column_of(&tree_row(&buffer, engine), "engine"),
            usize::from(gutter + 2 * level) + NO_MARKER.chars().count()
        );
    }

    #[test]
    fn the_guides_of_a_scrolled_window_are_the_ones_the_rows_above_it_earned() {
        // Two directories, each holding two files, and a window onto the middle
        // of it. The first row of the window is `a/two`, whose vertical comes
        // from `a` — a row above the window — and whose corner says it is the
        // last of `a`'s. Neither fact is on the row itself.
        let rows = vec![
            Row::new(0, "repo", None, NodeState::Unpacted).with_child_count(2),
            Row::new(1, "repo/a", None, NodeState::Unpacted),
            Row::file(2, "repo/a/one", NodeState::Unpacted),
            Row::file(2, "repo/a/two", NodeState::Unpacted),
            Row::new(1, "repo/b", None, NodeState::Unpacted),
            Row::file(2, "repo/b/one", NodeState::Unpacted),
            Row::file(2, "repo/b/two", NodeState::Unpacted),
        ];

        let whole = guide_prefixes(&rows, 0, rows.len());
        assert_eq!(
            whole,
            [
                String::new(),
                format!("{GUIDE_BRANCH} "),
                format!("{GUIDE} {GUIDE_BRANCH} "),
                format!("{GUIDE} {GUIDE_LAST} "),
                format!("{GUIDE_LAST} "),
                format!("  {GUIDE_BRANCH} "),
                format!("  {GUIDE_LAST} "),
            ]
        );

        // The window's rows are exactly what the whole list gave them, so a
        // scrolled tree draws the same guides an unscrolled one does.
        assert_eq!(guide_prefixes(&rows, 3, 2), whole[3..5]);
        // And a window past the end asks for nothing rather than panicking.
        assert!(guide_prefixes(&rows, rows.len(), 4).is_empty());
        assert_eq!(guide_prefixes(&rows, 5, 99), whole[5..]);
    }

    #[test]
    fn the_guides_are_drawn_in_the_guide_colour_and_the_row_keeps_its_states() {
        let app = App::from_tree(&fixture::tree());

        let buffer = render(&app, WIDTH, FIXTURE_HEIGHT);

        let area = rows_area(&buffer);
        let index = row_index(&app, "warlock/crates/engine");
        let columns = guide_columns(&buffer, index);
        assert_eq!(columns.len(), 2, "the fixture's engine sits at depth 2");
        for column in columns {
            let cell = &buffer[(area.x + column, area.y + index)];
            assert_eq!(cell.fg, GUIDE_COLOUR, "column {column}");
            // No `DIM` and no modifier of any other kind: the dimness is in the
            // colour, because `DIM` is honoured inconsistently.
            assert_eq!(cell.modifier, Modifier::empty(), "column {column}");
        }
        // And the row's own text is still its state's colour, guides or no.
        let state = app.rows()[usize::from(index)].state;
        assert_eq!(first_glyph_colour(&buffer, index), colour_for(state));
    }

    #[test]
    fn the_pulse_takes_the_row_in_flight_and_leaves_its_guides_alone() {
        let base = Instant::now();
        let mut app = select(pacting_app(base, WIDTH, FIXTURE_HEIGHT), "warlock/crates");
        assert!(app.toggle_pact().is_some(), "the crates row takes a pact");
        app.account_mut()
            .expect("a pact has started")
            .open_section("crates/engine", base);
        app.set_pact_in_flight("warlock/crates/engine", 1, 2);

        let stale = render_at(&app, WIDTH, FIXTURE_HEIGHT, base);
        let fresh = render_at(&app, WIDTH, FIXTURE_HEIGHT, base + PHASE);

        // The marker and name change colour from one frame to the next...
        let index = row_index(&app, "warlock/crates/engine");
        assert_ne!(
            first_glyph_colour(&stale, index),
            first_glyph_colour(&fresh, index)
        );
        // ...and the guides in front of them sit still: depth has nothing to
        // report about a run.
        for buffer in [&stale, &fresh] {
            let area = rows_area(buffer);
            let columns = guide_columns(buffer, index);
            assert_eq!(columns.len(), 2, "the fixture's engine sits at depth 2");
            for column in columns {
                assert_eq!(buffer[(area.x + column, area.y + index)].fg, GUIDE_COLOUR);
            }
        }
    }

    #[test]
    fn the_selections_highlight_covers_the_guides_in_front_of_the_selected_row() {
        // Down onto `crates`, which is one level in and so has a guide in front
        // of it for the highlight to reach over.
        let app = select(App::from_tree(&fixture::tree()), "warlock/crates");

        let buffer = render(&app, WIDTH, FIXTURE_HEIGHT);

        let area = rows_area(&buffer);
        let index = row_index(&app, "warlock/crates");
        let columns = guide_columns(&buffer, index);
        assert_eq!(columns.len(), 1, "the fixture's crates sits at depth 1");
        for column in columns {
            let cell = &buffer[(area.x + column, area.y + index)];
            assert!(
                cell.modifier.contains(Modifier::REVERSED),
                "the highlight stops short of the guides: {cell:?}"
            );
            // Still the guide's own colour underneath the highlight.
            assert_eq!(cell.fg, GUIDE_COLOUR);
        }
    }

    #[test]
    fn a_collapsed_directory_is_marked_differently_from_an_expanded_one() {
        let expanded = App::from_tree(&fixture::tree());
        let collapsed = App::from_tree(&fixture::tree()).with_collapsed(["warlock/crates"]);

        let before = render(&expanded, WIDTH, FIXTURE_HEIGHT);
        let after = render(&collapsed, WIDTH, FIXTURE_HEIGHT);

        // Same directory, same indent, same name, and the one thing that
        // differs is the marker saying whether anything is under it.
        let index = row_index(&expanded, "warlock/crates");
        assert_eq!(tree_row(&before, index), "  ├ - crates");
        assert_eq!(
            tree_row(&after, row_index(&collapsed, "warlock/crates")),
            "  ├ + crates"
        );
        // And what it was hiding is gone from the screen, leaving the root's
        // other child where the children were — the root's own document row
        // above it and `assets`' below it, neither of which `crates` hid.
        let drawn: Vec<String> = tree_rows(&after)
            .into_iter()
            .take(collapsed.rows().len())
            .collect();
        assert_eq!(
            drawn,
            [
                "> - warlock",
                "  ├   WARLOCK.md",
                "  ├ + crates",
                "  └ - assets",
                "    └   WARLOCK.md",
            ]
        );
    }

    #[test]
    fn a_directory_with_nothing_under_it_carries_neither_marker() {
        // Written out here rather than taken from the fixture: every leaf there
        // has a document, and the default view draws that document under it, so
        // no fixture directory is a row with nothing under it any more.
        let rows = || {
            vec![
                Row::new(0, "repo", "repo/WARLOCK.md", NodeState::PactedStale).with_child_count(2),
                Row::new(1, "repo/crates", None, NodeState::Unpacted).with_child_count(1),
                Row::new(
                    2,
                    "repo/crates/engine",
                    "repo/crates/engine/WARLOCK.md",
                    NodeState::PactedFresh,
                ),
                Row::new(1, "repo/assets", None, NodeState::Unpacted),
            ]
        };
        let app = App::from_rows(rows());

        let buffer = render(&app, WIDTH, FIXTURE_HEIGHT);

        // `assets` has nothing under it — no children and no document row — so
        // it is neither collapsed nor expanded and says so by carrying no
        // marker, while still lining its name up with the marked rows at its
        // own depth.
        let leaf = tree_row(&buffer, 3);
        assert_eq!(leaf, "  └   assets");
        assert!(!leaf.contains('+') && !leaf.contains('-'), "{leaf:?}");
        assert_eq!(
            column_of(&leaf, "assets"),
            column_of(&tree_row(&buffer, 1), "crates"),
            "a leaf's name should start where a sibling directory's does"
        );
        // Pressing space on it changes nothing on screen: nothing to hide.
        let pressed = App::from_rows(rows()).with_collapsed(["repo/assets"]);
        assert_eq!(
            rows_text(&render(&pressed, WIDTH, FIXTURE_HEIGHT)),
            rows_text(&buffer)
        );
    }

    #[test]
    fn each_line_is_drawn_in_its_states_colour() {
        let app = App::from_tree(&fixture::tree());

        let buffer = render(&app, WIDTH, FIXTURE_HEIGHT);

        // Including the selected row: the highlight only adds modifiers, so
        // the row's text keeps its state's colour underneath.
        for (index, row) in app.rows().iter().enumerate() {
            let index = u16::try_from(index).expect("the fixture tree is small");
            assert_eq!(
                first_glyph_colour(&buffer, index),
                colour_for(row.state),
                "row {index} ({}) is drawn in the wrong colour",
                row.path.display()
            );
        }
    }

    #[test]
    fn a_file_row_is_drawn_in_the_colour_of_the_directory_holding_it() {
        let tree = fixture::tree();
        let mut app = App::from_tree(&tree);
        app.toggle_files();

        let buffer = render(&app, WIDTH, FILES_HEIGHT);

        let mut colours = Vec::new();
        for (index, row) in app.rows().iter().enumerate() {
            if !row.is_file() {
                continue;
            }
            let index = u16::try_from(index).expect("the fixture tree is small");
            // Asked of the engine's tree rather than of the row, so what is
            // under test is that the colour on screen is the *directory's*
            // state and not merely whatever the row happened to be built with.
            let directory = row.path.parent().expect("a file sits in a directory");
            let node = tree.find(directory).expect("the file came from that node");
            let colour = first_glyph_colour(&buffer, index);
            assert_eq!(
                colour,
                colour_for(node.state),
                "file row {index} ({}) is not its module's colour",
                row.path.display()
            );
            colours.push(colour);
        }
        // And the fixture really did exercise more than one colour, so a row
        // drawn in some single colour for every file would fail above.
        assert!(colours.len() > 1, "the fixture should list some files");
        assert!(
            colours.iter().any(|colour| *colour != colours[0]),
            "every file was drawn in the same colour: {colours:?}"
        );
    }

    #[test]
    fn a_scoped_directory_reads_its_name_and_then_its_scope_in_parentheses() {
        let app = App::from_rows(labelled_rows(Some(SCOPE_TEAM), NodeState::PactedStale));

        let buffer = render(&app, SCOPE_ROOM_WIDTH, HEIGHT);

        // The row is the row it always was — guide, marker, name — with the
        // scope stated after the name and nothing else changed about it.
        assert_eq!(tree_row(&buffer, 1), format!("  └ - {SCOPED_LABEL}"));
    }

    #[test]
    fn the_whole_of_a_label_is_drawn_in_the_rows_own_state_colour() {
        for state in [NodeState::PactedStale, NodeState::PactedFresh] {
            let app = App::from_rows(labelled_rows(Some(SCOPE_TEAM), state));

            let buffer = render(&app, SCOPE_ROOM_WIDTH, HEIGHT);

            // Name, space and both parentheses: one colour, the row's own, and
            // no fourth colour or modifier smuggled in with the label.
            let styles = styles_of(&buffer, 1, SCOPED_LABEL);
            assert_eq!(styles.len(), display_width(SCOPED_LABEL));
            for (offset, style) in styles.iter().enumerate() {
                assert_eq!(
                    *style,
                    (colour_for(state), Modifier::empty()),
                    "column {offset} of {state:?}"
                );
            }
            // And the name the label was appended to is drawn in exactly what
            // the label is, so neither of them is the odd one out.
            assert_eq!(
                styles_of(&buffer, 1, SCOPED_NAME),
                styles[..SCOPED_NAME.len()]
            );
        }
    }

    #[test]
    fn only_the_directory_that_owns_a_scope_carries_the_label() {
        let mut app = App::from_rows(labelled_rows(Some(SCOPE_TEAM), NodeState::PactedStale));
        app.toggle_files();

        let buffer = render(&app, SCOPE_ROOM_WIDTH, HEIGHT);

        // The scoped directory says so...
        assert!(tree_row(&buffer, 1).ends_with(SCOPED_LABEL));
        // ...and the pacted directory under it, covered by that scope but
        // carrying none of its own, says nothing — nor does the file inside the
        // scoped directory, which is not a boundary either.
        for index in [0, 2, 3] {
            let row = tree_row(&buffer, index);
            assert!(!row.contains('('), "row {index} carries a label: {row:?}");
        }
        assert!(tree_row(&buffer, 2).ends_with("widgets"));
        assert!(tree_row(&buffer, 3).ends_with("ui.rs"));
    }

    #[test]
    fn a_row_with_no_scope_of_its_own_is_drawn_exactly_as_it_was() {
        let scoped = App::from_rows(labelled_rows(Some(SCOPE_TEAM), NodeState::PactedStale));
        let plain = App::from_rows(labelled_rows(None, NodeState::PactedStale));

        let with = render(&scoped, SCOPE_ROOM_WIDTH, HEIGHT);
        let without = render(&plain, SCOPE_ROOM_WIDTH, HEIGHT);

        // The unscoped row is the name and nothing after it...
        assert_eq!(tree_row(&without, 1), format!("  └ - {SCOPED_NAME}"));
        // ...and a scope on one row is the whole of the difference between the
        // two frames: every other row, and the chrome around them, is untouched.
        for (index, (labelled, bare)) in tree_rows(&with)
            .into_iter()
            .zip(tree_rows(&without))
            .enumerate()
        {
            if index == 1 {
                continue;
            }
            assert_eq!(labelled, bare, "row {index}");
        }
        assert_eq!(header_and_footer(&with), header_and_footer(&without));
    }

    #[test]
    fn a_label_is_there_whether_or_not_its_row_is_selected() {
        let mut app = App::from_rows(labelled_rows(Some(SCOPE_TEAM), NodeState::PactedStale));
        let needed = display_width(&tree_row(&render(&app, SCOPE_ROOM_WIDTH, HEIGHT), 1));
        let needed = u16::try_from(needed).expect("a narrow terminal");
        let wide = terminal_width_for(needed);
        let narrow = terminal_width_for(needed - 1);

        // The label-bearing row is row 1, so one press of down selects it.
        let unselected = [render(&app, wide, HEIGHT), render(&app, narrow, HEIGHT)];
        app.select_next();
        assert_eq!(app.selected(), 1);
        let selected = [render(&app, wide, HEIGHT), render(&app, narrow, HEIGHT)];

        // The two widths straddle the boundary, so this is not two frames that
        // both happen to have room: the label is on at one of them and off at
        // the other, and what is under test is that the selection moves neither.
        assert!(tree_row(&unselected[0], 1).contains(SCOPE_TEAM));
        assert!(!tree_row(&unselected[1], 1).contains(SCOPE_TEAM));

        // The selection reserves its gutter on every row whether it is on the
        // row or not, so it never buys a row two columns and never spends them:
        // present at both widths in both frames, or absent in both.
        for (index, (moved, still)) in selected.iter().zip(&unselected).enumerate() {
            assert_eq!(
                tree_row(moved, 1).contains(SCOPE_TEAM),
                tree_row(still, 1).contains(SCOPE_TEAM),
                "width {index} draws the label differently when the row is selected"
            );
        }
    }

    #[test]
    fn a_label_is_drawn_where_it_fits_and_dropped_whole_where_it_does_not() {
        let scoped = App::from_rows(labelled_rows(Some(SCOPE_TEAM), NodeState::PactedStale));
        let plain = App::from_rows(labelled_rows(None, NodeState::PactedStale));
        let needed = display_width(&tree_row(&render(&scoped, SCOPE_ROOM_WIDTH, HEIGHT), 1));
        let needed = u16::try_from(needed).expect("a narrow terminal");

        let fits = render(&scoped, terminal_width_for(needed), HEIGHT);
        let short = render(&scoped, terminal_width_for(needed - 1), HEIGHT);

        // A column wider than the labelled row needs and it is drawn whole...
        assert_eq!(tree_row(&fits, 1), format!("  └ - {SCOPED_LABEL}"));
        // ...one column narrower and it is gone entirely, rather than cut to an
        // ellipsised half-scope, leaving the row the unscoped one draws.
        assert!(!tree_row(&short, 1).contains(ELLIPSIS));
        assert_eq!(
            tree_row(&short, 1),
            tree_row(&render(&plain, terminal_width_for(needed - 1), HEIGHT), 1)
        );
        assert_eq!(tree_row(&short, 1), format!("  └ - {SCOPED_NAME}"));
    }

    #[test]
    fn the_row_in_flight_alternates_between_the_stale_and_the_fresh_colour() {
        let base = Instant::now();
        // The keypress first, as the reader makes it: a pact on `crates`
        // covers everything below it and paints the lot stale.
        let mut app = select(pacting_app(base, WIDTH, FILES_HEIGHT), "warlock/crates");
        assert!(app.toggle_pact().is_some(), "the crates row takes a pact");
        app.toggle_files();
        // Then the run reaching its first directory, the way the progress
        // handler does it: a section opened, and that path put in flight.
        app.account_mut()
            .expect("a pact has started")
            .open_section("crates/engine", base);
        app.set_pact_in_flight("warlock/crates/engine", 1, 2);
        assert_eq!(app.scroll_offset(), 0, "the fixture fits on this screen");

        // Three frames of the same app, differing only in what time the caller
        // says it is — no sleeping, no clock of the renderer's own.
        let first = render_at(&app, WIDTH, FILES_HEIGHT, base);
        let half = render_at(&app, WIDTH, FILES_HEIGHT, base + PHASE);
        let whole = render_at(&app, WIDTH, FILES_HEIGHT, base + CYCLE);

        // Half a second apart the row has changed colour; a second apart it is
        // back where it started, and it started on the stale colour the
        // keypress painted — the pulse leaves yellow reaching for green.
        let row = row_index(&app, "warlock/crates/engine");
        assert_eq!(
            first_glyph_colour(&first, row),
            colour_for(NodeState::PactedStale)
        );
        assert_eq!(
            first_glyph_colour(&half, row),
            colour_for(NodeState::PactedFresh)
        );
        assert_ne!(
            first_glyph_colour(&first, row),
            first_glyph_colour(&half, row)
        );
        assert_eq!(
            first_glyph_colour(&whole, row),
            first_glyph_colour(&first, row)
        );

        // The files inside the directory being worked move with it: they are
        // what the pass is reading, so their rows flash in step with their
        // directory's on every frame.
        for path in [
            "warlock/crates/engine/Cargo.toml",
            "warlock/crates/engine/WARLOCK.md",
        ] {
            let index = row_index(&app, path);
            for (when, buffer) in [("0 ms", &first), ("500 ms", &half), ("1 s", &whole)] {
                assert_eq!(
                    first_glyph_colour(buffer, index),
                    first_glyph_colour(buffer, row),
                    "{path} fell out of step with its directory at {when}"
                );
            }
        }
        // And nothing else moved: the sibling directory the run has not
        // reached yet holds the colour the keypress painted on every frame.
        let sibling = row_index(&app, "warlock/crates/tui");
        for (when, buffer) in [("0 ms", &first), ("500 ms", &half), ("1 s", &whole)] {
            assert_eq!(
                first_glyph_colour(buffer, sibling),
                colour_for(NodeState::PactedStale),
                "warlock/crates/tui changed colour at {when}"
            );
        }
        // A colour is the whole of what moves: the same rows, in the same
        // places, saying the same things on all three frames.
        assert_eq!(tree_rows(&half), tree_rows(&first));
        assert_eq!(tree_rows(&whole), tree_rows(&first));
    }

    #[test]
    fn each_directorys_pulse_starts_over_on_the_stale_colour() {
        let base = Instant::now();
        let mut app = select(pacting_app(base, WIDTH, FIXTURE_HEIGHT), "warlock/crates");
        assert!(app.toggle_pact().is_some(), "the crates row takes a pact");
        app.account_mut()
            .expect("a pact has started")
            .open_section("crates/engine", base);
        app.set_pact_in_flight("warlock/crates/engine", 1, 2);
        let engine = row_index(&app, "warlock/crates/engine");
        let tui = row_index(&app, "warlock/crates/tui");

        // A handover half a second in, so the first directory is mid-phase and
        // on the fresh colour at the moment the second one takes over.
        let handover = base + PHASE;
        let before = render_at(&app, WIDTH, FIXTURE_HEIGHT, handover);
        assert_eq!(
            first_glyph_colour(&before, engine),
            colour_for(NodeState::PactedFresh)
        );

        app.account_mut()
            .expect("a pact has started")
            .open_section("crates/tui", handover);
        app.set_pact_in_flight("warlock/crates/tui", 2, 2);
        let after = render_at(&app, WIDTH, FIXTURE_HEIGHT, handover);
        let later = render_at(&app, WIDTH, FIXTURE_HEIGHT, handover + PHASE);

        // The phase is measured from the section that just opened, not from the
        // start of the run: the new row begins on stale however far into the
        // last directory's phase the handover landed, and turns fresh half a
        // second after the handover rather than half a second after the run.
        assert_eq!(
            first_glyph_colour(&after, tui),
            colour_for(NodeState::PactedStale)
        );
        assert_eq!(
            first_glyph_colour(&later, tui),
            colour_for(NodeState::PactedFresh)
        );
        // And the directory the run has left behind stops moving: it is an
        // ordinary stale row again, on both frames.
        for buffer in [&after, &later] {
            assert_eq!(
                first_glyph_colour(buffer, engine),
                colour_for(NodeState::PactedStale)
            );
        }
    }

    #[test]
    fn a_pact_in_flight_with_no_open_section_draws_a_steady_stale_row() {
        let base = Instant::now();
        let mut app = select(pacting_app(base, WIDTH, FIXTURE_HEIGHT), "warlock/crates");
        assert!(app.toggle_pact().is_some(), "the crates row takes a pact");
        // The moment between the keypress and the first progress event: a path
        // in flight and no section to measure a phase against yet.
        app.set_pact_in_flight("warlock/crates/engine", 1, 2);
        assert!(
            app.account()
                .expect("a pact has started")
                .open_section_started()
                .is_none()
        );

        let row = row_index(&app, "warlock/crates/engine");
        for millis in [0, 250, 500, 1_000, 1_500] {
            let now = base + Duration::from_millis(millis);
            let buffer = render_at(&app, WIDTH, FIXTURE_HEIGHT, now);

            // Never blank, never unstyled and never fresh: the row sits in the
            // colour the keypress painted it until its own pass begins.
            assert!(
                tree_row(&buffer, row).ends_with("engine"),
                "the row went blank at {millis} ms"
            );
            assert_eq!(
                first_glyph_colour(&buffer, row),
                colour_for(NodeState::PactedStale),
                "at {millis} ms"
            );
        }
    }

    #[test]
    fn the_row_takes_its_own_states_colour_back_when_the_pact_stops_flying() {
        let base = Instant::now();
        let mut app = pacting_app(base, WIDTH, FIXTURE_HEIGHT);
        // No keypress here on purpose: the fixture's engine is fresh already,
        // so the pulse's stale phase is visibly not this row's own colour and a
        // row that kept pulsing could not pass for one that had stopped.
        app.account_mut()
            .expect("a pact has started")
            .open_section("crates/engine", base);
        app.set_pact_in_flight("warlock/crates/engine", 1, 2);
        let row = row_index(&app, "warlock/crates/engine");
        assert_eq!(
            first_glyph_colour(&render_at(&app, WIDTH, FIXTURE_HEIGHT, base), row),
            colour_for(NodeState::PactedStale),
            "the pulse should be running before it is stopped"
        );

        // Finished, failed or cancelled — whichever it was, the app is told the
        // same way, and the account is left exactly as the run left it.
        app.clear_pact_in_flight();

        // On the next frame and every frame after it, drawn at the instants the
        // pulse would have been stale at: its own state's colour instead.
        for offset in [Duration::ZERO, CYCLE, CYCLE + CYCLE] {
            let buffer = render_at(&app, WIDTH, FIXTURE_HEIGHT, base + offset);
            assert_eq!(
                first_glyph_colour(&buffer, row),
                colour_for(NodeState::PactedFresh),
                "{offset:?} after the run began"
            );
        }
    }

    #[test]
    fn a_file_is_drawn_under_its_directory_one_indent_deeper_and_with_no_marker() {
        let mut app = App::from_tree(&fixture::tree());
        let before = render(&app, WIDTH, FILES_HEIGHT);

        app.toggle_files();
        let buffer = render(&app, WIDTH, FILES_HEIGHT);

        let drawn: Vec<String> = tree_rows(&buffer)
            .into_iter()
            .take(app.rows().len())
            .collect();
        // Every file under the directory that lists it, one indent further in
        // than that directory, carrying neither collapse marker: there is
        // nothing under a file to hide. The directories listing them carry one,
        // including the three with no child directory of their own — with the
        // files drawn there is something under those rows to hide, and the
        // marker says what the collapse key will do rather than what the tree
        // is shaped like.
        assert_eq!(
            drawn,
            [
                "> - warlock",
                "  ├   README.md",
                "  ├   WARLOCK.md",
                "  ├ - crates",
                "  │ ├ - engine",
                "  │ │ ├   Cargo.toml",
                "  │ │ └   WARLOCK.md",
                "  │ └ - tui",
                "  │   └   WARLOCK.md",
                "  └ - assets",
                "    ├   WARLOCK.md",
                "    └   logo.svg",
            ]
        );
        assert!(
            column_of(&drawn[1], "README.md") > column_of(&drawn[0], "warlock"),
            "a file should indent past its directory's name: {drawn:?}"
        );
        // And pressing the key again draws what was on screen before it, to
        // the byte: the toggle takes rows off as cleanly as it puts them on.
        app.toggle_files();
        assert_eq!(
            rows_text(&render(&app, WIDTH, FILES_HEIGHT)),
            rows_text(&before)
        );
    }

    #[test]
    fn exactly_one_line_is_highlighted_and_it_is_the_selected_one() {
        let mut app = App::from_tree(&fixture::tree());
        app.select_next();
        app.select_next();

        let buffer = render(&app, WIDTH, FIXTURE_HEIGHT);

        // The third row of the tree, and nothing else anywhere on the screen.
        assert_eq!(highlighted_rows(&buffer), [rows_area(&buffer).y + 2]);
        assert!(tree_row(&buffer, 2).starts_with(SELECTION_MARKER));
    }

    #[test]
    fn the_selection_marker_moves_with_the_selection() {
        let app = select(App::from_tree(&fixture::tree()), "warlock/crates");

        let buffer = render(&app, WIDTH, FIXTURE_HEIGHT);

        assert!(!tree_row(&buffer, 0).starts_with(SELECTION_MARKER));
        assert_eq!(
            tree_row(&buffer, row_index(&app, "warlock/crates")),
            "> ├ - crates"
        );
    }

    #[test]
    fn a_tree_taller_than_the_screen_draws_one_windowful_with_the_selection_in_it() {
        let window = usize::from(tree_height(Size::new(WIDTH, HEIGHT)));
        assert!(window < MANY, "the tree must not fit on screen");

        // The first row, a row in the middle, and the last one: the three
        // places the window can be, and the two it can be pinned to.
        for selected in [0, MANY / 2, MANY - 1] {
            let app = tall_app(selected);
            let first = app.scroll_offset();

            let buffer = render(&app, WIDTH, HEIGHT);

            let drawn = tree_rows(&buffer);
            let expected: Vec<String> = (first..first + window)
                .map(|index| drawn_row(index, selected, MANY))
                .collect();
            // Exactly a windowful, and exactly the window the app says.
            assert_eq!(drawn.len(), window, "selection {selected}");
            assert_eq!(drawn, expected, "selection {selected}");
            // Which contains the selected row, marked and highlighted, once.
            assert!(
                drawn.contains(&drawn_row(selected, selected, MANY)),
                "row {selected} is off screen: {drawn:?}"
            );
            assert_eq!(
                highlighted_rows(&buffer),
                [rows_area(&buffer).y
                    + u16::try_from(selected - first).expect("the window is a few rows tall")],
                "selection {selected}"
            );
        }
    }

    #[test]
    fn the_header_and_footer_stay_put_while_the_tree_scrolls_under_them() {
        let window = usize::from(tree_height(Size::new(WIDTH, HEIGHT)));
        // The header line these frames are drawn under, built once: it is not
        // app state, so it does not change as the selection walks the tree.
        let chrome = Chrome::of("/repo", "/repo/crates");
        let mut fixed = None;
        let mut offsets = Vec::new();

        // Every row in turn, so the window is at the top, somewhere in the
        // middle, and at the bottom over the course of the walk.
        for selected in 0..MANY {
            let app = tall_app(selected);
            offsets.push(app.scroll_offset());

            let buffer = render_chrome(&app, &chrome, WIDTH, HEIGHT);

            let (header, footer) = header_and_footer(&buffer);
            assert_eq!(header, ["crates"], "selection {selected}");
            // The footer is still the footer, and no tree row has leaked into
            // it: the tally and the keys are on it, wherever it puts them. Not
            // every line of it: the message line is blank while the app has
            // nothing to say, which is the whole of this walk.
            //
            // The way out rather than a movement key, because this terminal is
            // narrower than the whole keys line and the movement names are the
            // first the layout gives up — see `KEY_DROP_ORDER`.
            assert!(
                footer.iter().any(|line| line.contains("unpacted"))
                    && footer.iter().any(|line| line.contains(QUIT_KEY)),
                "footer {footer:?} at selection {selected}"
            );
            assert!(
                footer.iter().all(|line| !line.contains("module")),
                "a tree row leaked into footer {footer:?} at selection {selected}"
            );
            // Byte for byte the same rows, whatever the tree between them is
            // showing.
            let fixed = fixed.get_or_insert_with(|| (header.clone(), footer.clone()));
            assert_eq!(*fixed, (header, footer), "selection {selected}");
        }

        // And the tree really did scroll under them, from one end to the other.
        assert_eq!(offsets.first(), Some(&0));
        assert_eq!(offsets.last(), Some(&(MANY - window)));
    }

    #[test]
    fn the_height_the_app_is_told_is_the_height_the_frame_gives_the_tree() {
        // A terminal with room for a single row of tree, the one the window
        // tests use, and three more on the way up: the measurement has to hold
        // at every height, not just at the one the other tests draw at.
        for height in [CHROME_HEIGHT + 1, CHROME_HEIGHT + 2, HEIGHT, 15, 24] {
            let measured = tree_height(Size::new(WIDTH, height));
            let mut app = App::from_rows(many_rows(MANY));
            app.set_viewport_height(measured);

            let buffer = render(&app, WIDTH, height);

            // The footer, the pane's border top and bottom, and the header,
            // taken off the terminal's height — and what is left is what the
            // frame draws, counted row by row off the buffer.
            assert_eq!(measured, height - CHROME_HEIGHT);
            let drawn = tree_rows(&buffer);
            assert_eq!(drawn.len(), usize::from(measured), "in {height} rows");
            assert!(
                drawn.iter().all(|line| !line.is_empty()),
                "a taller tree left blank rows in {height}: {drawn:?}"
            );
        }
    }

    #[test]
    fn a_terminal_with_no_room_for_a_tree_row_is_measured_at_none_rather_than_underflowing() {
        for height in 0..=CHROME_HEIGHT {
            let measured = tree_height(Size::new(WIDTH, height));

            let buffer = render(&App::from_rows(many_rows(MANY)), WIDTH, height.max(1));

            assert_eq!(measured, 0, "in {height} rows");
            if height > 0 {
                assert!(tree_rows(&buffer).is_empty(), "in {height} rows");
            }
        }
    }

    #[test]
    fn the_footer_shows_the_engines_counts_and_the_keys() {
        let tree = fixture::tree();
        let app = App::from_tree(&tree);
        let height = 10;

        // Wide enough for the whole key line — wider than the line, so that
        // adding a key widens the terminal here rather than quietly truncating
        // what is being asserted. What is under test is which keys it lists,
        // not how it survives a narrow terminal.
        let buffer = render(&app, KEYS_WIDTH, height);

        let tally = footer_line(&buffer, 0);
        for state in NodeState::ALL {
            assert!(
                tally.contains(&format!(
                    "{} {}",
                    tree.counts().get(state),
                    super::noun(state)
                )),
                "footer {tally:?} is missing the count for {state:?}"
            );
        }
        let keys = footer_line(&buffer, 1);
        // Every key, in full: equality rather than a bag of substrings, so a
        // line that has grown past the width it is drawn at fails here instead
        // of quietly losing whatever sat on the right-hand end of it.
        assert_eq!(
            keys,
            keys_line(app.mouse_captured(), usize::from(KEYS_WIDTH))
        );
        // "p: pact" and not the bare "p", which "PgUp" would satisfy.
        for key in [
            "k/j: row",
            // The page keys carry no label of their own: the word was already
            // in the keys' names, and those columns bought `s: scope`.
            "PgUp",
            "PgDn",
            "g/G: ends",
            // Named, not left to be discovered: the three keys that change what
            // there is to scroll through.
            "space: fold",
            "o: pacts",
            "f: files",
            "p: pact",
            // The two keys that run passes, next to each other because the
            // question they answer is the same one, and then the key that needs
            // one of them to have been pressed already.
            "r: refresh",
            "s: scope",
            // The mouse key, named by what pressing it does next rather than by
            // the state it is in: see
            // `the_keys_line_names_the_mouse_key_by_what_the_next_press_does`.
            "m: mouse",
            "q",
            "Esc",
            "Ctrl-C",
        ] {
            assert!(keys.contains(key), "footer {keys:?} is missing {key}");
        }
        // And the lines either side of it are the footer's own, untouched by
        // the new key: the tally still counts the whole tree, and the message
        // line is blank because nothing has been said.
        assert_eq!(footer_line(&buffer, FOOTER_HEIGHT - 1), "");
    }

    #[test]
    fn the_filter_changes_which_rows_are_drawn_and_nothing_else_in_the_footer() {
        let tree = fixture::tree();
        let mut app = App::from_tree(&tree);
        let height = FIXTURE_HEIGHT;

        let before = render(&app, WIDTH, height);
        app.toggle_pacted_only();
        let after = render(&app, WIDTH, height);

        // The tally describes the tree, not the window onto it, so it says the
        // same thing with the filter on; the keys line and the message line are
        // the same too.
        for line in 0..FOOTER_HEIGHT {
            let y = height - FOOTER_HEIGHT + line;
            assert_eq!(row_text(&before, y), row_text(&after, y), "footer line {y}");
        }
        // While the tree above it really did lose rows.
        assert_ne!(tree_rows(&before), tree_rows(&after));
    }

    #[test]
    fn a_refused_toggle_shows_its_message_on_the_footers_last_line_until_the_next_key() {
        let mut app = App::from_tree(&fixture::tree());
        // Onto a file, which is the one row `p` refuses: a pact is made with
        // the directory holding a file, not with the file.
        app.toggle_files();
        app.select_next();
        assert!(
            app.selected_row().expect("a row is selected").is_file(),
            "the second row with files shown is the root's first file"
        );

        assert!(app.toggle_pact().is_none(), "the toggle should be refused");

        // Wide enough for the whole sentence: what is under test is that it is
        // on screen, not how it survives a narrow terminal.
        let height = 10;
        let buffer = render(&app, 120, height);
        let said = app.message().expect("a refusal says why").to_owned();
        assert!(said.contains("is a file"), "{said:?}");
        assert_eq!(footer_line(&buffer, FOOTER_HEIGHT - 1), said);
        // And it took nothing else's line: the tally and the keys are still on
        // the two lines above it.
        assert!(footer_line(&buffer, 0).contains("unpacted"));
        assert!(footer_line(&buffer, 1).contains("p: pact"));

        // The next keystroke moves on, and the line goes blank again.
        app.select_next();
        let buffer = render(&app, 120, height);
        assert_eq!(app.message(), None);
        assert_eq!(footer_line(&buffer, FOOTER_HEIGHT - 1), "");
    }

    #[test]
    fn a_pact_in_flight_names_its_directory_on_the_footers_last_line() {
        let mut app = App::from_tree(&fixture::tree());
        let height = 10;
        let before = render(&app, KEYS_WIDTH, height);

        app.set_pact_in_flight("warlock/crates/engine", 3, 12);
        let buffer = render(&app, KEYS_WIDTH, height);

        // On the last row of the footer, which is the last row of the screen.
        // The fixture's paths are relative, and the engine's manifest spelling
        // takes a relative path to be relative to the root already; the app
        // tests cover the cutting-down an absolutely-rooted tree gets.
        assert_eq!(
            footer_line(&buffer, FOOTER_HEIGHT - 1),
            "pacting warlock/crates/engine (3/12)"
        );
        // The tally has not moved, and no fourth line grew under the footer:
        // the progress line took the message line rather than adding one.
        assert_eq!(footer_line(&buffer, 0), footer_line(&before, 0));
        assert_eq!(buffer.area.height, height);
        // And the tree above the footer is untouched: nothing marks the
        // directory being worked.
        assert_eq!(tree_rows(&buffer), tree_rows(&before));

        // It moves with the run.
        app.set_pact_in_flight("warlock/assets", 4, 12);
        let buffer = render(&app, KEYS_WIDTH, height);
        assert_eq!(
            footer_line(&buffer, FOOTER_HEIGHT - 1),
            "pacting warlock/assets (4/12)"
        );

        // And goes when the run does, leaving the line as blank as it started.
        app.clear_pact_in_flight();
        assert_eq!(
            rows_text(&render(&app, KEYS_WIDTH, height)),
            rows_text(&before)
        );
    }

    #[test]
    fn a_summarising_pass_words_the_progress_line_and_grows_no_fourth_footer_line() {
        let mut app = App::from_tree(&fixture::tree());
        let height = 10;

        app.set_pact_in_flight("warlock/crates/engine", 3, 12);
        let quiet = render(&app, KEYS_WIDTH, height);
        app.set_pact_summarising("warlock/crates/engine/Cargo.toml", 2, 9);
        let buffer = render(&app, KEYS_WIDTH, height);

        // A file too big for one request is a dozen model passes inside one
        // directory, and the footer says so: same last row, same directory and
        // fraction leading it, with the file and its part of the total added.
        assert_eq!(
            footer_line(&buffer, FOOTER_HEIGHT - 1),
            "pacting warlock/crates/engine (3/12) — summarising warlock/crates/engine/Cargo.toml (2/9)"
        );
        // The keys line is untouched — a pass running is not a key to press —
        // and so is the tally above it.
        assert_eq!(
            footer_line(&buffer, 1),
            pacting_keys_line(usize::from(KEYS_WIDTH))
        );
        assert_eq!(footer_line(&buffer, 0), footer_line(&quiet, 0));
        // And the footer is still exactly `FOOTER_HEIGHT` lines: the longer line
        // took the message line it was already on rather than wrapping onto a
        // fourth, so nothing above the footer moved and the tree is the tree it
        // was before the pass was announced.
        assert_eq!(buffer.area.height, height);
        for y in 0..height - FOOTER_HEIGHT {
            assert_eq!(row_text(&buffer, y), row_text(&quiet, y), "row {y}");
        }
        assert_eq!(tree_rows(&buffer), tree_rows(&quiet));

        // The refusal suffix still goes last, after the summarising clause, so
        // a narrow terminal cuts the answer to a keystroke before the fraction.
        app.set_pact_refused();
        assert!(
            row_text(&render(&app, KEYS_WIDTH, height), height - 1)
                .ends_with("(2/9) — already running")
        );
    }

    #[test]
    fn a_refused_pact_press_re_words_the_progress_line_and_moves_nothing_else() {
        let mut app = App::from_tree(&fixture::tree());
        let height = 10;

        app.set_pact_in_flight("warlock/crates/engine", 3, 12);
        let quiet = render(&app, KEYS_WIDTH, height);
        app.set_pact_refused();
        let buffer = render(&app, KEYS_WIDTH, height);

        // The refusal is on the progress line, on the end of it, and the
        // progress line is where it has always been: the last row of the
        // screen, which is the last row of the footer.
        assert_eq!(
            footer_line(&buffer, FOOTER_HEIGHT - 1),
            "pacting warlock/crates/engine (3/12) — already running"
        );
        // The keys line is still the pacting one: a press that started nothing
        // does not change what the keys do, and Esc still says cancel.
        assert_eq!(
            footer_line(&buffer, 1),
            pacting_keys_line(usize::from(KEYS_WIDTH))
        );
        // The tally is untouched, on the first line of the footer.
        assert_eq!(footer_line(&buffer, 0), footer_line(&quiet, 0));
        // And the footer is still exactly `FOOTER_HEIGHT` lines: no fourth line
        // grew under it, and nothing above it moved up to make room — every row
        // over the footer is the row it was before the press, tree included.
        assert_eq!(buffer.area.height, height);
        for y in 0..height - FOOTER_HEIGHT {
            assert_eq!(row_text(&buffer, y), row_text(&quiet, y), "row {y}");
        }
        assert_eq!(tree_rows(&buffer), tree_rows(&quiet));
    }

    #[test]
    fn a_terminal_too_narrow_for_the_refusal_cuts_the_suffix_and_keeps_the_fraction() {
        let mut app = App::from_tree(&fixture::tree());
        let height = 10;
        app.set_pact_in_flight("warlock/crates/engine", 3, 12);
        app.set_pact_refused();

        // The two halves of the line, so the test can be drawn at a width that
        // fits one and not the other.
        let progress = "pacting warlock/crates/engine (3/12)";
        let suffix = " — already running";
        assert_eq!(
            app.pact_line().expect("a pact is in flight"),
            format!("{progress}{suffix}"),
            "the app words the line; this test only measures it"
        );
        let fits = u16::try_from(display_width(progress) + 2 * usize::from(BORDER_THICKNESS))
            .expect("a footer line's worth of columns");

        // Drawn with exactly the fraction's columns of footer and not one more,
        // the inset at each edge aside: the fraction survives whole and the
        // suffix is the part that is gone.
        let buffer = render(&app, fits, height);
        assert_eq!(footer_line(&buffer, FOOTER_HEIGHT - 1), progress);
        // Gone rather than moved: an unwrapped line is cut at the right edge,
        // so no part of the suffix turns up on a line of its own, and the
        // footer is still three lines with the keys on the middle one.
        for y in 0..height - 1 {
            assert!(
                !row_text(&buffer, y).contains("already"),
                "the suffix wrapped onto row {y}"
            );
        }
        // The keys line is still on the middle line of a three-line footer, laid
        // out for this width rather than cut off at it: `fits` is narrower than
        // the whole pacting line, so what is on it is what survived — the two
        // names that answer "how do I stop this?".
        assert_eq!(
            footer_line(&buffer, 1),
            pacting_keys_line(footer_width(fits))
        );
        assert!(footer_line(&buffer, 1).contains(CANCEL_KEY));
        assert_eq!(buffer.area.height, height);

        // The cut is at the right edge and column by column: two columns wider
        // and the first of the suffix is back, with the fraction still whole in
        // front of it rather than shortened to make room for it.
        let wider = render(&app, fits + 2, height);
        assert_eq!(
            footer_line(&wider, FOOTER_HEIGHT - 1),
            format!("{progress} —")
        );

        // And a terminal wide enough for the whole of it draws the whole of it.
        let whole = u16::try_from(
            display_width(&format!("{progress}{suffix}")) + 2 * usize::from(BORDER_THICKNESS),
        )
        .expect("a footer line's worth of columns");
        assert_eq!(
            footer_line(&render(&app, whole, height), FOOTER_HEIGHT - 1),
            format!("{progress}{suffix}")
        );
    }

    #[test]
    fn the_keys_line_advertises_esc_as_cancel_while_a_pact_runs_and_says_quit_otherwise() {
        let mut app = App::from_tree(&fixture::tree());
        let height = 10;

        let idle = render(&app, KEYS_WIDTH, height);
        app.set_pact_in_flight("warlock/crates/engine", 3, 12);
        let pacting = render(&app, KEYS_WIDTH, height);

        // Byte for byte today's line with no pact running, and the pacting line
        // whole while one is: equality, so a line that outgrew the terminal it
        // is drawn on fails here rather than losing its right-hand end quietly.
        assert_eq!(
            footer_line(&idle, 1),
            keys_line(app.mouse_captured(), footer_width(KEYS_WIDTH))
        );
        assert_eq!(
            footer_line(&pacting, 1),
            pacting_keys_line(footer_width(KEYS_WIDTH))
        );
        // Esc means two things, and the line says which one it means now.
        let said = footer_line(&pacting, 1);
        assert!(said.contains("Esc: cancel"), "{said:?}");
        assert!(!said.contains("Esc/Ctrl-C: quit"), "{said:?}");
        assert!(keys_line(true, footer_width(KEYS_WIDTH)).contains("Esc/Ctrl-C: quit"));

        // The line is short enough to survive the narrow terminal the other
        // footer tests draw on whole, because it is the line that answers "how
        // do I stop this?": every name on it, in order, laid out or not.
        let narrow = render(&app, 120, height);
        assert_eq!(
            footer_line(&narrow, 1),
            "up/down k/j: move    space: collapse    Esc: cancel    q/Ctrl-C: quit"
        );

        // And the run ending puts today's line back, exactly.
        app.clear_pact_in_flight();
        assert_eq!(
            footer_line(&render(&app, KEYS_WIDTH, height), 1),
            keys_line(app.mouse_captured(), footer_width(KEYS_WIDTH))
        );
    }

    #[test]
    fn the_pacting_keys_line_gives_movement_up_before_either_way_out() {
        // What the line reads on a terminal with room for all of it, spelled
        // out here rather than assembled, so a name that changed has to be
        // changed here too.
        let whole = "up/down k/j: move    space: collapse    Esc: cancel    q/Ctrl-C: quit";
        assert_eq!(PACTING_KEYS.join(KEY_GAP), whole);
        let fits = display_width(whole);

        // Exactly today's line at any width that can hold it, down to the
        // column.
        assert_eq!(pacting_keys_line(usize::from(KEYS_WIDTH)), whole);
        assert_eq!(pacting_keys_line(fits), whole);

        // A column short of it and movement is what goes: the arrows and j/k
        // are the guesses a reader already has.
        assert_eq!(
            pacting_keys_line(fits - 1),
            "space: collapse    Esc: cancel    q/Ctrl-C: quit"
        );
        // Narrower still and the fold goes too, leaving both answers to "how do
        // I stop this?" — the run's own stop and the way out of warlock.
        assert_eq!(
            pacting_keys_line(
                display_width("space: collapse    Esc: cancel    q/Ctrl-C: quit") - 1
            ),
            "Esc: cancel    q/Ctrl-C: quit"
        );
        // And below even those two, they are cut rather than dropped: half a
        // name is still the start of the answer.
        assert_eq!(
            pacting_keys_line(display_width("Esc: cancel    q/Ctrl-C: quit") - 1),
            "Esc: cancel    q/Ctrl-C: qui"
        );

        // At every width from wider than the line down to nothing: what is
        // drawn fits, and no name outlives one that is kept after it.
        for width in 0..=fits + 8 {
            let line = pacting_keys_line(width);
            assert!(display_width(&line) <= width, "{width} columns: {line:?}");
            assert!(
                !line.contains(MOVE_KEYS) || line.contains(COLLAPSE_KEY),
                "{width} columns kept movement over the fold: {line:?}"
            );
            assert!(
                !line.contains(COLLAPSE_KEY) || line.contains(CANCEL_KEY),
                "{width} columns kept the fold over cancel: {line:?}"
            );
        }
    }

    /// The names the idle keys line carries while the mouse key reads one way or
    /// the other: [`KEYS`], that name, and the way out, in the order
    /// [`keys_line`] assembles them.
    ///
    /// Joined by [`KEY_GAP`] this is the whole line, so its width is the width
    /// the line needs — which is what the tests below shrink a terminal down
    /// from.
    fn idle_keys(mouse_captured: bool) -> Vec<&'static str> {
        let mut pieces = KEYS.to_vec();
        pieces.push(if mouse_captured {
            MOUSE_OFF_KEY
        } else {
            MOUSE_ON_KEY
        });
        pieces.push(QUIT_KEY);
        pieces
    }

    #[test]
    fn the_footer_keeps_the_whole_way_out_on_an_eighty_column_terminal() {
        let mut app = App::from_tree(&fixture::tree());
        let height = 10;
        let columns = footer_width(EIGHTY_COLUMNS);

        // Eighty columns is narrower than the whole keys line, so this is a line
        // that has given names up — and the name it keeps is the one a stuck
        // reader is looking for, whichever way the mouse key reads.
        for captured in [true, false] {
            app.set_mouse_captured(captured);

            let keys = footer_line(&render(&app, EIGHTY_COLUMNS, height), 1);

            // Whole, not the first few characters of it: `contains` of the
            // entire name, with nothing after it on the line.
            assert!(
                keys.contains(QUIT_KEY),
                "mouse captured {captured}: {keys:?}"
            );
            assert!(
                keys.ends_with(QUIT_KEY),
                "mouse captured {captured}: {keys:?}"
            );
            assert!(
                display_width(&keys) <= columns,
                "mouse captured {captured}: {keys:?}"
            );
            // And it really did have to give something up to keep it: this is
            // not a width the whole line fits in.
            assert!(
                !keys.contains(PAGE_KEYS),
                "mouse captured {captured}: {keys:?}"
            );
        }

        // While a pact runs the way out is `PACTING_QUIT_KEY` — Esc is spoken
        // for by `CANCEL_KEY` — and both of the line's answers to "how do I stop
        // this?" are on eighty columns whole.
        app.set_pact_in_flight("warlock/crates/engine", 3, 12);
        for captured in [true, false] {
            app.set_mouse_captured(captured);

            let keys = footer_line(&render(&app, EIGHTY_COLUMNS, height), 1);

            assert!(
                keys.contains(PACTING_QUIT_KEY),
                "mouse captured {captured}: {keys:?}"
            );
            assert!(
                keys.ends_with(PACTING_QUIT_KEY),
                "mouse captured {captured}: {keys:?}"
            );
            assert!(
                keys.contains(CANCEL_KEY),
                "mouse captured {captured}: {keys:?}"
            );
            assert!(
                display_width(&keys) <= columns,
                "mouse captured {captured}: {keys:?}"
            );
        }
    }

    /// What eighty columns costs, now that the composer has names on the line:
    /// its four and no other name before them, with the way out still whole.
    ///
    /// The assertions are about the row the footer drew rather than about what
    /// [`keys_line`] returned, because "nothing moves off the footer" is a claim
    /// about the screen: a line that fit the layout and was then cut by the
    /// backend would pass the one and fail the other.
    #[test]
    fn the_composers_names_are_the_first_the_eighty_column_footer_gives_up() {
        let mut app = App::from_tree(&fixture::tree());
        let height = 10;
        let columns = footer_width(EIGHTY_COLUMNS);
        // In the order `KEY_DROP_ORDER` loses them, which is not the order
        // `KEYS` lists them in.
        let composer = [COMPOSE_KEYS, LEAVE_KEY, COMMAND_KEY, FOCUS_KEY];

        for captured in [true, false] {
            app.set_mouse_captured(captured);
            let pieces = idle_keys(captured);

            let keys = footer_line(&render(&app, EIGHTY_COLUMNS, height), 1);

            // The names this width could not afford, in the order the line is
            // documented to give them up in.
            let dropped: Vec<&str> = KEY_DROP_ORDER
                .iter()
                .copied()
                .filter(|name| pieces.contains(name) && !keys.contains(name))
                .collect();
            // The composer's three are the first names gone, and nothing else
            // went before them: the names given up are a prefix of the order,
            // so no name that outranks them was spent instead.
            let order: Vec<&str> = KEY_DROP_ORDER
                .iter()
                .copied()
                .filter(|name| pieces.contains(name))
                .collect();
            assert!(
                dropped.len() >= composer.len(),
                "captured {captured}: {keys:?} gave up less than the composer"
            );
            assert_eq!(
                dropped,
                order[..dropped.len()].to_vec(),
                "captured {captured}: {keys:?} skipped a name in the drop order"
            );
            assert_eq!(
                dropped[..composer.len()].to_vec(),
                composer.to_vec(),
                "captured {captured}: {keys:?}"
            );
            for name in composer {
                assert!(!keys.contains(name), "captured {captured}: {keys:?}");
            }

            // What is left is whole names joined by `KEY_GAP` and nothing else
            // — no half-drawn name, no leftover gap where one used to be.
            let survivors: Vec<&str> = pieces
                .iter()
                .copied()
                .filter(|name| !dropped.contains(name))
                .collect();
            assert_eq!(keys, survivors.join(KEY_GAP), "captured {captured}");
            for name in &survivors {
                assert!(keys.contains(name), "captured {captured}: {keys:?}");
            }

            // And the way out is on it, whole and at the end of it, inside the
            // eighty columns the footer was given.
            assert!(keys.contains(QUIT_KEY), "captured {captured}: {keys:?}");
            assert!(keys.ends_with(QUIT_KEY), "captured {captured}: {keys:?}");
            assert!(
                display_width(&keys) <= columns,
                "captured {captured}: {keys:?}"
            );
        }
    }

    #[test]
    fn the_drawn_keys_line_never_outgrows_the_terminal_it_is_drawn_on() {
        let mut app = App::from_tree(&fixture::tree());
        let height = 10;

        // Every width from one column up to wider than the whole line, in both
        // mouse states and with a pact running and without: measured in columns
        // by `display_width`, which is what the backend charges for the row,
        // rather than in bytes.
        //
        // The line the layout produced and the line on screen, both: a row read
        // off the buffer fits the buffer whatever the widget did to it, so the
        // fit is asserted about the laid-out line and the row is asserted to be
        // that line — which is what says the backend cut nothing off the end.
        for pacting in [false, true] {
            if pacting {
                app.set_pact_in_flight("warlock/crates/engine", 3, 12);
            }
            for captured in [true, false] {
                app.set_mouse_captured(captured);
                for width in 1..=KEYS_WIDTH {
                    let columns = footer_width(width);
                    let laid_out = if pacting {
                        pacting_keys_line(columns)
                    } else {
                        keys_line(captured, columns)
                    };

                    let keys = footer_line(&render(&app, width, height), 1);

                    assert!(
                        display_width(&laid_out) <= columns,
                        "{width} columns, pacting {pacting}, captured {captured}: {laid_out:?}"
                    );
                    // Trailing blanks are trimmed off a row and off nothing
                    // else, so the row is the line with its own tail intact.
                    assert_eq!(
                        keys,
                        laid_out.trim_end(),
                        "{width} columns, pacting {pacting}, captured {captured}"
                    );
                    assert!(
                        display_width(&keys) <= columns,
                        "{width} columns, pacting {pacting}, captured {captured}: {keys:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn the_keys_line_gives_its_names_up_in_order_rather_than_losing_its_tail() {
        for captured in [true, false] {
            let pieces = idle_keys(captured);
            let whole = pieces.join(KEY_GAP);
            let full = display_width(&whole);
            // A terminal with room for all of it gets all of it.
            assert_eq!(keys_line(captured, full), whole, "captured {captured}");

            // The order this line's names are expected to go in: the documented
            // one, less the two reserved for keys that do not exist yet and the
            // mouse name this state does not use.
            let expected: Vec<&str> = KEY_DROP_ORDER
                .iter()
                .copied()
                .filter(|name| pieces.contains(name))
                .collect();
            assert_eq!(
                expected.iter().take(5).copied().collect::<Vec<&str>>(),
                vec![COMPOSE_KEYS, LEAVE_KEY, COMMAND_KEY, FOCUS_KEY, PAGE_KEYS],
                "the composer's names go first, then the page keys: {expected:?}"
            );

            // Column by column down from the whole line, noting each name as it
            // disappears. What comes out is the order they went in.
            let mut present = pieces.clone();
            let mut dropped: Vec<&str> = Vec::new();
            for width in (0..=full).rev() {
                let line = keys_line(captured, width);

                for name in &expected {
                    if present.contains(name) && !line.contains(name) {
                        dropped.push(name);
                        present.retain(|kept| kept != name);
                    }
                }
                assert!(
                    display_width(&line) <= width,
                    "{width} columns, captured {captured}: {line:?}"
                );
                // And the way out is whole at every width that could hold it,
                // however much else has gone.
                if width >= display_width(QUIT_KEY) {
                    assert!(
                        line.contains(QUIT_KEY),
                        "{width} columns, captured {captured}: {line:?}"
                    );
                }
            }
            assert_eq!(dropped, expected, "captured {captured}");

            // Which is a different line from the one a terminal that cut at its
            // right-hand edge would draw: that line is a prefix of the whole,
            // and this one is not — it kept its tail and lost its middle.
            let narrow = keys_line(captured, usize::from(EIGHTY_COLUMNS));
            assert!(
                narrow.ends_with(QUIT_KEY),
                "captured {captured}: {narrow:?}"
            );
            assert!(
                !whole.starts_with(&narrow),
                "captured {captured}: {narrow:?} is the whole line cut at the edge"
            );
        }
    }

    #[test]
    fn the_way_out_is_cut_rather_than_dropped_on_a_terminal_narrower_than_its_name() {
        let quit = display_width(QUIT_KEY);

        for captured in [true, false] {
            // Exactly the width of the name and nothing to spare: the name, and
            // only it.
            assert_eq!(keys_line(captured, quit), QUIT_KEY, "captured {captured}");

            // Below that there is nothing left to give up, so the name is cut —
            // the start of the way out rather than a blank line — down to the
            // terminal a single column wide.
            for width in 0..quit {
                assert_eq!(
                    keys_line(captured, width),
                    QUIT_KEY[..width],
                    "{width} columns, captured {captured}"
                );
            }
        }

        // And that is what is drawn, not just what is assembled: a terminal with
        // thirteen columns of footer says as much of the way out as it has room
        // for. Two columns wider than the text, because the footer's lines are
        // inset a column at each edge (see [`footer_text_area`]).
        let cut = "q/Esc/Ctrl-C:";
        let width = u16::try_from(display_width(cut) + 2 * usize::from(BORDER_THICKNESS))
            .expect("a footer line's worth of columns");
        let height = 10;

        let buffer = render(&App::from_tree(&fixture::tree()), width, height);

        assert_eq!(footer_line(&buffer, 1), cut);
    }

    #[test]
    fn the_footers_lines_start_in_the_column_the_panels_rows_do() {
        // The one thing the inset is for. The footer draws no border, so left
        // alone its three lines would begin in the column the panel's border
        // owns — one to the left of every row of text above them, which reads as
        // a slip rather than as a band of its own.
        let base = Instant::now();
        let mut app = pacting_app(base, WIDTH, HEIGHT);
        let account = app.account_mut().expect("a pact has started");
        account.open_section("crates/warlock-engine", base);
        let buffer = render_at(&app, WIDTH, HEIGHT, at(base, 1));

        let panel = panel_area(&buffer);
        let footer = HEIGHT - FOOTER_HEIGHT;

        // The account's heading is flush against the inside of the border, and
        // the tally and the keys under it begin in that very column — the
        // footer's own rows have no border in front of them, so the first thing
        // drawn on them is the first column of text on the screen.
        assert_ne!(
            buffer[(panel.x, panel.y)].symbol(),
            " ",
            "the account's heading is not flush left"
        );
        assert_eq!(first_column(&buffer, footer), Some(panel.x));
        assert_eq!(first_column(&buffer, footer + 1), Some(panel.x));
    }

    /// The first column of row `y` with something drawn in it, or `None` for a
    /// row that is all blanks.
    fn first_column(buffer: &Buffer, y: u16) -> Option<u16> {
        (buffer.area.x..buffer.area.x + buffer.area.width).find(|&x| buffer[(x, y)].symbol() != " ")
    }

    #[test]
    fn a_footer_with_room_for_every_key_draws_three_dim_lines_and_gives_nothing_up() {
        let tree = fixture::tree();
        let mut app = App::from_tree(&tree);
        app.set_message("nothing to refresh");
        let height = 10;

        let buffer = render(&app, KEYS_WIDTH, height);

        // Three lines and no more, where they have always been.
        let footer = areas(buffer.area, None).footer;
        assert_eq!(footer.height, FOOTER_HEIGHT);
        assert_eq!(footer.y, height - FOOTER_HEIGHT);

        // The tally, the whole keys line with nothing given up, and the message.
        let tally: Vec<String> = NodeState::ALL
            .iter()
            .map(|state| format!("{} {}", tree.counts().get(*state), super::noun(*state)))
            .collect();
        assert_eq!(footer_line(&buffer, 0), tally.join("  "));
        assert_eq!(
            footer_line(&buffer, 1),
            idle_keys(app.mouse_captured()).join(KEY_GAP)
        );
        assert_eq!(footer_line(&buffer, 2), "nothing to refresh");

        // The keys and the message are dim, every column of them, and the tally
        // is not: its colours are the node states', which dimness would mute.
        // Read from where the footer's text starts, which is a column in from
        // the edge the band begins at.
        let text_area = footer_text_area(footer);
        for line in [1, 2] {
            let y = footer.y + line;
            let text = text_in(&buffer, text_area, y);
            let columns = u16::try_from(display_width(&text)).expect("a line's worth of columns");
            for x in text_area.x..text_area.x + columns {
                assert!(
                    buffer[(x, y)].modifier.contains(Modifier::DIM),
                    "({x}, {y}) is not dim"
                );
            }
        }
        let first = &buffer[(text_area.x, footer.y)];
        assert!(!first.modifier.contains(Modifier::DIM));
        assert_eq!(first.fg, colour_for(NodeState::Unpacted));
    }

    #[test]
    fn the_keys_line_names_the_mouse_key_by_what_the_next_press_does() {
        let mut app = App::from_tree(&fixture::tree());
        let height = 10;

        // Reporting its mouse, which is how warlock starts: the key on offer is
        // the one that stops it.
        app.set_mouse_captured(true);
        let capturing = render(&app, KEYS_WIDTH, height);
        let keys = footer_line(&capturing, 1);
        assert_eq!(keys, keys_line(true, footer_width(KEYS_WIDTH)));
        assert!(keys.contains(MOUSE_OFF_KEY), "{keys:?}");
        assert!(!keys.contains(MOUSE_ON_KEY), "{keys:?}");

        // And with capture off, the same key named by what it does now: turn it
        // back on. This is the wording that matters — it is the only thing on
        // screen that says the wheel is the terminal's for the moment.
        app.set_mouse_captured(false);
        let released = render(&app, KEYS_WIDTH, height);
        let keys = footer_line(&released, 1);
        assert_eq!(keys, keys_line(false, footer_width(KEYS_WIDTH)));
        assert!(keys.contains(MOUSE_ON_KEY), "{keys:?}");
        assert!(!keys.contains(MOUSE_OFF_KEY), "{keys:?}");

        // Nothing else on the screen moved: the toggle is a fact about the
        // terminal, not about the tree, and it is not announced on the message
        // line either — that line is blank in both frames, and every row above
        // the keys line is the row it was.
        assert_eq!(footer_line(&capturing, FOOTER_HEIGHT - 1), "");
        assert_eq!(footer_line(&released, FOOTER_HEIGHT - 1), "");
        for row in 0..=(height - FOOTER_HEIGHT) {
            assert_eq!(
                row_text(&capturing, row),
                row_text(&released, row),
                "row {row}"
            );
        }

        // A pact in flight takes the line whichever way the toggle is left: the
        // short line that answers "how do I stop this?" is not the place for a
        // key about the pointer, and `PACTING_KEYS` says so by not naming it.
        app.set_pact_in_flight("warlock/crates/engine", 3, 12);
        let pacting = pacting_keys_line(footer_width(KEYS_WIDTH));
        assert_eq!(footer_line(&render(&app, KEYS_WIDTH, height), 1), pacting);
        app.set_mouse_captured(true);
        assert_eq!(footer_line(&render(&app, KEYS_WIDTH, height), 1), pacting);
        assert!(!pacting.contains("mouse"));
    }

    #[test]
    fn an_empty_app_draws_a_footer_and_no_rows() {
        // No scope either, so the header is blank along with the tree: an app
        // nobody told where its tree came from claims nothing.
        let app = App::from_rows(Vec::new());
        let height = HEIGHT;

        let buffer = render(&app, WIDTH, height);

        assert_eq!(header_text(&buffer), "");
        for (index, row) in tree_rows(&buffer).iter().enumerate() {
            assert_eq!(row, "", "tree row {index} should be blank");
        }
        assert!(footer_line(&buffer, 0).contains("0 unpacted"));
    }

    #[test]
    fn the_header_names_the_root_relative_to_the_repository_root() {
        let chrome = Chrome::of(
            Path::new("/repo"),
            Path::new("/repo").join("crates").join("warlock-engine"),
        );

        let buffer = render_chrome(&header_app(), &chrome, WIDTH, HEIGHT);

        // Forward slashes whatever the platform's separator is, because this
        // is the engine's manifest spelling of a module.
        assert_eq!(header_text(&buffer), "crates/warlock-engine");
    }

    #[test]
    fn a_tree_rooted_at_the_repository_root_draws_a_blank_header_and_keeps_its_row() {
        let buffer = render_chrome(&header_app(), &Chrome::of("/repo", "/repo"), WIDTH, HEIGHT);

        // Nothing to say — the whole repository is not a part of itself — and
        // the root row below already names the directory.
        assert_eq!(header_text(&buffer), "");
        // The row is still the header's, though, so the tree starts where it
        // starts whether or not there is a name on the line above it.
        assert_eq!(header_area(&buffer).height, HEADER_HEIGHT);
        assert_eq!(rows_area(&buffer).y, header_area(&buffer).y + HEADER_HEIGHT);
        assert!(tree_row(&buffer, 0).contains("warlock"));
    }

    #[test]
    fn the_header_states_what_this_machine_holds_after_the_tree_it_names() {
        let chrome = held_chrome(Sigils::held(["billing", "web"]));

        let buffer = render_chrome(&header_app(), &chrome, HELD_WIDTH, HEIGHT);

        // One line, both facts, the identity first: what am I looking at, and
        // then what do I hold for it.
        assert_eq!(
            header_text(&buffer),
            format!("crates{HEADER_GAP}holding `billing`, `web`")
        );
        assert_eq!(header_area(&buffer).height, HEADER_HEIGHT);
    }

    #[test]
    fn a_config_that_would_not_read_says_so_on_the_header_rather_than_going_quiet() {
        let buffer = render_chrome(
            &header_app(),
            &held_chrome(Sigils::Unknown),
            HELD_WIDTH,
            HEIGHT,
        );

        // Broken is never drawn as absent: a reader whose config will not parse
        // is told, on the same line and in the same row.
        assert_eq!(
            header_text(&buffer),
            format!("crates{HEADER_GAP}holding unknown")
        );
        assert_eq!(header_area(&buffer).height, HEADER_HEIGHT);
    }

    #[test]
    fn a_machine_holding_nothing_draws_the_frame_it_always_drew() {
        // Byte for byte, at both widths, in both spellings of nothing: the
        // reader who never runs `warlock config` must not be able to tell that
        // any of this arrived.
        let app = header_app();
        let before = held_chrome(Sigils::Nothing);
        let never_told = Chrome::of("/repo", "/repo/crates");

        for width in [HELD_WIDTH, NARROW_WIDTH] {
            let drawn = render_chrome(&app, &before, width, HEIGHT);

            assert_eq!(header_text(&drawn), "crates", "at {width} columns");
            assert_eq!(
                rows_text(&drawn),
                rows_text(&render_chrome(&app, &never_told, width, HEIGHT)),
                "at {width} columns"
            );
            assert_eq!(
                rows_text(&drawn),
                rows_text(&render_chrome(
                    &app,
                    &held_chrome(Sigils::held(Vec::<String>::new())),
                    width,
                    HEIGHT
                )),
                "at {width} columns"
            );
        }
    }

    #[test]
    fn a_header_with_no_room_for_both_drops_the_holding_and_keeps_the_tree_it_names() {
        let chrome = held_chrome(Sigils::held(["billing", "web"]));
        // Twenty columns of tree pane, eighteen inside it: room for the
        // identity several times over, and nowhere near room for both.
        let narrow = pane_inner(areas(Rect::new(0, 0, NARROW_WIDTH, HEIGHT), None).tree);
        assert!(
            display_width(&format!("crates{HEADER_GAP}holding `billing`, `web`"))
                > usize::from(narrow.width)
        );

        let buffer = render_chrome(&header_app(), &chrome, NARROW_WIDTH, HEIGHT);

        // The whole holding is gone rather than half of it: the identity is the
        // answer to "what am I looking at", so it is never spent on sigils, and
        // half a set of sigils would be a claim about what is held that is not
        // true.
        let header = header_text(&buffer);
        assert_eq!(header, "crates");
        assert!(!header.contains(ELLIPSIS), "{header}");
        assert!(!header.contains("holding"), "{header}");
        // And the line still keeps its row, so the tree starts where it started.
        assert_eq!(header_area(&buffer).height, HEADER_HEIGHT);
        assert_eq!(rows_area(&buffer).y, header_area(&buffer).y + HEADER_HEIGHT);
    }

    #[test]
    fn the_header_sits_above_the_first_tree_row_inside_the_panes_border() {
        let buffer = render_chrome(
            &header_app(),
            &Chrome::of("/repo", "/repo/crates"),
            WIDTH,
            HEIGHT,
        );

        let header = header_area(&buffer);
        let rows = rows_area(&buffer);
        assert_eq!(header_text(&buffer), "crates");
        assert_eq!(tree_row(&buffer, 0), "> - warlock");
        // Immediately above the rows, and both of them inside the border rather
        // than on it: the pane's top row is the border's, not the header's.
        assert_eq!(header.y + HEADER_HEIGHT, rows.y);
        assert_eq!(header.x, rows.x);
        let pane = areas(buffer.area, None).tree;
        assert_eq!(header.y, pane.y + BORDER_THICKNESS);
        assert_eq!(header.x, pane.x + BORDER_THICKNESS);
    }

    #[test]
    fn the_panel_is_the_majority_of_the_width_and_the_tree_column_takes_its_share_of_the_rest() {
        // Wide enough that the proportion, not the floor, decides: 30% of 160 is
        // 48, comfortably past the floor of 30.
        let panes = areas(Rect::new(0, 0, KEYS_WIDTH, 24), None);

        assert_eq!(panes.tree.width, KEYS_WIDTH * TREE_PERCENT / 100);
        assert_eq!(panes.panel.width, KEYS_WIDTH - panes.tree.width);
        assert!(
            panes.panel.width > panes.tree.width,
            "the panel should take the majority: {panes:?}",
            panes = (panes.panel, panes.tree)
        );
        // Side by side, no gap and no overlap, and both above the footer.
        assert_eq!(panes.panel.x, 0);
        assert_eq!(panes.panel.x + panes.panel.width, panes.tree.x);
        assert_eq!(panes.tree.x + panes.tree.width, KEYS_WIDTH);
        assert_eq!(panes.panel.height, panes.tree.height);
        assert_eq!(panes.panel.y + panes.panel.height, panes.footer.y);
    }

    #[test]
    fn a_narrow_terminal_holds_the_tree_columns_floor_until_it_cannot() {
        // Under 100 columns the proportion would take the tree column below its
        // floor, so the floor bites and the panel takes the rest — still the
        // majority.
        for width in [TREE_MIN_WIDTH * 2, 80, 99] {
            let panes = areas(Rect::new(0, 0, width, 24), None);

            assert!(
                width * TREE_PERCENT / 100 < TREE_MIN_WIDTH,
                "{width} is wide enough for the proportion, so the floor is not what is under test"
            );
            assert_eq!(panes.tree.width, TREE_MIN_WIDTH, "at {width} columns");
            assert_eq!(
                panes.panel.width,
                width - TREE_MIN_WIDTH,
                "at {width} columns"
            );
            assert!(panes.panel.width >= panes.tree.width, "at {width} columns");
        }

        // Below twice the floor there is no way to give the tree column its
        // floor and leave the panel the majority, so the floor is what gives:
        // the two panes halve the width, and the odd column goes to the panel.
        for width in [0, 1, 20, 40, 41, TREE_MIN_WIDTH * 2 - 1] {
            let panes = areas(Rect::new(0, 0, width, 24), None);

            assert_eq!(panes.tree.width, width / 2, "at {width} columns");
            assert_eq!(panes.panel.width, width - width / 2, "at {width} columns");
            assert!(panes.panel.width >= panes.tree.width, "at {width} columns");
        }
    }

    #[test]
    fn the_width_rule_never_loses_a_column_or_lets_the_tree_column_take_the_majority() {
        for width in 0..=KEYS_WIDTH {
            let tree = tree_width(width);

            assert!(
                tree <= width - tree,
                "the tree column took {tree} of {width}"
            );
            assert!(
                tree <= TREE_MIN_WIDTH.max(width * TREE_PERCENT / 100),
                "the tree column took {tree} of {width}"
            );
        }
    }

    #[test]
    fn a_point_is_answered_with_whatever_the_frame_draws_at_it() {
        let size = Size::new(WIDTH, HEIGHT);
        let panes = areas(Rect::from(size), None);
        let hit = |x, y| hit_test(x, y, size, None, None);

        // The footer runs the full width, all three of its lines, and belongs
        // to neither pane.
        for y in panes.footer.y..panes.footer.y + panes.footer.height {
            for x in [0, WIDTH / 2, WIDTH - 1] {
                assert_eq!(hit(x, y), Hit::Footer, "at ({x}, {y})");
            }
        }

        // Every edge of both panes is border, corners included — the column
        // between the two panes is two borders and no pane's inside.
        for pane in [panes.panel, panes.tree] {
            for y in pane.y..pane.y + pane.height {
                for x in [pane.x, pane.x + pane.width - 1] {
                    assert_eq!(hit(x, y), Hit::Border, "at ({x}, {y})");
                }
            }
            for x in pane.x..pane.x + pane.width {
                for y in [pane.y, pane.y + pane.height - 1] {
                    assert_eq!(hit(x, y), Hit::Border, "at ({x}, {y})");
                }
            }
        }

        // Inside the tree pane: one header line, and a row of the window per
        // row under it, counted from the top of the window rather than of the
        // screen.
        let inside = pane_inner(panes.tree);
        let rows = tree_rows_area(panes.tree);
        assert_eq!(rows.height, tree_height(size));
        for x in inside.x..inside.x + inside.width {
            assert_eq!(hit(x, inside.y), Hit::TreeHeader, "at column {x}");
        }
        for offset in 0..rows.height {
            for x in [rows.x, rows.x + rows.width - 1] {
                assert_eq!(hit(x, rows.y + offset), Hit::TreeRow { offset });
            }
        }

        // And the whole inside of the panel is a line of its window, drawn on
        // or not: the panel has no selection for a point to land on.
        let inside = pane_inner(panes.panel);
        assert_eq!(inside.height, panel_height(size, None, None));
        for offset in 0..inside.height {
            for x in [inside.x, inside.x + inside.width - 1] {
                assert_eq!(hit(x, inside.y + offset), Hit::PanelLine { offset });
            }
        }
    }

    #[test]
    fn the_row_a_point_lands_on_is_the_row_the_frame_drew_there() {
        // Measured against a frame rather than against the layout twice over:
        // what a click lands on has to be what the reader saw at that point.
        let app = tall_app(0);
        let size = Size::new(WIDTH, HEIGHT);
        let buffer = render(&app, WIDTH, HEIGHT);

        let rows = rows_area(&buffer);
        assert!(rows.height > 1, "the window should hold more than one row");
        for offset in 0..rows.height {
            assert_eq!(
                hit_test(rows.x, rows.y + offset, size, None, None),
                Hit::TreeRow { offset }
            );
            assert_eq!(
                tree_row(&buffer, offset),
                drawn_row(usize::from(offset), 0, MANY),
                "row {offset} of the window"
            );
        }
        // The header is the line the frame drew the tree's name on, not a row.
        let header = header_area(&buffer);
        assert_eq!(
            hit_test(header.x, header.y, size, None, None),
            Hit::TreeHeader
        );
        // Blank, because this frame was drawn through `render`, which passes
        // the default `Chrome`: the line is not the app's to fill any more.
        assert_eq!(header_text(&buffer), "");
    }

    #[test]
    fn a_terminal_too_short_for_a_tree_row_answers_no_row_rather_than_underflowing() {
        for height in 0..=CHROME_HEIGHT {
            let size = Size::new(WIDTH, height);
            assert_eq!(tree_height(size), 0, "in {height} rows");

            for y in 0..height {
                for x in 0..WIDTH {
                    let hit = hit_test(x, y, size, None, None);
                    assert!(
                        !matches!(hit, Hit::TreeRow { .. }),
                        "({x}, {y}) of {height} rows answered {hit:?}"
                    );
                }
            }
        }

        // The tallest of those has room for the header and nothing under it,
        // and says so; a screen with no room even for that is all footer and
        // border.
        let header = Size::new(WIDTH, CHROME_HEIGHT);
        let inside = pane_inner(areas(Rect::from(header), None).tree);
        assert_eq!(inside.height, HEADER_HEIGHT);
        assert_eq!(
            hit_test(inside.x, inside.y, header, None, None),
            Hit::TreeHeader
        );
        for height in 0..CHROME_HEIGHT - HEADER_HEIGHT {
            let size = Size::new(WIDTH, height);
            for y in 0..height {
                for x in 0..WIDTH {
                    let hit = hit_test(x, y, size, None, None);
                    assert!(
                        matches!(hit, Hit::Footer | Hit::Border),
                        "({x}, {y}) of {height} rows answered {hit:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn a_terminal_narrow_enough_to_halve_still_tells_the_two_panes_apart() {
        // The branch of `areas` where the tree column's floor is given up and
        // the two panes split what there is: the point is that hit-testing asks
        // the layout where the panes are rather than working it out from the
        // rule.
        for width in [40, 41, TREE_MIN_WIDTH * 2 - 1] {
            let size = Size::new(width, FIXTURE_HEIGHT);
            let panes = areas(Rect::from(size), None);
            assert_eq!(
                panes.tree.width,
                width / 2,
                "at {width} columns the floor should have given way"
            );

            let inside = pane_inner(panes.panel);
            let rows = tree_rows_area(panes.tree);
            assert_eq!(
                hit_test(inside.x, inside.y, size, None, None),
                Hit::PanelLine { offset: 0 },
                "at {width} columns"
            );
            assert_eq!(
                hit_test(
                    inside.x + inside.width - 1,
                    inside.y + inside.height - 1,
                    size,
                    None,
                    None
                ),
                Hit::PanelLine {
                    offset: inside.height - 1
                },
                "at {width} columns"
            );
            assert_eq!(
                hit_test(rows.x, rows.y, size, None, None),
                Hit::TreeRow { offset: 0 },
                "at {width} columns"
            );
            // The two columns between the panes' insides are border on both
            // sides of the join, whichever pane owns which.
            for x in [inside.x + inside.width, rows.x - 1] {
                assert_eq!(
                    hit_test(x, rows.y, size, None, None),
                    Hit::Border,
                    "at {width} columns"
                );
            }
        }
    }

    #[test]
    fn no_point_is_answered_with_an_offset_its_window_has_no_room_for() {
        for (width, height) in [
            (WIDTH, HEIGHT),
            (KEYS_WIDTH, 24),
            (40, FIXTURE_HEIGHT),
            (TREE_MIN_WIDTH * 2 - 1, FILES_HEIGHT),
            (1, 1),
            (0, 0),
        ] {
            let size = Size::new(width, height);
            for y in 0..height {
                for x in 0..width {
                    match hit_test(x, y, size, None, None) {
                        Hit::TreeRow { offset } => assert!(
                            offset < tree_height(size),
                            "({x}, {y}) of {width}x{height} is row {offset} of a window {} tall",
                            tree_height(size)
                        ),
                        Hit::PanelLine { offset } => assert!(
                            offset < panel_height(size, None, None),
                            "({x}, {y}) of {width}x{height} is line {offset} of a window {} tall",
                            panel_height(size, None, None)
                        ),
                        _ => {}
                    }
                }
            }

            // And a point off the end of the frame is nothing warlock drew,
            // rather than the nearest thing it did draw.
            assert_eq!(hit_test(width, 0, size, None, None), Hit::Offscreen);
            assert_eq!(hit_test(0, height, size, None, None), Hit::Offscreen);
            assert_eq!(
                hit_test(u16::MAX, u16::MAX, size, None, None),
                Hit::Offscreen
            );
        }
    }

    #[test]
    fn the_panel_draws_the_mark_and_not_one_word_inside_it_before_the_first_pact() {
        // Both focus states and a tree with something in it: whatever the app is
        // doing, the panel has nothing to say until a pact says it, so what is
        // inside its border is warlock's mark and blank rows either side of it.
        // Neither card holds anything, which is the whole of what puts the mark
        // there: one slot with two empty cards in it draws no lines at all.
        let mut app = App::from_tree(&fixture::tree());
        assert!(!app.has_account());
        assert!(!app.has_document());
        assert!(!app.has_panel_content());
        for _ in 0..2 {
            let buffer = render(&app, MARK_ROOM_WIDTH, MARK_ROOM_HEIGHT);

            assert_mark_drawn(&buffer);
            // And its border really is there, on all four sides, carrying
            // nothing of its own: no title, and no scrollback indicator on a
            // panel with nothing to scroll.
            let panel = areas(buffer.area, None).panel;
            assert_ne!(buffer[(panel.x, panel.y)].symbol(), " ");
            assert_ne!(
                buffer[(panel.x + panel.width - 1, panel.y + panel.height - 1)].symbol(),
                " "
            );
            let edge = panel_bottom_edge(&buffer);
            assert!(!edge.contains(SCROLLBACK_ARROW), "{edge:?}");
            assert!(!edge.contains("more"), "{edge:?}");

            app.toggle_focus();
        }
    }

    #[test]
    fn the_mark_is_drawn_dim_and_uncoloured_in_the_middle_of_a_panel_with_the_room_for_it() {
        let app = App::from_tree(&fixture::tree());

        let buffer = render(&app, MARK_WIDTH, MARK_HEIGHT);

        // The threshold size itself: the panel's inside is the art plus its
        // margins and nothing more, so this pins the smallest frame the mark is
        // drawn on as well as how it is drawn.
        let inner = panel_area(&buffer);
        assert_eq!(
            usize::from(inner.width),
            mark_width() + 2 * usize::from(MARK_MARGIN)
        );
        assert_eq!(
            usize::from(inner.height),
            MARK.len() + usize::from(MARK_MARGIN_ROWS)
        );
        assert!(
            mark_area(inner).is_some(),
            "{inner:?} is the threshold size"
        );
        assert_mark_drawn(&buffer);
    }

    #[test]
    fn a_panel_a_column_or_a_row_short_of_the_mark_draws_the_bare_border() {
        let app = App::from_tree(&fixture::tree());

        // A column short, a row short, and short of both: whole or nothing, so
        // every one of them draws what the panel drew before there was a mark.
        for (width, height) in [
            (BELOW_MARK_WIDTH, MARK_HEIGHT),
            (MARK_WIDTH, BELOW_MARK_HEIGHT),
            (BELOW_MARK_WIDTH, BELOW_MARK_HEIGHT),
        ] {
            let buffer = render(&app, width, height);

            let inner = panel_area(&buffer);
            assert!(
                mark_area(inner).is_none(),
                "{inner:?} of {width}x{height} claims room for the mark"
            );
            assert_bare_panel(&buffer);
        }
    }

    #[test]
    fn a_forty_column_terminal_draws_the_empty_panel_border_and_no_mark() {
        // Tall enough that height is not what stops it: at this width the panel
        // is twenty columns, and the mark is not drawn narrower to fit.
        let app = App::from_tree(&fixture::tree());

        let buffer = render(&app, NARROW_WIDTH, FILES_HEIGHT);

        assert_eq!(areas(buffer.area, None).panel.width, 20);
        assert_bare_panel(&buffer);
    }

    #[test]
    fn an_eighty_column_terminal_is_too_narrow_for_the_mark_and_draws_the_bare_border() {
        // Fifty columns of panel, forty-eight inside its border, against a mark
        // that wants a hundred and four. The ordinary terminal is on the wrong
        // side of the threshold and that is the deliberate part: the mark is
        // sized for the wide window warlock's two panes ask for, and every
        // narrower one gets the panel exactly as it was before there was a mark.
        let app = App::from_tree(&fixture::tree());

        let buffer = render(&app, STANDARD_WIDTH, MARK_HEIGHT);

        assert_eq!(areas(buffer.area, None).panel.width, 50);
        assert_bare_panel(&buffer);
    }

    #[test]
    fn the_mark_does_not_come_back_once_a_pact_has_started() {
        // Drawn at a size with all the room in the world for the mark, so what
        // keeps it off the screen is the account and never the width.
        let base = Instant::now();
        let mut app = pacting_app(base, MARK_ROOM_WIDTH, MARK_ROOM_HEIGHT);
        assert!(app.has_account());
        assert!(
            mark_area(panel_area(&render(&app, MARK_ROOM_WIDTH, MARK_ROOM_HEIGHT))).is_some(),
            "this size has room for the mark"
        );

        // A pact under way with nothing recorded yet is still a pact: the panel
        // is empty, and empty is not the same as free. The run took the panel
        // because the card it took it from had nothing on it (see
        // [`App::start_account`]), so what is drawn is the account's border.
        assert!(app.panel_lines(at(base, 1)).is_empty());
        assert!(!app.showing_thread(), "the run took the empty card");
        assert_bare_panel(&render_at(
            &app,
            MARK_ROOM_WIDTH,
            MARK_ROOM_HEIGHT,
            at(base, 1),
        ));

        // With lines in it, the account has the rows and the mark none of them.
        let account = app.account_mut().expect("a pact has started");
        account.open_section("crates/engine", base);
        for line in 0..MANY {
            account.record(&numbered(line), at(base, line as u64 + 1));
        }
        let buffer = render_at(&app, MARK_ROOM_WIDTH, MARK_ROOM_HEIGHT, at(base, 99));
        assert!(!panel_rows(&buffer)[0].is_empty());
        assert_no_mark(&buffer);

        // Scrolled back off the newest line, where the panel is looking at old
        // rows rather than at none.
        app.toggle_focus();
        app.select_first();
        let scrolled = render_at(&app, MARK_ROOM_WIDTH, MARK_ROOM_HEIGHT, at(base, 99));
        assert!(app.panel_lines_below() > 0);
        assert!(!panel_rows(&scrolled)[0].is_empty());
        assert_no_mark(&scrolled);
        app.toggle_focus();

        // And a second pact starts a fresh account with no lines in it, which
        // is the first case again and still not a screen for the mark.
        app.start_account(at(base, 100));
        assert!(app.panel_lines(at(base, 101)).is_empty());
        assert_bare_panel(&render_at(
            &app,
            MARK_ROOM_WIDTH,
            MARK_ROOM_HEIGHT,
            at(base, 101),
        ));
    }

    #[test]
    fn every_line_of_the_account_gets_one_row_under_the_directory_it_happened_in() {
        let base = Instant::now();
        let mut app = pacting_app(base, WIDTH, FIXTURE_HEIGHT);
        let account = app.account_mut().expect("a pact has started");
        account.open_section("crates/engine", base);
        account.record(&Activity::Thinking, at(base, 2));
        account.record(
            &Activity::Tool {
                name: "Read".to_owned(),
                detail: Some("src/lib.rs".to_owned()),
            },
            at(base, 9),
        );
        account.close_section(
            &Outcome::Wrote {
                document: "crates/engine/WARLOCK.md".into(),
                bytes: 2_341,
            },
            at(base, 30),
        );
        account.open_section("crates/tui", at(base, 31));
        account.record(&Activity::Thinking, at(base, 33));

        let buffer = render_at(&app, WIDTH, FIXTURE_HEIGHT, at(base, 40));

        // A heading per directory, in the order the run reached them, and under
        // each one a row per thing that pass was seen doing, clock first. The
        // second section's clock starts again at zero, and its newest line is
        // still counting up towards the instant this frame was drawn at.
        let drawn: Vec<String> = panel_rows(&buffer)
            .into_iter()
            .take(app.panel_lines(at(base, 40)).len())
            .collect();
        assert_eq!(
            drawn,
            [
                "crates/engine".to_owned(),
                format!("{PANEL_INDENT}0:09 thinking"),
                format!("{PANEL_INDENT}0:30 Read src/lib.rs"),
                format!(
                    "{PANEL_INDENT}0:30 wrote crates/engine/WARLOCK.md — 2341 bytes, no cost reported"
                ),
                "crates/tui".to_owned(),
                format!("{PANEL_INDENT}0:09 thinking"),
            ],
        );
        // Which is one row per line of the account and not one more: nothing
        // wrapped, and nothing was drawn that the account does not hold.
        assert_eq!(drawn.len(), app.account().expect("a pact").line_count());
        for (index, row) in panel_rows(&buffer).iter().enumerate().skip(drawn.len()) {
            assert_eq!(row, "", "panel row {index} should be blank");
        }
    }

    #[test]
    fn the_newest_lines_clock_counts_up_between_frames_with_no_event_arriving() {
        let base = Instant::now();
        let mut app = pacting_app(base, WIDTH, FIXTURE_HEIGHT);
        let account = app.account_mut().expect("a pact has started");
        account.open_section("crates/engine", base);
        account.record(&Activity::Thinking, at(base, 1));

        // Same app, same account, nothing recorded in between: the only thing
        // that changed is what time the caller says it is.
        let early = panel_rows(&render_at(&app, WIDTH, FIXTURE_HEIGHT, at(base, 9)));
        let later = panel_rows(&render_at(&app, WIDTH, FIXTURE_HEIGHT, at(base, 65)));

        assert_eq!(early[1], format!("{PANEL_INDENT}0:09 thinking"));
        assert_eq!(later[1], format!("{PANEL_INDENT}1:05 thinking"));
        assert_eq!(early[0], later[0], "the heading should not move");
    }

    #[test]
    fn a_line_too_long_for_the_panel_is_broken_under_itself_rather_than_cut() {
        // Narrow enough that both lines run off the end: the panel gets half of
        // forty columns, less its border.
        let narrow = 40;
        let base = Instant::now();
        let mut app = pacting_app(base, narrow, FIXTURE_HEIGHT);
        let account = app.account_mut().expect("a pact has started");
        account.open_section("crates/warlock-engine", base);
        account.record(
            &Activity::Tool {
                name: "Read".to_owned(),
                detail: Some("crates/warlock-engine/src/pact.rs".to_owned()),
            },
            at(base, 1),
        );
        account.record(&Activity::Thinking, at(base, 2));

        let buffer = render_at(&app, narrow, FIXTURE_HEIGHT, at(base, 2));

        let inner = panel_area(&buffer);
        assert_eq!(inner.width, 18, "the terminal is the narrow one");
        let drawn = panel_rows(&buffer);
        // Three lines, six rows, and the whole of every one of them on screen:
        // the heading broken where it had to be, the tool call broken under its
        // own clock so the path stays in the column it started in, and the line
        // that fits left exactly as it was.
        assert_eq!(
            drawn[..6],
            [
                "crates/warlock-eng".to_owned(),
                "ine".to_owned(),
                format!("{PANEL_INDENT}0:02 Read"),
                "       crates/warl".to_owned(),
                "       ock-engine/".to_owned(),
                "       src/pact.rs".to_owned(),
            ],
        );
        assert_eq!(drawn[6], format!("{PANEL_INDENT}0:02 thinking"));
        for row in &drawn {
            assert!(
                display_width(row) <= usize::from(inner.width),
                "row {row:?} is wider than the panel"
            );
        }
        // And nothing spilled onto the tree pane's border beside it.
        let border = inner.x + inner.width;
        for y in inner.y..inner.y + inner.height {
            assert_eq!(buffer[(border, y)].symbol(), "│", "at row {y}");
        }
    }

    /// The lines of a small document, one of them longer than a narrow panel and
    /// one of them empty, measured for a `width`×`height` terminal the way the
    /// binary measures one — height *and* width, since a document is wrapped to
    /// the width the frame gives the panel.
    ///
    /// `cut` is the read that stopped at the cap, which puts one line more on
    /// screen than the file has.
    fn viewing_app(width: u16, height: u16, cut: bool) -> App {
        let mut app = App::from_tree(&fixture::tree());
        app.set_viewport_height(tree_height(Size::new(width, height)));
        app.set_panel_height(panel_height(Size::new(width, height), None, None));
        app.set_panel_width(panel_width(Size::new(width, height)));
        app.show_document(
            [
                "# The engine",
                "",
                "It walks the tree and writes what it finds.",
            ],
            cut,
        );
        app
    }

    #[test]
    fn a_document_line_too_long_for_the_panel_is_wrapped_rather_than_cut() {
        // The same narrow terminal the account's truncation is pinned at: the
        // panel gets half of forty columns, less its border.
        let narrow = 40;
        let now = Instant::now();
        let app = viewing_app(narrow, FIXTURE_HEIGHT, false);

        let buffer = render_at(&app, narrow, FIXTURE_HEIGHT, now);

        let inner = panel_area(&buffer);
        assert_eq!(inner.width, 18, "the terminal is the narrow one");
        let drawn = panel_rows(&buffer);
        // The file's own lines, from the first, flush left and unindented — and
        // the long one broken at spaces onto rows of its own, continuations
        // included, so the end of the sentence is on screen rather than behind
        // an ellipsis. This is the one thing the panel's two cards do
        // differently: the account above is cut, a document is wrapped.
        assert_eq!(
            drawn[..5],
            [
                "# The engine",
                "",
                "It walks the tree",
                "and writes what it",
                "finds.",
            ],
        );
        assert!(
            !drawn.iter().any(|row| row.contains(ELLIPSIS)),
            "a document row was cut: {drawn:?}"
        );
        for row in &drawn {
            assert!(
                display_width(row) <= usize::from(inner.width),
                "row {row:?} is wider than the panel"
            );
        }
        // Three lines, five rows: what a wrapped line costs is rows of the
        // panel, which is what the window is cut out of and what the scrollback
        // counts.
        assert_eq!(app.panel_lines(now).len(), 5);
        for (index, row) in drawn.iter().enumerate().skip(5) {
            assert_eq!(row, "", "panel row {index} should be blank");
        }
        // And nothing spilled onto the tree pane's border beside it.
        let border = inner.x + inner.width;
        for y in inner.y..inner.y + inner.height {
            assert_eq!(buffer[(border, y)].symbol(), "│", "at row {y}");
        }
    }

    #[test]
    fn a_read_the_cap_cut_short_draws_a_last_row_saying_so() {
        // Wide enough that the sentence is on screen whole, and with all the
        // room in the world for the mark: what keeps the mark off is the
        // document, exactly as an account keeps it off.
        let now = Instant::now();
        let app = viewing_app(MARK_ROOM_WIDTH, MARK_ROOM_HEIGHT, true);

        let buffer = render_at(&app, MARK_ROOM_WIDTH, MARK_ROOM_HEIGHT, now);

        let drawn = panel_rows(&buffer);
        assert_eq!(drawn[0], "# The engine");
        assert_eq!(drawn[2], "It walks the tree and writes what it finds.");
        // One row under the last of the file's own, saying the file goes on.
        assert!(drawn[3].contains("cut"), "{:?}", drawn[3]);
        assert!(drawn[3].contains("the file goes on"), "{:?}", drawn[3]);
        for (index, row) in drawn.iter().enumerate().skip(4) {
            assert_eq!(row, "", "panel row {index} should be blank");
        }
        assert_no_mark(&buffer);

        // A read that fitted draws the file and not a word more.
        let whole = panel_rows(&render_at(
            &viewing_app(MARK_ROOM_WIDTH, MARK_ROOM_HEIGHT, false),
            MARK_ROOM_WIDTH,
            MARK_ROOM_HEIGHT,
            now,
        ));
        assert_eq!(whole[..3], drawn[..3]);
        assert_eq!(whole[3], "");
    }

    /// An app whose panel holds both cards: an account of a pact that started at
    /// `base`, with one section and one thing recorded in it, and a document
    /// showing over the top of it.
    ///
    /// Both filled on purpose — the slot holds two cards, and a document placed
    /// over a run puts the account behind it rather than throwing it out — so a
    /// test can swap between them and assert what reaches the screen.
    fn two_card_app(base: Instant, width: u16, height: u16) -> App {
        let mut app = pacting_app(base, width, height);
        let account = app.account_mut().expect("a pact has started");
        account.open_section("crates/engine", base);
        account.record(&Activity::Thinking, at(base, 1));
        app.show_document(["# The engine", "", "It walks the tree."], false);
        app
    }

    #[test]
    fn the_panel_draws_the_card_that_is_showing_and_a_swap_draws_the_other() {
        let base = Instant::now();
        let mut app = two_card_app(base, WIDTH, FIXTURE_HEIGHT);

        // The document is up: the file's own lines, from its first, and not one
        // row of the account behind it.
        let up = render_at(&app, WIDTH, FIXTURE_HEIGHT, at(base, 9));
        let document = panel_rows(&up);
        assert_eq!(
            document[..3],
            [
                "# The engine".to_owned(),
                String::new(),
                "It walks the tree.".to_owned(),
            ],
        );
        assert!(
            !document.iter().any(|row| row.contains("crates/engine")),
            "the account was drawn under the document: {document:?}"
        );
        assert_no_mark(&up);

        // Two swaps — the empty conversation is a stop of its own, since it is
        // where the field is — and the same panel draws the account: its heading
        // and its clocked line, the clock still counting up to the instant the
        // frame was drawn at, and nothing left of the document on screen.
        app.swap_card();
        app.swap_card();
        let swapped = render_at(&app, WIDTH, FIXTURE_HEIGHT, at(base, 9));
        let account = panel_rows(&swapped);
        assert_eq!(
            account[..2],
            [
                "crates/engine".to_owned(),
                format!("{PANEL_INDENT}0:09 thinking"),
            ],
        );
        assert!(
            !account.iter().any(|row| row.contains("It walks the tree")),
            "the document was drawn under the account: {account:?}"
        );
        assert_no_mark(&swapped);

        // And back: one slot and two cards, so the frame that comes back is the
        // frame that was there before the swap.
        app.swap_card();
        assert_eq!(
            panel_rows(&render_at(&app, WIDTH, FIXTURE_HEIGHT, at(base, 9))),
            document
        );
    }

    /// The words the account's heading and the thread's question both carry in
    /// the test below: a directory, because a heading is one.
    ///
    /// The point of saying the very same thing on both cards is that the frames
    /// still differ. Two cards that only looked different because their contents
    /// differ would be two cards a reader could be fooled by, and the question
    /// "which card am I on?" would be answerable only by reading the rows.
    const SAME_WORDS: &str = "crates/engine";

    /// The message the thread tests ask, and the sentence that comes back.
    ///
    /// The answer is prose in the model's own words, which is the one kind of
    /// text that must never reach the account's card: it is asserted for by
    /// substring, so a row that carried any of it anywhere on the frame fails.
    const QUESTION: &str = "what does the engine do?";
    /// See [`QUESTION`].
    const ANSWER: &str = "It walks the tree and writes what it finds.";

    #[test]
    fn the_thread_and_the_account_do_not_draw_alike_with_the_same_words_on_them() {
        let base = Instant::now();
        let mut app = pacting_app(base, WIDTH, FIXTURE_HEIGHT);
        let account = app.account_mut().expect("a pact has started");
        account.open_section(SAME_WORDS, base);
        account.record(&Activity::Thinking, base);

        // The account: a heading and one thing seen happening under it.
        let showing_account = render_at(&app, WIDTH, FIXTURE_HEIGHT, at(base, 9));
        assert_eq!(
            panel_rows(&showing_account)[..2],
            [
                SAME_WORDS.to_owned(),
                format!("{PANEL_INDENT}0:09 thinking"),
            ],
        );

        // The same two things said on the thread — the question is word for
        // word the account's heading, and the same activity is recorded at the
        // same instant — and the card still does not draw like the account.
        app.start_turn(SAME_WORDS, base);
        app.record_turn(&Activity::Thinking, base);
        let showing_thread = render_at(&app, WIDTH, FIXTURE_HEIGHT, at(base, 9));
        assert_eq!(
            panel_rows(&showing_thread)[..2],
            [
                format!("{SAID_MARKER}{SAME_WORDS}"),
                format!("{PANEL_INDENT}0:09 thinking"),
            ],
        );
        assert_ne!(rows_text(&showing_thread), rows_text(&showing_account));

        // Two things say it. The reader's own words are marked, where nothing an
        // account draws ever is...
        assert!(!panel_rows(&showing_account)[0].contains(SAID_MARKER));
        // ...and the panel names the card on its top edge, which the account's
        // does not.
        assert!(
            panel_top_edge(&showing_thread).contains(THREAD_TITLE.trim()),
            "{:?}",
            panel_top_edge(&showing_thread)
        );
        assert!(
            !panel_top_edge(&showing_account).contains(THREAD_TITLE.trim()),
            "{:?}",
            panel_top_edge(&showing_account)
        );

        // The title is on the border and not on a row of the card: the thread
        // draws exactly the rows the account draws.
        assert_eq!(
            panel_rows(&showing_thread).len(),
            panel_rows(&showing_account).len()
        );
        assert!(
            !panel_rows(&showing_thread)
                .iter()
                .any(|row| row.contains(THREAD_TITLE.trim())),
            "the title took a row of the thread"
        );
    }

    #[test]
    fn the_thread_says_which_card_it_is_with_the_question_scrolled_off_the_top() {
        let base = Instant::now();
        let height = usize::from(panel_height(Size::new(WIDTH, HEIGHT), None, None));
        let mut app = pacting_app(base, WIDTH, HEIGHT);
        app.start_turn(QUESTION, base);
        for line in 0..height * 3 {
            app.record_turn(&numbered(line), at(base, line as u64 + 1));
        }

        // Following the newest line, so what is on screen is work lines: the one
        // marked row is above the window and there is no prose yet either.
        let buffer = render_at(&app, WIDTH, HEIGHT, at(base, 99));
        let drawn = panel_rows(&buffer);
        assert!(app.panel_lines_below() == 0 && drawn.len() == height);
        assert!(
            !drawn.iter().any(|row| row.contains(SAID_MARKER)),
            "the question should be off the top: {drawn:?}"
        );
        // And the edge still says which card these rows belong to.
        assert!(
            panel_top_edge(&buffer).contains(THREAD_TITLE.trim()),
            "{:?}",
            panel_top_edge(&buffer)
        );
    }

    /// The border title is the whole of what brief mode looks like: the same
    /// card, the same rows, the same width, one word more on the edge — and the
    /// other cards go on naming nothing at all.
    #[test]
    fn the_thread_title_says_which_register_the_conversation_is_in() {
        let base = Instant::now();
        let mut app = pacting_app(base, WIDTH, FIXTURE_HEIGHT);
        app.start_turn(QUESTION, base);
        app.answer_turn(ANSWER, at(base, 1));

        // Chat is where a conversation starts, and the edge says only the card.
        assert_eq!(app.mode(), Mode::Chat);
        let chat = render_at(&app, WIDTH, FIXTURE_HEIGHT, at(base, 2));
        let chat_edge = panel_top_edge(&chat);
        assert!(chat_edge.contains(THREAD_TITLE.trim()), "{chat_edge:?}");
        assert!(!chat_edge.contains("brief"), "{chat_edge:?}");

        // The mode changed, so the same card at the same width says so.
        assert!(app.set_mode(Mode::Brief), "chat to brief is a change");
        let brief = render_at(&app, WIDTH, FIXTURE_HEIGHT, at(base, 2));
        let brief_edge = panel_top_edge(&brief);
        assert!(
            brief_edge.contains(BRIEF_THREAD_TITLE.trim()),
            "{brief_edge:?}"
        );

        // And says so on the border and nowhere else: not on a row of the card,
        // which has exactly the rows it had, and nowhere on the frame — no
        // artifact, no path, no "nothing written yet".
        assert_eq!(panel_rows(&chat), panel_rows(&brief));
        assert!(
            !panel_rows(&brief).iter().any(|row| row.contains("brief")),
            "the mode took a row of the thread: {:?}",
            panel_rows(&brief)
        );
        assert_eq!(
            rows_text(&brief)
                .iter()
                .filter(|row| row.contains("brief"))
                .count(),
            1,
            "the mode is said somewhere other than the border: {:?}",
            rows_text(&brief)
        );

        // A mode is a fact about the conversation, so the card the reader
        // swapped to still names nothing on its edge.
        assert!(app.showing_thread(), "the mode moved the card showing");
        app.swap_card();
        let account = panel_top_edge(&render_at(&app, WIDTH, FIXTURE_HEIGHT, at(base, 2)));
        assert!(!account.contains(THREAD_TITLE.trim()), "{account:?}");
        assert!(!account.contains("brief"), "{account:?}");
        assert_eq!(app.mode(), Mode::Brief, "the swap changed the register");

        // Setting the mode it is already in changes nothing, and is said to
        // change nothing: that is what a re-sent instruction is told by.
        assert!(!app.set_mode(Mode::Brief), "brief to brief is no change");
        assert!(app.set_mode(Mode::Chat), "brief to chat is a change");
    }

    #[test]
    fn no_model_prose_is_ever_drawn_on_the_account_card() {
        let base = Instant::now();
        let mut app = pacting_app(base, WIDTH, FIXTURE_HEIGHT);
        let account = app.account_mut().expect("a pact has started");
        account.open_section(SAME_WORDS, base);
        account.record(&Activity::Thinking, at(base, 1));

        // A turn asked and answered while the run is still going, so both cards
        // are being written and the answer is on one of them.
        app.start_turn(QUESTION, at(base, 2));
        app.record_turn(&Activity::Thinking, at(base, 3));
        app.answer_turn(ANSWER, at(base, 4));

        // The thread came to the front when the question was asked: the answer
        // is there, whole, in the model's own words.
        assert!(app.showing_thread());
        let showing_thread = render_at(&app, WIDTH, FIXTURE_HEIGHT, at(base, 9));
        assert!(
            panel_rows(&showing_thread)
                .iter()
                .any(|row| row.contains(ANSWER)),
            "{:?}",
            panel_rows(&showing_thread)
        );

        // Swap round to the account — past the document card, which nothing has
        // filled — and not a word of the model's prose is anywhere on the frame.
        app.swap_card();
        assert!(!app.showing_thread());
        let showing_account = render_at(&app, WIDTH, FIXTURE_HEIGHT, at(base, 9));
        for row in rows_text(&showing_account) {
            assert!(!row.contains(ANSWER), "{row:?}");
            assert!(!row.contains("walks the tree"), "{row:?}");
            assert!(!row.contains(QUESTION), "{row:?}");
            assert!(!row.contains(SAID_MARKER), "{row:?}");
        }
        // What is on the account is what the run put there, and only that.
        assert_eq!(
            panel_rows(&showing_account)[..2],
            [
                SAME_WORDS.to_owned(),
                format!("{PANEL_INDENT}0:09 thinking"),
            ],
        );

        // And the run going on afterwards puts nothing of the conversation on
        // it either: a second directory, its own lines, no prose.
        let account = app.account_mut().expect("a pact has started");
        account.open_section("crates/tui", at(base, 10));
        account.record(&Activity::Writing { bytes: 0 }, at(base, 11));
        for row in rows_text(&render_at(&app, WIDTH, FIXTURE_HEIGHT, at(base, 20))) {
            assert!(!row.contains("walks the tree"), "{row:?}");
        }
    }

    #[test]
    fn the_answer_is_drawn_as_plain_text_wrapped_to_the_panels_width() {
        // The same narrow terminal the document's wrapping is pinned at: the
        // panel gets half of forty columns, less its border.
        let narrow = 40;
        let base = Instant::now();
        let mut app = pacting_app(base, narrow, FIXTURE_HEIGHT);
        app.start_turn(QUESTION, base);
        app.answer_turn(ANSWER, at(base, 1));

        let buffer = render_at(&app, narrow, FIXTURE_HEIGHT, at(base, 2));

        let inner = panel_area(&buffer);
        assert_eq!(inner.width, 18, "the terminal is the narrow one");
        let drawn = panel_rows(&buffer);
        // The question broken under its own marker, the placeholder for a turn
        // that heard nothing, and then the answer broken at spaces onto rows of
        // its own: the whole of what was asked and the whole of what came back,
        // with nothing off the right-hand edge.
        assert_eq!(
            drawn[..6],
            [
                format!("{SAID_MARKER}what does the"),
                "  engine do?".to_owned(),
                format!("{PANEL_INDENT}0:01 waiting"),
                "It walks the tree".to_owned(),
                "and writes what it".to_owned(),
                "finds.".to_owned(),
            ],
        );
        for row in &drawn {
            assert!(
                display_width(row) <= usize::from(inner.width),
                "row {row:?} is wider than the panel"
            );
        }

        // The answer's rows are plain: no colour, no modifier, nothing added to
        // the left of them — where the question above them is bold, which is
        // what a heading gets here and what no prose does.
        for index in 3..6 {
            for x in inner.x..inner.x + inner.width {
                let cell = &buffer[(x, inner.y + index)];
                assert_eq!(cell.fg, Color::Reset, "at ({x}, {index})");
                assert_eq!(cell.modifier, Modifier::empty(), "at ({x}, {index})");
            }
        }
        assert!(
            buffer[(inner.x, inner.y)].modifier.contains(Modifier::BOLD),
            "the question should be bold"
        );
    }

    /// The width the single-row tests below draw into: room for a marker, a
    /// clock and a few words, and no more than a failure message can be read
    /// in.
    const ROW_WIDTH: u16 = 24;

    /// One line of a card drawn on its own, as what a reader would see: the
    /// text that lands in the buffer, and whether any of it is bold.
    ///
    /// Drawn rather than read off the value, because what tells warlock's own
    /// voice from the reader's and from a pass at work is what reaches the
    /// screen.
    fn row_drawn(line: &Entry) -> (String, bool) {
        let area = Rect::new(0, 0, ROW_WIDTH, 1);
        let mut buffer = Buffer::empty(area);
        Paragraph::new(panel_row(line, ROW_WIDTH)).render(area, &mut buffer);
        let bold = (0..ROW_WIDTH).any(|x| buffer[(x, 0)].modifier.contains(Modifier::BOLD));

        (text_in(&buffer, area, 0), bold)
    }

    #[test]
    fn a_note_is_drawn_as_neither_a_question_nor_a_work_line() {
        const WORDS: &str = "no such command";

        let note = row_drawn(&Entry::Note {
            text: WORDS.to_owned(),
        });
        let said = row_drawn(&Entry::Said {
            text: WORDS.to_owned(),
        });
        let clocked = row_drawn(&Entry::Clocked {
            clock: "0:09".to_owned(),
            text: WORDS.to_owned(),
        });

        // Warlock's own marker and then the words, with nothing else on the
        // row: no clock, because a note is not work that took time, and nothing
        // bold, because it heads nothing.
        assert_eq!(note.0, format!("{NOTE_MARKER}{WORDS}"));
        assert!(!note.0.contains(':'), "{:?} carries a clock", note.0);
        assert!(!note.1, "a note is not a heading");

        // And the same words typed by the reader or done by a pass are three
        // different rows on the screen rather than one row drawn three times.
        assert_ne!(note, said, "warlock's voice reads as the reader's");
        assert_ne!(note, clocked, "warlock's voice reads as a pass at work");
        assert_eq!(said.0, format!("{SAID_MARKER}{WORDS}"));
        assert!(said.1, "a question is the heading of its turn");
        assert_eq!(clocked.0, format!("{PANEL_INDENT}0:09 {WORDS}"));
    }

    #[test]
    fn the_mark_is_drawn_while_the_card_on_screen_is_unfilled_thread_or_not() {
        let base = Instant::now();
        let size = Size::new(MARK_ROOM_WIDTH, MARK_ROOM_HEIGHT);
        let mut app = App::from_tree(&fixture::tree());
        app.set_viewport_height(tree_height(size));
        app.set_panel_height(panel_height(size, None, None));
        app.set_panel_width(panel_width(size));

        // Nothing has happened at all: no pact, no question, no read. The panel
        // is warlock's mark on the conversation's own border — the card a
        // session opens on, named on its edge from the first frame, with the
        // field under it waiting for the first thing anybody types.
        let empty = render_at(&app, MARK_ROOM_WIDTH, MARK_ROOM_HEIGHT, base);
        assert_mark_drawn(&empty);
        assert!(
            panel_top_edge(&empty).contains(THREAD_TITLE.trim()),
            "{:?}",
            panel_top_edge(&empty)
        );

        // A question fills the thread and brings it to the front: the mark is
        // gone and the edge says which card took its place.
        app.start_turn(QUESTION, base);
        let asked = render_at(&app, MARK_ROOM_WIDTH, MARK_ROOM_HEIGHT, at(base, 1));
        assert_no_mark(&asked);
        assert_eq!(
            panel_rows(&asked)[..2],
            [
                format!("{SAID_MARKER}{QUESTION}"),
                format!("{PANEL_INDENT}0:01 waiting"),
            ],
        );
        assert!(panel_top_edge(&asked).contains(THREAD_TITLE.trim()));

        // And there is no press that brings the mark back over a conversation:
        // the account no pact has filled is not somewhere the swap key stops
        // while there is something to read, so the panel stays where it is and
        // the footer says why.
        app.swap_card();
        let swapped = render_at(&app, MARK_ROOM_WIDTH, MARK_ROOM_HEIGHT, at(base, 1));
        assert!(app.showing_thread());
        assert_no_mark(&swapped);
        assert!(panel_top_edge(&swapped).contains(THREAD_TITLE.trim()));
        assert!(app.message().is_some(), "the refusal says what would help");

        // And a pact starting under the thread does not move the panel or the
        // mark: the account it fills is the card behind.
        app.start_account(at(base, 2));
        let running = render_at(&app, MARK_ROOM_WIDTH, MARK_ROOM_HEIGHT, at(base, 3));
        assert!(app.showing_thread());
        assert_no_mark(&running);

        // Now that a pact has filled it the account is worth a press again, and
        // what it draws is the run rather than the mark.
        app.swap_card();
        let account = render_at(&app, MARK_ROOM_WIDTH, MARK_ROOM_HEIGHT, at(base, 3));
        assert!(!app.showing_thread());
        assert_no_mark(&account);
    }

    #[test]
    fn the_scrollback_and_the_run_header_reach_the_thread_as_they_reach_the_account() {
        let base = Instant::now();
        let height = usize::from(panel_height(Size::new(WIDTH, HEIGHT), None, None));
        let mut app = pacting_app(base, WIDTH, HEIGHT);
        app.start_turn(QUESTION, base);
        for line in 0..height * 3 {
            app.record_turn(&numbered(line), at(base, line as u64 + 1));
        }

        // Following the newest line: nothing below the view, so the edge says
        // nothing about scrollback and names the card and nothing else.
        let live = render_at(&app, WIDTH, HEIGHT, at(base, 99));
        assert_eq!(app.panel_lines_below(), 0);
        assert!(!panel_bottom_edge(&live).contains(SCROLLBACK_ARROW));

        // Scrolled back by the ordinary movement keys, the indicator counts the
        // thread's rows on the bottom edge while the top edge goes on naming it.
        app.toggle_focus();
        app.select_first();
        let scrolled = render_at(&app, WIDTH, HEIGHT, at(base, 99));
        let below = app.panel_lines_below();
        assert!(below > 0);
        let edge = panel_bottom_edge(&scrolled);
        assert!(
            edge.contains(&format!("{SCROLLBACK_ARROW} {below} more ({LIVE_KEY})")),
            "{edge:?}"
        );
        assert!(panel_top_edge(&scrolled).contains(THREAD_TITLE.trim()));
        assert_eq!(panel_rows(&scrolled).len(), height);
        app.select_last();
        app.toggle_focus();

        // A run starting behind the thread puts its header on the panel's top
        // row, as it does behind any card, and the thread keeps the rest.
        app.set_run_in_flight(Run::Pact, RUNNING_ON, 1, 2);
        measure_panel(&mut app, WIDTH, HEIGHT);
        let running = render_at(&app, WIDTH, HEIGHT, at(base, 99));
        let rows = panel_rows(&running);
        assert!(
            rows[0].starts_with(&format!("{PACTING_RUN} {RUNNING_LABEL} (1/2)")),
            "{:?}",
            rows[0]
        );
        // The header takes the rows it takes off the panel, exactly as it does
        // behind the account: the inside of the border is the height it always
        // was, and the thread is drawn that much shorter under it.
        let taken = usize::from(RUN_HEADER_HEIGHT);
        assert_eq!(rows.len(), height);
        assert_eq!(
            rows.len() - taken,
            usize::from(panel_height(
                Size::new(WIDTH, HEIGHT),
                None,
                app.run_header().as_ref()
            )),
        );
        // The rows under it are the thread's own, past the gap, and the edge
        // still says so.
        assert_eq!(rows[1].trim(), "", "{:?}", rows[1]);
        assert!(rows[taken].contains("Read line"), "{:?}", rows[taken]);
        assert!(panel_top_edge(&running).contains(THREAD_TITLE.trim()));
    }

    #[test]
    fn the_scrollback_indicator_reports_the_card_that_is_showing() {
        let base = Instant::now();
        let height = usize::from(panel_height(Size::new(WIDTH, HEIGHT), None, None));

        // An account longer than the panel, parked at its first line by the
        // ordinary movement keys, with a document shorter than the panel over
        // the top of it.
        let mut app = pacting_app(base, WIDTH, HEIGHT);
        let account = app.account_mut().expect("a pact has started");
        account.open_section("crates/engine", base);
        for line in 0..height * 3 {
            account.record(&numbered(line), at(base, line as u64 + 1));
        }
        app.toggle_focus();
        app.select_first();
        let parked = app.panel_lines_below();
        assert!(parked > 0, "the account should be scrolled back");
        app.show_document(["# The engine", "It walks the tree."], false);

        // The document is showing and the whole of it is on screen, so the edge
        // says nothing — whatever is under the account behind it.
        let showing_document = render_at(&app, WIDTH, HEIGHT, at(base, 99));
        assert_eq!(app.panel_lines_below(), 0);
        let edge = panel_bottom_edge(&showing_document);
        assert!(!edge.contains(SCROLLBACK_ARROW), "{edge:?}");
        assert!(!edge.contains("more"), "{edge:?}");

        // Swap round to the account, and the indicator comes back counting its
        // lines: the number on the edge is the showing card's and follows it
        // across.
        app.swap_card();
        app.swap_card();
        let showing_account = render_at(&app, WIDTH, HEIGHT, at(base, 99));
        assert_eq!(app.panel_lines_below(), parked);
        let edge = panel_bottom_edge(&showing_account);
        assert!(
            edge.contains(&format!("{SCROLLBACK_ARROW} {parked} more ({LIVE_KEY})")),
            "{edge:?}"
        );

        // The other way round, on a fresh app: a document longer than the panel
        // at its first line, over an account that is following its newest one.
        let mut app = pacting_app(base, WIDTH, HEIGHT);
        let account = app.account_mut().expect("a pact has started");
        account.open_section("crates/engine", base);
        for line in 0..height * 3 {
            account.record(&numbered(line), at(base, line as u64 + 1));
        }
        app.show_document((0..height * 3).map(|line| format!("line {line}")), false);

        let below = app.panel_lines_below();
        assert_eq!(below, height * 3 - height);
        let edge = panel_bottom_edge(&render_at(&app, WIDTH, HEIGHT, at(base, 99)));
        assert!(
            edge.contains(&format!("{SCROLLBACK_ARROW} {below} more ({LIVE_KEY})")),
            "{edge:?}"
        );

        // And swapping to an account that is following takes it away again, even
        // though the card left behind has most of itself below the window.
        app.swap_card();
        assert_eq!(app.panel_lines_below(), 0);
        let edge = panel_bottom_edge(&render_at(&app, WIDTH, HEIGHT, at(base, 99)));
        assert!(!edge.contains(SCROLLBACK_ARROW), "{edge:?}");
        assert!(!edge.contains("more"), "{edge:?}");
    }

    #[test]
    fn truncation_counts_columns_rather_than_bytes_or_characters() {
        // A plain path, one with a multi-byte character in it, and one whose
        // characters are two columns wide apiece: cut on a character boundary
        // in every case, and never wider than it was asked for.
        for text in [
            "crates/warlock-engine/src/pact.rs",
            "crates/naïve/données/résumé.rs",
            "crates/日本語/モジュール.rs",
        ] {
            for width in 0..=display_width(text) + 2 {
                let cut = truncated(text, width);

                assert!(
                    display_width(&cut) <= width,
                    "{cut:?} is wider than {width} columns"
                );
                if display_width(text) <= width {
                    assert_eq!(cut, text, "at {width} columns");
                } else if width > 0 {
                    assert!(cut.ends_with(ELLIPSIS), "{cut:?} at {width} columns");
                    assert!(text.starts_with(cut.trim_end_matches(ELLIPSIS)));
                }
            }
        }
    }

    #[test]
    fn a_scrolled_back_panel_says_what_is_below_it_and_the_key_back_to_live() {
        let base = Instant::now();
        let mut app = pacting_app(base, WIDTH, HEIGHT);
        let height = usize::from(panel_height(Size::new(WIDTH, HEIGHT), None, None));
        let account = app.account_mut().expect("a pact has started");
        account.open_section("crates/engine", base);
        for line in 0..height * 3 {
            account.record(&numbered(line), at(base, line as u64 + 1));
        }

        // Following the newest line: there is nothing below the view, so the
        // edge says nothing.
        let live = render_at(&app, WIDTH, HEIGHT, at(base, 99));
        assert_eq!(app.panel_lines_below(), 0);
        assert!(
            !panel_bottom_edge(&live).contains(SCROLLBACK_ARROW),
            "{:?}",
            panel_bottom_edge(&live)
        );

        // Scrolled back — with the panel focused, by the ordinary movement keys
        // — it says how far from the end it is and which key returns.
        app.toggle_focus();
        app.select_first();
        let buffer = render_at(&app, WIDTH, HEIGHT, at(base, 99));

        let below = app.panel_lines_below();
        assert_eq!(below, height * 3 + 1 - height);
        let edge = panel_bottom_edge(&buffer);
        assert!(
            edge.contains(&format!("{SCROLLBACK_ARROW} {below} more ({LIVE_KEY})")),
            "{edge:?}"
        );
        // On the border, not on a row of the account: the panel is still
        // drawing a full windowful of lines.
        assert_eq!(panel_rows(&buffer).len(), height);
        assert!(
            panel_rows(&buffer)
                .iter()
                .all(|row| !row.contains(SCROLLBACK_ARROW)),
            "the indicator took a row of the account"
        );

        // And the end-of-list key puts it back on live, which takes the
        // indicator away again: the frame is what it was before anyone
        // scrolled.
        app.select_last();
        let back = render_at(&app, WIDTH, HEIGHT, at(base, 99));
        assert_eq!(app.panel_lines_below(), 0);
        assert_eq!(rows_text(&back), rows_text(&live));
    }

    #[test]
    fn the_panel_width_the_app_is_told_is_the_width_the_frame_gives_the_panel() {
        // The wide terminal, the narrow one the truncation tests are pinned at,
        // and the widths either side of the point the tree gives up its floor.
        for width in [MARK_ROOM_WIDTH, WIDTH, 80, 41, 40, 20, 4, 0] {
            let measured = panel_width(Size::new(width, FIXTURE_HEIGHT));
            let app = viewing_app(width, FIXTURE_HEIGHT, false);

            let buffer = render_at(&app, width, FIXTURE_HEIGHT, Instant::now());

            // The same columns the frame is cut by, so a row wrapped to this
            // width is a row that fits between the panel's borders.
            assert_eq!(
                measured,
                panel_area(&buffer).width,
                "measured {width} columns wide"
            );
            for row in panel_rows(&buffer) {
                assert!(
                    display_width(&row) <= usize::from(measured),
                    "row {row:?} is wider than the panel at {width} columns"
                );
            }
        }
    }

    #[test]
    fn the_panel_height_the_app_is_told_is_the_height_the_frame_gives_the_panel() {
        // A terminal with room for a single line of account and three more on
        // the way up, and the two the other tests draw at.
        let chrome = FOOTER_HEIGHT + 2 * BORDER_THICKNESS;
        for height in [chrome + 1, chrome + 2, HEIGHT, FIXTURE_HEIGHT, 24] {
            let measured = panel_height(Size::new(WIDTH, height), None, None);
            let base = Instant::now();
            let mut app = pacting_app(base, WIDTH, height);
            let account = app.account_mut().expect("a pact has started");
            account.open_section("crates/engine", base);
            for line in 0..usize::from(measured) * 2 {
                account.record(&numbered(line), at(base, line as u64 + 1));
            }

            let buffer = render_at(&app, WIDTH, height, at(base, 99));

            assert_eq!(measured, height - chrome);
            let drawn = panel_rows(&buffer);
            assert_eq!(drawn.len(), usize::from(measured), "in {height} rows");
            assert!(
                drawn.iter().all(|row| !row.is_empty()),
                "a longer account left blank rows in {height}: {drawn:?}"
            );
        }

        // And a terminal with no room for a line of account is measured at none
        // rather than underflowing.
        for height in 0..=chrome {
            assert_eq!(
                panel_height(Size::new(WIDTH, height), None, None),
                0,
                "in {height} rows"
            );
        }
    }

    /// The directory the run-header tests report: a node of the fixture's tree,
    /// so the header names something the reader can see on screen beside it.
    const RUNNING_ON: &str = "warlock/crates/engine";

    /// How [`RUNNING_ON`] reaches the header, spelled by the one speller the
    /// footer uses: the fixture's paths are already relative to its root, so
    /// they are handed back as they stand.
    const RUNNING_LABEL: &str = RUNNING_ON;

    /// Tell `app` how many lines of account its window has, the way the event
    /// loop's `draw_frame` does: what is inside the panel's border, less the
    /// rows the run's header is about to take.
    ///
    /// Measured off the app's own header, and *before* the frame is drawn, which
    /// is the whole point — an app told the taller number would scroll its
    /// account by a row the header owns.
    fn measure_panel(app: &mut App, width: u16, height: u16) {
        let header = app.run_header();
        app.set_panel_height(panel_height(
            Size::new(width, height),
            None,
            header.as_ref(),
        ));
    }

    /// [`pacting_app`] with a run of `total` directories in flight, `position`
    /// of the way through [`RUNNING_ON`], measured the way the binary measures
    /// one.
    fn running_app(
        base: Instant,
        width: u16,
        height: u16,
        run: Run,
        position: usize,
        total: usize,
    ) -> App {
        let mut app = pacting_app(base, width, height);
        app.set_run_in_flight(run, RUNNING_ON, position, total);
        measure_panel(&mut app, width, height);
        app
    }

    /// Put `lines` numbered activities in `app`'s account, one a second from
    /// `base`, under a section for the directory the run is working.
    fn fill_account(app: &mut App, base: Instant, lines: usize) {
        let account = app.account_mut().expect("a pact has started");
        account.open_section(RUNNING_LABEL, base);
        for line in 0..lines {
            account.record(&numbered(line), at(base, line as u64 + 1));
        }
    }

    /// The run header's row of a frame drawn at `now`: the top row inside the
    /// panel's border, which is the header's while a run is in flight.
    fn run_header_row(app: &App, width: u16, height: u16, now: Instant) -> String {
        panel_rows(&render_at(app, width, height, now))
            .first()
            .expect("a panel with rows")
            .clone()
    }

    #[test]
    fn the_run_header_takes_the_panels_top_row_and_the_account_keeps_the_rest() {
        let base = Instant::now();
        let size = Size::new(WIDTH, HEIGHT);
        let window = panel_height(size, None, None);

        for (run, word) in [(Run::Pact, PACTING_RUN), (Run::Refresh, REFRESHING_RUN)] {
            // The same account either way, longer than the window and following
            // its newest line: what differs between the two frames below is a
            // run in flight and nothing else.
            let mut app = pacting_app(base, WIDTH, HEIGHT);
            fill_account(&mut app, base, usize::from(window) * 2);

            // With no run in flight there is no header and the window is the
            // whole inside of the border.
            let without = panel_rows(&render_at(&app, WIDTH, HEIGHT, at(base, 99)));
            assert_eq!(run_header_height(size, None, None), 0);
            assert_eq!(without.len(), usize::from(window));

            app.set_run_in_flight(run, RUNNING_ON, 2, 5);
            let header = app.run_header().expect("a run in flight has a header");
            assert_eq!(
                run_header_height(size, None, Some(&header)),
                RUN_HEADER_HEIGHT
            );
            assert_eq!(
                panel_height(size, None, Some(&header)) + RUN_HEADER_HEIGHT,
                window,
                "the account should lose exactly the rows the header takes"
            );
            measure_panel(&mut app, WIDTH, HEIGHT);

            let with = panel_rows(&render_at(&app, WIDTH, HEIGHT, at(base, 99)));

            // The top row inside the border is the run: which run it is, the
            // directory it is working spelled against the tree on screen, where
            // it is in the run, and a bar in what is left over.
            assert!(
                with[0].starts_with(&format!("{word} {RUNNING_LABEL} (2/5)")),
                "{:?}",
                with[0]
            );
            assert!(with[0].contains(BAR_FILLED), "{:?}", with[0]);
            assert!(with[0].contains(BAR_EMPTY), "{:?}", with[0]);

            // Then the blank row that keeps the run's line off the card's own
            // first one.
            assert_eq!(with[1].trim(), "", "{:?}", with[1]);

            // And the account under those is what it was, that much shorter: it
            // is still following its newest line, so the rows the header took
            // are the ones that were at the top and every other line is where it
            // was.
            let taken = usize::from(RUN_HEADER_HEIGHT);
            assert_eq!(with.len(), without.len());
            assert_eq!(
                with.len() - taken,
                usize::from(panel_height(size, None, Some(&header))),
                "the window drew a different number of lines than it was measured at"
            );
            assert_eq!(with[taken..], without[taken..]);
        }
    }

    #[test]
    fn the_run_headers_bar_is_the_fraction_and_moves_only_when_the_run_does() {
        let base = Instant::now();
        let total = 5;
        let mut app = running_app(base, WIDTH, HEIGHT, Run::Pact, 0, total);
        fill_account(&mut app, base, usize::from(HEIGHT) * 2);

        // Several frames, seconds and then an hour apart, with nothing arriving
        // in between: the header is drawn from the run's own counting, so the
        // row is the same bytes every time. Nothing here reads a clock, creeps,
        // or interpolates between two fractions.
        let first = run_header_row(&app, WIDTH, HEIGHT, at(base, 1));
        for seconds in [2, 9, 65, 3_600] {
            assert_eq!(
                run_header_row(&app, WIDTH, HEIGHT, at(base, seconds)),
                first,
                "the header moved between frames drawn {seconds}s apart"
            );
        }

        // Empty at the start of the run, and the bar is worth drawing at this
        // width.
        let columns = first.matches(BAR_FILLED).count() + first.matches(BAR_EMPTY).count();
        assert!(columns >= BAR_MIN_WIDTH, "{first:?}");
        assert_eq!(first.matches(BAR_FILLED).count(), 0, "{first:?}");

        // Filling to exactly `position/total` of it as the run reports its way
        // through, never less than it was...
        let mut filled = 0;
        for position in 1..=total {
            app.set_run_in_flight(Run::Pact, RUNNING_ON, position, total);
            let row = run_header_row(&app, WIDTH, HEIGHT, at(base, 99));
            let drawn = row.matches(BAR_FILLED).count();

            assert!(drawn >= filled, "the bar fell back at {position}/{total}");
            assert_eq!(drawn, position * columns / total, "at {position}/{total}");
            assert_eq!(row.matches(BAR_EMPTY).count(), columns - drawn);
            assert!(row.contains(&format!("({position}/{total})")), "{row:?}");
            filled = drawn;
        }

        // ...and full at the end of it, and only there.
        assert_eq!(filled, columns);
    }

    #[test]
    fn the_run_header_stays_where_it_is_while_the_account_is_scrolled_back() {
        let base = Instant::now();
        let mut app = running_app(base, WIDTH, HEIGHT, Run::Pact, 2, 5);
        let window = usize::from(panel_height(
            Size::new(WIDTH, HEIGHT),
            None,
            app.run_header().as_ref(),
        ));
        fill_account(&mut app, base, window * 3);

        let live = panel_rows(&render_at(&app, WIDTH, HEIGHT, at(base, 99)));
        assert_eq!(app.panel_lines_below(), 0, "the account is following");

        // Scrolled back through the account by the ordinary movement keys.
        app.toggle_focus();
        app.select_first();
        let scrolled = panel_rows(&render_at(&app, WIDTH, HEIGHT, at(base, 99)));

        // The window is the one the header left, so what is below it counts the
        // lines the reader has yet to come back down through — and not the row
        // the header is sitting on.
        assert_eq!(app.panel_lines_below(), window * 3 + 1 - window);
        // The header is not a line of the account: it is byte for byte where it
        // was, and everything that moved is under it.
        assert_eq!(scrolled.len(), live.len());
        assert_eq!(scrolled[0], live[0]);
        assert_ne!(scrolled[1..], live[1..]);
        assert!(
            scrolled[1..].iter().all(|row| row != &live[0]),
            "the header was drawn into the account: {scrolled:?}"
        );
    }

    #[test]
    fn a_point_on_the_run_header_is_not_a_line_of_the_account() {
        let mut app = App::from_tree(&fixture::tree());
        app.set_run_in_flight(Run::Pact, RUNNING_ON, 2, 5);
        let header = app.run_header().expect("a run in flight has a header");
        let size = Size::new(WIDTH, FIXTURE_HEIGHT);
        let panel = pane_inner(areas(Rect::from(size), None).panel);
        let window = panel_height(size, None, Some(&header));

        // The top row inside the border is the header's, and answers as itself
        // rather than as the account's first line.
        for x in panel.x..panel.x + panel.width {
            assert_eq!(
                hit_test(x, panel.y, size, None, Some(&header)),
                Hit::PanelHeader,
                "at {x},{}",
                panel.y
            );
        }

        // And every row under it is the account line the reader is looking at:
        // the offsets are counted from beneath the header, so they point at the
        // line drawn on that row and stay inside the window the app was told
        // about.
        for y in panel.y + RUN_HEADER_HEIGHT..panel.y + panel.height {
            let hit = hit_test(panel.x, y, size, None, Some(&header));

            assert_eq!(
                hit,
                Hit::PanelLine {
                    offset: y - panel.y - RUN_HEADER_HEIGHT
                }
            );
            assert!(matches!(hit, Hit::PanelLine { offset } if offset < window));
        }

        // The row the header took would have been the account's, and the hit
        // test knows it: the same point answers differently on a frame with no
        // run in flight.
        assert_eq!(
            hit_test(panel.x, panel.y, size, None, None),
            Hit::PanelLine { offset: 0 }
        );
    }

    #[test]
    fn a_panel_with_no_room_for_a_header_and_an_account_both_draws_the_account() {
        let base = Instant::now();
        let chrome = FOOTER_HEIGHT + 2 * BORDER_THICKNESS;
        let mut app = App::from_tree(&fixture::tree());
        app.set_run_in_flight(Run::Pact, RUNNING_ON, 2, 5);
        let header = app.run_header().expect("a run in flight has a header");

        // A panel with no more rows inside its border than the header itself
        // takes: a header there would be a run reporting its progress into a
        // window with no room to report anything in, so there is none and the
        // account keeps what it had.
        for height in 0..=chrome + RUN_HEADER_HEIGHT {
            let size = Size::new(WIDTH, height);

            assert_eq!(
                run_header_height(size, None, Some(&header)),
                0,
                "in {height} rows"
            );
            assert_eq!(
                panel_height(size, None, Some(&header)),
                panel_height(size, None, None),
                "in {height} rows"
            );
        }

        // And the rows such a panel does have are the account's rather than a
        // header drawn over nothing.
        let height = chrome + RUN_HEADER_HEIGHT;
        let mut app = running_app(base, WIDTH, height, Run::Pact, 2, 5);
        fill_account(&mut app, base, 4);
        let drawn = panel_rows(&render_at(&app, WIDTH, height, at(base, 99)));

        assert_eq!(drawn.len(), usize::from(RUN_HEADER_HEIGHT));
        for row in &drawn {
            assert!(!row.contains(PACTING_RUN), "{row:?}");
            assert!(!row.contains(BAR_EMPTY), "{row:?}");
        }
    }

    #[test]
    fn the_scrollback_counts_the_window_the_run_header_left_and_nothing_else() {
        let base = Instant::now();
        let size = Size::new(WIDTH, HEIGHT);
        // What the panel has inside its border at this one fixed terminal size,
        // before there is a header to pay for.
        let whole = usize::from(panel_height(size, None, None));
        let mut counted = Vec::new();

        for (name, in_flight) in [
            ("with no run in flight", false),
            ("under a run's header", true),
        ] {
            let mut app = pacting_app(base, WIDTH, HEIGHT);
            if in_flight {
                app.set_run_in_flight(Run::Pact, RUNNING_ON, 2, 5);
            }
            measure_panel(&mut app, WIDTH, HEIGHT);
            let header = app.run_header();
            let window = usize::from(panel_height(size, None, header.as_ref()));
            let taken = usize::from(run_header_height(size, None, header.as_ref()));

            assert_eq!(
                taken,
                usize::from(RUN_HEADER_HEIGHT) * usize::from(in_flight),
                "{name}"
            );
            assert_eq!(window + taken, whole, "{name}");

            // The same account either way: three windowfuls of it, under the
            // section heading that makes a line more.
            fill_account(&mut app, base, whole * 3);
            let lines = whole * 3 + 1;

            // Following its newest line, so there is nothing below the window
            // and the edge says nothing: the row the header sits on is not
            // something a reader can scroll back through.
            assert_eq!(app.panel_lines_below(), 0, "{name}");
            let edge = panel_bottom_edge(&render_at(&app, WIDTH, HEIGHT, at(base, 99)));
            assert!(!edge.contains(SCROLLBACK_ARROW), "{name}: {edge:?}");

            // Scrolled back to the first line: the number on the edge is the
            // account less the windowful on screen, counted against the window
            // the header left rather than against the whole of the border.
            app.toggle_focus();
            app.select_first();
            let buffer = render_at(&app, WIDTH, HEIGHT, at(base, 99));
            let below = app.panel_lines_below();

            assert_eq!(app.panel_scroll_offset(), 0, "{name}");
            assert_eq!(below, lines - window, "{name}");
            let edge = panel_bottom_edge(&buffer);
            assert!(
                edge.contains(&format!("{SCROLLBACK_ARROW} {below} more ({LIVE_KEY})")),
                "{name}: {edge:?}"
            );

            // The rows bear it out: the header's row and the window's rows are
            // the whole inside of the border, and the window drew the lines the
            // count was taken against.
            let rows = panel_rows(&buffer);
            assert_eq!(rows.len(), whole, "{name}");
            assert_eq!(rows.len() - taken, window, "{name}");

            // And the count is off by exactly nothing: a line at a time down
            // reaches the end of the account after that many presses, and the
            // indicator is gone when it gets there.
            for step in 1..=below {
                app.select_next();
                assert_eq!(app.panel_lines_below(), below - step, "{name}, {step} down");
            }
            assert!(app.panel_follows(), "{name}");
            let edge = panel_bottom_edge(&render_at(&app, WIDTH, HEIGHT, at(base, 99)));
            assert!(!edge.contains(SCROLLBACK_ARROW), "{name}: {edge:?}");

            counted.push(below);
        }

        // The whole of the difference between the two is the header's row: the
        // same account parked at the same line has exactly one more line below
        // a window that has paid for a header, and never two.
        assert_eq!(counted[1], counted[0] + usize::from(RUN_HEADER_HEIGHT));
    }

    #[test]
    fn the_account_has_the_headers_row_back_the_moment_the_run_is_over() {
        let base = Instant::now();
        let size = Size::new(WIDTH, HEIGHT);
        let whole = panel_height(size, None, None);
        let mut app = pacting_app(base, WIDTH, HEIGHT);
        fill_account(&mut app, base, usize::from(whole) * 2);

        // Before the first run there is no run to report: no header, and the
        // window is the whole inside of the border.
        assert!(app.run_header().is_none());
        let before = panel_rows(&render_at(&app, WIDTH, HEIGHT, at(base, 99)));
        assert_eq!(before.len(), usize::from(whole));
        assert!(
            before.iter().all(|row| !row.contains(PACTING_RUN)),
            "{before:?}"
        );

        // During the run the top rows inside the border are the header's — its
        // line and the gap under it — and the account keeps every row below
        // them: it is still following its newest line, so the lines the header
        // displaced are the ones that were at the top.
        app.set_run_in_flight(Run::Pact, RUNNING_ON, 2, 5);
        measure_panel(&mut app, WIDTH, HEIGHT);
        let during = panel_rows(&render_at(&app, WIDTH, HEIGHT, at(base, 99)));

        assert_eq!(during.len(), usize::from(whole));
        assert!(
            during[0].starts_with(&format!("{PACTING_RUN} {RUNNING_LABEL} (2/5)")),
            "{:?}",
            during[0]
        );
        assert_eq!(during[1].trim(), "", "{:?}", during[1]);
        assert_eq!(
            during[usize::from(RUN_HEADER_HEIGHT)..],
            before[usize::from(RUN_HEADER_HEIGHT)..]
        );

        // And when the run is over the rows come back: no header, the window is
        // the whole inside of the border again, and the frame is the frame from
        // before the run started.
        app.clear_pact_in_flight();
        measure_panel(&mut app, WIDTH, HEIGHT);
        let after = panel_rows(&render_at(&app, WIDTH, HEIGHT, at(base, 99)));

        assert!(app.run_header().is_none());
        assert_eq!(run_header_height(size, None, None), 0);
        assert_eq!(after, before);
    }

    /// The drafts the layout tests are run against: nothing typed, one row,
    /// several newlines, a run long enough to wrap, and one well past the cap.
    ///
    /// One list rather than a test per draft, because what is under test is
    /// arithmetic that has to hold whatever is in the field — and a draft that
    /// wants more rows than the cap is the case the arithmetic is easiest to get
    /// wrong at.
    fn drafts() -> Vec<Composer> {
        let many = (0..usize::from(COMPOSER_MAX_ROWS) * 3)
            .map(|line| format!("line {line}"))
            .collect::<Vec<_>>()
            .join("\n");

        vec![
            Composer::default(),
            Composer::new("a line"),
            Composer::new("one\ntwo\nthree"),
            Composer::new("x".repeat(usize::from(WIDTH) * 2)),
            Composer::new(many),
        ]
    }

    /// The rows of `buffer` the composer is drawn into, as text.
    fn composer_rows(buffer: &Buffer, composer: &Composer) -> Vec<String> {
        let area = pane_inner(
            areas(buffer.area, Some(composer))
                .composer
                .expect("a field on this frame"),
        );

        (0..area.height)
            .map(|index| text_in(buffer, area, area.y + index))
            .collect()
    }

    #[test]
    fn the_panel_the_header_and_the_composer_come_to_the_column_the_panel_had() {
        let chrome = FOOTER_HEIGHT + 2 * BORDER_THICKNESS;
        let mut running = App::from_tree(&fixture::tree());
        running.set_run_in_flight(Run::Pact, RUNNING_ON, 2, 5);
        let header = running.run_header().expect("a run in flight has a header");
        // Every height either side of the point there is room for a field at
        // all, the ones the other tests draw at, and a tall one.
        let heights = [
            0,
            1,
            chrome,
            chrome + 1,
            chrome + 2,
            chrome + 3,
            HEIGHT,
            FIXTURE_HEIGHT,
            24,
            60,
        ];
        for height in heights {
            for width in [WIDTH, 80, 40, 20, 4, 0] {
                let size = Size::new(width, height);
                // What the panel had before there was a composer to pay for.
                let before = panel_height(size, None, None);
                assert_eq!(composer_height(size, None), 0, "{width}x{height}");
                assert_eq!(run_header_height(size, None, None), 0, "{width}x{height}");

                for composer in &drafts() {
                    let field = Some(composer);
                    // With a run in flight and without: a row of the column is
                    // the account's, the header's or the field's, and never none
                    // of the three.
                    for run in [None, Some(&header)] {
                        assert_eq!(
                            panel_height(size, field, run)
                                + run_header_height(size, field, run)
                                + composer_height(size, field),
                            before,
                            "{width}x{height} does not add up with {:?} in the field",
                            composer.draft()
                        );
                    }
                    // And the width is untouched: the field takes rows off the
                    // panel and never a column, which is why a document wrapped
                    // at `panel_width` is wrapped at the width it is drawn at.
                    assert_eq!(
                        pane_inner(areas(Rect::from(size), field).panel).width,
                        panel_width(size),
                        "{width}x{height} moved the panel's edge"
                    );
                }
            }
        }
    }

    #[test]
    fn the_composer_is_one_row_empty_and_grows_a_row_at_a_time_to_its_cap() {
        let border = 2 * BORDER_THICKNESS;
        let size = Size::new(WIDTH, 60);
        assert_eq!(
            composer_height(size, Some(&Composer::default())),
            1 + border,
            "an empty field is a row to type on and its border"
        );

        // A newline at a time, up to the cap and then not past it.
        for lines in 1..usize::from(COMPOSER_MAX_ROWS) + 4 {
            let composer = Composer::new(vec!["x"; lines].join("\n"));
            let rows = u16::try_from(lines)
                .expect("a short draft")
                .min(COMPOSER_MAX_ROWS);
            assert_eq!(
                composer_height(size, Some(&composer)),
                rows + border,
                "{lines} lines"
            );
        }

        // And a wrap costs exactly what a newline does: one line of text, one
        // column wider than the field, is two rows.
        let inner = panel_width(size);
        let over = Composer::new("x".repeat(usize::from(inner) + 1));
        assert_eq!(composer_height(size, Some(&over)), 2 + border);
    }

    #[test]
    fn a_draft_past_the_cap_scrolls_inside_the_field_and_keeps_the_cursors_row() {
        let app = App::from_tree(&fixture::tree());
        let lines: Vec<String> = (0..usize::from(COMPOSER_MAX_ROWS) * 2)
            .map(|line| format!("line {line}"))
            .collect();
        let composer = Composer::new(lines.join("\n"));

        let buffer = render_composer(&app, &composer, WIDTH, 40);
        let drawn = composer_rows(&buffer, &composer);

        // The tail of the draft, so the row the cursor is on — the last one,
        // always — is on screen, and the field never grew past its cap to get it
        // there.
        assert_eq!(drawn.len(), usize::from(COMPOSER_MAX_ROWS));
        assert_eq!(drawn, lines[lines.len() - drawn.len()..]);
        assert_eq!(
            drawn.last().map(String::as_str),
            lines.last().map(String::as_str),
            "the newest line is the bottom row"
        );
    }

    #[test]
    fn the_composers_border_is_lit_only_while_the_keys_are_in_it() {
        let mut app = App::from_tree(&fixture::tree());
        let composer = Composer::new("what to ask for");
        let style = |buffer: &Buffer, pane: Rect| {
            let cell = &buffer[(pane.x, pane.y)];
            (cell.fg, cell.modifier)
        };

        let mut seen = Vec::new();
        for expected in [Focus::Tree, Focus::Panel, Focus::Composer] {
            assert_eq!(app.focus(), expected, "the cycle moved somewhere else");
            let buffer = render_composer(&app, &composer, WIDTH, FIXTURE_HEIGHT);
            let panes = areas(buffer.area, Some(&composer));
            let field = panes.composer.expect("a field on this frame");
            seen.push((
                expected,
                style(&buffer, field),
                style(&buffer, panes.panel),
                style(&buffer, panes.tree),
            ));
            app.toggle_focus();
        }

        for (focus, field, panel, tree) in seen {
            let lit = (FOCUS_COLOUR, Modifier::BOLD);
            let borders = [
                (Focus::Composer, field),
                (Focus::Panel, panel),
                (Focus::Tree, tree),
            ];
            for (whose, (fg, modifier)) in borders {
                if whose == focus {
                    assert_eq!((fg, modifier), lit, "{focus:?} should light {whose:?}");
                } else {
                    assert_ne!(fg, FOCUS_COLOUR, "{focus:?} lit {whose:?} as well");
                    assert!(modifier.contains(Modifier::DIM));
                }
            }
        }
    }

    #[test]
    fn a_muted_field_is_drawn_dim_and_with_no_caret_while_a_turn_is_answered() {
        // One question at a time: while a turn is being answered the field keeps
        // the keyboard and hears nothing with it, so it is drawn as a field
        // nobody is pointed at. The border is where it has to show — the draft
        // is cleared by the submit, so an empty box is what the reader is
        // looking at — and the caret goes with it, because nothing is landing.
        let mut app = App::from_tree(&fixture::tree());
        app.set_focus(Focus::Composer);
        assert_eq!(app.focus(), Focus::Composer, "the field has the keyboard");

        let live = Composer::new("what does the engine do?");
        let mut muted = live.clone();
        muted.set_muted(true);

        // The border's corner and the cell the caret sits in: the column after
        // the last character of the draft, on the draft's own row.
        let drawn = |composer: &Composer| {
            let buffer = render_composer(&app, composer, WIDTH, FIXTURE_HEIGHT);
            let field = areas(buffer.area, Some(composer))
                .composer
                .expect("a field on this frame");
            let inner = pane_inner(field);
            let caret = u16::try_from(display_width(composer.draft())).expect("a short draft");
            let border = &buffer[(field.x, field.y)];
            (
                (border.fg, border.modifier),
                buffer[(inner.x + caret, inner.y)].modifier,
                composer_rows(&buffer, composer),
            )
        };

        let ((lit_fg, lit_modifier), live_caret, live_rows) = drawn(&live);
        let ((dim_fg, dim_modifier), muted_caret, muted_rows) = drawn(&muted);

        assert_eq!(
            (lit_fg, lit_modifier),
            (FOCUS_COLOUR, Modifier::BOLD),
            "a live field the keys are in is lit"
        );
        assert_ne!(
            dim_fg, FOCUS_COLOUR,
            "a muted field is lit as if it listens"
        );
        assert!(
            dim_modifier.contains(Modifier::DIM),
            "a muted field should be dim: {dim_modifier:?}"
        );

        // The caret says where the next character lands, and while the field is
        // muted nothing is landing.
        assert!(
            live_caret.contains(Modifier::REVERSED),
            "a live field draws its caret: {live_caret:?}"
        );
        assert!(
            !muted_caret.contains(Modifier::REVERSED),
            "a muted field drew a caret: {muted_caret:?}"
        );

        // And the draft itself is untouched by muting: the same characters in
        // the same rows, still there when the answer lands.
        assert_eq!(live_rows, muted_rows);
        assert_eq!(muted_rows.last().map(String::as_str), Some(live.draft()));
    }

    /// An app showing a conversation with a run going on behind it: a question,
    /// its answer, and then a pact started from the tree, which fills the
    /// account card without touching the one on screen.
    ///
    /// The two tests below are about the field under that thread, so what they
    /// need of the app is that the thread is what is showing and that a run is
    /// under way — the clocks are read at `at(base, 9)` by both.
    fn app_running_under_a_thread(base: Instant) -> App {
        let mut app = App::from_tree(&fixture::tree());
        app.set_focus(Focus::Composer);
        app.start_turn("what does the engine do?", at(base, 1));
        app.answer_turn("It walks the tree.", at(base, 2));
        app.start_account(at(base, 3));
        app.set_pact_in_flight("warlock/crates/engine", 1, 2);

        assert!(app.showing_thread(), "the run swapped the card away");
        assert!(app.composer_showable(), "the run hid the field");
        app
    }

    #[test]
    fn a_field_under_a_conversation_is_drawn_the_same_whatever_a_run_is_doing() {
        // A pact or a refresh started while the reader is looking at the
        // conversation fills the card behind it and takes nothing from the field
        // in front: it is lit, it draws its caret, and it types. Muting is one
        // question at a time and nothing else (see `field_muted`), so the only
        // difference this frame can show is the one the flag makes — and the
        // flag is the loop's, from the turn alone.
        //
        // Muted or not it is the same pane in the same place holding the same
        // draft. The other reason a field is not typed into — the card showing
        // being one the field is not drawn under — is the test after this one,
        // and it is a different rule.
        let base = Instant::now();
        let now = at(base, 9);
        let app = app_running_under_a_thread(base);

        let live = Composer::new("a question half written");
        let mut muted = live.clone();
        muted.set_muted(true);

        // The border's corner, the cell the caret would sit in, and the rows of
        // both panes — so what the run changed and what it left alone are read
        // off the one frame.
        let drawn = |composer: &Composer| {
            let buffer = render_all(
                &app,
                &Chrome::default(),
                WIDTH,
                FIXTURE_HEIGHT,
                now,
                QuitConfirm::Closed,
                &ScopePrompt::Closed,
                &ScopePrompt::Closed,
                Some(composer),
            );
            let field = areas(buffer.area, Some(composer))
                .composer
                .expect("a field on this frame");
            let inner = pane_inner(field);
            let caret = u16::try_from(display_width(composer.draft())).expect("a short draft");
            let border = &buffer[(field.x, field.y)];
            (
                (border.fg, border.modifier),
                buffer[(inner.x + caret, inner.y)].modifier,
                composer_rows(&buffer, composer),
                panel_rows(&buffer),
                field,
            )
        };

        let ((lit_fg, lit_modifier), live_caret, live_rows, live_panel, live_area) = drawn(&live);
        let ((dim_fg, dim_modifier), muted_caret, muted_rows, muted_panel, muted_area) =
            drawn(&muted);

        assert_eq!(
            (lit_fg, lit_modifier),
            (FOCUS_COLOUR, Modifier::BOLD),
            "a live field the keys are in is lit"
        );
        assert_ne!(dim_fg, FOCUS_COLOUR, "a muted field is lit");
        assert!(
            dim_modifier.contains(Modifier::DIM),
            "a muted field should be dim: {dim_modifier:?}"
        );
        assert!(
            live_caret.contains(Modifier::REVERSED),
            "a live field draws its caret: {live_caret:?}"
        );
        assert!(
            !muted_caret.contains(Modifier::REVERSED),
            "a muted field drew a caret: {muted_caret:?}"
        );

        // Nothing else about the frame moved: the field is the same pane in the
        // same place, holding the same draft, and the panel above it is drawing
        // the same conversation on the same rows.
        assert_eq!(live_area, muted_area, "muting resized the field");
        assert_eq!(live_rows, muted_rows);
        assert_eq!(muted_rows.last().map(String::as_str), Some(live.draft()));
        assert_eq!(live_panel, muted_panel, "muting moved the panel's lines");
        assert_eq!(
            composer_on_screen(&app, &live),
            Some(&live),
            "a run behind the conversation took the field off the frame"
        );
        assert_eq!(
            panel_height(Size::new(WIDTH, FIXTURE_HEIGHT), Some(&muted), None),
            panel_height(Size::new(WIDTH, FIXTURE_HEIGHT), Some(&live), None),
            "a muted field costs the panel a different number of rows"
        );
    }

    #[test]
    fn a_document_read_during_a_run_hides_the_muted_field_and_gives_the_rows_back() {
        // The two rules are separate and they compose. Muting takes the field's
        // keys; the document card takes the field itself — so `v` during a run
        // puts the file on the panel, the field goes off the frame altogether,
        // and the panel gets those rows back exactly as it does for a live one.
        // Neither loses the draft: the swap back puts it on screen again, still
        // muted, with every character in it.
        let base = Instant::now();
        let now = at(base, 9);
        let mut app = app_running_under_a_thread(base);
        let size = Size::new(WIDTH, FIXTURE_HEIGHT);

        let live = Composer::new("a question half written");
        let mut muted = live.clone();
        muted.set_muted(true);
        assert_eq!(
            composer_on_screen(&app, &live),
            Some(&live),
            "a run behind the conversation took the field off the frame"
        );

        app.show_document(["# The engine", "", "It walks the tree."], false);

        assert_eq!(
            composer_on_screen(&app, &muted),
            None,
            "a document card left a muted field on screen"
        );
        assert_eq!(
            panel_height(size, None, None),
            panel_height(size, composer_on_screen(&app, &muted), None),
            "the panel should have the rows the muted field was taking"
        );
        assert_eq!(
            render_all(
                &app,
                &Chrome::default(),
                WIDTH,
                FIXTURE_HEIGHT,
                now,
                QuitConfirm::Closed,
                &ScopePrompt::Closed,
                &ScopePrompt::Closed,
                Some(&muted),
            ),
            render_all(
                &app,
                &Chrome::default(),
                WIDTH,
                FIXTURE_HEIGHT,
                now,
                QuitConfirm::Closed,
                &ScopePrompt::Closed,
                &ScopePrompt::Closed,
                None,
            ),
            "a muted draft reached a frame the document card has"
        );

        // Shift-Tab back to the conversation, which is the very next card, and
        // the field is there again — dim, with every character still in it.
        app.swap_card();
        assert!(app.showing_thread(), "the swap landed somewhere else");
        assert_eq!(composer_on_screen(&app, &muted), Some(&muted));
        assert_eq!(muted.draft(), live.draft());
    }

    #[test]
    fn the_document_card_takes_the_composers_rows_back() {
        let mut app = App::from_tree(&fixture::tree());
        let composer = Composer::new("one\ntwo\nthree");
        let size = Size::new(WIDTH, FIXTURE_HEIGHT);
        assert_eq!(
            composer_on_screen(&app, &composer),
            Some(&composer),
            "with an account showing the field is on screen"
        );

        app.show_document(["# The engine", "", "It walks the tree."], false);
        let field = composer_on_screen(&app, &composer);

        assert_eq!(field, None, "a document takes the whole column");
        assert_eq!(
            panel_height(size, field, None),
            panel_height(size, None, None),
            "the panel should have the rows the field was taking"
        );
        assert_eq!(composer_height(size, field), 0);

        // And nothing of the field reaches the frame: a draft handed to `draw`
        // while the card is up is drawn exactly as no draft at all.
        let with = render_composer(&app, &composer, WIDTH, FIXTURE_HEIGHT);
        let without = render(&app, WIDTH, FIXTURE_HEIGHT);
        assert_eq!(with, without);
    }

    #[test]
    fn a_point_on_the_composer_is_not_a_line_the_panel_no_longer_has() {
        let composer = Composer::new("one\ntwo");
        let size = Size::new(WIDTH, FIXTURE_HEIGHT);
        let field = Some(&composer);
        let cut = areas(Rect::from(size), field);
        let area = cut.composer.expect("a field on this frame");

        // Every point inside the field says so, and says nothing about a row of
        // an account that is drawn above it.
        let inside = pane_inner(area);
        for y in inside.y..inside.y + inside.height {
            for x in inside.x..inside.x + inside.width {
                assert_eq!(
                    hit_test(x, y, size, field, None),
                    Hit::Composer,
                    "at {x},{y}"
                );
            }
        }
        // Its border is a border, like either pane's.
        assert_eq!(hit_test(area.x, area.y, size, field, None), Hit::Border);

        // And the panel's lines are the lines the panel has: every offset the
        // hit test hands out is inside the window the app was told about.
        let panel = pane_inner(cut.panel);
        let height = panel_height(size, field, None);
        for y in panel.y..panel.y + panel.height {
            let hit = hit_test(panel.x, y, size, field, None);
            assert_eq!(
                hit,
                Hit::PanelLine {
                    offset: y - panel.y
                }
            );
            assert!(matches!(hit, Hit::PanelLine { offset } if offset < height));
        }
        // The rows the field took would have been the panel's, and the hit test
        // knows it: the same points answer differently on a frame with no field.
        assert_eq!(
            hit_test(inside.x, inside.y, size, None, None),
            Hit::PanelLine {
                offset: inside.y - panel.y
            }
        );
    }

    #[test]
    fn a_terminal_with_no_room_for_a_field_draws_none_and_the_panel_keeps_the_column() {
        let chrome = FOOTER_HEIGHT + 2 * BORDER_THICKNESS;
        let composer = Composer::new("something");
        let field = Some(&composer);

        // Up to the height the smallest field fits in, the column is the
        // panel's: a border round nowhere to type is furniture the terminal that
        // can least afford it would be paying for.
        for height in 0..chrome + COMPOSER_MIN_HEIGHT {
            let size = Size::new(WIDTH, height);
            assert_eq!(composer_height(size, field), 0, "in {height} rows");
            assert_eq!(
                panel_height(size, field, None),
                panel_height(size, None, None),
                "in {height} rows"
            );
        }

        // And the first height it does fit in, it fits in whole.
        let size = Size::new(WIDTH, chrome + COMPOSER_MIN_HEIGHT);
        assert_eq!(composer_height(size, field), COMPOSER_MIN_HEIGHT);
        assert_eq!(panel_height(size, field, None), 0);
    }

    #[test]
    fn the_focused_panes_border_is_lit_and_the_other_is_not() {
        let mut app = App::from_tree(&fixture::tree());
        assert_eq!(app.focus(), crate::app::Focus::Tree, "warlock opens here");

        let tree_focused = render(&app, WIDTH, FIXTURE_HEIGHT);
        app.toggle_focus();
        let panel_focused = render(&app, WIDTH, FIXTURE_HEIGHT);

        let areas = areas(tree_focused.area, None);
        let top_left = |buffer: &Buffer, pane: Rect| {
            let cell = &buffer[(pane.x, pane.y)];
            (cell.fg, cell.modifier)
        };
        // The same border glyphs either way — nothing moves — drawn in two
        // visibly different styles, and each pane's style follows the focus.
        assert_eq!(
            top_left(&tree_focused, areas.tree),
            top_left(&panel_focused, areas.panel),
            "the lit border should look the same whichever pane wears it"
        );
        assert_eq!(
            top_left(&tree_focused, areas.panel),
            top_left(&panel_focused, areas.tree),
            "the dim border should look the same whichever pane wears it"
        );
        assert_ne!(
            top_left(&tree_focused, areas.tree),
            top_left(&tree_focused, areas.panel),
            "the two borders are drawn alike, so nothing on screen says which pane the keys drive"
        );
        // Lit means this and dim means that, spelled out: a colour no node state
        // uses, plus a modifier for the terminals that show no colour.
        let (fg, modifier) = top_left(&tree_focused, areas.tree);
        assert_eq!(fg, FOCUS_COLOUR);
        assert!(modifier.contains(Modifier::BOLD));
        let (fg, modifier) = top_left(&tree_focused, areas.panel);
        assert_ne!(fg, FOCUS_COLOUR);
        assert!(modifier.contains(Modifier::DIM));
        for state in NodeState::ALL {
            assert_ne!(
                FOCUS_COLOUR,
                colour_for(state),
                "the focused border shares {state:?}'s colour"
            );
        }
        // And the focus changed the borders and nothing else: the tree still
        // says what it said, on the same rows.
        assert_eq!(tree_rows(&tree_focused), tree_rows(&panel_focused));
        assert_eq!(header_text(&tree_focused), header_text(&panel_focused));
        assert_eq!(
            row_text(&tree_focused, FIXTURE_HEIGHT - 1),
            row_text(&panel_focused, FIXTURE_HEIGHT - 1)
        );
    }

    /// The terminal the covering is asserted on: small enough that the window
    /// lands on all three parts of the frame at once — the panel's columns to
    /// its left, the tree pane's to its right, and the footer under its last
    /// rows — so "nothing shows through" is one assertion rather than three
    /// sizes of terminal.
    ///
    /// Smaller than warlock is meant to be run at, and that is the point: the
    /// window is fixed at what [`confirm_size`] says, so the way to put it over
    /// everything at once is to shrink the frame around it.
    const COVER_WIDTH: u16 = 60;
    /// See [`COVER_WIDTH`]: eight rows puts the bottom of the window over the
    /// top of the footer.
    const COVER_HEIGHT: u16 = 8;

    /// A word drawn all over the frame the confirmation is about to be drawn
    /// over, so that the covering is asserted against a screen with something to
    /// show through rather than against a blank one.
    const UNDERNEATH: &str = "underneath";

    /// An app with something on every part of the frame: the fixture's tree in
    /// the tree pane, an account long enough to fill the panel with lines wide
    /// enough to fill its rows, and the ordinary footer under both.
    fn busy_app(base: Instant, width: u16, height: u16) -> App {
        let mut app = pacting_app(base, width, height);
        let detail = [UNDERNEATH; MANY].join(" ");
        let account = app.account_mut().expect("a pact has started");
        account.open_section("crates/engine", base);
        for line in 0..MANY {
            account.record(
                &Activity::Tool {
                    name: "Read".to_owned(),
                    detail: Some(detail.clone()),
                },
                at(base, line as u64 + 1),
            );
        }
        app
    }

    /// Where the confirmation's window lands on a buffer of this size, measured
    /// off the very function [`draw`] places it with.
    fn confirm_rect(buffer: &Buffer) -> Rect {
        confirm_area(buffer.area)
    }

    /// The rows of the confirmation's window, as text, clipped to its own
    /// columns — so what these say is what the window says, with neither pane
    /// beside it on the end.
    fn confirm_rows(buffer: &Buffer) -> Vec<String> {
        let area = confirm_rect(buffer);
        (0..area.height)
            .map(|index| text_in(buffer, area, area.y + index))
            .collect()
    }

    /// What one row of the window actually says, with its border glyphs and the
    /// blanks either side of them taken off.
    ///
    /// Trimmed from the ends only, so the gap between the two answers survives
    /// and a row that leaked something from underneath keeps it: the whole use
    /// of this is that a row of the window is one of three known strings, and a
    /// helper that could turn a fourth into one of them would prove nothing.
    fn inside_the_border(row: &str) -> String {
        row.trim_matches(|glyph: char| "┌┐└┘─│ ".contains(glyph))
            .to_owned()
    }

    /// The answers line as it reads on screen: Yes, the gap, No.
    fn answers_text() -> String {
        format!("{CONFIRM_YES}{CONFIRM_ANSWER_GAP}{CONFIRM_NO}")
            .trim()
            .to_owned()
    }

    /// The colour and modifiers every cell of `answer`'s word is drawn with.
    ///
    /// Every cell rather than the first, because a highlight that covered half a
    /// word would still be a highlight the eye could find and would still be
    /// wrong. The padding either side of the word is skipped: what is asserted
    /// is that the answer is lit, and the columns the constant pads it with are
    /// [`CONFIRM_YES`]'s business.
    fn answer_style(buffer: &Buffer, answer: &str) -> Vec<(Color, Modifier)> {
        let area = confirm_rect(buffer);
        let word = answer.trim();
        let (index, row) = confirm_rows(buffer)
            .into_iter()
            .enumerate()
            .find(|(_, row)| row.contains(word))
            .unwrap_or_else(|| panic!("{word:?} is not on the window"));
        let start = u16::try_from(column_of(&row, word)).expect("a narrow window");
        let width = u16::try_from(word.chars().count()).expect("a short word");
        let y = area.y + u16::try_from(index).expect("a short window");

        (start..start + width)
            .map(|column| {
                let cell = &buffer[(area.x + column, y)];
                (cell.fg, cell.modifier)
            })
            .collect()
    }

    /// Assert that `answer` is the one the highlight is on.
    fn assert_lit(buffer: &Buffer, answer: &str) {
        let cells = answer_style(buffer, answer);
        assert!(!cells.is_empty(), "{answer:?} is drawn nowhere");
        for (fg, modifier) in cells {
            assert_eq!(fg, FOCUS_COLOUR, "{answer:?} should be lit");
            assert!(
                modifier.contains(Modifier::REVERSED),
                "{answer:?} should be lit on a terminal with no colour too"
            );
            assert!(
                modifier.contains(Modifier::BOLD),
                "{answer:?} should be lit"
            );
        }
    }

    /// Assert that `answer` is the one the highlight is not on.
    fn assert_unlit(buffer: &Buffer, answer: &str) {
        let cells = answer_style(buffer, answer);
        assert!(!cells.is_empty(), "{answer:?} is drawn nowhere");
        for (fg, modifier) in cells {
            assert_ne!(fg, FOCUS_COLOUR, "{answer:?} should not be lit");
            assert!(
                !modifier.contains(Modifier::REVERSED),
                "{answer:?} should not be lit"
            );
        }
    }

    #[test]
    fn the_window_is_sized_by_what_it_says_plus_its_margins_and_its_border() {
        // Measured off the text rather than written down, so a reworded
        // question is drawn whole rather than cut off by a width somebody
        // forgot to widen.
        let Size { width, height } = confirm_size();
        let answers = display_width(CONFIRM_YES)
            + display_width(CONFIRM_ANSWER_GAP)
            + display_width(CONFIRM_NO);
        let text = display_width(CONFIRM_QUESTION).max(answers);

        assert_eq!(
            usize::from(width),
            text + usize::from(2 * CONFIRM_MARGIN + 2 * BORDER_THICKNESS)
        );
        assert_eq!(height, CONFIRM_HEIGHT);
        assert_eq!(
            height,
            CONFIRM_LINES + 2 * CONFIRM_MARGIN_ROWS + 2 * BORDER_THICKNESS
        );
    }

    #[test]
    fn the_confirmation_is_a_small_window_centred_on_the_terminal() {
        let base = Instant::now();
        let app = busy_app(base, WIDTH, FIXTURE_HEIGHT);

        let buffer = render_confirm(&app, WIDTH, FIXTURE_HEIGHT, base, QuitConfirm::open());

        let area = confirm_rect(&buffer);
        let Size { width, height } = confirm_size();
        // Small: a window over the frame, not a second screen instead of it.
        assert_eq!((area.width, area.height), (width, height));
        assert!(area.width < WIDTH / 2, "the window is half the terminal");
        assert!(
            area.height < FIXTURE_HEIGHT,
            "the window is the whole height"
        );
        // Centred: the columns left over are shared out either side of it, and
        // so are the rows, give or take the odd one that cannot be halved.
        let left = area.x;
        let right = WIDTH - (area.x + area.width);
        let above = area.y;
        let below = FIXTURE_HEIGHT - (area.y + area.height);
        assert!(
            left.abs_diff(right) <= 1,
            "{left} columns left, {right} right"
        );
        assert!(
            above.abs_diff(below) <= 1,
            "{above} rows above, {below} below"
        );
        // And it is a window: a border all the way round it.
        let rows = confirm_rows(&buffer);
        assert!(
            rows[0].starts_with('┌') && rows[0].ends_with('┐'),
            "{rows:?}"
        );
        let last = rows.last().expect("the window has rows");
        assert!(last.starts_with('└') && last.ends_with('┘'), "{rows:?}");
        for row in &rows[1..rows.len() - 1] {
            assert!(row.starts_with('│') && row.ends_with('│'), "{rows:?}");
        }
    }

    #[test]
    fn the_confirmation_asks_its_question_and_offers_exactly_two_answers() {
        let base = Instant::now();
        let app = busy_app(base, WIDTH, FIXTURE_HEIGHT);

        let buffer = render_confirm(&app, WIDTH, FIXTURE_HEIGHT, base, QuitConfirm::open());

        let rows = confirm_rows(&buffer);
        let question = rows
            .iter()
            .position(|row| row.contains(CONFIRM_QUESTION))
            .unwrap_or_else(|| panic!("the question is not on the window: {rows:?}"));
        let answers = rows
            .iter()
            .position(|row| row.contains(CONFIRM_YES.trim()))
            .unwrap_or_else(|| panic!("the answers are not on the window: {rows:?}"));
        // Two things to read, on two lines, in the order they are read in.
        assert!(question < answers, "{rows:?}");
        // Exactly two answers: the line says Yes, the gap and No, and nothing
        // else — no third answer and no line of keys under it.
        assert_eq!(inside_the_border(&rows[answers]), answers_text());
        assert!(rows[answers].contains(CONFIRM_NO.trim()));
        // Yes to the left of No, which is what makes Left and Right positional
        // rather than a toggle. See `crate::confirm`.
        assert!(
            column_of(&rows[answers], CONFIRM_YES.trim())
                < column_of(&rows[answers], CONFIRM_NO.trim())
        );
        // Said once each: a question asked twice is two questions.
        assert_eq!(
            rows.iter()
                .filter(|row| row.contains(CONFIRM_QUESTION))
                .count(),
            1
        );
        assert_eq!(
            rows.iter()
                .filter(|row| row.contains(CONFIRM_NO.trim()))
                .count(),
            1
        );
    }

    #[test]
    fn nothing_from_the_frame_underneath_shows_through_the_window() {
        let base = Instant::now();
        let app = busy_app(base, COVER_WIDTH, COVER_HEIGHT);

        let closed = render_confirm(&app, COVER_WIDTH, COVER_HEIGHT, base, QuitConfirm::Closed);
        let open = render_confirm(&app, COVER_WIDTH, COVER_HEIGHT, base, QuitConfirm::open());

        // At this size the window is over the panel, over the tree pane and
        // over the top of the footer, and every row behind it has something on
        // it — without which the assertions below would pass on a blank screen.
        let area = confirm_rect(&open);
        let Areas { panel, tree, .. } = areas(open.area, None);
        assert!(
            area.x < panel.x + panel.width,
            "the window misses the panel"
        );
        assert!(
            area.x + area.width > tree.x,
            "the window misses the tree pane"
        );
        assert!(
            area.y + area.height > COVER_HEIGHT - FOOTER_HEIGHT,
            "the window misses the footer"
        );
        for (index, row) in confirm_rows(&closed).iter().enumerate() {
            assert!(
                !row.trim().is_empty(),
                "row {index} behind the window is blank, so this proves nothing"
            );
        }

        // What the window says is the question, the answers, or nothing at all.
        // Anything the frame underneath had put in these columns would be a
        // fourth thing here.
        let said = [String::new(), CONFIRM_QUESTION.to_owned(), answers_text()];
        let keys = *KEYS.first().expect("the keys line has keys");
        for (index, row) in confirm_rows(&open).iter().enumerate() {
            assert!(
                said.contains(&inside_the_border(row)),
                "window row {index} says {row:?}"
            );
            for leaked in [UNDERNEATH, "crates", "unpacted", keys] {
                assert!(
                    !row.contains(leaked),
                    "window row {index} shows {leaked:?} through: {row:?}"
                );
            }
        }
    }

    #[test]
    fn the_window_covers_its_own_cells_and_no_others() {
        let base = Instant::now();
        let app = busy_app(base, COVER_WIDTH, COVER_HEIGHT);

        let closed = render_confirm(&app, COVER_WIDTH, COVER_HEIGHT, base, QuitConfirm::Closed);
        let open = render_confirm(&app, COVER_WIDTH, COVER_HEIGHT, base, QuitConfirm::open());

        // Outside the window, cell for cell, the frame is the one that was
        // there before the question was asked: the panel, the tree and the
        // footer are drawn exactly as they always were and the question is laid
        // over the top of them.
        let area = confirm_rect(&open);
        for y in 0..open.area.height {
            for x in 0..open.area.width {
                if area.contains(Position::new(x, y)) {
                    continue;
                }
                assert_eq!(
                    open[(x, y)],
                    closed[(x, y)],
                    "the frame changed at column {x}, row {y}, outside the window"
                );
            }
        }
    }

    #[test]
    fn a_closed_confirmation_leaves_no_trace_of_itself_on_the_frame() {
        let base = Instant::now();
        let app = busy_app(base, WIDTH, FIXTURE_HEIGHT);

        let closed = render_confirm(&app, WIDTH, FIXTURE_HEIGHT, base, QuitConfirm::Closed);

        // Not a word of it anywhere on the frame.
        for (index, row) in rows_text(&closed).iter().enumerate() {
            assert!(!row.contains(CONFIRM_QUESTION), "row {index}: {row:?}");
            assert!(!row.contains(CONFIRM_YES.trim()), "row {index}: {row:?}");
            assert!(!row.contains(CONFIRM_NO.trim()), "row {index}: {row:?}");
        }
        // And the cells the window would have taken are still the frame's own:
        // nothing was cleared for a question nobody asked. With this and
        // `the_window_covers_its_own_cells_and_no_others`, a closed
        // confirmation draws exactly the frame that was drawn before there was
        // one — which is what every other test in this module asserts about,
        // since `render` draws with the question down.
        let behind = confirm_rows(&closed);
        assert!(
            behind.iter().any(|row| row.contains(UNDERNEATH)),
            "{behind:?}"
        );
    }

    #[test]
    fn the_highlight_opens_on_no_and_moves_to_yes() {
        let base = Instant::now();
        let app = busy_app(base, WIDTH, FIXTURE_HEIGHT);

        let opened = render_confirm(&app, WIDTH, FIXTURE_HEIGHT, base, QuitConfirm::open());
        let moved = render_confirm(
            &app,
            WIDTH,
            FIXTURE_HEIGHT,
            base,
            QuitConfirm::Open(Answer::Yes),
        );

        // The dangerous answer is never the one under the reader's finger when
        // the question arrives.
        assert_lit(&opened, CONFIRM_NO);
        assert_unlit(&opened, CONFIRM_YES);
        assert_lit(&moved, CONFIRM_YES);
        assert_unlit(&moved, CONFIRM_NO);
        // The highlight moved and nothing else did: the two answers are drawn
        // in the same columns whichever of them is lit, so the eye finds the
        // one that changed rather than re-reading a line that shifted.
        assert_eq!(confirm_rows(&opened), confirm_rows(&moved));
    }

    #[test]
    fn a_terminal_too_small_for_the_window_clamps_it_and_still_shows_it() {
        let base = Instant::now();

        // Down to one cell. A confirmation that declined to draw would leave
        // the reader in a mode with nothing on screen to say so, pressing keys
        // that reach nothing they can see — which is worse than a window with
        // its edges cut off.
        for (width, height) in [(20, 5), (4, 2), (1, 1), (2, 20), (30, 1)] {
            let app = busy_app(base, width, height);
            let closed = render_confirm(&app, width, height, base, QuitConfirm::Closed);
            let open = render_confirm(&app, width, height, base, QuitConfirm::open());
            let area = confirm_rect(&open);
            let size = format!("{width}x{height}");

            assert!(area.width > 0 && area.height > 0, "nothing drawn at {size}");
            assert!(area.x + area.width <= width, "off the right at {size}");
            assert!(area.y + area.height <= height, "off the bottom at {size}");
            let drawn = (area.y..area.y + area.height).any(|y| {
                (area.x..area.x + area.width).any(|x| !open[(x, y)].symbol().trim().is_empty())
            });
            assert!(drawn, "the window is blank at {size}");
            assert_ne!(
                rows_text(&open),
                rows_text(&closed),
                "the question changed nothing on screen at {size}"
            );
        }
    }

    #[test]
    fn the_footer_keeps_its_three_lines_and_its_wording_while_the_question_is_up() {
        let base = Instant::now();
        let app = busy_app(base, KEYS_WIDTH, MARK_ROOM_HEIGHT);

        let closed = render_confirm(
            &app,
            KEYS_WIDTH,
            MARK_ROOM_HEIGHT,
            base,
            QuitConfirm::Closed,
        );
        let open = render_confirm(
            &app,
            KEYS_WIDTH,
            MARK_ROOM_HEIGHT,
            base,
            QuitConfirm::open(),
        );

        // Three lines, in their places, saying what they said: the gate on the
        // way out is a window over the frame and not a fourth footer line, and
        // the keys line still names the way out it always named.
        let footer = areas(open.area, None).footer;
        assert_eq!(footer.height, FOOTER_HEIGHT);
        assert_eq!(open.area.height, MARK_ROOM_HEIGHT);
        for y in footer.y..footer.y + footer.height {
            assert_eq!(row_text(&open, y), row_text(&closed, y), "footer row {y}");
        }
        let keys = footer_line(&open, 1);
        for name in KEYS {
            assert!(keys.contains(name), "{keys:?} is missing {name}");
        }
        assert!(keys.contains(QUIT_KEY), "{keys:?}");
    }

    /// The directory the scope prompt's tests open over: a path with a separator
    /// in it, because a bare name would not show whether the heading has room
    /// for what a real module is called.
    const SCOPED: &str = "crates/warlock-engine";

    /// A scope already on that directory, so the field can be asserted holding
    /// something rather than only being empty.
    const CARRIED: &str = "data-plane";

    /// Where one prompt's window lands on a buffer, measured off the very
    /// functions [`draw`] places it with.
    ///
    /// The two sentences are what say which prompt: the same window is drawn
    /// twice over, so a helper that assumed the scope's words would measure the
    /// path prompt's window in the wrong columns.
    fn window_rect(buffer: &Buffer, field: &ScopeField, heading: &str, rules: &str) -> Rect {
        centred(buffer.area, scope_size(field, heading, rules))
    }

    /// The rows of one prompt's window, as text, clipped to its own columns —
    /// [`confirm_rows`] for the other kind of window.
    fn window_rows(buffer: &Buffer, field: &ScopeField, heading: &str, rules: &str) -> Vec<String> {
        let area = window_rect(buffer, field, heading, rules);
        (0..area.height)
            .map(|index| text_in(buffer, area, area.y + index))
            .collect()
    }

    /// The cell the cursor should be in: one column past `text` on the field's
    /// row, worked out from the window's own corner, its border and its margin
    /// rather than by looking for something that looks like a cursor.
    fn window_cursor(buffer: &Buffer, field: &ScopeField, heading: &str, rules: &str) -> Position {
        let area = window_rect(buffer, field, heading, rules);
        let typed = u16::try_from(display_width(field.text())).expect("a short line");

        Position::new(
            area.x + BORDER_THICKNESS + SCOPE_MARGIN + typed,
            area.y + BORDER_THICKNESS + SCOPE_MARGIN_ROWS + FIELD_LINE,
        )
    }

    /// [`window_rect`] for the scope prompt: the window drawn with the engine's
    /// sentence under it.
    fn scope_rect(buffer: &Buffer, field: &ScopeField) -> Rect {
        window_rect(buffer, field, SCOPE_HEADING, scope::RULES)
    }

    /// [`window_rows`] for the scope prompt.
    fn scope_rows(buffer: &Buffer, field: &ScopeField) -> Vec<String> {
        window_rows(buffer, field, SCOPE_HEADING, scope::RULES)
    }

    /// [`window_cursor`] for the scope prompt.
    fn cursor_cell(buffer: &Buffer, field: &ScopeField) -> Position {
        window_cursor(buffer, field, SCOPE_HEADING, scope::RULES)
    }

    /// [`window_rows`] for the path prompt: the window whose heading rides in
    /// the field and whose last line is [`PATH_RULES`].
    fn path_rows(buffer: &Buffer, field: &ScopeField) -> Vec<String> {
        window_rows(buffer, field, PATH_HEADING, PATH_RULES)
    }

    /// [`window_cursor`] for the path prompt.
    fn path_cursor(buffer: &Buffer, field: &ScopeField) -> Position {
        window_cursor(buffer, field, PATH_HEADING, PATH_RULES)
    }

    /// Which of the window's [`SCOPE_LINES`] the field is: the heading, a blank,
    /// then the field.
    const FIELD_LINE: u16 = 2;

    /// [`COVER_WIDTH`] for the scope prompt, which is the wider and taller of
    /// the two windows: a terminal small enough that the window lands over the
    /// panel, the tree pane and the top of the footer at once, and large enough
    /// that it is still a window with frame either side of it rather than a
    /// screen clamped to the terminal's own edges.
    const SCOPE_COVER_WIDTH: u16 = 80;
    /// See [`SCOPE_COVER_WIDTH`]: twelve rows puts the bottom of the window over
    /// the top of the footer.
    const SCOPE_COVER_HEIGHT: u16 = 12;

    #[test]
    fn the_scope_window_is_sized_by_what_it_says_plus_its_margins_and_its_border() {
        // Sized off the field as well as off the constants, because a path and a
        // refusal are as much of the window as the heading is.
        let refused = ScopeField::new(SCOPED, CARRIED).refused("a very long line about a rule");
        for field in [
            ScopeField::new(SCOPED, ""),
            ScopeField::new(SCOPED, CARRIED),
            ScopeField::new("a", ""),
            refused.clone(),
        ] {
            let Size { width, height } = scope_size(&field, SCOPE_HEADING, scope::RULES);
            let widest = (display_width(SCOPE_HEADING) + display_width(field.directory()))
                .max(display_width(field.text()) + display_width(SCOPE_CURSOR))
                .max(field.rule().map_or(0, display_width))
                .max(display_width(scope::RULES));

            assert_eq!(
                usize::from(width),
                widest + usize::from(2 * SCOPE_MARGIN + 2 * BORDER_THICKNESS),
                "{field:?}"
            );
            // And it is the same height whatever is in it: the row the broken
            // rule goes in is there before one is broken, so a refusal does not
            // move the field out from under the reader's eye.
            assert_eq!(height, SCOPE_HEIGHT, "{field:?}");
            assert_eq!(
                height,
                SCOPE_LINES + 2 * SCOPE_MARGIN_ROWS + 2 * BORDER_THICKNESS
            );
        }
        assert_eq!(
            scope_size(&ScopeField::new(SCOPED, ""), SCOPE_HEADING, scope::RULES).height,
            SCOPE_HEIGHT
        );
    }

    #[test]
    fn the_scope_prompt_names_the_directory_the_field_and_the_rules() {
        let base = Instant::now();
        let app = busy_app(base, WIDTH, FIXTURE_HEIGHT);
        let field = ScopeField::new(SCOPED, "");

        let buffer = render_scope(
            &app,
            WIDTH,
            FIXTURE_HEIGHT,
            base,
            &ScopePrompt::open(SCOPED, ""),
        );

        // A window, bordered all the way round, over the middle of the frame.
        let rows = scope_rows(&buffer, &field);
        assert!(
            rows[0].starts_with('┌') && rows[0].ends_with('┐'),
            "{rows:?}"
        );
        let last = rows.last().expect("the window has rows");
        assert!(last.starts_with('└') && last.ends_with('┘'), "{rows:?}");
        // Everything that has to be legible before a single character is typed:
        // what is being scoped, and the rules the answer will be judged by.
        let heading = rows
            .iter()
            .position(|row| row.contains(SCOPE_HEADING.trim()) && row.contains(SCOPED))
            .unwrap_or_else(|| panic!("the directory is not on the window: {rows:?}"));
        let rules = rows
            .iter()
            .position(|row| row.contains(scope::RULES))
            .unwrap_or_else(|| panic!("the rules are not on the window: {rows:?}"));
        // In the order they are read in, with the field between them.
        assert!(heading < rules, "{rows:?}");
        assert_eq!(
            heading,
            usize::from(BORDER_THICKNESS + SCOPE_MARGIN_ROWS),
            "{rows:?}"
        );
        // The rules are the engine's sentence, word for word: nothing in this
        // crate says how long a scope may be or which characters it may hold.
        assert!(rows[rules].contains(scope::RULES), "{rows:?}");
        // The field is empty because the directory carries no scope, and the
        // cursor is at the front of it waiting for the first character.
        assert_eq!(
            inside_the_border(
                &rows[usize::from(BORDER_THICKNESS + SCOPE_MARGIN_ROWS + FIELD_LINE)]
            ),
            ""
        );
        let cursor = cursor_cell(&buffer, &field);
        assert!(
            buffer[cursor].modifier.contains(Modifier::REVERSED),
            "no cursor at {cursor:?}: {rows:?}"
        );
    }

    #[test]
    fn the_field_opens_on_the_scope_the_directory_carries_with_the_cursor_after_it() {
        let base = Instant::now();
        let app = busy_app(base, WIDTH, FIXTURE_HEIGHT);
        let field = ScopeField::new(SCOPED, CARRIED);

        let buffer = render_scope(
            &app,
            WIDTH,
            FIXTURE_HEIGHT,
            base,
            &ScopePrompt::open(SCOPED, CARRIED),
        );

        // What is already true is on screen, so Enter on an untouched prompt
        // would set what is already set rather than clear it.
        let rows = scope_rows(&buffer, &field);
        let line = usize::from(BORDER_THICKNESS + SCOPE_MARGIN_ROWS + FIELD_LINE);
        assert_eq!(inside_the_border(&rows[line]), CARRIED, "{rows:?}");
        // And the cursor is where the next character will land: one column past
        // the text, which is the only place it can be — see `crate::prompt`.
        let cursor = cursor_cell(&buffer, &field);
        assert!(
            buffer[cursor].modifier.contains(Modifier::REVERSED),
            "no cursor at {cursor:?}: {rows:?}"
        );
        let before = Position::new(cursor.x - 1, cursor.y);
        assert_eq!(buffer[before].symbol(), &CARRIED[CARRIED.len() - 1..]);
        assert!(
            !buffer[before].modifier.contains(Modifier::REVERSED),
            "the cursor is over the text rather than after it"
        );
    }

    #[test]
    fn a_broken_rule_is_drawn_under_the_field_with_the_text_still_in_it() {
        let base = Instant::now();
        let app = busy_app(base, WIDTH, FIXTURE_HEIGHT);
        // Worded by whoever refused — the engine, in the loop — and printed
        // here without being read: this crate judges no scope and describes no
        // rule of its own.
        let broken = "a scope holds lowercase letters, digits, `-` and `_`";
        let typed = "control-plane, data-plane";
        let field = ScopeField::new(SCOPED, typed).refused(broken);

        let refused = render_scope(
            &app,
            WIDTH,
            FIXTURE_HEIGHT,
            base,
            &ScopePrompt::Open(field.clone()),
        );

        let rows = scope_rows(&refused, &field);
        let line = usize::from(BORDER_THICKNESS + SCOPE_MARGIN_ROWS + FIELD_LINE);
        // The text that was refused is still in the field, one character away
        // from being fixed, and the reason is on the row under it.
        assert_eq!(inside_the_border(&rows[line]), typed, "{rows:?}");
        assert!(rows[line + 1].contains(broken), "{rows:?}");
        // The rules are still there under that: a refusal adds a line, it does
        // not replace the one that was there before anything was typed.
        assert!(rows[line + 2].contains(scope::RULES), "{rows:?}");
        // And nothing moved: the field is on the same row of the window it was
        // on before the submit was refused.
        let opened = render_scope(
            &app,
            WIDTH,
            FIXTURE_HEIGHT,
            base,
            &ScopePrompt::open(SCOPED, typed),
        );
        let opened_rows = scope_rows(&opened, &field);
        assert_eq!(inside_the_border(&opened_rows[line]), typed);
        assert_eq!(opened_rows[line + 1].trim_matches('│').trim(), "");
    }

    #[test]
    fn nothing_from_the_frame_underneath_shows_through_the_scope_window() {
        let base = Instant::now();
        let app = busy_app(base, SCOPE_COVER_WIDTH, SCOPE_COVER_HEIGHT);
        let field = ScopeField::new(SCOPED, CARRIED);

        let closed = render_scope(
            &app,
            SCOPE_COVER_WIDTH,
            SCOPE_COVER_HEIGHT,
            base,
            &ScopePrompt::Closed,
        );
        let open = render_scope(
            &app,
            SCOPE_COVER_WIDTH,
            SCOPE_COVER_HEIGHT,
            base,
            &ScopePrompt::Open(field.clone()),
        );

        // At this size the window is over the panel, over the tree pane and over
        // the top of the footer.
        let over = scope_rect(&open, &field);
        let Areas { panel, tree, .. } = areas(open.area, None);
        assert!(
            over.x < panel.x + panel.width,
            "the window misses the panel"
        );
        assert!(
            over.x + over.width > tree.x,
            "the window misses the tree pane"
        );
        assert!(
            over.y + over.height > SCOPE_COVER_HEIGHT - FOOTER_HEIGHT,
            "the window misses the footer"
        );
        // Every row behind the window has something on it, without which the
        // assertions below would pass on a blank screen.
        for (index, row) in scope_rows(&closed, &field).iter().enumerate() {
            assert!(
                !row.trim().is_empty(),
                "row {index} behind the window is blank, so this proves nothing"
            );
        }
        // What the window says is its own five lines and nothing else.
        for (index, row) in scope_rows(&open, &field).iter().enumerate() {
            for leaked in [UNDERNEATH, "unpacted", "module"] {
                assert!(
                    !row.contains(leaked),
                    "window row {index} shows {leaked:?} through: {row:?}"
                );
            }
        }
        // Outside it, cell for cell, the frame is the one that was there before
        // the prompt opened.
        let area = scope_rect(&open, &field);
        for y in 0..open.area.height {
            for x in 0..open.area.width {
                if area.contains(Position::new(x, y)) {
                    continue;
                }
                assert_eq!(
                    open[(x, y)],
                    closed[(x, y)],
                    "the frame changed at column {x}, row {y}, outside the window"
                );
            }
        }
    }

    #[test]
    fn a_closed_scope_prompt_leaves_no_trace_of_itself_on_the_frame() {
        let base = Instant::now();
        let app = busy_app(base, WIDTH, FIXTURE_HEIGHT);

        let closed = render_scope(&app, WIDTH, FIXTURE_HEIGHT, base, &ScopePrompt::Closed);

        // Not a word of it anywhere, and the frame is the one every other test
        // in this module draws.
        for (index, row) in rows_text(&closed).iter().enumerate() {
            assert!(!row.contains(SCOPE_HEADING.trim()), "row {index}: {row:?}");
            assert!(!row.contains(scope::RULES), "row {index}: {row:?}");
        }
        assert_eq!(
            rows_text(&closed),
            rows_text(&render_at(&app, WIDTH, FIXTURE_HEIGHT, base))
        );
    }

    #[test]
    fn a_terminal_too_small_for_the_scope_window_clamps_it_and_still_shows_it() {
        let base = Instant::now();
        let field = ScopeField::new(SCOPED, CARRIED);

        // Down to one cell, for the reason the confirmation clamps: a prompt
        // that declined to draw would leave the reader typing into a window
        // they cannot see.
        for (width, height) in [(20, 5), (4, 2), (1, 1), (2, 20), (30, 1)] {
            let app = busy_app(base, width, height);
            let closed = render_scope(&app, width, height, base, &ScopePrompt::Closed);
            let open = render_scope(&app, width, height, base, &ScopePrompt::Open(field.clone()));
            let area = scope_rect(&open, &field);
            let size = format!("{width}x{height}");

            assert!(area.width > 0 && area.height > 0, "nothing drawn at {size}");
            assert!(area.x + area.width <= width, "off the right at {size}");
            assert!(area.y + area.height <= height, "off the bottom at {size}");
            assert_ne!(
                rows_text(&open),
                rows_text(&closed),
                "the prompt changed nothing on screen at {size}"
            );
        }
    }

    /// The heading the path prompt's window opens under, in the field rather
    /// than in front of it — see [`PATH_HEADING`], which is empty for exactly
    /// this reason.
    ///
    /// Spelled here rather than imported from whoever composes it: this crate
    /// draws the string it is handed and never asks what it means, so a test
    /// reading the binary's constant would be asserting that two modules agree
    /// on some words rather than that this window draws them.
    const HEADED: &str = "Write the brief to";

    /// The path a `/write` turn's answer proposes, as the binary works one out:
    /// the directory, the number and the slug a reader checks before pressing
    /// Enter, and long enough to be worth reading off the screen.
    const PROPOSED: &str = "docs/warlock-brief-13-scopes-and-sigils.md";

    #[test]
    fn the_path_window_heads_itself_holds_the_proposal_and_takes_a_refusal_under_it() {
        let base = Instant::now();
        let app = busy_app(base, WIDTH, FIXTURE_HEIGHT);
        // Worded by whoever refused — the write, in the loop — and printed here
        // without being read, the way a broken scope rule is: this window judges
        // no path either.
        let broken = "docs/warlock-brief-13-scopes-and-sigils.md is already there";
        let field = ScopeField::new(HEADED, PROPOSED);
        let refused = field.clone().refused(broken);

        let opened = render_path(
            &app,
            WIDTH,
            FIXTURE_HEIGHT,
            base,
            &ScopePrompt::open(HEADED, PROPOSED),
        );

        // The same window the scope prompt is drawn in, with the other question
        // in it: bordered all the way round, over the middle of the frame.
        let rows = path_rows(&opened, &field);
        assert!(
            rows[0].starts_with('┌') && rows[0].ends_with('┐'),
            "{rows:?}"
        );
        let last = rows.last().expect("the window has rows");
        assert!(last.starts_with('└') && last.ends_with('┘'), "{rows:?}");
        // What it is asking for, on the row the scope prompt heads itself in.
        // Nothing goes in front of it: the whole heading rides in the field.
        let heading = usize::from(BORDER_THICKNESS + SCOPE_MARGIN_ROWS);
        let line = heading + usize::from(FIELD_LINE);
        assert_eq!(inside_the_border(&rows[heading]), HEADED, "{rows:?}");
        // The path is already typed, so an Enter with nothing else pressed
        // writes the document where warlock proposed.
        assert_eq!(inside_the_border(&rows[line]), PROPOSED, "{rows:?}");
        // And the cursor is where the next character would land: one column
        // past the last of the path, not over it.
        let cursor = path_cursor(&opened, &field);
        assert!(
            opened[cursor].modifier.contains(Modifier::REVERSED),
            "no cursor at {cursor:?}: {rows:?}"
        );
        let before = Position::new(cursor.x - 1, cursor.y);
        assert_eq!(opened[before].symbol(), &PROPOSED[PROPOSED.len() - 1..]);
        assert!(
            !opened[before].modifier.contains(Modifier::REVERSED),
            "the cursor is over the path rather than after it"
        );
        // The last line is this window's own sentence and not the engine's: a
        // path is not a scope, and nothing here says what a scope may be.
        assert!(rows[line + 2].contains(PATH_RULES), "{rows:?}");
        for (index, row) in rows.iter().enumerate() {
            assert!(!row.contains(scope::RULES), "row {index}: {row:?}");
            assert!(!row.contains(SCOPE_HEADING.trim()), "row {index}: {row:?}");
        }

        let reopened = render_path(
            &app,
            WIDTH,
            FIXTURE_HEIGHT,
            base,
            &ScopePrompt::Open(refused.clone()),
        );

        // A path in the way reopens the window with the reason under the field
        // and the path still in it, a character away from being fixed — and the
        // rules line is still under that, because a refusal adds a line rather
        // than replacing the one that was there.
        let rows = path_rows(&reopened, &refused);
        assert_eq!(inside_the_border(&rows[heading]), HEADED, "{rows:?}");
        assert_eq!(inside_the_border(&rows[line]), PROPOSED, "{rows:?}");
        assert!(rows[line + 1].contains(broken), "{rows:?}");
        assert!(rows[line + 2].contains(PATH_RULES), "{rows:?}");
        // Nothing moved: the field is on the row of the window it was on before
        // the write was refused, and that row was blank until it was.
        assert_eq!(
            path_rows(&opened, &field)[line + 1]
                .trim_matches('│')
                .trim(),
            ""
        );
    }
}
