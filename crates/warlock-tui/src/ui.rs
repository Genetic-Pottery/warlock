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
//! the first pact of the session it draws a border and not one glyph inside it —
//! there is nothing true to put there, and placeholder text would be something
//! the screen said that was not so. Every row is exactly one row: a path longer
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
//! collapsed, with a marker on the line. There is deliberately no mouse.

use std::time::Instant;

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect, Size};
use ratatui::style::{Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, List, ListItem, ListState, Paragraph};
use warlock_engine::NodeState;

// Renamed on the way in: `Line` here is ratatui's, the thing a row is drawn as,
// and the account's `Line` is what a row says. Both names are right where they
// live, and this module is the one place both are in scope.
use crate::account::Line as Entry;
use crate::app::{App, Focus, Row};
use crate::colour::{FOCUS_COLOUR, colour_for};

/// One level of nesting, per unit of the depth the engine's walk yields.
const INDENT: &str = "  ";

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

/// The one line naming the tree's root.
const HEADER_HEIGHT: u16 = 1;

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

/// The keys line of the footer: every key that does something, in one line.
///
/// The movement keys first and together, in the order a reader reaches for
/// them: one row, one screen, the whole tree. Then the three keys that move
/// nothing but change what there is to move through — space, which hides a
/// subtree, `o`, which hides everything Warlock is not managing, and `f`, which
/// is the one of the three that puts rows on screen rather than taking them
/// off — and only then the keys that change something.
///
/// Every name here is as short as it can be and still be read: the line is
/// already wider than an eighty-column terminal, and a key nobody can see
/// because the line ran off the right-hand edge is a key nobody knows about.
/// That is why `o` is labelled with what it leaves on screen rather than with a
/// sentence about filtering, and `f` with what it shows rather than with a
/// sentence about a toggle.
const KEYS: &str = "up/down k/j: move    PgUp/PgDn: page    g/G: first/last    \
                    space: collapse    o: pacted    f: files    p: pact    \
                    q/Esc/Ctrl-C: quit";

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
/// against it, and a renderer that called [`Instant::now`] itself would be a
/// second clock for a test to fight. The event loop already redraws on a tick,
/// so handing it the instant it woke up at is all the ticking there is.
pub fn draw(frame: &mut Frame<'_>, app: &App, now: Instant) {
    let Areas {
        panel,
        tree,
        footer,
    } = areas(frame.area());

    draw_panel(frame, panel, app, now);
    draw_tree_pane(frame, tree, app);
    draw_footer(frame, footer, app);
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
/// Before the first pact there is no account and this draws the border and
/// nothing else — not a heading, not a title, not a word of welcome. A screen
/// that said something before anything had happened would be saying it about
/// nothing.
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
/// of those their meaning. Bold, which is not a colour, is all the headings get.
fn draw_panel(frame: &mut Frame<'_>, area: Rect, app: &App, now: Instant) {
    let below = app.panel_lines_below();
    let mut block = pane_block(app.focus() == Focus::Panel);
    if below > 0 {
        block = block.title_bottom(Line::from(scrollback(below)).right_aligned().dim());
    }

    let inner = pane_inner(area);
    frame.render_widget(block, area);

    let rows: Vec<Line<'static>> = app
        .panel_lines(now)
        .iter()
        .map(|line| panel_row(line, inner.width))
        .collect();
    frame.render_widget(Paragraph::new(rows), inner);
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
fn draw_tree_pane(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let inner = pane_inner(area);
    frame.render_widget(pane_block(app.focus() == Focus::Tree), area);

    let [header_area, rows_area] =
        Layout::vertical([Constraint::Length(HEADER_HEIGHT), Constraint::Min(0)]).areas(inner);
    draw_header(frame, header_area, app);
    draw_tree(frame, rows_area, app);
}

/// Draw the header: which tree this is, as the app state already words it.
///
/// Bold rather than coloured, because every colour on this screen already
/// means a node state and the header is not a node.
fn draw_header(frame: &mut Frame<'_>, area: Rect, app: &App) {
    frame.render_widget(
        Paragraph::new(Line::from(app.header().to_owned()).bold()),
        area,
    );
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
fn draw_tree(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let first = app.scroll_offset().min(app.rows().len());
    let height = usize::from(area.height);
    let items: Vec<ListItem<'_>> = app.rows()[first..]
        .iter()
        .take(height)
        .map(|row| ListItem::new(line(row, app.is_collapsed(&row.path))))
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
/// The whole line takes one style, the row's state colour, marker included:
/// colour on this screen means node state and nothing else, so a marker in a
/// colour of its own would be a second thing colour meant.
///
/// A file row needs no case of its own here and deliberately does not get one.
/// It carries no children, so it falls into [`NO_MARKER`] like any other
/// childless row — there is nothing under a file to collapse — and its depth is
/// already one deeper than its directory's, so it indents under it. Its colour
/// is its directory's state, copied onto the row when the tree was flattened
/// (see [`Row::file`]), which is how the design doc's rule that a file takes its
/// module's colour arrives here as an ordinary row with an ordinary colour.
fn line(row: &Row, collapsed: bool) -> Line<'static> {
    let name = row
        .path
        .file_name()
        .unwrap_or(row.path.as_os_str())
        .to_string_lossy();
    let marker = match (row.has_children(), collapsed) {
        (false, _) => NO_MARKER,
        (true, true) => COLLAPSED_MARKER,
        (true, false) => EXPANDED_MARKER,
    };

    Line::styled(
        format!("{}{marker}{name}", INDENT.repeat(row.depth)),
        colour_for(row.state),
    )
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

    let keys = Line::from(if app.is_pacting() { PACTING_KEYS } else { KEYS }).dim();

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

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::time::{Duration, Instant};

    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;
    use ratatui::layout::{Rect, Size};
    use ratatui::style::{Color, Modifier};
    use warlock_engine::NodeState;

    use super::{
        BORDER_THICKNESS, ELLIPSIS, FOOTER_HEIGHT, HEADER_HEIGHT, INDENT, KEYS, LIVE_KEY,
        NO_MARKER, PACTING_KEYS, PANEL_INDENT, SCROLLBACK_ARROW, SELECTION_MARKER, TREE_MIN_WIDTH,
        TREE_PERCENT, areas, display_width, draw, pane_inner, panel_height, tree_height,
        tree_rows_area, tree_width, truncated,
    };
    use crate::account::Outcome;
    use crate::app::{App, Row};
    use crate::claude::Activity;
    use crate::colour::{FOCUS_COLOUR, colour_for};
    use crate::fixture;

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

    /// A terminal wide enough for the whole of [`KEYS`], whatever it grows to:
    /// the footer test asserts that line for equality, and a line drawn onto a
    /// narrower terminal than it needs would be compared against its own
    /// truncation.
    const KEYS_WIDTH: u16 = 160;

    /// Tall enough for the whole fixture with its files on screen, chrome
    /// included, so a file test asserts about rows rather than about where the
    /// window happened to stop.
    const FILES_HEIGHT: u16 = 20;

    /// Tall enough for the whole fixture's directories on screen, chrome
    /// included: five rows and then some.
    const FIXTURE_HEIGHT: u16 = 12;

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
    /// selected row: the selection marker's gutter, the depth's indent, the
    /// blank a childless row carries where a collapse marker would go, then the
    /// name.
    fn drawn_row(index: usize, selected: usize) -> String {
        let gutter = if index == selected {
            SELECTION_MARKER.to_owned()
        } else {
            " ".repeat(SELECTION_MARKER.chars().count())
        };
        format!("{gutter}{INDENT}{NO_MARKER}module{index}")
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
    fn render_at(app: &App, width: u16, height: u16, now: Instant) -> Buffer {
        let mut terminal =
            Terminal::new(TestBackend::new(width, height)).expect("test backend never fails");
        terminal
            .draw(|frame| draw(frame, app, now))
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

    /// The foreground colour of tree row `index`'s first glyph, ignoring the
    /// gutter the selection marker lives in: that gutter is drawn by the list
    /// itself and takes no state colour, while everything to the right of it is
    /// the row's own text, however far it is indented.
    fn first_glyph_colour(buffer: &Buffer, index: u16) -> Color {
        let area = rows_area(buffer);
        let gutter = u16::try_from(SELECTION_MARKER.chars().count()).expect("a two-char marker");
        (area.x + gutter..area.x + area.width)
            .map(|x| &buffer[(x, area.y + index)])
            .find(|cell| !cell.symbol().trim().is_empty())
            .expect("row has a glyph on it")
            .fg
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
                "    - crates",
                "        engine",
                "        tui",
                "      assets",
            ]
        );
    }

    #[test]
    fn a_collapsed_directory_is_marked_differently_from_an_expanded_one() {
        let expanded = App::from_tree(&fixture::tree());
        let collapsed = App::from_tree(&fixture::tree()).with_collapsed(["warlock/crates"]);

        let before = render(&expanded, WIDTH, FIXTURE_HEIGHT);
        let after = render(&collapsed, WIDTH, FIXTURE_HEIGHT);

        // Same directory, same indent, same name, and the one thing that
        // differs is the marker saying whether anything is under it.
        assert_eq!(tree_row(&before, 1), "    - crates");
        assert_eq!(tree_row(&after, 1), "    + crates");
        // And what it was hiding is gone from the screen, leaving the root's
        // other child where the children were.
        let drawn: Vec<String> = tree_rows(&after)
            .into_iter()
            .take(collapsed.rows().len())
            .collect();
        assert_eq!(drawn, ["> - warlock", "    + crates", "      assets"]);
    }

    #[test]
    fn a_directory_with_nothing_under_it_carries_neither_marker() {
        let app = App::from_tree(&fixture::tree());

        let buffer = render(&app, WIDTH, FIXTURE_HEIGHT);

        // `assets` has no children, so it is neither collapsed nor expanded and
        // says so by carrying no marker — while still lining its name up with
        // the marked rows at its own depth.
        let leaf = tree_row(&buffer, 4);
        assert_eq!(leaf, "      assets");
        assert!(!leaf.contains('+') && !leaf.contains('-'), "{leaf:?}");
        assert_eq!(
            leaf.find("assets"),
            tree_row(&buffer, 1).find("crates"),
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
        // nothing under a file to hide.
        assert_eq!(
            drawn,
            [
                "> - warlock",
                "      README.md",
                "      WARLOCK.md",
                "    - crates",
                "        engine",
                "          Cargo.toml",
                "          WARLOCK.md",
                "        tui",
                "          WARLOCK.md",
                "      assets",
                "        WARLOCK.md",
                "        logo.svg",
            ]
        );
        assert!(
            drawn[1].find("README.md") > drawn[0].find("warlock"),
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
        assert_eq!(tree_row(&buffer, 1), ">   - crates");
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
                .map(|index| drawn_row(index, selected))
                .collect();
            // Exactly a windowful, and exactly the window the app says.
            assert_eq!(drawn.len(), window, "selection {selected}");
            assert_eq!(drawn, expected, "selection {selected}");
            // Which contains the selected row, marked and highlighted, once.
            assert!(
                drawn.contains(&drawn_row(selected, selected)),
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
        let mut chrome = None;
        let mut offsets = Vec::new();

        // Every row in turn, so the window is at the top, somewhere in the
        // middle, and at the bottom over the course of the walk.
        for selected in 0..MANY {
            let app = tall_app(selected).with_scope("/repo", "/repo/crates");
            offsets.push(app.scroll_offset());

            let buffer = render(&app, WIDTH, HEIGHT);

            let (header, footer) = header_and_footer(&buffer);
            assert_eq!(header, ["crates"], "selection {selected}");
            // The footer is still the footer, and no tree row has leaked into
            // it: the tally and the keys are on it, wherever it puts them. Not
            // every line of it: the message line is blank while the app has
            // nothing to say, which is the whole of this walk.
            assert!(
                footer.iter().any(|line| line.contains("unpacted"))
                    && footer.iter().any(|line| line.contains("move")),
                "footer {footer:?} at selection {selected}"
            );
            assert!(
                footer.iter().all(|line| !line.contains("module")),
                "a tree row leaked into footer {footer:?} at selection {selected}"
            );
            // Byte for byte the same rows, whatever the tree between them is
            // showing.
            let chrome = chrome.get_or_insert_with(|| (header.clone(), footer.clone()));
            assert_eq!(*chrome, (header, footer), "selection {selected}");
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
        assert_eq!(keys, KEYS);
        // "p: pact" and not the bare "p", which "up/down" would satisfy.
        for key in [
            "up/down",
            "k/j",
            "PgUp",
            "PgDn",
            "page",
            "g/G",
            "first/last",
            // Named, not left to be discovered: the three keys that change what
            // there is to scroll through.
            "space: collapse",
            "o: pacted",
            "f: files",
            "p: pact",
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
        assert_eq!(row_text(&idle, y), KEYS);
        assert_eq!(row_text(&pacting, y), PACTING_KEYS);
        // Esc means two things, and the line says which one it means now.
        let said = row_text(&pacting, y);
        assert!(said.contains("Esc: cancel"), "{said:?}");
        assert!(!said.contains("Esc/Ctrl-C: quit"), "{said:?}");
        assert!(KEYS.contains("Esc/Ctrl-C: quit"));

        // The line is short enough to survive the narrow terminal the other
        // footer tests draw on, because it is the line that answers "how do I
        // stop this?".
        let narrow = render(&app, 120, height);
        assert_eq!(row_text(&narrow, y), PACTING_KEYS);

        // And the run ending puts today's line back, exactly.
        app.clear_pact_in_flight();
        assert_eq!(row_text(&render(&app, KEYS_WIDTH, height), y), KEYS);
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
        let app = App::from_tree(&fixture::tree()).with_scope(
            Path::new("/repo"),
            Path::new("/repo").join("crates").join("warlock-engine"),
        );

        let buffer = render(&app, WIDTH, HEIGHT);

        // Forward slashes whatever the platform's separator is, because this
        // is the engine's manifest spelling of a module.
        assert_eq!(header_text(&buffer), "crates/warlock-engine");
    }

    #[test]
    fn a_tree_rooted_at_the_repository_root_gets_a_named_header_not_a_blank_one() {
        let app = App::from_tree(&fixture::tree()).with_scope("/repo", "/repo");

        let buffer = render(&app, WIDTH, HEIGHT);

        assert_eq!(header_text(&buffer), "(repository root)");
    }

    #[test]
    fn the_header_sits_above_the_first_tree_row_inside_the_panes_border() {
        let app = App::from_tree(&fixture::tree()).with_scope("/repo", "/repo/crates");

        let buffer = render(&app, WIDTH, HEIGHT);

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
    fn the_panel_draws_a_border_and_nothing_whatever_inside_it_before_the_first_pact() {
        // Both focus states and a tree with something in it: whatever the app is
        // doing, the panel has nothing to say until a pact says it.
        let mut app = App::from_tree(&fixture::tree());
        assert!(!app.has_account());
        for _ in 0..2 {
            let buffer = render(&app, WIDTH, FIXTURE_HEIGHT);

            let panel = areas(buffer.area).panel;
            let inner = pane_inner(panel);
            assert!(inner.width > 0 && inner.height > 0, "{inner:?}");
            for y in inner.y..inner.y + inner.height {
                for x in inner.x..inner.x + inner.width {
                    let cell = &buffer[(x, y)];
                    assert_eq!(
                        cell.symbol(),
                        " ",
                        "the panel drew {:?} at ({x}, {y})",
                        cell.symbol()
                    );
                }
            }
            // And its border really is there, on all four sides, carrying
            // nothing of its own: no title, and no scrollback indicator on a
            // panel with nothing to scroll.
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
            account.record(&Activity::Thinking, at(base, line as u64 + 1));
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
                account.record(&Activity::Thinking, at(base, line as u64 + 1));
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
}
