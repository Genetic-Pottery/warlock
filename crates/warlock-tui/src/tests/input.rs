use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};

use super::{Action, action_for};

fn press(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

#[test]
fn q_and_esc_quit_with_no_pact_running() {
    assert_eq!(
        action_for(press(KeyCode::Char('q')), false),
        Some(Action::Quit)
    );
    assert_eq!(action_for(press(KeyCode::Esc), false), Some(Action::Quit));
}

#[test]
fn esc_cancels_the_pact_in_flight_while_q_and_ctrl_c_still_quit() {
    let ctrl_c = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);

    assert_eq!(
        action_for(press(KeyCode::Esc), true),
        Some(Action::CancelPact),
        "Esc during a pact stops the pact, not warlock"
    );
    assert_eq!(
        action_for(press(KeyCode::Char('q')), true),
        Some(Action::Quit),
        "and the ways out are still the ways out"
    );
    assert_eq!(action_for(ctrl_c, true), Some(Action::Quit));
}

#[test]
fn esc_is_the_only_key_a_pact_in_flight_changes_the_meaning_of() {
    // Everything else the tree answers to keeps working while a run works,
    // which is the point of running it on a thread at all.
    let codes = [
        KeyCode::Char('q'),
        KeyCode::Up,
        KeyCode::Down,
        KeyCode::Char('k'),
        KeyCode::Char('j'),
        KeyCode::PageUp,
        KeyCode::PageDown,
        KeyCode::Char('g'),
        KeyCode::Char('G'),
        KeyCode::Char(' '),
        KeyCode::Char('o'),
        KeyCode::Char('f'),
        KeyCode::Char('p'),
        KeyCode::Char('r'),
        KeyCode::Char('s'),
        KeyCode::Char('v'),
        KeyCode::Char('e'),
        KeyCode::Tab,
        KeyCode::BackTab,
        KeyCode::Char('x'),
    ];

    for code in codes {
        assert_eq!(
            action_for(press(code), true),
            action_for(press(code), false),
            "{code:?} means something different mid-pact"
        );
    }
}

#[test]
fn ctrl_c_quits_but_a_bare_c_does_not() {
    let ctrl_c = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);

    assert_eq!(action_for(ctrl_c, false), Some(Action::Quit));
    assert_eq!(action_for(press(KeyCode::Char('c')), false), None);
}

#[test]
fn ctrl_c_quits_with_caps_lock_or_shift_held() {
    // Some terminals report Ctrl-C as an upper-case `C` when shift or caps
    // lock is in play; it is still the key everyone reaches for to get out.
    let ctrl_shift_c = KeyEvent::new(
        KeyCode::Char('C'),
        KeyModifiers::CONTROL | KeyModifiers::SHIFT,
    );

    assert_eq!(action_for(ctrl_shift_c, false), Some(Action::Quit));
}

#[test]
fn tab_moves_the_keys_to_the_other_pane() {
    assert_eq!(
        action_for(press(KeyCode::Tab), false),
        Some(Action::ToggleFocus)
    );
}

#[test]
fn tab_means_the_same_thing_during_a_pact() {
    // Esc is the one key a run in flight re-reads, and focus is nothing to
    // do with a run: the tree stays drivable while a pact works
    // (WAR-21.05), so the key that says which pane is being driven has to
    // work then too.
    assert_eq!(
        action_for(press(KeyCode::Tab), true),
        Some(Action::ToggleFocus)
    );
}

#[test]
fn releases_and_repeats_of_tab_move_no_focus() {
    // The same rule as every other key, and with the same consequence: a
    // release acted on would put focus straight back where the press took
    // it from, so one keystroke would look like none at all.
    for kind in [KeyEventKind::Release, KeyEventKind::Repeat] {
        let event = KeyEvent::new_with_kind_and_state(
            KeyCode::Tab,
            KeyModifiers::NONE,
            kind,
            KeyEventState::NONE,
        );

        assert_eq!(
            action_for(event, false),
            None,
            "{kind:?} of Tab should not move focus"
        );
    }
}

#[test]
fn tab_is_the_only_key_that_moves_focus() {
    // Its neighbours on the keyboard and the keys it sits between in the
    // match arms above, plus the back-tab a terminal sends for Shift-Tab,
    // which is a keystroke of its own: it swaps the panel's card, and moving
    // focus is the one thing it must not be confused with.
    for code in [
        KeyCode::BackTab,
        KeyCode::Esc,
        KeyCode::Char('q'),
        KeyCode::Char(' '),
        KeyCode::Enter,
        KeyCode::Up,
        KeyCode::Char('p'),
    ] {
        assert_ne!(
            action_for(press(code), false),
            Some(Action::ToggleFocus),
            "{code:?} should not move focus"
        );
        assert_ne!(
            action_for(press(code), true),
            Some(Action::ToggleFocus),
            "{code:?} should not move focus mid-pact"
        );
    }
}

#[test]
fn up_and_k_move_the_selection_up() {
    assert_eq!(
        action_for(press(KeyCode::Up), false),
        Some(Action::SelectPrevious)
    );
    assert_eq!(
        action_for(press(KeyCode::Char('k')), false),
        Some(Action::SelectPrevious)
    );
}

#[test]
fn down_and_j_move_the_selection_down() {
    assert_eq!(
        action_for(press(KeyCode::Down), false),
        Some(Action::SelectNext)
    );
    assert_eq!(
        action_for(press(KeyCode::Char('j')), false),
        Some(Action::SelectNext)
    );
}

#[test]
fn page_up_and_page_down_move_the_selection_by_a_screenful() {
    assert_eq!(
        action_for(press(KeyCode::PageUp), false),
        Some(Action::SelectPageUp)
    );
    assert_eq!(
        action_for(press(KeyCode::PageDown), false),
        Some(Action::SelectPageDown)
    );
}

#[test]
fn lower_g_jumps_to_the_first_row_and_upper_g_to_the_last() {
    assert_eq!(
        action_for(press(KeyCode::Char('g')), false),
        Some(Action::SelectFirst)
    );
    assert_eq!(
        action_for(press(KeyCode::Char('G')), false),
        Some(Action::SelectLast)
    );
}

#[test]
fn upper_g_still_jumps_to_the_last_row_with_shift_reported() {
    // Terminals disagree about whether the modifier rides along with the
    // upper-case letter; both spellings are the same keystroke.
    let shift_g = KeyEvent::new(KeyCode::Char('G'), KeyModifiers::SHIFT);

    assert_eq!(action_for(shift_g, false), Some(Action::SelectLast));
}

#[test]
fn releases_and_repeats_of_the_new_movement_keys_move_nothing() {
    let codes = [
        KeyCode::PageUp,
        KeyCode::PageDown,
        KeyCode::Char('g'),
        KeyCode::Char('G'),
    ];

    for code in codes {
        for kind in [KeyEventKind::Release, KeyEventKind::Repeat] {
            let event = KeyEvent::new_with_kind_and_state(
                code,
                KeyModifiers::NONE,
                kind,
                KeyEventState::NONE,
            );

            assert_eq!(
                action_for(event, false),
                None,
                "{kind:?} of {code:?} should not move anything"
            );
        }
    }
}

#[test]
fn space_toggles_the_collapse_of_the_selected_directory() {
    assert_eq!(
        action_for(press(KeyCode::Char(' ')), false),
        Some(Action::ToggleCollapsed)
    );
}

#[test]
fn releases_and_repeats_of_space_collapse_nothing() {
    // The same rule as every other key: a release acted on would expand
    // again what the press had just collapsed, so one keystroke would look
    // like none at all.
    for kind in [KeyEventKind::Release, KeyEventKind::Repeat] {
        let event = KeyEvent::new_with_kind_and_state(
            KeyCode::Char(' '),
            KeyModifiers::NONE,
            kind,
            KeyEventState::NONE,
        );

        assert_eq!(
            action_for(event, false),
            None,
            "{kind:?} of space should not collapse anything"
        );
    }
}

#[test]
fn space_is_the_only_key_that_collapses() {
    // Neighbours on the keyboard and in the match arms above, in case a
    // space ever gets typed into the wrong pattern.
    for code in [
        KeyCode::Enter,
        KeyCode::Tab,
        KeyCode::Char('s'),
        KeyCode::Char('p'),
        KeyCode::Char('g'),
    ] {
        assert_ne!(
            action_for(press(code), false),
            Some(Action::ToggleCollapsed),
            "{code:?} should not collapse anything"
        );
    }
}

#[test]
fn o_toggles_the_pacted_only_filter() {
    assert_eq!(
        action_for(press(KeyCode::Char('o')), false),
        Some(Action::TogglePactedOnly)
    );
}

#[test]
fn releases_and_repeats_of_o_filter_nothing() {
    // The same rule as space: a release acted on would restore the whole
    // tree the press had just narrowed, so one keystroke would look like
    // none at all.
    for kind in [KeyEventKind::Release, KeyEventKind::Repeat] {
        let event = KeyEvent::new_with_kind_and_state(
            KeyCode::Char('o'),
            KeyModifiers::NONE,
            kind,
            KeyEventState::NONE,
        );

        assert_eq!(
            action_for(event, false),
            None,
            "{kind:?} of o should not filter anything"
        );
    }
}

#[test]
fn o_is_the_only_key_that_filters() {
    // Its neighbours on the keyboard, the key it sits next to in the match
    // arms above, and its upper-case self, which this binding does not
    // answer to.
    for code in [
        KeyCode::Char('i'),
        KeyCode::Char('p'),
        KeyCode::Char('k'),
        KeyCode::Char('l'),
        KeyCode::Char('O'),
        KeyCode::Char('r'),
        KeyCode::Char(' '),
    ] {
        assert_ne!(
            action_for(press(code), false),
            Some(Action::TogglePactedOnly),
            "{code:?} should not filter anything"
        );
    }
}

#[test]
fn f_toggles_the_files_inside_each_directory() {
    assert_eq!(
        action_for(press(KeyCode::Char('f')), false),
        Some(Action::ToggleFiles)
    );
}

#[test]
fn releases_and_repeats_of_f_show_nothing() {
    // The same rule as space and `o`: a release acted on would hide again
    // the files the press had just shown, so one keystroke would look like
    // none at all.
    for kind in [KeyEventKind::Release, KeyEventKind::Repeat] {
        let event = KeyEvent::new_with_kind_and_state(
            KeyCode::Char('f'),
            KeyModifiers::NONE,
            kind,
            KeyEventState::NONE,
        );

        assert_eq!(
            action_for(event, false),
            None,
            "{kind:?} of f should not show anything"
        );
    }
}

#[test]
fn f_is_the_only_key_that_shows_files() {
    // Its neighbours on the keyboard, the keys it sits between in the match
    // arms above, and its upper-case self, which this binding does not
    // answer to.
    for code in [
        KeyCode::Char('d'),
        KeyCode::Char('g'),
        KeyCode::Char('r'),
        KeyCode::Char('o'),
        KeyCode::Char('p'),
        KeyCode::Char('F'),
        KeyCode::Char(' '),
    ] {
        assert_ne!(
            action_for(press(code), false),
            Some(Action::ToggleFiles),
            "{code:?} should not show any files"
        );
    }
}

#[test]
fn p_toggles_the_pact_on_the_selected_node() {
    assert_eq!(
        action_for(press(KeyCode::Char('p')), false),
        Some(Action::TogglePact)
    );
}

#[test]
fn releases_and_repeats_of_p_write_nothing() {
    // The same rule as for movement, and it matters more here: a release
    // acted on would undo the pact the press had just written, and a held
    // key would rewrite the manifest as fast as the terminal repeats.
    for kind in [KeyEventKind::Release, KeyEventKind::Repeat] {
        let event = KeyEvent::new_with_kind_and_state(
            KeyCode::Char('p'),
            KeyModifiers::NONE,
            kind,
            KeyEventState::NONE,
        );

        assert_eq!(
            action_for(event, false),
            None,
            "{kind:?} should not write anything"
        );
    }
}

#[test]
fn r_asks_for_a_refresh_with_a_run_in_flight_or_without_one() {
    // Like every key but Esc, `r` means one thing in both situations: what
    // a refresh does about a run already working is the app's answer to
    // give, and a second `p` is refused in exactly the same place.
    for in_flight in [false, true] {
        assert_eq!(
            action_for(press(KeyCode::Char('r')), in_flight),
            Some(Action::Refresh),
            "r should ask for a refresh with a run in flight = {in_flight}"
        );
    }
}

#[test]
fn upper_r_asks_for_nothing() {
    // Lower case only, like `o`, `f`, `p` and `m`: the upper-case letter is
    // a different keystroke, and leaving it unbound keeps it free for a
    // later one.
    for in_flight in [false, true] {
        assert_eq!(action_for(press(KeyCode::Char('R')), in_flight), None);
    }
}

#[test]
fn releases_and_repeats_of_r_start_nothing() {
    // The same rule as `p`, and it matters for the same reason: a release
    // acted on would ask for a second run on the heels of the one the press
    // started, and a held key would ask as fast as the terminal repeats.
    for kind in [KeyEventKind::Release, KeyEventKind::Repeat] {
        let event = KeyEvent::new_with_kind_and_state(
            KeyCode::Char('r'),
            KeyModifiers::NONE,
            kind,
            KeyEventState::NONE,
        );

        assert_eq!(
            action_for(event, false),
            None,
            "{kind:?} of r should not start anything"
        );
    }
}

#[test]
fn r_is_the_only_key_that_refreshes() {
    // Its neighbours on the keyboard, the keys it sits beside in the match
    // arms above, and its upper-case self, which this binding does not
    // answer to.
    for code in [
        KeyCode::Char('e'),
        KeyCode::Char('t'),
        KeyCode::Char('f'),
        KeyCode::Char('p'),
        KeyCode::Char('m'),
        KeyCode::Char('R'),
        KeyCode::Char(' '),
        KeyCode::Enter,
    ] {
        assert_ne!(
            action_for(press(code), false),
            Some(Action::Refresh),
            "{code:?} should not refresh anything"
        );
        assert_ne!(
            action_for(press(code), true),
            Some(Action::Refresh),
            "{code:?} should not refresh anything mid-run"
        );
    }
}

#[test]
fn s_asks_for_the_scope_prompt_with_a_run_in_flight_or_without_one() {
    // Like `p` and `r`, and like every key but Esc, `s` means one thing in
    // both situations: a run in flight is a reason to refuse the prompt,
    // and refusing is the loop's answer to give rather than this
    // function's.
    for in_flight in [false, true] {
        assert_eq!(
            action_for(press(KeyCode::Char('s')), in_flight),
            Some(Action::OpenScope),
            "s should ask for the prompt with a run in flight = {in_flight}"
        );
    }
}

#[test]
fn upper_s_asks_for_nothing() {
    // Lower case only, like `o`, `f`, `p`, `r` and `m`: the upper-case
    // letter is a different keystroke, and leaving it unbound keeps it free
    // for a later one.
    for in_flight in [false, true] {
        assert_eq!(action_for(press(KeyCode::Char('S')), in_flight), None);
    }
}

#[test]
fn releases_and_repeats_of_s_open_nothing() {
    // The same rule as `p` and `r`, and here it decides whether the prompt
    // can be typed into at all: acting on a release would reopen the prompt
    // on the release of the very key that opened it, and a held `s` would
    // reopen it — losing whatever had been typed — as fast as the terminal
    // repeats.
    for kind in [KeyEventKind::Release, KeyEventKind::Repeat] {
        let event = KeyEvent::new_with_kind_and_state(
            KeyCode::Char('s'),
            KeyModifiers::NONE,
            kind,
            KeyEventState::NONE,
        );

        assert_eq!(
            action_for(event, false),
            None,
            "{kind:?} of s should open nothing"
        );
    }
}

#[test]
fn s_is_the_only_key_that_scopes() {
    // Its neighbours on the keyboard, the keys it sits between in the match
    // arms above, and its upper-case self, which this binding does not
    // answer to.
    for code in [
        KeyCode::Char('a'),
        KeyCode::Char('d'),
        KeyCode::Char('w'),
        KeyCode::Char('p'),
        KeyCode::Char('r'),
        KeyCode::Char('m'),
        KeyCode::Char('S'),
        KeyCode::Char(' '),
        KeyCode::Enter,
    ] {
        assert_ne!(
            action_for(press(code), false),
            Some(Action::OpenScope),
            "{code:?} should not ask for a scope"
        );
        assert_ne!(
            action_for(press(code), true),
            Some(Action::OpenScope),
            "{code:?} should not ask for a scope mid-run"
        );
    }
}

#[test]
fn v_asks_to_read_the_selected_file_with_a_run_in_flight_or_without_one() {
    // Like `p`, `r` and `s`, and like every key but Esc, `v` means one
    // thing in both situations — and here the mode has nothing it could
    // change even in principle: a read is not a run, so there is no second
    // run for it to be refused as.
    for in_flight in [false, true] {
        assert_eq!(
            action_for(press(KeyCode::Char('v')), in_flight),
            Some(Action::ViewFile),
            "v should ask for the file with a run in flight = {in_flight}"
        );
    }
}

#[test]
fn upper_v_asks_for_nothing() {
    // Lower case only, like `o`, `f`, `p`, `r`, `s` and `m`: the upper-case
    // letter is a different keystroke, and leaving it unbound keeps it free
    // for a later one.
    for in_flight in [false, true] {
        assert_eq!(action_for(press(KeyCode::Char('V')), in_flight), None);
    }
}

#[test]
fn releases_and_repeats_of_v_read_nothing() {
    // The same rule as the keys above. Nothing is written by this one, so a
    // stray read costs no manifest — but a held `v` would re-read the file
    // from disk as fast as the terminal repeats, and throw the panel's
    // window back to the top of it every time.
    for kind in [KeyEventKind::Release, KeyEventKind::Repeat] {
        let event = KeyEvent::new_with_kind_and_state(
            KeyCode::Char('v'),
            KeyModifiers::NONE,
            kind,
            KeyEventState::NONE,
        );

        assert_eq!(
            action_for(event, false),
            None,
            "{kind:?} of v should read nothing"
        );
    }
}

#[test]
fn v_is_the_only_key_that_reads_a_file() {
    // Its neighbours on the keyboard, the keys it sits between in the match
    // arms above, and its upper-case self, which this binding does not
    // answer to.
    for code in [
        KeyCode::Char('c'),
        KeyCode::Char('b'),
        KeyCode::Char('p'),
        KeyCode::Char('r'),
        KeyCode::Char('s'),
        KeyCode::Char('m'),
        KeyCode::Char('V'),
        KeyCode::Char(' '),
        KeyCode::Enter,
    ] {
        assert_ne!(
            action_for(press(code), false),
            Some(Action::ViewFile),
            "{code:?} should not read a file"
        );
        assert_ne!(
            action_for(press(code), true),
            Some(Action::ViewFile),
            "{code:?} should not read a file mid-run"
        );
    }
}

