//! One frame, drawn from the arguments and nothing else.
//!
//! Nothing here reads a clock, a terminal or a global: the instant a pulse is
//! measured against arrives as a parameter, so a frame can be asserted against
//! an in-memory buffer with no tty attached.
//!
//! The measuring entry points — [`tree_height`], [`panel_height`],
//! [`run_header_height`], [`composer_height`], [`panel_width`] and
//! [`hit_test`] — all go back through [`areas`], the same cut [`draw`] uses.
//! That is the point of them: they are how the app asks what fits, and a
//! second copy of the layout arithmetic here would answer for a screen that
//! was never drawn. Sizes it needs are therefore derived, never remembered.

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
use crate::account::{Account, Line as Entry, Voice};
use crate::app::{App, Chrome, Focus, Row, Run, RunHeader};
use crate::colour::{CONVERSATION_COLOUR, FOCUS_COLOUR, GUIDE_COLOUR, SYSTEM_COLOUR, colour_for};
use crate::composer::Composer;
use crate::confirm::{Answer, QuitConfirm};
use crate::panel::Mode;
use crate::prompt::{RecordField, RecordForm, RecordPrompt, ScopeField, ScopePrompt};
// Renamed for the reason `Entry` above is: `Span` here is ratatui's piece of a
// drawn line, and the selection's is the cells of one row the highlight covers;
// its `Window` is the panel's view of the card, which is neither of the
// terminal windows this module knows about.
use crate::selection::{Span as Selected, Window as Viewport, spans_at};
use crate::wrap::shape;

const INDENT: &str = "  ";

const GUIDE: &str = "│";

const GUIDE_BRANCH: &str = "├";

const GUIDE_LAST: &str = "└";

const SELECTION_MARKER: &str = "> ";

const COLLAPSED_MARKER: &str = "+ ";

const EXPANDED_MARKER: &str = "- ";

const NO_MARKER: &str = "  ";

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

const MARK_MARGIN: u16 = 2;

const MARK_MARGIN_ROWS: u16 = 1;

const HEADER_HEIGHT: u16 = 1;

const RUN_HEADER_HEIGHT: u16 = 2;

const PACTING_RUN: &str = "pacting";

const REFRESHING_RUN: &str = "refreshing";

const BAR_FILLED: &str = "█";

const BAR_EMPTY: &str = "░";

const BAR_GAP: &str = " ";

const BAR_MIN_WIDTH: usize = 4;

/// Columns kept for the percentage beside the bar, which is `{:>3}%` — so the
/// bar is the same width at 7% as at 100% instead of shrinking by a column
/// twice on the way up.
const PERCENT_WIDTH: usize = 4;

const HEADER_GAP: &str = " — ";

pub(crate) const PANEL_INDENT: &str = "  ";

const THREAD_TITLE: &str = " thread ";

const BRIEF_THREAD_TITLE: &str = " thread · brief ";

const fn thread_title(mode: Mode) -> &'static str {
    match mode {
        Mode::Chat => THREAD_TITLE,
        Mode::Brief => BRIEF_THREAD_TITLE,
    }
}

pub(crate) const SAID_MARKER: &str = "› ";

pub(crate) const NOTE_MARKER: &str = "· ";

const ELLIPSIS: &str = "…";

const SCROLLBACK_ARROW: &str = "↓";

const LIVE_KEY: &str = "G";

const ROW_KEY: &str = "k/j: row";

const FOLD_KEY: &str = "space: fold";

const PACTS_KEY: &str = "o: pacts";

const FILES_KEY: &str = "f: files";

const PACT_KEY: &str = "p: pact";

const REFRESH_KEY: &str = "r: refresh";

const SCOPE_KEY: &str = "s: scope";

const KEYS: &[&str] = &[
    ROW_KEY,
    FOLD_KEY,
    PACTS_KEY,
    FILES_KEY,
    PACT_KEY,
    REFRESH_KEY,
    SCOPE_KEY,
];

const KEY_GAP: &str = "    ";

const QUIT_KEY: &str = "q/Esc/Ctrl-C: quit";

// The order the footer gives keys up in as the terminal narrows, first named
// first dropped. `QUIT_KEY` is deliberately absent, so it is the one piece that
// survives every width: a reader who cannot see how to leave has no way to find
// out. Every other name on the footer is here: walking the tree first, because
// a reader looking at a tree can guess at how to move through one, and the keys
// that start a pass last, because nothing else on screen says they exist.
const KEY_DROP_ORDER: &[&str] = &[
    ROW_KEY,
    FILES_KEY,
    PACTS_KEY,
    FOLD_KEY,
    SCOPE_KEY,
    REFRESH_KEY,
    PACT_KEY,
];

