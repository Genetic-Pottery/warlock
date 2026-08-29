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
//! terminal is every panel there is. Every row is exactly
//! one row: a path longer
//! than the panel is cut with an ellipsis rather than wrapped, so the number of
//! rows on screen is the number of things that happened. See [`draw_panel`].
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
use warlock_engine::{NodeState, SCOPE_RULES};

// Renamed on the way in: `Line` here is ratatui's, the thing a row is drawn as,
// and the account's `Line` is what a row says. Both names are right where they
// live, and this module is the one place both are in scope.
use crate::account::{Account, Line as Entry};
use crate::app::{App, Chrome, Focus, Row};
use crate::colour::{FOCUS_COLOUR, GUIDE_COLOUR, colour_for};
use crate::confirm::{Answer, QuitConfirm};
use crate::prompt::{ScopeField, ScopePrompt};

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
const PANEL_INDENT: &str = "  ";

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
/// Movement goes first, all of it: `PgUp/PgDn`, then `g/G: ends`, then
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
/// [`QUIT_KEY`] on a terminal narrower than the way out itself, where the start
/// of the name is more use than a blank line.
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
/// only thing ever cut is [`QUIT_KEY`] on a terminal narrower than that name, and
/// a mark saying the key goes on would cost a column of the key it was marking.
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
const PACTING_KEYS: &str = "up/down k/j: move    space: collapse    Esc: cancel    q/Ctrl-C: quit";

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
const SCOPE_HEADING: &str = "Scope for ";

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
const SCOPE_LINES: u16 = 5;

/// How tall the whole scope window is: its lines, the rows kept clear around
/// them, and the border. [`CONFIRM_HEIGHT`]'s arithmetic, over
/// [`SCOPE_LINES`].
const SCOPE_HEIGHT: u16 = SCOPE_LINES + 2 * SCOPE_MARGIN_ROWS + 2 * BORDER_THICKNESS;

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
/// The two are drawn in that order, and the scope prompt last, so that a frame
/// somehow carrying both shows the one whose keys are live rather than half of
/// each. The event loop never opens both — while either is up it consults that
/// one instead of the app, so the key that would open the other never reaches
/// anything — and this is what that costs to be safe about it: one `if` in the
/// order the modes stack.
pub fn draw(
    frame: &mut Frame<'_>,
    app: &App,
    chrome: &Chrome,
    now: Instant,
    confirm: QuitConfirm,
    scope: &ScopePrompt,
) {
    let screen = frame.area();
    let Areas {
        panel,
        tree,
        footer,
    } = areas(screen);

    draw_panel(frame, panel, app, now);
    draw_tree_pane(frame, tree, app, chrome, now);
    draw_footer(frame, footer, app);

    if let Some(highlighted) = confirm.highlighted() {
        draw_confirm(frame, screen, highlighted);
    }
    if let Some(field) = scope.field() {
        draw_scope(frame, screen, field);
    }
}

