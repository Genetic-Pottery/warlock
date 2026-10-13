//! One frame, drawn from the arguments and nothing else.
//!
//! Nothing here reads a clock, a terminal or a global: the instant a pulse is
//! measured against arrives as a parameter, so a frame can be asserted against
//! an in-memory buffer with no tty attached.
//!
//! Every size the app asks for, every hit test and [`draw`] itself read one
//! [`Cut`], made from a [`Shape`]. A second copy of the layout arithmetic here
//! would answer for a screen that was never drawn.

use std::fmt::Display;
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
use crate::app::{App, Chrome, Focus, Row, RunHeader, Sigils, Target, Working};
use crate::colour::{CONVERSATION_COLOUR, FOCUS_COLOUR, GUIDE_COLOUR, SYSTEM_COLOUR, colour_for};
use crate::composer::Composer;
use crate::confirm::{Answer, Carry, Choice, Cutting, Filing, Review, Undertaking};
use crate::modal::Modal;
use crate::panel::Mode;
use crate::prompt::{RecordField, RecordForm, ScopeField};
// Renamed for the reason `Entry` above is: `Span` here is ratatui's piece of a
// drawn line, and the selection's is the cells of one row the highlight covers;
// its `Window` is the panel's view of the card, which is neither of the
// terminal windows this module knows about.
use crate::selection::{Span as Selected, Window as Viewport, spans_at};
use crate::submission::help_rows;
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

const RUN_HEADER_HEIGHT: u16 = 2;

pub(crate) const PACTING_RUN: &str = "pacting";

pub(crate) const REFRESHING_RUN: &str = "refreshing";

const BAR_FILLED: &str = "█";

const BAR_EMPTY: &str = "░";

const BAR_GAP: &str = " ";

const BAR_MIN_WIDTH: usize = 4;

/// Columns kept for the percentage beside the bar, which is `{:>3}%` — so the
/// bar is the same width at 7% as at 100% instead of shrinking by a column
/// twice on the way up.
const PERCENT_WIDTH: usize = 4;

pub(crate) const PANEL_INDENT: &str = "  ";

const THREAD_TITLE: &str = " thread ";

pub(crate) const SAID_MARKER: &str = "› ";

pub(crate) const NOTE_MARKER: &str = "· ";

const ELLIPSIS: &str = "…";

const SCROLLBACK_ARROW: &str = "↓";

const LIVE_KEY: &str = "G";

const HELP_KEY: &str = "?: help";

const QUIT_KEY: &str = "q/Esc/Ctrl-C: quit";

const KEYS: &[&str] = &[HELP_KEY, QUIT_KEY];

const KEY_GAP: &str = "    ";

// The order either footer gives keys up in as the terminal narrows. A key not
// named here survives every width: the way out, because a reader who cannot see
// how to leave has no way to find out, and during a run the cancel key, for
// `PACTING_KEYS`' reason. Everything else is on the help sheet.
const DROP_ORDER: &[&str] = &[HELP_KEY];

