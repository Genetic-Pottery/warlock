use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};

use super::{Action, Pressed, action_for, press_for};
use crate::confirm::QuitConfirm;

fn press(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

#[test]
fn q_and_esc_quit() {
    assert_eq!(action_for(press(KeyCode::Char('q'))), Some(Action::Quit));
    assert_eq!(action_for(press(KeyCode::Esc)), Some(Action::Quit));
}

#[test]
fn ctrl_c_asks_to_quit_with_caps_lock_or_shift_held() {
    // Some terminals report Ctrl-C as an upper-case `C` when shift or caps
    // lock is in play; it is still the key everyone reaches for to get out.
    let ctrl_shift_c = KeyEvent::new(
        KeyCode::Char('C'),
        KeyModifiers::CONTROL | KeyModifiers::SHIFT,
    );

    assert_eq!(
        press_for(ctrl_shift_c, None, None, false),
        Pressed::Confirm(QuitConfirm::open())
    );
}

#[test]
fn tab_moves_the_keys_to_the_other_pane() {
    assert_eq!(action_for(press(KeyCode::Tab)), Some(Action::ToggleFocus));
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
            action_for(event),
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
    ] {
        assert_ne!(
            action_for(press(code)),
            Some(Action::ToggleFocus),
            "{code:?} should not move focus"
        );
    }
}

#[test]
fn up_and_k_move_the_selection_up() {
    assert_eq!(action_for(press(KeyCode::Up)), Some(Action::SelectPrevious));
    assert_eq!(
        action_for(press(KeyCode::Char('k'))),
        Some(Action::SelectPrevious)
    );
}

#[test]
fn down_and_j_move_the_selection_down() {
    assert_eq!(action_for(press(KeyCode::Down)), Some(Action::SelectNext));
    assert_eq!(
        action_for(press(KeyCode::Char('j'))),
        Some(Action::SelectNext)
    );
}

#[test]
fn page_up_and_page_down_move_the_selection_by_a_screenful() {
    assert_eq!(
        action_for(press(KeyCode::PageUp)),
        Some(Action::SelectPageUp)
    );
    assert_eq!(
        action_for(press(KeyCode::PageDown)),
        Some(Action::SelectPageDown)
    );
}

#[test]
fn lower_g_jumps_to_the_first_row_and_upper_g_to_the_last() {
    assert_eq!(
        action_for(press(KeyCode::Char('g'))),
        Some(Action::SelectFirst)
    );
    assert_eq!(
        action_for(press(KeyCode::Char('G'))),
        Some(Action::SelectLast)
    );
}

#[test]
fn upper_g_still_jumps_to_the_last_row_with_shift_reported() {
    // Terminals disagree about whether the modifier rides along with the
    // upper-case letter; both spellings are the same keystroke.
    let shift_g = KeyEvent::new(KeyCode::Char('G'), KeyModifiers::SHIFT);

    assert_eq!(action_for(shift_g), Some(Action::SelectLast));
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
                action_for(event),
                None,
                "{kind:?} of {code:?} should not move anything"
            );
        }
    }
}

#[test]
fn space_toggles_the_collapse_of_the_selected_directory() {
    assert_eq!(
        action_for(press(KeyCode::Char(' '))),
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
            action_for(event),
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
        KeyCode::Char('o'),
        KeyCode::Char('g'),
    ] {
        assert_ne!(
            action_for(press(code)),
            Some(Action::ToggleCollapsed),
            "{code:?} should not collapse anything"
        );
    }
}

#[test]
fn o_toggles_the_scoped_only_filter() {
    assert_eq!(
        action_for(press(KeyCode::Char('o'))),
        Some(Action::ToggleScopedOnly)
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
            action_for(event),
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
        KeyCode::Char('f'),
        KeyCode::Char(' '),
    ] {
        assert_ne!(
            action_for(press(code)),
            Some(Action::ToggleScopedOnly),
            "{code:?} should not filter anything"
        );
    }
}

#[test]
fn f_toggles_the_files_inside_each_directory() {
    assert_eq!(
        action_for(press(KeyCode::Char('f'))),
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
            action_for(event),
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
        KeyCode::Char('s'),
        KeyCode::Char('F'),
        KeyCode::Char(' '),
    ] {
        assert_ne!(
            action_for(press(code)),
            Some(Action::ToggleFiles),
            "{code:?} should not show any files"
        );
    }
}

#[test]
fn s_asks_for_the_scope_prompt() {
    assert_eq!(
        action_for(press(KeyCode::Char('s'))),
        Some(Action::OpenScope)
    );
}

#[test]
fn upper_s_asks_for_nothing() {
    // Lower case only, like `o`, `f` and `m`: the upper-case letter is a
    // different keystroke, and leaving it unbound keeps it free for a later
    // one.
    assert_eq!(action_for(press(KeyCode::Char('S'))), None);
}

#[test]
fn releases_and_repeats_of_s_open_nothing() {
    // The same rule as the keys above, and here it decides whether the
    // prompt can be typed into at all: acting on a release would reopen the
    // prompt on the release of the very key that opened it, and a held `s`
    // would reopen it — losing whatever had been typed — as fast as the
    // terminal repeats.
    for kind in [KeyEventKind::Release, KeyEventKind::Repeat] {
        let event = KeyEvent::new_with_kind_and_state(
            KeyCode::Char('s'),
            KeyModifiers::NONE,
            kind,
            KeyEventState::NONE,
        );

        assert_eq!(action_for(event), None, "{kind:?} of s should open nothing");
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
        KeyCode::Char('f'),
        KeyCode::Char('v'),
        KeyCode::Char('S'),
        KeyCode::Char(' '),
        KeyCode::Enter,
    ] {
        assert_ne!(
            action_for(press(code)),
            Some(Action::OpenScope),
            "{code:?} should not ask for a scope"
        );
    }
}

#[test]
fn v_asks_to_read_the_selected_file() {
    assert_eq!(
        action_for(press(KeyCode::Char('v'))),
        Some(Action::ViewFile)
    );
}

#[test]
fn upper_v_asks_for_nothing() {
    // Lower case only, like `o`, `f`, `s` and `m`: the upper-case letter is
    // a different keystroke, and leaving it unbound keeps it free for a
    // later one.
    assert_eq!(action_for(press(KeyCode::Char('V'))), None);
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

        assert_eq!(action_for(event), None, "{kind:?} of v should read nothing");
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
        KeyCode::Char('s'),
        KeyCode::Char('e'),
        KeyCode::Char('V'),
        KeyCode::Char(' '),
        KeyCode::Enter,
    ] {
        assert_ne!(
            action_for(press(code)),
            Some(Action::ViewFile),
            "{code:?} should not read a file"
        );
    }
}

#[test]
fn e_asks_to_edit_the_selected_file() {
    assert_eq!(
        action_for(press(KeyCode::Char('e'))),
        Some(Action::EditFile)
    );
}

#[test]
fn upper_e_asks_for_nothing() {
    // Lower case only, like `o`, `f`, `s`, `v` and `m`: the upper-case
    // letter is a different keystroke, and leaving it unbound keeps it free
    // for a later one.
    assert_eq!(action_for(press(KeyCode::Char('E'))), None);
}

#[test]
fn releases_and_repeats_of_e_start_nothing() {
    // The same rule as the keys above: a release acted on would hand the
    // terminal to a second editor the moment the first one was asked for,
    // and a held `e` would suspend warlock as fast as the terminal repeats.
    for kind in [KeyEventKind::Release, KeyEventKind::Repeat] {
        let event = KeyEvent::new_with_kind_and_state(
            KeyCode::Char('e'),
            KeyModifiers::NONE,
            kind,
            KeyEventState::NONE,
        );

        assert_eq!(
            action_for(event),
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
        KeyCode::Char('d'),
        KeyCode::Char('m'),
        KeyCode::Char('E'),
        KeyCode::Char(' '),
        KeyCode::Enter,
    ] {
        assert_ne!(
            action_for(press(code)),
            Some(Action::EditFile),
            "{code:?} should not edit a file"
        );
    }
}

#[test]
fn shift_tab_swaps_the_panel_card() {
    assert_eq!(action_for(press(KeyCode::BackTab)), Some(Action::SwapCard));
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
            action_for(event),
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
            action_for(press(code)),
            Some(Action::SwapCard),
            "{code:?} should not swap the panel's card"
        );
    }
}

