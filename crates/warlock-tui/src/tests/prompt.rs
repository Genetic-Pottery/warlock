use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};

use super::{Edited, ScopeField, ScopePrompt, edit_for};

// Every field below is opened over this, so a test that meant to change the
// text cannot pass by changing the directory instead.
const DIRECTORY: &str = "crates/warlock-engine";

fn press(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn field(text: &str) -> ScopeField {
    ScopeField::new(DIRECTORY, text)
}

fn after(key: KeyEvent, field: &ScopeField) -> ScopeField {
    match edit_for(key, field) {
        Edited::Open(next) => next,
        other => panic!("{key:?} should have left the prompt open, and gave {other:?}"),
    }
}

// Every key the tree answers to, plus a character bound to nothing anywhere:
// the list the prompt has to swallow whole, so no keystroke reaches the app
// underneath while the field is up.
const BINDINGS: [KeyCode; 17] = [
    KeyCode::Char('j'),
    KeyCode::Char('k'),
    KeyCode::Char('g'),
    KeyCode::Char('G'),
    KeyCode::Char(' '),
    KeyCode::Char('o'),
    KeyCode::Char('f'),
    KeyCode::Char('p'),
    KeyCode::Char('r'),
    KeyCode::Char('m'),
    KeyCode::Char('s'),
    KeyCode::Tab,
    KeyCode::PageUp,
    KeyCode::PageDown,
    KeyCode::Up,
    KeyCode::Down,
    KeyCode::Char('x'),
];

fn typed(code: KeyCode) -> Option<char> {
    match code {
        KeyCode::Char(character) => Some(character),
        _ => None,
    }
}

#[test]
fn a_fresh_prompt_is_up_over_the_directory_and_the_scope_it_carries() {
    // It opens on what is true: the scope the directory has now, so Enter
    // straight after the key that opened this writes back what was already
    // there rather than clearing it.
    let prompt = ScopePrompt::open(DIRECTORY, "data-plane");

    assert_eq!(prompt, ScopePrompt::Open(field("data-plane")));
    assert!(prompt.is_open());
    assert_eq!(prompt.field(), Some(&field("data-plane")));
    assert_eq!(prompt.field().map(ScopeField::directory), Some(DIRECTORY));
    assert_eq!(prompt.field().map(ScopeField::text), Some("data-plane"));
    assert_eq!(prompt.field().and_then(ScopeField::rule), None);
}

#[test]
fn a_directory_carrying_no_scope_opens_on_an_empty_field() {
    let prompt = ScopePrompt::open(DIRECTORY, "");

    assert_eq!(prompt.field().map(ScopeField::text), Some(""));
    assert_eq!(prompt.field().and_then(ScopeField::rule), None);
}

#[test]
fn a_closed_prompt_is_the_default_and_holds_no_field() {
    // `Default` is what the event loop starts a session on, and a closed
    // prompt has no text to draw or to hand to `edit_for`.
    assert_eq!(ScopePrompt::default(), ScopePrompt::Closed);
    assert!(!ScopePrompt::Closed.is_open());
    assert_eq!(ScopePrompt::Closed.field(), None);
}

#[test]
fn printable_characters_append_in_the_order_they_are_typed() {
    let mut current = field("");
    for character in "data-plane_2".chars() {
        current = after(press(KeyCode::Char(character)), &current);
    }

    assert_eq!(current, field("data-plane_2"));
}

#[test]
fn characters_append_exactly_as_typed_and_are_not_folded_or_refused() {
    // Case folding belongs to the caller that asks the engine, and judging
    // belongs to the engine: a field that silently dropped a capital or a
    // comma would leave somebody typing at a prompt that ignores them.
    let mut current = field("");
    for character in "Control Plane, 9".chars() {
        current = after(press(KeyCode::Char(character)), &current);
    }

    assert_eq!(current.text(), "Control Plane, 9");
}

#[test]
fn backspace_takes_back_one_character_at_a_time() {
    assert_eq!(after(press(KeyCode::Backspace), &field("web")), field("we"));
    assert_eq!(after(press(KeyCode::Backspace), &field("w")), field(""));
}

#[test]
fn backspace_takes_back_a_character_and_not_a_byte() {
    // A multi-byte character is not a scope and never will be, but it can
    // be typed, and a delete that took one byte of it would leave the
    // buffer holding half a character.
    assert_eq!(after(press(KeyCode::Backspace), &field("wéb")), field("wé"));
    assert_eq!(after(press(KeyCode::Backspace), &field("wé")), field("w"));
}

#[test]
fn backspace_on_an_empty_field_changes_nothing_and_does_not_close() {
    // One press past the start is a typo, not an abandonment: Esc is the
    // key that leaves, and it is the only one.
    let empty = field("");

    assert_eq!(
        edit_for(press(KeyCode::Backspace), &empty),
        Edited::Open(empty.clone())
    );

    let refused = empty.refused("a scope cannot be empty");
    assert_eq!(
        edit_for(press(KeyCode::Backspace), &refused),
        Edited::Open(refused.clone()),
        "a key that changes nothing leaves the rule line where it was"
    );
}

#[test]
fn enter_submits_whatever_is_there_including_nothing() {
    // An empty field is how a scope is cleared, so it is an answer rather
    // than an error, and it is not this module's business either way.
    for text in ["data-plane", "", "  ", "control-plane, data-plane"] {
        assert_eq!(
            edit_for(press(KeyCode::Enter), &field(text)),
            Edited::Submit,
            "Enter should submit {text:?}"
        );
    }
}

#[test]
fn esc_closes_from_any_text() {
    for text in ["", "data-plane", "not a scope at all"] {
        assert_eq!(
            edit_for(press(KeyCode::Esc), &field(text)),
            Edited::Close,
            "Esc should close over {text:?}"
        );
    }
}

#[test]
fn a_refusal_reopens_the_field_with_the_text_untouched() {
    let refused = field("data plane").refused("a scope holds only lowercase letters");

    assert_eq!(refused.text(), "data plane");
    assert_eq!(refused.directory(), DIRECTORY);
    assert_eq!(refused.rule(), Some("a scope holds only lowercase letters"));
}

#[test]
fn an_edit_clears_the_rule_line_and_a_non_edit_leaves_it() {
    // The line names a rule the text broke, and the text has just changed.
    let refused = field("data plane").refused("a scope holds only lowercase letters");

    assert_eq!(after(press(KeyCode::Backspace), &refused).rule(), None);
    assert_eq!(after(press(KeyCode::Char('x')), &refused).rule(), None);
    assert_eq!(
        after(press(KeyCode::Tab), &refused).rule(),
        Some("a scope holds only lowercase letters"),
        "a key that types nothing has not made the complaint stale"
    );
}

#[test]
fn every_tree_binding_is_swallowed_as_text_or_as_nothing() {
    // Asserted over the whole list because "nothing reaches the app" is the
    // promise: a stray `p` that pacted something behind the prompt, or a
    // `j` that moved a selection nobody can see, would be the same accident
    // by two routes.
    for code in BINDINGS {
        let before = field("web");
        let next = after(press(code), &before);

        let expected = typed(code).map_or_else(
            || before.clone(),
            |character| field(&format!("web{character}")),
        );

        assert_eq!(next, expected, "{code:?} should be text or nothing");
        assert_eq!(next.directory(), DIRECTORY);
    }
}

#[test]
fn nothing_but_enter_submits_and_nothing_but_esc_closes() {
    // Said the other way round, because what must not happen is the
    // interesting half: of every key this module sees, one writes and one
    // abandons, and no third key does either.
    for code in BINDINGS.into_iter().chain([KeyCode::Backspace]) {
        let outcome = edit_for(press(code), &field("web"));

        assert_ne!(outcome, Edited::Submit, "{code:?} should not submit");
        assert_ne!(outcome, Edited::Close, "{code:?} should not close");
    }
}

#[test]
fn ctrl_c_is_not_answered_here() {
    // It is a key event, not a signal, and the loop has to take it before
    // it consults this mode. Through here it is one of the keys that change
    // nothing — which is why the loop answering it first is load-bearing
    // rather than tidy, and why a chord is never text.
    let ctrl_c = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
    let before = field("web");

    assert_eq!(edit_for(ctrl_c, &before), Edited::Open(before.clone()));
}

#[test]
fn a_chord_or_a_control_character_is_not_text() {
    // Every modifier that makes a character a command rather than a letter,
    // plus a control character arriving as itself.
    let before = field("web");

    for modifiers in [
        KeyModifiers::CONTROL,
        KeyModifiers::ALT,
        KeyModifiers::SUPER,
        KeyModifiers::HYPER,
        KeyModifiers::META,
        KeyModifiers::CONTROL | KeyModifiers::SHIFT,
    ] {
        let key = KeyEvent::new(KeyCode::Char('u'), modifiers);

        assert_eq!(
            edit_for(key, &before),
            Edited::Open(before.clone()),
            "{modifiers:?} makes `u` a command, not a letter"
        );
    }

    assert_eq!(
        edit_for(press(KeyCode::Char('\u{7}')), &before),
        Edited::Open(before.clone()),
        "a control character is not text however it arrived"
    );
}

#[test]
fn shift_still_types() {
    // Terminals disagree about whether the modifier rides along with an
    // upper-case letter, and either way it is somebody typing.
    let key = KeyEvent::new(KeyCode::Char('W'), KeyModifiers::SHIFT);

    assert_eq!(after(key, &field("")), field("W"));
}

#[test]
fn releases_and_repeats_type_nothing() {
    // The same rule as `action_for`: acting on a release would type the
    // release of the very key that opened the prompt, and `s` would arrive
    // in its own field.
    let before = field("web");

    for code in [
        KeyCode::Char('s'),
        KeyCode::Backspace,
        KeyCode::Enter,
        KeyCode::Esc,
    ] {
        for kind in [KeyEventKind::Release, KeyEventKind::Repeat] {
            let event = KeyEvent::new_with_kind_and_state(
                code,
                KeyModifiers::NONE,
                kind,
                KeyEventState::NONE,
            );

            assert_eq!(
                edit_for(event, &before),
                Edited::Open(before.clone()),
                "{kind:?} of {code:?} should do nothing"
            );
        }
    }
}

#[test]
fn no_key_ever_changes_the_directory_being_scoped() {
    // The heading is what the window is about, and it is decided when the
    // prompt opens: nothing typed into the field can move it.
    let before = field("web").refused("a scope cannot be empty");

    for code in BINDINGS.into_iter().chain([
        KeyCode::Backspace,
        KeyCode::Left,
        KeyCode::Right,
        KeyCode::Home,
        KeyCode::End,
        KeyCode::Delete,
    ]) {
        assert_eq!(after(press(code), &before).directory(), DIRECTORY);
    }
}

#[test]
fn the_editing_keys_this_field_still_does_not_have_do_nothing() {
    // No selection, no history, and one line, so no Up and no Down: what
    // the cursor answers is Left, Right, Home, End and the two deletes, and
    // the tests below are those.
    let before = field("web");

    for code in [
        KeyCode::Insert,
        KeyCode::BackTab,
        KeyCode::Up,
        KeyCode::Down,
    ] {
        assert_eq!(
            edit_for(press(code), &before),
            Edited::Open(before.clone()),
            "{code:?} is not an editing key this field has"
        );
    }
}

// Pressed one at a time from a field that opens with its cursor at the end,
// because that is the only way in: the cursor is not a thing a caller sets.
fn walked(text: &str, keys: &[KeyCode]) -> ScopeField {
    keys.iter()
        .fold(field(text), |current, code| after(press(*code), &current))
}

#[test]
fn a_field_opens_with_its_cursor_after_what_is_already_there() {
    assert_eq!(field("web").cursor(), 3);
    assert_eq!(field("").cursor(), 0);
}

#[test]
fn the_cursor_walks_one_character_at_a_time_and_stops_at_both_ends() {
    assert_eq!(walked("web", &[KeyCode::Left]).cursor(), 2);
    assert_eq!(walked("web", &[KeyCode::Left, KeyCode::Left]).cursor(), 1);
    assert_eq!(walked("web", &[KeyCode::Left, KeyCode::Right]).cursor(), 3);

    // Walked past both ends and still on the text: one press past the edge
    // is a key that changes nothing, exactly as Backspace on an empty field
    // is.
    let left = [KeyCode::Left; 5];
    assert_eq!(walked("web", &left).cursor(), 0);
    assert_eq!(walked("web", &[KeyCode::Right; 5]).cursor(), 3);
}

#[test]
fn the_cursor_moves_by_characters_and_not_by_bytes() {
    // Two bytes, one character, one press: a cursor counted in bytes would
    // land inside `é` and panic the next slice of this text.
    assert_eq!(walked("wéb", &[KeyCode::Left, KeyCode::Left]).cursor(), 1);
    assert_eq!(walked("wéb", &[KeyCode::Home, KeyCode::Right]).cursor(), 1);
}

#[test]
fn home_goes_to_the_start_and_end_goes_back_to_the_finish() {
    assert_eq!(walked("web", &[KeyCode::Home]).cursor(), 0);
    assert_eq!(walked("web", &[KeyCode::Home, KeyCode::End]).cursor(), 3);
}

#[test]
fn a_character_lands_where_the_cursor_is_and_the_cursor_follows_it() {
    let typed = walked(
        "web",
        &[KeyCode::Home, KeyCode::Char('m'), KeyCode::Char('y')],
    );

    assert_eq!(typed.text(), "myweb");
    assert_eq!(typed.cursor(), 2, "the cursor stayed behind what was typed");
}

#[test]
fn backspace_takes_the_character_behind_the_cursor_and_delete_the_one_in_front() {
    let back = walked("web", &[KeyCode::Left, KeyCode::Backspace]);
    assert_eq!(back.text(), "wb");
    assert_eq!(back.cursor(), 1);

    let forward = walked("web", &[KeyCode::Left, KeyCode::Delete]);
    assert_eq!(forward.text(), "we");
    assert_eq!(
        forward.cursor(),
        2,
        "a delete in front of the cursor moved it"
    );
}

#[test]
fn a_delete_at_either_edge_changes_nothing_and_does_not_close() {
    let start = walked("web", &[KeyCode::Home]);
    assert_eq!(
        edit_for(press(KeyCode::Backspace), &start),
        Edited::Open(start.clone()),
        "Backspace with nothing behind the cursor"
    );

    let end = field("web");
    assert_eq!(
        edit_for(press(KeyCode::Delete), &end),
        Edited::Open(end.clone()),
        "Delete with nothing in front of the cursor"
    );
}

#[test]
fn a_move_leaves_the_rule_line_up_and_an_edit_takes_it_down() {
    // The line names something wrong with the text. Walking the cursor over
    // it leaves it just as wrong, so the complaint stands; changing a
    // character is what makes it stale.
    let refused = field("web team").refused("a scope is one word");

    assert_eq!(
        after(press(KeyCode::Left), &refused).rule(),
        Some("a scope is one word"),
        "a move made the complaint stale"
    );
    assert_eq!(after(press(KeyCode::Backspace), &refused).rule(), None);
}
