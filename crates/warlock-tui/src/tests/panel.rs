use std::time::{Duration, Instant};

use super::{Mode, Panel, Showing};
use crate::account::Line;

fn base() -> Instant {
    Instant::now()
}

fn sized() -> Panel {
    let mut panel = Panel::default();
    panel.set_height(10);
    panel.set_width(40);
    panel
}

#[test]
fn a_fresh_panel_opens_on_the_conversation_with_nothing_in_it() {
    let panel = Panel::default();

    assert_eq!(panel.showing(), Showing::Thread);
    assert_eq!(panel.mode(), Mode::Chat);
    assert!(!panel.has_account());
    assert!(!panel.has_thread());
    assert!(!panel.has_document());
    assert!(
        !panel.has_content(),
        "an app that has just started has nothing to show"
    );
}

#[test]
fn the_swap_key_has_nowhere_to_go_until_something_else_has_anything_in_it() {
    let mut panel = sized();

    assert_eq!(
        panel.next_card(),
        None,
        "a panel holding only an empty conversation has nowhere to swap to"
    );

    panel.open_account(base());

    assert_eq!(
        panel.next_card(),
        Some(Showing::Account),
        "a run has happened and the swap key should reach it"
    );
}

#[test]
fn the_swap_key_steps_over_the_card_with_nothing_in_it() {
    let mut panel = sized();
    panel.show_document(["a line of a file"], false);

    // Showing the document, with no account ever started: the only other
    // card worth stopping on is the conversation.
    assert_eq!(panel.showing(), Showing::Document);
    assert_eq!(panel.next_card(), Some(Showing::Thread));
}

#[test]
fn each_card_keeps_its_own_place_while_another_one_is_showing() {
    let mut panel = sized();
    panel.open_account(base());
    panel.show_document((0..40).map(|n| format!("line {n}")), false);

    // The document is showing and scrolled off its own tail; the account
    // behind it is untouched and still following.
    panel.scroll_to(3);

    assert_eq!(panel.window_of(Showing::Document).0, 3);
    assert!(
        !panel.window_of(Showing::Document).1,
        "a card scrolled by hand stops following its own newest line"
    );
    assert!(
        panel.window_of(Showing::Account).1,
        "the card nobody touched stopped following"
    );
}

#[test]
fn a_document_is_shown_the_moment_it_is_filled() {
    let mut panel = sized();

    panel.show_document(["one line"], false);

    assert_eq!(panel.showing(), Showing::Document);
    assert!(panel.has_document());
    assert_eq!(
        panel.document_lines(),
        [Line::Text {
            text: "one line".to_owned()
        }]
    );
}

#[test]
fn a_second_read_replaces_the_document_rather_than_adding_to_it() {
    let mut panel = sized();
    panel.show_document(["before"], false);

    panel.show_document(["after"], false);

    assert_eq!(
        panel.document_lines().len(),
        1,
        "the card grew instead of being replaced: {:?}",
        panel.document_lines()
    );
}

#[test]
fn a_mode_change_is_reported_only_when_it_changes_something() {
    let mut panel = Panel::default();

    assert!(panel.set_mode(Mode::Brief), "chat to brief is a change");
    assert_eq!(panel.mode(), Mode::Brief);
    assert!(
        !panel.set_mode(Mode::Brief),
        "brief to brief is not a change and must not be announced as one"
    );
}

#[test]
fn a_run_opens_a_fresh_account_over_whatever_the_last_one_left() {
    let mut panel = sized();
    // Showing it as well as opening it: `open_account` places the card, and
    // putting it on screen is the app's own step (see `App::start_account`).
    panel.show(Showing::Account);
    panel.open_account(base());
    panel.write_run(|account| account.open_section("crates", base()));
    let first = panel.window(base()).len();

    panel.open_account(base() + Duration::from_secs(1));

    assert!(first > 0, "the first run wrote nothing to begin with");
    assert!(
        panel.window(base() + Duration::from_secs(1)).len() < first,
        "the new run inherited the old one's lines"
    );
}

#[test]
fn the_window_is_the_height_it_was_given_and_no_more() {
    let mut panel = sized();
    panel.show_document((0..100).map(|n| format!("line {n}")), false);

    assert_eq!(
        panel.window(base()).len(),
        10,
        "the window is not the panel's height"
    );
    assert!(
        panel.lines_below() > 0,
        "a hundred lines in a ten-line window has nothing below it"
    );
}

fn talked_at() -> Panel {
    let mut panel = sized();
    panel.start_turn("a question", base());
    for n in 0..30 {
        panel.note(format!("note {n}"), base());
    }
    panel
}

#[test]
fn a_held_thread_card_stays_where_it_was_while_text_arrives_under_it() {
    let mut panel = talked_at();
    let (offset, follows) = panel.window_of(Showing::Thread);
    let rows = panel.window(base());

    panel.hold_thread(true);
    panel.note("arriving while the button is down", base());

    assert!(follows, "a conversation nobody scrolled stopped following");
    assert_eq!(
        panel.window_of(Showing::Thread).0,
        offset,
        "the window moved under a held drag"
    );
    assert_eq!(panel.window(base()), rows, "the rows drawn changed");
}

#[test]
fn a_held_card_that_was_following_goes_back_to_the_newest_line_on_release() {
    let mut panel = talked_at();
    panel.hold_thread(true);
    panel.note("arriving while the button is down", base());

    panel.hold_thread(false);
    panel.note("arriving after it came up", base());

    assert!(
        panel.window_of(Showing::Thread).1,
        "following was cleared by the hold rather than suspended"
    );
    assert_eq!(
        panel.lines_below(),
        0,
        "the window is not at the newest row"
    );
}

#[test]
fn a_card_that_was_not_following_is_not_following_after_a_release() {
    let mut panel = talked_at();
    panel.scroll_to(2);
    panel.hold_thread(true);
    panel.note("arriving while the button is down", base());
    panel.hold_thread(false);

    assert!(
        !panel.window_of(Showing::Thread).1,
        "a card the reader had scrolled up started following on release"
    );
    assert_eq!(
        panel.window_of(Showing::Thread).0,
        2,
        "the reader's place was lost"
    );
}