fn laid_out_keys(width: usize, pieces: &[&str], drop_order: &[&str]) -> String {
    let mut kept: Vec<&str> = pieces.to_vec();
    for name in drop_order {
        if display_width(&kept.join(KEY_GAP)) <= width {
            break;
        }
        if let Some(at) = kept.iter().position(|piece| piece == name) {
            kept.remove(at);
        }
    }

    clipped(&kept.join(KEY_GAP), width)
}

// Cuts to fit and says nothing about it, where `truncated` spends columns on an
// ellipsis to say it cut. The two are not interchangeable: the footer has
// already dropped whole keys to reach this width, so an ellipsis there would
// mark the one loss that is already visible and cost a column to do it.
fn clipped(text: &str, width: usize) -> String {
    if display_width(text) <= width {
        return text.to_owned();
    }

    fitted(text, width).to_owned()
}

fn fitted(text: &str, columns: usize) -> &str {
    let mut taken = 0;
    let mut end = 0;
    for (index, character) in text.char_indices() {
        let next = index + character.len_utf8();
        let width = display_width(&text[index..next]);
        if taken + width > columns {
            break;
        }
        taken += width;
        end = next;
    }

    &text[..end]
}

fn keys_line(width: usize) -> String {
    let mut pieces = KEYS.to_vec();
    pieces.push(QUIT_KEY);
    laid_out_keys(width, &pieces, KEY_DROP_ORDER)
}

const PACTING_KEYS: &[&str] = &[MOVE_KEYS, COLLAPSE_KEY, CANCEL_KEY, PACTING_QUIT_KEY];

const MOVE_KEYS: &str = "up/down k/j: move";

const COLLAPSE_KEY: &str = "space: collapse";

const CANCEL_KEY: &str = "Esc: cancel";

const PACTING_QUIT_KEY: &str = "q/Ctrl-C: quit";

const PACTING_KEY_DROP_ORDER: &[&str] = &[MOVE_KEYS, COLLAPSE_KEY];

fn pacting_keys_line(width: usize) -> String {
    laid_out_keys(width, PACTING_KEYS, PACTING_KEY_DROP_ORDER)
}

const FOOTER_HEIGHT: u16 = 3;

const TREE_PERCENT: u16 = 30;

const TREE_MIN_WIDTH: u16 = 30;

const BORDER_THICKNESS: u16 = 1;

const PULSE_PHASE: Duration = Duration::from_millis(500);

const CONFIRM_QUESTION: &str = "Leave warlock?";

const CONFIRM_YES: &str = " Yes ";

const CONFIRM_NO: &str = " No ";

const CONFIRM_ANSWER_GAP: &str = "    ";

const CONFIRM_MARGIN: u16 = 3;

const CONFIRM_MARGIN_ROWS: u16 = 1;

const CONFIRM_LINES: u16 = 3;

const CONFIRM_HEIGHT: u16 = CONFIRM_LINES + 2 * CONFIRM_MARGIN_ROWS + 2 * BORDER_THICKNESS;

const SCOPE_HEADING: &str = "Scope for ";

const PATH_HEADING: &str = "";

const PATH_RULES: &str = "Enter writes the document, Esc writes nothing";

const SCOPE_MARGIN: u16 = CONFIRM_MARGIN;

const SCOPE_MARGIN_ROWS: u16 = CONFIRM_MARGIN_ROWS;

const SCOPE_CURSOR: &str = " ";

const SCOPE_LINES: u16 = 5;

const SCOPE_HEIGHT: u16 = SCOPE_LINES + 2 * SCOPE_MARGIN_ROWS + 2 * BORDER_THICKNESS;

const RECORD_HEADING: &str = "Record for ";

const RECORD_RULES: &str =
    "Tab moves to the next field, Enter writes the scope and record, Esc writes nothing";

const RECORD_LABEL_GAP: &str = "  ";

/// Heading, directory, a blank, then each of the three fields with the row its
/// own refusal goes in under it, then the rules: [`record_lines`] asserts it
/// draws exactly this many. Every field's rule row is there before any rule is
/// broken, so a refusal moves no field out from under the reader's eye — the
/// same reason [`SCOPE_LINES`] counts a row nothing is usually in.
const RECORD_LINES: u16 = 3 + 2 * 3 + 1;

const RECORD_HEIGHT: u16 = RECORD_LINES + 2 * SCOPE_MARGIN_ROWS + 2 * BORDER_THICKNESS;

/// Reversed rather than a background colour of its own. Colour on the thread
/// already means whose words these are — the model's, the reader's, warlock's —
/// and painting over it would take that away from exactly the rows somebody is
/// reading; swapping what the cell already had keeps every one of those apart
/// under the highlight.
const SELECTED: Style = Style::new().add_modifier(Modifier::REVERSED);