#[test]
fn e_asks_to_edit_the_selected_file_with_a_run_in_flight_or_without_one() {
    // Like `p`, `r`, `s` and `v`, and like every key but Esc, `e` means one
    // thing in both situations. A run in flight is a reason to refuse the
    // editor — the terminal cannot be handed to a child while a pass is
    // still drawing on it — but refusing is the loop's answer to give, in
    // the same place a second `p` is refused, and not this function's.
    for in_flight in [false, true] {
        assert_eq!(
            action_for(press(KeyCode::Char('e')), in_flight),
            Some(Action::EditFile),
            "e should ask for the editor with a run in flight = {in_flight}"
        );
    }
}

#[test]
fn upper_e_asks_for_nothing() {
    // Lower case only, like `o`, `f`, `p`, `r`, `s`, `v` and `m`: the
    // upper-case letter is a different keystroke, and leaving it unbound
    // keeps it free for a later one.
    for in_flight in [false, true] {
        assert_eq!(action_for(press(KeyCode::Char('E')), in_flight), None);
    }
}

#[test]
fn releases_and_repeats_of_e_start_nothing() {
    // The same rule as the keys above, and it matters here as much as it
    // does for `p`: a release acted on would hand the terminal to a second
    // editor the moment the first one was asked for, and a held `e` would
    // suspend warlock as fast as the terminal repeats.
    for kind in [KeyEventKind::Release, KeyEventKind::Repeat] {
        let event = KeyEvent::new_with_kind_and_state(
            KeyCode::Char('e'),
            KeyModifiers::NONE,
            kind,
            KeyEventState::NONE,
        );

        assert_eq!(
            action_for(event, false),
            None,
            "{kind:?} of e should start nothing"
        );
    }
}

#[test]
fn e_is_the_only_key_that_edits_a_file() {
    // Its neighbours on the keyboard, the keys it sits beside in the match
    // arms above — `v` first, since viewing a file and editing one are the
    // two halves this binding must not blur — and its upper-case self,
    // which this binding does not answer to.
    for code in [
        KeyCode::Char('v'),
        KeyCode::Char('w'),
        KeyCode::Char('r'),
        KeyCode::Char('p'),
        KeyCode::Char('s'),
        KeyCode::Char('m'),
        KeyCode::Char('E'),
        KeyCode::Char(' '),
        KeyCode::Enter,
    ] {
        assert_ne!(
            action_for(press(code), false),
            Some(Action::EditFile),
            "{code:?} should not edit a file"
        );
        assert_ne!(
            action_for(press(code), true),
            Some(Action::EditFile),
            "{code:?} should not edit a file mid-run"
        );
    }
}

#[test]
fn shift_tab_swaps_the_panel_card_with_a_run_in_flight_or_without_one() {
    // Crossterm spells Shift-Tab `BackTab`, and like every key but Esc it
    // means one thing in both situations — here there is nothing the mode
    // could change even in principle: both cards are already in the app, so
    // a swap races nothing and there is no run for it to be refused as. A
    // run that could take a document off the screen is the whole thing this
    // binding exists to prevent.
    for in_flight in [false, true] {
        assert_eq!(
            action_for(press(KeyCode::BackTab), in_flight),
            Some(Action::SwapCard),
            "Shift-Tab should swap the card with a run in flight = {in_flight}"
        );
    }
}

#[test]
fn releases_and_repeats_of_shift_tab_swap_nothing() {
    // The same rule as Tab, and with the same consequence: a release acted
    // on would swap straight back to the card the press had just left, so
    // one keystroke would look like none at all.
    for kind in [KeyEventKind::Release, KeyEventKind::Repeat] {
        let event = KeyEvent::new_with_kind_and_state(
            KeyCode::BackTab,
            KeyModifiers::NONE,
            kind,
            KeyEventState::NONE,
        );

        assert_eq!(
            action_for(event, false),
            None,
            "{kind:?} of Shift-Tab should swap nothing"
        );
    }
}

#[test]
fn shift_tab_is_the_only_key_that_swaps_the_card() {
    // Tab first, because the two are one shift apart and a terminal that
    // reported the modifier on an ordinary `Tab` is the accident worth
    // catching; then the keys it sits between in the match arms above and
    // `v`, which is the other key that decides what the panel shows.
    for code in [
        KeyCode::Tab,
        KeyCode::Esc,
        KeyCode::Enter,
        KeyCode::Char('v'),
        KeyCode::Char(' '),
    ] {
        assert_ne!(
            action_for(press(code), false),
            Some(Action::SwapCard),
            "{code:?} should not swap the panel's card"
        );
        assert_ne!(
            action_for(press(code), true),
            Some(Action::SwapCard),
            "{code:?} should not swap the panel's card mid-run"
        );
    }
}

#[test]
fn m_toggles_the_mouse_with_a_pact_in_flight_or_without_one() {
    // The one key here that is about the terminal rather than the tree, and
    // it reads the same way in both situations — like everything but Esc.
    // Mid-run is in fact when a reader most wants it: the panel is filling
    // up with output worth copying, and copying it means handing the pointer
    // back to the terminal for a moment.
    for in_flight in [false, true] {
        assert_eq!(
            action_for(press(KeyCode::Char('m')), in_flight),
            Some(Action::ToggleMouseCapture),
            "m should toggle capture with a pact in flight = {in_flight}"
        );
    }
}

#[test]
fn the_mouse_key_neither_quits_nor_moves_anything() {
    // Said against every other action by name, because what the key must not
    // do is the interesting half of it: it does not leave, it does not stop a
    // run, it does not move the keys to the other pane and it does not touch
    // a row. One variant is all it can come to, and the list below is the
    // rest of them.
    for in_flight in [false, true] {
        let action = action_for(press(KeyCode::Char('m')), in_flight);
        for other in [
            Action::Quit,
            Action::CancelPact,
            Action::ToggleFocus,
            Action::SelectPrevious,
            Action::SelectNext,
            Action::SelectPageUp,
            Action::SelectPageDown,
            Action::SelectFirst,
            Action::SelectLast,
            Action::ToggleCollapsed,
            Action::TogglePactedOnly,
            Action::ToggleFiles,
            Action::TogglePact,
            Action::Refresh,
            Action::OpenScope,
            Action::ViewFile,
            Action::EditFile,
            Action::SwapCard,
        ] {
            assert_ne!(action, Some(other), "m should not mean {other:?}");
        }
    }
}

#[test]
fn m_is_the_only_key_that_touches_the_mouse() {
    // Its neighbours in the match arms above, the letter beside it on the
    // keyboard, and its upper-case self, which this binding does not answer
    // to any more than `o`, `f` and `p` answer to theirs.
    for code in [
        KeyCode::Char('n'),
        KeyCode::Char('o'),
        KeyCode::Char('f'),
        KeyCode::Char('p'),
        KeyCode::Char('r'),
        KeyCode::Char('M'),
        KeyCode::Char(' '),
        KeyCode::Enter,
    ] {
        assert_ne!(
            action_for(press(code), false),
            Some(Action::ToggleMouseCapture),
            "{code:?} should not touch the mouse"
        );
    }
}

#[test]
fn releases_and_repeats_of_m_toggle_nothing() {
    // The same rule as the keys above, and here it is the difference between
    // a working key and none: a release acted on would turn capture straight
    // back on after the press turned it off, and a held `m` would flip the
    // terminal's reporting as fast as it repeats.
    for kind in [KeyEventKind::Release, KeyEventKind::Repeat] {
        let event = KeyEvent::new_with_kind_and_state(
            KeyCode::Char('m'),
            KeyModifiers::NONE,
            kind,
            KeyEventState::NONE,
        );

        assert_eq!(
            action_for(event, false),
            None,
            "{kind:?} of m should not toggle anything"
        );
    }
}

#[test]
fn keys_with_no_meaning_here_are_ignored() {
    assert_eq!(action_for(press(KeyCode::Char('x')), false), None);
    assert_eq!(action_for(press(KeyCode::Enter), false), None);
    assert_eq!(action_for(press(KeyCode::Left), false), None);
}

#[test]
fn releases_and_repeats_are_ignored_so_one_keystroke_moves_one_row() {
    for kind in [KeyEventKind::Release, KeyEventKind::Repeat] {
        let event = KeyEvent::new_with_kind_and_state(
            KeyCode::Down,
            KeyModifiers::NONE,
            kind,
            KeyEventState::NONE,
        );

        assert_eq!(
            action_for(event, false),
            None,
            "{kind:?} should not move anything"
        );
    }
}

mod gate {
    use std::time::Instant;

    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
    use ratatui::layout::Size;
    use warlock_engine::NodeState;
    use warlock_tui::{
        Answer, App, Composed, Composer, Edited, Focus, PushConfirm, QuitConfirm, RecordPrompt,
        Row, ScopeField, ScopePrompt, edit_for, panel_height, push_answer_for, tree_height,
    };

    use super::super::{Action, Pressed, action_for, press_for as every_window};

