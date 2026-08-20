//! What one frame looks like.
//!
//! The whole screen is three things stacked: a header naming which tree is on
//! screen, the flattened tree itself, one node per line, and a footer carrying
//! the tally, the keys and whatever the app has to say about the last
//! keystroke. [`draw`] takes the app state and a frame and nothing else — no
//! terminal setup, no globals, no reaching back into the engine — so what
//! appears on screen is a pure function of what the app state says, and a test
//! can assert it against an in-memory buffer with no tty attached.
//!
//! The tree area is a window onto the flattened rows: it draws the slice
//! starting at the app's scroll offset and running for as many rows as the area
//! is tall, so a tree taller than the terminal scrolls under a header and a
//! footer that stay where they are. The window is the app's, not the widget's —
//! see [`draw_tree`]. There is deliberately no expand/collapse and no mouse:
//! whatever rows the app holds are the rows drawn.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect, Size};
use ratatui::style::{Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{List, ListItem, ListState, Paragraph};
use warlock_engine::NodeState;

use crate::app::{App, Row};
use crate::colour::colour_for;

/// One level of nesting, per unit of the depth the engine's walk yields.
const INDENT: &str = "  ";

/// Drawn to the left of the selected row. The reversed highlight already says
/// where the selection is on any terminal with colour; the marker says it
/// again for the ones without.
const SELECTION_MARKER: &str = "> ";

/// The one line naming the tree's root.
const HEADER_HEIGHT: u16 = 1;

/// The tally line, the keys line and the message line.
///
/// The message line is there whether or not there is a message to put on it: a
/// footer that grew a line when the app had something to say would shove the
/// tree down a row and reflow the whole window on a keystroke that changed
/// nothing about the tree.
const FOOTER_HEIGHT: u16 = 3;

/// Draw the whole frame: the header at the top, the tree between, the footer
/// below.
///
/// Pure in the sense that matters here — it reads `app` and writes `frame`,
/// touching no terminal state of its own.
///
/// The tree takes `Min(0)` and so gives its rows up first when the terminal is
/// short: on a screen with no room for everything, which nodes are off the
/// bottom matters less than still being told which tree they belong to and
/// how to get out.
pub fn draw(frame: &mut Frame<'_>, app: &App) {
    let [header_area, tree_area, footer_area] = areas(frame.area());

    draw_header(frame, header_area, app);
    draw_tree(frame, tree_area, app);
    draw_footer(frame, footer_area, app);
}

/// The three areas one frame is cut into, top to bottom: header, tree, footer.
///
/// Split out so that [`tree_height`] answers the same question [`draw`] does,
/// from the same call: a caller that told the app one height while the frame
/// used another would scroll by a window that is not on screen.
fn areas(area: Rect) -> [Rect; 3] {
    Layout::vertical([
        Constraint::Length(HEADER_HEIGHT),
        Constraint::Min(0),
        Constraint::Length(FOOTER_HEIGHT),
    ])
    .areas(area)
}

/// How many rows of tree a terminal of `size` has room for, once the header and
/// the footer have taken theirs.
///
/// This is what [`App::set_viewport_height`] wants, and the only reason it is
/// public: the app's scroll offset is only right if it was computed against the
/// height the next frame actually gives the tree, and the layout is the one
/// thing that knows that height. The caller asks before it draws, so the offset
/// the frame reads was computed for the frame being drawn.
#[must_use]
pub fn tree_height(size: Size) -> u16 {
    areas(Rect::from(size))[1].height
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
        .map(|row| ListItem::new(line(row)))
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

/// One line of the tree: indented by depth, named by its last path component,
/// coloured by its state.
///
/// The indentation already spells out the ancestry, so repeating the full
/// path on every line would be noise; a path with no final component (a bare
/// root) falls back to printing itself, because a blank line is worse than a
/// long one.
fn line(row: &Row) -> Line<'static> {
    let name = row
        .path
        .file_name()
        .unwrap_or(row.path.as_os_str())
        .to_string_lossy();

    Line::styled(
        format!("{}{name}", INDENT.repeat(row.depth)),
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

    // The movement keys first and together, in the order a reader reaches for
    // them: one row, one screen, the whole tree.
    let keys = Line::from(
        "up/down or k/j: move    PgUp/PgDn: page    g/G: first/last    \
         p: pact    q / Esc / Ctrl-C: quit",
    )
    .dim();

    let message = Line::from(app.message().unwrap_or_default().to_owned()).dim();
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

    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;
    use ratatui::layout::Size;
    use ratatui::style::{Color, Modifier};
    use warlock_engine::NodeState;

    use super::{FOOTER_HEIGHT, HEADER_HEIGHT, INDENT, SELECTION_MARKER, draw, tree_height};
    use crate::app::{App, Row};
    use crate::colour::colour_for;
    use crate::fixture;

    /// How many rows the window tests work with: comfortably more than fit on
    /// the terminal they draw into.
    const MANY: usize = 20;

    /// The terminal those tests draw into — tall enough for a header, a footer
    /// and a handful of tree rows, and no taller.
    const WIDTH: u16 = 40;
    /// See [`WIDTH`].
    const HEIGHT: u16 = 8;

    /// `count` rows of nothing in particular, named so that a line on screen
    /// says which row of the tree it is.
    fn many_rows(count: usize) -> Vec<Row> {
        (0..count)
            .map(|index| {
                Row::new(
                    1,
                    format!("repo/module{index}"),
                    format!("repo/module{index}/README.md"),
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

    /// The rows of `buffer` the tree is drawn into: everything between the
    /// header and the footer, wherever those two leave it.
    fn tree_rows(buffer: &Buffer) -> Vec<String> {
        (HEADER_HEIGHT..buffer.area.height - FOOTER_HEIGHT)
            .map(|y| row_text(buffer, y))
            .collect()
    }

    /// The header block of `buffer`, and its footer block: the two that must
    /// not move when the tree scrolls.
    fn header_and_footer(buffer: &Buffer) -> (Vec<String>, Vec<String>) {
        let height = buffer.area.height;
        (
            (0..HEADER_HEIGHT).map(|y| row_text(buffer, y)).collect(),
            (height - FOOTER_HEIGHT..height)
                .map(|y| row_text(buffer, y))
                .collect(),
        )
    }

    /// The line row `index` of [`many_rows`] is drawn as when `selected` is the
    /// selected row: the marker's gutter, the depth's indent, then the name.
    fn drawn_row(index: usize, selected: usize) -> String {
        let gutter = if index == selected {
            SELECTION_MARKER.to_owned()
        } else {
            " ".repeat(SELECTION_MARKER.chars().count())
        };
        format!("{gutter}{INDENT}module{index}")
    }

    /// Draw `app` onto an in-memory terminal of the given size and hand back
    /// the buffer. No tty is involved, so this runs anywhere `cargo test` does.
    fn render(app: &App, width: u16, height: u16) -> Buffer {
        let mut terminal =
            Terminal::new(TestBackend::new(width, height)).expect("test backend never fails");
        terminal
            .draw(|frame| draw(frame, app))
            .expect("test backend never fails");
        terminal.backend().buffer().clone()
    }

    /// The text of one row of a buffer, trailing blanks trimmed off.
    fn row_text(buffer: &Buffer, y: u16) -> String {
        let text: String = (0..buffer.area.width)
            .map(|x| buffer[(x, y)].symbol())
            .collect();
        text.trim_end().to_string()
    }

    /// Every row of a buffer, as text.
    fn rows_text(buffer: &Buffer) -> Vec<String> {
        (0..buffer.area.height)
            .map(|y| row_text(buffer, y))
            .collect()
    }

    /// The foreground colour of a row's first glyph, ignoring the gutter the
    /// selection marker lives in: that gutter is drawn by the list itself and
    /// takes no state colour, while everything to the right of it is the row's
    /// own text, however far it is indented.
    fn first_glyph_colour(buffer: &Buffer, y: u16) -> Color {
        let gutter = u16::try_from(SELECTION_MARKER.chars().count()).expect("a two-char marker");
        (gutter..buffer.area.width)
            .map(|x| &buffer[(x, y)])
            .find(|cell| !cell.symbol().trim().is_empty())
            .expect("row has a glyph on it")
            .fg
    }

    #[test]
    fn every_node_gets_its_own_line_indented_by_depth_in_walk_order() {
        let app = App::from_tree(&fixture::tree());

        let buffer = render(&app, 40, 10);

        let drawn: Vec<String> = rows_text(&buffer)
            .into_iter()
            .skip(usize::from(HEADER_HEIGHT))
            .take(app.rows().len())
            .collect();
        assert_eq!(
            drawn,
            [
                "> warlock",
                "    crates",
                "      engine",
                "      tui",
                "    assets",
            ]
        );
    }

    #[test]
    fn each_line_is_drawn_in_its_states_colour() {
        let app = App::from_tree(&fixture::tree());

        let buffer = render(&app, 40, 10);

        // Including the selected row: the highlight only adds modifiers, so
        // the row's text keeps its state's colour underneath.
        for (index, row) in app.rows().iter().enumerate() {
            let y = HEADER_HEIGHT + u16::try_from(index).expect("the fixture tree is small");
            assert_eq!(
                first_glyph_colour(&buffer, y),
                colour_for(row.state),
                "row {index} ({}) is drawn in the wrong colour",
                row.path.display()
            );
        }
    }

    #[test]
    fn exactly_one_line_is_highlighted_and_it_is_the_selected_one() {
        let mut app = App::from_tree(&fixture::tree());
        app.select_next();
        app.select_next();

        let buffer = render(&app, 40, 10);

        let highlighted: Vec<u16> = (0..buffer.area.height)
            .filter(|&y| buffer[(0, y)].modifier.contains(Modifier::REVERSED))
            .collect();
        // The third row of the tree, one row down from where it would sit
        // without the header above it.
        assert_eq!(highlighted, [HEADER_HEIGHT + 2]);
        assert!(row_text(&buffer, HEADER_HEIGHT + 2).starts_with(SELECTION_MARKER));
    }

    #[test]
    fn the_selection_marker_moves_with_the_selection() {
        let mut app = App::from_tree(&fixture::tree());
        app.select_next();

        let buffer = render(&app, 40, 10);

        assert!(!row_text(&buffer, HEADER_HEIGHT).starts_with(SELECTION_MARKER));
        assert_eq!(row_text(&buffer, HEADER_HEIGHT + 1), ">   crates");
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
            let highlighted: Vec<u16> = (0..buffer.area.height)
                .filter(|&y| buffer[(0, y)].modifier.contains(Modifier::REVERSED))
                .collect();
            assert_eq!(
                highlighted,
                [HEADER_HEIGHT
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
        // tests use, and one somewhere above it.
        for height in [HEADER_HEIGHT + FOOTER_HEIGHT + 1, HEIGHT, 15] {
            let measured = tree_height(Size::new(WIDTH, height));
            let mut app = App::from_rows(many_rows(MANY));
            app.set_viewport_height(measured);

            let buffer = render(&app, WIDTH, height);

            assert_eq!(measured, height - HEADER_HEIGHT - FOOTER_HEIGHT);
            let drawn = tree_rows(&buffer);
            assert_eq!(drawn.len(), usize::from(measured), "in {height} rows");
            assert!(
                drawn.iter().all(|line| !line.is_empty()),
                "a taller tree left blank rows in {height}: {drawn:?}"
            );
        }
    }

    #[test]
    fn the_footer_shows_the_engines_counts_and_the_keys() {
        let tree = fixture::tree();
        let app = App::from_tree(&tree);
        let height = 10;

        // Wide enough for the whole key line: what is under test is which keys
        // it lists, not how it survives a narrow terminal.
        let buffer = render(&app, 120, height);

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
        // "p: pact" and not the bare "p", which "up/down" would satisfy.
        for key in [
            "up/down",
            "k/j",
            "PgUp",
            "PgDn",
            "page",
            "g/G",
            "first/last",
            "p: pact",
            "q",
            "Esc",
            "Ctrl-C",
        ] {
            assert!(keys.contains(key), "footer {keys:?} is missing {key}");
        }
    }

    #[test]
    fn a_refused_toggle_shows_its_message_on_the_footers_last_line_until_the_next_key() {
        let mut app = App::from_tree(&fixture::tree());
        // Onto the directory with no documentation yet, which `p` refuses.
        app.select_next();
        assert!(
            app.selected_row()
                .expect("a row is selected")
                .readme
                .is_none(),
            "the fixture's second row is the one with no README"
        );

        assert!(app.toggle_pact().is_none(), "the toggle should be refused");

        // Wide enough for the whole sentence: what is under test is that it is
        // on screen, not how it survives a narrow terminal.
        let height = 10;
        let buffer = render(&app, 120, height);
        let said = app.message().expect("a refusal says why").to_owned();
        assert!(said.contains("has no README"), "{said:?}");
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
    fn an_empty_app_draws_a_footer_and_no_rows() {
        // No scope either, so the header is blank along with the tree: an app
        // nobody told where its tree came from claims nothing.
        let app = App::from_rows(Vec::new());
        let height = 6;

        let buffer = render(&app, 40, height);

        for y in 0..height - FOOTER_HEIGHT {
            assert_eq!(row_text(&buffer, y), "", "row {y} should be blank");
        }
        assert!(row_text(&buffer, height - FOOTER_HEIGHT).contains("0 unpacted"));
    }

    #[test]
    fn the_header_names_the_root_relative_to_the_repository_root() {
        let app = App::from_tree(&fixture::tree()).with_scope(
            Path::new("/repo"),
            Path::new("/repo").join("crates").join("warlock-engine"),
        );

        let buffer = render(&app, 40, 10);

        // Forward slashes whatever the platform's separator is, because this
        // is the engine's manifest spelling of a module.
        assert_eq!(row_text(&buffer, 0), "crates/warlock-engine");
    }

    #[test]
    fn a_tree_rooted_at_the_repository_root_gets_a_named_header_not_a_blank_one() {
        let app = App::from_tree(&fixture::tree()).with_scope("/repo", "/repo");

        let buffer = render(&app, 40, 10);

        assert_eq!(row_text(&buffer, 0), "(repository root)");
    }

    #[test]
    fn the_header_sits_above_the_first_tree_row() {
        let app = App::from_tree(&fixture::tree()).with_scope("/repo", "/repo/crates");

        let buffer = render(&app, 40, 10);

        assert_eq!(row_text(&buffer, 0), "crates");
        assert_eq!(row_text(&buffer, HEADER_HEIGHT), "> warlock");
    }
}