const COMPOSER_CURSOR: &str = SCOPE_CURSOR;

const COMPOSER_MIN_HEIGHT: u16 = 1 + 2 * BORDER_THICKNESS;

#[expect(
    clippy::too_many_arguments,
    reason = "one frame's worth of state, and the point of it is that the binary \
              draws a frame in one call: the four windows that can be over the \
              app are four parameters here rather than four entry points"
)]
pub fn draw(
    frame: &mut Frame<'_>,
    app: &App,
    chrome: &Chrome,
    now: Instant,
    confirm: QuitConfirm,
    scope: &ScopePrompt,
    record: &RecordPrompt,
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

    // Over the finished frame rather than instead of it, each clearing the cells
    // behind it, so what a prompt is answered against is still on screen around
    // it. Nothing below is skipped when one of these is up.
    if let Some(highlighted) = confirm.highlighted() {
        draw_confirm(frame, screen, highlighted);
    }
    if let Some(field) = path.field() {
        draw_scope(frame, screen, field, PATH_HEADING, PATH_RULES);
    }
    if let Some(field) = scope.field() {
        draw_scope(frame, screen, field, SCOPE_HEADING, scope::RULES);
    }
    if let Some(form) = record.form() {
        draw_record(frame, screen, form);
    }
}

struct Areas {
    panel: Rect,
    composer: Option<Rect>,
    tree: Rect,
    footer: Rect,
}

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

fn split_column(column: Rect, composer: Option<&Composer>) -> (Rect, Option<Rect>) {
    let Some(composer) = composer else {
        return (column, None);
    };

    let wanted = composer
        .height(pane_inner(column).width)
        .saturating_add(2 * BORDER_THICKNESS);
    let rows = wanted.min(column.height.saturating_sub(2 * BORDER_THICKNESS));
    // Too little room and the composer is not drawn at all rather than drawn
    // short: below `COMPOSER_MIN_HEIGHT` it is two borders and no line to type
    // on. The panel keeps the whole column, and the `None` is also what stops
    // `draw` from looking for a caret in an area that is not on the frame.
    if rows < COMPOSER_MIN_HEIGHT {
        return (column, None);
    }

    let [panel, field] =
        Layout::vertical([Constraint::Min(0), Constraint::Length(rows)]).areas(column);

    (panel, Some(field))
}

// Exported so the binary can route a keystroke by the same question the frame
// was cut by. `on_screen` below is this one over an `Option`, and `draw` and
// `areas` both go through it: if the two answers ever came from two rules, keys
// would go to a field the reader cannot see.
#[must_use]
pub fn composer_on_screen<'a>(app: &App, composer: &'a Composer) -> Option<&'a Composer> {
    app.panel().composer_showable().then_some(composer)
}

fn on_screen<'a>(app: &App, composer: Option<&'a Composer>) -> Option<&'a Composer> {
    composer.and_then(|field| composer_on_screen(app, field))
}

fn tree_width(width: u16) -> u16 {
    let share = u32::from(width) * u32::from(TREE_PERCENT) / 100;
    let share = u16::try_from(share).unwrap_or(width);

    // Floor first, then ceiling, so the ceiling wins: on a terminal narrower
    // than twice `TREE_MIN_WIDTH` the tree gets less than its minimum rather
    // than more than half the screen. Swapping the two would let a narrow
    // terminal hand the tree everything and leave the panel a bare border.
    share.max(TREE_MIN_WIDTH).min(width / 2)
}

fn pane_inner(area: Rect) -> Rect {
    let inner = pane_block(false).inner(area);
    debug_assert_eq!(
        inner.height,
        area.height.saturating_sub(2 * BORDER_THICKNESS),
        "the pane border no longer costs {BORDER_THICKNESS} row top and bottom"
    );

    inner
}

fn tree_split(tree: Rect) -> [Rect; 2] {
    Layout::vertical([Constraint::Length(HEADER_HEIGHT), Constraint::Min(0)])
        .areas(pane_inner(tree))
}

fn tree_rows_area(tree: Rect) -> Rect {
    tree_split(tree)[1]
}

fn panel_split(panel: Rect, header: Option<&RunHeader>) -> (Option<Rect>, Rect) {
    let inner = pane_inner(panel);
    // A panel with no room to spare loses the run header whole rather than
    // sharing: `Length` would take its rows anyway and leave the account with
    // none, so the reader would watch a progress bar over an empty pane.
    if header.is_none() || inner.height <= RUN_HEADER_HEIGHT {
        return (None, inner);
    }

    let [header, rows] =
        Layout::vertical([Constraint::Length(RUN_HEADER_HEIGHT), Constraint::Min(0)]).areas(inner);

    (Some(header), rows)
}

fn panel_rows_area(panel: Rect, header: Option<&RunHeader>) -> Rect {
    panel_split(panel, header).1
}