    // The gate with both of `/push`'s windows down, which is every round in
    // this module bar their own tests: those call `press_for` itself, with one
    // of them up.
    #[expect(
        clippy::too_many_arguments,
        reason = "the gate's own signature, minus the two windows these tests \
                  never have up"
    )]
    fn gate_for(
        key: KeyEvent,
        confirm: QuitConfirm,
        prompt: &ScopePrompt,
        record: &RecordPrompt,
        write: &ScopePrompt,
        composer: Option<&Composer>,
        in_flight: bool,
        answered: bool,
    ) -> Pressed {
        every_window(
            key,
            confirm,
            &PushConfirm::Closed,
            &ScopePrompt::Closed,
            prompt,
            record,
            write,
            composer,
            in_flight,
            answered,
        )
    }

    fn press_for(
        key: KeyEvent,
        confirm: QuitConfirm,
        prompt: &ScopePrompt,
        composer: Option<&Composer>,
        in_flight: bool,
        answered: bool,
    ) -> Pressed {
        gate_for(
            key,
            confirm,
            prompt,
            &RecordPrompt::Closed,
            &ScopePrompt::Closed,
            composer,
            in_flight,
            answered,
        )
    }

    const SIZE: Size = Size {
        width: 80,
        height: 24,
    };

    const DIRECTORY: &str = "crates/warlock-engine";

    const INERT: [KeyCode; 19] = [
        KeyCode::Char('j'),
        KeyCode::Char('k'),
        KeyCode::Char('g'),
        KeyCode::Char('G'),
        KeyCode::Char(' '),
        KeyCode::Char('o'),
        KeyCode::Char('f'),
        KeyCode::Char('p'),
        KeyCode::Char('r'),
        KeyCode::Char('s'),
        KeyCode::Char('v'),
        KeyCode::Char('m'),
        KeyCode::Tab,
        KeyCode::BackTab,
        KeyCode::PageUp,
        KeyCode::PageDown,
        KeyCode::Up,
        KeyCode::Down,
        KeyCode::Char('x'),
    ];

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Round {
        Stayed,
        Left,
    }

    fn press(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn ctrl_c() -> KeyEvent {
        KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)
    }

    fn app_on_screen() -> App {
        let mut rows = vec![
            Row::new(0, "/repo", "/repo/WARLOCK.md", NodeState::PactedStale).with_child_count(12),
        ];
        for n in 0..12 {
            let directory = format!("/repo/d{n:02}");
            let state = if n % 2 == 0 {
                NodeState::PactedFresh
            } else {
                NodeState::Unpacted
            };
            rows.push(Row::new(1, directory.clone(), None, state));
            rows.push(Row::file(2, format!("{directory}/lib.rs"), state));
        }
        let mut app = App::from_rows(rows);
        app.set_viewport_height(tree_height(SIZE));
        app.panel_mut().set_height(panel_height(SIZE, None, None));
        app
    }

    fn app_in_use() -> App {
        let mut app = app_on_screen();
        app.toggle_files();
        app.toggle_pacted_only();
        app.select_row(9);
        app.toggle_collapsed();
        app.select_previous();
        // A panel with more in it than its window holds, so that scrolling
        // it back is a real offset rather than a no-op: an app with no
        // account has exactly one place its window can be, and a field that
        // cannot move cannot catch a key that moved it.
        let started = Instant::now();
        app.start_account(started);
        if let Some(account) = app.panel_mut().account_mut() {
            for n in 0..40 {
                account.open_section(format!("/repo/d{n:02}"), started);
            }
        }
        app.scroll_panel_up(5);
        app.set_focus(Focus::Panel);
        app.set_message("something worth keeping");
        app
    }

    fn round(app: &mut App, confirm: &mut QuitConfirm, key: KeyEvent) -> Round {
        round_under(app, confirm, &mut ScopePrompt::Closed, key)
    }

    fn offered<'a>(app: &App, composer: &'a Composer) -> Option<&'a Composer> {
        (app.focus() == Focus::Composer).then_some(composer)
    }

    fn round_under(
        app: &mut App,
        confirm: &mut QuitConfirm,
        prompt: &mut ScopePrompt,
        key: KeyEvent,
    ) -> Round {
        round_composing(app, confirm, prompt, &mut Composer::default(), key)
    }

    fn round_composing(
        app: &mut App,
        confirm: &mut QuitConfirm,
        prompt: &mut ScopePrompt,
        composer: &mut Composer,
        key: KeyEvent,
    ) -> Round {
        // Two falses, and they are the two situations this round is never
        // in: no run in flight — the question cannot be up during one — and
        // no turn being answered, which is what the tests further down that
        // *are* about a turn hand in for themselves.
        round_running(app, confirm, prompt, composer, key, false)
    }

    fn round_running(
        app: &mut App,
        confirm: &mut QuitConfirm,
        prompt: &mut ScopePrompt,
        composer: &mut Composer,
        key: KeyEvent,
        in_flight: bool,
    ) -> Round {
        match press_for(
            key,
            *confirm,
            prompt,
            offered(app, composer),
            in_flight,
            false,
        ) {
            Pressed::Leave | Pressed::Act(Action::Quit) => return Round::Left,
            // Never reached from a round with nothing being answered, and
            // panicking rather than leaving quietly for the reason the four
            // arms below do: a cancel conjured out of a session with no
            // question out is the gate reading a situation nobody is in.
            Pressed::CancelTurn => panic!("a turn was cancelled with none in flight"),
            Pressed::Confirm(next) => *confirm = next,
            Pressed::Scope(Edited::Open(field)) => *prompt = ScopePrompt::Open(field),
            Pressed::Scope(Edited::Close) => *prompt = ScopePrompt::Closed,
            // What a submit comes to is a manifest saved on the loop's own
            // thread, and nothing an app can see: the prompt stays up until
            // the engine has judged the text, which is the next slice's.
            // All this arm can say is where the key came from, and it says
            // it rather than nothing so that a submit conjured out of a
            // closed prompt would be caught here.
            Pressed::Scope(Edited::Submit) => {
                assert!(prompt.is_open(), "a submit came from a prompt that is up");
            }
            // Unreachable through [`press_for`] above, which hands the gate
            // a closed write prompt: the rounds here are about a session
            // with no document waiting to be written. A key that arrived
            // from that window anyway would be the gate answering a question
            // nobody asked, so it is loud rather than quiet — the write
            // prompt's own tests call the gate directly and assert on what
            // comes back.
            Pressed::Write(edited) => {
                panic!("{edited:?} came from a write prompt that is not up")
            }
            // Unreachable for the same reason and loud for the same reason:
            // the gate above is handed a closed record window too, and the
            // record window's own tests call the gate directly.
            Pressed::Record(edited) => {
                panic!("{edited:?} came from a record window that is not up")
            }
            // And once more for the push dialog, which `gate_for` above also
            // hands in closed: its own tests call `press_for` itself with it
            // up, so an answer arriving here is a question nobody asked.
            Pressed::Push(answered) => {
                panic!("{answered:?} came from a push dialog that is not up")
            }
            // And once more for the field that dialog can come up behind,
            // which is handed in closed by the same shim.
            Pressed::Filing(edited) => {
                panic!("{edited:?} came from a scope field that is not up")
            }
            // The loop's three composer arms, and the reason the draft is a
            // local here exactly as it is there: nothing about it is ever
            // handed to the app.
            Pressed::Compose(Composed::Typing(next)) => *composer = next,
            Pressed::Compose(Composed::Leave) => app.set_focus(Focus::Panel),
            // Inert, as it is in the loop: this slice has no consumer for a
            // submitted draft, so nothing is started, nothing is written and
            // the footer is told nothing. What is asserted rather than done
            // is where the key came from — a submit conjured out of a blank
            // draft, or out of a composer nobody was pointed at, would be
            // caught here.
            Pressed::Compose(Composed::Submit) => {
                assert_eq!(
                    app.focus(),
                    Focus::Composer,
                    "a submit came from a composer that has the keyboard"
                );
                assert!(
                    composer.is_submittable(),
                    "a submit came from a draft with something in it"
                );
            }
            Pressed::Act(Action::ToggleFocus) => app.toggle_focus(),
            Pressed::Act(Action::SelectPrevious) => app.select_previous(),
            Pressed::Act(Action::SelectNext) => app.select_next(),
            Pressed::Act(Action::SelectPageUp) => app.select_page_up(),
            Pressed::Act(Action::SelectPageDown) => app.select_page_down(),
            Pressed::Act(Action::SelectFirst) => app.select_first(),
            Pressed::Act(Action::SelectLast) => app.select_last(),
            Pressed::Act(Action::ToggleCollapsed) => app.toggle_collapsed(),
            Pressed::Act(Action::TogglePactedOnly) => app.toggle_pacted_only(),
            Pressed::Act(Action::ToggleFiles) => app.toggle_files(),
            // With the plain arms rather than the panicking ones below: a
            // swap is answered by the app between two frames, like a
            // collapse or a filter, and it starts no worker, opens no window
            // and writes nothing to the terminal. What matters here is that
            // it is done at all — a Shift-Tab that leaked past either window
            // would change the card under it, which is exactly what the
            // `app == before` assertions are watching for.
            Pressed::Act(Action::SwapCard) => app.swap_card(),
            Pressed::Act(
                action @ (Action::CancelPact
                | Action::TogglePact
                | Action::Refresh
                | Action::OpenScope
                | Action::ViewFile
                | Action::EditFile
                | Action::ToggleMouseCapture),
            ) => panic!("{action:?} reached the app"),
            Pressed::Nothing => {}
        }
        Round::Stayed
    }

    #[test]
    fn the_app_the_no_answer_is_compared_against_is_off_its_defaults() {
        // The teeth behind every `assert_eq!(app, before)` below. An app
        // sitting on its defaults would compare equal to one a leaked
        // keystroke had put back there, so each of the things the
        // confirmation promises to leave alone is somewhere a stray key
        // would move it away from — and the fixture is asserted rather than
        // assumed, because a later edit that flattened it would leave the
        // tests passing and testing nothing.
        let app = app_in_use();
        let fresh = app_on_screen();

        assert!(app.show_files(), "the file filter is on");
        assert!(app.pacted_only(), "and so is the pacted-only filter");
        assert_ne!(app.selected(), fresh.selected(), "the selection has moved");
        assert_ne!(
            app.panel().scroll_offset(),
            0,
            "the panel's window is off the top"
        );
        assert!(
            !app.panel().follows(),
            "and no longer following the newest line"
        );
        assert_eq!(app.focus(), Focus::Panel, "the panel has the keys");
        assert!(app.message().is_some(), "and there is a line worth keeping");
        assert_ne!(
            app.rows().len(),
            fresh.rows().len(),
            "something is collapsed or filtered out of the list"
        );
    }

    #[test]
    fn esc_and_q_ask_before_they_leave() {
        // The whole ticket in one assertion each: the key that used to end
        // the session now puts a question in front of it, with the safe
        // answer lit.
        for code in [KeyCode::Esc, KeyCode::Char('q')] {
            let mut app = app_in_use();
            let before = app.clone();
            let mut confirm = QuitConfirm::Closed;

            assert_eq!(
                round(&mut app, &mut confirm, press(code)),
                Round::Stayed,
                "{code:?} should not leave on its own"
            );
            assert_eq!(confirm, QuitConfirm::Open(Answer::No));
            assert_eq!(app, before, "opening the question changed nothing");
        }
    }

    #[test]
    fn the_question_swallows_every_key_the_tree_answers_to() {
        // Asserted at both highlight positions, and in both layers: the key
        // comes to a mode and never to an `Action`, and the app behind the
        // dialog is the app that was there before it opened.
        for lit in [Answer::Yes, Answer::No] {
            let mut app = app_in_use();
            let before = app.clone();
            let mut confirm = QuitConfirm::Open(lit);

            for code in INERT {
                assert_eq!(
                    press_for(
                        press(code),
                        QuitConfirm::Open(lit),
                        &ScopePrompt::Closed,
                        None,
                        false,
                        false
                    ),
                    Pressed::Confirm(QuitConfirm::Open(lit)),
                    "{code:?} should reach neither the app nor the way out with {lit:?} lit"
                );
                assert_eq!(round(&mut app, &mut confirm, press(code)), Round::Stayed);
            }

            assert_eq!(
                confirm,
                QuitConfirm::Open(lit),
                "the highlight did not move"
            );
            assert_eq!(app, before, "nothing reached the tree underneath");
        }
    }

    #[test]
    fn answering_yes_leaves_by_the_road_a_quit_already_takes() {
        // Both spellings of Yes, and the same value Ctrl-C comes to: one
        // road out of the loop means one `return Ok(())`, so the terminal
        // guard restores the screen and a running `claude` is taken down by
        // the run's own drop, exactly as before this gate existed.
        for key in [press(KeyCode::Char('y')), press(KeyCode::Enter)] {
            let mut app = app_in_use();
            let mut confirm = QuitConfirm::Open(Answer::Yes);

            assert_eq!(
                press_for(key, confirm, &ScopePrompt::Closed, None, false, false),
                Pressed::Leave
            );
            assert_eq!(
                press_for(key, confirm, &ScopePrompt::Closed, None, false, false),
                press_for(ctrl_c(), confirm, &ScopePrompt::Closed, None, false, false)
            );
            assert_eq!(round(&mut app, &mut confirm, key), Round::Left);
        }
    }

    #[test]
    fn answering_no_closes_the_question_and_leaves_the_app_untouched() {
        // The three ways of saying No — the key, the key that opened the
        // question, and Enter on the answer that is lit when it opens — each
        // with the highlight walked over to Yes and back first, so the app
        // is compared after a handful of keystrokes rather than after one.
        for code in [KeyCode::Char('n'), KeyCode::Esc, KeyCode::Enter] {
            let mut app = app_in_use();
            let before = app.clone();
            let mut confirm = QuitConfirm::Closed;

            assert_eq!(
                round(&mut app, &mut confirm, press(KeyCode::Esc)),
                Round::Stayed
            );
            assert_eq!(
                round(&mut app, &mut confirm, press(KeyCode::Left)),
                Round::Stayed
            );
            assert_eq!(
                round(&mut app, &mut confirm, press(KeyCode::Right)),
                Round::Stayed
            );
            assert_eq!(
                round(&mut app, &mut confirm, press(code)),
                Round::Stayed,
                "{code:?} should answer No"
            );

            assert_eq!(confirm, QuitConfirm::Closed, "the question came down");
            assert_eq!(app, before, "and took nothing with it");
        }
    }

    #[test]
    fn the_reflex_second_esc_closes_the_question_rather_than_the_session() {
        // The accident this gate exists for, spelled out: two presses of the
        // key nearest to hand leave warlock exactly where it was.
        let mut app = app_in_use();
        let before = app.clone();
        let mut confirm = QuitConfirm::Closed;

        for _ in 0..4 {
            assert_eq!(
                round(&mut app, &mut confirm, press(KeyCode::Esc)),
                Round::Stayed
            );
        }

        assert_eq!(confirm, QuitConfirm::Closed, "an even number of presses");
        assert_eq!(app, before);
    }

    #[test]
    fn ctrl_c_leaves_at_once_with_the_question_up_or_down() {
        // Answered before the mode is consulted, which is what keeps it out
        // of `answer_for`'s "every other key" arm: through there it would be
        // an ordinary `c` with a modifier riding along, and the one
        // keystroke every reader trusts would be the one the dialog ate.
        //
        // Pinned with the composer holding the keyboard as well as without
        // it, because the field is the third thing that could have eaten the
        // key: through `compose_for` it is a chord rather than text, i.e.
        // one of the keys that change nothing, so a gate that consulted the
        // draft first would swallow it in silence.
        let draft = Composer::new("web");
        for confirm in [
            QuitConfirm::Closed,
            QuitConfirm::Open(Answer::No),
            QuitConfirm::Open(Answer::Yes),
        ] {
            for composer in [None, Some(&draft)] {
                for in_flight in [false, true] {
                    assert_eq!(
                        press_for(
                            ctrl_c(),
                            confirm,
                            &ScopePrompt::Closed,
                            composer,
                            in_flight,
                            false
                        ),
                        Pressed::Leave,
                        "Ctrl-C should leave with {confirm:?}, {composer:?} and a run in \
                             flight = {in_flight}"
                    );
                }
            }
        }

        let mut app = app_in_use();
        let mut confirm = QuitConfirm::open();
        assert_eq!(round(&mut app, &mut confirm, ctrl_c()), Round::Left);
    }

    #[test]
    fn ctrl_c_stops_the_turn_rather_than_the_session_while_one_is_answered() {
        // The one situation the key does not end the session in, and the
        // whole of what decides it: a question is out. A turn is one
        // `claude` and seconds of waiting, so the reader who wants it back
        // presses this; a turn escapable only by leaving warlock would be a
        // question nobody can take back.
        //
        // The same matrix the answer above is pinned across, because the
        // reading must not depend on any of it: the question up or down, the
        // keyboard in the field or not, a run in flight or not. It is
        // answered before the confirmation for the reason it is answered
        // first at all — a Ctrl-C that meant one thing with a window up and
        // another with it down would be the keystroke of last resort
        // behaving differently depending on what is on screen.
        let draft = Composer::new("web");
        for confirm in [
            QuitConfirm::Closed,
            QuitConfirm::Open(Answer::No),
            QuitConfirm::Open(Answer::Yes),
        ] {
            for composer in [None, Some(&draft)] {
                for in_flight in [false, true] {
                    assert_eq!(
                        press_for(
                            ctrl_c(),
                            confirm,
                            &ScopePrompt::Closed,
                            composer,
                            in_flight,
                            true
                        ),
                        Pressed::CancelTurn,
                        "Ctrl-C should stop the turn with {confirm:?}, {composer:?} and a run \
                             in flight = {in_flight}"
                    );
                }
            }
        }

        // And the scope prompt, the other window it goes round.
        assert_eq!(
            press_for(
                ctrl_c(),
                QuitConfirm::Closed,
                &ScopePrompt::Open(ScopeField::new(DIRECTORY, "web")),
                None,
                false,
                true
            ),
            Pressed::CancelTurn
        );

        // The press after it leaves, because by then nothing is being
        // answered: one key, one meaning — stop what I asked for — and the
        // reader who presses it twice gets the cancel and then the way out.
        assert_eq!(
            press_for(
                ctrl_c(),
                QuitConfirm::Closed,
                &ScopePrompt::Closed,
                None,
                false,
                false
            ),
            Pressed::Leave
        );
    }

    #[test]
    fn a_turn_being_answered_changes_no_other_key_at_all() {
        // Ctrl-C is the only key a turn is allowed to re-read. `q` still
        // asks the question it has always asked and Esc still means what the
        // run in flight says it means, because a turn is not a run: it
        // writes nothing, so there is nothing for the keys that guard a
        // pact to guard.
        for in_flight in [false, true] {
            for code in INERT {
                assert_eq!(
                    press_for(
                        press(code),
                        QuitConfirm::Closed,
                        &ScopePrompt::Closed,
                        None,
                        in_flight,
                        true
                    ),
                    press_for(
                        press(code),
                        QuitConfirm::Closed,
                        &ScopePrompt::Closed,
                        None,
                        in_flight,
                        false
                    ),
                    "{code:?} read differently with a turn being answered"
                );
            }

            for code in [KeyCode::Char('q'), KeyCode::Esc] {
                assert_eq!(
                    press_for(
                        press(code),
                        QuitConfirm::Closed,
                        &ScopePrompt::Closed,
                        None,
                        in_flight,
                        true
                    ),
                    press_for(
                        press(code),
                        QuitConfirm::Closed,
                        &ScopePrompt::Closed,
                        None,
                        in_flight,
                        false
                    ),
                    "{code:?} read differently with a turn being answered"
                );
            }
        }

        // Said plainly as well as by comparison, because it is the promise:
        // `q` with a turn out and nothing running still asks first, and `q`
        // with a run in flight still leaves outright.
        assert_eq!(
            press_for(
                press(KeyCode::Char('q')),
                QuitConfirm::Closed,
                &ScopePrompt::Closed,
                None,
                false,
                true
            ),
            Pressed::Confirm(QuitConfirm::open())
        );
        assert_eq!(
            press_for(
                press(KeyCode::Char('q')),
                QuitConfirm::Closed,
                &ScopePrompt::Closed,
                None,
                true,
                true
            ),
            Pressed::Leave
        );
    }

    #[test]
    fn a_run_in_flight_puts_no_question_in_front_of_anybody() {
        // Esc still cancels the run and `q` still leaves, pinned at both
        // settings of the flag: the gate is for the twitch that follows a
        // cancel, and during a run Esc already means cancel.
        assert_eq!(
            press_for(
                press(KeyCode::Esc),
                QuitConfirm::Closed,
                &ScopePrompt::Closed,
                None,
                true,
                false
            ),
            Pressed::Act(Action::CancelPact),
        );
        assert_eq!(
            press_for(
                press(KeyCode::Char('q')),
                QuitConfirm::Closed,
                &ScopePrompt::Closed,
                None,
                true,
                false
            ),
            Pressed::Leave,
        );

        // And the same two keys with nothing running, which is the only
        // difference the flag makes here.
        assert_eq!(
            press_for(
                press(KeyCode::Esc),
                QuitConfirm::Closed,
                &ScopePrompt::Closed,
                None,
                false,
                false
            ),
            Pressed::Confirm(QuitConfirm::open()),
        );
        assert_eq!(
            press_for(
                press(KeyCode::Char('q')),
                QuitConfirm::Closed,
                &ScopePrompt::Closed,
                None,
                false,
                false
            ),
            Pressed::Confirm(QuitConfirm::open()),
        );
    }

    #[test]
    fn every_other_key_still_means_what_it_always_meant() {
        // The gate is one question in front of two keys and nothing else:
        // with it closed, every binding reaches the app as before, at both
        // settings of the flag.
        for in_flight in [false, true] {
            for code in INERT {
                assert_eq!(
                    press_for(
                        press(code),
                        QuitConfirm::Closed,
                        &ScopePrompt::Closed,
                        None,
                        in_flight,
                        false
                    ),
                    action_for(press(code), in_flight).map_or(Pressed::Nothing, Pressed::Act),
                    "{code:?} should read as it always has, in flight = {in_flight}"
                );
            }
        }
    }

    #[test]
    fn releases_and_repeats_neither_open_the_question_nor_answer_it() {
        // The same rule the two key functions already keep, and here it is
        // the difference between a gate and no gate: acting on a release
        // would answer the question with the release of the very key that
        // opened it.
        for kind in [KeyEventKind::Release, KeyEventKind::Repeat] {
            for code in [KeyCode::Esc, KeyCode::Char('q'), KeyCode::Char('y')] {
                let key = KeyEvent::new_with_kind_and_state(
                    code,
                    KeyModifiers::NONE,
                    kind,
                    KeyEventState::NONE,
                );

                assert_eq!(
                    press_for(
                        key,
                        QuitConfirm::Closed,
                        &ScopePrompt::Closed,
                        None,
                        false,
                        false
                    ),
                    Pressed::Nothing,
                    "{kind:?} of {code:?} should open nothing"
                );
                assert_eq!(
                    press_for(
                        key,
                        QuitConfirm::open(),
                        &ScopePrompt::Closed,
                        None,
                        false,
                        false
                    ),
                    Pressed::Confirm(QuitConfirm::open()),
                    "{kind:?} of {code:?} should answer nothing"
                );
            }
        }
    }

    #[test]
    fn the_scope_prompt_swallows_every_key_the_tree_answers_to() {
        // The confirmation's rule, said again for the other window: while
        // somebody is typing a scope, `j`, `k`, `g`, `G`, space, `o`, `f`,
        // `p`, `r`, `s`, `m`, Tab and the page keys are letters going into a
        // field or keystrokes that mean nothing, and `action_for` is not
        // consulted at all. Both layers: the key comes back as the prompt's
        // own answer and never as an `Action`, and the app behind the window
        // is the app that was there before it opened.
        //
        // Asserted on an empty field and on one already holding a scope,
        // because what is in the field is nothing to do with what the gate
        // does with a key.
        for text in ["", "data-plane"] {
            let mut app = app_in_use();
            let before = app.clone();
            let field = ScopeField::new(DIRECTORY, text);
            let mut prompt = ScopePrompt::Open(field.clone());
            let mut confirm = QuitConfirm::Closed;

            for code in INERT {
                let key = press(code);
                let pressed = press_for(key, QuitConfirm::Closed, &prompt, None, false, false);

                assert_eq!(
                    pressed,
                    Pressed::Scope(edit_for(key, prompt.field().expect("the prompt is up"))),
                    "{code:?} should go to the prompt and nowhere else"
                );
                assert!(
                    matches!(pressed, Pressed::Scope(_)),
                    "{code:?} reached something other than the prompt: {pressed:?}"
                );

                assert_eq!(
                    round_under(&mut app, &mut confirm, &mut prompt, key),
                    Round::Stayed
                );
            }

            assert!(prompt.is_open(), "the prompt is still up");
            assert_eq!(confirm, QuitConfirm::Closed, "and no question was opened");
            assert_eq!(app, before, "nothing reached the tree underneath");
        }
    }

    #[test]
    fn esc_and_q_belong_to_the_prompt_while_it_is_up() {
        // The order the gate decides in, where it is visible: the prompt is
        // asked before `action_for`, so `q` is a character somebody typed
        // rather than a way out, and Esc takes the prompt down rather than
        // putting a question in front of a session nobody asked to end.
        let field = ScopeField::new(DIRECTORY, "web");
        let prompt = ScopePrompt::Open(field.clone());

        assert_eq!(
            press_for(
                press(KeyCode::Char('q')),
                QuitConfirm::Closed,
                &prompt,
                None,
                false,
                false
            ),
            Pressed::Scope(Edited::Open(ScopeField::new(DIRECTORY, "webq"))),
            "q is a letter while the field has the keyboard"
        );
        assert_eq!(
            press_for(
                press(KeyCode::Esc),
                QuitConfirm::Closed,
                &prompt,
                None,
                false,
                false
            ),
            Pressed::Scope(Edited::Close),
            "Esc abandons the prompt rather than opening the question"
        );

        // And through the loop's arms: the prompt comes down, the question
        // does not go up, and the app never heard either keystroke.
        let mut app = app_in_use();
        let before = app.clone();
        let mut confirm = QuitConfirm::Closed;
        let mut prompt = ScopePrompt::Open(field);

        assert_eq!(
            round_under(&mut app, &mut confirm, &mut prompt, press(KeyCode::Esc)),
            Round::Stayed
        );
        assert_eq!(prompt, ScopePrompt::Closed, "the prompt came down");
        assert_eq!(confirm, QuitConfirm::Closed, "and nothing took its place");
        assert_eq!(app, before);
    }

    #[test]
    fn ctrl_c_leaves_at_once_with_the_scope_prompt_up() {
        // Answered before either window is consulted, and for the reason it
        // is answered before the question: through `edit_for` it is a `c`
        // wearing a modifier, i.e. one of the keys that change nothing, and
        // the last resort of a reader who wants out would be the one
        // keystroke the field swallowed. Pinned with an empty field, with
        // something typed, and at both settings of the run flag.
        for prompt in [
            ScopePrompt::open(DIRECTORY, ""),
            ScopePrompt::open(DIRECTORY, "data-plane"),
        ] {
            for in_flight in [false, true] {
                assert_eq!(
                    press_for(
                        ctrl_c(),
                        QuitConfirm::Closed,
                        &prompt,
                        None,
                        in_flight,
                        false
                    ),
                    Pressed::Leave,
                    "Ctrl-C should leave with {prompt:?} up and a run in flight = {in_flight}"
                );
            }
        }

        let mut app = app_in_use();
        let mut confirm = QuitConfirm::Closed;
        let mut prompt = ScopePrompt::open(DIRECTORY, "billing");
        assert_eq!(
            round_under(&mut app, &mut confirm, &mut prompt, ctrl_c()),
            Round::Left
        );
    }

    #[test]
    fn the_order_is_ctrl_c_the_question_the_prompt_the_composer_then_the_keys() {
        // The whole decision order in one test, each step asserted by taking
        // the situation above it away and pressing the same key again. `j`
        // is the key it is said with because it means something different to
        // every one of them: a letter to both fields, a key the question
        // ignores, and a movement to the app.
        let key = press(KeyCode::Char('j'));
        let draft = Composer::new("web");
        let prompt = ScopePrompt::open(DIRECTORY, "web");
        let question = QuitConfirm::open();

        // Ctrl-C, over all three at once. It is a key event and not a
        // signal, so if the gate does not answer it here nothing does.
        assert_eq!(
            press_for(ctrl_c(), question, &prompt, Some(&draft), false, false),
            Pressed::Leave
        );
        // Then the question, which is drawn over everything else on the
        // frame: a key cannot be both typed into a field and answered by the
        // dialog covering it.
        assert_eq!(
            press_for(key, question, &prompt, Some(&draft), false, false),
            Pressed::Confirm(question)
        );
        // Then the prompt, over the composer, for the same reason again.
        assert_eq!(
            press_for(
                key,
                QuitConfirm::Closed,
                &prompt,
                Some(&draft),
                false,
                false
            ),
            Pressed::Scope(edit_for(key, prompt.field().expect("the prompt is up")))
        );
        // Then the composer, over the keys: this is where `j` stops being a
        // movement and becomes the letter j.
        assert_eq!(
            press_for(
                key,
                QuitConfirm::Closed,
                &ScopePrompt::Closed,
                Some(&draft),
                false,
                false
            ),
            Pressed::Compose(Composed::Typing(Composer::new("webj")))
        );
        // And then the keys, as they have always been read.
        assert_eq!(
            press_for(
                key,
                QuitConfirm::Closed,
                &ScopePrompt::Closed,
                None,
                false,
                false
            ),
            Pressed::Act(Action::SelectNext)
        );
    }

    mod writing {
        use super::{
            Action, Composer, Edited, INERT, KeyCode, KeyEvent, Pressed, QuitConfirm, RecordPrompt,
            ScopeField, ScopePrompt, action_for, ctrl_c, edit_for, gate_for, press,
        };

        const PROPOSED: &str = "docs/warlock-brief-13-scopes-and-sigils.md";

        const HEADING: &str = "Write the brief to";

        fn open() -> ScopePrompt {
            ScopePrompt::open(HEADING, PROPOSED)
        }

        fn field() -> ScopeField {
            open().field().expect("the prompt is up").clone()
        }

        fn asked(key: KeyEvent, write: &ScopePrompt) -> Pressed {
            gate_for(
                key,
                QuitConfirm::Closed,
                &ScopePrompt::Closed,
                &RecordPrompt::Closed,
                write,
                None,
                false,
                false,
            )
        }

        #[test]
        fn every_binding_goes_into_the_field_and_none_of_them_reaches_the_app() {
            // The promise, over the whole list: while a path is being typed
            // there is no `p` that pacts, no `j` that moves a selection
            // nobody can see and no Tab that hands the keyboard somewhere
            // else — every one of them is `edit_for`'s answer and nothing
            // else's.
            let write = open();
            let field = field();

            for code in INERT {
                let key = press(code);

                assert_eq!(
                    asked(key, &write),
                    Pressed::Write(edit_for(key, &field)),
                    "{code:?} should have gone into the field"
                );
            }
        }

        #[test]
        fn enter_submits_and_esc_closes_and_nothing_else_does_either() {
            let write = open();

            assert_eq!(
                asked(press(KeyCode::Enter), &write),
                Pressed::Write(Edited::Submit)
            );
            assert_eq!(
                asked(press(KeyCode::Esc), &write),
                Pressed::Write(Edited::Close)
            );
            for code in INERT.into_iter().chain([KeyCode::Backspace]) {
                let pressed = asked(press(code), &write);

                assert_ne!(pressed, Pressed::Write(Edited::Submit), "{code:?}");
                assert_ne!(pressed, Pressed::Write(Edited::Close), "{code:?}");
            }
        }

        #[test]
        fn ctrl_c_is_still_answered_before_the_field() {
            // The keystroke of last resort, with this window up as with
            // every other: it leaves with nothing out, and stops the turn
            // when one is being answered. Typed into the field it would be
            // a `c` on the end of a path.
            let write = open();

            assert_eq!(asked(ctrl_c(), &write), Pressed::Leave);
            assert_eq!(
                gate_for(
                    ctrl_c(),
                    QuitConfirm::Closed,
                    &ScopePrompt::Closed,
                    &RecordPrompt::Closed,
                    &write,
                    None,
                    false,
                    true,
                ),
                Pressed::CancelTurn
            );
        }

        #[test]
        fn the_composer_is_not_consulted_while_the_field_is_up() {
            // The window is drawn over the field at the foot of the panel,
            // so a key cannot be both typed into the path and typed into a
            // draft. Tab included: it is the key that moves the keyboard
            // *past* the composer, and there is nowhere for it to move it
            // while a window is up.
            let write = open();
            let draft = Composer::new("web");

            for code in [KeyCode::Char('j'), KeyCode::Tab, KeyCode::Enter] {
                let key = press(code);

                assert_eq!(
                    gate_for(
                        key,
                        QuitConfirm::Closed,
                        &ScopePrompt::Closed,
                        &RecordPrompt::Closed,
                        &write,
                        Some(&draft),
                        false,
                        false,
                    ),
                    Pressed::Write(edit_for(key, &field())),
                    "{code:?} reached the draft from behind the window"
                );
            }
        }

        #[test]
        fn the_scope_prompt_has_the_keys_while_both_windows_are_up() {
            // The precedence, and the one situation that needs one: `s`
            // opens the scope prompt from the tree while a `/write` turn is
            // still out, and the answer to that turn opens this window with
            // no keystroke at all. The scope prompt is the one somebody is
            // typing in, so it keeps the keyboard; the write prompt waits
            // underneath with its path exactly as it was, which is what the
            // loop's separate arms then act on.
            let write = open();
            let scope = ScopePrompt::open("crates/warlock-engine", "data-plane");
            let scope_field = scope.field().expect("the prompt is up").clone();

            for code in INERT.into_iter().chain([KeyCode::Enter, KeyCode::Esc]) {
                let key = press(code);

                assert_eq!(
                    gate_for(
                        key,
                        QuitConfirm::Closed,
                        &scope,
                        &RecordPrompt::Closed,
                        &write,
                        None,
                        false,
                        false,
                    ),
                    Pressed::Scope(edit_for(key, &scope_field)),
                    "{code:?} was answered by the wrong window"
                );
            }
            // And the window underneath is untouched by any of it: it is
            // read, never written, on that road.
            assert_eq!(write, open());
        }

        #[test]
        fn the_keys_mean_what_they_always_did_once_the_window_is_down() {
            // The other half of the promise, and the one that says
            // `action_for` was not touched: with nothing up, every one of
            // the keys the field swallowed is the command it has always
            // been, straight out of the function that has always decided it.
            for code in INERT {
                let key = press(code);
                let expected = match action_for(key, false) {
                    Some(Action::Quit) => Pressed::Confirm(QuitConfirm::open()),
                    Some(action) => Pressed::Act(action),
                    None => Pressed::Nothing,
                };

                assert_eq!(
                    asked(key, &ScopePrompt::Closed),
                    expected,
                    "{code:?} stopped meaning what it meant"
                );
            }
        }
    }

    // The fourth window: the three values a brand-new scope name is recorded
    // with, which `s` puts up as it takes the scope window down.
    mod recording {
        use warlock_tui::{RecordEdited, RecordForm, RecordPrompt, record_edit_for};

        use super::{
            Composer, INERT, KeyCode, KeyEvent, Pressed, QuitConfirm, ScopePrompt, ctrl_c,
            gate_for, press,
        };

        const DIRECTORY: &str = "crates/warlock-engine";

        const SCOPE: &str = "data-plane";

        fn open() -> RecordPrompt {
            RecordPrompt::open(DIRECTORY, SCOPE)
        }

        fn form() -> RecordForm {
            open().form().expect("the window is up").clone()
        }

        fn asked(key: KeyEvent, record: &RecordPrompt) -> Pressed {
            asked_composing(key, record, None)
        }

        fn asked_composing(
            key: KeyEvent,
            record: &RecordPrompt,
            composer: Option<&Composer>,
        ) -> Pressed {
            gate_for(
                key,
                QuitConfirm::Closed,
                // Closed, and there is no test here of the two being up
                // together, because they cannot be: the record window opens
                // exactly as the scope window closes, out of the one value
                // `scope_submit` hands back.
                &ScopePrompt::Closed,
                record,
                &ScopePrompt::Closed,
                composer,
                false,
                false,
            )
        }

        #[test]
        fn every_binding_goes_into_the_focused_field_and_none_of_them_reaches_the_app() {
            // The same promise the other two windows make, and the one that
            // matters most here because `s` is on the list: while a record is
            // being filled in there is no `s` that reopens the window over the
            // scope it is already recording, no `p` that pacts and no `j` that
            // moves a selection nobody can see.
            let record = open();
            let form = form();

            for code in INERT {
                let key = press(code);

                assert_eq!(
                    asked(key, &record),
                    Pressed::Record(record_edit_for(key, &form)),
                    "{code:?} should have gone into the focused field"
                );
            }
        }

        #[test]
        fn enter_submits_and_esc_closes_and_tab_only_moves_the_focus() {
            let record = open();

            assert_eq!(
                asked(press(KeyCode::Enter), &record),
                Pressed::Record(RecordEdited::Submit)
            );
            assert_eq!(
                asked(press(KeyCode::Esc), &record),
                Pressed::Record(RecordEdited::Close)
            );
            for code in [KeyCode::Tab, KeyCode::BackTab, KeyCode::Down, KeyCode::Up] {
                let pressed = asked(press(code), &record);

                assert_eq!(
                    pressed,
                    Pressed::Record(record_edit_for(press(code), &form())),
                    "{code:?} should be the window's own move"
                );
                assert_ne!(pressed, Pressed::Record(RecordEdited::Submit), "{code:?}");
                assert_ne!(pressed, Pressed::Record(RecordEdited::Close), "{code:?}");
            }
        }

        #[test]
        fn ctrl_c_is_still_answered_before_the_fields() {
            // The keystroke of last resort, with this window up as with every
            // other: it leaves with nothing out, and stops the turn when one is
            // being answered. Typed into a field it would be a `c` in a team
            // name.
            let record = open();

            assert_eq!(asked(ctrl_c(), &record), Pressed::Leave);
            assert_eq!(
                gate_for(
                    ctrl_c(),
                    QuitConfirm::Closed,
                    &ScopePrompt::Closed,
                    &record,
                    &ScopePrompt::Closed,
                    None,
                    false,
                    true,
                ),
                Pressed::CancelTurn
            );
        }

        #[test]
        fn the_quit_confirmation_is_answered_before_the_fields_too() {
            // Not a situation the loop gets into either — `q` is not a key this
            // window lets through, so the question cannot come up behind it —
            // but the order is the gate's and it is asserted where the gate is
            // read, not inferred from the window that happens to be up.
            let record = open();

            assert_eq!(
                gate_for(
                    press(KeyCode::Char('y')),
                    QuitConfirm::open(),
                    &ScopePrompt::Closed,
                    &record,
                    &ScopePrompt::Closed,
                    None,
                    false,
                    false,
                ),
                Pressed::Leave
            );
        }

        #[test]
        fn the_composer_is_not_consulted_while_the_window_is_up() {
            // The window is drawn over the draft at the foot of the panel, so a
            // key cannot be both typed into a field and typed into a draft. Tab
            // included: here it moves the focus between the three fields, which
            // is the one place it can go while a window is up.
            let record = open();
            let draft = Composer::new("web");

            for code in [KeyCode::Char('j'), KeyCode::Tab, KeyCode::Enter] {
                let key = press(code);

                assert_eq!(
                    asked_composing(key, &record, Some(&draft)),
                    Pressed::Record(record_edit_for(key, &form())),
                    "{code:?} reached the draft from behind the window"
                );
            }
        }

        #[test]
        fn the_window_being_down_leaves_every_key_to_the_tree() {
            // The other half: nothing above is a rule about the key, only about
            // the window, so with it closed the gate is the gate it always was.
            for code in INERT {
                let key = press(code);

                assert_eq!(
                    asked(key, &RecordPrompt::Closed),
                    super::press_for(
                        key,
                        QuitConfirm::Closed,
                        &ScopePrompt::Closed,
                        None,
                        false,
                        false
                    ),
                    "{code:?} was answered by a window that is not up"
                );
            }
        }
    }

    // The fifth window: the question a `/push` asks before anything leaves the
    // machine. Nothing opens it yet, so every round in here opens it by hand
    // and calls the gate itself rather than the shim the rest of this module
    // uses.
    mod pushing {
        use super::{
            Composer, INERT, KeyCode, KeyEvent, Pressed, PushConfirm, QuitConfirm, RecordPrompt,
            ScopePrompt, ctrl_c, every_window, press, push_answer_for,
        };

        fn open() -> PushConfirm {
            PushConfirm::open(
                "Push a brief to the board",
                "warlock-team",
                "Warlock",
                "work",
            )
        }

        // The gate with this window up and every other one down, which is the
        // only way it is ever up in a session.
        fn asked(key: KeyEvent, push: &PushConfirm) -> Pressed {
            asking(key, push, None, false)
        }

        fn asking(
            key: KeyEvent,
            push: &PushConfirm,
            composer: Option<&Composer>,
            answered: bool,
        ) -> Pressed {
            every_window(
                key,
                QuitConfirm::Closed,
                push,
                &ScopePrompt::Closed,
                &ScopePrompt::Closed,
                &RecordPrompt::Closed,
                &ScopePrompt::Closed,
                composer,
                false,
                answered,
            )
        }

        // What the dialog itself says about a key, which is what the gate has
        // to hand back for every one of them.
        fn answered(key: KeyEvent, push: &PushConfirm) -> Pressed {
            let filing = push.filing().expect("the dialog under test is up");
            Pressed::Push(push_answer_for(key, filing.answer()))
        }

        #[test]
        fn every_tree_binding_is_the_dialogs_and_none_of_them_reaches_the_app() {
            // The promise over the whole list: while the question is up there
            // is no `p` that pacts, no `j` that moves a selection under the
            // dialog and no `r` that starts a refresh nobody asked for.
            let push = open();

            for code in INERT {
                let key = press(code);

                assert_eq!(
                    asked(key, &push),
                    answered(key, &push),
                    "{code:?} should have been answered by the dialog"
                );
            }
        }

        #[test]
        fn q_is_the_dialogs_too_so_the_quit_question_cannot_come_up_underneath() {
            // `q` is the key that would otherwise open the *other* dialog, and
            // two questions stacked on one frame is a question answered blind.
            let push = open();
            let key = press(KeyCode::Char('q'));

            assert_eq!(asked(key, &push), answered(key, &push));
            assert_ne!(asked(key, &push), Pressed::Confirm(QuitConfirm::open()));
        }

        #[test]
        fn ctrl_c_is_still_answered_before_the_dialog() {
            // The keystroke of last resort, with this window up as with every
            // other: it leaves with nothing sent, and stops the turn when one
            // is being answered. Through the dialog it would be an inert `c`.
            let push = open();

            assert_eq!(asked(ctrl_c(), &push), Pressed::Leave);
            assert_eq!(asking(ctrl_c(), &push, None, true), Pressed::CancelTurn);
        }

        #[test]
        fn the_composer_is_not_consulted_while_the_dialog_is_up() {
            // The dialog is drawn over the field at the foot of the panel, so
            // a key cannot be both typed into a draft and answered here. Tab
            // included: there is nowhere for the keyboard to move while a
            // window is up.
            let push = open();
            let draft = Composer::new("web");

            for code in [KeyCode::Char('j'), KeyCode::Tab, KeyCode::Enter] {
                let key = press(code);

                assert_eq!(
                    asking(key, &push, Some(&draft), false),
                    answered(key, &push),
                    "{code:?} reached the draft from behind the dialog"
                );
            }
        }

        #[test]
        fn the_dialog_has_the_keys_while_any_of_the_three_fields_is_up() {
            // The precedence, written down: a `/write` turn still out opens the
            // write prompt on no keystroke at all, and a field that came up
            // under this dialog does not get to take the keys off it.
            let push = open();
            let scope = ScopePrompt::open("crates/warlock-engine", "data-plane");
            let record = RecordPrompt::open("crates/warlock-engine", "data-plane");
            let write = ScopePrompt::open("Write the brief to", "docs/brief.md");

            for code in INERT.into_iter().chain([KeyCode::Enter, KeyCode::Esc]) {
                let key = press(code);

                assert_eq!(
                    every_window(
                        key,
                        QuitConfirm::Closed,
                        &push,
                        &ScopePrompt::Closed,
                        &scope,
                        &record,
                        &write,
                        None,
                        false,
                        false,
                    ),
                    answered(key, &push),
                    "{code:?} was answered by the wrong window"
                );
            }
        }

        #[test]
        fn the_quit_question_is_asked_before_it() {
            // The other side of the order, and a situation no session is in:
            // `q` reaches nothing while this dialog is up, so the two are never
            // both on the frame. Asserted anyway, because which one answers has
            // to be decided somewhere rather than by the order of two `if`s
            // nobody looked at.
            let push = open();

            assert_eq!(
                asking(press(KeyCode::Enter), &push, None, false),
                answered(press(KeyCode::Enter), &push)
            );
            assert_eq!(
                every_window(
                    press(KeyCode::Enter),
                    QuitConfirm::open(),
                    &push,
                    &ScopePrompt::Closed,
                    &ScopePrompt::Closed,
                    &RecordPrompt::Closed,
                    &ScopePrompt::Closed,
                    None,
                    false,
                    false,
                ),
                Pressed::Confirm(QuitConfirm::Closed)
            );
        }

        #[test]
        fn the_keys_mean_what_they_always_did_once_the_dialog_is_down() {
            // The other half of the promise: with the question answered, every
            // key it swallowed is whatever the gate with nothing up says it is.
            for code in INERT.into_iter().chain([KeyCode::Char('q')]) {
                let key = press(code);

                assert_eq!(
                    asked(key, &PushConfirm::Closed),
                    super::press_for(
                        key,
                        QuitConfirm::Closed,
                        &ScopePrompt::Closed,
                        None,
                        false,
                        false
                    ),
                    "{code:?} was answered by a window that is not up"
                );
            }
        }
    }

    // The sixth window: the field that comes up in front of that dialog when
    // this machine can file to more than one board. Nothing but a `/push`
    // opens it either, so every round in here opens it by hand.
    mod filing {
        use super::{
            Composer, Edited, INERT, KeyCode, KeyEvent, Pressed, PushConfirm, QuitConfirm,
            RecordPrompt, ScopePrompt, ctrl_c, edit_for, every_window, press,
        };

        fn open() -> ScopePrompt {
            ScopePrompt::open("Scope to file the brief to", "dat")
        }

        fn asked(key: KeyEvent, filing: &ScopePrompt) -> Pressed {
            asking(key, filing, None, false)
        }

        fn asking(
            key: KeyEvent,
            filing: &ScopePrompt,
            composer: Option<&Composer>,
            answered: bool,
        ) -> Pressed {
            every_window(
                key,
                QuitConfirm::Closed,
                &PushConfirm::Closed,
                filing,
                &ScopePrompt::Closed,
                &RecordPrompt::Closed,
                &ScopePrompt::Closed,
                composer,
                false,
                answered,
            )
        }

        // What the field itself says about a key, which is what the gate has
        // to hand back for every one of them.
        fn typed(key: KeyEvent, filing: &ScopePrompt) -> Pressed {
            let field = filing.field().expect("the field under test is up");
            Pressed::Filing(edit_for(key, field))
        }

        #[test]
        fn every_tree_binding_is_text_in_the_field_and_none_of_them_reaches_the_app() {
            // While this is up, `j`, `p`, `r` and the rest are letters
            // somebody is typing a scope name with.
            let filing = open();

            for code in INERT
                .into_iter()
                .chain([KeyCode::Char('q'), KeyCode::Enter])
            {
                let key = press(code);

                assert_eq!(
                    asked(key, &filing),
                    typed(key, &filing),
                    "{code:?} should have been typed into the field"
                );
            }
        }

        #[test]
        fn ctrl_c_is_still_answered_before_the_field() {
            // The keystroke of last resort, here as everywhere else: through
            // the field it would be an inert `c`.
            let filing = open();

            assert_eq!(asked(ctrl_c(), &filing), Pressed::Leave);
            assert_eq!(asking(ctrl_c(), &filing, None, true), Pressed::CancelTurn);
        }

        #[test]
        fn the_composer_is_not_consulted_while_the_field_is_up() {
            let filing = open();
            let draft = Composer::new("web");

            for code in [KeyCode::Char('j'), KeyCode::Tab, KeyCode::Enter] {
                let key = press(code);

                assert_eq!(
                    asking(key, &filing, Some(&draft), false),
                    typed(key, &filing),
                    "{code:?} reached the draft from behind the field"
                );
            }
        }

        #[test]
        fn it_has_the_keys_while_any_of_the_other_three_fields_is_up() {
            // The precedence, written down: a `/write` turn still out opens
            // the write prompt on no keystroke at all, and a field that came
            // up under this one does not get to take the keys off it.
            let filing = open();
            let scope = ScopePrompt::open("crates/warlock-engine", "data-plane");
            let record = RecordPrompt::open("crates/warlock-engine", "data-plane");
            let write = ScopePrompt::open("Write the brief to", "docs/brief.md");

            for code in INERT.into_iter().chain([KeyCode::Enter, KeyCode::Esc]) {
                let key = press(code);

                assert_eq!(
                    every_window(
                        key,
                        QuitConfirm::Closed,
                        &PushConfirm::Closed,
                        &filing,
                        &scope,
                        &record,
                        &write,
                        None,
                        false,
                        false,
                    ),
                    typed(key, &filing),
                    "{code:?} was answered by the wrong window"
                );
            }
        }

        #[test]
        fn the_dialog_and_the_quit_question_are_both_asked_before_it() {
            // Neither is a situation a session is in — the submit that takes
            // this window down is the one that puts the dialog up — but which
            // window answers has to be decided somewhere rather than by the
            // order of two `if`s nobody looked at.
            let filing = open();
            let push = PushConfirm::open(
                "Push a brief to the board",
                "warlock-team",
                "Warlock",
                "work",
            );
            let key = press(KeyCode::Enter);

            assert!(matches!(
                every_window(
                    key,
                    QuitConfirm::Closed,
                    &push,
                    &filing,
                    &ScopePrompt::Closed,
                    &RecordPrompt::Closed,
                    &ScopePrompt::Closed,
                    None,
                    false,
                    false,
                ),
                Pressed::Push(_)
            ));
            assert_eq!(
                every_window(
                    key,
                    QuitConfirm::open(),
                    &PushConfirm::Closed,
                    &filing,
                    &ScopePrompt::Closed,
                    &RecordPrompt::Closed,
                    &ScopePrompt::Closed,
                    None,
                    false,
                    false,
                ),
                Pressed::Confirm(QuitConfirm::Closed)
            );
        }

        #[test]
        fn esc_abandons_it_and_enter_offers_the_name_up() {
            // The two keys that end it, which are the scope prompt's own: what
            // a submit comes to is the loop's, and all this says is where the
            // key came from.
            let filing = open();

            assert_eq!(
                asked(press(KeyCode::Esc), &filing),
                Pressed::Filing(Edited::Close)
            );
            assert_eq!(
                asked(press(KeyCode::Enter), &filing),
                Pressed::Filing(Edited::Submit)
            );
        }

        #[test]
        fn the_keys_mean_what_they_always_did_once_the_field_is_down() {
            for code in INERT.into_iter().chain([KeyCode::Char('q')]) {
                let key = press(code);

                assert_eq!(
                    asked(key, &ScopePrompt::Closed),
                    super::press_for(
                        key,
                        QuitConfirm::Closed,
                        &ScopePrompt::Closed,
                        None,
                        false,
                        false
                    ),
                    "{code:?} was answered by a window that is not up"
                );
            }
        }
    }

    mod composing {
        use std::time::Instant;

        use super::{
            Action, App, Composed, Composer, Focus, INERT, KeyCode, Pressed, QuitConfirm, Round,
            ScopePrompt, app_in_use, app_on_screen, ctrl_c, offered, press, press_for,
            round_composing,
        };

        const TYPED: &str = "web";

        fn app_composing() -> App {
            let mut app = app_in_use();
            let asked = Instant::now();
            app.panel_mut()
                .start_turn("what does the engine do?", asked);
            app.panel_mut()
                .answer_turn("It walks the tree.\n".repeat(40), asked);
            app.scroll_panel_up(5);
            app.set_focus(Focus::Composer);
            assert_eq!(
                app.focus(),
                Focus::Composer,
                "the composer can hold the keyboard with the thread card up"
            );
            app
        }

        fn types(code: char) {
            let key = press(KeyCode::Char(code));
            let before = Composer::new(TYPED);
            let typed = Composer::new(format!("{TYPED}{code}"));

            assert_eq!(
                press_for(
                    key,
                    QuitConfirm::Closed,
                    &ScopePrompt::Closed,
                    Some(&before),
                    false,
                    false
                ),
                Pressed::Compose(Composed::Typing(typed.clone())),
                "{code} should be a letter while the composer has the keyboard"
            );

            let mut app = app_composing();
            let untouched = app.clone();
            let mut composer = before;
            let mut confirm = QuitConfirm::Closed;
            let mut prompt = ScopePrompt::Closed;

            assert_eq!(
                round_composing(&mut app, &mut confirm, &mut prompt, &mut composer, key),
                Round::Stayed,
                "{code} should not end the session"
            );
            assert_eq!(composer, typed, "{code} should have gone into the draft");
            assert_eq!(app, untouched, "{code} reached the app behind the composer");
            assert_eq!(confirm, QuitConfirm::Closed, "and opened no question");
            assert_eq!(prompt, ScopePrompt::Closed, "and no prompt");
        }

        fn acts(code: char, action: Action) {
            let key = press(KeyCode::Char(code));
            let composer = Composer::new(TYPED);

            for focus in [Focus::Tree, Focus::Panel] {
                let mut app = app_in_use();
                app.set_focus(focus);

                assert_eq!(
                    press_for(
                        key,
                        QuitConfirm::Closed,
                        &ScopePrompt::Closed,
                        offered(&app, &composer),
                        false,
                        false
                    ),
                    Pressed::Act(action),
                    "{code} should mean {action:?} again with the keys at {focus:?}"
                );
            }

            assert_eq!(
                composer,
                Composer::new(TYPED),
                "{code} should have typed nothing anywhere"
            );
        }

        #[test]
        fn p_is_the_letter_p_while_the_composer_has_the_keyboard() {
            // The key the whole arrangement is for: `p` writes a manifest,
            // so a letter that pacted a directory would be the one typo that
            // costs somebody minutes of model time.
            types('p');
        }

        #[test]
        fn p_pacts_again_once_the_composer_has_let_go() {
            acts('p', Action::TogglePact);
        }

        #[test]
        fn r_is_the_letter_r_while_the_composer_has_the_keyboard() {
            types('r');
        }

        #[test]
        fn r_refreshes_again_once_the_composer_has_let_go() {
            acts('r', Action::Refresh);
        }

        #[test]
        fn s_is_the_letter_s_while_the_composer_has_the_keyboard() {
            // And a window that opened over the field somebody is typing in
            // would take the keyboard off them mid-sentence.
            types('s');
        }

        #[test]
        fn s_scopes_again_once_the_composer_has_let_go() {
            acts('s', Action::OpenScope);
        }

        #[test]
        fn v_is_the_letter_v_while_the_composer_has_the_keyboard() {
            types('v');
        }

        #[test]
        fn v_reads_a_file_again_once_the_composer_has_let_go() {
            acts('v', Action::ViewFile);
        }

        #[test]
        fn e_is_the_letter_e_while_the_composer_has_the_keyboard() {
            // The worst of them to leak: `e` hands the terminal to an editor,
            // so a typed letter would take the screen away mid-draft.
            types('e');
        }

        #[test]
        fn e_edits_a_file_again_once_the_composer_has_let_go() {
            acts('e', Action::EditFile);
        }

        #[test]
        fn f_is_the_letter_f_while_the_composer_has_the_keyboard() {
            types('f');
        }

        #[test]
        fn f_shows_the_files_again_once_the_composer_has_let_go() {
            acts('f', Action::ToggleFiles);
        }

        #[test]
        fn g_is_the_letter_g_while_the_composer_has_the_keyboard() {
            types('g');
        }

        #[test]
        fn g_jumps_to_the_first_row_again_once_the_composer_has_let_go() {
            acts('g', Action::SelectFirst);
        }

        #[test]
        fn upper_g_is_the_letter_g_while_the_composer_has_the_keyboard() {
            // Its own test rather than a second case of `g`'s: the pair is
            // told apart by case alone, so a field that folded the letter
            // would be a field somebody cannot write a sentence in.
            types('G');
        }

        #[test]
        fn upper_g_jumps_to_the_last_row_again_once_the_composer_has_let_go() {
            acts('G', Action::SelectLast);
        }

        #[test]
        fn j_is_the_letter_j_while_the_composer_has_the_keyboard() {
            types('j');
        }

        #[test]
        fn j_moves_the_selection_down_again_once_the_composer_has_let_go() {
            acts('j', Action::SelectNext);
        }

        #[test]
        fn k_is_the_letter_k_while_the_composer_has_the_keyboard() {
            types('k');
        }

        #[test]
        fn k_moves_the_selection_up_again_once_the_composer_has_let_go() {
            acts('k', Action::SelectPrevious);
        }

        #[test]
        fn every_other_binding_the_tree_has_is_the_composers_too() {
            // The ten keys above one by one, and then the rest of the list in
            // a loop: space, `o`, `m`, Shift-Tab, the arrows and the page
            // keys are text or nothing while the field has the keyboard, and
            // none of them is an `Action`. Tab is the exception and has its
            // own test below.
            let mut app = app_composing();
            let untouched = app.clone();
            let mut composer = Composer::new(TYPED);
            let mut confirm = QuitConfirm::Closed;
            let mut prompt = ScopePrompt::Closed;

            for code in INERT.into_iter().filter(|code| *code != KeyCode::Tab) {
                let pressed = press_for(
                    press(code),
                    QuitConfirm::Closed,
                    &ScopePrompt::Closed,
                    Some(&composer),
                    false,
                    false,
                );

                assert!(
                    matches!(pressed, Pressed::Compose(_)),
                    "{code:?} reached something other than the composer: {pressed:?}"
                );
                assert_eq!(
                    round_composing(
                        &mut app,
                        &mut confirm,
                        &mut prompt,
                        &mut composer,
                        press(code)
                    ),
                    Round::Stayed
                );
            }

            assert_eq!(app, untouched, "nothing reached the app underneath");
            assert_eq!(confirm, QuitConfirm::Closed, "and no question was opened");
            assert_eq!(prompt, ScopePrompt::Closed, "and no prompt");
        }

        #[test]
        fn every_key_a_muted_field_is_given_neither_types_nor_acts() {
            // One question at a time, and the half of it that has teeth: a
            // muted field is one whose last question is still being
            // answered, and a letter pressed at it must not fall through to
            // the tree's bindings. A `p` that did would start a pact over
            // whatever row happened to be selected — the very accident the
            // field was built to stop, arriving by the other road.
            let mut muted = Composer::new(TYPED);
            muted.set_muted(true);

            let mut app = app_composing();
            let untouched = app.clone();
            let mut confirm = QuitConfirm::Closed;
            let mut prompt = ScopePrompt::Closed;

            // Every letter of both cases, the digits, and the rest of the
            // list the tree answers to: the whole keyboard, so a binding
            // added later is covered by the loop rather than by somebody
            // remembering to add it here.
            let letters = ('a'..='z').chain('A'..='Z').chain('0'..='9');
            let codes = letters
                .map(KeyCode::Char)
                .chain(INERT)
                .chain([KeyCode::Enter, KeyCode::Esc])
                .filter(|code| *code != KeyCode::Tab);

            for code in codes {
                let mut composer = muted.clone();

                assert_eq!(
                    press_for(
                        press(code),
                        QuitConfirm::Closed,
                        &ScopePrompt::Closed,
                        Some(&composer),
                        false,
                        true
                    ),
                    Pressed::Nothing,
                    "{code:?} did something at a muted field"
                );
                // And through the loop's own round, which panics on the
                // four keys that start a run or open a window: a `p` that
                // leaked past the muting would be caught there rather than
                // quietly passing here.
                assert_eq!(
                    round_composing(
                        &mut app,
                        &mut confirm,
                        &mut prompt,
                        &mut composer,
                        press(code)
                    ),
                    Round::Stayed,
                    "{code:?} ended the session from a muted field"
                );
                assert_eq!(composer, muted, "{code:?} moved a muted draft");
            }

            assert_eq!(app, untouched, "a key reached the app behind a muted field");
            assert_eq!(confirm, QuitConfirm::Closed, "and opened no question");
            assert_eq!(prompt, ScopePrompt::Closed, "and no prompt");
        }

        #[test]
        fn the_two_keys_a_muted_field_does_not_swallow_are_tab_and_ctrl_c() {
            // Muting is about the letters. Tab is outside the field
            // altogether — it is the key that moves the keyboard, and a
            // field that ate it while refusing to be typed in would have no
            // way out until the model answered — and Ctrl-C is answered
            // before the field is consulted at all, which is what makes it
            // the way to take a question back.
            let mut muted = Composer::new(TYPED);
            muted.set_muted(true);

            assert_eq!(
                press_for(
                    press(KeyCode::Tab),
                    QuitConfirm::Closed,
                    &ScopePrompt::Closed,
                    Some(&muted),
                    false,
                    true
                ),
                Pressed::Act(Action::ToggleFocus),
            );
            assert_eq!(
                press_for(
                    ctrl_c(),
                    QuitConfirm::Closed,
                    &ScopePrompt::Closed,
                    Some(&muted),
                    false,
                    true
                ),
                Pressed::CancelTurn,
            );
        }

        #[test]
        fn a_run_started_while_the_thread_shows_leaves_the_field_typing() {
            // What used to be the second thing that muted the field, and is
            // not any more. A pact or a refresh started from the tree fills
            // the card behind the conversation, and the two never wait on
            // each other: a reader watching a long run is exactly who most
            // wants to ask something about the repository it is walking. So
            // `in_flight` changes nothing about what a key at a live field
            // does — including `p`, which is a letter here and the pact key
            // only at the tree.
            let mut app = app_composing();
            app.panel_mut()
                .start_turn("what does the engine do?", Instant::now());
            app.set_pact_in_flight("/repo/crates/engine", 3, 12);
            assert!(
                app.panel().showing_thread(),
                "the thread is the card on screen"
            );
            assert_eq!(app.focus(), Focus::Composer, "the field has the keyboard");

            let live = Composer::new(TYPED);
            for code in ['p', 'r', 'e', 'v', 'q'] {
                let key = press(KeyCode::Char(code));
                let typed = Composer::new(format!("{TYPED}{code}"));

                assert_eq!(
                    press_for(
                        key,
                        QuitConfirm::Closed,
                        &ScopePrompt::Closed,
                        Some(&live),
                        true,
                        false
                    ),
                    Pressed::Compose(Composed::Typing(typed)),
                    "{code} did not reach a field with a run in flight"
                );
            }
        }

        #[test]
        fn the_field_types_again_the_moment_the_run_ends() {
            // The muting's other half over a run rather than a turn: what
            // the loop hands the field is worked out from the run being in
            // flight, so the round after it ends — however it ended — is the
            // round the same draft types in again.
            let mut composer = Composer::new(TYPED);
            composer.set_muted(true);

            assert_eq!(
                press_for(
                    press(KeyCode::Char('p')),
                    QuitConfirm::Closed,
                    &ScopePrompt::Closed,
                    Some(&composer),
                    true,
                    false
                ),
                Pressed::Nothing,
            );

            composer.set_muted(false);

            assert_eq!(
                press_for(
                    press(KeyCode::Char('p')),
                    QuitConfirm::Closed,
                    &ScopePrompt::Closed,
                    Some(&composer),
                    false,
                    false
                ),
                Pressed::Compose(Composed::Typing(Composer::new(format!("{TYPED}p")))),
            );
        }

        #[test]
        fn the_field_types_again_the_moment_the_turn_ends() {
            // The other half of the muting, and the one that matters to
            // somebody waiting: the flag is set once a round off the turn in
            // flight, so a turn that ended in any of its five ways leaves a
            // field that hears the next key. Same field, same draft, one
            // flag down.
            let mut composer = Composer::new(TYPED);
            composer.set_muted(true);

            assert_eq!(
                press_for(
                    press(KeyCode::Char('p')),
                    QuitConfirm::Closed,
                    &ScopePrompt::Closed,
                    Some(&composer),
                    false,
                    true
                ),
                Pressed::Nothing,
            );

            composer.set_muted(false);

            assert_eq!(
                press_for(
                    press(KeyCode::Char('p')),
                    QuitConfirm::Closed,
                    &ScopePrompt::Closed,
                    Some(&composer),
                    false,
                    false
                ),
                Pressed::Compose(Composed::Typing(Composer::new(format!("{TYPED}p")))),
            );
        }

        #[test]
        fn tab_still_moves_the_keyboard_on_rather_than_being_typed() {
            // The one key the composer does not get. It is not text on any
            // terminal, and a field that swallowed it would be a field whose
            // only way out is Esc — which means something else.
            let composer = Composer::new(TYPED);

            assert_eq!(
                press_for(
                    press(KeyCode::Tab),
                    QuitConfirm::Closed,
                    &ScopePrompt::Closed,
                    Some(&composer),
                    false,
                    false
                ),
                Pressed::Act(Action::ToggleFocus)
            );

            let mut app = app_composing();
            let mut composer = composer;
            let mut confirm = QuitConfirm::Closed;
            let mut prompt = ScopePrompt::Closed;

            assert_eq!(
                round_composing(
                    &mut app,
                    &mut confirm,
                    &mut prompt,
                    &mut composer,
                    press(KeyCode::Tab)
                ),
                Round::Stayed
            );
            assert_eq!(app.focus(), Focus::Tree, "the cycle went on round");
            assert_eq!(composer.draft(), TYPED, "and typed nothing on the way");
        }

        #[test]
        fn esc_hands_the_keyboard_back_and_leaves_the_draft_where_it_is() {
            let composer = Composer::new(TYPED);

            assert_eq!(
                press_for(
                    press(KeyCode::Esc),
                    QuitConfirm::Closed,
                    &ScopePrompt::Closed,
                    Some(&composer),
                    false,
                    false
                ),
                Pressed::Compose(Composed::Leave),
                "Esc belongs to the field rather than to the gate on the way out"
            );

            let mut app = app_composing();
            let mut expected = app.clone();
            expected.set_focus(Focus::Panel);
            let mut composer = composer;
            let mut confirm = QuitConfirm::Closed;
            let mut prompt = ScopePrompt::Closed;

            assert_eq!(
                round_composing(
                    &mut app,
                    &mut confirm,
                    &mut prompt,
                    &mut composer,
                    press(KeyCode::Esc)
                ),
                Round::Stayed,
                "Esc at the composer does not end the session"
            );
            assert_eq!(app, expected, "it moved the focus to the panel and no more");
            assert_eq!(composer.draft(), TYPED, "and threw nothing away");
            assert_eq!(confirm, QuitConfirm::Closed, "and asked nothing");
        }

        #[test]
        fn esc_at_the_composer_leaves_a_run_alone_and_the_next_one_cancels_it() {
            // Deliberate, and the same rule the scope prompt keeps: the Esc
            // pressed while a field has the keyboard is answered by that
            // field, and the press after it — with the keyboard back on the
            // panel — is the one that stops the run.
            let composer = Composer::new(TYPED);

            assert_eq!(
                press_for(
                    press(KeyCode::Esc),
                    QuitConfirm::Closed,
                    &ScopePrompt::Closed,
                    Some(&composer),
                    true,
                    false
                ),
                Pressed::Compose(Composed::Leave)
            );
            assert_eq!(
                press_for(
                    press(KeyCode::Esc),
                    QuitConfirm::Closed,
                    &ScopePrompt::Closed,
                    None,
                    true,
                    false
                ),
                Pressed::Act(Action::CancelPact)
            );
        }

        #[test]
        fn enter_offers_the_draft_up_and_the_loop_does_nothing_whatever_with_it() {
            // The submission has no consumer in this slice: nothing is
            // started, nothing is spawned and nothing is written. The round
            // panics on every arm that would do any of those, so "inert" is
            // asserted rather than described — and the app it was holding
            // comes out of the round unchanged, message and all.
            let composer = Composer::new("why nine passes");

            assert_eq!(
                press_for(
                    press(KeyCode::Enter),
                    QuitConfirm::Closed,
                    &ScopePrompt::Closed,
                    Some(&composer),
                    false,
                    false
                ),
                Pressed::Compose(Composed::Submit)
            );

            let mut app = app_composing();
            let untouched = app.clone();
            let mut composer = composer;
            let mut confirm = QuitConfirm::Closed;
            let mut prompt = ScopePrompt::Closed;

            assert_eq!(
                round_composing(
                    &mut app,
                    &mut confirm,
                    &mut prompt,
                    &mut composer,
                    press(KeyCode::Enter)
                ),
                Round::Stayed
            );
            assert_eq!(app, untouched, "a submit changed something");
            assert_eq!(
                composer.draft(),
                "why nine passes",
                "and the draft is left for the consumer this slice does not have"
            );
            assert_eq!(prompt, ScopePrompt::Closed, "and opened no window");
        }

        #[test]
        fn an_empty_or_blank_submit_puts_no_message_on_the_footer() {
            // A submission with nothing in it is a keystroke, not a mistake:
            // it leaves the draft as it was and says nothing at all. Asserted
            // on an app with a clean footer, so a line put there would be the
            // only line there is.
            for draft in ["", " ", "  \t ", "\n", " \n \n "] {
                let mut app = app_on_screen();
                app.set_focus(Focus::Composer);
                let untouched = app.clone();
                let mut composer = Composer::new(draft);
                let mut confirm = QuitConfirm::Closed;
                let mut prompt = ScopePrompt::Closed;

                assert!(app.message().is_none(), "the footer starts with nothing");
                assert_eq!(
                    round_composing(
                        &mut app,
                        &mut confirm,
                        &mut prompt,
                        &mut composer,
                        press(KeyCode::Enter)
                    ),
                    Round::Stayed
                );

                assert_eq!(
                    app.message(),
                    None,
                    "submitting {draft:?} said something on the footer"
                );
                assert_eq!(app, untouched, "and changed something");
                assert_eq!(composer, Composer::new(draft), "and moved the draft");
            }
        }

        #[test]
        fn ctrl_c_leaves_at_once_and_types_no_c_while_the_composer_has_it() {
            // The order the gate decides in, where it matters most: through
            // `compose_for` Ctrl-C is a chord rather than text, so a gate
            // that consulted the field first would answer the one keystroke
            // every reader trusts with nothing at all — and would not even
            // leave a `c` behind to show for it.
            for draft in ["", TYPED] {
                let composer = Composer::new(draft);

                for in_flight in [false, true] {
                    assert_eq!(
                        press_for(
                            ctrl_c(),
                            QuitConfirm::Closed,
                            &ScopePrompt::Closed,
                            Some(&composer),
                            in_flight,
                            false
                        ),
                        Pressed::Leave,
                        "Ctrl-C should leave from {draft:?} with a run in flight = {in_flight}"
                    );
                }
            }

            let mut app = app_composing();
            let mut composer = Composer::new(TYPED);
            let mut confirm = QuitConfirm::Closed;
            let mut prompt = ScopePrompt::Closed;

            assert_eq!(
                round_composing(&mut app, &mut confirm, &mut prompt, &mut composer, ctrl_c()),
                Round::Left
            );
            assert_eq!(composer.draft(), TYPED, "and typed no c on the way out");
        }

        #[test]
        fn the_draft_survives_esc_the_focus_cycle_and_a_run_that_started_and_ended() {
            // Where the draft is kept, said as a fact about a session rather
            // than as a rule somebody follows: it is a local of the event
            // loop, so nothing that happens to the `App` can reach it. The
            // run is the case that decides it — a pact or a refresh that
            // recorded nothing puts the copy taken before it back over the
            // live app and keeps only the panel (`App::restore_from`), so a
            // draft stored there would be a draft a run swallowed half a
            // sentence into.
            let mut app = app_composing();
            let mut composer = Composer::default();
            let mut confirm = QuitConfirm::Closed;
            let mut prompt = ScopePrompt::Closed;

            for character in "why nine".chars() {
                round_composing(
                    &mut app,
                    &mut confirm,
                    &mut prompt,
                    &mut composer,
                    press(KeyCode::Char(character)),
                );
            }
            assert_eq!(composer.draft(), "why nine");

            // Esc: the keyboard goes back to the panel and the draft stays.
            round_composing(
                &mut app,
                &mut confirm,
                &mut prompt,
                &mut composer,
                press(KeyCode::Esc),
            );
            assert_eq!(app.focus(), Focus::Panel);
            assert_eq!(composer.draft(), "why nine", "Esc threw the draft away");

            // A run that started and ended with nothing recorded, which is
            // the one move that replaces the whole app.
            let before = app.clone();
            app.start_account(Instant::now());
            app.restore_from(before);
            assert_eq!(
                composer.draft(),
                "why nine",
                "a run that ended took the draft with it"
            );

            // And the focus all the way round the cycle: panel, composer,
            // tree, panel.
            for _ in 0..3 {
                round_composing(
                    &mut app,
                    &mut confirm,
                    &mut prompt,
                    &mut composer,
                    press(KeyCode::Tab),
                );
            }
            assert_eq!(app.focus(), Focus::Panel, "back where it started");
            assert_eq!(
                composer.draft(),
                "why nine",
                "the focus cycle typed into the draft or emptied it"
            );

            // Typing carries on exactly where it left off.
            app.set_focus(Focus::Composer);
            for character in " passes".chars() {
                round_composing(
                    &mut app,
                    &mut confirm,
                    &mut prompt,
                    &mut composer,
                    press(KeyCode::Char(character)),
                );
            }
            assert_eq!(composer.draft(), "why nine passes");
        }
    }
}

