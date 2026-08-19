//! What one frame looks like.
//!
//! The whole screen is three things stacked: a header naming which tree is on
//! screen, the flattened tree itself, one node per line, and a footer carrying
//! the tally and the keys. [`draw`] takes the app state and a frame and
//! nothing else — no terminal setup, no globals, no reaching back into the
//! engine — so what appears on screen is a pure function of what the app state
//! says, and a test can assert it against an in-memory buffer with no tty
//! attached.
//!
//! There is deliberately no scrolling viewport, no expand/collapse and no
//! mouse: the flattened tree is drawn as-is.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
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

/// The tally line and the keys line.
const FOOTER_HEIGHT: u16 = 2;

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
    let [header_area, tree_area, footer_area] = Layout::vertical([
        Constraint::Length(HEADER_HEIGHT),
        Constraint::Min(0),
        Constraint::Length(FOOTER_HEIGHT),
    ])
    .areas(frame.area());

    draw_header(frame, header_area, app);
    draw_tree(frame, tree_area, app);
    draw_footer(frame, footer_area, app);
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

/// Draw every row of the flattened tree, one per line, with the selected one
/// highlighted.
fn draw_tree(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let items: Vec<ListItem<'_>> = app
        .rows()
        .iter()
        .map(|row| ListItem::new(line(row)))
        .collect();
    let list = List::new(items)
        .highlight_symbol(SELECTION_MARKER)
        .highlight_style(Style::new().add_modifier(Modifier::REVERSED | Modifier::BOLD));

    // The selection lives in the app state; this widget-local state is just
    // how a `List` is told about it, and is thrown away with the frame.
    let mut state = ListState::default().with_selected(app.selected_row().map(|_| app.selected()));
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

/// Draw the tally of nodes by state and the keys that do something.
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

    let keys = Line::from("up/down or k/j: move    q / Esc / Ctrl-C: quit").dim();
    frame.render_widget(Paragraph::new(vec![Line::from(tally), keys]), area);
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
    use ratatui::style::{Color, Modifier};
    use warlock_engine::NodeState;

    use super::{FOOTER_HEIGHT, HEADER_HEIGHT, SELECTION_MARKER, draw};
    use crate::app::App;
    use crate::colour::colour_for;
    use crate::fixture;

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
    fn the_footer_shows_the_engines_counts_and_the_keys() {
        let tree = fixture::tree();
        let app = App::from_tree(&tree);
        let height = 10;

        let buffer = render(&app, 60, height);

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
        let keys = row_text(&buffer, height - 1);
        for key in ["up/down", "k/j", "q", "Esc", "Ctrl-C"] {
            assert!(keys.contains(key), "footer {keys:?} is missing {key}");
        }
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