#[must_use]
pub fn tree_height(size: Size) -> u16 {
    tree_rows_area(areas(Rect::from(size), None).tree).height
}

#[must_use]
pub fn panel_height(size: Size, composer: Option<&Composer>, header: Option<&RunHeader>) -> u16 {
    panel_rows_area(areas(Rect::from(size), composer).panel, header).height
}

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

#[must_use]
pub fn composer_height(size: Size, composer: Option<&Composer>) -> u16 {
    areas(Rect::from(size), composer)
        .composer
        .map_or(0, |area| area.height)
}

#[must_use]
pub fn panel_width(size: Size) -> u16 {
    pane_inner(areas(Rect::from(size), None).panel).width
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Hit {
    Offscreen,
    Footer,
    Border,
    TreeHeader,
    TreeRow { offset: u16 },
    TreeBelowRows,
    PanelHeader,
    PanelLine { offset: u16, column: u16 },
    Composer,
}

#[must_use]
pub fn hit_test(
    column: u16,
    row: u16,
    size: Size,
    composer: Option<&Composer>,
    header: Option<&RunHeader>,
) -> Hit {
    // Answered by measuring the same `areas` cut the frame was drawn by, not by
    // anything remembered from drawing it, so a click lands on what the reader
    // is looking at. Every test below is against a `pane_inner`, which is why a
    // border belongs to no pane and falls through to `Border` without being
    // asked about. This knows where the panes are and nothing else: what a row
    // offset stands for is the app's to say, and there is no hover.
    //
    // `PanelLine` reports the column beside the row, both counted from the first
    // cell of the rows area the panel drew into, so a pointer event becomes a
    // `selection::Cell` without measuring the layout a second time — a second
    // measurement is what would drift from the frame under the pointer.
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
            column: column.saturating_sub(rows.x),
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Reach {
    Above { rows: u16, column: u16 },
    Inside { column: u16 },
    Below { rows: u16, column: u16 },
}

/// How far past the panel's rows a point is, measured by the same [`areas`] cut
/// [`hit_test`] and [`draw`] go through.
///
/// A pointer held past the card arrives as [`Hit::Footer`], [`Hit::Border`],
/// [`Hit::Composer`] or [`Hit::Offscreen`], none of which says which way it left
/// or by how much, so this answers the question [`Hit`] cannot. The row is
/// counted against the rows area alone: the run header and the composer are not
/// in it, so a point on either is past an edge here even though it is inside the
/// panel's column.
///
/// `column` is clamped into the rows area rather than refused, so a pointer that
/// wandered sideways — onto the border, the tree, past the right edge of the
/// screen — still names the cell of the card nearest it.
#[must_use]
pub fn panel_reach(
    column: u16,
    row: u16,
    size: Size,
    composer: Option<&Composer>,
    header: Option<&RunHeader>,
) -> Reach {
    let rows = panel_rows_area(areas(Rect::from(size), composer).panel, header);
    let column = column
        .saturating_sub(rows.x)
        .min(rows.width.saturating_sub(1));

    // Tested against the rows area's own bounds rather than against its height,
    // so an area with no rows in it — a terminal too short for the panel to hold
    // a line — is all edge and no inside, and no point is answered as a row that
    // was never drawn.
    if row < rows.y {
        Reach::Above {
            rows: rows.y - row,
            column,
        }
    } else if row < rows.bottom() {
        Reach::Inside { column }
    } else {
        Reach::Below {
            rows: (row - rows.bottom()).saturating_add(1),
            column,
        }
    }
}

fn pane_block(focused: bool) -> Block<'static> {
    let style = if focused {
        Style::new().fg(FOCUS_COLOUR).add_modifier(Modifier::BOLD)
    } else {
        Style::new().add_modifier(Modifier::DIM)
    };

    Block::bordered().border_style(style)
}

fn draw_panel(frame: &mut Frame<'_>, area: Rect, app: &App, now: Instant) {
    let panel = app.panel();
    let below = panel.lines_below();
    let conversation = panel.showing_thread();
    let mut block = pane_block(app.focus() == Focus::Panel);
    if conversation {
        block = block.title_top(Line::from(thread_title(panel.mode())).bold());
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

    if !panel.has_content() {
        draw_mark(frame, inner);
        return;
    }

    let rows: Vec<Line<'static>> = panel
        .window(now)
        .iter()
        .map(|line| panel_row(line, inner.width, conversation))
        .collect();
    frame.render_widget(Paragraph::new(rows), inner);

    // Only the conversation is selectable, so only the conversation is drawn
    // with a highlight: the account and the document are read past, not copied
    // out of, and a highlight measured against the thread would sit over
    // whatever text took its place.
    if conversation {
        draw_highlight(frame, inner, app, now);
    }
}

/// Painted over the rows after they are drawn rather than woven into them.
/// [`panel_row`] truncates a row that still does not fit, so text built with a
/// highlight in it would have to be cut by a second rule that agreed with that
/// one; here the cut is [`Rect::intersection`], and a span reaching past the
/// panel stops at its last column instead of on the border or the tree beside
/// it.
fn draw_highlight(frame: &mut Frame<'_>, rows: Rect, app: &App, now: Instant) {
    let panel = app.panel();
    let (Some(selection), Some(thread)) = (app.selection(), panel.thread()) else {
        return;
    };

    // The panel's own offset, width and height — the three numbers the rows
    // above were wrapped and cut by — rather than the area's, which a frame
    // drawn before the panel was told its size would differ from. A highlight
    // measured against a second window is a highlight over text that is not
    // under it.
    let window = Viewport {
        scroll: panel.scroll_offset(),
        width: panel.width(),
        height: panel.height(),
    };
    for span in spans_at(thread, selection, window, now) {
        let cells = highlighted(rows, span);
        frame.buffer_mut().set_style(cells, SELECTED);
    }
}

fn highlighted(rows: Rect, span: Selected) -> Rect {
    let over = Rect {
        x: rows.x.saturating_add(cells(span.from)),
        y: rows.y.saturating_add(cells(span.row)),
        width: cells(span.to.saturating_sub(span.from)),
        height: 1,
    };

    rows.intersection(over)
}

/// Saturating rather than failing: a column count past `u16::MAX` is off every
/// pane there could be, and the intersection above drops it either way.
fn cells(count: usize) -> u16 {
    u16::try_from(count).unwrap_or(u16::MAX)
}

fn draw_run_header(frame: &mut Frame<'_>, area: Rect, header: &RunHeader) {
    // Into the whole of the area it was given, which is the line's row and the
    // blank one under it (see [`RUN_HEADER_HEIGHT`]): one line drawn into two
    // rows leaves the second one as the border cleared it, which is the gap.
    frame.render_widget(
        Paragraph::new(
            Line::from(run_header_line(header, usize::from(area.width)))
                .bold()
                .fg(SYSTEM_COLOUR),
        ),
        area,
    );
}

fn run_header_line(header: &RunHeader, width: usize) -> String {
    let words = format!(
        "{} {} ({}/{})",
        run_word(header.run()),
        header.directory(),
        header.completed(),
        header.total(),
    );

    let room = width
        .saturating_sub(display_width(&words))
        .saturating_sub(display_width(BAR_GAP) * 2)
        .saturating_sub(PERCENT_WIDTH);
    if room < BAR_MIN_WIDTH {
        return truncated(&words, width);
    }

    let (done, total) = fraction(header);
    truncated(
        &format!(
            "{words}{BAR_GAP}{}{BAR_GAP}{:>3}%",
            bar(done, total, room),
            share(done, total, 100),
        ),
        width,
    )
}

const fn run_word(run: Run) -> &'static str {
    match run {
        Run::Pact => PACTING_RUN,
        Run::Refresh => REFRESHING_RUN,
    }
}

/// How far through the run the bar and the percentage both are, in whatever
/// unit is the finest the run has reported.
///
/// Directories while a directory is all the engine has said, and files once it
/// has: a run of five directories working the second of them, four files in of
/// eight, is `1 * 8 + 4` of `5 * 8`. Scaling the whole run by *this*
/// directory's file count is deliberate and is why the denominator moves — the
/// alternative is knowing every directory's file count before the run starts,
/// which costs a walk of the whole subtree to answer a question about a
/// progress bar. What the reader gets instead is a bar that advances inside a
/// directory and is exact at every directory boundary, which is the property
/// that matters: it never goes backwards past one.
///
/// Saturating rather than checked because the products are small — a run is
/// directories times files, both of them counts of things on a disk — and a
/// saturated total still divides.
fn fraction(header: &RunHeader) -> (usize, usize) {
    // What the run has finished, not what it has started: see
    // [`RunHeader::completed`].
    match header.files() {
        Some((position, total)) if total > 0 => (
            header
                .completed()
                .saturating_mul(total)
                .saturating_add(position),
            header.total().saturating_mul(total),
        ),
        _ => (header.completed(), header.total()),
    }
}

/// `done / total` of `whole`, floored, and never past `whole`. The guard is the
/// division: a run counted at zero nodes has nothing to divide by, and the
/// count is a caller's to report, past `total` included.
fn share(done: usize, total: usize, whole: usize) -> usize {
    done.saturating_mul(whole)
        .checked_div(total)
        .unwrap_or(0)
        .min(whole)
}

fn bar(done: usize, total: usize, columns: usize) -> String {
    let filled = share(done, total, columns);

    format!(
        "{}{}",
        BAR_FILLED.repeat(filled),
        BAR_EMPTY.repeat(columns - filled)
    )
}

fn draw_mark(frame: &mut Frame<'_>, inner: Rect) {
    let Some(area) = mark_area(inner) else {
        return;
    };

    let rows: Vec<Line<'static>> = MARK.iter().map(|row| Line::raw(*row)).collect();
    let style = Style::new().add_modifier(Modifier::DIM);
    frame.render_widget(Paragraph::new(rows).style(style), area);
}

fn mark_area(inner: Rect) -> Option<Rect> {
    let width = MARK.iter().copied().map(display_width).max().unwrap_or(0);
    let width = u16::try_from(width).ok()?;
    let height = u16::try_from(MARK.len()).ok()?;

    // All of the mark or none of it. A partial mark reads as a rendering fault
    // rather than as a logo, so a panel that cannot hold the whole thing shows
    // the bare border — which on a narrow terminal is every panel there is.
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

fn draw_composer(frame: &mut Frame<'_>, area: Rect, composer: &Composer, live: bool) {
    let inner = pane_inner(area);
    frame.render_widget(pane_block(live), area);

    let window = composer.window(inner.width, inner.height);
    let mut lines: Vec<Line<'static>> = window
        .rows
        .iter()
        .map(|row| Line::raw(row.clone()))
        .collect();

    // A height of zero leaves no rows and so no row to put a caret on, which is
    // `get_mut`'s answer rather than a check of its own.
    if live && let Some(line) = lines.get_mut(window.row) {
        let row = window.rows[window.row].as_str();
        let (before, at, after) = split_at_column(row, window.column);
        // Past the last character of the row, so the caret is a blank of its
        // own — and only if the row has a column left to spend on it. The row a
        // fold kept its break character on is a column wider than the field,
        // and the drawing truncates it at the pane edge.
        let room = u16::try_from(display_width(row)).unwrap_or(u16::MAX) < inner.width;
        let caret = if at.is_empty() {
            room.then_some(COMPOSER_CURSOR)
        } else {
            Some(at)
        };

        if let Some(caret) = caret {
            *line = Line::from(vec![
                Span::raw(before.to_owned()),
                Span::styled(
                    caret.to_owned(),
                    Style::new().add_modifier(Modifier::REVERSED),
                ),
                Span::raw(after.to_owned()),
            ]);
        }
    }

    frame.render_widget(Paragraph::new(lines), inner);
}

fn split_at_column(row: &str, column: usize) -> (&str, &str, &str) {
    let mut at = row.len();
    let mut taken = 0;
    for (index, character) in row.char_indices() {
        if taken >= column {
            at = index;
            break;
        }
        taken += display_width(&row[index..index + character.len_utf8()]);
    }

    let (before, rest) = row.split_at(at);
    let (at, after) = rest.split_at(rest.chars().next().map_or(0, char::len_utf8));

    (before, at, after)
}

fn scrollback(below: usize) -> String {
    format!(" {SCROLLBACK_ARROW} {below} more ({LIVE_KEY}) ")
}

// One entry, one row. Nothing is wrapped here: a line too long for the panel
// has already been broken into the rows it needs, under its own clock or marker
// (see `crate::wrap`), so the truncation below is the last-resort cut for a row
// that still does not fit and not the way long text is handled.
//
// `conversation` is which card is being drawn, and it is a parameter rather
// than something read off the entry because the same variant means different
// things on different cards: a `Text` row is the model's answer on the thread
// and a paragraph of a file on the document, and only the first of those is the
// model speaking. The colour goes on the whole row, prefix included, so the
// marker in front of a line belongs to whoever the line belongs to rather than
// sitting in a span of its own.
fn panel_row(line: &Entry, width: u16, conversation: bool) -> Line<'static> {
    let shape = shape(line);
    let row = Line::from(truncated(
        &format!("{}{}", shape.prefix, shape.text),
        usize::from(width),
    ));
    let row = if shape.heading { row.bold() } else { row };
    match line.voice() {
        Voice::Model if conversation => row.fg(CONVERSATION_COLOUR),
        // The reader's own words keep the terminal's foreground, and so does a
        // paragraph of a file on the document card: text being read is not the
        // model talking about it.
        Voice::Operator | Voice::Model => row,
        Voice::Warlock => row.fg(SYSTEM_COLOUR),
    }
}

fn truncated(text: &str, width: usize) -> String {
    if display_width(text) <= width {
        return text.to_owned();
    }
    if width == 0 {
        return String::new();
    }

    let budget = width.saturating_sub(display_width(ELLIPSIS));

    format!("{}{ELLIPSIS}", fitted(text, budget))
}

pub(crate) fn display_width(text: &str) -> usize {
    Span::raw(text).width()
}

fn draw_tree_pane(frame: &mut Frame<'_>, area: Rect, app: &App, chrome: &Chrome, now: Instant) {
    frame.render_widget(pane_block(app.focus() == Focus::Tree), area);

    let [header_area, rows_area] = tree_split(area);
    draw_header(frame, header_area, chrome);
    draw_tree(frame, rows_area, app, now);
}

fn draw_header(frame: &mut Frame<'_>, area: Rect, chrome: &Chrome) {
    frame.render_widget(
        Paragraph::new(Line::from(header_line(chrome, usize::from(area.width))).bold()),
        area,
    );
}

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
    // Both or neither, and never a cut one: the sigils are the answer to what
    // this machine is allowed to do, and half of that list read as the whole of
    // it would be worse than not saying. So a header too narrow for both drops
    // the sigils and keeps the identity, rather than truncating the pair.
    if display_width(&both) <= width {
        both
    } else {
        identity.to_owned()
    }
}

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

    // Every unit is one `INDENT` wide: a glyph in the first column and blanks
    // for the rest of the level, whichever glyph it is. Built from `INDENT`
    // rather than from two literal columns so that the guides keep step with the
    // indent if it ever changes width.
    let blanks = " ".repeat(INDENT.chars().count() - 1);

    // Forwards: `stack[level]` is whether the ancestor sitting at that level has
    // rows of its own still to come, which is the vertical the levels above a
    // row are drawn with.
    let mut stack: Vec<bool> = Vec::new();
    let mut prefixes = Vec::with_capacity(last - first);
    for (index, row) in all.iter().enumerate().take(last) {
        stack.resize(row.depth, false);
        if index >= first {
            // The root's own column is never drawn — a depth-0 row carries no
            // guide — so the verticals start one level in, and the row's own
            // level is the corner rather than a vertical.
            let mut prefix = String::new();
            for open in stack.iter().skip(1) {
                prefix.push_str(if *open { GUIDE } else { " " });
                prefix.push_str(&blanks);
            }
            if row.depth > 0 {
                prefix.push_str(if continues[index] {
                    GUIDE_BRANCH
                } else {
                    GUIDE_LAST
                });
                prefix.push_str(&blanks);
            }
            prefixes.push(prefix);
        }
        stack.push(continues[index]);
    }
    prefixes
}