mod pointer {
    use std::time::{Duration, Instant};

    use ratatui::crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
    use ratatui::layout::Size;
    use warlock_engine::NodeState;
    use warlock_tui::{
        App, Cell, Composer, Focus, PushConfirm, QuitConfirm, Reach, RecordPrompt, Row,
        ScopePrompt, panel_height, panel_width, tree_height,
    };

    use super::super::{MouseAction, WHEEL_NOTCH, mouse_action};

    // The one terminal every test below points at, and the layout it comes
    // to. Eighty columns is wide enough that the tree takes its floor of
    // thirty rather than an even split, which is what puts the panes where
    // the columns and rows underneath say they are — narrow this and every
    // constant below moves:
    //
    //   columns  0        panel        49 50       tree        79
    //   row  0   ┌───────────────────────┐┌───────────────────────┐
    //   row  1   │ panel line 0          ││ tree header           │
    //   row  2   │ panel line 1          ││ tree row 0            │
    //    ...     │  ...                  ││  ...                  │
    //   row 19   │ panel line 18         ││ tree row 17           │
    //   row 20   └───────────────────────┘└───────────────────────┘
    //   rows 21-23                     the footer
    const SIZE: Size = Size {
        width: 80,
        height: 24,
    };

    const IN_TREE: u16 = 65;

