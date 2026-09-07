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
use crate::account::{Account, Line as Entry};
use crate::app::{App, Chrome, Focus, Row, Run, RunHeader};
use crate::colour::{FOCUS_COLOUR, GUIDE_COLOUR, colour_for};
use crate::composer::Composer;
use crate::confirm::{Answer, QuitConfirm};
use crate::panel::Mode;
use crate::prompt::{ScopeField, ScopePrompt};
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

const PAGE_KEYS: &str = "PgUp/PgDn";

const ENDS_KEY: &str = "g/G: ends";

const FOLD_KEY: &str = "space: fold";

const PACTS_KEY: &str = "o: pacts";

const FILES_KEY: &str = "f: files";

const PACT_KEY: &str = "p: pact";

const REFRESH_KEY: &str = "r: refresh";

const SCOPE_KEY: &str = "s: scope";

const FOCUS_KEY: &str = "Tab: focus";

const COMPOSE_KEYS: &str = "Enter/Alt+Enter: send/newline";

const LEAVE_KEY: &str = "Esc: leave, draft kept";

const COMMAND_KEY: &str = "/: command";

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

const KEY_GAP: &str = "    ";

const MOUSE_OFF_KEY: &str = "m: mouse off";

const MOUSE_ON_KEY: &str = "m: mouse on";

const QUIT_KEY: &str = "q/Esc/Ctrl-C: quit";

const VIEW_KEY: &str = "v: view";

const EDIT_KEY: &str = "e: edit";

// The order the footer gives keys up in as the terminal narrows, first named
// first dropped. `QUIT_KEY` is deliberately absent, so it is the one piece that
// survives every width: a reader who cannot see how to leave has no way to find
// out. Names not on the footer at the time — the mouse key that is not the
// current one, and keys no footer carries yet — are skipped rather than being
// an error, which is what lets this be one list instead of one per key set.
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

// Cuts to fit and says nothing about it, where `truncated` spends columns on an
// ellipsis to say it cut. The two are not interchangeable: the footer has
// already dropped whole keys to reach this width, so an ellipsis there would
// mark the one loss that is already visible and cost a column to do it.
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

const COMPOSER_CURSOR: &str = SCOPE_CURSOR;

const COMPOSER_MIN_HEIGHT: u16 = 1 + 2 * BORDER_THICKNESS;

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

fn tree_rows_area(tree: Rect) -> Rect {
    let [_header, rows] = Layout::vertical([Constraint::Length(HEADER_HEIGHT), Constraint::Min(0)])
        .areas(pane_inner(tree));

    rows
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
    TreeRow {
        offset: u16,
    },
    TreeBelowRows,
    PanelHeader,
    PanelLine {
        offset: u16,
    },
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
    // asked about. This knows the three numbers and nothing else: what a row
    // offset stands for is the app's to say, and there is no hover.
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

fn pane_block(focused: bool) -> Block<'static> {
    let style = if focused {
        Style::new().fg(FOCUS_COLOUR).add_modifier(Modifier::BOLD)
    } else {
        Style::new().add_modifier(Modifier::DIM)
    };

    Block::bordered().border_style(style)
}

fn draw_panel(frame: &mut Frame<'_>, area: Rect, app: &App, now: Instant) {
    let below = app.panel().lines_below();
    let mut block = pane_block(app.focus() == Focus::Panel);
    if app.panel().showing_thread() {
        block = block.title_top(Line::from(thread_title(app.panel().mode())).bold());
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

    if !app.panel().has_content() {
        draw_mark(frame, inner);
        return;
    }

    let rows: Vec<Line<'static>> = app
        .panel()
        .window(now)
        .iter()
        .map(|line| panel_row(line, inner.width))
        .collect();
    frame.render_widget(Paragraph::new(rows), inner);
}

fn draw_run_header(frame: &mut Frame<'_>, area: Rect, header: &RunHeader) {
    // Into the whole of the area it was given, which is the line's row and the
    // blank one under it (see [`RUN_HEADER_HEIGHT`]): one line drawn into two
    // rows leaves the second one as the border cleared it, which is the gap.
    frame.render_widget(
        Paragraph::new(Line::from(run_header_line(header, usize::from(area.width))).bold()),
        area,
    );
}

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

const fn run_word(run: Run) -> &'static str {
    match run {
        Run::Pact => PACTING_RUN,
        Run::Refresh => REFRESHING_RUN,
    }
}

fn bar(header: &RunHeader, columns: usize) -> String {
    // Both guards feed the subtraction below, which would panic without them: a
    // run counted at zero nodes has nothing to divide by, and `position` is the
    // furthest node reached, which a caller is free to report past `total`.
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
fn panel_row(line: &Entry, width: u16) -> Line<'static> {
    let shape = shape(line);
    let row = Line::from(truncated(
        &format!("{}{}", shape.prefix, shape.text),
        usize::from(width),
    ));
    if shape.heading { row.bold() } else { row }
}

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

pub(crate) fn display_width(text: &str) -> usize {
    Span::raw(text).width()
}