fn pulse_colour(app: &App, now: Instant) -> Option<Color> {
    if !app.is_pacting() {
        return None;
    }

    let fresh = app
        .panel()
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
        keys_line(width)
    })
    .dim();

    let message = Line::from(
        app.pact_line()
            .unwrap_or_else(|| app.message().unwrap_or_default().to_owned()),
    )
    .dim();
    frame.render_widget(Paragraph::new(vec![Line::from(tally), keys, message]), area);
}

fn footer_text_area(footer: Rect) -> Rect {
    Rect {
        x: footer.x + BORDER_THICKNESS,
        width: footer.width.saturating_sub(2 * BORDER_THICKNESS),
        ..footer
    }
}

const fn noun(state: NodeState) -> &'static str {
    match state {
        NodeState::Unpacted => "unpacted",
        NodeState::PactedStale => "stale",
        NodeState::PactedFresh => "fresh",
    }
}

fn draw_confirm(frame: &mut Frame<'_>, screen: Rect, highlighted: Answer) {
    draw_over(
        frame,
        confirm_area(screen),
        Padding::symmetric(CONFIRM_MARGIN, CONFIRM_MARGIN_ROWS),
        vec![
            Line::from(CONFIRM_QUESTION).centered(),
            Line::default(),
            answers_line(highlighted),
        ],
    );
}