    const IN_PANEL: u16 = 10;

    const FIRST_TREE_ROW: u16 = 2;

    const FIRST_PANEL_LINE: u16 = 1;

    const TREE_HEADER: u16 = 1;

    const FOOTER: u16 = 22;

    // The footer is nobody's pane; a border is the line between two of
    // them rather than a place a reader means to point at. The columns are
    // the panel's left border, the two panes' shared edge and the tree's
    // right, and the rows are the panes' top and bottom.
    const NOWHERE: [(u16, u16); 8] = [
        (IN_PANEL, FOOTER),
        (IN_TREE, FOOTER),
        (0, FIRST_PANEL_LINE),
        (49, FIRST_TREE_ROW),
        (50, FIRST_TREE_ROW),
        (79, FIRST_TREE_ROW),
        (IN_TREE, 0),
        (IN_PANEL, 20),
    ];

    fn viewport() -> usize {
        usize::from(tree_height(SIZE))
    }

    fn wheel_down(column: u16, row: u16) -> MouseEvent {
        event(MouseEventKind::ScrollDown, column, row)
    }

    fn wheel_up(column: u16, row: u16) -> MouseEvent {
        event(MouseEventKind::ScrollUp, column, row)
    }

    fn left_click(column: u16, row: u16) -> MouseEvent {
        event(MouseEventKind::Down(MouseButton::Left), column, row)
    }

