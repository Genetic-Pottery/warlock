use std::fs;
use std::path::Path;

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
use warlock_engine::{keys, keys_path, load_key, save_key};

use super::{
    Edited, FIELD_CHARS, KeyChoice, KeyEdited, KeyPrompt, RecordEdited, RecordField, RecordForm,
    RecordPrompt, RecordTracker, ScopeField, ScopePrompt, edit_for, key_edit_for, record_edit_for,
};

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
    KeyCode::Char('v'),
    KeyCode::Char('e'),
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

    let opened = field("data-plane").capped(warlock_engine::scope::MAXIMUM_CHARACTERS);
    assert_eq!(prompt, ScopePrompt::Open(opened.clone()));
    assert!(prompt.is_open());
    assert_eq!(prompt.field(), Some(&opened));
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
    // promise: a stray `e` that opened an editor behind the prompt, or a
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

// The name the first prompt settled on, already folded, which is what the
// record is written under.
const SCOPE: &str = "data-plane";

fn form() -> RecordForm {
    RecordForm::new(RecordTracker::Linear, DIRECTORY, SCOPE)
}

// The Linear form's fields, in the order it holds them, which is what the tests
// below walk and type into.
const LINEAR: [RecordField; 3] = [
    RecordField::TeamKey,
    RecordField::ReviewState,
    RecordField::Label,
];

// A field the form is known to hold, so a test about its text or its rule line
// fails loudly rather than reading `None` as the answer it was looking for.
fn held(form: &RecordForm, which: RecordField) -> &ScopeField {
    form.field(which)
        .unwrap_or_else(|| panic!("this form holds no {}", which.name()))
}

fn names(form: &RecordForm) -> Vec<RecordField> {
    form.fields().iter().map(|(which, _)| *which).collect()
}

fn after_record(key: KeyEvent, form: &RecordForm) -> RecordForm {
    match record_edit_for(key, form) {
        RecordEdited::Open(next) => next,
        other => panic!("{key:?} should have left the record open, and gave {other:?}"),
    }
}

fn typed_into(form: &RecordForm, text: &str) -> RecordForm {
    text.chars().fold(form.clone(), |current, character| {
        after_record(press(KeyCode::Char(character)), &current)
    })
}

fn texts(form: &RecordForm) -> Vec<&str> {
    form.fields()
        .iter()
        .map(|(_, field)| field.text())
        .collect()
}

#[test]
fn a_fresh_record_is_up_over_the_path_and_the_scope_with_three_empty_fields() {
    let prompt = RecordPrompt::open(RecordTracker::Linear, DIRECTORY, SCOPE);

    assert!(prompt.is_open());
    assert_eq!(prompt.form(), Some(&form()));
    assert_eq!(prompt.form().map(RecordForm::path), Some(DIRECTORY));
    assert_eq!(prompt.form().map(RecordForm::scope), Some(SCOPE));
    assert_eq!(texts(&form()), ["", "", ""]);
    assert_eq!(form().focus(), RecordField::TeamKey);
    // Nothing is guessable from the scope name, so no field opens complaining
    // about text nobody has typed yet.
    for which in LINEAR {
        assert_eq!(held(&form(), which).rule(), None);
    }
}

#[test]
fn a_form_holds_its_own_tracker_s_fields_and_no_others() {
    // Linear's team key where Jira's two are, and the two every record carries
    // under both, each in the order that tracker's record is written in.
    assert_eq!(names(&form()), LINEAR);
    assert_eq!(
        names(&RecordForm::new(RecordTracker::Jira, DIRECTORY, SCOPE)),
        [
            RecordField::BaseUrl,
            RecordField::ProjectKey,
            RecordField::ReviewState,
            RecordField::Label,
        ]
    );

    // And a field the tracker does not need is not an empty one: there is no
    // answer about it at all, so nothing can read a blank out of a form that
    // was never going to be asked for it.
    for which in [RecordField::BaseUrl, RecordField::ProjectKey] {
        assert_eq!(form().field(which), None, "{which:?}");
    }
    let jira = RecordForm::new(RecordTracker::Jira, DIRECTORY, SCOPE);
    assert_eq!(jira.field(RecordField::TeamKey), None);
    assert_eq!(texts(&jira), ["", "", "", ""]);
    assert_eq!(jira.focus(), RecordField::BaseUrl);
}

