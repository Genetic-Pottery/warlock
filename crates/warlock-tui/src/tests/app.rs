use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use warlock_engine::{Node, Tree};

use super::{
    App, Chrome, Focus, Held, OpenScopes, Row, cut_at_cap_message, no_document_message, reseat_on,
    scroll_offset_for,
};
use crate::account::Line;
use crate::claude::Activity;
use crate::fixture;
use crate::panel::{Mode, Showing, panel_offset_for};
use crate::selection::{Position, copied_text};
use crate::thread::Ending;

const MANY: usize = 20;
const WINDOW: u16 = 5;

type Movement = fn(&mut App);

const MOVEMENTS: [(&str, Movement); 6] = [
    ("select_previous", App::select_previous),
    ("select_next", App::select_next),
    ("select_page_up", App::select_page_up),
    ("select_page_down", App::select_page_down),
    ("select_first", App::select_first),
    ("select_last", App::select_last),
];

const KEYSTROKES: [(&str, Movement); 9] = [
    ("select_previous", App::select_previous),
    ("select_next", App::select_next),
    ("select_page_up", App::select_page_up),
    ("select_page_down", App::select_page_down),
    ("select_first", App::select_first),
    ("select_last", App::select_last),
    ("toggle_collapsed", App::toggle_collapsed),
    ("toggle_scoped_only", App::toggle_scoped_only),
    ("toggle_files", App::toggle_files),
];

fn three_rows() -> Vec<Row> {
    vec![
        Row::new(0, "repo", false),
        Row::new(1, "repo/crates", true).with_scope(Some("platform".to_owned())),
        Row::new(1, "repo/assets", false),
    ]
}

fn rooted_rows() -> Vec<Row> {
    vec![
        Row::new(0, "/repo", false),
        Row::new(1, "/repo/crates", false),
        Row::new(2, "/repo/crates/warlock-engine", true).with_scope(Some("data-plane".to_owned())),
    ]
}

// Joined rather than spelled out as one literal: what these tests are
// holding the label to is that a path carrying the platform's own separator
// comes back in the engine's forward-slash manifest spelling, and a literal
// would hand over forward slashes to begin with and prove nothing.
fn rooted_crates() -> PathBuf {
    Path::new("/repo").join("crates")
}

fn rooted_engine() -> PathBuf {
    rooted_crates().join("warlock-engine")
}

fn tally(app: &App) -> (usize, usize) {
    let directories = app.rows().iter().filter(|row| !row.is_file());
    let scoped = directories.clone().filter(|row| row.covered).count();
    (scoped, directories.count() - scoped)
}

fn many_rows(count: usize) -> Vec<Row> {
    (0..count)
        .map(|index| Row::new(1, format!("repo/module{index}"), false))
        .collect()
}

fn scrolled_to(selected: usize) -> App {
    let mut app = App::from_rows(many_rows(MANY));
    app.set_viewport_height(WINDOW);
    while app.selected() < selected {
        app.select_next();
    }
    app
}

fn drawn(app: &App) -> Vec<String> {
    app.rows()
        .iter()
        .map(|row| row.path.to_string_lossy().into_owned())
        .collect()
}

fn node_paths(app: &App) -> Vec<String> {
    app.rows()
        .iter()
        .filter(|row| !row.is_file())
        .map(|row| row.path.to_string_lossy().into_owned())
        .collect()
}

fn whole_fixture() -> Vec<String> {
    vec![
        "warlock".to_owned(),
        "warlock/crates".to_owned(),
        "warlock/crates/engine".to_owned(),
        "warlock/crates/tui".to_owned(),
        "warlock/assets".to_owned(),
    ]
}

fn scoped_fixture() -> Vec<String> {
    vec![
        "warlock".to_owned(),
        "warlock/crates".to_owned(),
        "warlock/crates/engine".to_owned(),
        "warlock/crates/tui".to_owned(),
    ]
}

fn collapsed_over_crates() -> Vec<String> {
    vec![
        "warlock".to_owned(),
        "warlock/crates".to_owned(),
        "warlock/assets".to_owned(),
    ]
}

fn whole_fixture_with_files() -> Vec<String> {
    vec![
        "warlock".to_owned(),
        "warlock/README.md".to_owned(),
        "warlock/crates".to_owned(),
        "warlock/crates/engine".to_owned(),
        "warlock/crates/engine/Cargo.toml".to_owned(),
        "warlock/crates/tui".to_owned(),
        "warlock/crates/tui/Cargo.toml".to_owned(),
        "warlock/assets".to_owned(),
        "warlock/assets/logo.svg".to_owned(),
    ]
}

fn app_selecting(path: &str) -> App {
    select(App::from_tree(&fixture::tree()), path)
}