    fn drag(column: u16, row: u16) -> MouseEvent {
        event(MouseEventKind::Drag(MouseButton::Left), column, row)
    }

    fn release(column: u16, row: u16) -> MouseEvent {
        event(MouseEventKind::Up(MouseButton::Left), column, row)
    }

    fn event(kind: MouseEventKind, column: u16, row: u16) -> MouseEvent {
        MouseEvent {
            kind,
            column,
            row,
            modifiers: KeyModifiers::NONE,
        }
    }

    fn whole_pointer() -> [MouseEvent; 8] {
        [
            wheel_down(IN_TREE, FIRST_TREE_ROW),
            wheel_up(IN_TREE, FIRST_TREE_ROW),
            wheel_down(IN_PANEL, FIRST_PANEL_LINE),
            wheel_up(IN_PANEL, FIRST_PANEL_LINE),
            left_click(IN_TREE, FIRST_TREE_ROW),
            left_click(IN_TREE, FIRST_TREE_ROW + 9),
            left_click(IN_TREE, TREE_HEADER),
            left_click(IN_PANEL, FIRST_PANEL_LINE + 3),
        ]
    }

    fn rows() -> Vec<Row> {
        let mut rows = vec![
            Row::new(0, "/repo", "/repo/WARLOCK.md", NodeState::PactedStale).with_child_count(24),
        ];
        for n in 0..24 {
            let directory = format!("/repo/d{n:02}");
            // No child count: the row under each of these is the file below,
            // and a file is not a child. What makes them collapsible is the
            // file toggle being on, which is `App::can_collapse`'s answer
            // and not the tree's.
            rows.push(Row::new(1, directory.clone(), None, NodeState::Unpacted));
            rows.push(Row::file(
                2,
                format!("{directory}/lib.rs"),
                NodeState::Unpacted,
            ));
        }
        rows
    }

    fn app_on_screen() -> App {
        let mut app = App::from_rows(rows());
        app.set_viewport_height(tree_height(SIZE));
        app.panel_mut().set_height(panel_height(SIZE, None, None));
        app.panel_mut().set_width(panel_width(SIZE));
        app
    }

    // The same screen with something on the thread card, which is the one
    // card a gesture selects in: an app with the conversation showing and
    // nothing recorded on it is a card with no text under the pointer, and
    // the fixture above is deliberately left that way.
    fn app_talking() -> App {
        let mut app = app_on_screen();
        let base = Instant::now();
        app.panel_mut().start_turn("what does the engine do?", base);
        app.panel_mut().answer_turn(
            "It walks the tree and writes what it finds.",
            base + Duration::from_secs(1),
        );
        assert!(app.panel().showing_thread());
        assert!(app.panel().has_thread());
        app
    }

    fn panel_cells() -> usize {
        usize::from(panel_width(SIZE))
    }

    fn asks(mouse: MouseEvent, app: &App) -> Option<MouseAction> {
        asks_under(
            mouse,
            app,
            QuitConfirm::Closed,
            &ScopePrompt::Closed,
            &RecordPrompt::Closed,
            &ScopePrompt::Closed,
        )
    }

    fn asks_under(
        mouse: MouseEvent,
        app: &App,
        confirm: QuitConfirm,
        prompt: &ScopePrompt,
        record: &RecordPrompt,
        write: &ScopePrompt,
    ) -> Option<MouseAction> {
        mouse_action(
            mouse,
            SIZE,
            app,
            confirm,
            &PushConfirm::Closed,
            &ScopePrompt::Closed,
            prompt,
            record,
            write,
            None,
        )
    }

    fn asks_composing(mouse: MouseEvent, app: &App, composer: &Composer) -> Option<MouseAction> {
        mouse_action(
            mouse,
            SIZE,
            app,
            QuitConfirm::Closed,
            &PushConfirm::Closed,
            &ScopePrompt::Closed,
            &ScopePrompt::Closed,
            &RecordPrompt::Closed,
            &ScopePrompt::Closed,
            Some(composer),
        )
    }

    fn round(app: &mut App, confirm: QuitConfirm, mouse: MouseEvent) {
        round_under(
            app,
            confirm,
            &ScopePrompt::Closed,
            &RecordPrompt::Closed,
            &ScopePrompt::Closed,
            mouse,
        );
    }

    fn round_under(
        app: &mut App,
        confirm: QuitConfirm,
        prompt: &ScopePrompt,
        record: &RecordPrompt,
        write: &ScopePrompt,
        mouse: MouseEvent,
    ) {
        match asks_under(mouse, app, confirm, prompt, record, write) {
            Some(MouseAction::SelectNextBy(rows)) => app.select_next_by(rows),
            Some(MouseAction::SelectPreviousBy(rows)) => app.select_previous_by(rows),
            Some(MouseAction::ScrollPanelDown(lines)) => app.scroll_panel_down(lines),
            Some(MouseAction::ScrollPanelUp(lines)) => app.scroll_panel_up(lines),
            Some(MouseAction::SelectRow(index)) => {
                app.set_focus(Focus::Tree);
                app.select_row(index);
            }
            Some(MouseAction::ToggleCollapsed) => {
                app.set_focus(Focus::Tree);
                app.toggle_collapsed();
            }
            Some(MouseAction::Focus(focus)) => app.set_focus(focus),
            // The loop's own arms for the gesture and no more, so what these
            // tests assert about an app after a press, a drag or a release
            // is what the binary would do with the same event.
            Some(MouseAction::StartSelection(_)) => app.set_focus(Focus::Panel),
            Some(
                MouseAction::ExtendSelection(_)
                | MouseAction::EndSelection(_)
                | MouseAction::ExtendPastEdge(_)
                | MouseAction::EndPastEdge(_),
            )
            | None => {}
        }
    }

    #[test]
    fn the_screen_these_tests_point_at_is_the_one_they_describe() {
        // The table above is load-bearing: every point below is a literal
        // read off it, so a layout that moved would otherwise turn these
        // tests into assertions about somewhere else.
        assert_eq!(viewport(), 18, "eighteen rows of tree at 80x24");
        assert_eq!(
            usize::from(panel_height(SIZE, None, None)),
            19,
            "nineteen lines of panel: no header of its own"
        );
    }

    #[test]
    fn a_notch_over_the_tree_moves_the_selection_three_rows() {
        let app = app_on_screen();

        assert_eq!(
            asks(wheel_down(IN_TREE, FIRST_TREE_ROW + 4), &app),
            Some(MouseAction::SelectNextBy(3)),
        );
        assert_eq!(
            asks(wheel_up(IN_TREE, FIRST_TREE_ROW + 4), &app),
            Some(MouseAction::SelectPreviousBy(3)),
        );
        // Every part of the pane's inside answers for the pane, the header
        // included: a wheel is aimed at a column, and a notch that did
        // nothing because the pointer sat on the naming line would read as a
        // wheel that sticks.
        assert_eq!(
            asks(wheel_down(IN_TREE, TREE_HEADER), &app),
            Some(MouseAction::SelectNextBy(3)),
        );
    }

    #[test]
    fn three_notched_rows_are_three_pressed_ones_and_stop_at_the_ends() {
        let mut app = app_on_screen();
        let mut pressed = app.clone();
        for _ in 0..3 {
            pressed.select_next();
        }

        round(
            &mut app,
            QuitConfirm::Closed,
            wheel_down(IN_TREE, FIRST_TREE_ROW),
        );
        assert_eq!(app, pressed, "a notch is three presses of the movement key");

        // Clamped at both ends rather than wrapping or running off: the
        // wheel is spun past the end far more easily than a key is held
        // there.
        for _ in 0..20 {
            round(
                &mut app,
                QuitConfirm::Closed,
                wheel_up(IN_TREE, FIRST_TREE_ROW),
            );
        }
        assert_eq!(app.selected(), 0, "stopped at the first row");
        for _ in 0..20 {
            round(
                &mut app,
                QuitConfirm::Closed,
                wheel_down(IN_TREE, FIRST_TREE_ROW),
            );
        }
        assert_eq!(app.selected(), app.rows().len() - 1, "stopped at the last");
    }