#[test]
fn a_refusal_about_a_field_the_tracker_does_not_need_changes_nothing() {
    // The fields a refusal can name are the ones the form handed over, so this
    // is a road nothing takes; what it must not do is move the focus onto a
    // field that is not on screen or hang a rule line on one.
    let before = typed_into(&form(), "platform");

    for which in [RecordField::BaseUrl, RecordField::ProjectKey] {
        let after = before.clone().refused(which, "a base URL cannot be blank");

        assert_eq!(after, before, "{which:?}");
        assert_eq!(after.focus(), RecordField::TeamKey);
    }
}

#[test]
fn tab_walks_a_jira_form_through_its_four_fields_and_wraps() {
    let jira = RecordForm::new(RecordTracker::Jira, DIRECTORY, SCOPE);
    let mut current = jira.clone();

    for expected in [
        RecordField::ProjectKey,
        RecordField::ReviewState,
        RecordField::Label,
        // Wrapped, and onto its own first field rather than Linear's.
        RecordField::BaseUrl,
    ] {
        current = after_record(press(KeyCode::Tab), &current);
        assert_eq!(current.focus(), expected);
    }

    current = after_record(press(KeyCode::Up), &jira);
    assert_eq!(current.focus(), RecordField::Label);
    // Nothing on the walk is Linear's, and typing follows the focus: the team
    // key is not a field this form has to type into.
    let typed = typed_into(&jira, "https://acme.atlassian.net");
    assert_eq!(
        texts(&typed),
        ["https://acme.atlassian.net", "", "", ""],
        "the first field a Jira form offers is its base URL"
    );
    assert_eq!(typed.field(RecordField::TeamKey), None);
}

#[test]
fn a_closed_record_is_the_default_and_holds_no_form() {
    assert_eq!(RecordPrompt::default(), RecordPrompt::Closed);
    assert!(!RecordPrompt::Closed.is_open());
    assert_eq!(RecordPrompt::Closed.form(), None);
}

#[test]
fn a_character_lands_in_the_focused_field_and_in_no_other() {
    let typed = typed_into(&form(), "platform");
    assert_eq!(texts(&typed), ["platform", "", ""]);

    let moved = typed_into(&after_record(press(KeyCode::Tab), &typed), "in review");
    assert_eq!(texts(&moved), ["platform", "in review", ""]);

    let last = typed_into(&after_record(press(KeyCode::Tab), &moved), "WAR-105");
    assert_eq!(texts(&last), ["platform", "in review", "WAR-105"]);
}

#[test]
fn tab_walks_the_fields_forward_and_back_tab_walks_them_back() {
    let mut current = form();
    for expected in [
        RecordField::ReviewState,
        RecordField::Label,
        // Wrapped: there is nothing past the last field but the first one.
        RecordField::TeamKey,
    ] {
        current = after_record(press(KeyCode::Tab), &current);
        assert_eq!(current.focus(), expected);
    }

    for expected in [
        RecordField::Label,
        RecordField::ReviewState,
        RecordField::TeamKey,
    ] {
        current = after_record(press(KeyCode::BackTab), &current);
        assert_eq!(current.focus(), expected);
    }
}

#[test]
fn the_arrows_that_the_field_itself_ignores_walk_between_the_fields() {
    // Down and Up are not text and not editing keys in a one-line field, so
    // the record can be filled in without a keystroke that could have been a
    // letter.
    assert_eq!(
        after_record(press(KeyCode::Down), &form()).focus(),
        RecordField::ReviewState
    );
    assert_eq!(
        after_record(press(KeyCode::Up), &form()).focus(),
        RecordField::Label
    );
}

#[test]
fn moving_between_fields_types_nothing_and_edits_nothing() {
    let typed = typed_into(&form(), "platform");

    for code in [KeyCode::Tab, KeyCode::BackTab, KeyCode::Up, KeyCode::Down] {
        let next = after_record(press(code), &typed);

        assert_eq!(
            texts(&next),
            ["platform", "", ""],
            "{code:?} typed something"
        );
        assert_eq!(held(&next, RecordField::TeamKey).cursor(), 8);
    }
}