/// The three areas one frame is cut into: the panel, the tree column beside it,
/// and the footer under both.
///
/// Split out so that [`tree_height`] answers the same question [`draw`] does,
/// from the same call: a caller that told the app one height while the frame
/// used another would scroll by a window that is not on screen.
struct Areas {
    /// The left-hand pane, the majority of the width, drawn by [`draw_panel`].
    panel: Rect,
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
fn areas(area: Rect) -> Areas {
    let [above, footer] =
        Layout::vertical([Constraint::Min(0), Constraint::Length(FOOTER_HEIGHT)]).areas(area);
    let column = tree_width(above.width);
    let [panel, tree] = Layout::horizontal([
        Constraint::Length(above.width - column),
        Constraint::Length(column),
    ])
    .areas(above);

    Areas {
        panel,
        tree,
        footer,
    }
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
    tree_rows_area(areas(Rect::from(size)).tree).height
}

/// How many lines of account a terminal of `size` has room for in the panel,
/// once the footer and the panel's own border have taken theirs.
///
/// [`tree_height`]'s counterpart, public for the same reason and measured the
/// same way: off the very [`areas`] call the frame is cut by, so the height the
/// app scrolls the panel's window by is the height the next frame draws it at.
/// The panel has no header of its own, so it keeps everything inside its border.
#[must_use]
pub fn panel_height(size: Size) -> u16 {
    pane_inner(areas(Rect::from(size)).panel).height
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
    /// A line of the panel's window: `offset` lines below the first one on
    /// screen. The whole inside of the panel answers this, drawn on or not — the
    /// panel has no selection, so a point in it is a point in the panel.
    PanelLine {
        /// How many lines below the top of the panel's window the point is.
        offset: u16,
    },
}

/// What is drawn at column `column`, row `row` of a terminal of `size`.
///
/// The one place a screen point is turned into something warlock has a name
/// for, and the mouse's counterpart to [`tree_height`] and [`panel_height`]:
/// measured off the same [`areas`] call [`draw`] cuts the frame by, so what a
/// click lands on is what the reader saw at that point rather than what a second
/// opinion about the layout thinks is there.
///
/// A function of three numbers. No frame, no app state, no terminal — which is
/// what lets the event loop's answer to a click be tested with nothing attached
/// to stdout, and what keeps this file from needing to know what a row of the
/// tree is.
///
/// Every case is asked of a [`Rect`] the layout produced, so a terminal too
/// short for a tree row, too short for a header, or too short for anything but a
/// footer answers what it has rather than underflowing its way to a row that is
/// not there.
#[must_use]
pub fn hit_test(column: u16, row: u16, size: Size) -> Hit {
    let point = Position::new(column, row);
    let screen = Rect::from(size);
    if !screen.contains(point) {
        return Hit::Offscreen;
    }

    let Areas {
        panel,
        tree,
        footer,
    } = areas(screen);
    if footer.contains(point) {
        return Hit::Footer;
    }

    let inside = pane_inner(panel);
    if inside.contains(point) {
        return Hit::PanelLine {
            offset: row.saturating_sub(inside.y),
        };
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

/// Draw the panel: the window onto the account of the pact, one row per line,
/// inside its border.
///
/// Before the first pact there is no account, and what this draws inside the
/// border is [`MARK`], centred and dim: warlock's own `W` and not one word — no
/// heading, no title, no welcome, no key hints. A screen that said something
/// before anything had happened would be saying it about nothing; a screen
/// carrying the program's mark is saying whose screen it is, which is true
/// before anything happens and stops being worth the room the moment there is an
/// account to put there. [`App::has_account`] is the switch, and not the number
/// of lines: an account that has started and has nothing in it yet is a pact
/// under way, and the mark does not come back for it. A panel too small for the
/// mark and its margins draws the bare border, exactly as it always did.
///
/// With an account, every row is one line of it: a section heading naming a
/// directory, or one thing that pass was seen doing with the elapsed clock of
/// its own section in front of it, or the line the run finished with. Which
/// lines those are is [`App::panel_lines`]'s answer, window and all — the app
/// owns the scrolling, exactly as it owns the tree's — and this only words them
/// and cuts them to the width.
///
/// A [`Paragraph`] with no [`Wrap`](ratatui::widgets::Wrap): every row is one
/// row, whatever is on it. A line that wrapped would put one activity on two
/// rows, which makes the count of rows on screen stop being the count of things
/// that happened and moves every row beneath it for a reason that has nothing to
/// do with the run.
///
/// While the window is scrolled back, the bottom edge of the border says how
/// much is below it and which key returns to live. It goes on the border rather
/// than on a row of its own, because a row of its own would be a row taken off
/// the account by the act of looking at it — and it goes away the moment the
/// panel is back at the end, since an indicator that always says `0 more` is
/// furniture rather than information.
///
/// No colour anywhere in here. The three node-state colours are the tree's and
/// [`FOCUS_COLOUR`] is the border's; a fourth meaning for colour would cost both
/// of those their meaning. Bold, which is not a colour, is all the headings get,
/// and dim, which is not one either, is all the mark gets.
fn draw_panel(frame: &mut Frame<'_>, area: Rect, app: &App, now: Instant) {
    let below = app.panel_lines_below();
    let mut block = pane_block(app.focus() == Focus::Panel);
    if below > 0 {
        block = block.title_bottom(Line::from(scrollback(below)).right_aligned().dim());
    }

    let inner = pane_inner(area);
    frame.render_widget(block, area);

    if !app.has_account() {
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

/// What the bottom edge of a scrolled-back panel says: how many lines are below
/// the view, and the key back to the newest one.
///
/// `↓ 214 more (G)`, padded a column each side so it does not sit against the
/// border's corner. Counted in lines rather than in screenfuls or in a
/// percentage, because a line is the thing the reader is scrolling past.
fn scrollback(below: usize) -> String {
    format!(" {SCROLLBACK_ARROW} {below} more ({LIVE_KEY}) ")
}

/// One line of the account as one row of the panel, cut to `width`.
///
/// A heading is the directory's path, bold and flush left; a clocked line is its
/// elapsed time and what happened, indented under the heading it belongs to; the
/// summary is the run's last word, flush left and bold like a heading because it
/// is about the whole run rather than about any one directory.
///
/// The row is built whole and cut once, rather than assembled from a styled
/// clock and a styled text: the width is a fact about the row, and two spans
/// each guessing at their share of it is how a line ends up one column too wide.
fn panel_row(line: &Entry, width: u16) -> Line<'static> {
    let width = usize::from(width);
    match line {
        Entry::Directory { path } => {
            Line::from(truncated(&path.display().to_string(), width)).bold()
        }
        Entry::Clocked { clock, text } => {
            Line::from(truncated(&format!("{PANEL_INDENT}{clock} {text}"), width))
        }
        Entry::Summary { text } => Line::from(truncated(text, width)).bold(),
    }
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
fn display_width(text: &str) -> usize {
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
/// than here) and the keys line becomes [`PACTING_KEYS`]. Same three lines,
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
/// for `area.width` by [`keys_line`], which gives whole names up in
/// [`KEY_DROP_ORDER`] until what is left fits. Cut at the edge instead, the name
/// it would lose first is the way out.
fn draw_footer(frame: &mut Frame<'_>, area: Rect, app: &App) {
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

    let keys = Line::from(if app.is_pacting() {
        PACTING_KEYS.to_owned()
    } else {
        keys_line(app.mouse_captured(), usize::from(area.width))
    })
    .dim();

    let message = Line::from(
        app.pact_line()
            .unwrap_or_else(|| app.message().unwrap_or_default().to_owned()),
    )
    .dim();
    frame.render_widget(Paragraph::new(vec![Line::from(tally), keys, message]), area);
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

/// Draw the scope prompt: what is being scoped, what has been typed into it, and
/// the rules a scope keeps, in a bordered window over the middle of `screen`.
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
fn draw_scope(frame: &mut Frame<'_>, screen: Rect, field: &ScopeField) {
    let area = centred(screen, scope_size(field));
    let block = Block::bordered().padding(Padding::symmetric(SCOPE_MARGIN, SCOPE_MARGIN_ROWS));
    let inner = block.inner(area);

    frame.render_widget(Clear, area);
    frame.render_widget(block, area);
    frame.render_widget(Paragraph::new(scope_lines(field)), inner);
}

/// The [`SCOPE_LINES`] lines of the scope window, in the order they are drawn.
///
/// The directory is bold against the plain [`SCOPE_HEADING`]: the heading is the
/// same words every time the window opens and the path is the one part of it
/// worth reading, and bold is what the tree already spends on the row the keys
/// are driving, so no new colour is invented for a window that is up for a few
/// seconds.
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
fn scope_lines(field: &ScopeField) -> Vec<Line<'_>> {
    vec![
        Line::from(vec![
            Span::raw(SCOPE_HEADING),
            Span::raw(field.directory()).bold(),
        ]),
        Line::default(),
        Line::from(vec![
            Span::raw(field.text()),
            Span::styled(SCOPE_CURSOR, Style::new().add_modifier(Modifier::REVERSED)),
        ]),
        Line::from(field.rule().unwrap_or_default()),
        Line::from(SCOPE_RULES).dim(),
    ]
}

/// How big the scope window wants to be: its widest line, the margins and the
/// border.
///
/// Measured off the lines the way [`confirm_size`] is, and off the field as well
/// as the constants, because a path and a broken rule are as much of the window
/// as the heading is. In practice [`SCOPE_RULES`] is the floor and the window
/// does not breathe as somebody types: the rules sentence is wider than a
/// directory that fits on a tree row and wider than any scope the engine would
/// accept, so the width only moves for a path or a refusal longer than it.
///
/// [`SCOPE_RULES`] is the engine's sentence rather than one written here, and
/// that is a rule rather than a convenience: a window that spelled out how long
/// a scope may be or which characters it may hold would be this crate judging a
/// scope, and there is one judge — see [`mod@crate::prompt`].
fn scope_size(field: &ScopeField) -> Size {
    let heading = display_width(SCOPE_HEADING) + display_width(field.directory());
    let typed = display_width(field.text()) + display_width(SCOPE_CURSOR);
    let widest = heading
        .max(typed)
        .max(field.rule().map_or(0, display_width))
        .max(display_width(SCOPE_RULES));
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
    use warlock_engine::{NodeState, SCOPE_RULES};

    use super::{
        Areas, BORDER_THICKNESS, CONFIRM_ANSWER_GAP, CONFIRM_HEIGHT, CONFIRM_LINES, CONFIRM_MARGIN,
        CONFIRM_MARGIN_ROWS, CONFIRM_NO, CONFIRM_QUESTION, CONFIRM_YES, ELLIPSIS, FOOTER_HEIGHT,
        GUIDE, GUIDE_BRANCH, GUIDE_LAST, HEADER_GAP, HEADER_HEIGHT, Hit, INDENT, KEYS, LIVE_KEY,
        MARK, MARK_MARGIN, MARK_MARGIN_ROWS, MOUSE_OFF_KEY, MOUSE_ON_KEY, NO_MARKER, PACTING_KEYS,
        PANEL_INDENT, QUIT_KEY, SCOPE_CURSOR, SCOPE_HEADING, SCOPE_HEIGHT, SCOPE_LINES,
        SCOPE_MARGIN, SCOPE_MARGIN_ROWS, SCROLLBACK_ARROW, SELECTION_MARKER, TREE_MIN_WIDTH,
        TREE_PERCENT, areas, centred, confirm_area, confirm_size, display_width, draw,
        guide_prefixes, hit_test, keys_line, mark_area, pane_inner, panel_height, scope_size,
        tree_height, tree_rows_area, tree_width, truncated,
    };
    use crate::account::Outcome;
    use crate::app::{App, Chrome, Row, Sigils};
    use crate::claude::Activity;
    use crate::colour::{FOCUS_COLOUR, GUIDE_COLOUR, colour_for};
    use crate::confirm::{Answer, QuitConfirm};
    use crate::fixture;
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
    const KEYS_WIDTH: u16 = 160;

    /// Tall enough for the whole fixture with its files on screen, chrome
    /// included, so a file test asserts about rows rather than about where the
    /// window happened to stop.
    const FILES_HEIGHT: u16 = 20;

    /// Tall enough for the whole fixture's directories on screen, chrome
    /// included: five rows and then some.
    const FIXTURE_HEIGHT: u16 = 12;

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
                tree_rows_area(areas(Rect::new(0, 0, *width, HEIGHT)).tree).width == rows_width
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
        app.set_panel_height(panel_height(Size::new(width, height)));
        app.start_account(base);
        app
    }

    /// Where the panel's lines land in a buffer of this size: inside the
    /// panel's border, which is the whole of the pane — the panel has no header.
    fn panel_area(buffer: &Buffer) -> Rect {
        pane_inner(areas(buffer.area).panel)
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
        let panel = areas(buffer.area).panel;

        text_in(buffer, panel, panel.y + panel.height - 1)
    }

    /// Where the tree's rows land in a buffer of this size: inside the tree
    /// pane's border, under its header. Measured off the layout the frame was
    /// cut by, so a test asserts about where the tree is rather than about where
    /// it used to be.
    fn rows_area(buffer: &Buffer) -> Rect {
        tree_rows_area(areas(buffer.area).tree)
    }

    /// The tree pane's header line: one row, inside the border, above the rows.
    fn header_area(buffer: &Buffer) -> Rect {
        let inner = pane_inner(areas(buffer.area).tree);

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

    /// The one place a frame is actually drawn: the app, the instant, and both
    /// windows that can be over it.
    fn render_windows(
        app: &App,
        chrome: &Chrome,
        width: u16,
        height: u16,
        now: Instant,
        confirm: QuitConfirm,
        scope: &ScopePrompt,
    ) -> Buffer {
        let mut terminal =
            Terminal::new(TestBackend::new(width, height)).expect("test backend never fails");
        terminal
            .draw(|frame| draw(frame, app, chrome, now, confirm, scope))
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
    /// four sides, and nothing on its bottom edge.
    fn assert_bare_panel(buffer: &Buffer) {
        let panel = areas(buffer.area).panel;
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
        for x in panel.x..panel.x + panel.width {
            assert_ne!(buffer[(x, panel.y)].symbol(), " ", "the top edge at {x}");
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
        assert_eq!(
            drawn,
            [
                "> - warlock",
                "  ├ - crates",
                "  │ ├   engine",
                "  │ └   tui",
                "  └   assets",
            ]
        );
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
        assert_eq!(guide_columns(&buffer, 1), [gutter]);
        // `engine`, two levels down: two guides, one level apart — the vertical
        // carrying `crates` on past it and then its own corner — and its name
        // still starts one marker past the last of them.
        assert_eq!(guide_columns(&buffer, 2), [gutter, gutter + level]);
        assert_eq!(
            column_of(&tree_row(&buffer, 2), "engine"),
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
        let mut app = pacting_app(base, WIDTH, FIXTURE_HEIGHT);
        app.select_next();
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
        let mut app = App::from_tree(&fixture::tree());
        // Down onto `crates`, which is one level in and so has a guide in front
        // of it for the highlight to reach over.
        app.select_next();

        let buffer = render(&app, WIDTH, FIXTURE_HEIGHT);

        let area = rows_area(&buffer);
        let columns = guide_columns(&buffer, 1);
        assert_eq!(columns.len(), 1, "the fixture's crates sits at depth 1");
        for column in columns {
            let cell = &buffer[(area.x + column, area.y + 1)];
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
        assert_eq!(tree_row(&before, 1), "  ├ - crates");
        assert_eq!(tree_row(&after, 1), "  ├ + crates");
        // And what it was hiding is gone from the screen, leaving the root's
        // other child where the children were.
        let drawn: Vec<String> = tree_rows(&after)
            .into_iter()
            .take(collapsed.rows().len())
            .collect();
        assert_eq!(drawn, ["> - warlock", "  ├ + crates", "  └   assets"]);
    }

    #[test]
    fn a_directory_with_nothing_under_it_carries_neither_marker() {
        let app = App::from_tree(&fixture::tree());

        let buffer = render(&app, WIDTH, FIXTURE_HEIGHT);

        // `assets` has no children, so it is neither collapsed nor expanded and
        // says so by carrying no marker — while still lining its name up with
        // the marked rows at its own depth.
        let leaf = tree_row(&buffer, 4);
        assert_eq!(leaf, "  └   assets");
        assert!(!leaf.contains('+') && !leaf.contains('-'), "{leaf:?}");
        assert_eq!(
            column_of(&leaf, "assets"),
            column_of(&tree_row(&buffer, 1), "crates"),
            "a leaf's name should start where a sibling directory's does"
        );
        // Pressing space on it changes nothing on screen: nothing to hide.
        let pressed = App::from_tree(&fixture::tree()).with_collapsed(["warlock/assets"]);
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
        let mut app = pacting_app(base, WIDTH, FILES_HEIGHT);
        // The keypress first, as the reader makes it: a pact on `crates`
        // covers everything below it and paints the lot stale.
        app.select_next();
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
        let mut app = pacting_app(base, WIDTH, FIXTURE_HEIGHT);
        app.select_next();
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
        let mut app = pacting_app(base, WIDTH, FIXTURE_HEIGHT);
        app.select_next();
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
        let mut app = App::from_tree(&fixture::tree());
        app.select_next();

        let buffer = render(&app, WIDTH, FIXTURE_HEIGHT);

        assert!(!tree_row(&buffer, 0).starts_with(SELECTION_MARKER));
        assert_eq!(tree_row(&buffer, 1), "> ├ - crates");
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

        let tally = row_text(&buffer, height - FOOTER_HEIGHT);
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
        let keys = row_text(&buffer, height - FOOTER_HEIGHT + 1);
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
        assert_eq!(row_text(&buffer, height - 1), "");
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
        assert_eq!(row_text(&buffer, height - 1), said);
        // And it took nothing else's line: the tally and the keys are still on
        // the two lines above it.
        assert!(row_text(&buffer, height - FOOTER_HEIGHT).contains("unpacted"));
        assert!(row_text(&buffer, height - FOOTER_HEIGHT + 1).contains("p: pact"));

        // The next keystroke moves on, and the line goes blank again.
        app.select_next();
        let buffer = render(&app, 120, height);
        assert_eq!(app.message(), None);
        assert_eq!(row_text(&buffer, height - 1), "");
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
            row_text(&buffer, height - 1),
            "pacting warlock/crates/engine (3/12)"
        );
        // The tally has not moved, and no fourth line grew under the footer:
        // the progress line took the message line rather than adding one.
        assert_eq!(
            row_text(&buffer, height - FOOTER_HEIGHT),
            row_text(&before, height - FOOTER_HEIGHT)
        );
        assert_eq!(buffer.area.height, height);
        // And the tree above the footer is untouched: nothing marks the
        // directory being worked.
        assert_eq!(tree_rows(&buffer), tree_rows(&before));

        // It moves with the run.
        app.set_pact_in_flight("warlock/assets", 4, 12);
        let buffer = render(&app, KEYS_WIDTH, height);
        assert_eq!(
            row_text(&buffer, height - 1),
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
            row_text(&buffer, height - 1),
            "pacting warlock/crates/engine (3/12) — summarising warlock/crates/engine/Cargo.toml (2/9)"
        );
        // The keys line is untouched — a pass running is not a key to press —
        // and so is the tally above it.
        assert_eq!(row_text(&buffer, height - FOOTER_HEIGHT + 1), PACTING_KEYS);
        assert_eq!(
            row_text(&buffer, height - FOOTER_HEIGHT),
            row_text(&quiet, height - FOOTER_HEIGHT)
        );
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
            row_text(&buffer, height - 1),
            "pacting warlock/crates/engine (3/12) — already running"
        );
        // The keys line is still the pacting one: a press that started nothing
        // does not change what the keys do, and Esc still says cancel.
        assert_eq!(row_text(&buffer, height - FOOTER_HEIGHT + 1), PACTING_KEYS);
        // The tally is untouched, on the first line of the footer.
        assert_eq!(
            row_text(&buffer, height - FOOTER_HEIGHT),
            row_text(&quiet, height - FOOTER_HEIGHT)
        );
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
        let fits =
            u16::try_from(display_width(progress)).expect("a footer line's worth of columns");

        // Drawn exactly as wide as the fraction and not a column more: the
        // fraction survives whole and the suffix is the part that is gone.
        let buffer = render(&app, fits, height);
        assert_eq!(row_text(&buffer, height - 1), progress);
        // Gone rather than moved: an unwrapped line is cut at the right edge,
        // so no part of the suffix turns up on a line of its own, and the
        // footer is still three lines with the keys on the middle one.
        for y in 0..height - 1 {
            assert!(
                !row_text(&buffer, y).contains("already"),
                "the suffix wrapped onto row {y}"
            );
        }
        assert!(row_text(&buffer, height - FOOTER_HEIGHT + 1).starts_with("up/down"));
        assert_eq!(buffer.area.height, height);

        // The cut is at the right edge and column by column: two columns wider
        // and the first of the suffix is back, with the fraction still whole in
        // front of it rather than shortened to make room for it.
        let wider = render(&app, fits + 2, height);
        assert_eq!(row_text(&wider, height - 1), format!("{progress} —"));

        // And a terminal wide enough for the whole of it draws the whole of it.
        let whole = u16::try_from(display_width(&format!("{progress}{suffix}")))
            .expect("a footer line's worth of columns");
        assert_eq!(
            row_text(&render(&app, whole, height), height - 1),
            format!("{progress}{suffix}")
        );
    }

    #[test]
    fn the_keys_line_advertises_esc_as_cancel_while_a_pact_runs_and_says_quit_otherwise() {
        let mut app = App::from_tree(&fixture::tree());
        let height = 10;
        let y = height - FOOTER_HEIGHT + 1;

        let idle = render(&app, KEYS_WIDTH, height);
        app.set_pact_in_flight("warlock/crates/engine", 3, 12);
        let pacting = render(&app, KEYS_WIDTH, height);

        // Byte for byte today's line with no pact running, and the pacting line
        // whole while one is: equality, so a line that outgrew the terminal it
        // is drawn on fails here rather than losing its right-hand end quietly.
        assert_eq!(
            row_text(&idle, y),
            keys_line(app.mouse_captured(), usize::from(KEYS_WIDTH))
        );
        assert_eq!(row_text(&pacting, y), PACTING_KEYS);
        // Esc means two things, and the line says which one it means now.
        let said = row_text(&pacting, y);
        assert!(said.contains("Esc: cancel"), "{said:?}");
        assert!(!said.contains("Esc/Ctrl-C: quit"), "{said:?}");
        assert!(keys_line(true, usize::from(KEYS_WIDTH)).contains("Esc/Ctrl-C: quit"));

        // The line is short enough to survive the narrow terminal the other
        // footer tests draw on, because it is the line that answers "how do I
        // stop this?".
        let narrow = render(&app, 120, height);
        assert_eq!(row_text(&narrow, y), PACTING_KEYS);

        // And the run ending puts today's line back, exactly.
        app.clear_pact_in_flight();
        assert_eq!(
            row_text(&render(&app, KEYS_WIDTH, height), y),
            keys_line(app.mouse_captured(), usize::from(KEYS_WIDTH))
        );
    }

    #[test]
    fn the_keys_line_names_the_mouse_key_by_what_the_next_press_does() {
        let mut app = App::from_tree(&fixture::tree());
        let height = 10;
        let y = height - FOOTER_HEIGHT + 1;

        // Reporting its mouse, which is how warlock starts: the key on offer is
        // the one that stops it.
        app.set_mouse_captured(true);
        let capturing = render(&app, KEYS_WIDTH, height);
        let keys = row_text(&capturing, y);
        assert_eq!(keys, keys_line(true, usize::from(KEYS_WIDTH)));
        assert!(keys.contains(MOUSE_OFF_KEY), "{keys:?}");
        assert!(!keys.contains(MOUSE_ON_KEY), "{keys:?}");

        // And with capture off, the same key named by what it does now: turn it
        // back on. This is the wording that matters — it is the only thing on
        // screen that says the wheel is the terminal's for the moment.
        app.set_mouse_captured(false);
        let released = render(&app, KEYS_WIDTH, height);
        let keys = row_text(&released, y);
        assert_eq!(keys, keys_line(false, usize::from(KEYS_WIDTH)));
        assert!(keys.contains(MOUSE_ON_KEY), "{keys:?}");
        assert!(!keys.contains(MOUSE_OFF_KEY), "{keys:?}");

        // Nothing else on the screen moved: the toggle is a fact about the
        // terminal, not about the tree, and it is not announced on the message
        // line either — that line is blank in both frames, and every row above
        // the keys line is the row it was.
        assert_eq!(row_text(&capturing, height - 1), "");
        assert_eq!(row_text(&released, height - 1), "");
        for row in 0..y {
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
        assert_eq!(row_text(&render(&app, KEYS_WIDTH, height), y), PACTING_KEYS);
        app.set_mouse_captured(true);
        assert_eq!(row_text(&render(&app, KEYS_WIDTH, height), y), PACTING_KEYS);
        assert!(!PACTING_KEYS.contains("mouse"));
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
        assert!(row_text(&buffer, height - FOOTER_HEIGHT).contains("0 unpacted"));
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
        let narrow = pane_inner(areas(Rect::new(0, 0, NARROW_WIDTH, HEIGHT)).tree);
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
        let pane = areas(buffer.area).tree;
        assert_eq!(header.y, pane.y + BORDER_THICKNESS);
        assert_eq!(header.x, pane.x + BORDER_THICKNESS);
    }

    #[test]
    fn the_panel_is_the_majority_of_the_width_and_the_tree_column_takes_its_share_of_the_rest() {
        // Wide enough that the proportion, not the floor, decides: 30% of 160 is
        // 48, comfortably past the floor of 30.
        let panes = areas(Rect::new(0, 0, KEYS_WIDTH, 24));

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
            let panes = areas(Rect::new(0, 0, width, 24));

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
            let panes = areas(Rect::new(0, 0, width, 24));

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
        let panes = areas(Rect::from(size));
        let hit = |x, y| hit_test(x, y, size);

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
        assert_eq!(inside.height, panel_height(size));
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
                hit_test(rows.x, rows.y + offset, size),
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
        assert_eq!(hit_test(header.x, header.y, size), Hit::TreeHeader);
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
                    let hit = hit_test(x, y, size);
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
        let inside = pane_inner(areas(Rect::from(header)).tree);
        assert_eq!(inside.height, HEADER_HEIGHT);
        assert_eq!(hit_test(inside.x, inside.y, header), Hit::TreeHeader);
        for height in 0..CHROME_HEIGHT - HEADER_HEIGHT {
            let size = Size::new(WIDTH, height);
            for y in 0..height {
                for x in 0..WIDTH {
                    let hit = hit_test(x, y, size);
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
            let panes = areas(Rect::from(size));
            assert_eq!(
                panes.tree.width,
                width / 2,
                "at {width} columns the floor should have given way"
            );

            let inside = pane_inner(panes.panel);
            let rows = tree_rows_area(panes.tree);
            assert_eq!(
                hit_test(inside.x, inside.y, size),
                Hit::PanelLine { offset: 0 },
                "at {width} columns"
            );
            assert_eq!(
                hit_test(
                    inside.x + inside.width - 1,
                    inside.y + inside.height - 1,
                    size
                ),
                Hit::PanelLine {
                    offset: inside.height - 1
                },
                "at {width} columns"
            );
            assert_eq!(
                hit_test(rows.x, rows.y, size),
                Hit::TreeRow { offset: 0 },
                "at {width} columns"
            );
            // The two columns between the panes' insides are border on both
            // sides of the join, whichever pane owns which.
            for x in [inside.x + inside.width, rows.x - 1] {
                assert_eq!(hit_test(x, rows.y, size), Hit::Border, "at {width} columns");
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
                    match hit_test(x, y, size) {
                        Hit::TreeRow { offset } => assert!(
                            offset < tree_height(size),
                            "({x}, {y}) of {width}x{height} is row {offset} of a window {} tall",
                            tree_height(size)
                        ),
                        Hit::PanelLine { offset } => assert!(
                            offset < panel_height(size),
                            "({x}, {y}) of {width}x{height} is line {offset} of a window {} tall",
                            panel_height(size)
                        ),
                        _ => {}
                    }
                }
            }

            // And a point off the end of the frame is nothing warlock drew,
            // rather than the nearest thing it did draw.
            assert_eq!(hit_test(width, 0, size), Hit::Offscreen);
            assert_eq!(hit_test(0, height, size), Hit::Offscreen);
            assert_eq!(hit_test(u16::MAX, u16::MAX, size), Hit::Offscreen);
        }
    }

    #[test]
    fn the_panel_draws_the_mark_and_not_one_word_inside_it_before_the_first_pact() {
        // Both focus states and a tree with something in it: whatever the app is
        // doing, the panel has nothing to say until a pact says it, so what is
        // inside its border is warlock's mark and blank rows either side of it.
        let mut app = App::from_tree(&fixture::tree());
        assert!(!app.has_account());
        for _ in 0..2 {
            let buffer = render(&app, MARK_ROOM_WIDTH, MARK_ROOM_HEIGHT);

            assert_mark_drawn(&buffer);
            // And its border really is there, on all four sides, carrying
            // nothing of its own: no title, and no scrollback indicator on a
            // panel with nothing to scroll.
            let panel = areas(buffer.area).panel;
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

        assert_eq!(areas(buffer.area).panel.width, 20);
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

        assert_eq!(areas(buffer.area).panel.width, 50);
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
        // is empty, and empty is not the same as free.
        assert!(app.panel_lines(at(base, 1)).is_empty());
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
    fn a_line_too_long_for_the_panel_is_cut_with_an_ellipsis_and_never_wrapped() {
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
        // Cut, with the ellipsis in place of what was cut, and the row that
        // fits left alone. Three lines, three rows: what did not fit is gone
        // rather than pushed onto a row of its own.
        assert_eq!(
            drawn[..3],
            [
                format!("crates/warlock-en{ELLIPSIS}"),
                format!("{PANEL_INDENT}0:02 Read crate{ELLIPSIS}"),
                format!("{PANEL_INDENT}0:02 thinking"),
            ],
        );
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
        let height = usize::from(panel_height(Size::new(WIDTH, HEIGHT)));
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
    fn the_panel_height_the_app_is_told_is_the_height_the_frame_gives_the_panel() {
        // A terminal with room for a single line of account and three more on
        // the way up, and the two the other tests draw at.
        let chrome = FOOTER_HEIGHT + 2 * BORDER_THICKNESS;
        for height in [chrome + 1, chrome + 2, HEIGHT, FIXTURE_HEIGHT, 24] {
            let measured = panel_height(Size::new(WIDTH, height));
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
                panel_height(Size::new(WIDTH, height)),
                0,
                "in {height} rows"
            );
        }
    }

    #[test]
    fn the_focused_panes_border_is_lit_and_the_other_is_not() {
        let mut app = App::from_tree(&fixture::tree());
        assert_eq!(app.focus(), crate::app::Focus::Tree, "warlock opens here");

        let tree_focused = render(&app, WIDTH, FIXTURE_HEIGHT);
        app.toggle_focus();
        let panel_focused = render(&app, WIDTH, FIXTURE_HEIGHT);

        let areas = areas(tree_focused.area);
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
        let Areas { panel, tree, .. } = areas(open.area);
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
        let footer = areas(open.area).footer;
        assert_eq!(footer.height, FOOTER_HEIGHT);
        assert_eq!(open.area.height, MARK_ROOM_HEIGHT);
        for y in footer.y..footer.y + footer.height {
            assert_eq!(row_text(&open, y), row_text(&closed, y), "footer row {y}");
        }
        let keys = row_text(&open, footer.y + 1);
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

    /// Where the scope prompt's window lands on a buffer, measured off the very
    /// functions [`draw`] places it with.
    fn scope_rect(buffer: &Buffer, field: &ScopeField) -> Rect {
        centred(buffer.area, scope_size(field))
    }

    /// The rows of the scope prompt's window, as text, clipped to its own
    /// columns — [`confirm_rows`] for the other window.
    fn scope_rows(buffer: &Buffer, field: &ScopeField) -> Vec<String> {
        let area = scope_rect(buffer, field);
        (0..area.height)
            .map(|index| text_in(buffer, area, area.y + index))
            .collect()
    }

    /// The cell the cursor should be in: one column past `text` on the field's
    /// row, worked out from the window's own corner, its border and its margin
    /// rather than by looking for something that looks like a cursor.
    fn cursor_cell(buffer: &Buffer, field: &ScopeField) -> Position {
        let area = scope_rect(buffer, field);
        let typed = u16::try_from(display_width(field.text())).expect("a short scope");

        Position::new(
            area.x + BORDER_THICKNESS + SCOPE_MARGIN + typed,
            area.y + BORDER_THICKNESS + SCOPE_MARGIN_ROWS + FIELD_LINE,
        )
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
            let Size { width, height } = scope_size(&field);
            let widest = (display_width(SCOPE_HEADING) + display_width(field.directory()))
                .max(display_width(field.text()) + display_width(SCOPE_CURSOR))
                .max(field.rule().map_or(0, display_width))
                .max(display_width(SCOPE_RULES));

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
            scope_size(&ScopeField::new(SCOPED, "")).height,
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
            .position(|row| row.contains(SCOPE_RULES))
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
        assert!(rows[rules].contains(SCOPE_RULES), "{rows:?}");
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
        assert!(rows[line + 2].contains(SCOPE_RULES), "{rows:?}");
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
        let Areas { panel, tree, .. } = areas(open.area);
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
            assert!(!row.contains(SCOPE_RULES), "row {index}: {row:?}");
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
}