    #[test]
    fn a_notch_over_the_panel_scrolls_it_three_lines() {
        let app = app_on_screen();

        assert_eq!(
            asks(wheel_down(IN_PANEL, FIRST_PANEL_LINE + 7), &app),
            Some(MouseAction::ScrollPanelDown(3)),
        );
        assert_eq!(
            asks(wheel_up(IN_PANEL, FIRST_PANEL_LINE), &app),
            Some(MouseAction::ScrollPanelUp(3)),
        );
    }

    #[test]
    fn the_wheel_drives_the_pane_it_is_over_and_moves_no_focus() {
        // The keys are pointed at the panel and the pointer at the tree,
        // which is the case the convention is for: the wheel scrolls what
        // the reader is looking at, and a wheel that scrolled the focused
        // pane instead would move the half of the screen they are not.
        let mut app = app_on_screen();
        app.set_focus(Focus::Panel);
        round(
            &mut app,
            QuitConfirm::Closed,
            wheel_down(IN_TREE, FIRST_TREE_ROW + 2),
        );

        assert_eq!(app.selected(), 3, "the tree moved under the pointer");
        assert_eq!(app.focus(), Focus::Panel, "the keys did not follow");

        // And the other way round: the tree has the keys, the pointer is
        // over the panel, and the notch is the panel's.
        let mut app = app_on_screen();
        let selected = app.selected();
        round(
            &mut app,
            QuitConfirm::Closed,
            wheel_up(IN_PANEL, FIRST_PANEL_LINE),
        );

        assert_eq!(app.focus(), Focus::Tree, "the keys did not follow");
        assert_eq!(app.selected(), selected, "the tree did not move");
    }

    #[test]
    fn a_notch_over_the_footer_or_a_border_does_nothing() {
        let app = app_on_screen();
        for (column, row) in NOWHERE {
            assert_eq!(
                asks(wheel_down(column, row), &app),
                None,
                "a notch at {column},{row} should change nothing"
            );
            assert_eq!(
                asks(wheel_up(column, row), &app),
                None,
                "a notch at {column},{row} should change nothing"
            );
        }
    }

    #[test]
    fn a_click_on_a_row_selects_it_and_takes_the_keys() {
        let mut app = app_on_screen();
        app.set_focus(Focus::Panel);

        assert_eq!(
            asks(left_click(IN_TREE, FIRST_TREE_ROW + 5), &app),
            Some(MouseAction::SelectRow(5)),
            "the sixth row of a window that has not scrolled"
        );

        round(
            &mut app,
            QuitConfirm::Closed,
            left_click(IN_TREE, FIRST_TREE_ROW + 5),
        );
        assert_eq!(app.selected(), 5);
        assert_eq!(app.focus(), Focus::Tree, "the reader pointed at the tree");
    }

    #[test]
    fn a_click_names_a_row_of_the_tree_and_not_of_the_window() {
        // The window is scrolled to the bottom, so the offset the hit test
        // hands over is short of the row by exactly where the window starts.
        let mut app = app_on_screen();
        app.select_last();
        let offset = app.scroll_offset();
        assert_eq!(
            offset,
            app.rows().len() - viewport(),
            "the window is at the end"
        );

        assert_eq!(
            asks(left_click(IN_TREE, FIRST_TREE_ROW + 3), &app),
            Some(MouseAction::SelectRow(offset + 3)),
        );
    }

    #[test]
    fn a_second_click_on_a_directory_row_opens_and_closes_it() {
        let mut app = app_on_screen();
        // Files shown, so the directory clicked has a row under it to hide.
        // Without them it holds nothing on screen and the collapse refuses,
        // which is what the test below this one is about.
        app.toggle_files();
        // The row under the pointer is selected first, by a click of its
        // own: the second click is the one that collapses, and it is the
        // same point twice.
        let point = left_click(IN_TREE, FIRST_TREE_ROW + 1);
        round(&mut app, QuitConfirm::Closed, point);
        let path = app.selected_row().expect("a row is selected").path.clone();
        assert!(!app.is_collapsed(&path), "nothing collapsed by selecting");

        assert_eq!(asks(point, &app), Some(MouseAction::ToggleCollapsed),);
        round(&mut app, QuitConfirm::Closed, point);
        assert!(app.is_collapsed(&path), "the second click closed it");

        // And back open, which is what space does on the third press too.
        round(&mut app, QuitConfirm::Closed, point);
        assert!(!app.is_collapsed(&path), "the third click opened it");
    }

    #[test]
    fn a_second_click_on_a_file_row_does_nothing_more() {
        // Files shown, so a file row can be pointed at. It is a row like any
        // other to the hit test — what refuses it is the collapse itself,
        // which is exactly what refuses space on the same row.
        let mut app = app_on_screen();
        app.toggle_files();
        let point = left_click(IN_TREE, FIRST_TREE_ROW + 2);
        round(&mut app, QuitConfirm::Closed, point);
        assert!(
            app.selected_row().expect("a row is selected").is_file(),
            "the third drawn row is a file"
        );

        let before = app.clone();
        round(&mut app, QuitConfirm::Closed, point);
        assert_eq!(app, before, "a file row has nothing to open");
    }

    #[test]
    fn a_click_in_the_panel_takes_the_keys_and_no_more() {
        let mut app = app_on_screen();
        let before = app.clone();

        assert_eq!(
            asks(left_click(IN_PANEL, FIRST_PANEL_LINE + 9), &app),
            Some(MouseAction::Focus(Focus::Panel)),
            "the panel has no selection, so focus is the whole of it"
        );

        round(
            &mut app,
            QuitConfirm::Closed,
            left_click(IN_PANEL, FIRST_PANEL_LINE + 9),
        );
        assert_eq!(app.focus(), Focus::Panel);
        assert_eq!(app.selected(), before.selected(), "the tree did not move");
        assert_eq!(
            app.panel().scroll_offset(),
            before.panel().scroll_offset(),
            "the panel's window did not move either"
        );
    }

    #[test]
    fn a_press_on_the_conversation_anchors_a_selection_and_still_takes_the_keys() {
        let mut app = app_talking();
        let before = app.clone();

        // Column nine, row two of the card: the pane's border is column zero
        // and row zero of the screen, and neither is a cell of the rows area
        // the text was drawn into.
        assert_eq!(
            asks(left_click(IN_PANEL, FIRST_PANEL_LINE + 2), &app),
            Some(MouseAction::StartSelection(Cell {
                column: 9,
                row: 2,
                scroll: 0,
                width: panel_cells(),
            })),
        );

        round(
            &mut app,
            QuitConfirm::Closed,
            left_click(IN_PANEL, FIRST_PANEL_LINE + 2),
        );
        assert_eq!(
            app.focus(),
            Focus::Panel,
            "a press on the conversation still points the keys at it"
        );
        assert_eq!(app.selected(), before.selected(), "the tree did not move");
        assert_eq!(
            app.panel().scroll_offset(),
            before.panel().scroll_offset(),
            "the panel's window did not move either"
        );
    }

    #[test]
    fn a_drag_extends_the_selection_and_the_release_ends_it() {
        let app = app_talking();

        // The three halves of one gesture over one card, each carrying the
        // cell it landed on: the drag is what the press is anchored for, and
        // the release is read for the drag it ends.
        assert_eq!(
            asks(left_click(IN_PANEL, FIRST_PANEL_LINE), &app),
            Some(MouseAction::StartSelection(Cell {
                column: 9,
                row: 0,
                scroll: 0,
                width: panel_cells(),
            })),
        );
        assert_eq!(
            asks(drag(IN_PANEL + 4, FIRST_PANEL_LINE + 1), &app),
            Some(MouseAction::ExtendSelection(Cell {
                column: 13,
                row: 1,
                scroll: 0,
                width: panel_cells(),
            })),
        );
        assert_eq!(
            asks(release(IN_PANEL + 4, FIRST_PANEL_LINE + 1), &app),
            Some(MouseAction::EndSelection(Cell {
                column: 13,
                row: 1,
                scroll: 0,
                width: panel_cells(),
            })),
        );
    }

    #[test]
    fn the_cell_a_gesture_carries_is_the_panel_s_own_window() {
        // The panel's scroll and width rather than a second measurement of
        // the frame: the row under the pointer is the row the card's own
        // window put there, and a cell measured against another window names
        // a character the reader is not pointing at. Scrolled back far enough
        // that the offset is a number rather than zero, which is what a
        // second measurement would answer.
        let mut app = app_talking();
        let base = Instant::now();
        for turn in 0..12 {
            app.panel_mut()
                .start_turn(format!("turn {turn}"), base + Duration::from_secs(turn));
            app.panel_mut().answer_turn(
                format!("answer {turn}"),
                base + Duration::from_secs(turn + 1),
            );
        }
        app.scroll_panel_up(4);
        let scroll = app.panel().scroll_offset();
        assert_ne!(scroll, 0, "the window is off the top of the card");

        for mouse in [
            left_click(IN_PANEL, FIRST_PANEL_LINE + 3),
            drag(IN_PANEL, FIRST_PANEL_LINE + 3),
            release(IN_PANEL, FIRST_PANEL_LINE + 3),
        ] {
            let carried = match asks(mouse, &app) {
                Some(
                    MouseAction::StartSelection(cell)
                    | MouseAction::ExtendSelection(cell)
                    | MouseAction::EndSelection(cell),
                ) => cell,
                other => panic!("{mouse:?} should be part of the gesture, not {other:?}"),
            };

            assert_eq!(
                carried,
                Cell {
                    column: 9,
                    row: 3,
                    scroll,
                    width: panel_cells(),
                }
            );
        }
    }

    #[test]
    fn a_thread_card_with_nothing_on_it_takes_no_selection() {
        // A press on an empty conversation is the focus it has always been:
        // there is no text under the pointer to anchor in, and a drag across
        // one selects nothing either.
        let app = app_on_screen();
        assert!(app.panel().showing_thread());
        assert!(!app.panel().has_thread());

        assert_eq!(
            asks(left_click(IN_PANEL, FIRST_PANEL_LINE + 2), &app),
            Some(MouseAction::Focus(Focus::Panel)),
        );
        assert_eq!(asks(drag(IN_PANEL, FIRST_PANEL_LINE + 2), &app), None);
        assert_eq!(asks(release(IN_PANEL, FIRST_PANEL_LINE + 2), &app), None);
    }

    #[test]
    fn the_other_two_cards_take_no_selection_either() {
        // Selection is the conversation's alone: the account is a record of a
        // run and the document is a file that was read, and neither is
        // highlighted by this gesture. The conversation underneath is the one
        // the fixture recorded, so this is the same app that *would* answer
        // with the other card showing.
        fn plain_focus(app: &App) {
            assert!(!app.panel().showing_thread());
            assert_eq!(
                asks(left_click(IN_PANEL, FIRST_PANEL_LINE + 2), app),
                Some(MouseAction::Focus(Focus::Panel)),
                "a press on this card is the focus it always was"
            );
            assert_eq!(asks(drag(IN_PANEL, FIRST_PANEL_LINE + 2), app), None);
            assert_eq!(asks(release(IN_PANEL, FIRST_PANEL_LINE + 2), app), None);
        }

        let mut app = app_talking();
        app.start_account(Instant::now());
        app.show_document(["a line of a file that was read"], false);

        // The file, which is the card `show_document` leaves showing, then
        // round past the conversation to the account.
        plain_focus(&app);
        app.swap_card();
        assert!(app.panel().showing_thread());
        app.swap_card();
        plain_focus(&app);
    }

    #[test]
    fn a_gesture_anywhere_but_the_conversation_means_what_it_meant_before() {
        // Every other place the pointer can be, over an app whose card would
        // answer a gesture if the pointer were on it. A press goes on meaning
        // what it means everywhere; a drag means nothing anywhere it is still
        // level with the card's rows, however far sideways it has gone —
        // sideways is not a direction to scroll in, so the border, the tree
        // and the column between them all answer as they did.
        //
        // The four points that do answer are past an edge, and the answer is
        // which edge and how far: the last row of the card is 19, so row 20
        // is one below it and the footer at 22 is three, and row 0 is one
        // above the first. The column comes back clamped into the rows, which
        // is why a point over the tree column answers with the card's last.
        let app = app_talking();
        let last = u16::try_from(panel_cells() - 1).expect("the card is a few columns wide");
        for (column, row, pressed, past) in [
            (
                IN_TREE,
                TREE_HEADER,
                Some(MouseAction::Focus(Focus::Tree)),
                None,
            ),
            (
                IN_TREE,
                FIRST_TREE_ROW,
                Some(MouseAction::ToggleCollapsed),
                None,
            ),
            (
                IN_TREE,
                FIRST_TREE_ROW + 4,
                Some(MouseAction::SelectRow(4)),
                None,
            ),
            (
                IN_PANEL,
                FOOTER,
                None,
                Some(Reach::Below {
                    rows: 3,
                    column: IN_PANEL - 1,
                }),
            ),
            (
                IN_TREE,
                FOOTER,
                None,
                Some(Reach::Below {
                    rows: 3,
                    column: last,
                }),
            ),
            (0, FIRST_PANEL_LINE, None, None),
            (49, FIRST_TREE_ROW, None, None),
            (50, FIRST_TREE_ROW, None, None),
            (
                IN_TREE,
                0,
                None,
                Some(Reach::Above {
                    rows: 1,
                    column: last,
                }),
            ),
            (
                IN_PANEL,
                20,
                None,
                Some(Reach::Below {
                    rows: 1,
                    column: IN_PANEL - 1,
                }),
            ),
        ] {
            assert_eq!(
                asks(left_click(column, row), &app),
                pressed,
                "a press at {column},{row} changed meaning"
            );
            assert_eq!(
                asks(drag(column, row), &app),
                past.map(MouseAction::ExtendPastEdge),
                "a drag at {column},{row} changed meaning"
            );
            assert_eq!(
                asks(release(column, row), &app),
                past.map(MouseAction::EndPastEdge),
                "a release at {column},{row} changed meaning"
            );
        }
    }

    #[test]
    fn a_drag_held_past_an_edge_says_how_many_rows_past_it_the_pointer_is() {
        // The number a scroll off the tick is geared by, so it is counted
        // from the edge rather than from anywhere else: the row beside the
        // first line of the card is one above it, and the row beside the last
        // is one below.
        let app = app_talking();
        let first = FIRST_PANEL_LINE;
        let last = FIRST_PANEL_LINE + panel_height(SIZE, None, None) - 1;
        assert_eq!(last, 19, "nineteen lines of card, ending at row 19");

        for (row, past) in [
            (0, Reach::Above { rows: 1, column: 9 }),
            (last + 1, Reach::Below { rows: 1, column: 9 }),
            (last + 2, Reach::Below { rows: 2, column: 9 }),
            (SIZE.height - 1, Reach::Below { rows: 4, column: 9 }),
        ] {
            assert_eq!(
                asks(drag(IN_PANEL, row), &app),
                Some(MouseAction::ExtendPastEdge(past)),
                "a drag at row {row}"
            );
            assert_eq!(
                asks(release(IN_PANEL, row), &app),
                Some(MouseAction::EndPastEdge(past)),
                "a release at row {row} should end the gesture, not vanish"
            );
        }

        // And the two rows either side of that pair are the card itself,
        // which still answers with the cell under the pointer.
        for row in [first, last] {
            assert!(
                matches!(
                    asks(drag(IN_PANEL, row), &app),
                    Some(MouseAction::ExtendSelection(_))
                ),
                "row {row} is a line of the card"
            );
            assert!(
                matches!(
                    asks(release(IN_PANEL, row), &app),
                    Some(MouseAction::EndSelection(_))
                ),
                "row {row} is a line of the card"
            );
        }
    }

    #[test]
    fn a_drag_past_the_edge_of_a_card_with_no_text_on_it_still_means_nothing() {
        // The refusal `cell_under` makes is the refusal the edge makes: the
        // account and the document are read past rather than copied out of,
        // and an empty thread card has nothing to scroll a selection through.
        let app = app_on_screen();
        assert!(app.panel().showing_thread() && !app.panel().has_thread());
        for row in [0, 20, FOOTER] {
            assert_eq!(asks(drag(IN_PANEL, row), &app), None, "at row {row}");
            assert_eq!(asks(release(IN_PANEL, row), &app), None, "at row {row}");
        }

        let mut app = app_talking();
        app.start_account(Instant::now());
        app.show_document(["a line of a file that was read"], false);
        for card in 0..2 {
            assert!(!app.panel().showing_thread(), "card {card}");
            for row in [0, 20, FOOTER] {
                assert_eq!(
                    asks(drag(IN_PANEL, row), &app),
                    None,
                    "card {card}, row {row}"
                );
                assert_eq!(
                    asks(release(IN_PANEL, row), &app),
                    None,
                    "card {card}, row {row}"
                );
            }
            // The file first, which is the card `show_document` leaves
            // showing, then round past the conversation to the account.
            app.swap_card();
            app.swap_card();
        }
    }

    #[test]
    fn nothing_but_a_held_left_button_answers_for_a_point_past_the_edge() {
        // The wheel over the footer and the border is still nothing, a press
        // out there is still nothing, and the other buttons are still read and
        // dropped: only the two halves of a held drag learned a new answer.
        let app = app_talking();
        for row in [0, 20, FOOTER] {
            for mouse in [
                wheel_down(IN_PANEL, row),
                wheel_up(IN_PANEL, row),
                left_click(IN_PANEL, row),
                event(MouseEventKind::Moved, IN_PANEL, row),
                event(MouseEventKind::Drag(MouseButton::Right), IN_PANEL, row),
                event(MouseEventKind::Up(MouseButton::Middle), IN_PANEL, row),
            ] {
                assert_eq!(asks(mouse, &app), None, "{mouse:?} at row {row}");
            }
        }

        // The wheel over the tree keeps both panes' notches, which a point
        // level with the card's rows is what it always was.
        assert_eq!(
            asks(wheel_down(IN_TREE, FIRST_TREE_ROW), &app),
            Some(MouseAction::SelectNextBy(WHEEL_NOTCH)),
        );
        assert_eq!(
            asks(wheel_up(IN_PANEL, FIRST_PANEL_LINE), &app),
            Some(MouseAction::ScrollPanelUp(WHEEL_NOTCH)),
        );
    }