#[test]
fn m_toggles_the_mouse() {
    assert_eq!(
        action_for(press(KeyCode::Char('m'))),
        Some(Action::ToggleMouseCapture)
    );
}

#[test]
fn the_mouse_key_neither_quits_nor_moves_anything() {
    // Said against every other action by name, because what the key must not
    // do is the interesting half of it: it does not leave, it does not move
    // the keys to the other pane and it does not touch a row. One variant is
    // all it can come to, and the list below is the rest of them.
    let action = action_for(press(KeyCode::Char('m')));
    for other in [
        Action::Quit,
        Action::ToggleFocus,
        Action::SelectPrevious,
        Action::SelectNext,
        Action::SelectPageUp,
        Action::SelectPageDown,
        Action::SelectFirst,
        Action::SelectLast,
        Action::ToggleCollapsed,
        Action::ToggleScopedOnly,
        Action::ToggleFiles,
        Action::OpenScope,
        Action::ViewFile,
        Action::EditFile,
        Action::SwapCard,
        Action::Help,
    ] {
        assert_ne!(action, Some(other), "m should not mean {other:?}");
    }
}

#[test]
fn m_is_the_only_key_that_touches_the_mouse() {
    // Its neighbours in the match arms above, the letter beside it on the
    // keyboard, and its upper-case self, which this binding does not answer
    // to any more than `o` and `f` answer to theirs.
    for code in [
        KeyCode::Char('n'),
        KeyCode::Char('o'),
        KeyCode::Char('f'),
        KeyCode::Char('e'),
        KeyCode::Char('M'),
        KeyCode::Char(' '),
        KeyCode::Enter,
    ] {
        assert_ne!(
            action_for(press(code)),
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
            action_for(event),
            None,
            "{kind:?} of m should not toggle anything"
        );
    }
}

#[test]
fn keys_with_no_meaning_here_are_ignored() {
    for code in [
        KeyCode::Char('x'),
        KeyCode::Char('p'),
        KeyCode::Char('r'),
        KeyCode::Enter,
        KeyCode::Left,
    ] {
        assert_eq!(
            action_for(press(code)),
            None,
            "{code:?} should mean nothing"
        );
    }
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

        assert_eq!(action_for(event), None, "{kind:?} should not move anything");
    }
}

mod gate {
    use std::time::Instant;

    use super::super::{Action, Pressed, action_for, press_for};
    use crate::app::{App, Focus, Row};
    use crate::composer::{Composed, Composer};
    use crate::confirm::{
        Answer, CutConfirm, PullConfirm, PushConfirm, QuitConfirm, cut_answer_for, pull_answer_for,
        push_answer_for,
    };
    use crate::modal::Modals;
    use crate::prompt::{Edited, RecordPrompt, ScopeField, ScopePrompt, edit_for};
    use crate::ui::Shape;
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
    use ratatui::layout::Size;

    const SIZE: Size = Size {
        width: 80,
        height: 24,
    };

    const DIRECTORY: &str = "crates/warlock-engine";

    const INERT: [KeyCode; 18] = [
        KeyCode::Char('j'),
        KeyCode::Char('k'),
        KeyCode::Char('g'),
        KeyCode::Char('G'),
        KeyCode::Char(' '),
        KeyCode::Char('o'),
        KeyCode::Char('f'),
        KeyCode::Char('e'),
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
        let mut rows = vec![Row::new(0, "/repo", false)];
        for n in 0..12 {
            let directory = format!("/repo/d{n:02}");
            let covered = n % 2 == 0;
            rows.push(Row::new(1, directory.clone(), covered));
            rows.push(Row::file(2, format!("{directory}/lib.rs"), covered));
        }
        let mut app = App::from_rows(rows);
        let cut = Shape::new(SIZE).cut();
        app.set_viewport_height(cut.tree_height());
        app.panel_mut().set_height(cut.panel_height());
        app
    }