fn draw_over(frame: &mut Frame<'_>, area: Rect, padding: Padding, lines: Vec<Line<'_>>) {
    let block = Block::bordered().padding(padding);
    let inner = block.inner(area);

    frame.render_widget(Clear, area);
    frame.render_widget(block, area);
    frame.render_widget(Paragraph::new(lines), inner);
}

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

fn confirm_area(screen: Rect) -> Rect {
    centred(screen, confirm_size())
}

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

fn confirm_size() -> Size {
    let answers =
        display_width(CONFIRM_YES) + display_width(CONFIRM_ANSWER_GAP) + display_width(CONFIRM_NO);
    let widest = display_width(CONFIRM_QUESTION).max(answers);

    Size::new(padded_width(widest, CONFIRM_MARGIN), CONFIRM_HEIGHT)
}

fn padded_width(widest: usize, margin: u16) -> u16 {
    u16::try_from(widest)
        .unwrap_or(u16::MAX)
        .saturating_add(2 * margin)
        .saturating_add(2 * BORDER_THICKNESS)
}

fn draw_scope(frame: &mut Frame<'_>, screen: Rect, field: &ScopeField, heading: &str, rules: &str) {
    draw_over(
        frame,
        centred(screen, scope_size(field, heading, rules)),
        Padding::symmetric(SCOPE_MARGIN, SCOPE_MARGIN_ROWS),
        scope_lines(field, heading, rules),
    );
}