#[test]
fn a_refusal_names_the_field_it_is_about_and_puts_the_cursor_in_it() {
    let typed = typed_into(&form(), "platform");
    let refused = typed.refused(RecordField::Label, "a label cannot be blank");

    assert_eq!(refused.focus(), RecordField::Label);
    assert_eq!(
        held(&refused, RecordField::Label).rule(),
        Some("a label cannot be blank")
    );
    assert_eq!(held(&refused, RecordField::TeamKey).rule(), None);
    assert_eq!(
        texts(&refused),
        ["platform", "", ""],
        "a refusal leaves every field's text where it was"
    );
}

#[test]
fn a_rule_line_survives_a_move_and_goes_down_on_an_edit_to_that_field() {
    let refused = form().refused(RecordField::TeamKey, "a team key cannot be blank");

    let walked = after_record(press(KeyCode::Tab), &refused);
    assert_eq!(
        held(&walked, RecordField::TeamKey).rule(),
        Some("a team key cannot be blank"),
        "walking off the field left the text just as blank"
    );

    let back = after_record(press(KeyCode::BackTab), &walked);
    assert_eq!(
        held(&back, RecordField::TeamKey).rule(),
        Some("a team key cannot be blank")
    );
    assert_eq!(
        held(
            &after_record(press(KeyCode::Home), &back),
            RecordField::TeamKey
        )
        .rule(),
        Some("a team key cannot be blank"),
        "a cursor move inside the field is not an edit either"
    );

    let edited = after_record(press(KeyCode::Char('p')), &back);
    assert_eq!(held(&edited, RecordField::TeamKey).rule(), None);
    // Only the field that was typed into: a complaint about the label is about
    // text nobody has touched.
    let elsewhere = form().refused(RecordField::Label, "a label cannot be blank");
    let elsewhere = typed_into(&after_record(press(KeyCode::Tab), &elsewhere), "x");
    assert_eq!(
        held(&elsewhere, RecordField::Label).rule(),
        Some("a label cannot be blank")
    );
}

#[test]
fn the_focused_field_edits_exactly_as_the_scope_field_does() {
    let typed = typed_into(&form(), "platfrom");
    let fixed = [
        KeyCode::Left,
        KeyCode::Left,
        KeyCode::Left,
        KeyCode::Delete,
        KeyCode::Right,
        KeyCode::Char('r'),
        KeyCode::End,
        KeyCode::Char('!'),
    ]
    .iter()
    .fold(typed, |current, code| after_record(press(*code), &current));

    assert_eq!(texts(&fixed), ["platform!", "", ""]);
    assert_eq!(held(&fixed, RecordField::TeamKey).cursor(), 9);
}

#[test]
fn esc_closes_and_enter_submits_from_whichever_field_is_focused() {
    let mut current = typed_into(&form(), "platform");

    for _ in LINEAR {
        assert_eq!(
            record_edit_for(press(KeyCode::Esc), &current),
            RecordEdited::Close
        );
        assert_eq!(
            record_edit_for(press(KeyCode::Enter), &current),
            RecordEdited::Submit,
            "blankness is the caller's refusal, not this module's"
        );
        current = after_record(press(KeyCode::Tab), &current);
    }
}

#[test]
fn nothing_but_enter_submits_and_nothing_but_esc_closes_in_the_record() {
    for code in BINDINGS.into_iter().chain([
        KeyCode::Backspace,
        KeyCode::Delete,
        KeyCode::BackTab,
        KeyCode::Left,
        KeyCode::Right,
    ]) {
        let outcome = record_edit_for(press(code), &form());

        assert_ne!(outcome, RecordEdited::Submit, "{code:?} should not submit");
        assert_ne!(outcome, RecordEdited::Close, "{code:?} should not close");
    }
}