    #[test]
    fn a_press_on_the_composer_still_points_the_keyboard_at_it() {
        // The field takes rows from the panel, so the hit test is handed the
        // draft the frame was drawn with; a press on it points the keyboard
        // at the field.
        //
        // A drag is the other half of this: the field sits under the card, so
        // a pointer dragged down onto it is a pointer past the card's bottom
        // edge and is answered as one. Anything else would stop the drag dead
        // at the first row the reader drags onto.
        let mut app = app_talking();
        let composer = Composer::default();
        let rows = panel_height(SIZE, Some(&composer), None);
        app.panel_mut().set_height(rows);
        // The panel's rows start at `FIRST_PANEL_LINE`, so its bottom border
        // is the row after the last of them and the field's own top border
        // the row after that: the line to type on is the next one down.
        let field = FIRST_PANEL_LINE + rows + 2;
        let past = Reach::Below {
            rows: 3,
            column: IN_PANEL - 1,
        };

        assert_eq!(
            asks_composing(left_click(IN_PANEL, field), &app, &composer),
            Some(MouseAction::Focus(Focus::Composer)),
            "the field takes the bottom of the panel's column"
        );
        assert_eq!(
            asks_composing(drag(IN_PANEL, field), &app, &composer),
            Some(MouseAction::ExtendPastEdge(past)),
        );
        assert_eq!(
            asks_composing(release(IN_PANEL, field), &app, &composer),
            Some(MouseAction::EndPastEdge(past)),
        );

        // The edge the field moved: the card's own last row is still the card,
        // and the border under it is one row past rather than three.
        assert!(matches!(
            asks_composing(drag(IN_PANEL, FIRST_PANEL_LINE + rows - 1), &app, &composer),
            Some(MouseAction::ExtendSelection(_))
        ));
        assert_eq!(
            asks_composing(drag(IN_PANEL, FIRST_PANEL_LINE + rows), &app, &composer),
            Some(MouseAction::ExtendPastEdge(Reach::Below {
                rows: 1,
                column: IN_PANEL - 1,
            })),
        );
    }

    #[test]
    fn a_press_on_the_run_header_still_takes_the_keys_and_no_more() {
        // The header takes the top of the panel while a run is out, so the
        // rows of the card move down under it: a press on the header is the
        // focus a press in the panel has always been, and it anchors nothing
        // because the header is not the reader's text.
        //
        // A drag onto it is a drag above the card's rows — two rows above,
        // since the header is that tall — for the reason a drag onto the
        // composer is a drag below them.
        let mut app = app_talking();
        app.set_pact_in_flight("/repo/d00", 0, 24);
        let header = app.run_header();
        app.panel_mut()
            .set_height(panel_height(SIZE, None, header.as_ref()));
        let past = Reach::Above {
            rows: 2,
            column: IN_PANEL - 1,
        };

        assert_eq!(
            asks(left_click(IN_PANEL, FIRST_PANEL_LINE), &app),
            Some(MouseAction::Focus(Focus::Panel)),
        );
        assert_eq!(
            asks(drag(IN_PANEL, FIRST_PANEL_LINE), &app),
            Some(MouseAction::ExtendPastEdge(past)),
        );
        assert_eq!(
            asks(release(IN_PANEL, FIRST_PANEL_LINE), &app),
            Some(MouseAction::EndPastEdge(past)),
        );

        // And the first row of the card, which the header has pushed two rows
        // down: it is row zero of the rows area all the same, because the
        // cell is counted from where the card was drawn.
        assert_eq!(
            asks(left_click(IN_PANEL, FIRST_PANEL_LINE + 2), &app),
            Some(MouseAction::StartSelection(Cell {
                column: 9,
                row: 0,
                scroll: 0,
                width: panel_cells(),
            })),
        );
    }

    #[test]
    fn only_the_left_button_selects() {
        // Over a conversation, where the left button's drag and release do
        // mean something: the other two buttons and a hover mean nothing
        // there either, which is what keeps a pointer swept across the card
        // from costing a redraw per move.
        let app = app_talking();
        for kind in [
            MouseEventKind::Moved,
            MouseEventKind::Drag(MouseButton::Right),
            MouseEventKind::Drag(MouseButton::Middle),
            MouseEventKind::Up(MouseButton::Right),
            MouseEventKind::Up(MouseButton::Middle),
            MouseEventKind::Down(MouseButton::Right),
            MouseEventKind::Down(MouseButton::Middle),
            MouseEventKind::ScrollLeft,
            MouseEventKind::ScrollRight,
        ] {
            assert_eq!(
                asks(event(kind, IN_PANEL, FIRST_PANEL_LINE + 2), &app),
                None,
                "{kind:?} over the conversation should mean nothing"
            );
        }
    }

    #[test]
    fn a_window_swallows_the_gesture_as_it_swallows_everything_else() {
        // Both dialogs and all three prompts are answered from the keyboard,
        // and a gesture that got through one would drag a highlight across a
        // card the reader cannot see, behind a window they are in the middle of
        // answering. Over a conversation, which is the one card where the three
        // halves would otherwise mean something.
        let app = app_talking();
        let open = ScopePrompt::open("crates/warlock-engine", "data-plane");
        let recording = RecordPrompt::open("crates/warlock-engine", "data-plane");
        for (confirm, scope, record, write) in [
            (
                QuitConfirm::open(),
                &ScopePrompt::Closed,
                &RecordPrompt::Closed,
                &ScopePrompt::Closed,
            ),
            (
                QuitConfirm::Closed,
                &open,
                &RecordPrompt::Closed,
                &ScopePrompt::Closed,
            ),
            (
                QuitConfirm::Closed,
                &ScopePrompt::Closed,
                &recording,
                &ScopePrompt::Closed,
            ),
            (
                QuitConfirm::Closed,
                &ScopePrompt::Closed,
                &RecordPrompt::Closed,
                &open,
            ),
        ] {
            for mouse in [
                left_click(IN_PANEL, FIRST_PANEL_LINE + 2),
                drag(IN_PANEL + 3, FIRST_PANEL_LINE + 3),
                release(IN_PANEL + 3, FIRST_PANEL_LINE + 3),
            ] {
                assert_eq!(
                    asks_under(mouse, &app, confirm, scope, record, write),
                    None,
                    "{mouse:?} should mean nothing while a window is up"
                );
            }
        }

        // And the fifth window, which `asks_under` above cannot put up because
        // every other test in here has it down.
        let push = PushConfirm::open(
            "Push a brief to the board",
            "warlock-team",
            "Warlock",
            "work",
        );
        for mouse in [
            left_click(IN_PANEL, FIRST_PANEL_LINE + 2),
            drag(IN_PANEL + 3, FIRST_PANEL_LINE + 3),
            release(IN_PANEL + 3, FIRST_PANEL_LINE + 3),
            wheel_down(IN_TREE, FIRST_TREE_ROW),
        ] {
            assert_eq!(
                mouse_action(
                    mouse,
                    SIZE,
                    &app,
                    QuitConfirm::Closed,
                    &push,
                    &ScopePrompt::Closed,
                    &ScopePrompt::Closed,
                    &RecordPrompt::Closed,
                    &ScopePrompt::Closed,
                    None,
                ),
                None,
                "{mouse:?} should mean nothing while the push dialog is up"
            );
        }
    }

    #[test]
    fn a_click_on_the_tree_header_takes_the_keys_and_no_more() {
        let mut app = app_on_screen();
        app.set_focus(Focus::Panel);
        let selected = app.selected();

        assert_eq!(
            asks(left_click(IN_TREE, TREE_HEADER), &app),
            Some(MouseAction::Focus(Focus::Tree)),
        );

        round(
            &mut app,
            QuitConfirm::Closed,
            left_click(IN_TREE, TREE_HEADER),
        );
        assert_eq!(app.focus(), Focus::Tree);
        assert_eq!(app.selected(), selected, "the selection did not move");
    }

    #[test]
    fn a_click_below_the_last_row_selects_nothing() {
        // A window taller than the tree in it: one row drawn and seventeen
        // rows of blank pane under it, which is a click in the pane and no
        // more. The app is asked rather than the layout, because only the
        // app knows how many rows it has.
        let mut app = App::from_rows(vec![Row::new(
            0,
            "/repo",
            "/repo/WARLOCK.md",
            NodeState::PactedStale,
        )]);
        app.set_viewport_height(tree_height(SIZE));
        app.set_focus(Focus::Panel);

        assert_eq!(
            asks(left_click(IN_TREE, FIRST_TREE_ROW + 6), &app),
            Some(MouseAction::Focus(Focus::Tree)),
        );

        let before = app.clone();
        round(
            &mut app,
            QuitConfirm::Closed,
            left_click(IN_TREE, FIRST_TREE_ROW + 6),
        );
        assert_eq!(app.focus(), Focus::Tree);
        assert_eq!(app.rows(), before.rows(), "nothing was opened or closed");
        assert_eq!(app.selected(), 0, "the one row stayed selected");
    }

    #[test]
    fn a_click_on_the_footer_or_a_border_does_nothing_at_all() {
        let app = app_on_screen();
        for (column, row) in NOWHERE {
            assert_eq!(
                asks(left_click(column, row), &app),
                None,
                "a click at {column},{row} should change nothing"
            );
        }
    }

    #[test]
    fn everything_but_the_wheel_and_the_left_press_is_read_and_dropped() {
        let app = app_on_screen();
        // Out of scope by decision: hovering, the other two buttons and the
        // horizontal wheel. The left button's own drag and release are in
        // the list too, because this app's thread card has nothing recorded
        // on it: with no text under the pointer there is nothing to select,
        // and they mean here what everything else here means. Asked at every
        // kind of point, because dropping them is what keeps a pointer swept
        // across the screen from costing anything — a highlight that
        // followed it would cost a redraw per move to say what the selection
        // already says.
        for kind in [
            MouseEventKind::Moved,
            MouseEventKind::Drag(MouseButton::Left),
            MouseEventKind::Drag(MouseButton::Right),
            MouseEventKind::Up(MouseButton::Left),
            MouseEventKind::Up(MouseButton::Right),
            MouseEventKind::Down(MouseButton::Right),
            MouseEventKind::Down(MouseButton::Middle),
            MouseEventKind::Up(MouseButton::Middle),
            MouseEventKind::ScrollLeft,
            MouseEventKind::ScrollRight,
        ] {
            for (column, row) in [
                (IN_TREE, FIRST_TREE_ROW),
                (IN_TREE, TREE_HEADER),
                (IN_PANEL, FIRST_PANEL_LINE),
                (IN_PANEL, FOOTER),
                (50, FIRST_TREE_ROW),
            ] {
                assert_eq!(
                    asks(event(kind, column, row), &app),
                    None,
                    "{kind:?} at {column},{row} should mean nothing"
                );
            }
        }
    }

    #[test]
    fn the_pointer_is_read_and_dropped_while_the_confirmation_is_up() {
        // The dialog is answered from the keyboard and has no clickable Yes
        // or No, so a click that got through would land on a tree the
        // reader cannot see, behind a window that is about to close. Asked
        // over the whole pointer — both notches, a click on a row, a click
        // on the row already selected and a click in the panel — and then
        // asserted about the app itself, since "read and dropped" is a
        // claim about what did not move.
        let mut app = app_on_screen();
        app.toggle_files();
        // Selected and focused somewhere other than where it started, so a
        // leak has something to disturb: the panel has the keys and its
        // window has been scrolled back, and the tree's selection is a row
        // down the list rather than the first one.
        app.select_row(9);
        app.scroll_panel_down(4);
        app.set_focus(Focus::Panel);
        let before = app.clone();
        assert_eq!(
            app.scroll_offset(),
            0,
            "the tree's window has not moved, so drawn row nine is row nine"
        );

        for mouse in whole_pointer() {
            assert_eq!(
                asks_under(
                    mouse,
                    &app,
                    QuitConfirm::open(),
                    &ScopePrompt::Closed,
                    &RecordPrompt::Closed,
                    &ScopePrompt::Closed
                ),
                None,
                "{mouse:?} should mean nothing while the question is up"
            );
            round(&mut app, QuitConfirm::open(), mouse);
        }

        assert_eq!(app, before, "the pointer moved nothing behind the dialog");
    }

    #[test]
    fn the_pointer_is_read_and_dropped_while_the_scope_prompt_is_up() {
        // The same rule as the confirmation above, for the same reasons:
        // the prompt is typed into and has no buttons, and a click that got
        // through would move a selection under a window the reader is in
        // the middle of answering. The whole pointer again — both notches,
        // a click on a row, a click on the row already selected, a click on
        // the header and a click in the panel — with the app asserted
        // afterwards, since "read and dropped" is a claim about what did
        // not move.
        let mut app = app_on_screen();
        app.toggle_files();
        app.select_row(9);
        app.scroll_panel_down(4);
        app.set_focus(Focus::Panel);
        let before = app.clone();
        assert_eq!(
            app.scroll_offset(),
            0,
            "the tree's window has not moved, so drawn row nine is row nine"
        );

        // Both an empty field and one with something typed into it: the
        // gate is the prompt being up, not what is in it.
        for prompt in [
            ScopePrompt::open("crates/warlock-engine", ""),
            ScopePrompt::open("crates/warlock-engine", "data-plane"),
        ] {
            for mouse in whole_pointer() {
                assert_eq!(
                    asks_under(
                        mouse,
                        &app,
                        QuitConfirm::Closed,
                        &prompt,
                        &RecordPrompt::Closed,
                        &ScopePrompt::Closed
                    ),
                    None,
                    "{mouse:?} should mean nothing while the prompt is up"
                );
                round_under(
                    &mut app,
                    QuitConfirm::Closed,
                    &prompt,
                    &RecordPrompt::Closed,
                    &ScopePrompt::Closed,
                    mouse,
                );
            }
        }

        assert_eq!(app, before, "the pointer moved nothing behind the prompt");
    }

    #[test]
    fn the_pointer_is_read_and_dropped_while_the_write_prompt_is_up() {
        // The third window and the same rule: a path being typed has no
        // buttons either, and a click on the tree behind it would select a
        // row the reader cannot see under a window that is about to close.
        let mut app = app_on_screen();
        app.select_row(3);
        let before = app.clone();
        let write = ScopePrompt::open("Write the brief to", "docs/warlock-brief-13-x.md");

        for mouse in whole_pointer() {
            assert_eq!(
                asks_under(
                    mouse,
                    &app,
                    QuitConfirm::Closed,
                    &ScopePrompt::Closed,
                    &RecordPrompt::Closed,
                    &write
                ),
                None,
                "{mouse:?} should mean nothing while the path prompt is up"
            );
            round_under(
                &mut app,
                QuitConfirm::Closed,
                &ScopePrompt::Closed,
                &RecordPrompt::Closed,
                &write,
                mouse,
            );
        }

        assert_eq!(app, before, "the pointer moved nothing behind the prompt");
    }

    #[test]
    fn the_pointer_is_read_and_dropped_while_the_record_window_is_up() {
        // The fourth window and the same rule once more: three fields being
        // typed into, no buttons in any of them, and a tree underneath that a
        // click would silently move while the reader is filling a record in.
        let mut app = app_on_screen();
        app.toggle_files();
        app.select_row(9);
        app.scroll_panel_down(4);
        app.set_focus(Focus::Panel);
        let before = app.clone();
        let record = RecordPrompt::open("crates/warlock-engine", "data-plane");

        for mouse in whole_pointer() {
            assert_eq!(
                asks_under(
                    mouse,
                    &app,
                    QuitConfirm::Closed,
                    &ScopePrompt::Closed,
                    &record,
                    &ScopePrompt::Closed
                ),
                None,
                "{mouse:?} should mean nothing while the record window is up"
            );
            round_under(
                &mut app,
                QuitConfirm::Closed,
                &ScopePrompt::Closed,
                &record,
                &ScopePrompt::Closed,
                mouse,
            );
        }

        assert_eq!(app, before, "the pointer moved nothing behind the window");
    }
}

// What the session keeps between one pointer event and the next, which is
// the only thing the loop's tick has to go on.
mod holding {
    use ratatui::crossterm::event::{MouseButton, MouseEventKind};
    use warlock_tui::{Cell, Reach};

    use super::super::{Drag, MouseAction, drag_after};

    const PRESS: MouseEventKind = MouseEventKind::Down(MouseButton::Left);
    const DRAG: MouseEventKind = MouseEventKind::Drag(MouseButton::Left);
    const RELEASE: MouseEventKind = MouseEventKind::Up(MouseButton::Left);
    const WHEEL: MouseEventKind = MouseEventKind::ScrollDown;

    const CELL: Cell = Cell {
        column: 4,
        row: 2,
        scroll: 0,
        width: 40,
    };

    const PAST: Reach = Reach::Below { rows: 3, column: 4 };

    fn holding() -> Option<Drag> {
        drag_after(None, PRESS, Some(MouseAction::StartSelection(CELL)))
    }

    #[test]
    fn a_press_on_the_conversation_is_the_only_one_that_starts_a_drag() {
        assert_eq!(holding(), Some(Drag { past: None }));
        for action in [
            Some(MouseAction::Focus(super::super::Focus::Panel)),
            Some(MouseAction::SelectRow(2)),
            Some(MouseAction::ToggleCollapsed),
            // The footer, the border, off the screen: a press that reads as
            // nothing at all.
            None,
        ] {
            assert_eq!(
                drag_after(None, PRESS, action),
                None,
                "{action:?} started a drag over the conversation"
            );
            assert_eq!(
                drag_after(holding(), PRESS, action),
                None,
                "{action:?} left a drag standing that it did not start"
            );
        }
    }

    #[test]
    fn the_last_drag_event_past_the_edge_is_what_is_kept() {
        assert_eq!(
            drag_after(holding(), DRAG, Some(MouseAction::ExtendPastEdge(PAST))),
            Some(Drag { past: Some(PAST) })
        );
        let further = Reach::Above { rows: 9, column: 0 };
        assert_eq!(
            drag_after(
                Some(Drag { past: Some(PAST) }),
                DRAG,
                Some(MouseAction::ExtendPastEdge(further))
            ),
            Some(Drag {
                past: Some(further)
            }),
            "a drag back past the other edge was read against the first one"
        );
        assert_eq!(
            drag_after(
                Some(Drag { past: Some(PAST) }),
                DRAG,
                Some(MouseAction::ExtendSelection(CELL))
            ),
            Some(Drag { past: None }),
            "a pointer back level with the rows went on scrolling"
        );
    }

    #[test]
    fn a_drag_past_the_edge_with_no_press_behind_it_is_not_a_drag() {
        // The press landed in the tree, the composer or on another card, and
        // the pointer has since been dragged off the panel's rows.
        for action in [
            MouseAction::ExtendPastEdge(PAST),
            MouseAction::ExtendSelection(CELL),
        ] {
            assert_eq!(drag_after(None, DRAG, Some(action)), None);
        }
    }

    #[test]
    fn the_button_coming_up_ends_it_wherever_the_pointer_is() {
        for (held, action) in [
            (holding(), Some(MouseAction::EndSelection(CELL))),
            (
                Some(Drag { past: Some(PAST) }),
                Some(MouseAction::EndPastEdge(PAST)),
            ),
            // Another card went up mid-drag: there is nothing of the
            // conversation under the pointer to end the drag on, and the
            // button is up all the same.
            (holding(), None),
        ] {
            assert_eq!(drag_after(held, RELEASE, action), None);
        }
    }

    #[test]
    fn the_wheel_does_not_let_go_of_a_button_somebody_is_holding() {
        let held = Some(Drag { past: Some(PAST) });
        assert_eq!(
            drag_after(held, WHEEL, Some(MouseAction::ScrollPanelDown(3))),
            held
        );
    }
}