fn laid_out_keys(width: usize, pieces: &[&str]) -> String {
    let mut kept: Vec<&str> = pieces.to_vec();
    for name in DROP_ORDER {
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
    fitted(text, width).to_owned()
}

pub(crate) fn fitted(text: &str, columns: usize) -> &str {
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
    laid_out_keys(width, KEYS)
}

// Esc is on the footer during a run because it stops meaning quit for as long as
// the run lasts, and `q` drops the question, so neither can be left to the help
// sheet to say.
const PACTING_KEYS: &[&str] = &[HELP_KEY, CANCEL_KEY, PACTING_QUIT_KEY];

const CANCEL_KEY: &str = "Esc: cancel";

const PACTING_QUIT_KEY: &str = "q/Ctrl-C: quit";

fn pacting_keys_line(width: usize) -> String {
    laid_out_keys(width, PACTING_KEYS)
}

const FOOTER_HEIGHT: u16 = 3;

const TREE_PERCENT: u16 = 30;

const TREE_MIN_WIDTH: u16 = 30;

const BORDER_THICKNESS: u16 = 1;

const PULSE_PHASE: Duration = Duration::from_millis(500);

const CONFIRM_QUESTION: &str = "Leave warlock?";

const HELP_TITLE: &str = " keys · Esc or ? closes · j/k scrolls ";

const HELP_KEY_WIDTH: usize = 24;

enum Rows {
    Keys(&'static [(&'static str, &'static str)]),
    Commands,
}

// A short copy of `CONTROLS.md`, which has the reasons. A key added or changed
// in `input.rs` or `composer.rs` needs its line here too; a command's rows are
// written with the command, in `submission.rs`.
const HELP_SECTIONS: &[(&str, Rows)] = &[
    (
        "Moving",
        Rows::Keys(&[
            ("j / k   ↓ / ↑", "down or up one row"),
            ("PgDn / PgUp", "down or up one screen"),
            ("g / G", "first or last row"),
            ("space", "fold or unfold a directory"),
            ("Tab", "next pane: tree, panel, chat bar"),
            ("Shift-Tab", "switch the panel's card"),
        ]),
    ),
    (
        "The tree",
        Rows::Keys(&[
            ("o", "show only pacted directories"),
            ("f", "show files too"),
            ("p", "pact or un-pact the directory"),
            ("r", "refresh its stale documents"),
            ("s", "set its scope"),
            ("v", "read the file in the panel"),
            ("e", "open the file in $EDITOR"),
            ("m", "mouse capture off or on"),
            ("drag", "select panel text to copy it"),
        ]),
    ),
    (
        "The chat bar",
        Rows::Keys(&[
            ("Enter", "send"),
            ("Alt-Enter", "new line"),
            ("Ctrl-U", "delete to the start of the line"),
            ("→", "take the dimmed suggestion"),
            ("Esc", "leave the chat bar, keep the text"),
        ]),
    ),
    ("Commands", Rows::Commands),
    (
        "Stopping",
        Rows::Keys(&[
            ("Esc", "cancel a pact or refresh"),
            ("Ctrl-C", "cancel a chat turn"),
            ("q / Esc / Ctrl-C", "ask whether to quit"),
        ]),
    ),
];

const CONFIRM_YES: &str = " Yes ";

const CONFIRM_NO: &str = " No ";

const CONFIRM_ANSWER_GAP: &str = "    ";

const DIALOG_MARGIN: u16 = 3;

const DIALOG_MARGIN_ROWS: u16 = 1;

const PUSH_QUESTION: &str = "File this brief to the board?";

/// Also the cut and pull dialogs' team label, as [`PUSH_KEY`] is the cut
/// dialog's key label: they label the same facts, and two spellings for one fact
/// would read as two facts.
const PUSH_TEAM: &str = "team ";

/// The key is named and never shown, here as everywhere else warlock mentions
/// one: the dialog's value carries no key bytes at all, so this label can only
/// ever be followed by the name it is held under.
const PUSH_KEY: &str = "key ";

const CUT_QUESTION: &str = "Draft tickets for this project?";

/// The status the board answered with, labelled rather than left to be read as
/// a second name: the project above it is a title and this is a column value,
/// and the two would otherwise be two bare strings under one another.
const CUT_STATUS: &str = "status ";

const CUT_SLICES: &str = "slices ";

const PULL_QUESTION: &str = "Work this ticket to a pull request?";

/// A question of its own rather than the one above with a word added, because
/// a resumed run is not the same undertaking: the branch is already there and
/// somebody else's halt is being carried on from.
const PULL_RESUME_QUESTION: &str = "Resume this ticket and carry on working it?";

const PULL_SCOPE: &str = "scope ";

const PULL_BRANCH: &str = "branch ";

/// Said before the sub-task a resumed run starts from, which is the one line
/// that is on this window and on neither of the others.
const PULL_FROM: &str = "from ";

const REVIEW_QUESTION: &str = "File these drafts as issues?";

/// After the count of drafts. The keys are the ones `input.rs` gives the panel
/// while this window is up.
const REVIEW_READ: &str = " on the panel — ↑ ↓ PgUp PgDn to read";

/// The three answers, each with the space around it the other windows' two
/// carry, so a reader who has answered one of those is looking at the same
/// shapes in the same place.
const REVIEW_CREATE: &str = " Create ";

const REVIEW_EDIT: &str = " Edit ";

const REVIEW_SKIP: &str = " Skip ";

const REVIEW_FEEDBACK: &str = " Feedback ";

const REVIEW_ANSWER_GAP: &str = CONFIRM_ANSWER_GAP;

const CARRY_QUESTION: &str = "Carry on to the next slice?";

/// Said after the count the run handed over, which words the noun: `3 slices`
/// becomes `3 slices left`.
const CARRY_LEFT: &str = " left";

const SCOPE_HEADING: &str = "Scope for ";

const PATH_HEADING: &str = "";

const PATH_RULES: &str = "Enter writes the document, Esc writes nothing";

const SCOPE_CURSOR: &str = " ";

const RECORD_HEADING: &str = "Record for ";

const RECORD_RULES: &str =
    "Tab moves to the next field, Enter writes the scope and record, Esc writes nothing";

const RECORD_LABEL_GAP: &str = "  ";

/// Reversed rather than a background colour of its own. Colour on the thread
/// already means whose words these are — the model's, the reader's, warlock's —
/// and painting over it would take that away from exactly the rows somebody is
/// reading; swapping what the cell already had keeps every one of those apart
/// under the highlight.
const SELECTED: Style = Style::new().add_modifier(Modifier::REVERSED);

const COMPOSER_CURSOR: &str = SCOPE_CURSOR;

const COMPOSER_MIN_HEIGHT: u16 = 1 + 2 * BORDER_THICKNESS;

// `cut` is the round's own, measured from the same `Shape::of` the hit test and
// the app's heights were; `composer` only draws where `cut` gave it rows, so a
// field that is not on screen is never drawn whichever composer is passed.
pub(crate) fn draw(
    frame: &mut Frame<'_>,
    cut: &Cut,
    app: &App,
    chrome: &Chrome,
    now: Instant,
    modal: Option<Modal<'_>>,
    composer: Option<&Composer>,
) {
    let screen = frame.area();
    debug_assert_eq!(cut.screen, screen, "the cut was measured for another frame");
    let header = app.run_header();

    draw_panel(frame, cut, app, header.as_ref(), now);
    if let (Some(area), Some(composer)) = (cut.composer, composer) {
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
        let strip = workflow_line(app.panel().mode(), app.panel().ready(), app.working());
        draw_composer(frame, area, composer, live, strip);
    }
    draw_tree_pane(frame, cut, app, chrome, now);
    if let Some(area) = cut.sigils {
        draw_sigils(frame, area, chrome.sigils());
    }
    draw_footer(frame, cut.footer, app);

    // Over the finished frame rather than instead of it, clearing only the cells
    // behind it, so what a window is answered against is still on screen around
    // it.
    if let Some(modal) = modal {
        Dialog::of(modal).draw(frame, screen);
    }
}

#[cfg(test)]
static NO_SIGILS: Sigils = Sigils::Nothing;

/// Everything the layout depends on, and nothing else. A field added here is
/// read by the frame and by every measurement at once; an input passed to one
/// of them beside a `Shape` would let the two cut different screens.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Shape<'a> {
    size: Size,
    composer: Option<&'a Composer>,
    run_header: bool,
    sigils: &'a Sigils,
}

impl<'a> Shape<'a> {
    #[must_use]
    pub(crate) fn of(
        size: Size,
        app: &App,
        composer: Option<&'a Composer>,
        sigils: &'a Sigils,
    ) -> Self {
        Self {
            size,
            composer: composer.and_then(|field| composer_on_screen(app, field)),
            run_header: app.has_run(),
            sigils,
        }
    }

    #[must_use]
    #[cfg(test)]
    pub(crate) fn new(size: Size) -> Self {
        Self {
            size,
            composer: None,
            run_header: false,
            sigils: &NO_SIGILS,
        }
    }

    #[must_use]
    #[cfg(test)]
    pub(crate) fn composer(self, composer: Option<&'a Composer>) -> Self {
        Self { composer, ..self }
    }

    #[must_use]
    #[cfg(test)]
    pub(crate) fn header(self, run_header: bool) -> Self {
        Self { run_header, ..self }
    }

    #[must_use]
    #[cfg(test)]
    pub(crate) fn sigils(self, sigils: &'a Sigils) -> Self {
        Self { sigils, ..self }
    }

    #[must_use]
    pub(crate) fn cut(self) -> Cut {
        let Columns {
            screen,
            panel,
            column,
            footer,
        } = Columns::of(self.size);
        let (panel, composer) = split_column(panel, self.composer);
        let (run_header, panel_rows) = panel_split(panel, self.run_header);
        let (tree, sigils) = tree_column(column, self.sigils);

        Cut {
            screen,
            panel,
            run_header,
            panel_rows,
            composer,
            tree,
            tree_rows: pane_inner(tree),
            sigils,
            footer,
        }
    }
}

/// The screen as one [`Shape`] divides it.
///
/// Made again from the current inputs every time it is asked for, and never
/// kept from the frame before: a click is answered against the screen the
/// reader is looking at, and a cut remembered from drawing goes stale the
/// moment the terminal, the draft or the run header changes under it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Cut {
    screen: Rect,
    panel: Rect,
    run_header: Option<Rect>,
    panel_rows: Rect,
    composer: Option<Rect>,
    tree: Rect,
    tree_rows: Rect,
    sigils: Option<Rect>,
    footer: Rect,
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

#[must_use]
fn composer_on_screen<'a>(app: &App, composer: &'a Composer) -> Option<&'a Composer> {
    app.panel().composer_showable().then_some(composer)
}