#[test]
fn a_chord_or_a_control_character_is_not_text_in_the_record_either() {
    let before = typed_into(&form(), "platform");

    for modifiers in [
        KeyModifiers::CONTROL,
        KeyModifiers::ALT,
        KeyModifiers::SUPER,
        KeyModifiers::HYPER,
        KeyModifiers::META,
    ] {
        let key = KeyEvent::new(KeyCode::Char('u'), modifiers);

        assert_eq!(
            record_edit_for(key, &before),
            RecordEdited::Open(before.clone()),
            "{modifiers:?} makes `u` a command, not a letter"
        );
    }

    assert_eq!(
        record_edit_for(press(KeyCode::Char('\u{7}')), &before),
        RecordEdited::Open(before.clone())
    );
    assert_eq!(
        held(
            &after_record(
                KeyEvent::new(KeyCode::Char('W'), KeyModifiers::SHIFT),
                &before
            ),
            RecordField::TeamKey
        )
        .text(),
        "platformW",
        "shift still types"
    );
}

#[test]
fn releases_and_repeats_change_nothing_in_the_record() {
    let before = typed_into(&form(), "platform");

    for code in [
        KeyCode::Char('s'),
        KeyCode::Tab,
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
                record_edit_for(event, &before),
                RecordEdited::Open(before.clone()),
                "{kind:?} of {code:?} should do nothing"
            );
        }
    }
}

#[test]
fn no_key_ever_changes_the_path_or_the_scope_being_recorded() {
    // Both are settled by the prompt that came before this one: the record
    // written from here has to be for the scope that prompt drew.
    let before = form().refused(RecordField::TeamKey, "a team key cannot be blank");

    for code in BINDINGS.into_iter().chain([
        KeyCode::Backspace,
        KeyCode::Delete,
        KeyCode::Left,
        KeyCode::Right,
        KeyCode::Home,
        KeyCode::End,
        KeyCode::BackTab,
    ]) {
        let next = after_record(press(code), &before);

        assert_eq!(next.path(), DIRECTORY);
        assert_eq!(next.scope(), SCOPE);
    }
}

#[test]
fn typing_stops_at_the_cap() {
    let full = field(&"é".repeat(FIELD_CHARS));
    assert_eq!(after(press(KeyCode::Char('x')), &full), full);

    let short = after(press(KeyCode::Backspace), &full);
    let typed = after(press(KeyCode::Char('x')), &short);
    assert_eq!(typed.text().chars().count(), FIELD_CHARS);
    assert!(typed.text().ends_with('x'));
}

#[test]
fn a_field_opened_past_the_cap_keeps_its_text_and_still_edits() {
    let long = field(&"a".repeat(FIELD_CHARS + 20));
    assert_eq!(long.text().chars().count(), FIELD_CHARS + 20);
    assert_eq!(after(press(KeyCode::Char('x')), &long), long);
    let trimmed = after(press(KeyCode::Backspace), &long);
    assert_eq!(trimmed.text().chars().count(), FIELD_CHARS + 19);
}

#[test]
fn the_scope_prompt_stops_at_the_engines_ceiling_and_the_path_window_does_not() {
    let scope = "a".repeat(warlock_engine::scope::MAXIMUM_CHARACTERS);
    let full = ScopePrompt::open(DIRECTORY, scope.as_str())
        .field()
        .expect("open")
        .clone();
    assert_eq!(after(press(KeyCode::Char('x')), &full), full);

    let path = ScopePrompt::open_path(DIRECTORY, scope.as_str())
        .field()
        .expect("open")
        .clone();
    let typed = after(press(KeyCode::Char('x')), &path);
    assert_eq!(typed.text().chars().count(), scope.len() + 1);
}

#[test]
fn a_field_taken_out_of_a_form_still_takes_typing() {
    let taken = ScopeField::default();
    assert_eq!(after(press(KeyCode::Char('x')), &taken).text(), "x");
}

// Stored under every name below, so "no part of this window holds a key" has one
// needle to look for and a window that leaked could not leak a value the
// assertions do not know about.
const SECRET: &str = "lin_api_this_value_must_never_be_held";

// Every home here is a temporary one, so nothing below reads the developer's
// real `~/.warlock` or writes a key into it.
fn a_dir() -> tempfile::TempDir {
    tempfile::tempdir().expect("a temporary directory")
}

// Saved in an order the listing does not keep, so a test asserting the rows
// cannot pass by accident of the order they were written in.
fn a_home() -> tempfile::TempDir {
    let home = a_dir();
    for name in ["work", "acme", "personal"] {
        save_key(home.path(), name, SECRET).expect("a key stores");
    }
    home
}