fn draw_tree_pane(frame: &mut Frame<'_>, area: Rect, app: &App, chrome: &Chrome, now: Instant) {
    let inner = pane_inner(area);
    frame.render_widget(pane_block(app.focus() == Focus::Tree), area);

    let [header_area, rows_area] =
        Layout::vertical([Constraint::Length(HEADER_HEIGHT), Constraint::Min(0)]).areas(inner);
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
    let width = u16::try_from(widest)
        .unwrap_or(u16::MAX)
        .saturating_add(2 * CONFIRM_MARGIN)
        .saturating_add(2 * BORDER_THICKNESS);

    Size::new(width, CONFIRM_HEIGHT)
}

fn draw_scope(frame: &mut Frame<'_>, screen: Rect, field: &ScopeField, heading: &str, rules: &str) {
    let area = centred(screen, scope_size(field, heading, rules));
    let block = Block::bordered().padding(Padding::symmetric(SCOPE_MARGIN, SCOPE_MARGIN_ROWS));
    let inner = block.inner(area);

    frame.render_widget(Clear, area);
    frame.render_widget(block, area);
    frame.render_widget(Paragraph::new(scope_lines(field, heading, rules)), inner);
}

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
        CANCEL_KEY, COLLAPSE_KEY, COMMAND_KEY, COMPOSE_KEYS, COMPOSER_CURSOR, COMPOSER_MIN_HEIGHT,
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

    const MANY: usize = 20;

    const WIDTH: u16 = 120;
    const HEIGHT: u16 = 10;

    const CHROME_HEIGHT: u16 = FOOTER_HEIGHT + 2 * BORDER_THICKNESS + HEADER_HEIGHT;

    const KEYS_WIDTH: u16 = 240;

    const EIGHTY_COLUMNS: u16 = 80;

    const FILES_HEIGHT: u16 = 20;

    const FIXTURE_HEIGHT: u16 = 16;

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

    const MARK_WIDTH: u16 = 151;

    const BELOW_MARK_WIDTH: u16 = MARK_WIDTH - 1;

    const MARK_HEIGHT: u16 = 14;

    const BELOW_MARK_HEIGHT: u16 = MARK_HEIGHT - 1;

    const MARK_ROOM_WIDTH: u16 = 170;

    const MARK_ROOM_HEIGHT: u16 = 20;

    const STANDARD_WIDTH: u16 = 80;

    const HELD_WIDTH: u16 = 160;

    const NARROW_WIDTH: u16 = 40;

    const PHASE: Duration = Duration::from_millis(500);
    const CYCLE: Duration = Duration::from_secs(1);

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

    const SCOPED_NAME: &str = "warlock-terminal-ui";
    const SCOPE_TEAM: &str = "tui-team";
    const SCOPED_LABEL: &str = "warlock-terminal-ui (tui-team)";

    const SCOPE_ROOM_WIDTH: u16 = 160;

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

    fn terminal_width_for(rows_width: u16) -> u16 {
        (2 * TREE_MIN_WIDTH..=400)
            .find(|width| {
                tree_rows_area(areas(Rect::new(0, 0, *width, HEIGHT), None).tree).width
                    == rows_width
            })
            .unwrap_or_else(|| panic!("no terminal gives the tree's rows {rows_width} columns"))
    }

    fn tall_app(selected: usize) -> App {
        let mut app = App::from_rows(many_rows(MANY));
        app.set_viewport_height(tree_height(Size::new(WIDTH, HEIGHT)));
        while app.selected() < selected {
            app.select_next();
        }
        app
    }

    fn at(base: Instant, seconds: u64) -> Instant {
        base + Duration::from_secs(seconds)
    }

    fn numbered(line: usize) -> Activity {
        Activity::Tool {
            name: "Read".to_owned(),
            detail: Some(format!("line {line}")),
        }
    }

    fn pacting_app(base: Instant, width: u16, height: u16) -> App {
        let mut app = App::from_tree(&fixture::tree());
        app.set_viewport_height(tree_height(Size::new(width, height)));
        app.panel_mut()
            .set_height(panel_height(Size::new(width, height), None, None));
        app.panel_mut()
            .set_width(panel_width(Size::new(width, height)));
        app.start_account(base);
        app
    }

    fn panel_area(buffer: &Buffer) -> Rect {
        pane_inner(areas(buffer.area, None).panel)
    }

    fn panel_rows(buffer: &Buffer) -> Vec<String> {
        let area = panel_area(buffer);
        (0..area.height)
            .map(|index| text_in(buffer, area, area.y + index))
            .collect()
    }

    fn panel_bottom_edge(buffer: &Buffer) -> String {
        let panel = areas(buffer.area, None).panel;

        text_in(buffer, panel, panel.y + panel.height - 1)
    }

    fn panel_top_edge(buffer: &Buffer) -> String {
        let panel = areas(buffer.area, None).panel;

        text_in(buffer, panel, panel.y)
    }

    fn rows_area(buffer: &Buffer) -> Rect {
        tree_rows_area(areas(buffer.area, None).tree)
    }

    fn header_area(buffer: &Buffer) -> Rect {
        let inner = pane_inner(areas(buffer.area, None).tree);

        Rect {
            height: HEADER_HEIGHT.min(inner.height),
            ..inner
        }
    }

    fn tree_rows(buffer: &Buffer) -> Vec<String> {
        let area = rows_area(buffer);
        (0..area.height)
            .map(|index| text_in(buffer, area, area.y + index))
            .collect()
    }

    fn tree_row(buffer: &Buffer, index: u16) -> String {
        let area = rows_area(buffer);

        text_in(buffer, area, area.y + index)
    }

    fn held_chrome(sigils: Sigils) -> Chrome {
        Chrome::of("/repo", "/repo/crates").with_sigils(sigils)
    }

    fn header_app() -> App {
        App::from_tree(&fixture::tree())
    }

    fn header_text(buffer: &Buffer) -> String {
        let area = header_area(buffer);

        text_in(buffer, area, area.y)
    }

    fn header_and_footer(buffer: &Buffer) -> (Vec<String>, Vec<String>) {
        let height = buffer.area.height;
        (
            vec![header_text(buffer)],
            (height - FOOTER_HEIGHT..height)
                .map(|y| row_text(buffer, y))
                .collect(),
        )
    }

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

    fn render(app: &App, width: u16, height: u16) -> Buffer {
        render_at(app, width, height, Instant::now())
    }

    fn render_at(app: &App, width: u16, height: u16, now: Instant) -> Buffer {
        render_confirm(app, width, height, now, QuitConfirm::Closed)
    }

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

    fn row_text(buffer: &Buffer, y: u16) -> String {
        text_in(buffer, buffer.area, y)
    }

    fn text_in(buffer: &Buffer, area: Rect, y: u16) -> String {
        let text: String = (area.x..area.x + area.width)
            .map(|x| buffer[(x, y)].symbol())
            .collect();
        text.trim_end().to_string()
    }

    fn footer_line(buffer: &Buffer, line: u16) -> String {
        let band = Rect {
            x: buffer.area.x,
            y: buffer.area.height - FOOTER_HEIGHT,
            width: buffer.area.width,
            height: FOOTER_HEIGHT,
        };

        text_in(buffer, footer_text_area(band), band.y + line)
    }

    fn footer_width(width: u16) -> usize {
        usize::from(footer_text_area(Rect::new(0, 0, width, FOOTER_HEIGHT)).width)
    }

    fn rows_text(buffer: &Buffer) -> Vec<String> {
        (0..buffer.area.height)
            .map(|y| row_text(buffer, y))
            .collect()
    }

    const GUIDE_GLYPHS: [&str; 3] = [GUIDE, GUIDE_BRANCH, GUIDE_LAST];

    fn first_glyph_colour(buffer: &Buffer, index: u16) -> Color {
        let area = rows_area(buffer);
        let gutter = u16::try_from(SELECTION_MARKER.chars().count()).expect("a two-char marker");
        (area.x + gutter..area.x + area.width)
            .map(|x| &buffer[(x, area.y + index)])
            .find(|cell| !cell.symbol().trim().is_empty() && !GUIDE_GLYPHS.contains(&cell.symbol()))
            .expect("row has a glyph on it")
            .fg
    }

    fn column_of(line: &str, needle: &str) -> usize {
        let byte = line
            .find(needle)
            .unwrap_or_else(|| panic!("{needle:?} is not on {line:?}"));

        line[..byte].chars().count()
    }

    fn guide_columns(buffer: &Buffer, index: u16) -> Vec<u16> {
        let area = rows_area(buffer);
        (0..area.width)
            .filter(|column| {
                GUIDE_GLYPHS.contains(&buffer[(area.x + column, area.y + index)].symbol())
            })
            .collect()
    }

    fn row_index(app: &App, path: &str) -> u16 {
        let index = app
            .rows()
            .iter()
            .position(|row| row.path.as_path() == Path::new(path))
            .unwrap_or_else(|| panic!("{path} is not a row of this app"));

        u16::try_from(index).expect("the fixture tree is small")
    }

    fn select(mut app: App, path: &str) -> App {
        while app.selected_row().expect("the app has rows").path != Path::new(path) {
            let before = app.selected();
            app.select_next();
            assert_ne!(app.selected(), before, "no row for {path}");
        }
        app
    }

    fn highlighted_rows(buffer: &Buffer) -> Vec<u16> {
        let x = rows_area(buffer).x;
        (0..buffer.area.height)
            .filter(|&y| buffer[(x, y)].modifier.contains(Modifier::REVERSED))
            .collect()
    }

    fn mark_width() -> usize {
        MARK.iter()
            .copied()
            .map(display_width)
            .max()
            .expect("the art has rows")
    }

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
        app.panel_mut()
            .account_mut()
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
        app.panel_mut()
            .account_mut()
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
        app.panel_mut()
            .account_mut()
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

        app.panel_mut()
            .account_mut()
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
            app.panel()
                .account()
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
        app.panel_mut()
            .account_mut()
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
        let account = app.panel_mut().account_mut().expect("a pact has started");
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
        assert!(!app.panel().has_account());
        assert!(!app.panel().has_document());
        assert!(!app.panel().has_content());
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
        assert!(app.panel().has_account());
        assert!(
            mark_area(panel_area(&render(&app, MARK_ROOM_WIDTH, MARK_ROOM_HEIGHT))).is_some(),
            "this size has room for the mark"
        );

        // A pact under way with nothing recorded yet is still a pact: the panel
        // is empty, and empty is not the same as free. The run took the panel
        // because the card it took it from had nothing on it (see
        // [`App::start_account`]), so what is drawn is the account's border.
        assert!(app.panel().window(at(base, 1)).is_empty());
        assert!(!app.panel().showing_thread(), "the run took the empty card");
        assert_bare_panel(&render_at(
            &app,
            MARK_ROOM_WIDTH,
            MARK_ROOM_HEIGHT,
            at(base, 1),
        ));

        // With lines in it, the account has the rows and the mark none of them.
        let account = app.panel_mut().account_mut().expect("a pact has started");
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
        assert!(app.panel().lines_below() > 0);
        assert!(!panel_rows(&scrolled)[0].is_empty());
        assert_no_mark(&scrolled);
        app.toggle_focus();

        // And a second pact starts a fresh account with no lines in it, which
        // is the first case again and still not a screen for the mark.
        app.start_account(at(base, 100));
        assert!(app.panel().window(at(base, 101)).is_empty());
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
        let account = app.panel_mut().account_mut().expect("a pact has started");
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
            .take(app.panel().window(at(base, 40)).len())
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
        assert_eq!(
            drawn.len(),
            app.panel().account().expect("a pact").line_count()
        );
        for (index, row) in panel_rows(&buffer).iter().enumerate().skip(drawn.len()) {
            assert_eq!(row, "", "panel row {index} should be blank");
        }
    }

    #[test]
    fn the_newest_lines_clock_counts_up_between_frames_with_no_event_arriving() {
        let base = Instant::now();
        let mut app = pacting_app(base, WIDTH, FIXTURE_HEIGHT);
        let account = app.panel_mut().account_mut().expect("a pact has started");
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
        let account = app.panel_mut().account_mut().expect("a pact has started");
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

    fn viewing_app(width: u16, height: u16, cut: bool) -> App {
        let mut app = App::from_tree(&fixture::tree());
        app.set_viewport_height(tree_height(Size::new(width, height)));
        app.panel_mut()
            .set_height(panel_height(Size::new(width, height), None, None));
        app.panel_mut()
            .set_width(panel_width(Size::new(width, height)));
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
        assert_eq!(app.panel().window(now).len(), 5);
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

    fn two_card_app(base: Instant, width: u16, height: u16) -> App {
        let mut app = pacting_app(base, width, height);
        let account = app.panel_mut().account_mut().expect("a pact has started");
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

    const SAME_WORDS: &str = "crates/engine";

    const QUESTION: &str = "what does the engine do?";
    const ANSWER: &str = "It walks the tree and writes what it finds.";

    #[test]
    fn the_thread_and_the_account_do_not_draw_alike_with_the_same_words_on_them() {
        let base = Instant::now();
        let mut app = pacting_app(base, WIDTH, FIXTURE_HEIGHT);
        let account = app.panel_mut().account_mut().expect("a pact has started");
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
        app.panel_mut().start_turn(SAME_WORDS, base);
        app.panel_mut().record_turn(&Activity::Thinking, base);
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
        app.panel_mut().start_turn(QUESTION, base);
        for line in 0..height * 3 {
            app.panel_mut()
                .record_turn(&numbered(line), at(base, line as u64 + 1));
        }

        // Following the newest line, so what is on screen is work lines: the one
        // marked row is above the window and there is no prose yet either.
        let buffer = render_at(&app, WIDTH, HEIGHT, at(base, 99));
        let drawn = panel_rows(&buffer);
        assert!(app.panel().lines_below() == 0 && drawn.len() == height);
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

    #[test]
    fn the_thread_title_says_which_register_the_conversation_is_in() {
        let base = Instant::now();
        let mut app = pacting_app(base, WIDTH, FIXTURE_HEIGHT);
        app.panel_mut().start_turn(QUESTION, base);
        app.panel_mut().answer_turn(ANSWER, at(base, 1));

        // Chat is where a conversation starts, and the edge says only the card.
        assert_eq!(app.panel().mode(), Mode::Chat);
        let chat = render_at(&app, WIDTH, FIXTURE_HEIGHT, at(base, 2));
        let chat_edge = panel_top_edge(&chat);
        assert!(chat_edge.contains(THREAD_TITLE.trim()), "{chat_edge:?}");
        assert!(!chat_edge.contains("brief"), "{chat_edge:?}");

        // The mode changed, so the same card at the same width says so.
        assert!(
            app.panel_mut().set_mode(Mode::Brief),
            "chat to brief is a change"
        );
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
        assert!(
            app.panel().showing_thread(),
            "the mode moved the card showing"
        );
        app.swap_card();
        let account = panel_top_edge(&render_at(&app, WIDTH, FIXTURE_HEIGHT, at(base, 2)));
        assert!(!account.contains(THREAD_TITLE.trim()), "{account:?}");
        assert!(!account.contains("brief"), "{account:?}");
        assert_eq!(
            app.panel().mode(),
            Mode::Brief,
            "the swap changed the register"
        );

        // Setting the mode it is already in changes nothing, and is said to
        // change nothing: that is what a re-sent instruction is told by.
        assert!(
            !app.panel_mut().set_mode(Mode::Brief),
            "brief to brief is no change"
        );
        assert!(
            app.panel_mut().set_mode(Mode::Chat),
            "brief to chat is a change"
        );
    }

    #[test]
    fn no_model_prose_is_ever_drawn_on_the_account_card() {
        let base = Instant::now();
        let mut app = pacting_app(base, WIDTH, FIXTURE_HEIGHT);
        let account = app.panel_mut().account_mut().expect("a pact has started");
        account.open_section(SAME_WORDS, base);
        account.record(&Activity::Thinking, at(base, 1));

        // A turn asked and answered while the run is still going, so both cards
        // are being written and the answer is on one of them.
        app.panel_mut().start_turn(QUESTION, at(base, 2));
        app.panel_mut()
            .record_turn(&Activity::Thinking, at(base, 3));
        app.panel_mut().answer_turn(ANSWER, at(base, 4));

        // The thread came to the front when the question was asked: the answer
        // is there, whole, in the model's own words.
        assert!(app.panel().showing_thread());
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
        assert!(!app.panel().showing_thread());
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
        let account = app.panel_mut().account_mut().expect("a pact has started");
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
        app.panel_mut().start_turn(QUESTION, base);
        app.panel_mut().answer_turn(ANSWER, at(base, 1));

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

    const ROW_WIDTH: u16 = 24;

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
        app.panel_mut().set_height(panel_height(size, None, None));
        app.panel_mut().set_width(panel_width(size));

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
        app.panel_mut().start_turn(QUESTION, base);
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
        assert!(app.panel().showing_thread());
        assert_no_mark(&swapped);
        assert!(panel_top_edge(&swapped).contains(THREAD_TITLE.trim()));
        assert!(app.message().is_some(), "the refusal says what would help");

        // And a pact starting under the thread does not move the panel or the
        // mark: the account it fills is the card behind.
        app.start_account(at(base, 2));
        let running = render_at(&app, MARK_ROOM_WIDTH, MARK_ROOM_HEIGHT, at(base, 3));
        assert!(app.panel().showing_thread());
        assert_no_mark(&running);

        // Now that a pact has filled it the account is worth a press again, and
        // what it draws is the run rather than the mark.
        app.swap_card();
        let account = render_at(&app, MARK_ROOM_WIDTH, MARK_ROOM_HEIGHT, at(base, 3));
        assert!(!app.panel().showing_thread());
        assert_no_mark(&account);
    }

    #[test]
    fn the_scrollback_and_the_run_header_reach_the_thread_as_they_reach_the_account() {
        let base = Instant::now();
        let height = usize::from(panel_height(Size::new(WIDTH, HEIGHT), None, None));
        let mut app = pacting_app(base, WIDTH, HEIGHT);
        app.panel_mut().start_turn(QUESTION, base);
        for line in 0..height * 3 {
            app.panel_mut()
                .record_turn(&numbered(line), at(base, line as u64 + 1));
        }

        // Following the newest line: nothing below the view, so the edge says
        // nothing about scrollback and names the card and nothing else.
        let live = render_at(&app, WIDTH, HEIGHT, at(base, 99));
        assert_eq!(app.panel().lines_below(), 0);
        assert!(!panel_bottom_edge(&live).contains(SCROLLBACK_ARROW));

        // Scrolled back by the ordinary movement keys, the indicator counts the
        // thread's rows on the bottom edge while the top edge goes on naming it.
        app.toggle_focus();
        app.select_first();
        let scrolled = render_at(&app, WIDTH, HEIGHT, at(base, 99));
        let below = app.panel().lines_below();
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
        let account = app.panel_mut().account_mut().expect("a pact has started");
        account.open_section("crates/engine", base);
        for line in 0..height * 3 {
            account.record(&numbered(line), at(base, line as u64 + 1));
        }
        app.toggle_focus();
        app.select_first();
        let parked = app.panel().lines_below();
        assert!(parked > 0, "the account should be scrolled back");
        app.show_document(["# The engine", "It walks the tree."], false);

        // The document is showing and the whole of it is on screen, so the edge
        // says nothing — whatever is under the account behind it.
        let showing_document = render_at(&app, WIDTH, HEIGHT, at(base, 99));
        assert_eq!(app.panel().lines_below(), 0);
        let edge = panel_bottom_edge(&showing_document);
        assert!(!edge.contains(SCROLLBACK_ARROW), "{edge:?}");
        assert!(!edge.contains("more"), "{edge:?}");

        // Swap round to the account, and the indicator comes back counting its
        // lines: the number on the edge is the showing card's and follows it
        // across.
        app.swap_card();
        app.swap_card();
        let showing_account = render_at(&app, WIDTH, HEIGHT, at(base, 99));
        assert_eq!(app.panel().lines_below(), parked);
        let edge = panel_bottom_edge(&showing_account);
        assert!(
            edge.contains(&format!("{SCROLLBACK_ARROW} {parked} more ({LIVE_KEY})")),
            "{edge:?}"
        );

        // The other way round, on a fresh app: a document longer than the panel
        // at its first line, over an account that is following its newest one.
        let mut app = pacting_app(base, WIDTH, HEIGHT);
        let account = app.panel_mut().account_mut().expect("a pact has started");
        account.open_section("crates/engine", base);
        for line in 0..height * 3 {
            account.record(&numbered(line), at(base, line as u64 + 1));
        }
        app.show_document((0..height * 3).map(|line| format!("line {line}")), false);

        let below = app.panel().lines_below();
        assert_eq!(below, height * 3 - height);
        let edge = panel_bottom_edge(&render_at(&app, WIDTH, HEIGHT, at(base, 99)));
        assert!(
            edge.contains(&format!("{SCROLLBACK_ARROW} {below} more ({LIVE_KEY})")),
            "{edge:?}"
        );

        // And swapping to an account that is following takes it away again, even
        // though the card left behind has most of itself below the window.
        app.swap_card();
        assert_eq!(app.panel().lines_below(), 0);
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
        let account = app.panel_mut().account_mut().expect("a pact has started");
        account.open_section("crates/engine", base);
        for line in 0..height * 3 {
            account.record(&numbered(line), at(base, line as u64 + 1));
        }

        // Following the newest line: there is nothing below the view, so the
        // edge says nothing.
        let live = render_at(&app, WIDTH, HEIGHT, at(base, 99));
        assert_eq!(app.panel().lines_below(), 0);
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

        let below = app.panel().lines_below();
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
        assert_eq!(app.panel().lines_below(), 0);
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
            let account = app.panel_mut().account_mut().expect("a pact has started");
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

    const RUNNING_ON: &str = "warlock/crates/engine";

    const RUNNING_LABEL: &str = RUNNING_ON;

    fn measure_panel(app: &mut App, width: u16, height: u16) {
        let header = app.run_header();
        app.panel_mut().set_height(panel_height(
            Size::new(width, height),
            None,
            header.as_ref(),
        ));
    }

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

    fn fill_account(app: &mut App, base: Instant, lines: usize) {
        let account = app.panel_mut().account_mut().expect("a pact has started");
        account.open_section(RUNNING_LABEL, base);
        for line in 0..lines {
            account.record(&numbered(line), at(base, line as u64 + 1));
        }
    }

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
        assert_eq!(app.panel().lines_below(), 0, "the account is following");

        // Scrolled back through the account by the ordinary movement keys.
        app.toggle_focus();
        app.select_first();
        let scrolled = panel_rows(&render_at(&app, WIDTH, HEIGHT, at(base, 99)));

        // The window is the one the header left, so what is below it counts the
        // lines the reader has yet to come back down through — and not the row
        // the header is sitting on.
        assert_eq!(app.panel().lines_below(), window * 3 + 1 - window);
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
            assert_eq!(app.panel().lines_below(), 0, "{name}");
            let edge = panel_bottom_edge(&render_at(&app, WIDTH, HEIGHT, at(base, 99)));
            assert!(!edge.contains(SCROLLBACK_ARROW), "{name}: {edge:?}");

            // Scrolled back to the first line: the number on the edge is the
            // account less the windowful on screen, counted against the window
            // the header left rather than against the whole of the border.
            app.toggle_focus();
            app.select_first();
            let buffer = render_at(&app, WIDTH, HEIGHT, at(base, 99));
            let below = app.panel().lines_below();

            assert_eq!(app.panel().scroll_offset(), 0, "{name}");
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
                assert_eq!(
                    app.panel().lines_below(),
                    below - step,
                    "{name}, {step} down"
                );
            }
            assert!(app.panel().follows(), "{name}");
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

    fn composer_rows(buffer: &Buffer, composer: &Composer) -> Vec<String> {
        let area = composer_field(buffer, composer);

        (0..area.height)
            .map(|index| text_in(buffer, area, area.y + index))
            .collect()
    }

    fn composer_field(buffer: &Buffer, composer: &Composer) -> Rect {
        pane_inner(
            areas(buffer.area, Some(composer))
                .composer
                .expect("a field on this frame"),
        )
    }

    fn caret_cells(buffer: &Buffer, composer: &Composer) -> Vec<(u16, u16, String)> {
        let area = composer_field(buffer, composer);

        (0..area.height)
            .flat_map(|row| (0..area.width).map(move |column| (column, row)))
            .filter_map(|(column, row)| {
                let cell = &buffer[(area.x + column, area.y + row)];
                cell.modifier
                    .contains(Modifier::REVERSED)
                    .then(|| (column, row, cell.symbol().to_owned()))
            })
            .collect()
    }

    fn caret_cell(buffer: &Buffer, composer: &Composer) -> (u16, u16, String) {
        let mut cells = caret_cells(buffer, composer);
        assert_eq!(cells.len(), 1, "one caret and one only: {cells:?}");

        cells.remove(0)
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
    fn the_caret_is_drawn_at_the_cursor_and_not_after_the_last_character() {
        // The field used to draw its caret by appending a reversed blank to the
        // last row it drew, which was the right cell only while the cursor was
        // at the end of the draft. With the cursor somewhere in the middle of
        // one, the caret is the character it is on — the cell what is typed next
        // pushes along — and there is nothing reversed at the end of the draft.
        let mut app = App::from_tree(&fixture::tree());
        app.set_focus(Focus::Composer);

        let draft = "what does the engine do?";
        // `what |does the engine do?`: the first character of the second word.
        let middle = Composer::new(draft).at(5);
        let buffer = render_composer(&app, &middle, WIDTH, FIXTURE_HEIGHT);

        assert_eq!(
            composer_rows(&buffer, &middle).first().map(String::as_str),
            Some(draft),
            "the draft is drawn as it always was"
        );
        // One caret and one only, so the blank after the last character is not
        // reversed as well.
        assert_eq!(
            caret_cell(&buffer, &middle),
            (5, 0, "d".to_owned()),
            "the caret is the character the cursor is on"
        );

        // And the end of the draft is where it goes when that is where the
        // cursor is: the reversed blank, in the column after the last character,
        // which is what the field drew before there was a cursor to ask.
        let end = Composer::new(draft);
        let buffer = render_composer(&app, &end, WIDTH, FIXTURE_HEIGHT);
        let after = u16::try_from(display_width(draft)).expect("a short draft");
        assert_eq!(
            caret_cell(&buffer, &end),
            (after, 0, COMPOSER_CURSOR.to_owned())
        );
    }

    #[test]
    fn the_caret_lands_in_the_cell_a_wide_character_draws_in() {
        // Columns are cells and not characters. Three characters two columns
        // wide, and from the second of them a caret counted by character would
        // be drawn a column short of the character it is meant to be on — and
        // then a column further short for each one after it.
        let mut app = App::from_tree(&fixture::tree());
        app.set_focus(Focus::Composer);

        let draft = "日本語 and more";
        // The same row read out of the buffer cell by cell: the renderer leaves
        // the second cell of a wide character blank, so each of the three has a
        // blank behind it and the draft's own space comes after the third.
        let drawn = "日 本 語  and more";
        // The byte offset the cursor is at, the column the caret is drawn in,
        // and the character drawn there. The offsets are three bytes apart
        // across the wide characters and one byte apart after them; the columns
        // are two apart and then one.
        let cells = [
            (0, 0, "日"),
            (3, 2, "本"),
            (6, 4, "語"),
            (9, 6, " "),
            (10, 7, "a"),
        ];

        for (offset, column, character) in cells {
            let composer = Composer::new(draft).at(offset);
            let buffer = render_composer(&app, &composer, WIDTH, FIXTURE_HEIGHT);

            assert_eq!(
                composer_rows(&buffer, &composer)
                    .first()
                    .map(String::as_str),
                Some(drawn),
                "the draft is drawn as it always was"
            );
            assert_eq!(
                caret_cell(&buffer, &composer),
                (column, 0, character.to_owned()),
                "the cursor at byte {offset}"
            );
        }
    }

    #[test]
    fn the_space_that_wraps_a_row_puts_the_caret_at_the_start_of_the_row_below() {
        // An offset at a row break is column zero of the lower row and never a
        // cell past the end of the row above — the rule the composer's maps
        // keep, here on the drawn buffer. The space the reader typed is a
        // character of the draft with a cell of its own at the end of the upper
        // row, and the cursor it left behind is the cell after it, which is on
        // the row below.
        let mut app = App::from_tree(&fixture::tree());
        app.set_focus(Focus::Composer);

        // The width the field is actually drawn at, asked of a frame rather than
        // worked out, so the draft below breaks exactly once wherever the layout
        // puts the pane's edge.
        let probe = Composer::default();
        let width = usize::from(
            composer_field(
                &render_composer(&app, &probe, WIDTH, FIXTURE_HEIGHT),
                &probe,
            )
            .width,
        );

        // A word a column short of the field, so the space after it is the last
        // cell that fits and the word after it is what does not.
        let word = "a".repeat(width - 1);
        let rest = "b".repeat(width - 1);
        // Where typing that space leaves the cursor: immediately after it.
        let composer = Composer::new(format!("{word} {rest}")).at(word.len() + 1);
        let buffer = render_composer(&app, &composer, WIDTH, FIXTURE_HEIGHT);

        // Two rows, and the space stayed on the upper one (the row text is read
        // with its trailing blanks trimmed off, so it reads as the word).
        assert_eq!(composer_rows(&buffer, &composer), [word, rest]);
        // Column zero of the lower row, and the one caret, so the cell at the
        // end of the row above is not reversed either.
        assert_eq!(caret_cell(&buffer, &composer), (0, 1, "b".to_owned()));
    }

    #[test]
    fn the_drawn_rows_follow_the_cursor_up_and_down_a_draft_past_the_cap() {
        // A field six rows tall and a draft twice that: the rows on screen are
        // the ones the cursor is among, wherever it has been moved to, so an Up
        // never moves a cursor the reader cannot see.
        let mut app = App::from_tree(&fixture::tree());
        app.set_focus(Focus::Composer);

        let lines: Vec<String> = (0..usize::from(COMPOSER_MAX_ROWS) * 2)
            .map(|line| format!("line {line}"))
            .collect();
        let draft = lines.join("\n");
        // The byte offset each line starts at: its own length and the newline
        // after it, line by line.
        let starts: Vec<usize> = lines
            .iter()
            .scan(0, |offset, line| {
                let start = *offset;
                *offset += line.len() + 1;
                Some(start)
            })
            .collect();

        // The cases below name their lines, so the arithmetic they were worked
        // out from is written down rather than assumed.
        assert_eq!(COMPOSER_MAX_ROWS, 6);
        assert_eq!(lines.len(), 12);

        // Which line the cursor is on, and which line the top row of the field
        // is then drawn from: the tail while the cursor is at the end of the
        // draft, which is what the field drew before it followed anything; up
        // with the cursor when it is moved above those rows, at the two-row
        // margin until the top of the draft stops it; and down again after it.
        for (line, first) in [(11, 6), (1, 0), (0, 0), (8, 6), (4, 2)] {
            let composer = Composer::new(draft.clone()).at(starts[line]);
            let buffer = render_composer(&app, &composer, WIDTH, 40);
            let rows = usize::from(COMPOSER_MAX_ROWS);

            assert_eq!(
                composer_rows(&buffer, &composer),
                lines[first..first + rows],
                "the cursor on line {line}"
            );
            // On screen, on the row of its own line, at the start of it.
            let row = u16::try_from(line - first).expect("a row of the field");
            assert_eq!(
                caret_cell(&buffer, &composer),
                (0, row, "l".to_owned()),
                "the cursor on line {line}"
            );
        }
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

    fn app_running_under_a_thread(base: Instant) -> App {
        let mut app = App::from_tree(&fixture::tree());
        app.set_focus(Focus::Composer);
        app.panel_mut()
            .start_turn("what does the engine do?", at(base, 1));
        app.panel_mut()
            .answer_turn("It walks the tree.", at(base, 2));
        app.start_account(at(base, 3));
        app.set_pact_in_flight("warlock/crates/engine", 1, 2);

        assert!(
            app.panel().showing_thread(),
            "the run swapped the card away"
        );
        assert!(app.panel().composer_showable(), "the run hid the field");
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
        assert!(
            app.panel().showing_thread(),
            "the swap landed somewhere else"
        );
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

    const COVER_WIDTH: u16 = 60;
    const COVER_HEIGHT: u16 = 8;

    const UNDERNEATH: &str = "underneath";

    fn busy_app(base: Instant, width: u16, height: u16) -> App {
        let mut app = pacting_app(base, width, height);
        let detail = [UNDERNEATH; MANY].join(" ");
        let account = app.panel_mut().account_mut().expect("a pact has started");
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

    fn confirm_rect(buffer: &Buffer) -> Rect {
        confirm_area(buffer.area)
    }

    fn confirm_rows(buffer: &Buffer) -> Vec<String> {
        let area = confirm_rect(buffer);
        (0..area.height)
            .map(|index| text_in(buffer, area, area.y + index))
            .collect()
    }

    fn inside_the_border(row: &str) -> String {
        row.trim_matches(|glyph: char| "┌┐└┘─│ ".contains(glyph))
            .to_owned()
    }

    fn answers_text() -> String {
        format!("{CONFIRM_YES}{CONFIRM_ANSWER_GAP}{CONFIRM_NO}")
            .trim()
            .to_owned()
    }

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

    const SCOPED: &str = "crates/warlock-engine";

    const CARRIED: &str = "data-plane";

    fn window_rect(buffer: &Buffer, field: &ScopeField, heading: &str, rules: &str) -> Rect {
        centred(buffer.area, scope_size(field, heading, rules))
    }

    fn window_rows(buffer: &Buffer, field: &ScopeField, heading: &str, rules: &str) -> Vec<String> {
        let area = window_rect(buffer, field, heading, rules);
        (0..area.height)
            .map(|index| text_in(buffer, area, area.y + index))
            .collect()
    }

    fn window_cursor(buffer: &Buffer, field: &ScopeField, heading: &str, rules: &str) -> Position {
        let area = window_rect(buffer, field, heading, rules);
        let typed = u16::try_from(display_width(field.text())).expect("a short line");

        Position::new(
            area.x + BORDER_THICKNESS + SCOPE_MARGIN + typed,
            area.y + BORDER_THICKNESS + SCOPE_MARGIN_ROWS + FIELD_LINE,
        )
    }

    fn scope_rect(buffer: &Buffer, field: &ScopeField) -> Rect {
        window_rect(buffer, field, SCOPE_HEADING, scope::RULES)
    }

    fn scope_rows(buffer: &Buffer, field: &ScopeField) -> Vec<String> {
        window_rows(buffer, field, SCOPE_HEADING, scope::RULES)
    }

    fn cursor_cell(buffer: &Buffer, field: &ScopeField) -> Position {
        window_cursor(buffer, field, SCOPE_HEADING, scope::RULES)
    }

    fn path_rows(buffer: &Buffer, field: &ScopeField) -> Vec<String> {
        window_rows(buffer, field, PATH_HEADING, PATH_RULES)
    }

    fn path_cursor(buffer: &Buffer, field: &ScopeField) -> Position {
        window_cursor(buffer, field, PATH_HEADING, PATH_RULES)
    }

    const FIELD_LINE: u16 = 2;

    const SCOPE_COVER_WIDTH: u16 = 80;
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

    const HEADED: &str = "Write the brief to";

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