// The horizontal half of a cut, on its own so the panel's width can be read
// before anything vertical is known, and from the same split the frame uses.
struct Columns {
    screen: Rect,
    panel: Rect,
    column: Rect,
    footer: Rect,
}

impl Columns {
    fn of(size: Size) -> Self {
        let screen = Rect::from(size);
        let [above, footer] =
            Layout::vertical([Constraint::Min(0), Constraint::Length(FOOTER_HEIGHT)]).areas(screen);
        let width = tree_width(above.width);
        let [panel, column] = Layout::horizontal([
            Constraint::Length(above.width - width),
            Constraint::Length(width),
        ])
        .areas(above);
        Self {
            screen,
            panel,
            column,
            footer,
        }
    }
}

#[must_use]
pub(crate) fn panel_width(size: Size) -> u16 {
    pane_inner(Columns::of(size).panel).width
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

fn panel_split(panel: Rect, header: bool) -> (Option<Rect>, Rect) {
    let inner = pane_inner(panel);
    // A panel with no room to spare loses the run header whole rather than
    // sharing: `Length` would take its rows anyway and leave the account with
    // none, so the reader would watch a progress bar over an empty pane.
    if !header || inner.height <= RUN_HEADER_HEIGHT {
        return (None, inner);
    }

    let [header, rows] =
        Layout::vertical([Constraint::Length(RUN_HEADER_HEIGHT), Constraint::Min(0)]).areas(inner);

    (Some(header), rows)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Hit {
    Offscreen,
    Footer,
    Border,
    TreeRow { offset: u16 },
    TreeBelowRows,
    PanelHeader,
    PanelLine { offset: u16, column: u16 },
    Composer,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Reach {
    Above { rows: u16, column: u16 },
    Inside { column: u16 },
    Below { rows: u16, column: u16 },
}

impl Cut {
    #[must_use]
    pub(crate) const fn screen(&self) -> Rect {
        self.screen
    }

    #[must_use]
    pub(crate) fn tree_height(&self) -> u16 {
        self.tree_rows.height
    }

    #[must_use]
    pub(crate) fn panel_height(&self) -> u16 {
        self.panel_rows.height
    }

    /// Where the panes are and nothing else: what a row offset stands for is
    /// the app's to say, and there is no hover.
    ///
    /// `PanelLine` reports the column beside the row, both counted from the
    /// first cell of the panel's rows, so a pointer event becomes a
    /// `selection::Cell` without measuring the layout a second time.
    #[must_use]
    pub(crate) fn hit(&self, column: u16, row: u16) -> Hit {
        let point = Position::new(column, row);
        if !self.screen.contains(point) {
            return Hit::Offscreen;
        }

        if self.footer.contains(point) {
            return Hit::Footer;
        }

        if pane_inner(self.panel).contains(point) {
            let rows = self.panel_rows;
            if row < rows.y {
                return Hit::PanelHeader;
            }
            return Hit::PanelLine {
                offset: row.saturating_sub(rows.y),
                column: column.saturating_sub(rows.x),
            };
        }

        if self
            .composer
            .is_some_and(|field| pane_inner(field).contains(point))
        {
            return Hit::Composer;
        }

        if pane_inner(self.tree).contains(point) {
            let rows = self.tree_rows;
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

    /// How far past the panel's rows a point is.
    ///
    /// A pointer held past the card arrives as [`Hit::Footer`],
    /// [`Hit::Border`], [`Hit::Composer`] or [`Hit::Offscreen`], none of which
    /// says which way it left or by how much, so this answers the question
    /// [`Hit`] cannot. The row is counted against the rows area alone: the run
    /// header and the composer are not in it, so a point on either is past an
    /// edge here even though it is inside the panel's column.
    ///
    /// `column` is clamped into the rows area rather than refused, so a pointer
    /// that wandered sideways — onto the border, the tree, past the right edge
    /// of the screen — still names the cell of the card nearest it.
    #[must_use]
    pub(crate) fn reach(&self, column: u16, row: u16) -> Reach {
        let rows = self.panel_rows;
        let column = column
            .saturating_sub(rows.x)
            .min(rows.width.saturating_sub(1));

        // Tested against the rows area's own bounds rather than against its
        // height, so an area with no rows in it — a terminal too short for the
        // panel to hold a line — is all edge and no inside, and no point is
        // answered as a row that was never drawn.
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
}

fn pane_block(focused: bool) -> Block<'static> {
    let style = if focused {
        Style::new().fg(FOCUS_COLOUR).add_modifier(Modifier::BOLD)
    } else {
        Style::new().add_modifier(Modifier::DIM)
    };

    Block::bordered().border_style(style)
}

fn draw_panel(
    frame: &mut Frame<'_>,
    cut: &Cut,
    app: &App,
    header: Option<&RunHeader>,
    now: Instant,
) {
    let (area, inner) = (cut.panel, cut.panel_rows);
    let panel = app.panel();
    let view = panel.frame(now);
    let below = view.below;
    let conversation = panel.showing_thread();
    let mut block = pane_block(app.focus() == Focus::Panel);
    if conversation {
        block = block.title_top(Line::from(THREAD_TITLE).bold());
    }
    if below > 0 {
        block = block.title_bottom(Line::from(scrollback(below)).right_aligned().dim());
    }

    frame.render_widget(block, area);

    if let (Some(area), Some(header)) = (cut.run_header, header) {
        draw_run_header(frame, area, header);
    }

    if !panel.has_content() {
        draw_mark(frame, inner);
        return;
    }

    let rows: Vec<Line<'static>> = view
        .lines
        .iter()
        .map(|line| panel_row(line, inner.width, conversation))
        .collect();
    frame.render_widget(Paragraph::new(rows), inner);

    // Only the conversation is selectable, so only the conversation is drawn
    // with a highlight: the account and the document are read past, not copied
    // out of, and a highlight measured against the thread would sit over
    // whatever text took its place.
    if conversation {
        draw_highlight(frame, inner, app, view.offset, now);
    }
}

/// Painted over the rows after they are drawn rather than woven into them.
/// [`panel_row`] truncates a row that still does not fit, so text built with a
/// highlight in it would have to be cut by a second rule that agreed with that
/// one; here the cut is [`Rect::intersection`], and a span reaching past the
/// panel stops at its last column instead of on the border or the tree beside
/// it.
fn draw_highlight(frame: &mut Frame<'_>, rows: Rect, app: &App, scroll: usize, now: Instant) {
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
        scroll,
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
    let words = header.words();

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

/// How far through the run the bar and the percentage both are, in whatever
/// unit is the finest the run has reported.
///
/// Directories while a directory is all the engine has said, and files once it
/// has: a run of five directories working the second of them, describing the
/// fourth file of eight, is `1 * 8 + 3` of `5 * 8`. Scaling the whole run by
/// *this* directory's file count is deliberate and is why the denominator moves
/// — the alternative is knowing every directory's file count before the run
/// starts, which costs a walk of the whole subtree to answer a question about a
/// progress bar. The bar advances inside a directory and never goes backwards
/// past a boundary: the last file of a directory leaves it one file short of
/// the next directory's start, and the start moves it forward.
///
/// Both counts are what the run has finished, not what it has started. The
/// engine's `Describing` is sent before its file's pass, as `position` is the
/// directory being worked (see [`RunHeader::completed`]), so each is one less
/// than the number reported. Counting the one in flight is what drew a full bar
/// over a single-file refresh that had finished nothing.
///
/// Saturating rather than checked because the products are small — a run is
/// directories times files, both of them counts of things on a disk — and a
/// saturated total still divides.
fn fraction(header: &RunHeader) -> (usize, usize) {
    match header.files() {
        Some((position, total)) if total > 0 => (
            header
                .completed()
                .saturating_mul(total)
                .saturating_add(position.saturating_sub(1)),
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

fn draw_composer(
    frame: &mut Frame<'_>,
    area: Rect,
    composer: &Composer,
    live: bool,
    strip: Line<'static>,
) {
    let inner = pane_inner(area);
    let mut block = pane_block(live).title_top(strip.right_aligned());
    // The one thing about the field that is not on the field: while something
    // other than the conversation is waiting on a draft, what it is waiting for
    // is written on the border, because a draft that answers somewhere else is
    // otherwise indistinguishable from one that answers here. A title costs no
    // row — it is drawn in the border the field already has.
    if let Some(answering) = composer.answering() {
        block = block.title_top(Line::from(answering.to_owned()).bold());
    }
    frame.render_widget(block, area);

    let ghost = composer.ghost_rows(inner.width, inner.height);
    if !ghost.is_empty() {
        let dim = Style::new().fg(GUIDE_COLOUR);
        let mut lines: Vec<Line<'static>> = ghost
            .iter()
            .map(|row| Line::styled(row.clone(), dim))
            .collect();
        // The caret on the suggestion's first character, so the field still
        // reads as the one the keys go to.
        if live && let Some(first) = ghost.first() {
            let (_, at, after) = split_at_column(first, 0);
            lines[0] = Line::from(vec![
                Span::styled(at.to_owned(), dim.add_modifier(Modifier::REVERSED)),
                Span::styled(after.to_owned(), dim),
            ]);
        }
        frame.render_widget(Paragraph::new(lines), inner);
        return;
    }

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

const WORKFLOW_LIT: Style = Style::new()
    .fg(Color::Black)
    .bg(FOCUS_COLOUR)
    .add_modifier(Modifier::BOLD);

const WORKFLOW_DIM: Style = Style::new().fg(Color::Indexed(245)).bg(Color::Indexed(236));

// Every stage is drawn whatever is lit, so a reader new to warlock sees the
// whole road and where they are on it. Brief and write light together when the
// model thinks the brief is ready. A push, draft or pull running lights alone:
// the conversation's mode beside it read as a second thing going on.
fn workflow_line(mode: Mode, ready: bool, working: Option<Working>) -> Line<'static> {
    let talking = working.is_none();
    let stages = [
        ("chat", talking && mode == Mode::Chat),
        ("brief", talking && mode == Mode::Brief),
        ("write", talking && mode == Mode::Brief && ready),
        ("push", working == Some(Working::Push)),
        ("draft", working == Some(Working::Draft)),
        ("pull", working == Some(Working::Pull)),
    ];
    let mut spans = Vec::new();
    for (stage, lit) in stages {
        if !spans.is_empty() {
            spans.push(Span::raw(" "));
        }
        let style = if lit { WORKFLOW_LIT } else { WORKFLOW_DIM };
        spans.push(Span::styled(format!(" {stage} "), style));
    }
    spans.push(Span::raw(" "));
    Line::from(spans)
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

fn draw_tree_pane(frame: &mut Frame<'_>, cut: &Cut, app: &App, chrome: &Chrome, now: Instant) {
    let area = cut.tree;
    let room = usize::from(area.width.saturating_sub(2 * BORDER_THICKNESS));
    let title = target_title(chrome.target(), room);
    let mut block = pane_block(app.focus() == Focus::Tree).title_top(Line::from(title).bold());
    if let Some(identity) = identity_title(chrome.header(), room) {
        block = block.title_bottom(Line::from(identity).bold());
    }
    frame.render_widget(block, area);

    draw_tree(frame, cut.tree_rows, app, now);
}

// Fewer columns than this and the tree's own path is not drawn at all: a path
// cut to two characters names nothing.
const IDENTITY_MIN_WIDTH: usize = 6;

// On the bottom border rather than beside the target on the top one, because
// ` pulls into the default branch ` alone fills most of a 30-column tree and
// would leave the path no room at ordinary widths. Empty when warlock opened
// at the repository root, which is the usual case and needs no name.
pub(crate) fn identity_title(identity: &str, width: usize) -> Option<String> {
    if identity.is_empty() || width < IDENTITY_MIN_WIDTH {
        return None;
    }
    Some(format!(" {} ", truncated(identity, width - 2)))
}

const TARGET_LABEL: &str = " pulls into ";

// The branch is cut with an ellipsis rather than left to the border, because
// ratatui clips a title without saying so and a clipped branch name reads as a
// whole one: the one mistake this title is here to prevent.
pub(crate) fn target_title(target: Target<'_>, width: usize) -> String {
    let name = match target {
        Target::Set(base) => base.to_owned(),
        Target::Default(head) => format!("{head} (default)"),
        Target::Unknown => "the default branch".to_owned(),
    };
    let room = width.saturating_sub(display_width(TARGET_LABEL) + 1);
    format!("{TARGET_LABEL}{} ", truncated(&name, room))
}

const SIGILS_TITLE: &str = " sigils ";

const SIGILS_MOST_LINES: usize = 5;

// Rows the tree keeps before the sigils box gives way to it: below this the box
// is not drawn at all, because a tree of one or two rows is no tree to read.
const TREE_MIN_ROWS: u16 = 3;

const WILDCARD: &str = "*";

const UNREADABLE: &str = "can't read the config";

// A name is never cut: a sigil is at most 24 characters and the column is at
// least `TREE_MIN_WIDTH`, so the box only ever has to drop whole lines, and it
// says how many it dropped. Holding nothing is a blank line rather than a word,
// because any one word could be a sigil somebody holds. The unreadable line has
// spaces in it for the same reason: no sigil can, so it never reads as one.
pub(crate) fn sigil_lines(sigils: &Sigils) -> Vec<String> {
    let named = |sigil: &String| {
        if sigil == WILDCARD {
            format!("{WILDCARD}  (every scope)")
        } else {
            sigil.clone()
        }
    };
    match sigils {
        Sigils::Nothing => vec![String::new()],
        Sigils::Unknown => vec![UNREADABLE.to_owned()],
        Sigils::Held(held) if held.len() <= SIGILS_MOST_LINES => held.iter().map(named).collect(),
        Sigils::Held(held) => {
            let shown = SIGILS_MOST_LINES - 1;
            let mut lines: Vec<String> = held[..shown].iter().map(named).collect();
            lines.push(format!("+{} more", held.len() - shown));
            lines
        }
    }
}

// The length of `sigil_lines` without building them, for a layout cut on every
// round and every mouse motion. The two must agree, or the box is cut for one
// count of lines and filled with another.
fn sigil_line_count(sigils: &Sigils) -> usize {
    match sigils {
        Sigils::Held(held) => held.len().min(SIGILS_MOST_LINES),
        Sigils::Nothing | Sigils::Unknown => 1,
    }
}

fn tree_column(tree: Rect, sigils: &Sigils) -> (Rect, Option<Rect>) {
    let lines = u16::try_from(sigil_line_count(sigils)).unwrap_or(u16::MAX);
    let rows = lines.saturating_add(2 * BORDER_THICKNESS);
    let floor = 2 * BORDER_THICKNESS + TREE_MIN_ROWS;
    if tree.height < rows.saturating_add(floor) {
        return (tree, None);
    }
    let [tree, sigils] =
        Layout::vertical([Constraint::Min(0), Constraint::Length(rows)]).areas(tree);
    (tree, Some(sigils))
}

fn draw_sigils(frame: &mut Frame<'_>, area: Rect, sigils: &Sigils) {
    let block = pane_block(false).title_top(Line::from(SIGILS_TITLE).bold());
    let style = match sigils {
        Sigils::Held(_) => Style::new(),
        Sigils::Nothing | Sigils::Unknown => Style::new().dim(),
    };
    let lines: Vec<Line<'_>> = sigil_lines(sigils)
        .into_iter()
        .map(|line| Line::styled(line, style))
        .collect();
    frame.render_widget(Paragraph::new(lines).block(block), area);
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
        app.refused_line()
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

// Every window warlock puts over the frame, sized off the lines it draws: the
// width is the widest of them, the height is how many there are, and both carry
// the same margins and border. Centred, so every question is answered in the
// same place on the screen — except the one `at_bottom` says otherwise for.
struct Dialog<'a> {
    lines: Vec<Line<'a>>,
    floor: usize,
    bottom: bool,
    title: Option<&'static str>,
    scroll: u16,
}

impl<'a> Dialog<'a> {
    fn of(modal: Modal<'a>) -> Self {
        match modal {
            Modal::Quit(highlighted) => Self::quit(highlighted),
            Modal::Help(scroll) => Self::help().scrolled(scroll),
            Modal::Push(filing) => Self::push(filing),
            Modal::Cut(cutting) => Self::cut(cutting),
            Modal::Pull(undertaking) => Self::pull(undertaking),
            Modal::Review(review) => Self::review(review),
            Modal::Carry(carry) => Self::carry(carry),
            Modal::Scope(field) => Self::scope(field, SCOPE_HEADING, scope::RULES),
            Modal::Record(form) => Self::record(form),
            Modal::Write(field) => Self::scope(field, PATH_HEADING, PATH_RULES),
        }
    }

    fn new(lines: Vec<Line<'a>>) -> Self {
        Self {
            lines,
            floor: 0,
            bottom: false,
            title: None,
            scroll: 0,
        }
    }

    // The review window is the one question asked about text on the panel
    // behind it, so it sits on the bottom edge rather than over the middle of
    // the drafts it asks about.
    fn at_bottom(self) -> Self {
        Self {
            bottom: true,
            ..self
        }
    }

    // A width the text keeps even when no line is that wide yet: the field
    // windows hold a column for the caret whether or not it is drawn there.
    fn at_least(self, floor: usize) -> Self {
        Self { floor, ..self }
    }

    // The one window taller than some terminals, so the one that scrolls. The
    // title carries how to close it rather than a last line, which would scroll
    // away with the rest.
    fn scrolled(self, scroll: u16) -> Self {
        Self { scroll, ..self }
    }

    fn help() -> Self {
        let mut lines = Vec::new();
        for (index, (heading, rows)) in HELP_SECTIONS.iter().enumerate() {
            if index > 0 {
                lines.push(Line::default());
            }
            lines.push(Line::from(*heading).bold());
            let row = |(key, does): &(&str, &'static str)| {
                Line::from(vec![
                    Span::raw(format!("{key:<HELP_KEY_WIDTH$}")),
                    Span::raw(*does).dim(),
                ])
            };
            match rows {
                Rows::Keys(keys) => lines.extend(keys.iter().map(row)),
                Rows::Commands => lines.extend(help_rows().map(row)),
            }
        }

        Self {
            title: Some(HELP_TITLE),
            ..Self::new(lines)
        }
    }

    fn quit(highlighted: Answer) -> Self {
        Self::new(vec![
            Line::from(CONFIRM_QUESTION).centered(),
            Line::default(),
            answers_line(highlighted),
        ])
    }

    fn push(filing: &'a Filing) -> Self {
        Self::new(vec![
            Line::from(PUSH_QUESTION).centered(),
            Line::default(),
            Line::from(filing.project()).bold().centered(),
            labelled(PUSH_TEAM, filing.destination().team_key()),
            labelled(PUSH_KEY, filing.destination().key()),
            Line::default(),
            answers_line(filing.answer()),
        ])
    }

    fn cut(cutting: &'a Cutting) -> Self {
        Self::new(vec![
            Line::from(CUT_QUESTION).centered(),
            Line::default(),
            Line::from(cutting.project()).bold().centered(),
            labelled(CUT_STATUS, cutting.status()),
            labelled(CUT_SLICES, cutting.slices()),
            labelled(PUSH_TEAM, cutting.team()),
            labelled(PUSH_KEY, cutting.key()),
            Line::default(),
            answers_line(cutting.answer()),
        ])
    }

    fn pull(undertaking: &'a Undertaking) -> Self {
        let mut lines = vec![
            Line::from(pull_question(undertaking)).centered(),
            Line::default(),
            Line::from(undertaking.ticket()).bold().centered(),
            Line::from(undertaking.title()).centered(),
            labelled(PULL_SCOPE, undertaking.scope()),
            labelled(PUSH_TEAM, undertaking.team()),
            labelled(PULL_BRANCH, undertaking.branch()),
        ];
        // The one row a fresh pull does not draw, said where the branch is said
        // because it is the same kind of fact: where the next session picks the
        // work up.
        if let Some(from) = pull_from_line(undertaking) {
            lines.push(Line::from(from).dim().centered());
        }
        lines.push(Line::default());
        lines.push(answers_line(undertaking.answer()));

        Self::new(lines)
    }

    fn review(review: &'a Review) -> Self {
        let lines = vec![
            Line::from(REVIEW_QUESTION).centered(),
            Line::default(),
            Line::from(review.slice()).bold().centered(),
            Line::from(review_read_line(review)).dim().centered(),
            Line::default(),
            review_answers_line(review),
        ];

        Self::new(lines).at_bottom()
    }

    fn carry(carry: &Carry) -> Self {
        Self::new(vec![
            Line::from(CARRY_QUESTION).centered(),
            Line::default(),
            Line::from(carry_left_line(carry)).dim().centered(),
            Line::default(),
            answers_line(carry.answer()),
        ])
    }

    // The refusal row is drawn, empty, before any rule is broken, so a refusal
    // does not move the field out from under the reader's eye. The record
    // window keeps one under each of its fields for the same reason.
    fn scope(field: &'a ScopeField, heading: &'a str, rules: &'a str) -> Self {
        Self::new(vec![
            Line::from(vec![
                Span::raw(heading),
                Span::raw(field.directory()).bold(),
            ]),
            Line::default(),
            Line::from(typed_spans(field)),
            Line::from(field.rule().unwrap_or_default()),
            Line::from(rules).dim(),
        ])
        .at_least(display_width(field.text()) + display_width(SCOPE_CURSOR))
    }

    // Its own constructor rather than a third heading-and-rules argument to
    // `scope`: three fields, three refusal rows and a column of labels is a
    // different shape of window.
    fn record(form: &'a RecordForm) -> Self {
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

        // Every field is measured with a caret column, focused or not, so that
        // tabbing between them moves nothing sideways.
        let label = label_width() + display_width(RECORD_LABEL_GAP);
        let floor = RecordField::ALL
            .into_iter()
            .map(|which| label + display_width(form.field(which).text()))
            .max()
            .unwrap_or(0)
            + display_width(SCOPE_CURSOR);

        Self::new(lines).at_least(floor)
    }

    fn size(&self) -> Size {
        let widest = self
            .lines
            .iter()
            .map(Line::width)
            .fold(self.floor, usize::max);
        let rows = u16::try_from(self.lines.len()).unwrap_or(u16::MAX);

        Size::new(
            u16::try_from(widest)
                .unwrap_or(u16::MAX)
                .saturating_add(2 * DIALOG_MARGIN)
                .saturating_add(2 * BORDER_THICKNESS),
            rows.saturating_add(2 * DIALOG_MARGIN_ROWS)
                .saturating_add(2 * BORDER_THICKNESS),
        )
    }

    fn area(&self, screen: Rect) -> Rect {
        let area = centred(screen, self.size());
        if self.bottom {
            Rect {
                y: screen.y + screen.height - area.height,
                ..area
            }
        } else {
            area
        }
    }

    fn draw(self, frame: &mut Frame<'_>, screen: Rect) {
        let area = self.area(screen);
        let mut block =
            Block::bordered().padding(Padding::symmetric(DIALOG_MARGIN, DIALOG_MARGIN_ROWS));
        if let Some(title) = self.title {
            block = block.title_top(Line::from(title).centered());
        }
        let inner = block.inner(area);

        frame.render_widget(Clear, area);
        frame.render_widget(block, area);
        frame.render_widget(Paragraph::new(self.lines).scroll((self.scroll, 0)), inner);
    }
}

/// How far down the help sheet can scroll on a screen this size: far enough to
/// bring its last line to the bottom of the window, and no further.
pub(crate) fn help_most_scroll(size: Size) -> u16 {
    let sheet = Dialog::help();
    let area = sheet.area(Rect::new(0, 0, size.width, size.height));
    let shown = area
        .height
        .saturating_sub(2 * DIALOG_MARGIN_ROWS)
        .saturating_sub(2 * BORDER_THICKNESS);
    u16::try_from(sheet.lines.len())
        .unwrap_or(u16::MAX)
        .saturating_sub(shown)
}

// Which question is being asked, off the one fact that tells the two runs
// apart: see [`PULL_RESUME_QUESTION`].
fn pull_question(undertaking: &Undertaking) -> &'static str {
    if undertaking.resuming().is_some() {
        PULL_RESUME_QUESTION
    } else {
        PULL_QUESTION
    }
}

fn pull_from_line(undertaking: &Undertaking) -> Option<String> {
    undertaking
        .resuming()
        .map(|subtask| format!("{PULL_FROM}{subtask}"))
}

// The same lit and unlit styles as every other question's, so what is under
// the finger reads the same way.
fn review_answers_line(review: &Review) -> Line<'static> {
    let style = |choice: Choice| answer_style(choice == review.choice());

    Line::from(vec![
        Span::styled(REVIEW_CREATE, style(Choice::Create)),
        Span::raw(REVIEW_ANSWER_GAP),
        Span::styled(REVIEW_EDIT, style(Choice::Edit)),
        Span::raw(REVIEW_ANSWER_GAP),
        Span::styled(REVIEW_SKIP, style(Choice::Skip)),
        Span::raw(REVIEW_ANSWER_GAP),
        Span::styled(REVIEW_FEEDBACK, style(Choice::Feedback)),
    ])
    .centered()
}

fn labelled(label: &str, value: impl Display) -> Line<'static> {
    Line::from(format!("{label}{value}")).dim().centered()
}

fn review_read_line(review: &Review) -> String {
    let count = review.titles().len();
    let noun = if count == 1 { "draft" } else { "drafts" };
    format!("{count} {noun}{REVIEW_READ}")
}

fn carry_left_line(carry: &Carry) -> String {
    format!("{}{CARRY_LEFT}", carry.left())
}

fn answers_line(highlighted: Answer) -> Line<'static> {
    let style = |answer: Answer| answer_style(answer == highlighted);

    Line::from(vec![
        Span::styled(CONFIRM_YES, style(Answer::Yes)),
        Span::raw(CONFIRM_ANSWER_GAP),
        Span::styled(CONFIRM_NO, style(Answer::No)),
    ])
    .centered()
}

fn answer_style(lit: bool) -> Style {
    if lit {
        Style::new()
            .fg(FOCUS_COLOUR)
            .add_modifier(Modifier::REVERSED | Modifier::BOLD)
    } else {
        Style::new().add_modifier(Modifier::DIM)
    }
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

/// The text with the caret on the character the cursor names, and past the last
/// one a blank of its own. The windows that draw this reserve the blank's column
/// whether or not the cursor is in the text (see [`Dialog::at_least`]), so a
/// window does not change width as the cursor walks a field.
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

#[cfg(test)]
#[path = "tests/ui.rs"]
mod tests;