// The names the store lists, which is the order the rows are in: the store is a
// `BTreeMap`, so it answers sorted whatever order the keys were added in.
const NAMES: [&str; 3] = ["acme", "personal", "work"];

fn opened(home: &Path) -> KeyPrompt {
    KeyPrompt::open(home, SCOPE).expect("a store that reads opens a window")
}

fn choice(home: &Path) -> KeyChoice {
    let prompt = opened(home);
    prompt
        .choice()
        .expect("an open prompt holds its choice")
        .clone()
}

fn after_key(key: KeyEvent, choice: &KeyChoice) -> KeyChoice {
    match key_edit_for(key, choice) {
        KeyEdited::Open(next) => next,
        other => panic!("{key:?} should have left the key prompt open, and gave {other:?}"),
    }
}

#[test]
fn a_fresh_key_prompt_is_up_over_the_scope_with_every_stored_name() {
    let home = a_home();
    let prompt = opened(home.path());

    assert!(prompt.is_open());
    assert_eq!(prompt, KeyPrompt::Open(choice(home.path())));

    let choice = choice(home.path());
    assert_eq!(choice.scope(), SCOPE);
    assert_eq!(choice.names(), NAMES);
    assert_eq!(
        choice.highlighted(),
        Some("acme"),
        "the first row opens lit, so one Enter binds something"
    );
}

#[test]
fn a_closed_key_prompt_is_the_default_and_holds_no_choice() {
    assert_eq!(KeyPrompt::default(), KeyPrompt::Closed);
    assert!(!KeyPrompt::Closed.is_open());
    assert_eq!(KeyPrompt::Closed.choice(), None);
}

#[test]
fn the_highlight_walks_the_rows_and_stops_at_both_ends() {
    let home = a_home();
    let top = choice(home.path());

    // One press past the top is a key that changes nothing, as Backspace on an
    // empty field is.
    assert_eq!(
        key_edit_for(press(KeyCode::Up), &top),
        KeyEdited::Open(top.clone()),
        "Up on the first row has nowhere to go"
    );

    let mut current = top.clone();
    for expected in ["personal", "work"] {
        current = after_key(press(KeyCode::Down), &current);
        assert_eq!(current.highlighted(), Some(expected));
    }

    // Stopped rather than wrapped: a Down that jumped back to the top would bind
    // whichever key is first to the next Enter.
    assert_eq!(
        key_edit_for(press(KeyCode::Down), &current),
        KeyEdited::Open(current.clone()),
        "Down on the last row has nowhere to go"
    );

    for expected in ["personal", "acme"] {
        current = after_key(press(KeyCode::Up), &current);
        assert_eq!(current.highlighted(), Some(expected));
    }
    assert_eq!(
        current, top,
        "walked down the list and back to where it was"
    );
}

#[test]
fn no_key_ever_changes_the_scope_or_the_rows() {
    // Both are settled when the window opens: the binding written from here has
    // to be for the scope it named, out of the keys this machine really holds.
    let home = a_home();
    let mut current = choice(home.path());

    for code in BINDINGS.into_iter().chain([KeyCode::Backspace]) {
        current = after_key(press(code), &current);

        assert_eq!(current.scope(), SCOPE);
        assert_eq!(current.names(), NAMES);
    }
}

#[test]
fn enter_submits_and_the_pick_is_the_row_that_is_lit() {
    let home = a_home();
    let first = choice(home.path());

    assert_eq!(
        key_edit_for(press(KeyCode::Enter), &first),
        KeyEdited::Submit
    );

    // The submit carries nothing, so the caller reads the name off the choice it
    // already holds — which is the only copy of the pick there is.
    let moved = after_key(press(KeyCode::Down), &first);
    assert_eq!(moved.highlighted(), Some("personal"));
    assert_eq!(
        key_edit_for(press(KeyCode::Enter), &moved),
        KeyEdited::Submit
    );
}

#[test]
fn esc_closes_from_any_row() {
    let home = a_home();
    let mut current = choice(home.path());

    for _ in NAMES {
        assert_eq!(
            key_edit_for(press(KeyCode::Esc), &current),
            KeyEdited::Close
        );
        current = after_key(press(KeyCode::Down), &current);
    }
}