fn scope_lines<'a>(field: &'a ScopeField, heading: &'a str, rules: &'a str) -> Vec<Line<'a>> {
    vec![
        Line::from(vec![
            Span::raw(heading),
            Span::raw(field.directory()).bold(),
        ]),
        Line::default(),
        Line::from(typed_spans(field)),
        Line::from(field.rule().unwrap_or_default()),
        Line::from(rules).dim(),
    ]
}

/// The text with the caret on the character the cursor names, and past the last
/// one a blank of its own. Both sizers leave the column for that blank whether
/// or not the cursor is in the text, so a window does not change width as the
/// cursor walks a field.
fn typed_spans(field: &ScopeField) -> Vec<Span<'_>> {
    let (before, rest) = field.text().split_at(field.cursor());
    let (at, after) = match rest.chars().next() {
        Some(character) => rest.split_at(character.len_utf8()),
        None => (SCOPE_CURSOR, ""),
    };

    vec![
        Span::raw(before),
        Span::styled(at, Style::new().add_modifier(Modifier::REVERSED)),
        Span::raw(after),
    ]
}

fn scope_size(field: &ScopeField, heading: &str, rules: &str) -> Size {
    let heading = display_width(heading) + display_width(field.directory());
    let typed = display_width(field.text()) + display_width(SCOPE_CURSOR);
    let widest = heading
        .max(typed)
        .max(field.rule().map_or(0, display_width))
        .max(display_width(rules));

    Size::new(padded_width(widest, SCOPE_MARGIN), SCOPE_HEIGHT)
}