    fn app_in_use() -> App {
        let mut app = app_on_screen();
        app.toggle_files();
        app.toggle_scoped_only();
        app.select_row(9);
        app.toggle_collapsed();
        app.select_previous();
        // A panel with more in it than its window holds, so that scrolling
        // it back is a real offset rather than a no-op: an empty panel has
        // exactly one place its window can be, and a field that cannot move
        // cannot catch a key that moved it.
        let started = Instant::now();
        for n in 0..40 {
            app.panel_mut().note(format!("/repo/d{n:02}"), started);
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
        match press_for(
            key,
            Modals {
                quit: *confirm,
                scope: prompt,
                ..Modals::default()
            }
            .current(),
            offered(app, composer),
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
            // The rest are unreachable: this round hands the gate the quit
            // question and the scope prompt and every other window closed, so
            // an answer from any of them would be the gate answering a question
            // nobody asked. Loud rather than quiet for that reason; each
            // window's own tests call `press_for` with it up and assert on what
            // comes back.
            Pressed::Write(edited) => {
                panic!("{edited:?} came from a write prompt that is not up")
            }
            Pressed::Record(edited) => {
                panic!("{edited:?} came from a record window that is not up")
            }
            Pressed::Push(answered) => {
                panic!("{answered:?} came from a push dialog that is not up")
            }
            Pressed::Cut(answered) => {
                panic!("{answered:?} came from a draft dialog that is not up")
            }
            Pressed::Pull(answered) => {
                panic!("{answered:?} came from a pull dialog that is not up")
            }
            Pressed::Review(reviewed) => {
                panic!("{reviewed:?} came from a review window that is not up")
            }
            Pressed::Scroll(scroll) => {
                panic!("{scroll:?} came from a review window that is not up")
            }
            Pressed::Carry(answered) => {
                panic!("{answered:?} came from a carry-on question that is not up")
            }
            Pressed::WriteBrief(answered) => {
                panic!("{answered:?} came from a write question that is not up")
            }
            Pressed::Help(sheet) => panic!("{sheet:?} came from a help key no round presses"),
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
            Pressed::Act(Action::ToggleScopedOnly) => app.toggle_scoped_only(),
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
                action @ (Action::OpenScope
                | Action::ViewFile
                | Action::EditFile
                | Action::ToggleMouseCapture
                | Action::Help),
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
        assert!(app.scoped_only(), "and so is the scoped-only filter");
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
                        Modals {
                            quit: QuitConfirm::Open(lit),
                            ..Modals::default()
                        }
                        .current(),
                        None,
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
        // Both spellings of Yes, and one road out of the loop: one
        // `return Ok(())`, so the terminal guard restores the screen and a
        // running `claude` is taken down by the run's own drop, exactly as
        // before this gate existed.
        for key in [press(KeyCode::Char('y')), press(KeyCode::Enter)] {
            let mut app = app_in_use();
            let mut confirm = QuitConfirm::Open(Answer::Yes);

            assert_eq!(
                press_for(
                    key,
                    Modals {
                        quit: confirm,
                        ..Modals::default()
                    }
                    .current(),
                    None,
                    false
                ),
                Pressed::Leave
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
    fn ctrl_c_asks_with_the_question_down_and_keeps_it_as_it_was_up() {
        // Answered before the mode is consulted, which is what keeps it out
        // of `answer_for`'s "every other key" arm: through there it would be
        // an ordinary `c` with a modifier riding along, and the chord would
        // move nothing while the reader went on pressing it.
        //
        // Pinned with the composer holding the keyboard as well as without
        // it, because the field is the third thing that could have eaten the
        // key: through `compose_for` it is a chord rather than text, i.e.
        // one of the keys that change nothing, so a gate that consulted the
        // draft first would swallow it in silence.
        //
        // Never `Leave`: Ctrl-C is the copy chord everywhere else, and a
        // second press at the question is a reader still reaching for copy,
        // not one answering Yes.
        let draft = Composer::new("web");
        for (confirm, expected) in [
            (QuitConfirm::Closed, QuitConfirm::open()),
            (QuitConfirm::Open(Answer::No), QuitConfirm::Open(Answer::No)),
            (
                QuitConfirm::Open(Answer::Yes),
                QuitConfirm::Open(Answer::Yes),
            ),
        ] {
            for composer in [None, Some(&draft)] {
                assert_eq!(
                    press_for(
                        ctrl_c(),
                        Modals {
                            quit: confirm,
                            ..Modals::default()
                        }
                        .current(),
                        composer,
                        false
                    ),
                    Pressed::Confirm(expected),
                    "Ctrl-C should ask with {confirm:?} and {composer:?}"
                );
            }
        }

        let mut app = app_in_use();
        let before = app.clone();
        let mut confirm = QuitConfirm::Closed;
        for _ in 0..3 {
            assert_eq!(round(&mut app, &mut confirm, ctrl_c()), Round::Stayed);
            assert_eq!(confirm, QuitConfirm::open());
        }
        assert_eq!(app, before);
    }

    #[test]
    fn ctrl_c_stops_the_turn_rather_than_the_session_while_one_is_answered() {
        // The one situation the key does not ask to quit in, and the whole
        // of what decides it: a question is out. A turn is one `claude` and
        // seconds of waiting, so the reader who wants it back presses this;
        // a turn escapable only by leaving warlock would be a question nobody
        // can take back.
        //
        // The same matrix the answer above is pinned across, because the
        // reading must not depend on any of it: the question up or down, and
        // the keyboard in the field or not. A Ctrl-C that
        // stopped the turn with one window up and not with another would be
        // the cancel behaving differently depending on what is on screen.
        let draft = Composer::new("web");
        for confirm in [
            QuitConfirm::Closed,
            QuitConfirm::Open(Answer::No),
            QuitConfirm::Open(Answer::Yes),
        ] {
            for composer in [None, Some(&draft)] {
                assert_eq!(
                    press_for(
                        ctrl_c(),
                        Modals {
                            quit: confirm,
                            ..Modals::default()
                        }
                        .current(),
                        composer,
                        true
                    ),
                    Pressed::CancelTurn,
                    "Ctrl-C should stop the turn with {confirm:?} and {composer:?}"
                );
            }
        }

        // And the scope prompt, the other window it goes round.
        assert_eq!(
            press_for(
                ctrl_c(),
                Modals {
                    scope: &ScopePrompt::Open(ScopeField::new(DIRECTORY, "web")),
                    ..Modals::default()
                }
                .current(),
                None,
                true
            ),
            Pressed::CancelTurn
        );

        // The press after it asks, because by then nothing is being
        // answered: the reader who presses it twice gets the cancel and then
        // the quit question, never the way out unasked.
        assert_eq!(
            press_for(ctrl_c(), None, None, false),
            Pressed::Confirm(QuitConfirm::open())
        );
    }

    #[test]
    fn a_turn_being_answered_changes_no_other_key_at_all() {
        // Ctrl-C is the only key a turn is allowed to re-read: `q` and Esc
        // still ask the question they have always asked.
        for code in INERT.into_iter().chain([KeyCode::Char('q'), KeyCode::Esc]) {
            assert_eq!(
                press_for(press(code), None, None, true),
                press_for(press(code), None, None, false),
                "{code:?} read differently with a turn being answered"
            );
        }

        // Said plainly as well as by comparison, because it is the promise.
        for code in [KeyCode::Char('q'), KeyCode::Esc] {
            assert_eq!(
                press_for(press(code), None, None, true),
                Pressed::Confirm(QuitConfirm::open()),
                "{code:?} with a turn out should still ask first"
            );
        }
    }

    #[test]
    fn every_other_key_still_means_what_it_always_meant() {
        // The gate is one question in front of two keys and nothing else:
        // with it closed, every binding reaches the app as before.
        for code in INERT {
            assert_eq!(
                press_for(press(code), None, None, false),
                action_for(press(code)).map_or(Pressed::Nothing, Pressed::Act),
                "{code:?} should read as it always has"
            );
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
                    press_for(key, None, None, false),
                    Pressed::Nothing,
                    "{kind:?} of {code:?} should open nothing"
                );
                assert_eq!(
                    press_for(
                        key,
                        Modals {
                            quit: QuitConfirm::open(),
                            ..Modals::default()
                        }
                        .current(),
                        None,
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
        // `e`, `s`, `v`, `m`, Tab and the page keys are letters going into a
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
                let pressed = press_for(
                    key,
                    Modals {
                        scope: &prompt,
                        ..Modals::default()
                    }
                    .current(),
                    None,
                    false,
                );

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
                Modals {
                    scope: &prompt,
                    ..Modals::default()
                }
                .current(),
                None,
                false
            ),
            Pressed::Scope(Edited::Open(ScopeField::new(DIRECTORY, "webq"))),
            "q is a letter while the field has the keyboard"
        );
        assert_eq!(
            press_for(
                press(KeyCode::Esc),
                Modals {
                    scope: &prompt,
                    ..Modals::default()
                }
                .current(),
                None,
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
    fn ctrl_c_asks_with_the_scope_prompt_up() {
        // Answered before either window is consulted, and for the reason it
        // is answered before the question: through `edit_for` it is a `c`
        // wearing a modifier, i.e. one of the keys that change nothing, and
        // the field would swallow it. Pinned with an empty field and with
        // something typed.
        for prompt in [
            ScopePrompt::open(DIRECTORY, ""),
            ScopePrompt::open(DIRECTORY, "data-plane"),
        ] {
            assert_eq!(
                press_for(
                    ctrl_c(),
                    Modals {
                        scope: &prompt,
                        ..Modals::default()
                    }
                    .current(),
                    None,
                    false
                ),
                Pressed::Confirm(QuitConfirm::open()),
                "Ctrl-C should ask with {prompt:?} up"
            );
        }

        let mut app = app_in_use();
        let mut confirm = QuitConfirm::Closed;
        let mut prompt = ScopePrompt::open(DIRECTORY, "billing");
        let held = prompt.clone();
        assert_eq!(
            round_under(&mut app, &mut confirm, &mut prompt, ctrl_c()),
            Round::Stayed
        );
        assert_eq!(confirm, QuitConfirm::open());
        assert_eq!(
            prompt, held,
            "the field under the question kept what was typed"
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
        // signal, so if the gate does not answer it here nothing does. Over
        // the question it keeps the question; over the prompt and the field
        // alone it opens it, where `j` would have been typed.
        assert_eq!(
            press_for(
                ctrl_c(),
                Modals {
                    quit: question,
                    scope: &prompt,
                    ..Modals::default()
                }
                .current(),
                Some(&draft),
                false
            ),
            Pressed::Confirm(question)
        );
        assert_eq!(
            press_for(
                ctrl_c(),
                Modals {
                    scope: &prompt,
                    ..Modals::default()
                }
                .current(),
                Some(&draft),
                false
            ),
            Pressed::Confirm(QuitConfirm::open())
        );
        // Then the question, which is drawn over everything else on the
        // frame: a key cannot be both typed into a field and answered by the
        // dialog covering it.
        assert_eq!(
            press_for(
                key,
                Modals {
                    quit: question,
                    scope: &prompt,
                    ..Modals::default()
                }
                .current(),
                Some(&draft),
                false
            ),
            Pressed::Confirm(question)
        );
        // Then the prompt, over the composer, for the same reason again.
        assert_eq!(
            press_for(
                key,
                Modals {
                    scope: &prompt,
                    ..Modals::default()
                }
                .current(),
                Some(&draft),
                false
            ),
            Pressed::Scope(edit_for(key, prompt.field().expect("the prompt is up")))
        );
        // Then the composer, over the keys: this is where `j` stops being a
        // movement and becomes the letter j.
        assert_eq!(
            press_for(key, None, Some(&draft), false),
            Pressed::Compose(Composed::Typing(Composer::new("webj")))
        );
        // And then the keys, as they have always been read.
        assert_eq!(
            press_for(key, None, None, false),
            Pressed::Act(Action::SelectNext)
        );
    }

    mod writing {
        use super::{
            Action, Composer, Edited, INERT, KeyCode, KeyEvent, Modals, Pressed, QuitConfirm,
            ScopeField, ScopePrompt, action_for, ctrl_c, edit_for, press, press_for,
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
            press_for(
                key,
                Modals {
                    write,
                    ..Modals::default()
                }
                .current(),
                None,
                false,
            )
        }

        #[test]
        fn every_binding_goes_into_the_field_and_none_of_them_reaches_the_app() {
            // The promise, over the whole list: while a path is being typed
            // there is no `e` that hands the terminal to an editor, no `j`
            // that moves a selection nobody can see and no Tab that hands the
            // keyboard somewhere else — every one of them is `edit_for`'s
            // answer and nothing else's.
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
            // Answered before this window as before every other: with
            // nothing out it asks the quit question, and it stops the turn
            // when one is being answered. Typed into the field it would be
            // a `c` on the end of a path.
            let write = open();

            assert_eq!(
                asked(ctrl_c(), &write),
                Pressed::Confirm(QuitConfirm::open())
            );
            assert_eq!(
                press_for(
                    ctrl_c(),
                    Modals {
                        write: &write,
                        ..Modals::default()
                    }
                    .current(),
                    None,
                    true
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
                    press_for(
                        key,
                        Modals {
                            write: &write,
                            ..Modals::default()
                        }
                        .current(),
                        Some(&draft),
                        false
                    ),
                    Pressed::Write(edit_for(key, &field())),
                    "{code:?} reached the draft from behind the window"
                );
            }
        }

        #[test]
        fn the_scope_prompt_has_the_keys_while_both_windows_are_up() {
            // The precedence, and the one situation that needs one: `s`
            // opens the scope prompt from the tree while a write turn is
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
                    press_for(
                        key,
                        Modals {
                            scope: &scope,
                            write: &write,
                            ..Modals::default()
                        }
                        .current(),
                        None,
                        false
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
                let expected = match action_for(key) {
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
    mod offering {
        use super::{Answer, Composer, INERT, KeyCode, Modals, Pressed, ctrl_c, press, press_for};
        use crate::confirm::{Answered, answer_for};

        fn asked(key: super::KeyEvent, lit: Answer, composer: Option<&Composer>) -> Pressed {
            press_for(
                key,
                Modals {
                    write_brief: Some(lit),
                    ..Modals::default()
                }
                .current(),
                composer,
                false,
            )
        }

        #[test]
        fn every_key_is_the_questions_and_none_reaches_the_app_or_the_field() {
            let draft = Composer::new("say something");

            for code in
                INERT
                    .into_iter()
                    .chain([KeyCode::Char('a'), KeyCode::Char('q'), KeyCode::Enter])
            {
                let key = press(code);

                assert_eq!(
                    asked(key, Answer::No, Some(&draft)),
                    Pressed::WriteBrief(answer_for(key, Answer::No)),
                    "{code:?} while the write question is up"
                );
            }
        }

        #[test]
        fn y_says_yes_and_enter_on_the_lit_no_says_no() {
            assert_eq!(
                asked(press(KeyCode::Char('y')), Answer::No, None),
                Pressed::WriteBrief(Answered::Leave)
            );
            assert_eq!(
                asked(press(KeyCode::Enter), Answer::No, None),
                Pressed::WriteBrief(Answered::Close)
            );
            assert_eq!(
                asked(press(KeyCode::Left), Answer::No, None),
                Pressed::WriteBrief(Answered::Open(Answer::Yes))
            );
            assert_eq!(
                asked(press(KeyCode::Enter), Answer::Yes, None),
                Pressed::WriteBrief(Answered::Leave)
            );
        }

        #[test]
        fn ctrl_c_is_still_the_quit_question() {
            assert!(matches!(
                asked(ctrl_c(), Answer::No, None),
                Pressed::Confirm(_)
            ));
        }
    }

    mod recording {
        use super::{
            Composer, INERT, KeyCode, KeyEvent, Modals, Pressed, QuitConfirm, ctrl_c, press,
            press_for,
        };
        use crate::prompt::{RecordEdited, RecordForm, RecordPrompt, record_edit_for};

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
            // No scope window beside it, and no test of the two up together,
            // because they cannot be: the record window opens exactly as the
            // scope window closes, out of the one value `scope_submit` hands
            // back.
            press_for(
                key,
                Modals {
                    record,
                    ..Modals::default()
                }
                .current(),
                composer,
                false,
            )
        }

        #[test]
        fn every_binding_goes_into_the_focused_field_and_none_of_them_reaches_the_app() {
            // The same promise the other two windows make, and the one that
            // matters most here because `s` is on the list: while a record is
            // being filled in there is no `s` that reopens the window over the
            // scope it is already recording, no `e` that hands the terminal to
            // an editor and no `j` that moves a selection nobody can see.
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
            // Answered before this window as before every other: with nothing
            // out it asks the quit question, and it stops the turn when one is
            // being answered. Typed into a field it would be a `c` in a team
            // name.
            let record = open();

            assert_eq!(
                asked(ctrl_c(), &record),
                Pressed::Confirm(QuitConfirm::open())
            );
            assert_eq!(
                press_for(
                    ctrl_c(),
                    Modals {
                        record: &record,
                        ..Modals::default()
                    }
                    .current(),
                    None,
                    true
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
                press_for(
                    press(KeyCode::Char('y')),
                    Modals {
                        quit: QuitConfirm::open(),
                        record: &record,
                        ..Modals::default()
                    }
                    .current(),
                    None,
                    false
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
                    press_for(key, None, None, false),
                    "{code:?} was answered by a window that is not up"
                );
            }
        }
    }

    mod pushing {
        use super::{
            Composer, INERT, KeyCode, KeyEvent, Modals, Pressed, PushConfirm, QuitConfirm,
            RecordPrompt, ScopePrompt, ctrl_c, press, press_for, push_answer_for,
        };

        fn open() -> PushConfirm {
            PushConfirm::open(
                "Push a brief to the board",
                warlock_engine::Destination::new("warlock-team", "Warlock", "warlock", "work"),
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
            press_for(
                key,
                Modals {
                    push,
                    ..Modals::default()
                }
                .current(),
                composer,
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
            // is no `s` that opens a scope prompt under it, no `j` that moves a
            // selection under the dialog and no `e` that hands the terminal to
            // an editor nobody asked for.
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
            // Answered before this window as before every other: with nothing
            // out it asks the quit question, and it stops the turn when one is
            // being answered. Through the dialog it would be an inert `c`.
            let push = open();

            assert_eq!(
                asked(ctrl_c(), &push),
                Pressed::Confirm(QuitConfirm::open())
            );
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
            // The precedence, written down: a write turn still out opens the
            // write prompt on no keystroke at all, and a field that came up
            // under this dialog does not get to take the keys off it.
            let push = open();
            let scope = ScopePrompt::open("crates/warlock-engine", "data-plane");
            let record = RecordPrompt::open("crates/warlock-engine", "data-plane");
            let write = ScopePrompt::open("Write the brief to", "docs/brief.md");

            for code in INERT.into_iter().chain([KeyCode::Enter, KeyCode::Esc]) {
                let key = press(code);

                assert_eq!(
                    press_for(
                        key,
                        Modals {
                            push: &push,
                            scope: &scope,
                            record: &record,
                            write: &write,
                            ..Modals::default()
                        }
                        .current(),
                        None,
                        false
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
                press_for(
                    press(KeyCode::Enter),
                    Modals {
                        quit: QuitConfirm::open(),
                        push: &push,
                        ..Modals::default()
                    }
                    .current(),
                    None,
                    false
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
                    press_for(key, None, None, false),
                    "{code:?} was answered by a window that is not up"
                );
            }
        }
    }

    mod cutting {
        use super::{
            Composer, CutConfirm, INERT, KeyCode, KeyEvent, Modals, Pressed, PushConfirm,
            QuitConfirm, RecordPrompt, ScopePrompt, ctrl_c, cut_answer_for, press, press_for,
        };

        fn open() -> CutConfirm {
            CutConfirm::open(
                "Cut a planned project into tickets",
                "planned",
                9,
                "Warlock",
                "work",
            )
        }

        // The gate with this window up and every other one down, which is the
        // only way it is ever up in a session.
        fn asked(key: KeyEvent, cut: &CutConfirm) -> Pressed {
            asking(key, cut, None, false)
        }

        fn asking(
            key: KeyEvent,
            cut: &CutConfirm,
            composer: Option<&Composer>,
            answered: bool,
        ) -> Pressed {
            press_for(
                key,
                Modals {
                    cut,
                    ..Modals::default()
                }
                .current(),
                composer,
                answered,
            )
        }

        // What the dialog itself says about a key, which is what the gate has
        // to hand back for every one of them — and it hands it back as a
        // variant of its own, so no arm can mistake a cut's answer for a
        // push's.
        fn answered(key: KeyEvent, cut: &CutConfirm) -> Pressed {
            let cutting = cut.cutting().expect("the dialog under test is up");
            Pressed::Cut(cut_answer_for(key, cutting.answer()))
        }

        #[test]
        fn every_tree_binding_is_the_dialogs_and_none_of_them_reaches_the_app() {
            // The promise over the whole list, as for the push dialog: while
            // the question is up there is no `s` that opens a scope prompt, no
            // `j` that moves a selection under it and no `e` that opens an
            // editor.
            let cut = open();

            for code in INERT {
                let key = press(code);

                assert_eq!(
                    asked(key, &cut),
                    answered(key, &cut),
                    "{code:?} should have been answered by the dialog"
                );
            }
        }

        #[test]
        fn q_is_the_dialogs_too_so_the_quit_question_cannot_come_up_underneath() {
            let cut = open();
            let key = press(KeyCode::Char('q'));

            assert_eq!(asked(key, &cut), answered(key, &cut));
            assert_ne!(asked(key, &cut), Pressed::Confirm(QuitConfirm::open()));
        }

        #[test]
        fn ctrl_c_is_still_answered_before_the_dialog() {
            // Answered before this window as before every other: with nothing
            // out it asks the quit question, and it stops the turn when one is
            // being answered.
            let cut = open();

            assert_eq!(asked(ctrl_c(), &cut), Pressed::Confirm(QuitConfirm::open()));
            assert_eq!(asking(ctrl_c(), &cut, None, true), Pressed::CancelTurn);
        }

        #[test]
        fn the_composer_is_not_consulted_while_the_dialog_is_up() {
            // Which is also why a second `/draft` cannot be typed while this is
            // up: the field that would take the command is behind the window.
            let cut = open();
            let draft = Composer::new("web");

            for code in [KeyCode::Char('j'), KeyCode::Tab, KeyCode::Enter] {
                let key = press(code);

                assert_eq!(
                    asking(key, &cut, Some(&draft), false),
                    answered(key, &cut),
                    "{code:?} reached the draft from behind the dialog"
                );
            }
        }

        #[test]
        fn the_dialog_has_the_keys_while_any_of_the_three_fields_is_up() {
            // The precedence, written down: a write turn still out opens the
            // write prompt on no keystroke at all, and a field that came up
            // under this dialog does not get to take the keys off it.
            let cut = open();
            let scope = ScopePrompt::open("crates/warlock-engine", "data-plane");
            let record = RecordPrompt::open("crates/warlock-engine", "data-plane");
            let write = ScopePrompt::open("Write the brief to", "docs/brief.md");

            for code in INERT.into_iter().chain([KeyCode::Enter, KeyCode::Esc]) {
                let key = press(code);

                assert_eq!(
                    press_for(
                        key,
                        Modals {
                            cut: &cut,
                            scope: &scope,
                            record: &record,
                            write: &write,
                            ..Modals::default()
                        }
                        .current(),
                        None,
                        false
                    ),
                    answered(key, &cut),
                    "{code:?} was answered by the wrong window"
                );
            }
        }

        #[test]
        fn the_quit_question_is_asked_before_it() {
            // The other side of the order, and a situation no session is in.
            // Asserted anyway, because which window answers has to be decided
            // somewhere rather than by the order of two `if`s nobody looked at.
            let cut = open();

            assert_eq!(
                asking(press(KeyCode::Enter), &cut, None, false),
                answered(press(KeyCode::Enter), &cut)
            );
            assert_eq!(
                press_for(
                    press(KeyCode::Enter),
                    Modals {
                        quit: QuitConfirm::open(),
                        cut: &cut,
                        ..Modals::default()
                    }
                    .current(),
                    None,
                    false
                ),
                Pressed::Confirm(QuitConfirm::Closed)
            );
        }

        #[test]
        fn the_push_dialog_is_asked_in_the_same_place_and_first() {
            // The one place both could be filled, which no session reaches:
            // the push dialog is up while a brief is being filed and this one
            // only after a project has been read back. Decided here rather
            // than left to whichever `if` happens to come first.
            let cut = open();
            let push = PushConfirm::open(
                "Push a brief to the board",
                warlock_engine::Destination::new("warlock-team", "Warlock", "warlock", "work"),
            );
            let key = press(KeyCode::Enter);

            assert!(matches!(
                press_for(
                    key,
                    Modals {
                        push: &push,
                        cut: &cut,
                        ..Modals::default()
                    }
                    .current(),
                    None,
                    false
                ),
                Pressed::Push(_)
            ));
        }

        #[test]
        fn the_keys_mean_what_they_always_did_once_the_dialog_is_down() {
            // The other half of the promise: with the question answered, every
            // key it swallowed is whatever the gate with nothing up says it is.
            for code in INERT.into_iter().chain([KeyCode::Char('q')]) {
                let key = press(code);

                assert_eq!(
                    asked(key, &CutConfirm::Closed),
                    press_for(key, None, None, false),
                    "{code:?} was answered by a window that is not up"
                );
            }
        }
    }

    // The question a `/pull` asks once the ticket is chosen, gated exactly as the
    // cut dialog beside it is: the same list of keys swallowed, the same Ctrl-C
    // before it, and the same precedence written down rather than inferred.
    mod pulling {
        use super::{
            Composer, CutConfirm, INERT, KeyCode, KeyEvent, Modals, Pressed, PullConfirm,
            PushConfirm, QuitConfirm, RecordPrompt, ScopePrompt, ctrl_c, press, press_for,
            pull_answer_for,
        };

        fn open() -> PullConfirm {
            PullConfirm::open(
                "WAR-143",
                "Add /pull and /resume to the panel",
                "warlock-team",
                "Warlock",
                "war-143/add-pull-and-resume-to-the-panel",
            )
        }

        // The gate with this window up and every other one down, which is the
        // only way it is ever up in a session.
        fn asked(key: KeyEvent, pull: &PullConfirm) -> Pressed {
            asking(key, pull, None, false)
        }

        fn asking(
            key: KeyEvent,
            pull: &PullConfirm,
            composer: Option<&Composer>,
            answered: bool,
        ) -> Pressed {
            press_for(
                key,
                Modals {
                    pull,
                    ..Modals::default()
                }
                .current(),
                composer,
                answered,
            )
        }

        // What the dialog itself says about a key, which is what the gate has to
        // hand back for every one of them — and it hands it back as a variant of
        // its own, so no arm can mistake a pull's answer for a cut's.
        fn answered(key: KeyEvent, pull: &PullConfirm) -> Pressed {
            let undertaking = pull.undertaking().expect("the dialog under test is up");
            Pressed::Pull(pull_answer_for(key, undertaking.answer()))
        }

        #[test]
        fn every_tree_binding_is_the_dialogs_and_none_of_them_reaches_the_app() {
            // While the question is up there is no `s` that opens a scope
            // prompt, no `j` that moves a selection under it and no `e` that
            // opens an editor.
            let pull = open();

            for code in INERT {
                let key = press(code);

                assert_eq!(
                    asked(key, &pull),
                    answered(key, &pull),
                    "{code:?} should have been answered by the dialog"
                );
            }
        }

        #[test]
        fn q_is_the_dialogs_too_so_the_quit_question_cannot_come_up_underneath() {
            let pull = open();
            let key = press(KeyCode::Char('q'));

            assert_eq!(asked(key, &pull), answered(key, &pull));
            assert_ne!(asked(key, &pull), Pressed::Confirm(QuitConfirm::open()));
        }

        #[test]
        fn ctrl_c_is_still_answered_before_the_dialog() {
            // Answered before this window as before every other: with nothing
            // out it asks the quit question, and it stops the turn when one is
            // being answered.
            let pull = open();

            assert_eq!(
                asked(ctrl_c(), &pull),
                Pressed::Confirm(QuitConfirm::open())
            );
            assert_eq!(asking(ctrl_c(), &pull, None, true), Pressed::CancelTurn);
        }

        #[test]
        fn the_composer_is_not_consulted_while_the_dialog_is_up() {
            // Which is also why a second `/pull` cannot be typed while this is
            // up: the field that would take the command is behind the window.
            let pull = open();
            let draft = Composer::new("web");

            for code in [KeyCode::Char('j'), KeyCode::Tab, KeyCode::Enter] {
                let key = press(code);

                assert_eq!(
                    asking(key, &pull, Some(&draft), false),
                    answered(key, &pull),
                    "{code:?} reached the draft from behind the dialog"
                );
            }
        }

        #[test]
        fn the_dialog_has_the_keys_while_any_of_the_three_fields_is_up() {
            // The precedence, written down: a write turn still out opens the
            // write prompt on no keystroke at all, and a field that came up
            // under this dialog does not get to take the keys off it.
            let pull = open();
            let scope = ScopePrompt::open("crates/warlock-engine", "data-plane");
            let record = RecordPrompt::open("crates/warlock-engine", "data-plane");
            let write = ScopePrompt::open("Write the brief to", "docs/brief.md");

            for code in INERT.into_iter().chain([KeyCode::Enter, KeyCode::Esc]) {
                let key = press(code);

                assert_eq!(
                    press_for(
                        key,
                        Modals {
                            pull: &pull,
                            scope: &scope,
                            record: &record,
                            write: &write,
                            ..Modals::default()
                        }
                        .current(),
                        None,
                        false
                    ),
                    answered(key, &pull),
                    "{code:?} was answered by the wrong window"
                );
            }
        }

        #[test]
        fn the_quit_question_and_the_other_two_dialogs_are_asked_before_it() {
            // Three situations no session is in — a `/pull` is answered before
            // anything else it could sit under is open — and asserted anyway,
            // because which window answers has to be decided somewhere rather
            // than by the order of three `if`s nobody looked at.
            let pull = open();
            let push = PushConfirm::open(
                "Push a brief to the board",
                warlock_engine::Destination::new("warlock-team", "Warlock", "warlock", "work"),
            );
            let cut = CutConfirm::open("A project", "planned", 9, "Warlock", "work");
            let key = press(KeyCode::Enter);

            assert_eq!(asked(key, &pull), answered(key, &pull));
            assert_eq!(
                press_for(
                    key,
                    Modals {
                        quit: QuitConfirm::open(),
                        pull: &pull,
                        ..Modals::default()
                    }
                    .current(),
                    None,
                    false
                ),
                Pressed::Confirm(QuitConfirm::Closed)
            );
            assert!(matches!(
                press_for(
                    key,
                    Modals {
                        push: &push,
                        pull: &pull,
                        ..Modals::default()
                    }
                    .current(),
                    None,
                    false
                ),
                Pressed::Push(_)
            ));
            assert!(matches!(
                press_for(
                    key,
                    Modals {
                        cut: &cut,
                        pull: &pull,
                        ..Modals::default()
                    }
                    .current(),
                    None,
                    false
                ),
                Pressed::Cut(_)
            ));
        }

        #[test]
        fn the_keys_mean_what_they_always_did_once_the_dialog_is_down() {
            // The other half of the promise: with the question answered, every
            // key it swallowed is whatever the gate with nothing up says it is.
            for code in INERT.into_iter().chain([KeyCode::Char('q')]) {
                let key = press(code);

                assert_eq!(
                    asked(key, &PullConfirm::Closed),
                    press_for(key, None, None, false),
                    "{code:?} was answered by a window that is not up"
                );
            }
        }
    }

    // The two windows a confirmed cut puts up while it is running: the one a
    // slice's drafts wait behind, and the question a skipped slice leaves. Both
    // are states of the run rather than fields of the session, so both are
    // handed to the gate as an `Option` and every round here hands one over.
    mod running {
        use super::{
            Composer, CutConfirm, INERT, KeyCode, KeyEvent, Modals, Pressed, QuitConfirm, ctrl_c,
            press, press_for,
        };
        use crate::confirm::{
            Carry, Choice, Review, Reviewed, carry_answer_for, review_answer_for,
        };
        use crate::input::Scroll;

        const SLICE: &str = "slice 1 `Gate the drafts`";

        fn review() -> Review {
            Review::open(SLICE, vec!["Put a window in the way".to_owned()])
        }

        fn carry() -> Carry {
            Carry::open("2 slices")
        }

        // The gate with one of the two up and every other window down, which is
        // the only way either is ever up in a session.
        fn asking(
            key: KeyEvent,
            review: Option<&Review>,
            carry: Option<&Carry>,
            composer: Option<&Composer>,
            answered: bool,
        ) -> Pressed {
            press_for(
                key,
                Modals {
                    review,
                    carry,
                    ..Modals::default()
                }
                .current(),
                composer,
                answered,
            )
        }

        #[test]
        fn every_tree_binding_is_the_review_windows_and_none_of_them_reaches_the_app() {
            // While drafts are waiting to be answered about there is no `s`
            // that opens a scope prompt, no `j` that moves a selection behind
            // the window and
            // no `q` that puts the quit question up under it. The keys that
            // move through a page scroll the drafts on the panel instead.
            let drafts = review();

            for code in INERT.into_iter().chain([KeyCode::Char('q'), KeyCode::Esc]) {
                let key = press(code);
                let expected = match code {
                    KeyCode::Up | KeyCode::Char('k') => Pressed::Scroll(Scroll::LineUp),
                    KeyCode::Down | KeyCode::Char('j') => Pressed::Scroll(Scroll::LineDown),
                    KeyCode::PageUp => Pressed::Scroll(Scroll::PageUp),
                    KeyCode::PageDown => Pressed::Scroll(Scroll::PageDown),
                    _ => Pressed::Review(review_answer_for(key, &drafts)),
                };

                assert_eq!(
                    asking(key, Some(&drafts), None, None, false),
                    expected,
                    "{code:?} while the review window is up"
                );
            }
        }

        #[test]
        fn every_tree_binding_is_the_carry_on_questions_too() {
            let asked = carry();

            for code in INERT.into_iter().chain([KeyCode::Char('q'), KeyCode::Esc]) {
                let key = press(code);

                assert_eq!(
                    asking(key, None, Some(&asked), None, false),
                    Pressed::Carry(carry_answer_for(key, asked.answer())),
                    "{code:?} should have been answered by the carry-on question"
                );
            }
        }

        #[test]
        fn the_composer_is_not_consulted_while_either_is_up() {
            // Which is why a slice cannot be answered into the field while its
            // own drafts are waiting: the field is behind the window.
            let drafts = review();
            let asked = carry();
            let draft = Composer::new("say something");

            for code in [KeyCode::Char('a'), KeyCode::Tab, KeyCode::Enter] {
                let key = press(code);

                assert_eq!(
                    asking(key, Some(&drafts), None, Some(&draft), false),
                    Pressed::Review(review_answer_for(key, &drafts)),
                    "{code:?} reached the draft from behind the review window"
                );
                assert_eq!(
                    asking(key, None, Some(&asked), Some(&draft), false),
                    Pressed::Carry(carry_answer_for(key, asked.answer())),
                    "{code:?} reached the draft from behind the carry-on question"
                );
            }
        }

        #[test]
        fn ctrl_c_is_still_answered_before_either_of_them() {
            // Answered before these windows as before every other.
            assert_eq!(
                asking(ctrl_c(), Some(&review()), None, None, false),
                Pressed::Confirm(QuitConfirm::open())
            );
            assert_eq!(
                asking(ctrl_c(), None, Some(&carry()), None, true),
                Pressed::CancelTurn
            );
        }

        #[test]
        fn the_quit_question_and_the_two_dialogs_are_asked_before_them() {
            // Situations no session is in — the dialog is answered and gone
            // before a slice is drafted — asserted anyway, so which window
            // answers is decided here rather than by the order of two `if`s
            // nobody looked at.
            let drafts = review();
            let key = press(KeyCode::Enter);

            assert_eq!(
                press_for(
                    key,
                    Modals {
                        quit: QuitConfirm::open(),
                        review: Some(&drafts),
                        ..Modals::default()
                    }
                    .current(),
                    None,
                    false
                ),
                Pressed::Confirm(QuitConfirm::Closed)
            );
            assert!(matches!(
                press_for(
                    key,
                    Modals {
                        cut: &CutConfirm::open("A project", "planned", 9, "Warlock", "work"),
                        review: Some(&drafts),
                        ..Modals::default()
                    }
                    .current(),
                    None,
                    false
                ),
                Pressed::Cut(_)
            ));
        }

        #[test]
        fn the_review_window_is_asked_before_the_carry_on_question() {
            // The one place both could be filled, which no run reaches: a slice
            // is being reviewed, or asking whether to carry on, or neither.
            let drafts = review();

            assert_eq!(
                asking(
                    press(KeyCode::Enter),
                    Some(&drafts),
                    Some(&carry()),
                    None,
                    false
                ),
                Pressed::Review(Reviewed::Create)
            );
        }

        #[test]
        fn a_windows_answer_cannot_be_mistaken_for_any_other_windows() {
            // Each window hands its answer back as a variant of its own, so no
            // arm of the loop can file a slice's drafts because a quit question
            // was answered Yes.
            let create = review().with_choice(Choice::Create);

            assert_eq!(
                asking(press(KeyCode::Enter), Some(&create), None, None, false),
                Pressed::Review(Reviewed::Create)
            );
            assert_ne!(
                asking(press(KeyCode::Enter), Some(&create), None, None, false),
                Pressed::Confirm(QuitConfirm::Closed)
            );
        }

        #[test]
        fn the_keys_mean_what_they_always_did_once_both_are_down() {
            // The other half of the promise: with the run past its windows,
            // every key they swallowed is whatever the gate with nothing up
            // says it is.
            for code in INERT.into_iter().chain([KeyCode::Char('q')]) {
                let key = press(code);

                assert_eq!(
                    asking(key, None, None, None, false),
                    press_for(key, None, None, false),
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
                press_for(key, None, Some(&before), false),
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
                    press_for(key, None, offered(&app, &composer), false),
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
                let pressed = press_for(press(code), None, Some(&composer), false);

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
            // the tree's bindings. An `e` that did would hand the terminal to
            // an editor over whatever row happened to be selected — the very
            // accident the field was built to stop, arriving by the other
            // road.
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
                    press_for(press(code), None, Some(&composer), true),
                    Pressed::Nothing,
                    "{code:?} did something at a muted field"
                );
                // And through the loop's own round, which panics on the keys
                // that open a window, a file or an editor: an `s` that leaked
                // past the muting would be caught there rather than quietly
                // passing here.
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
                press_for(press(KeyCode::Tab), None, Some(&muted), true),
                Pressed::Act(Action::ToggleFocus),
            );
            assert_eq!(
                press_for(ctrl_c(), None, Some(&muted), true),
                Pressed::CancelTurn,
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
                press_for(press(KeyCode::Char('p')), None, Some(&composer), true),
                Pressed::Nothing,
            );

            composer.set_muted(false);

            assert_eq!(
                press_for(press(KeyCode::Char('p')), None, Some(&composer), false),
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
                press_for(press(KeyCode::Tab), None, Some(&composer), false),
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
                press_for(press(KeyCode::Esc), None, Some(&composer), false),
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
        fn esc_at_the_composer_hands_the_keyboard_back_and_the_next_one_asks_to_quit() {
            // The same rule the scope prompt keeps: the Esc pressed while a
            // field has the keyboard is answered by that field, and only the
            // press after it — with the keyboard back on the panel — reaches
            // the quit question.
            let composer = Composer::new(TYPED);

            assert_eq!(
                press_for(press(KeyCode::Esc), None, Some(&composer), false),
                Pressed::Compose(Composed::Leave)
            );
            assert_eq!(
                press_for(press(KeyCode::Esc), None, None, false),
                Pressed::Confirm(QuitConfirm::open())
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
                press_for(press(KeyCode::Enter), None, Some(&composer), false),
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
        fn ctrl_c_asks_and_types_no_c_while_the_composer_has_it() {
            // The order the gate decides in, where it matters most: through
            // `compose_for` Ctrl-C is a chord rather than text, so a gate
            // that consulted the field first would answer it with nothing at
            // all — and would not even leave a `c` behind to show for it.
            for draft in ["", TYPED] {
                let composer = Composer::new(draft);

                assert_eq!(
                    press_for(ctrl_c(), None, Some(&composer), false),
                    Pressed::Confirm(QuitConfirm::open()),
                    "Ctrl-C should ask from {draft:?}"
                );
            }

            let mut app = app_composing();
            let mut composer = Composer::new(TYPED);
            let mut confirm = QuitConfirm::Closed;
            let mut prompt = ScopePrompt::Closed;

            assert_eq!(
                round_composing(&mut app, &mut confirm, &mut prompt, &mut composer, ctrl_c()),
                Round::Stayed
            );
            assert_eq!(confirm, QuitConfirm::open());
            assert_eq!(composer.draft(), TYPED, "and typed no c under the question");
        }

        #[test]
        fn the_draft_survives_esc_and_the_focus_cycle() {
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

    use super::super::{MouseAction, WHEEL_NOTCH, mouse_action};
    use crate::app::{App, Focus, Row};
    use crate::composer::Composer;
    use crate::confirm::{Carry, CutConfirm, PushConfirm, QuitConfirm, Review};
    use crate::modal::{Modal, Modals};
    use crate::prompt::{RecordPrompt, ScopePrompt};
    use crate::selection::Cell;
    use crate::ui::{Reach, Shape, panel_width};

    // The one terminal every test below points at, and the layout it comes
    // to. Eighty columns is wide enough that the tree takes its floor of
    // thirty rather than an even split, which is what puts the panes where
    // the columns and rows underneath say they are — narrow this and every
    // constant below moves:
    //
    //   columns  0        panel        49 50       tree        79
    //   row  0   ┌───────────────────────┐┌ pulls into … ─────────┐
    //   row  1   │ panel line 0          ││ tree row 0            │
    //    ...     │  ...                  ││  ...                  │
    //   row 16   │ panel line 15         ││ tree row 15           │
    //   row 17   │ panel line 16         │└───────────────────────┘
    //   row 18   │ panel line 17         │┌ sigils ───────────────┐
    //   row 19   │ panel line 18         ││                       │
    //   row 20   └───────────────────────┘└───────────────────────┘
    //   rows 21-23                     the footer
    const SIZE: Size = Size {
        width: 80,
        height: 24,
    };

    const IN_TREE: u16 = 65;

    const IN_PANEL: u16 = 10;

    const FIRST_TREE_ROW: u16 = 1;

    const FIRST_PANEL_LINE: u16 = 1;

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
        usize::from(Shape::new(SIZE).cut().tree_height())
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
            left_click(IN_TREE, FIRST_TREE_ROW + 3),
            left_click(IN_PANEL, FIRST_PANEL_LINE + 3),
        ]
    }

    fn rows() -> Vec<Row> {
        let mut rows = vec![Row::new(0, "/repo", false)];
        for n in 0..24 {
            let directory = format!("/repo/d{n:02}");
            // The row under each of these is the file below. What makes them
            // collapsible is the file toggle being on, which is
            // `App::can_collapse`'s answer and not the tree's.
            rows.push(Row::new(1, directory.clone(), false));
            rows.push(Row::file(2, format!("{directory}/lib.rs"), false));
        }
        rows
    }

    fn app_on_screen() -> App {
        let mut app = App::from_rows(rows());
        let cut = Shape::new(SIZE).cut();
        app.set_viewport_height(cut.tree_height());
        app.panel_mut().set_height(cut.panel_height());
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
        asks_under(mouse, app, None)
    }

    fn asks_under(mouse: MouseEvent, app: &App, modal: Option<Modal<'_>>) -> Option<MouseAction> {
        mouse_action(mouse, Shape::new(SIZE), app, modal)
    }

    fn asks_composing(mouse: MouseEvent, app: &App, composer: &Composer) -> Option<MouseAction> {
        let shape = Shape::new(SIZE).composer(Some(composer));
        mouse_action(mouse, shape, app, None)
    }

    fn round(app: &mut App, confirm: QuitConfirm, mouse: MouseEvent) {
        round_under(
            app,
            Modals {
                quit: confirm,
                ..Modals::default()
            }
            .current(),
            mouse,
        );
    }

    fn round_under(app: &mut App, modal: Option<Modal<'_>>, mouse: MouseEvent) {
        match asks_under(mouse, app, modal) {
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
        assert_eq!(viewport(), 16, "sixteen rows of tree at 80x24");
        assert_eq!(
            usize::from(Shape::new(SIZE).cut().panel_height()),
            19,
            "nineteen lines of panel"
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
    fn the_document_card_takes_no_selection_either() {
        // Selection is the conversation's alone: the document is a file that
        // was read, and it is not highlighted by this gesture. The
        // conversation underneath is the one the fixture recorded, so this is
        // the same app that *would* answer with the other card showing.
        let mut app = app_talking();
        app.show_document(["a line of a file that was read"], false);

        assert!(!app.panel().showing_thread());
        assert_eq!(
            asks(left_click(IN_PANEL, FIRST_PANEL_LINE + 2), &app),
            Some(MouseAction::Focus(Focus::Panel)),
            "a press on this card is the focus it always was"
        );
        assert_eq!(asks(drag(IN_PANEL, FIRST_PANEL_LINE + 2), &app), None);
        assert_eq!(asks(release(IN_PANEL, FIRST_PANEL_LINE + 2), &app), None);

        app.swap_card();
        assert!(
            app.panel().showing_thread(),
            "the conversation is the other card"
        );
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
        let last = FIRST_PANEL_LINE + Shape::new(SIZE).cut().panel_height() - 1;
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
        // document is read past rather than copied out of, and an empty
        // thread card has nothing to scroll a selection through.
        let app = app_on_screen();
        assert!(app.panel().showing_thread() && !app.panel().has_thread());
        for row in [0, 20, FOOTER] {
            assert_eq!(asks(drag(IN_PANEL, row), &app), None, "at row {row}");
            assert_eq!(asks(release(IN_PANEL, row), &app), None, "at row {row}");
        }

        let mut app = app_talking();
        app.show_document(["a line of a file that was read"], false);
        assert!(!app.panel().showing_thread());
        for row in [0, 20, FOOTER] {
            assert_eq!(asks(drag(IN_PANEL, row), &app), None, "document, row {row}");
            assert_eq!(
                asks(release(IN_PANEL, row), &app),
                None,
                "document, row {row}"
            );
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
        let rows = Shape::new(SIZE)
            .composer(Some(&composer))
            .cut()
            .panel_height();
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
                    asks_under(
                        mouse,
                        &app,
                        Modals {
                            quit: confirm,
                            scope,
                            record,
                            write,
                            ..Modals::default()
                        }
                        .current()
                    ),
                    None,
                    "{mouse:?} should mean nothing while a window is up"
                );
            }
        }

        let push = PushConfirm::open(
            "Push a brief to the board",
            warlock_engine::Destination::new("warlock-team", "Warlock", "warlock", "work"),
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
                    Shape::new(SIZE),
                    &app,
                    Modals {
                        push: &push,
                        ..Modals::default()
                    }
                    .current(),
                ),
                None,
                "{mouse:?} should mean nothing while the push dialog is up"
            );
        }
    }

    #[test]
    fn the_cut_dialog_swallows_the_gesture_too() {
        // A click that reached the tree from behind a question would select a
        // row the reader cannot see, and a wheel notch would scroll the thread
        // it is drawn over.
        let app = app_talking();
        let cut = CutConfirm::open(
            "Cut a planned project into tickets",
            "planned",
            9,
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
                    Shape::new(SIZE),
                    &app,
                    Modals {
                        cut: &cut,
                        ..Modals::default()
                    }
                    .current(),
                ),
                None,
                "{mouse:?} should mean nothing while the draft dialog is up"
            );
        }
    }

    #[test]
    fn the_two_windows_a_running_cut_puts_up_swallow_the_gesture_as_well() {
        // For the cut dialog's reason: neither has anything
        // clickable in it, and a wheel notch that reached the thread behind one
        // would scroll the very titles being answered about out of sight.
        let app = app_talking();
        let review = Review::open(
            "slice 1 `Gate the drafts`",
            vec!["Put a window in the way".to_owned()],
        );
        let carry = Carry::open("2 slices");
        for mouse in [
            left_click(IN_PANEL, FIRST_PANEL_LINE + 2),
            drag(IN_PANEL + 3, FIRST_PANEL_LINE + 3),
            release(IN_PANEL + 3, FIRST_PANEL_LINE + 3),
            wheel_down(IN_TREE, FIRST_TREE_ROW),
        ] {
            for (drafts, asked) in [(Some(&review), None), (None, Some(&carry))] {
                assert_eq!(
                    mouse_action(
                        mouse,
                        Shape::new(SIZE),
                        &app,
                        Modals {
                            review: drafts,
                            carry: asked,
                            ..Modals::default()
                        }
                        .current(),
                    ),
                    None,
                    "{mouse:?} should mean nothing while a run's window is up"
                );
            }
        }
    }

    #[test]
    fn a_click_below_the_last_row_selects_nothing() {
        // A window taller than the tree in it: one row drawn and seventeen
        // rows of blank pane under it, which is a click in the pane and no
        // more. The app is asked rather than the layout, because only the
        // app knows how many rows it has.
        let mut app = App::from_rows(vec![Row::new(0, "/repo", false)]);
        app.set_viewport_height(Shape::new(SIZE).cut().tree_height());
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
                (IN_TREE, FIRST_TREE_ROW + 1),
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
                    Modals {
                        quit: QuitConfirm::open(),
                        ..Modals::default()
                    }
                    .current()
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
                        Modals {
                            scope: &prompt,
                            ..Modals::default()
                        }
                        .current()
                    ),
                    None,
                    "{mouse:?} should mean nothing while the prompt is up"
                );
                round_under(
                    &mut app,
                    Modals {
                        scope: &prompt,
                        ..Modals::default()
                    }
                    .current(),
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
                    Modals {
                        write: &write,
                        ..Modals::default()
                    }
                    .current()
                ),
                None,
                "{mouse:?} should mean nothing while the path prompt is up"
            );
            round_under(
                &mut app,
                Modals {
                    write: &write,
                    ..Modals::default()
                }
                .current(),
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
                    Modals {
                        record: &record,
                        ..Modals::default()
                    }
                    .current()
                ),
                None,
                "{mouse:?} should mean nothing while the record window is up"
            );
            round_under(
                &mut app,
                Modals {
                    record: &record,
                    ..Modals::default()
                }
                .current(),
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

    use super::super::{Drag, MouseAction, drag_after};
    use crate::selection::Cell;
    use crate::ui::Reach;

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

mod help {
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    use super::super::{Pressed, press_for};
    use crate::composer::{Composed, Composer};
    use crate::confirm::QuitConfirm;
    use crate::help::HelpSheet;
    use crate::modal::Modals;

    fn press(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn up(scroll: u16) -> Modals<'static> {
        Modals {
            help: HelpSheet::Open(scroll),
            ..Modals::default()
        }
    }

    #[test]
    fn a_question_mark_opens_the_sheet() {
        assert_eq!(
            press_for(press(KeyCode::Char('?')), None, None, false),
            Pressed::Help(HelpSheet::open())
        );
    }

    #[test]
    fn a_question_mark_in_the_chat_bar_is_text() {
        let draft = Composer::new("web");
        assert!(matches!(
            press_for(press(KeyCode::Char('?')), None, Some(&draft), false),
            Pressed::Compose(Composed::Typing(_))
        ));
    }

    #[test]
    fn the_sheet_takes_the_keys_and_lets_none_through_to_the_tree() {
        assert_eq!(
            press_for(press(KeyCode::Char('s')), up(2).current(), None, false),
            Pressed::Help(HelpSheet::Open(2))
        );
        assert_eq!(
            press_for(press(KeyCode::Char('j')), up(2).current(), None, false),
            Pressed::Help(HelpSheet::Open(3))
        );
        assert_eq!(
            press_for(press(KeyCode::Esc), up(2).current(), None, false),
            Pressed::Help(HelpSheet::Closed)
        );
    }

    #[test]
    fn ctrl_c_asks_to_quit_over_the_sheet() {
        let ctrl_c = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert_eq!(
            press_for(ctrl_c, up(2).current(), None, false),
            Pressed::Confirm(QuitConfirm::open())
        );
    }
}