#[test]
fn nothing_but_enter_submits_and_nothing_but_esc_closes_in_the_key_prompt() {
    let home = a_home();
    let lit = choice(home.path());

    for code in BINDINGS.into_iter().chain([
        KeyCode::Backspace,
        KeyCode::Delete,
        KeyCode::BackTab,
        KeyCode::Left,
        KeyCode::Right,
        KeyCode::Home,
        KeyCode::End,
        KeyCode::Insert,
    ]) {
        let outcome = key_edit_for(press(code), &lit);

        assert_ne!(outcome, KeyEdited::Submit, "{code:?} should not submit");
        assert_ne!(outcome, KeyEdited::Close, "{code:?} should not close");
        if !matches!(code, KeyCode::Up | KeyCode::Down) {
            assert_eq!(
                outcome,
                KeyEdited::Open(lit.clone()),
                "{code:?} is not a key this window has"
            );
        }
    }
}

#[test]
fn releases_and_repeats_change_nothing_in_the_key_prompt() {
    let home = a_home();
    let before = after_key(press(KeyCode::Down), &choice(home.path()));

    for code in [
        KeyCode::Up,
        KeyCode::Down,
        KeyCode::Enter,
        KeyCode::Esc,
        KeyCode::Char('k'),
    ] {
        for kind in [KeyEventKind::Release, KeyEventKind::Repeat] {
            let event = KeyEvent::new_with_kind_and_state(
                code,
                KeyModifiers::NONE,
                kind,
                KeyEventState::NONE,
            );

            assert_eq!(
                key_edit_for(event, &before),
                KeyEdited::Open(before.clone()),
                "{kind:?} of {code:?} should do nothing"
            );
        }
    }
}

#[test]
fn a_machine_with_no_store_opens_a_window_with_no_rows() {
    let home = a_dir();
    assert!(
        !keys_path(home.path()).exists(),
        "nothing has run `warlock key add` here"
    );

    let empty = choice(home.path());
    assert!(empty.names().is_empty());
    assert_eq!(empty.scope(), SCOPE);
    assert_eq!(
        empty.highlighted(),
        None,
        "no rows is the one state with nothing lit"
    );

    // No row to move onto and no name to submit, so all three are keys that
    // change nothing. Esc still closes: a window a reader cannot leave is a trap.
    for code in [KeyCode::Up, KeyCode::Down, KeyCode::Enter] {
        assert_eq!(
            key_edit_for(press(code), &empty),
            KeyEdited::Open(empty.clone()),
            "{code:?} over an empty window should change nothing"
        );
    }
    assert_eq!(key_edit_for(press(KeyCode::Esc), &empty), KeyEdited::Close);
}

#[test]
fn a_store_that_will_not_parse_is_the_engines_error_and_not_an_empty_window() {
    // The two look identical on screen and mean opposite things: drawn as an
    // empty list, a store warlock could not read would tell somebody they hold
    // no keys while their keys sit in the file.
    let home = a_dir();
    let path = keys_path(home.path());
    fs::create_dir_all(path.parent().expect("the store is in a directory")).expect("creates");
    fs::write(&path, "this is not toml\n").expect("writes");

    let error = KeyPrompt::open(home.path(), SCOPE).expect_err("a broken store");
    assert!(matches!(error, keys::Error::Syntax { .. }), "{error:?}");
}

#[test]
fn the_window_holds_names_and_never_a_key_value() {
    let home = a_home();
    let choice = choice(home.path());

    assert_eq!(
        load_key(home.path(), "work")
            .expect("the store reads")
            .as_deref(),
        Some(SECRET),
        "the store these names came from really does hold a value under each"
    );

    // Every field at once, through `Debug`: a value in any of them — including
    // one a later edit added — would print here.
    let held = format!("{choice:?}");
    assert!(
        !held.contains(SECRET),
        "a key value reached the window: {held}"
    );
    for name in NAMES {
        assert!(held.contains(name), "the names are what it holds: {held}");
    }

    let moved = after_key(press(KeyCode::Down), &choice);
    assert!(
        !format!("{moved:?}").contains(SECRET),
        "moving the highlight read no value"
    );
}