// A sibling of `draw_scope` rather than a third heading-and-rules argument to
// it: three fields, three refusal rows and a column of labels is a different
// shape of window, and the two share what they are actually the same about —
// `draw_over`, `centred`, `padded_width` and the margins.
fn draw_record(frame: &mut Frame<'_>, screen: Rect, form: &RecordForm) {
    draw_over(
        frame,
        centred(screen, record_size(form)),
        Padding::symmetric(SCOPE_MARGIN, SCOPE_MARGIN_ROWS),
        record_lines(form),
    );
}

fn record_lines(form: &RecordForm) -> Vec<Line<'_>> {
    let mut lines = vec![
        Line::from(vec![
            Span::raw(RECORD_HEADING),
            Span::raw(form.scope()).bold(),
        ]),
        Line::from(form.path()).dim(),
        Line::default(),
    ];
    for which in RecordField::ALL {
        let field = form.field(which);
        let mut spans = vec![Span::raw(label_column(which)).dim()];
        // The caret goes in the focused field and nowhere else: three carets
        // would say three fields were taking the next character.
        if which == form.focus() {
            spans.extend(typed_spans(field));
        } else {
            spans.push(Span::raw(field.text()));
        }
        lines.push(Line::from(spans));
        lines.push(Line::from(field.rule().unwrap_or_default()));
    }
    lines.push(Line::from(RECORD_RULES).dim());

    debug_assert_eq!(
        u16::try_from(lines.len()).unwrap_or(u16::MAX),
        RECORD_LINES,
        "the record window is no longer {RECORD_LINES} lines tall"
    );

    lines
}

fn label_column(which: RecordField) -> String {
    let width = label_width();

    format!("{:<width$}{RECORD_LABEL_GAP}", which.name())
}

fn label_width() -> usize {
    RecordField::ALL
        .into_iter()
        .map(|which| display_width(which.name()))
        .max()
        .unwrap_or(0)
}

fn record_size(form: &RecordForm) -> Size {
    let heading = display_width(RECORD_HEADING) + display_width(form.scope());
    let label = label_width() + display_width(RECORD_LABEL_GAP);
    // Every field is measured with a cursor column, focused or not, so that
    // tabbing between them moves nothing sideways — the window is the width of
    // the widest thing it could say rather than of what it is saying now.
    let widest = RecordField::ALL.into_iter().fold(
        heading
            .max(display_width(form.path()))
            .max(display_width(RECORD_RULES)),
        |widest, which| {
            let field = form.field(which);
            widest
                .max(label + display_width(field.text()) + display_width(SCOPE_CURSOR))
                .max(field.rule().map_or(0, display_width))
        },
    );

    Size::new(padded_width(widest, SCOPE_MARGIN), RECORD_HEIGHT)
}

#[cfg(test)]
#[path = "tests/ui.rs"]
mod tests;