fn app_with_files_selecting(path: &str) -> App {
    let mut app = App::from_tree(&fixture::tree());
    app.toggle_files();
    select(app, path)
}

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
        Node::new("repo").with_children([
            Node::new("repo/crates")
                .with_scope(Some("platform".to_owned()))
                .with_children([
                    Node::new("repo/crates/engine").with_covering(Some("platform".to_owned()))
                ]),
            Node::new("repo/assets"),
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
fn flattening_a_tree_keeps_every_node_and_whether_a_scope_covers_it() {
    let tree = fixture::tree();

    let app = App::from_tree(&tree);

    assert_eq!(node_paths(&app).len(), tree.walk().count());
    for row in app.rows().iter().filter(|row| !row.is_file()) {
        let node = tree.find(&row.path).expect("row came from the tree");
        assert_eq!(row.covered, node.is_covered(), "{}", row.path.display());
    }
}

#[test]
fn an_app_counts_scoped_and_unscoped_directories_across_the_whole_walk() {
    let app = App::from_tree(&fixture::tree());

    // `crates`, `engine` and `tui` are covered; the root and `assets` are not.
    // Files are not directories and are not counted, shown or hidden.
    assert_eq!(app.coverage(), (3, 2));
    let mut with_files = app.clone();
    with_files.toggle_files();
    assert_eq!(with_files.coverage(), (3, 2));
}

#[test]
fn an_app_built_from_bare_rows_counts_the_rows_it_was_handed() {
    assert_eq!(App::from_rows(three_rows()).coverage(), (1, 2));
    assert_eq!(App::from_rows(Vec::new()).coverage(), (0, 0));
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

fn held(open: OpenScopes) -> Held {
    Held { open, base: None }
}

#[test]
fn a_chrome_nobody_built_states_nothing_at_all() {
    let chrome = Chrome::default();

    assert_eq!(chrome.header(), "");
    assert_eq!(chrome.open(), &OpenScopes::Nothing);
}

#[test]
fn the_header_is_not_app_state_and_no_app_carries_one() {
    // The structural claim this type exists for, and the reason there is no
    // `App::header` to assert against: both halves of the header are
    // resolved once and cannot change while warlock runs, so an app rebuilt
    // on every reload has no business holding either. What is left to check
    // is that building one costs an app nothing — a `Chrome` is a function
    // of two paths and a config, and there is no `App` anywhere in it.
    let chrome =
        Chrome::of("/repo", "/repo/crates").with_held(held(OpenScopes::named(["billing"])));

    assert_eq!(chrome.header(), "crates");
    assert_eq!(
        chrome.open(),
        &OpenScopes::Named(vec!["billing".to_owned()])
    );
}

#[test]
fn a_chrome_that_was_never_told_what_is_open_states_nothing() {
    // The default, and the state a reader who has never run `warlock
    // open` is in: there is no wording for it at all, so there is nothing
    // for the header to join onto the line it already had.
    let chrome = Chrome::of("/repo", "/repo/crates");

    assert_eq!(chrome.open(), &OpenScopes::Nothing);
}

#[test]
fn the_open_scopes_are_listed_in_the_order_the_config_has_them() {
    let chrome = Chrome::default().with_held(held(OpenScopes::named(["billing", "web"])));

    assert_eq!(
        chrome.open(),
        &OpenScopes::Named(vec!["billing".to_owned(), "web".to_owned()])
    );
}

#[test]
fn a_missing_config_and_an_empty_set_are_the_same_nothing() {
    // Two ways for AI to work in no scope, and the header must not tell them
    // apart: a machine that closed every scope and one that never opened any
    // have the same nothing.
    let empty: [String; 0] = [];

    assert_eq!(OpenScopes::named(empty), OpenScopes::Nothing);
    assert_eq!(OpenScopes::named(Vec::<String>::new()), OpenScopes::Nothing);
}

#[test]
fn a_config_that_would_not_read_is_said_out_loud_rather_than_looking_absent() {
    // The whole of why this is not an `Option<Vec<String>>`: broken and
    // absent mean opposite things about what is on disk, so they must not
    // draw the same line.
    let unknown = OpenScopes::Unknown;

    assert_ne!(unknown, OpenScopes::Nothing);
    assert_ne!(unknown, OpenScopes::named(["billing"]));
    assert!(unknown.as_slice().is_empty());
}

#[test]
fn no_scope_open_leaves_the_header_byte_for_byte_what_it_was() {
    // No scope open, in either spelling of nothing, leaves the header
    // exactly as it was and gives no line of open scopes.
    let told = Chrome::of("/repo", "/repo/crates/engine");
    let before = told.header().to_owned();

    for nothing in [OpenScopes::Nothing, OpenScopes::named(Vec::<String>::new())] {
        let chrome = told.clone().with_held(held(nothing));

        assert_eq!(chrome.header(), before);
        assert_eq!(chrome.open(), &OpenScopes::Nothing);
    }
}

#[test]
fn what_is_open_changes_nothing_but_what_the_header_states() {
    // The identity half of the header is not touched by the open scopes. That no
    // *row* is coloured, filtered, sorted or re-ordered by one used to be
    // asserted here too, over an `App`; it is now true by construction,
    // because a `Chrome` has no rows to reach.
    let chrome = Chrome::of("/repo", "/repo/crates");

    let opened = chrome
        .clone()
        .with_held(held(OpenScopes::named(["billing"])));

    assert_eq!(opened.header(), chrome.header());
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

    // Pinned whole: every file the fixture lists, once each, in path order,
    // directly after the directory listing it and one level deeper.
    assert_eq!(
        shape(&app),
        [
            ("warlock".to_owned(), 0, false),
            ("warlock/README.md".to_owned(), 1, true),
            ("warlock/crates".to_owned(), 1, false),
            ("warlock/crates/engine".to_owned(), 2, false),
            ("warlock/crates/engine/Cargo.toml".to_owned(), 3, true),
            ("warlock/crates/tui".to_owned(), 2, false),
            ("warlock/crates/tui/Cargo.toml".to_owned(), 3, true),
            ("warlock/assets".to_owned(), 1, false),
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
fn the_next_keystroke_clears_the_message() {
    let mut app = app_with_files_selecting("warlock/assets/logo.svg");
    assert_eq!(app.scope_target(), None);
    assert!(app.message().is_some());

    app.select_next();

    assert_eq!(app.message(), None);
}

#[test]
fn every_movement_clears_a_message() {
    for (name, movement) in MOVEMENTS {
        let mut app = app_selecting("warlock/crates");
        app.set_message("something to forget");

        movement(&mut app);

        assert_eq!(app.message(), None, "{name} left the message behind");
    }
}

#[test]
fn every_keystroke_clears_a_message() {
    for (name, keystroke) in KEYSTROKES {
        // `warlock/crates` has children, so the collapse key is a keystroke
        // that does something here rather than a no-op.
        let mut app = app_selecting("warlock/crates");
        app.set_message("something to forget");

        keystroke(&mut app);

        assert_eq!(app.message(), None, "{name} left the message behind");
    }
}

#[test]
fn a_row_is_labelled_relative_to_the_root_in_the_manifest_spelling() {
    let app = App::from_rows(rooted_rows());

    assert_eq!(app.label_for(&rooted_engine()), "crates/warlock-engine");
    assert_eq!(app.label_for(&rooted_crates()), "crates");
    // The root itself is no relative path at all, so it is named as it
    // stands rather than as a dot.
    assert_eq!(app.label_for(Path::new("/repo")), "/repo");
}

fn tree_with_one_scoped_directory() -> Tree {
    Tree::new(
        Node::new("repo")
            .with_scope(Some("tui-team".to_owned()))
            .with_files([PathBuf::from("repo/README.md")])
            .with_children([
                Node::new("repo/crates").with_covering(Some("tui-team".to_owned())),
                Node::new("repo/notes").with_covering(Some("tui-team".to_owned())),
            ]),
    )
}

#[test]
fn flattening_a_tree_carries_each_nodes_own_scope_onto_its_row() {
    let mut app = App::from_tree(&tree_with_one_scoped_directory());
    app.toggle_files();

    let scoped: Vec<(String, Option<&str>, bool)> = app
        .rows()
        .iter()
        .map(|row| {
            (
                row.path.to_string_lossy().into_owned(),
                row.scope.as_deref(),
                row.covered,
            )
        })
        .collect();
    assert_eq!(
        scoped,
        [
            ("repo".to_owned(), Some("tui-team"), true),
            // A file is scoped by nothing of its own, but it is covered by
            // its directory's scope and so takes its directory's colour.
            ("repo/README.md".to_owned(), None, true),
            // The label marks where a boundary starts, so nothing is
            // inherited down: covered, and carrying no scope.
            ("repo/crates".to_owned(), None, true),
            ("repo/notes".to_owned(), None, true),
        ]
    );
}

#[test]
fn a_scope_on_a_row_moves_nothing_else_and_needs_no_tree() {
    let plain = Row::new(0, "repo", true);
    let scoped = plain.clone().with_scope(Some("tui-team".to_owned()));

    assert_eq!(plain.scope, None);
    assert_eq!(scoped.scope.as_deref(), Some("tui-team"));
    assert_eq!(scoped.covered, plain.covered);
    assert_eq!(scoped.clone().with_scope(None), plain);

    let app = App::from_rows(vec![scoped]);
    assert_eq!(app.rows()[0].scope.as_deref(), Some("tui-team"));
}

#[test]
fn scoping_a_directory_hands_it_back_and_changes_nothing_at_all() {
    // Scoped, covered from above, and under no scope at all: the key opens the
    // prompt on every one of them, since an unscoped directory is where a
    // first scope is drawn.
    for path in [
        "warlock",
        "warlock/crates",
        "warlock/crates/tui",
        "warlock/crates/engine",
        "warlock/assets",
    ] {
        let mut app = app_selecting(path);
        app.set_message("something the last keystroke said");
        let before = app.clone();

        let asked = app.scope_target();

        assert_eq!(asked, Some(PathBuf::from(path)), "{path}");
        // Nothing whatever moved — including the message line, which this
        // key alone leaves standing: a prompt opened and dismissed with Esc
        // has to leave the app exactly as it was found.
        assert_eq!(app, before, "an accepted press moved something on {path}");
    }
}

#[test]
fn a_file_row_is_refused_by_the_scope_key_and_nothing_else_moves() {
    for path in [
        "warlock/assets/logo.svg",
        "warlock/crates/engine/Cargo.toml",
    ] {
        let mut app = app_with_files_selecting(path);
        let mut before = app.clone();

        assert_eq!(app.scope_target(), None, "{path} opened something");

        let message = app.message().expect("a file row is refused out loud");
        assert_eq!(
            message,
            format!(
                "{path} is a file — scopes are drawn on the directory holding it, not on a file"
            )
        );
        before.set_message(message);
        assert_eq!(app, before, "refusing {path} moved something else");
    }
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

    app.set_message("could not write the scope manifest");

    assert_eq!(app.message(), Some("could not write the scope manifest"));
    app.select_next();
    assert_eq!(app.message(), None);
}

#[test]
fn setting_the_mode_changes_the_mode_and_nothing_else() {
    let base = Instant::now();
    let mut app = App::from_rows(rooted_rows());
    app.panel_mut().start_turn("what does this do", base);
    app.panel_mut()
        .answer_turn("it walks the tree", at(base, 1));

    let before = app.panel().window(at(base, 2));

    // Chat until somebody says otherwise, and the change is reported.
    assert_eq!(app.panel().mode(), Mode::Chat);
    assert!(
        app.panel_mut().set_mode(Mode::Brief),
        "chat to brief is a change"
    );
    assert_eq!(app.panel().mode(), Mode::Brief);

    // Nothing else moved: same rows in the same order, same card — the mode
    // wrote no note and started no turn of its own.
    assert_eq!(app.panel().window(at(base, 2)), before);
    assert!(
        app.panel().showing_thread(),
        "the mode moved the card showing"
    );

    // Setting the mode it is already in is no change, and says so: that is
    // how a re-sent instruction knows to add no second note.
    assert!(
        !app.panel_mut().set_mode(Mode::Brief),
        "brief to brief is no change"
    );
    assert_eq!(app.panel().mode(), Mode::Brief);
    assert!(
        app.panel_mut().set_mode(Mode::Chat),
        "brief to chat is a change"
    );
    assert_eq!(app.panel().window(at(base, 2)), before);
}

#[test]
fn a_fresh_app_has_nothing_collapsed_and_draws_the_whole_walk() {
    let app = App::from_tree(&fixture::tree());

    assert!(app.collapsed().is_empty());
    assert_eq!(drawn(&app), whole_fixture());
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
    // order, same depths, same coverage.
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
    assert_eq!(app.selected(), 4);

    let app = app.with_collapsed(["warlock/crates"]);

    // Two rows fewer above it, and the same node under it.
    assert_eq!(app.selected(), 2);
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
    let mut app = select(
        App::from_rows(vec![
            Row::new(0, "repo", false),
            Row::new(1, "repo/crates", false),
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
fn collapsing_leaves_the_coverage_count_alone() {
    let mut app = App::from_tree(&fixture::tree());
    let coverage = app.coverage();

    app.select_first();
    app.toggle_collapsed();

    // Four of the five rows are hidden; the footer still describes the
    // whole tree.
    assert_eq!(app.rows().len(), 1);
    assert_eq!(app.coverage(), coverage);
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

fn window_is_in_range(app: &App) -> bool {
    app.scroll_offset() <= app.rows().len().saturating_sub(app.viewport_height())
}

#[test]
fn a_collapse_clears_the_last_keystrokes_message() {
    let mut app = app_selecting("warlock");
    app.set_message("something from the last keystroke");

    app.toggle_collapsed();

    assert_eq!(app.message(), None);
}

#[test]
fn collapsing_a_directory_leaves_a_sibling_whose_name_it_prefixes_alone() {
    let tree = Tree::new(Node::new("repo").with_children([
        Node::new("repo/crates").with_children([Node::new("repo/crates/engine")]),
        Node::new("repo/crates-old"),
    ]));
    let mut app = App::from_tree(&tree);

    app.select_next();
    app.toggle_collapsed();

    // A sibling is not a descendant, however much of its name it shares.
    assert_eq!(drawn(&app), ["repo", "repo/crates", "repo/crates-old"]);
}

#[test]
fn rows_from_a_bare_list_collapse_only_where_a_deeper_row_follows() {
    let mut app = App::from_rows(vec![
        Row::new(0, "repo", false),
        Row::new(1, "repo/crates", false),
    ]);

    app.toggle_collapsed();
    assert_eq!(drawn(&app), ["repo"]);

    app.toggle_collapsed();
    assert_eq!(drawn(&app), ["repo", "repo/crates"]);
    // The child, with nothing deeper after it, is not collapsible.
    app.select_next();
    app.toggle_collapsed();
    assert!(app.collapsed().is_empty());
}

#[test]
fn a_fresh_app_draws_the_whole_tree_rather_than_only_the_scoped_part() {
    let app = App::from_tree(&fixture::tree());

    assert!(!app.scoped_only());
    assert_eq!(drawn(&app), whole_fixture());
}

#[test]
fn the_filter_keeps_the_covered_nodes_and_the_way_to_them_and_nothing_else() {
    let mut app = app_selecting("warlock/crates/tui");
    let before = app.clone();

    app.toggle_scoped_only();

    assert!(app.scoped_only());
    assert_eq!(drawn(&app), scoped_fixture());
    // Every drawn row is covered or is on the way to one that is, and every
    // covered node in the tree is drawn.
    for row in app.rows() {
        assert!(
            row.covered
                || app
                    .rows()
                    .iter()
                    .any(|other| other.covered && other.path.starts_with(&row.path)),
            "{} is neither covered nor the way to anything covered",
            row.path.display()
        );
    }
    assert_eq!(
        app.rows()
            .iter()
            .filter(|row| !row.is_file() && row.covered)
            .count(),
        fixture::tree()
            .walk()
            .filter(|(node, _)| node.is_covered())
            .count()
    );

    app.toggle_scoped_only();

    // Widening again is the whole walk back, and — the selection having
    // survived the narrowing — an app indistinguishable from the one before.
    assert!(!app.scoped_only());
    assert_eq!(drawn(&app), whole_fixture());
    assert_eq!(app, before);
}

#[test]
fn the_filter_keeps_the_depths_and_the_order_the_walk_gave() {
    let mut app = App::from_tree(&fixture::tree());

    app.toggle_scoped_only();

    let seen: Vec<(usize, &str)> = app
        .rows()
        .iter()
        .map(|row| (row.depth, row.path.to_str().expect("ascii path")))
        .collect();
    // `engine` and `tui` still sit at depth 2 under `crates`, under a root
    // that is drawn only as their way in: the filter narrows the view, it
    // does not reparent anything.
    assert_eq!(
        seen,
        [
            (0, "warlock"),
            (1, "warlock/crates"),
            (2, "warlock/crates/engine"),
            (2, "warlock/crates/tui"),
        ]
    );
}

#[test]
fn filtering_the_selected_node_away_lands_the_selection_on_a_drawn_row() {
    let mut app = app_selecting("warlock/assets");

    app.toggle_scoped_only();

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
    app.toggle_scoped_only();

    // The last row the narrowed view has, which the wide view puts somewhere
    // else entirely.
    app.select_last();
    assert_eq!(
        app.selected_row().map(|row| row.path.clone()),
        Some(PathBuf::from("warlock/crates/tui"))
    );

    app.toggle_scoped_only();

    assert_eq!(
        app.selected_row().map(|row| row.path.clone()),
        Some(PathBuf::from("warlock/crates/tui"))
    );
    assert_eq!(drawn(&app), whole_fixture());
}

#[test]
fn the_filter_leaves_the_coverage_count_alone() {
    let mut app = App::from_tree(&fixture::tree());
    let before = app.coverage();

    app.toggle_scoped_only();

    // One row fewer on screen — `assets` — and the same five directories
    // counted: the footer describes the tree, not the view.
    assert_eq!(drawn(&app), scoped_fixture());
    assert_eq!(app.coverage(), before);
    assert_ne!(tally(&app), before);

    app.toggle_scoped_only();

    assert_eq!(app.coverage(), before);
    assert_eq!(tally(&app), before);
}

#[test]
fn a_directory_collapsed_before_the_filter_is_still_collapsed_after_it() {
    let mut app = app_selecting("warlock/crates");
    app.toggle_collapsed();
    assert_eq!(drawn(&app), collapsed_over_crates());

    app.toggle_scoped_only();

    // The root is unscoped, and `crates`' descendants are hidden, but the
    // root is still the way to them, so it keeps its row: what survives the
    // filter is read off the whole walk, not off what collapsing left drawn.
    assert_eq!(drawn(&app), ["warlock", "warlock/crates"]);
    assert!(app.is_collapsed("warlock/crates"));

    app.toggle_scoped_only();

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
    app.toggle_scoped_only();

    app.toggle_collapsed();

    // The root, collapsed over everything the filter kept.
    assert_eq!(drawn(&app), ["warlock"]);
    assert!(app.is_collapsed("warlock"));

    app.toggle_collapsed();
    assert_eq!(drawn(&app), scoped_fixture());
}

#[test]
fn nothing_scoped_narrows_to_nothing_at_all() {
    let mut app = App::from_rows(vec![
        Row::new(0, "repo", false),
        Row::new(1, "repo/crates", false),
    ]);

    app.toggle_scoped_only();

    // No covered node means no ancestors worth keeping either, and an app
    // with no rows selects nothing rather than an index off the end.
    assert!(app.is_empty());
    assert_eq!(app.selected(), 0);
    assert_eq!(app.selected_row(), None);

    app.toggle_scoped_only();
    assert_eq!(drawn(&app), ["repo", "repo/crates"]);
}

#[test]
fn an_unscoped_subtree_goes_whole_while_its_unscoped_siblings_way_in_stays() {
    let tree = Tree::new(
        Node::new("repo").with_children([
            Node::new("repo/kept")
                .with_children([Node::new("repo/kept/deep").with_scope(Some("deep".to_owned()))]),
            Node::new("repo/gone").with_children([Node::new("repo/gone/deep")]),
        ]),
    );
    let mut app = App::from_tree(&tree);

    app.toggle_scoped_only();

    // An unscoped root and an unscoped directory both survive as the way to
    // one scoped node; the sibling branch with nothing scoped in it goes
    // whole, parent and child together.
    assert_eq!(drawn(&app), ["repo", "repo/kept", "repo/kept/deep"]);
}

#[test]
fn toggling_the_filter_clears_the_last_keystrokes_message() {
    let mut app = app_with_files_selecting("warlock/assets/logo.svg");
    assert_eq!(app.scope_target(), None);
    assert!(app.message().is_some());

    app.toggle_scoped_only();

    assert_eq!(app.message(), None);
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
            (1, "warlock/crates", false),
            (2, "warlock/crates/engine", false),
            (3, "warlock/crates/engine/Cargo.toml", true),
            (2, "warlock/crates/tui", false),
            (3, "warlock/crates/tui/Cargo.toml", true),
            (1, "warlock/assets", false),
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
fn a_file_row_carries_no_scope_of_its_own_and_takes_its_directorys_coverage() {
    let tree = fixture::tree();
    let mut app = App::from_tree(&tree);

    app.toggle_files();

    for row in app.rows().iter().filter(|row| row.is_file()) {
        assert_eq!(row.scope, None, "{} claims a scope", row.path.display());
        let directory = row.path.parent().expect("a file sits in a directory");
        let node = tree.find(directory).expect("the file came from that node");
        assert_eq!(
            row.covered,
            node.is_covered(),
            "{} is not its directory's colour",
            row.path.display()
        );
    }
    // Including files under both a covered and an uncovered directory, so
    // the copy is a copy of the right node rather than of any node.
    assert!(app.rows().iter().any(|row| row.is_file() && row.covered));
    assert!(app.rows().iter().any(|row| row.is_file() && !row.covered));
}

#[test]
fn showing_files_leaves_the_coverage_count_alone() {
    let tree = fixture::tree();
    let mut app = App::from_tree(&tree);
    let before = app.coverage();
    assert_eq!(before, tally(&app));

    app.toggle_files();

    // More rows on screen and not one more directory: the footer counts
    // directories.
    assert!(app.rows().len() > tree.walk().count());
    assert_eq!(app.coverage(), before);
    assert_eq!(tally(&app), before);

    app.toggle_files();

    assert_eq!(app.coverage(), before);
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
            "warlock/crates",
            "warlock/crates/engine",
            "warlock/crates/engine/Cargo.toml",
            "warlock/crates/tui",
            "warlock/crates/tui/Cargo.toml",
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
            "warlock/crates",
            "warlock/assets",
            "warlock/assets/logo.svg",
        ]
    );

    app.toggle_collapsed();

    // Byte for byte what was there before: same rows, same order, same
    // depths, same coverage.
    assert_eq!(app.rows(), before);
}

#[test]
fn a_directory_holding_only_files_collapses_exactly_when_the_files_are_shown() {
    // `repo/assets` has no child directories and lists one file, so whether
    // anything is under it is the file toggle's answer and not the tree's.
    let tree = Tree::new(Node::new("repo").with_children([
        Node::new("repo/assets").with_files([PathBuf::from("repo/assets/logo.svg")]),
    ]));
    let mut app = select(App::from_tree(&tree), "repo/assets");
    let selected = app.selected();

    assert!(!app.can_collapse(selected), "nothing is drawn under it");

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
    // and the scoped-only view takes it away. A row with nothing under it is
    // a row with nothing to hide, whichever filter emptied it.
    let mut app = App::from_rows(vec![
        Row::new(0, "repo", true).with_scope(Some("platform".to_owned())),
        Row::new(1, "repo/crates", false),
        Row::new(2, "repo/crates/tui", false),
    ]);

    assert!(app.can_collapse(0), "the whole walk is drawn");

    app.toggle_scoped_only();

    assert_eq!(drawn(&app), ["repo"], "the unscoped pair is filtered out");
    assert!(!app.can_collapse(0), "so the root holds nothing");

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

    // The file's row is gone, and what it went behind is the directory
    // that held it.
    assert_eq!(
        app.selected_row().map(|row| row.path.clone()),
        Some(PathBuf::from("warlock/crates/engine"))
    );
    assert!(app.selected() < app.rows().len());
}

#[test]
fn the_filter_keeps_the_files_of_covered_directories_and_no_others() {
    let mut app = App::from_tree(&fixture::tree());
    app.toggle_files();

    app.toggle_scoped_only();

    // The root survives as the way down to the scoped directories, but it is
    // unscoped, so it brings no files with it. `assets/` is unscoped and goes
    // whole, its files with it.
    assert_eq!(
        drawn(&app),
        [
            "warlock",
            "warlock/crates",
            "warlock/crates/engine",
            "warlock/crates/engine/Cargo.toml",
            "warlock/crates/tui",
            "warlock/crates/tui/Cargo.toml",
        ]
    );

    app.toggle_files();

    // And with the files hidden again the narrowed view is exactly the one
    // it was before files existed.
    assert_eq!(drawn(&app), scoped_fixture());
}

#[test]
fn toggling_the_files_clears_the_last_keystrokes_message() {
    let mut app = app_with_files_selecting("warlock/assets/logo.svg");
    assert_eq!(app.scope_target(), None);
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
        Row::new(0, "repo", false),
        Row::file(1, "repo/README.md", false),
        Row::new(1, "repo/crates", false),
    ]);

    assert!(!app.show_files());
    assert_eq!(drawn(&app), ["repo", "repo/crates"]);

    let mut app = app;
    app.toggle_files();
    assert_eq!(drawn(&app), ["repo", "repo/README.md", "repo/crates"]);
}

#[test]
fn narrowing_leaves_the_window_in_range_with_the_selection_in_it() {
    let mut app = app_selecting("warlock/assets");
    app.set_viewport_height(2);
    assert!(selection_is_on_screen(&app));

    app.toggle_scoped_only();

    assert!(window_is_in_range(&app));
    assert!(selection_is_on_screen(&app));

    app.toggle_scoped_only();

    assert!(window_is_in_range(&app));
    assert!(selection_is_on_screen(&app));
}

fn panel_focused() -> App {
    let mut app = scrolled_to(MANY / 2);
    app.set_focus(Focus::Panel);
    app
}

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
    let mut app = App::from_rows(three_rows());
    assert!(
        app.panel().composer_showable(),
        "an app opens with room to type"
    );

    // The file gives the panel those rows back: a field under it is a field
    // about the wrong card.
    app.show_document(["a line of a file"], false);
    assert!(
        !app.panel().composer_showable(),
        "the document card hid nothing"
    );

    app.swap_card();
    assert_eq!(app.panel.showing(), Showing::Thread);
    assert!(
        app.panel().composer_showable(),
        "the conversation gave it back"
    );

    app.swap_card();
    assert_eq!(app.panel.showing(), Showing::Document);
    assert!(!app.panel().composer_showable());
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
fn no_movement_key_moves_anything_while_the_composer_has_the_focus() {
    // The keyboard is the draft's, so a movement key is a character and
    // never arrives here; if one does, it moves neither pane. A conversation
    // of more lines than the panel is tall and a selection in the middle of the
    // tree, so both windows have somewhere to go if anything lets them.
    for (name, movement) in MOVEMENTS {
        let base = Instant::now();
        let mut app = app_talking(9, base);
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
            app.panel().scroll_offset(),
            before.panel().scroll_offset(),
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

#[test]
fn nothing_but_a_movement_key_cares_which_pane_has_the_focus() {
    type Change = fn(&mut App);
    let changes: [(&str, Change); 6] = [
        ("toggle_collapsed", App::toggle_collapsed),
        ("toggle_scoped_only", App::toggle_scoped_only),
        ("toggle_files", App::toggle_files),
        ("scope_target", |app| {
            app.scope_target();
        }),
        ("view_target", |app| {
            app.view_target();
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

const PANEL: u16 = 3;

fn at(base: Instant, seconds: u64) -> Instant {
    base + Duration::from_secs(seconds)
}

fn app_at_the_panel() -> App {
    let mut app = App::from_rows(three_rows());
    app.panel_mut().set_height(PANEL);
    app.toggle_focus();
    app
}

// A turn still under way, so that what arrives next is a work line, and a work
// line leaves a parked window where it is: a note or a question would put the
// card back on its newest line.
fn app_talking(lines: u64, base: Instant) -> App {
    let mut app = App::from_rows(three_rows());
    app.panel_mut().set_height(PANEL);
    app.toggle_focus();
    app.panel_mut().start_turn("start", base);
    for line in 0..lines {
        work(&mut app, line, at(base, line + 1));
    }
    app
}

fn work(app: &mut App, line: u64, at: Instant) {
    app.panel_mut().record_turn(
        &Activity::Tool {
            name: format!("line {line}"),
            detail: None,
        },
        at,
    );
}

// A document's line is its own text and nothing else, which is exactly what
// a test asserting on what is drawn wants back.
fn line_text(line: &Line) -> String {
    match line {
        Line::Clocked { text, .. }
        | Line::Text { text }
        | Line::Said { text }
        | Line::Note { text }
        | Line::Wrapped { text, .. } => text.clone(),
    }
}

fn panel_text(app: &App, now: Instant) -> Vec<String> {
    app.panel().window(now).iter().map(line_text).collect()
}

#[test]
fn an_app_nobody_has_talked_to_has_no_thread_and_draws_no_panel() {
    let now = Instant::now();

    for app in [
        App::default(),
        App::from_rows(three_rows()),
        App::from_tree(&fixture::tree()),
    ] {
        let mut app = app;
        app.panel_mut().set_height(PANEL);

        assert!(!app.panel().has_thread());
        assert_eq!(app.panel().window(now), Vec::new());
        assert_eq!(app.panel().lines_below(), 0);
        assert_eq!(app.panel().scroll_offset(), 0);

        // Not even a movement key can make a panel with nothing in it say
        // something.
        app.toggle_focus();
        for (_, movement) in MOVEMENTS {
            movement(&mut app);
        }
        assert!(!app.panel().has_thread());
        assert_eq!(app.panel().window(now), Vec::new());
    }
}

#[test]
fn the_panel_keeps_the_newest_line_on_the_bottom_row_while_it_follows() {
    let base = Instant::now();
    let mut app = app_talking(2, base);

    // Shorter than the panel: everything is on screen and there is nothing
    // to scroll.
    assert_eq!(app.panel().scroll_offset(), 0);
    assert_eq!(panel_text(&app, at(base, 3)), ["start", "line 0", "line 1"],);

    // Longer than the panel: the window is the last screenful, and moves as
    // each line arrives without anybody telling it to.
    for line in 2..6 {
        work(&mut app, line, at(base, line + 1));
    }

    assert!(app.panel().follows());
    assert_eq!(app.panel().scroll_offset(), 7 - usize::from(PANEL));
    assert_eq!(app.panel().lines_below(), 0);
    assert_eq!(
        panel_text(&app, at(base, 7)),
        ["line 3", "line 4", "line 5"],
    );
}

#[test]
fn scrolling_up_in_the_panel_stops_it_following() {
    let base = Instant::now();
    let mut app = app_talking(9, base);
    assert_eq!(panel_text(&app, at(base, 9)).first().unwrap(), "line 6");

    app.select_previous();

    assert!(!app.panel().follows());
    let parked = panel_text(&app, at(base, 9));
    assert_eq!(parked, ["line 5", "line 6", "line 7"]);

    // And the lines that arrive afterwards leave the window where it is.
    for line in 9..20 {
        app.panel_mut()
            .record_turn(&Activity::Thinking, at(base, line + 1));
    }

    assert_eq!(panel_text(&app, at(base, 30)), parked);
    assert_eq!(app.panel().scroll_offset(), 6);
}

#[test]
fn the_end_of_list_key_puts_the_panel_back_on_live() {
    let base = Instant::now();
    let mut app = app_talking(9, base);
    app.select_first();
    assert!(!app.panel().follows());
    assert_eq!(app.panel().scroll_offset(), 0);

    app.select_last();

    assert!(app.panel().follows());
    assert_eq!(
        panel_text(&app, at(base, 9)),
        ["line 6", "line 7", "line 8"],
    );

    // Live means live: the next line to arrive moves the window again.
    app.panel_mut()
        .record_turn(&Activity::Thinking, at(base, 10));

    assert_eq!(
        panel_text(&app, at(base, 10)),
        ["line 7", "line 8", "thinking"],
    );
}

#[test]
fn scrolling_back_down_to_the_end_is_following_again() {
    let base = Instant::now();
    let mut app = app_talking(9, base);

    app.select_page_up();
    assert!(!app.panel().follows());

    app.select_next();
    app.select_next();
    assert!(!app.panel().follows());

    app.select_next();

    assert!(app.panel().follows());
    assert_eq!(app.panel().lines_below(), 0);
}

#[test]
fn the_panel_says_how_many_lines_are_below_the_view() {
    let base = Instant::now();
    let mut app = app_talking(9, base);

    // Ten lines, three of them on screen at the end: nothing below.
    assert_eq!(app.panel().lines_below(), 0);

    app.select_first();
    assert_eq!(app.panel().lines_below(), 10 - usize::from(PANEL));

    app.select_next();
    assert_eq!(app.panel().lines_below(), 10 - usize::from(PANEL) - 1);

    app.select_last();
    assert_eq!(app.panel().lines_below(), 0);
}

#[test]
fn a_panel_nobody_has_measured_has_nothing_to_scroll() {
    let base = Instant::now();
    let mut app = app_talking(9, base);
    app.panel_mut().set_height(0);

    // No height means no window: nothing is drawn, so the honest offset is
    // the top however long the card is, and no movement key can push it
    // off one — the same reading the tree's rule takes of a viewport of
    // zero rows.
    for (name, movement) in MOVEMENTS {
        movement(&mut app);
        assert_eq!(
            app.panel().scroll_offset(),
            0,
            "{name} moved a panel of no height"
        );
        assert_eq!(app.panel().window(at(base, 9)), Vec::new(), "{name}");
        // Nothing is drawn, so the whole card is below what is drawn.
        assert_eq!(app.panel().lines_below(), 10, "{name}");
    }

    // And the window is measurable again the moment somebody measures it.
    app.panel_mut().set_height(PANEL);
    assert_eq!(app.panel().scroll_offset(), 10 - usize::from(PANEL));
}

#[test]
fn the_panel_window_rule_follows_the_end_or_stays_where_it_was_put() {
    // Following: the last screenful, whatever the card's length.
    assert_eq!(panel_offset_for(10, 3, 0, true), 7);
    assert_eq!(panel_offset_for(11, 3, 99, true), 8);
    // Parked: the reader's own offset, clamped to what is there.
    assert_eq!(panel_offset_for(10, 3, 4, false), 4);
    assert_eq!(panel_offset_for(10, 3, 99, false), 7);
    // A card that fits, and a panel nobody has measured, are both the
    // top — and following one is the same as being parked at it.
    for following in [true, false] {
        assert_eq!(panel_offset_for(3, 3, 2, following), 0);
        assert_eq!(panel_offset_for(0, 3, 2, following), 0);
        assert_eq!(panel_offset_for(10, 0, 2, following), 0);
    }
}

fn document_lines() -> Vec<String> {
    (0..5).map(|line| format!("line {line}")).collect()
}

#[test]
fn a_document_is_drawn_from_its_first_line_in_the_order_it_was_given() {
    let now = Instant::now();
    let mut app = App::from_tree(&fixture::tree());
    app.panel_mut().set_height(PANEL);

    app.show_document(document_lines(), false);

    // The window is the top of the file: a file is read from its first line,
    // and nothing is appended to it for the panel to follow.
    assert!(app.panel().has_document());
    assert!(!app.panel().follows());
    assert_eq!(app.panel().scroll_offset(), 0);
    assert_eq!(panel_text(&app, now), ["line 0", "line 1", "line 2"]);
    // And the count agrees with what was handed over: five lines, three of
    // them on screen.
    assert_eq!(app.panel().lines_below(), 5 - usize::from(PANEL));

    // A panel tall enough for the whole file draws the whole file, in order,
    // with nothing below it.
    app.panel_mut().set_height(9);
    assert_eq!(panel_text(&app, now), document_lines());
    assert_eq!(app.panel().lines_below(), 0);
}

#[test]
fn a_read_the_cap_cut_short_says_so_under_the_last_line_it_got() {
    let now = Instant::now();
    let mut app = App::from_tree(&fixture::tree());
    app.panel_mut().set_height(9);

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

fn a_long_line() -> Vec<String> {
    [
        "# The engine",
        "It walks the tree and writes what it finds.",
        "done",
    ]
    .map(str::to_owned)
    .to_vec()
}

const NARROW: u16 = 18;

#[test]
fn a_document_line_wider_than_the_panel_is_drawn_in_as_many_rows_as_it_needs() {
    let now = Instant::now();
    let mut app = App::from_tree(&fixture::tree());
    app.panel_mut().set_height(9);
    app.panel_mut().set_width(NARROW);

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
    assert_eq!(app.panel().lines_below(), 0);
}

#[test]
fn a_panel_nobody_has_measured_wraps_nothing() {
    // What an app is between being built and being drawn: every frame tells
    // it the width, and until one has, a line is the row it arrived as.
    let now = Instant::now();
    let mut app = App::from_tree(&fixture::tree());
    app.panel_mut().set_height(9);

    app.show_document(a_long_line(), false);

    assert_eq!(app.panel().width(), 0);
    assert_eq!(panel_text(&app, now), a_long_line());
}

#[test]
fn the_rows_a_document_draws_as_follow_the_width_the_panel_was_last_told() {
    // The reader drags the terminal narrower with a file up: the document
    // re-flows to the width of the frame it is drawn in, from the lines it
    // has always held, without anything being read again.
    let now = Instant::now();
    let mut app = App::from_tree(&fixture::tree());
    app.panel_mut().set_height(9);
    app.panel_mut().set_width(80);
    app.show_document(a_long_line(), false);
    assert_eq!(panel_text(&app, now), a_long_line());

    app.panel_mut().set_width(NARROW);

    assert_eq!(panel_text(&app, now).len(), 5);
    // And wider again is the document it was: nothing was lost on the way
    // through the narrow panel, because the lines held are the file's.
    app.panel_mut().set_width(80);
    assert_eq!(panel_text(&app, now), a_long_line());
}

#[test]
fn the_panel_scrolls_by_rows_of_a_wrapped_document_rather_than_by_its_lines() {
    let now = Instant::now();
    let mut app = App::from_tree(&fixture::tree());
    app.panel_mut().set_height(2);
    app.panel_mut().set_width(NARROW);
    app.show_document(a_long_line(), false);

    // Five rows in a panel two tall: what is below the window is counted in
    // rows, so a reader scrolling past a wrapped line scrolls through it.
    assert_eq!(app.panel().lines_below(), 3);
    assert_eq!(panel_text(&app, now), ["# The engine", "It walks the tree"]);

    app.scroll_panel_down(1);

    assert_eq!(app.panel().scroll_offset(), 1);
    assert_eq!(
        panel_text(&app, now),
        ["It walks the tree", "and writes what it"]
    );
    assert_eq!(app.panel().lines_below(), 2);
}

fn thread_window(app: &App) -> (usize, bool) {
    app.panel.window_of(Showing::Thread)
}

fn document_window(app: &App) -> (usize, bool) {
    app.panel.window_of(Showing::Document)
}

fn document_text(app: &App) -> Vec<String> {
    app.panel.document_lines().iter().map(line_text).collect()
}

#[test]
fn each_card_keeps_a_window_of_its_own() {
    let base = Instant::now();
    let mut app = app_talking(9, base);

    // The conversation parked a line down, and then a document over it.
    app.select_first();
    app.select_next();
    assert_eq!(thread_window(&app), (1, false));

    app.show_document(document_lines(), false);
    assert_eq!(document_window(&app), (0, false));

    // Moving the document's window moves the document's window: the
    // conversation is where the reader left it, still parked.
    app.select_next();
    assert_eq!(document_window(&app), (1, false));
    assert_eq!(app.panel().scroll_offset(), 1);
    assert_eq!(thread_window(&app), (1, false));

    // Including when the movement takes the document to its own end, which
    // is the one thing that sets a follow flag.
    app.select_last();
    assert_eq!(document_window(&app), (5 - usize::from(PANEL), true));
    assert!(app.panel().follows());
    assert_eq!(thread_window(&app), (1, false));
}

#[test]
fn a_movement_at_the_panel_moves_only_the_card_that_is_showing() {
    let base = Instant::now();

    for (name, movement) in MOVEMENTS {
        let mut app = app_talking(9, base);
        app.select_first();
        app.show_document(document_lines(), false);
        let parked = thread_window(&app);

        movement(&mut app);

        assert_eq!(thread_window(&app), parked, "{name} moved the conversation");
        assert_eq!(
            app.panel().scroll_offset(),
            document_window(&app).0,
            "{name} scrolled something other than the document"
        );
    }

    // And the wheel, which names the pane rather than accepting the focus,
    // reads the same way.
    for lines in [1, 3, 99] {
        let mut app = app_talking(9, base);
        app.select_first();
        app.show_document(document_lines(), false);
        let parked = thread_window(&app);

        app.scroll_panel_down(lines);
        assert_eq!(
            thread_window(&app),
            parked,
            "a wheel moved the conversation"
        );
        app.scroll_panel_up(lines);
        assert_eq!(
            thread_window(&app),
            parked,
            "a wheel moved the conversation"
        );
        assert_eq!(document_window(&app), (0, false));
    }
}

#[test]
fn an_empty_document_is_still_something_the_panel_is_holding() {
    let now = Instant::now();
    let mut app = App::from_tree(&fixture::tree());
    app.panel_mut().set_height(PANEL);

    app.show_document(Vec::<String>::new(), false);

    // An empty file read is not the same as nothing having happened: the
    // panel is holding it, so the mark does not come back.
    assert!(app.panel().has_document());
    assert!(app.panel().has_content());
    assert_eq!(app.panel().window(now), Vec::new());
    assert_eq!(app.panel().lines_below(), 0);
    assert_eq!(app.panel().scroll_offset(), 0);
}

#[test]
fn the_swap_shows_the_other_card_and_the_next_swap_shows_it_back() {
    let base = Instant::now();
    let mut app = app_talking(9, base);
    let thread = panel_text(&app, at(base, 9));

    app.show_document(document_lines(), false);
    let document = panel_text(&app, at(base, 9));
    assert_ne!(document, thread, "the two cards draw the same thing");

    // Round the cycle, because a swap that only went one way would strand a
    // reader on whichever card they were not reading.
    app.swap_card();
    assert_eq!(app.panel.showing(), Showing::Thread);
    assert_eq!(panel_text(&app, at(base, 9)), thread);

    app.swap_card();
    assert_eq!(panel_text(&app, at(base, 9)), document);
    assert!(app.panel().has_content());
}

#[test]
fn a_swap_moves_nothing_but_which_card_is_showing() {
    let base = Instant::now();
    let mut app = app_talking(9, base);
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
    before.panel.show(Showing::Thread);
    assert_eq!(app, before, "the swap moved something other than the card");
}

fn rewritten_lines() -> Vec<String> {
    ["# Rewritten", "by somebody else"]
        .map(str::to_owned)
        .to_vec()
}

#[test]
fn a_re_read_fills_the_document_card_without_bringing_it_to_the_front() {
    let base = Instant::now();
    let mut app = app_talking(9, base);
    // A document read once and then left: the reader is back on the
    // conversation.
    app.show_document(document_lines(), false);
    app.swap_card();
    assert_eq!(app.panel.showing(), Showing::Thread);
    let thread = panel_text(&app, at(base, 9));

    app.panel_mut().refill_document(rewritten_lines(), false);

    // The panel is exactly where they left it. The new lines are on the card
    // behind it, waiting for the swap they will ask for themselves — a file
    // being saved in an editor is not a reason to take the conversation off
    // the screen.
    assert_eq!(panel_text(&app, at(base, 9)), thread);
    assert!(app.panel().has_document());
    assert_eq!(document_text(&app), rewritten_lines());

    app.swap_card();
    assert_eq!(panel_text(&app, at(base, 9)), rewritten_lines());
}

#[test]
fn a_re_read_under_a_showing_document_leaves_it_showing_from_its_first_line() {
    let base = Instant::now();
    let mut app = app_talking(9, base);
    app.show_document(document_lines(), false);
    // Parked at the end of the file that was there before, which is the
    // window a re-read has to decide what to do with.
    app.select_last();
    assert!(app.panel().follows());
    let parked = thread_window(&app);

    app.panel_mut().refill_document(rewritten_lines(), false);

    // Still the card on screen — nothing about which card is showing moved —
    // and showing the new file from its first line: line five of what was
    // there is not line five of what is there now.
    assert_eq!(panel_text(&app, at(base, 9)), rewritten_lines());
    assert_eq!(app.panel().scroll_offset(), 0);
    assert!(!app.panel().follows());
    // And the conversation behind it is where it was, still following its
    // own newest line.
    assert_eq!(thread_window(&app), parked);
}

#[test]
fn a_re_read_the_cap_cut_short_says_so_exactly_as_the_first_read_did() {
    let now = Instant::now();
    let mut app = App::from_tree(&fixture::tree());
    app.panel_mut().set_height(9);
    app.show_document(document_lines(), false);

    app.panel_mut().refill_document(document_lines(), true);

    // The one line a document did not write, added here for the same reason
    // it is added to a first read: the words are the screen's.
    let drawn = panel_text(&app, now);
    assert_eq!(drawn.len(), document_lines().len() + 1);
    assert_eq!(drawn.last(), Some(&cut_at_cap_message()));
}

#[test]
fn a_re_read_moves_nothing_but_the_document_card() {
    let base = Instant::now();
    let mut app = app_talking(9, base);
    app.show_document(document_lines(), false);
    // A window of the document's own, the focus and a selection of the
    // tree's own, and something on the footer: everything a re-read could
    // disturb, put somewhere a default would not be.
    app.select_next();
    app.toggle_focus();
    app.select_next();
    app.set_message("something the last keystroke said");
    let mut before = app.clone();

    app.panel_mut().refill_document(rewritten_lines(), false);

    // The whole of what changed is what is on the one card: the
    // conversation, the focus, the selection, the tree's window, the bit
    // saying which card is drawn and the footer are all the app's own clone,
    // untouched.
    before.panel.refill_document(rewritten_lines(), false);
    assert_eq!(
        app, before,
        "the re-read moved something other than the card"
    );
}

#[test]
fn each_cards_window_survives_a_swap_away_and_back() {
    let base = Instant::now();
    let mut app = app_talking(9, base);

    // The conversation parked a line down from the top, and a document over
    // it parked a line down from its own.
    app.select_first();
    app.select_next();
    let parked = thread_window(&app);
    assert_eq!(parked, (1, false));

    app.show_document(document_lines(), false);
    app.select_next();
    let left = document_window(&app);
    assert_eq!(left, (1, false));

    // Away: the conversation's window is the one the reader left it on, not
    // the top and not the end.
    app.swap_card();
    assert_eq!(app.panel.showing(), Showing::Thread);
    assert_eq!(app.panel().scroll_offset(), parked.0);
    assert_eq!(panel_text(&app, at(base, 9))[0], "line 0");

    // And back: the document is on the line they left it on.
    app.swap_card();
    assert_eq!(document_window(&app), left);
    assert_eq!(app.panel().scroll_offset(), left.0);
    assert_eq!(
        panel_text(&app, at(base, 9)),
        ["line 1", "line 2", "line 3"]
    );
}

#[test]
fn the_conversation_shows_its_newest_line_on_return_unless_it_was_parked() {
    let base = Instant::now();

    // Following: a turn that went on reporting while the document was up is
    // at its newest line the moment the reader swaps to it.
    let mut app = app_talking(9, base);
    app.panel_mut().start_turn(QUESTION, at(base, 10));
    app.show_document(document_lines(), false);
    record_lines(&mut app, 11..14, base);

    app.swap_card();
    let drawn = panel_text(&app, at(base, 20));
    assert!(app.panel().follows());
    assert_eq!(drawn.last().expect("the thread has lines"), "Read line 13");

    // Parked: the reader put that window where it is, and lines arriving
    // behind a document do not take it off the line they chose.
    let mut app = app_talking(9, base);
    app.panel_mut().start_turn(QUESTION, at(base, 10));
    app.select_first();
    app.show_document(document_lines(), false);
    record_lines(&mut app, 11..14, base);

    app.swap_card();
    assert!(!app.panel().follows());
    assert_eq!(app.panel().scroll_offset(), 0);
    assert_eq!(panel_text(&app, at(base, 20))[0], "start");
}

// Work lines on the turn in flight, which unlike a note do not bring the
// conversation to the front.
fn record_lines(app: &mut App, lines: std::ops::Range<u64>, base: Instant) {
    for line in lines {
        app.panel_mut().record_turn(
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
    let mut app = app_talking(9, base);
    app.panel_mut().set_height(9);
    app.show_document(document_lines(), false);
    assert_eq!(document_text(&app), document_lines());

    // Everything a session does that is not another read: the reader walks
    // the tree, reshapes it, scrolls both cards, swaps between them, and a
    // turn reports behind the document all the while.
    app.toggle_focus();
    app.select_next();
    app.select_last();
    app.toggle_collapsed();
    app.toggle_files();
    app.toggle_scoped_only();
    app.toggle_focus();
    app.select_next();
    app.swap_card();
    app.select_next();
    app.panel_mut().start_turn(QUESTION, at(base, 100));
    app.swap_card();
    record_lines(&mut app, 0..4, at(base, 100));
    app.set_message("something the last keystroke said");

    // The card holds the lines it was handed, in the order it was handed
    // them: it does not follow the selection, it is not appended to by the
    // turn, and no window the reader moved has edited what it is a window
    // over.
    assert_eq!(document_text(&app), document_lines());
    assert_eq!(panel_text(&app, at(base, 200)), document_lines());

    // The only thing that writes the card is another read, which replaces it
    // whole rather than adding to it.
    app.show_document(["a line of another file"], false);
    assert_eq!(document_text(&app), ["a line of another file"]);
}

#[test]
fn a_swap_with_nothing_but_the_conversation_is_refused_naming_the_view_key() {
    // The only press that has nowhere to go. The conversation is always a
    // stop, so a swap is refused exactly when it is the only card there is:
    // no file has been read, and the card the key would reach is a card
    // about nothing.
    let mut app = App::from_rows(three_rows());
    app.panel_mut().set_height(PANEL);
    app.set_message("something the last keystroke said");
    let mut before = app.clone();

    app.swap_card();

    // The mark stays up, the conversation stays showing, and the refusal is
    // about the document because the document is the card a keystroke can
    // make. The message is the whole of what the press changed.
    let message = app.message().expect("a swap with nothing read is refused");
    assert_eq!(message, no_document_message(), "{message}");
    assert!(message.contains("press v"), "{message}");
    assert_eq!(app.panel.showing(), Showing::Thread);
    assert!(!app.panel().has_content());
    before.set_message(message);
    assert_eq!(app, before, "the refusal moved something else");
}

const QUESTION: &str = "what does the engine do?";
const ANSWER: &str = "It walks the tree and writes what it finds.";

fn ask_and_answer(app: &mut App, base: Instant) {
    app.panel_mut().start_turn(QUESTION, at(base, 1));
    for line in 0..3 {
        app.panel_mut().record_turn(
            &Activity::Tool {
                name: "Grep".to_owned(),
                detail: Some(format!("thread line {line}")),
            },
            at(base, line + 2),
        );
    }
    app.panel_mut().answer_turn(ANSWER, at(base, 5));
}

// "does" out of the question, which is piece 0 of the thread
// `ask_and_answer` leaves behind.
const DOES: (Position, Position) = (Position::new(0, 5), Position::new(0, 9));

fn selected_text(app: &App) -> String {
    let thread = app.panel().thread().expect("a conversation to select in");
    let selection = app.selection().expect("something highlighted");
    copied_text(&thread.pieces(), selection)
}

#[test]
fn a_press_highlights_nothing_until_a_drag_extends_it() {
    let base = Instant::now();
    let mut app = app_at_the_panel();
    ask_and_answer(&mut app, base);

    let (from, to) = DOES;
    app.start_selection(from);

    // A press with no drag after it is a reader putting the pointer down,
    // and it copies nothing: both ends on the same character.
    let selection = app.selection().expect("the press anchored one");
    assert_eq!(selection.start(), from);
    assert_eq!(selection.end(), from);
    assert_eq!(selected_text(&app), String::new());

    app.extend_selection(to);
    assert_eq!(selected_text(&app), "does");
}

#[test]
fn a_drag_back_past_its_anchor_selects_from_the_anchor() {
    let base = Instant::now();
    let mut app = app_at_the_panel();
    ask_and_answer(&mut app, base);

    // The press lands at the end of the word and the drag goes left, which
    // is the case an anchor is kept for: the selection reads the same way
    // round as the same drag made rightwards.
    let (left, right) = DOES;
    app.start_selection(right);
    app.extend_selection(left);
    let selection = app.selection().expect("the drag extended one");
    assert_eq!(selection.start(), left);
    assert_eq!(selection.end(), right);
    assert_eq!(selected_text(&app), "does");

    // And the far end keeps moving from the same anchor rather than from
    // wherever the last drag event left it.
    app.extend_selection(Position::new(0, 0));
    assert_eq!(selected_text(&app), "what does");
}

#[test]
fn a_drag_with_no_press_behind_it_highlights_nothing() {
    let base = Instant::now();
    let mut app = app_at_the_panel();
    ask_and_answer(&mut app, base);

    app.extend_selection(DOES.1);

    assert_eq!(app.selection(), None);
}

#[test]
fn a_press_with_no_conversation_to_measure_against_highlights_nothing() {
    let mut app = app_at_the_panel();
    assert!(!app.panel().has_thread());

    app.start_selection(DOES.0);
    app.extend_selection(DOES.1);

    assert_eq!(app.selection(), None);
}

#[test]
fn nothing_but_the_swap_takes_the_highlight_down() {
    let base = Instant::now();
    let mut app = app_at_the_panel();
    app.show_document(document_lines(), false);
    ask_and_answer(&mut app, base);
    app.start_selection(DOES.0);
    app.extend_selection(DOES.1);
    let held = app.selection().expect("the drag made one");

    // A wheel notch over the panel, either way.
    app.scroll_panel_down(2);
    assert_eq!(app.selection(), Some(held), "a wheel notch cleared it");
    app.scroll_panel_up(1);
    assert_eq!(app.selection(), Some(held), "a wheel notch cleared it");

    // A key press: a movement in the tree, the focus key, and a keystroke
    // that was refused and said so.
    app.toggle_focus();
    app.select_next();
    app.select_previous();
    app.set_message("something the last keystroke said");
    assert_eq!(app.selection(), Some(held), "a key press cleared it");

    // New text arriving on the thread, under the selection and after it.
    app.panel_mut().note("a note nobody asked for", at(base, 6));
    app.panel_mut()
        .start_turn("and how long does it take?", at(base, 7));
    app.panel_mut().answer_turn("About a second.", at(base, 8));
    assert_eq!(app.selection(), Some(held), "new text cleared it");

    // A redraw, which is everything the draw path reads off the app.
    let _drawn = app.panel().window(at(base, 9));
    let _rows = app.rows().len();
    assert_eq!(app.selection(), Some(held), "a redraw cleared it");

    // The one thing that does clear it: the card it was drawn on going away.
    app.swap_card();
    assert_eq!(app.selection(), None, "the swap left it standing");
}

#[test]
fn text_arriving_under_a_selection_leaves_it_on_the_same_words() {
    let base = Instant::now();
    let mut app = app_at_the_panel();
    ask_and_answer(&mut app, base);
    app.start_selection(DOES.0);
    app.extend_selection(DOES.1);

    // Positions are the thread's own, so a turn filed after them cannot move
    // the text they name — which is the whole reason they are not cells.
    app.panel_mut().note("a refusal", at(base, 6));
    app.panel_mut().start_turn("and again?", at(base, 7));
    app.panel_mut().answer_turn("Yes.", at(base, 8));
    app.scroll_panel_down(4);

    assert_eq!(selected_text(&app), "does");
}

#[test]
fn the_swap_takes_the_highlight_down_wherever_it_lands() {
    let base = Instant::now();
    let mut app = app_at_the_panel();
    ask_and_answer(&mut app, base);
    app.show_document(document_lines(), false);
    app.swap_card();
    assert!(app.panel().showing_thread());

    app.start_selection(DOES.0);
    app.extend_selection(DOES.1);
    assert!(app.selection().is_some());

    // Round the whole cycle: the highlight is gone at the document and
    // does not come back when the conversation does.
    app.swap_card();
    assert_eq!(app.selection(), None);
    app.swap_card();
    assert!(app.panel().showing_thread());
    assert_eq!(app.selection(), None, "the swap back put it back");
}

#[test]
fn a_swap_that_has_nowhere_to_go_leaves_the_highlight_alone() {
    let base = Instant::now();
    let mut app = App::from_rows(three_rows());
    app.panel_mut().set_height(PANEL);
    ask_and_answer(&mut app, base);
    app.start_selection(DOES.0);
    app.extend_selection(DOES.1);
    let held = app.selection().expect("the drag made one");

    // Nothing to swap to: the conversation is still the card showing, so
    // the highlight is still over the text it was measured against.
    app.swap_card();

    assert!(app.panel().showing_thread());
    assert_eq!(app.selection(), Some(held));
}

#[test]
fn a_swap_out_of_a_document_lands_on_the_conversation_it_left() {
    let base = Instant::now();
    let mut app = App::from_rows(three_rows());
    app.panel_mut().set_height(PANEL);
    ask_and_answer(&mut app, base);
    let thread = panel_text(&app, at(base, 9));

    app.show_document(document_lines(), false);
    let document = panel_text(&app, at(base, 9));
    assert_ne!(document, thread, "the file and the conversation draw alike");

    app.swap_card();

    assert_eq!(panel_text(&app, at(base, 9)), thread);
    assert!(app.message().is_none(), "a swap that worked said something");

    // And back to the file: a cycle of the two cards.
    app.swap_card();
    assert_eq!(panel_text(&app, at(base, 9)), document);
    app.swap_card();
    assert_eq!(panel_text(&app, at(base, 9)), thread);
}

#[test]
fn a_swap_out_of_a_document_with_nothing_else_lands_where_the_field_is() {
    // A document hides the composer, so the way back out of one has to land
    // on a card the field is drawn under, even an empty one: a reader who
    // has read a file on a fresh session can still type.
    let mut app = App::from_rows(three_rows());
    app.panel_mut().set_height(PANEL);
    app.show_document(document_lines(), false);
    assert!(!app.panel().composer_showable());

    app.swap_card();

    assert!(app.panel().composer_showable());
    assert!(
        !app.panel().has_content(),
        "the conversation is the empty one"
    );
    assert!(app.message().is_none(), "a swap that worked said something");
}

#[test]
fn showing_one_card_throws_the_other_away() {
    let base = Instant::now();
    let mut app = app_at_the_panel();

    // A thread parked a line down from its top, then a document over it
    // parked a line down from its own.
    ask_and_answer(&mut app, base);
    app.select_first();
    app.select_next();
    assert_eq!(thread_window(&app), (1, false));

    app.show_document(document_lines(), false);
    app.select_next();
    assert_eq!(document_window(&app), (1, false));

    // Both cards are still filled and both windows are still where they were
    // left: showing one card neither empties the other nor moves it.
    assert!(app.panel().has_thread());
    assert!(app.panel().has_document());
    assert_eq!(thread_window(&app), (1, false));
    assert_eq!(document_window(&app), (1, false));

    // And round the cycle: each card draws from the line the reader left it
    // on rather than from its top or its end.
    app.swap_card();
    assert_eq!(panel_text(&app, at(base, 9))[0], "Grep thread line 0");
    app.swap_card();
    assert_eq!(
        panel_text(&app, at(base, 9)),
        ["line 1", "line 2", "line 3"]
    );
}

#[test]
fn each_card_keeps_its_own_follow_flag() {
    let base = Instant::now();
    let mut app = app_at_the_panel();
    // The thread left following, the document at the top of a file it was
    // never following: two cards, two answers.
    ask_and_answer(&mut app, base);
    app.show_document(document_lines(), false);

    assert_eq!(thread_window(&app), (2, true));
    assert_eq!(document_window(&app), (0, false));

    // Whichever card is showing is the one the panel answers about, and the
    // other goes on being what it was.
    assert!(!app.panel().follows());
    assert_eq!(app.panel().scroll_offset(), 0);
    app.swap_card();
    assert!(app.panel().follows());
    assert_eq!(app.panel().scroll_offset(), 2);

    // A turn already under way goes on being answered behind a document,
    // and the document does not move an inch while it is. Asking brings the
    // conversation to the front, so the file is a press back.
    app.panel_mut()
        .start_turn("and how long does it take?", at(base, 6));
    assert_eq!(app.panel.showing(), Showing::Thread);
    app.swap_card();
    assert_eq!(app.panel.showing(), Showing::Document);
    assert_eq!(
        panel_text(&app, at(base, 9)),
        ["line 0", "line 1", "line 2"]
    );
    app.panel_mut().answer_turn("About a second.", at(base, 7));
    assert_eq!(
        panel_text(&app, at(base, 9)),
        ["line 0", "line 1", "line 2"]
    );

    // And the thread is at its newest row the moment the reader swaps back
    // to it.
    app.swap_card();
    assert_eq!(app.panel.showing(), Showing::Thread);
    assert!(app.panel().follows());
    assert_eq!(
        panel_text(&app, at(base, 9)).last().map(String::as_str),
        Some("About a second.")
    );
}

#[test]
fn every_line_wider_than_the_panel_is_drawn_in_as_many_rows_as_it_needs() {
    let base = Instant::now();
    let mut app = app_at_the_panel();
    app.panel_mut().set_height(11);
    ask_and_answer(&mut app, base);

    // Nobody has measured the panel yet, so nothing is wrapped: five lines,
    // five rows.
    assert_eq!(app.panel().width(), 0);
    assert_eq!(panel_text(&app, at(base, 5)).len(), 5);

    app.panel_mut().set_width(NARROW);

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
    assert_eq!(app.panel().lines_below(), 0);

    // The window is cut out of those rows and not out of the lines: a panel
    // three tall over eleven rows shows the last three and has eight above
    // them, and a reader who goes back to the top has those eight below.
    app.panel_mut().set_height(PANEL);
    assert_eq!(app.panel().scroll_offset(), 8);
    assert_eq!(app.panel().lines_below(), 0);
    app.select_first();
    assert_eq!(app.panel().lines_below(), 8);

    // And wider again is the answer it always was — the lines held are the
    // model's own, so a terminal dragged about re-flows what is on screen
    // rather than asking anything again.
    app.panel_mut().set_height(9);
    app.panel_mut().set_width(80);
    let drawn = panel_text(&app, at(base, 5));
    assert_eq!(drawn.len(), 5);
    assert_eq!(drawn.last().map(String::as_str), Some(ANSWER));
}

#[test]
fn a_second_question_goes_under_the_first_rather_than_in_place_of_it() {
    let base = Instant::now();
    let mut app = app_at_the_panel();
    // Nothing asked yet: there is no thread at all, and what a turn would
    // have reported falls on the floor rather than making one.
    assert!(!app.panel().has_thread());
    assert!(app.panel().thread().is_none());
    app.panel_mut().record_turn(&Activity::Thinking, base);
    app.panel_mut().answer_turn("nobody asked", base);
    assert!(!app.panel().has_thread());

    ask_and_answer(&mut app, base);
    app.panel_mut()
        .start_turn("and what did that cost?", at(base, 6));
    app.panel_mut()
        .record_turn(&Activity::Cost { usd: 0.02 }, at(base, 7));
    app.panel_mut().end_turn(&Ending::Cancelled, at(base, 8));

    // One session, one conversation: the second turn is under the first and
    // the first is exactly as it was answered.
    let thread = app.panel().thread().expect("a question has been asked");
    assert_eq!(thread.turns().len(), 2);
    assert_eq!(thread.turns()[0].message(), QUESTION);
    assert_eq!(thread.turns()[0].answer(), Some(ANSWER));
    assert_eq!(thread.turns()[1].message(), "and what did that cost?");
    assert_eq!(thread.turns()[1].ending(), Some(&Ending::Cancelled));

    // A second ending is the first one still: the line on screen wins.
    app.panel_mut().end_turn(&Ending::NothingSaid, at(base, 9));
    let thread = app.panel().thread().expect("a question has been asked");
    assert_eq!(thread.turns()[1].ending(), Some(&Ending::Cancelled));
}

#[test]
fn a_note_brings_the_conversation_forward_and_costs_nobody_a_turn() {
    // Not warlock's own refusal, which is `Submitted::refusal`'s and is written
    // down in one place: any line the panel can be handed will do here.
    const REFUSED: &str = "/plan is not a command warlock has";

    let base = Instant::now();
    let mut app = app_at_the_panel();
    // A file on screen, and not one question asked: the note has to make
    // the card as well as land on it.
    app.show_document(document_lines(), false);
    assert_eq!(app.panel.showing(), Showing::Document);
    assert!(!app.panel().has_thread());

    app.panel_mut().note(REFUSED, base);

    // The conversation comes to the front the way a question brings it: the
    // line answers what the reader just typed, and an answer on a card they
    // are not looking at is not an answer.
    assert_eq!(app.panel.showing(), Showing::Thread);
    assert!(app.panel().has_thread());
    assert_eq!(panel_text(&app, at(base, 9)), [REFUSED]);

    // And it is warlock's own line, not a turn: nobody was asked anything,
    // so nothing is in flight and the composer is free.
    let thread = app.panel().thread().expect("the note made the card");
    assert!(thread.turns().is_empty());
    assert!(thread.in_flight().is_none());
    assert_eq!(thread.line_count(), 1);

    // The card accumulates, as it does for a question: what is said next
    // goes under the note rather than in place of it.
    ask_and_answer(&mut app, base);
    app.panel_mut().note("wrote docs/brief.md", at(base, 6));

    let lines = app
        .panel()
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
    assert_eq!(
        app.panel().thread().map(|thread| thread.turns().len()),
        Some(1)
    );

    // The document is exactly as it was left, lines and all.
    assert!(app.panel().has_document());
    assert_eq!(document_text(&app), document_lines());
}

#[test]
fn a_reload_carries_the_thread_over_with_the_rest_of_the_panel() {
    let base = Instant::now();
    let mut app = app_at_the_panel();
    ask_and_answer(&mut app, base);
    let thread = panel_text(&app, at(base, 9));

    let reseated = reseat_on(app.clone(), &fixture::tree_after_a_rescope());

    // The tree is read again *because* something happened, and a
    // conversation is not a claim about the tree: the card, the turn on it
    // and the card showing all come over.
    assert!(reseated.panel().has_thread());
    assert_eq!(
        reseated.panel().thread().map(|thread| thread.turns().len()),
        Some(1)
    );
    assert_eq!(panel_text(&reseated, at(base, 9)), thread);
}

#[test]
fn a_file_row_hands_its_own_path_to_the_view_key_and_changes_nothing() {
    for path in ["warlock/assets/logo.svg", "warlock/crates/tui/Cargo.toml"] {
        let mut app = app_with_files_selecting(path);
        app.set_message("something the last keystroke said");
        let before = app.clone();

        let asked = app.view_target();

        assert_eq!(asked, Some(PathBuf::from(path)), "{path}");
        // Nothing moved — not the message, not the panel, nothing: the
        // reading has not happened yet.
        assert_eq!(app, before, "an accepted press moved something on {path}");
        assert!(!app.panel().has_content(), "{path} drew something");
    }
}

#[test]
fn a_directory_is_refused_by_the_view_key_naming_the_file_rows() {
    // Scoped or not, with files of its own or none: a directory is not a
    // thing to read.
    for path in ["warlock/crates", "warlock/crates/tui", "warlock/assets"] {
        let mut app = app_selecting(path);
        let mut before = app.clone();

        assert_eq!(app.view_target(), None, "{path} was read");

        let message = app.message().expect("a directory row is refused");
        assert_eq!(
            message,
            format!("{path} is a directory — press v on one of its files to read it")
        );
        // The message is the whole of what the press changed, and the panel
        // is untouched.
        before.set_message(message);
        assert_eq!(app, before, "refusing {path} moved something else");
        assert!(!app.panel().has_content(), "{path} drew something");
    }
}

#[test]
fn an_app_with_no_rows_views_nothing() {
    let mut app = App::from_rows(Vec::new());

    assert_eq!(app.view_target(), None);
    assert_eq!(app.message(), None);
    assert!(!app.panel().has_content());
}

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
        // `app_talking` leaves the panel focused, so the keys reach it; the
        // wheel is asked of an app whose keys are pointed at the tree.
        let mut stepped = app_talking(9, base);
        let mut wheeled = app_talking(9, base);
        wheeled.set_focus(Focus::Tree);
        for _ in 0..lines {
            stepped.select_previous();
        }
        wheeled.scroll_panel_up(lines);
        assert_eq!(
            wheeled.panel().scroll_offset(),
            stepped.panel().scroll_offset(),
            "{lines} up"
        );
        assert_eq!(
            wheeled.panel().follows(),
            stepped.panel().follows(),
            "{lines} up"
        );

        for _ in 0..lines {
            stepped.select_next();
        }
        wheeled.scroll_panel_down(lines);
        assert_eq!(
            wheeled.panel().scroll_offset(),
            stepped.panel().scroll_offset(),
            "{lines} back down"
        );
        assert_eq!(
            wheeled.panel().follows(),
            stepped.panel().follows(),
            "{lines} back down"
        );
    }
}

#[test]
fn the_wheel_over_the_panel_breaks_follow_and_scrolling_back_restores_it() {
    let base = Instant::now();
    let mut app = app_talking(9, base);
    // The keys are pointed at the tree throughout: the pointer is over the
    // panel, and that is the whole of what decides which pane scrolls.
    app.set_focus(Focus::Tree);
    let selected = app.selected();
    assert!(app.panel().follows());

    app.scroll_panel_up(NOTCH);

    assert!(!app.panel().follows());
    assert_eq!(
        panel_text(&app, at(base, 9)),
        ["line 3", "line 4", "line 5"],
    );

    // Parked means parked: the lines that arrive next leave it where it is.
    app.panel_mut()
        .record_turn(&Activity::Thinking, at(base, 10));
    assert_eq!(app.panel().scroll_offset(), 10 - usize::from(PANEL) - NOTCH);
    assert!(!app.panel().follows());

    // And scrolling back to the end is live again, with nothing having to
    // say so.
    app.scroll_panel_down(MANY);

    assert!(app.panel().follows());
    assert_eq!(app.panel().lines_below(), 0);
    assert_eq!(
        panel_text(&app, at(base, 10)),
        ["line 7", "line 8", "thinking"],
    );
    assert_eq!(app.selected(), selected, "the panel moved the tree");
    assert_eq!(app.focus(), Focus::Tree, "the panel took the keys");
}

#[test]
fn the_wheel_over_the_panel_leaves_the_last_keystrokes_message_up() {
    // The same reading a movement key at the panel takes: the line belongs
    // to the tree column, and nothing there has moved.
    let base = Instant::now();
    let mut app = app_talking(9, base);
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
    let coverage = app.coverage();

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
    assert_eq!(app.coverage(), coverage, "the pointer moved the tally");
}

#[test]
fn a_re_seat_carries_the_conversation_and_the_panels_window() {
    let base = Instant::now();
    let mut app = app_talking(9, base);
    app.select_first();
    let before = panel_text(&app, at(base, 9));

    let reseated = reseat_on(app.clone(), &fixture::tree());

    assert_eq!(reseated.panel().thread(), app.panel().thread());
    assert_eq!(reseated.panel().height(), usize::from(PANEL));
    assert_eq!(reseated.panel().scroll_offset(), 0);
    assert!(!reseated.panel().follows());
    assert_eq!(panel_text(&reseated, at(base, 9)), before);
}

#[test]
fn a_re_seat_under_a_document_leaves_it_showing_with_the_conversation_behind_it() {
    let base = Instant::now();
    let mut app = app_talking(9, base);
    app.show_document(document_lines(), false);
    // A line down the document, so what carries is the reader's window and
    // not the top of the card, which a rebuild would arrive at by accident.
    app.select_next();
    let showing = panel_text(&app, at(base, 9));
    assert_eq!(document_window(&app), (1, false));

    // What the watcher does when something on disk changed: a new tree,
    // everything else carried.
    let reseated = reseat_on(app.clone(), &fixture::tree_after_a_rescope());

    // The same card is on screen, on the same line of it. A reload that put
    // the conversation back would take a document out of the reader's hands
    // at exactly the moment they were reading it.
    assert_eq!(reseated.panel.showing(), Showing::Document);
    assert!(reseated.panel().has_document());
    assert_eq!(panel_text(&reseated, at(base, 9)), showing);
    assert_eq!(document_window(&reseated), document_window(&app));
    // And the conversation is intact behind it, lines, window and all.
    assert!(reseated.panel().has_thread());
    assert_eq!(reseated.panel().thread(), app.panel().thread());
    assert_eq!(thread_window(&reseated), thread_window(&app));
    // The card is a snapshot: the reload rewrote none of its lines, and the
    // swap still comes back to the whole of what was read.
    assert_eq!(document_text(&reseated), document_lines());
    let mut reseated = reseated;
    for _ in 0..2 {
        reseated.swap_card();
    }
    assert_eq!(reseated.panel.showing(), Showing::Document);
    assert_eq!(document_text(&reseated), document_lines());
    assert_eq!(panel_text(&reseated, at(base, 9)), showing);
}

fn tree_without_the_tui_crate() -> Tree {
    Tree::new(
        Node::new("warlock").with_children([
            Node::new("warlock/crates")
                .with_scope(Some("platform".to_owned()))
                .with_children([
                    Node::new("warlock/crates/engine").with_scope(Some("data-plane".to_owned()))
                ]),
            Node::new("warlock/assets"),
        ]),
    )
}

fn selected_path(app: &App) -> Option<&Path> {
    app.selected_row().map(|row| row.path.as_path())
}

#[test]
fn a_re_seat_takes_its_rows_and_their_scopes_from_the_new_tree() {
    let app = App::from_tree(&fixture::tree());

    let reseated = reseat_on(app.clone(), &fixture::tree_after_a_rescope());

    // The same five nodes, and the one rescoped carrying the scope written
    // on it, drawn covered.
    assert_eq!(drawn(&reseated), whole_fixture());
    let assets = &reseated.rows()[4];
    assert_eq!(assets.path, PathBuf::from("warlock/assets"));
    assert_eq!(assets.scope.as_deref(), Some("assets"));
    assert!(assets.covered);
    // The old app still says what it always said: a re-seat builds a new
    // value rather than editing the one it was handed.
    assert_eq!(app.rows()[4].path, PathBuf::from("warlock/assets"));
    assert_eq!(app.rows()[4].scope, None);
    assert!(!app.rows()[4].covered);

    assert_eq!(reseated.coverage(), (4, 1));
    assert_eq!(tally(&reseated), reseated.coverage());
}

#[test]
fn a_file_the_new_tree_lists_and_the_old_one_did_not_gets_a_row() {
    let mut app = App::from_tree(&fixture::tree());
    app.toggle_files();
    let written = "warlock/crates/README.md".to_owned();
    assert!(!drawn(&app).contains(&written));

    let tree = Tree::new(
        Node::new("warlock")
            .with_files([PathBuf::from("warlock/README.md")])
            .with_children([Node::new("warlock/crates")
                .with_scope(Some("platform".to_owned()))
                .with_files([PathBuf::from(&written)])]),
    );
    let reseated = reseat_on(app.clone(), &tree);

    assert!(reseated.show_files(), "the file toggle did not carry");
    assert!(
        drawn(&reseated).contains(&written),
        "a file on disk is nowhere on screen"
    );
}

#[test]
fn a_re_seat_keeps_the_selection_on_the_node_it_was_on() {
    let app = app_selecting("warlock/crates/tui");

    let reseated = reseat_on(app.clone(), &fixture::tree_after_a_rescope());

    assert_eq!(
        selected_path(&reseated),
        Some(Path::new("warlock/crates/tui"))
    );
}

#[test]
fn a_selection_the_new_tree_lost_lands_on_its_nearest_surviving_ancestor() {
    let app = app_selecting("warlock/crates/tui");

    let reseated = reseat_on(app.clone(), &tree_without_the_tui_crate());

    // The directory the node was in, which is where it went — not the first
    // row, which is a different part of the tree entirely.
    assert_eq!(selected_path(&reseated), Some(Path::new("warlock/crates")));
    assert_ne!(reseated.selected(), 0);

    // The first row only when nothing on the way to the node survived at
    // all, which is the honest answer rather than a shortcut to it.
    let elsewhere = Tree::new(Node::new("elsewhere"));
    let reseated = reseat_on(app.clone(), &elsewhere);

    assert_eq!(reseated.selected(), 0);
    assert_eq!(selected_path(&reseated), Some(Path::new("elsewhere")));
}

#[test]
fn a_re_seat_carries_the_collapsed_set_including_paths_the_new_tree_lacks() {
    let app = App::from_tree(&fixture::tree()).with_collapsed(["warlock/crates", "warlock/gone"]);

    let reseated = reseat_on(app.clone(), &tree_without_the_tui_crate());

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
    app.toggle_scoped_only();
    app.set_viewport_height(3);
    let mut app = select(app, "warlock/crates/tui");
    app.set_message("something from the last keystroke");
    app.toggle_focus();

    let reseated = reseat_on(app.clone(), &fixture::tree_after_a_rescope());

    assert!(reseated.show_files());
    assert!(reseated.scoped_only());
    assert_eq!(reseated.viewport_height(), 3);
    assert_eq!(reseated.focus(), Focus::Panel);
    assert_eq!(
        reseated.message(),
        Some("something from the last keystroke")
    );
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
    app.toggle_scoped_only();
    let mut app = select(app, "warlock/crates/engine");
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
    app.toggle_focus();
    // A view with something to lose in every field there is.
    assert!(app.scroll_offset() > 0);
    assert!(app.selected() < app.scroll_offset() + app.viewport_height() - 1);

    // What the binary does when a load turns up a tree nothing has happened
    // to: every field of the re-seated app is the field it carried, down to
    // the scroll offset.
    let reseated = reseat_on(app.clone(), &fixture::tree());

    assert_eq!(reseated, app);
}
