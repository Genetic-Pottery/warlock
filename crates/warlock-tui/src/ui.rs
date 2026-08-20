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
//! see [`draw_tree`]. Which rows exist at all is the app's too: collapsing a
//! directory takes its descendants out of [`App::rows`], and whatever rows the
//! app holds are the rows drawn — this module only says which of them is
//! collapsed, with a marker on the line. There is deliberately no mouse.

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

    let keys = Line::from(KEYS).dim();

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

    use super::{
        FOOTER_HEIGHT, HEADER_HEIGHT, INDENT, KEYS, NO_MARKER, SELECTION_MARKER, draw, tree_height,
    };
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

    /// A terminal wide enough for the whole of [`KEYS`], whatever it grows to:
    /// the footer test asserts that line for equality, and a line drawn onto a
    /// narrower terminal than it needs would be compared against its own
    /// truncation.
    const KEYS_WIDTH: u16 = 160;

    /// Tall enough for the whole fixture with its files on screen, header and
    /// footer included, so a file test asserts about rows rather than about
    /// where the window happened to stop.
    const FILES_HEIGHT: u16 = 20;

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

        let before = render(&expanded, 40, 10);
        let after = render(&collapsed, 40, 10);

        // Same directory, same indent, same name, and the one thing that
        // differs is the marker saying whether anything is under it.
        assert_eq!(row_text(&before, HEADER_HEIGHT + 1), "    - crates");
        assert_eq!(row_text(&after, HEADER_HEIGHT + 1), "    + crates");
        // And what it was hiding is gone from the screen, leaving the root's
        // other child where the children were.
        let drawn: Vec<String> = rows_text(&after)
            .into_iter()
            .skip(usize::from(HEADER_HEIGHT))
            .take(collapsed.rows().len())
            .collect();
        assert_eq!(drawn, ["> - warlock", "    + crates", "      assets"]);
    }

    #[test]
    fn a_directory_with_nothing_under_it_carries_neither_marker() {
        let app = App::from_tree(&fixture::tree());

        let buffer = render(&app, 40, 10);

        // `assets` has no children, so it is neither collapsed nor expanded and
        // says so by carrying no marker — while still lining its name up with
        // the marked rows at its own depth.
        let leaf = row_text(&buffer, HEADER_HEIGHT + 4);
        assert_eq!(leaf, "      assets");
        assert!(!leaf.contains('+') && !leaf.contains('-'), "{leaf:?}");
        assert_eq!(
            leaf.find("assets"),
            row_text(&buffer, HEADER_HEIGHT + 1).find("crates"),
            "a leaf's name should start where a sibling directory's does"
        );
        // Pressing space on it changes nothing on screen: nothing to hide.
        let pressed = App::from_tree(&fixture::tree()).with_collapsed(["warlock/assets"]);
        assert_eq!(rows_text(&render(&pressed, 40, 10)), rows_text(&buffer));
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
    fn a_file_row_is_drawn_in_the_colour_of_the_directory_holding_it() {
        let tree = fixture::tree();
        let mut app = App::from_tree(&tree);
        app.toggle_files();

        let buffer = render(&app, 40, FILES_HEIGHT);

        let mut colours = Vec::new();
        for (index, row) in app.rows().iter().enumerate() {
            if !row.is_file() {
                continue;
            }
            let y = HEADER_HEIGHT + u16::try_from(index).expect("the fixture tree is small");
            // Asked of the engine's tree rather than of the row, so what is
            // under test is that the colour on screen is the *directory's*
            // state and not merely whatever the row happened to be built with.
            let directory = row.path.parent().expect("a file sits in a directory");
            let node = tree.find(directory).expect("the file came from that node");
            let colour = first_glyph_colour(&buffer, y);
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
        let before = render(&app, 40, FILES_HEIGHT);

        app.toggle_files();
        let buffer = render(&app, 40, FILES_HEIGHT);

        let drawn: Vec<String> = rows_text(&buffer)
            .into_iter()
            .skip(usize::from(HEADER_HEIGHT))
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
            rows_text(&render(&app, 40, FILES_HEIGHT)),
            rows_text(&before)
        );
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
        assert_eq!(row_text(&buffer, HEADER_HEIGHT + 1), ">   - crates");
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
        let height = 10;

        let before = render(&app, 120, height);
        app.toggle_pacted_only();
        let after = render(&app, 120, height);

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
        assert_eq!(row_text(&buffer, HEADER_HEIGHT), "> - warlock");
    }
}
