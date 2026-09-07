//! Events turned into intentions, with nothing attached to stdout:
//! [`action_for`] for a key, [`press_for`] for a key once the windows have had
//! their say, and [`mouse_action`] for a pointer event.
//!
//! Everything about the situation arrives as a parameter rather than being
//! looked up, which is what keeps each of the three a pure function and every
//! rule below one assertion. Two of those parameters are read in exactly one
//! arm each: `in_flight` re-reads Esc in [`action_for`] and `q` in
//! [`press_for`], and `answered` re-reads Ctrl-C — which [`press_for`] takes at
//! the top, before [`action_for`] is ever asked. No other key means anything
//! different while a run or a turn is out.
//!
//! Nothing here decides what a window *is* — [`answer_for`], [`edit_for`] and
//! [`compose_for`] own their own keys. [`press_for`] only decides which of them
//! is asked, and the order it asks in is the precedence.

use ratatui::crossterm::event::{
    KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::layout::Size;
use warlock_tui::{
    Answered, App, Composed, Composer, Edited, Focus, Hit, QuitConfirm, ScopePrompt, answer_for,
    compose_for, edit_for, hit_test,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Action {
    Quit,
    CancelPact,
    ToggleFocus,
    SelectPrevious,
    SelectNext,
    SelectPageUp,
    SelectPageDown,
    SelectFirst,
    SelectLast,
    ToggleCollapsed,
    TogglePactedOnly,
    ToggleFiles,
    TogglePact,
    Refresh,
    OpenScope,
    ViewFile,
    EditFile,
    SwapCard,
    ToggleMouseCapture,
}

pub(crate) fn action_for(key: KeyEvent, in_flight: bool) -> Option<Action> {
    // Presses only. Crossterm reports releases and auto-repeats on Windows and
    // on terminals speaking the Kitty protocol, and not on the rest, so a
    // release acted on would move the selection twice per keystroke on some
    // machines and once on others — and would toggle `p`'s pact straight back
    // off again on the way up.
    if key.kind != KeyEventKind::Press {
        return None;
    }

    match key.code {
        // `contains` rather than equality: shift or caps lock can ride along
        // (some terminals report the upper-case letter with it), and Ctrl-C is
        // still Ctrl-C.
        KeyCode::Char('c' | 'C') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            Some(Action::Quit)
        }
        // Before the quit arm below, and the only arm the mode touches: Esc
        // stops being a way out for as long as there is a run to stop, while
        // `q` and Ctrl-C — the keys a reader reaches for deliberately — go on
        // meaning quit.
        KeyCode::Esc if in_flight => Some(Action::CancelPact),
        KeyCode::Char('q') | KeyCode::Esc => Some(Action::Quit),
        KeyCode::Tab => Some(Action::ToggleFocus),
        // Shift-Tab is a keystroke of its own: the terminal sends a code for it
        // that crossterm spells `BackTab`, so there is no shift riding along on
        // a `Tab` here to tell the two apart by.
        KeyCode::BackTab => Some(Action::SwapCard),
        KeyCode::Up | KeyCode::Char('k') => Some(Action::SelectPrevious),
        KeyCode::Down | KeyCode::Char('j') => Some(Action::SelectNext),
        KeyCode::PageUp => Some(Action::SelectPageUp),
        KeyCode::PageDown => Some(Action::SelectPageDown),
        // The one pair told apart by case, and matched on the character rather
        // than on `SHIFT`: terminals disagree about whether the modifier rides
        // along with an upper-case letter, and about caps lock, so a `G` is a
        // `G` however it is reported.
        KeyCode::Char('g') => Some(Action::SelectFirst),
        KeyCode::Char('G') => Some(Action::SelectLast),
        // Crossterm has no `KeyCode::Space`; the space bar arrives as an
        // ordinary character.
        KeyCode::Char(' ') => Some(Action::ToggleCollapsed),
        // The letters are lower case only, `g`/`G` above excepted: `O`, `F`,
        // `P`, `R`, `S`, `V`, `E` and `M` are keystrokes of their own that mean
        // nothing here rather than second spellings of these, which leaves them
        // free for later bindings.
        KeyCode::Char('o') => Some(Action::TogglePactedOnly),
        KeyCode::Char('f') => Some(Action::ToggleFiles),
        KeyCode::Char('p') => Some(Action::TogglePact),
        KeyCode::Char('r') => Some(Action::Refresh),
        KeyCode::Char('s') => Some(Action::OpenScope),
        KeyCode::Char('v') => Some(Action::ViewFile),
        KeyCode::Char('e') => Some(Action::EditFile),
        KeyCode::Char('m') => Some(Action::ToggleMouseCapture),
        _ => None,
    }
}

// `Act` never carries `Action::Quit`: every way out is `Leave`, which is what
// makes "the gate cannot be bypassed" a property of this type rather than a
// rule the event loop is trusted to keep. `Scope` and `Write` are two variants
// over one `Edited` from one `edit_for` because the loop does different things
// with a submit from each, and an `Edited` arriving with no way to say which
// window it came from is exactly the confusion this type exists to prevent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Pressed {
    Leave,
    CancelTurn,
    Confirm(QuitConfirm),
    Scope(Edited),
    Write(Edited),
    Compose(Composed),
    Act(Action),
    Nothing,
}

// Spelled here rather than reached for through `action_for`, because
// `press_for` has to answer Ctrl-C before it consults anything else: two
// spellings of the one keystroke that must always work is how it stops working
// in one of them.
fn is_ctrl_c(key: KeyEvent) -> bool {
    key.kind == KeyEventKind::Press
        && matches!(key.code, KeyCode::Char('c' | 'C'))
        && key.modifiers.contains(KeyModifiers::CONTROL)
}

// Every kind of event, releases included, because what this decides is not what
// Tab does but which function is asked: a release handed past the composer
// comes to nothing in `action_for`, where every release already comes to
// nothing.
fn is_tab(key: KeyEvent) -> bool {
    key.code == KeyCode::Tab
}

// The order of the tests below is the precedence, and it is the whole of what
// this function decides.
//
// Ctrl-C first, before the windows: raw mode is exactly the mode in which the
// terminal stops turning it into `SIGINT`, so if nothing here answers it
// nothing does, and routed through the question it would arrive at `answer_for`
// as an ordinary `c` with a modifier riding along — the last resort of a reader
// who wants out, swallowed by the dialog. `answered` is the one thing that
// changes what it means: with a turn out it stops that turn, with none it
// leaves. Esc is deliberately not this key, because a turn and a run are two
// things and the key that stops one must not stop the other.
//
// Then each window, and on those roads `action_for` is not called at all, which
// is the plain statement of "nothing leaks through to the tree underneath". The
// scope prompt is asked before the write prompt because both can be up at once
// — `s` opens one from the tree while a `/write` turn is still out, and the
// answer to that turn opens the other with no keystroke — and the scope prompt
// is the one somebody is typing in now.
//
// The composer is asked last of the four, because a window is drawn over it: a
// key cannot be both typed into a field on the frame and answered by the dialog
// covering it. `composer` is `Some` only when the focus is on the field, which
// is the caller's line, not a lookup here.
pub(crate) fn press_for(
    key: KeyEvent,
    confirm: QuitConfirm,
    prompt: &ScopePrompt,
    write: &ScopePrompt,
    composer: Option<&Composer>,
    in_flight: bool,
    answered: bool,
) -> Pressed {
    if is_ctrl_c(key) {
        return if answered {
            Pressed::CancelTurn
        } else {
            Pressed::Leave
        };
    }

    if let Some(highlighted) = confirm.highlighted() {
        return match answer_for(key, highlighted) {
            Answered::Open(answer) => Pressed::Confirm(QuitConfirm::Open(answer)),
            Answered::Close => Pressed::Confirm(QuitConfirm::Closed),
            Answered::Leave => Pressed::Leave,
        };
    }

    if let Some(field) = prompt.field() {
        return Pressed::Scope(edit_for(key, field));
    }

    if let Some(field) = write.field() {
        return Pressed::Write(edit_for(key, field));
    }

    // Tab goes past the field rather than into it: it is not text on any
    // terminal, and a field that ate it would be a field whose only exit is
    // Esc, which means something else — hand the keyboard back and leave the
    // draft where it is.
    if let Some(draft) = composer
        && !is_tab(key)
    {
        // A muted field takes no keys and hands none on either: a `p` that fell
        // through a dead field to `action_for` would arrive at the tree as the
        // pact key. Nothing is said about it — the dim border says it once,
        // rather than the footer saying it per keystroke.
        if draft.is_muted() {
            return Pressed::Nothing;
        }
        return Pressed::Compose(compose_for(key, draft));
    }

    match action_for(key, in_flight) {
        // The gate, in two arms, and the only use `in_flight` is put to here.
        // With a run out, `q` leaves outright rather than asking: the confirm
        // is for the reflex second Esc after a cancel, not for a reader who has
        // decided, and Esc during a run is the cancel anyway (see
        // `action_for`). Ctrl-C never reaches this arm — it is taken at the top
        // of this function, where `answered` decides what it means.
        Some(Action::Quit) if !in_flight => Pressed::Confirm(QuitConfirm::open()),
        Some(Action::Quit) => Pressed::Leave,
        Some(action) => Pressed::Act(action),
        None => Pressed::Nothing,
    }
}

// One number for both panes, so the gearing does not change under a hand that
// crosses from one to the other.
const WHEEL_NOTCH: usize = 3;

// No variant for a hover, a drag, or a button other than the left one. Those
// events are read and dropped in `mouse_action`, and a name for one here would
// be an invitation to behaviour warlock has decided against — a highlight
// following the pointer costs a redraw per pointer move to say what the
// selection already says.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MouseAction {
    SelectNextBy(usize),
    SelectPreviousBy(usize),
    ScrollPanelDown(usize),
    ScrollPanelUp(usize),
    SelectRow(usize),
    ToggleCollapsed,
    Focus(Focus),
}

// `size` is the size the round measured before it drew, and `composer` the
// draft that frame was drawn with, because the hit test has to agree with the
// frame the reader is pointing at: a second opinion about the layout — or one
// that had not been told the field took rows from the panel — answers
// `PanelLine` for a point on the composer and scrolls a window the pointer is
// not over.
//
// None of the three windows has anything clickable in it, so while any is up
// every event is dropped, wheel and click alike: a click that reached the tree
// behind one would select a row the reader cannot see.
pub(crate) fn mouse_action(
    mouse: MouseEvent,
    size: Size,
    app: &App,
    confirm: QuitConfirm,
    prompt: &ScopePrompt,
    write: &ScopePrompt,
    composer: Option<&Composer>,
) -> Option<MouseAction> {
    if confirm.is_open() || prompt.is_open() || write.is_open() {
        return None;
    }

    let header = app.run_header();
    let hit = hit_test(mouse.column, mouse.row, size, composer, header.as_ref());
    match mouse.kind {
        // Down the tree and down the account are the same direction, so one
        // notch reads the same way over either pane.
        MouseEventKind::ScrollDown => wheel(
            hit,
            MouseAction::SelectNextBy(WHEEL_NOTCH),
            MouseAction::ScrollPanelDown(WHEEL_NOTCH),
        ),
        MouseEventKind::ScrollUp => wheel(
            hit,
            MouseAction::SelectPreviousBy(WHEEL_NOTCH),
            MouseAction::ScrollPanelUp(WHEEL_NOTCH),
        ),
        // The press, not the release: it is the half of a click a reader means,
        // and answering both would do everything twice.
        MouseEventKind::Down(MouseButton::Left) => click(hit, app),
        _ => None,
    }
}

// The wheel drives whichever pane the pointer is over, focus notwithstanding,
// and every part of a pane's inside answers for it — headers included, because
// a notch that did nothing on the one line naming the tree reads as a wheel
// that sticks. The composer answers nothing: it scrolls itself as somebody
// types, and scrolling the account because the pointer was resting on the field
// would move the half of the screen the reader is not pointing at.
fn wheel(hit: Hit, tree: MouseAction, panel: MouseAction) -> Option<MouseAction> {
    match hit {
        Hit::TreeHeader | Hit::TreeRow { .. } | Hit::TreeBelowRows => Some(tree),
        Hit::PanelHeader | Hit::PanelLine { .. } => Some(panel),
        Hit::Composer | Hit::Footer | Hit::Border | Hit::Offscreen => None,
    }
}

fn click(hit: Hit, app: &App) -> Option<MouseAction> {
    match hit {
        Hit::TreeRow { offset } => {
            // The hit counts from the top of the tree's window, so only the
            // app's scroll offset turns it into a row. The window can be taller
            // than the tree in it: an index past the last row is a click on the
            // blank part of a half-full pane, which is a click in the pane and
            // no more. A second click on the row already selected is the reader
            // asking for something other than the selection they have, and what
            // a file tree does then is open or close it — through the very
            // method space goes through, so a row with nothing under it does
            // nothing at all.
            let index = app.scroll_offset().saturating_add(usize::from(offset));
            if index >= app.rows().len() {
                Some(MouseAction::Focus(Focus::Tree))
            } else if index == app.selected() {
                Some(MouseAction::ToggleCollapsed)
            } else {
                Some(MouseAction::SelectRow(index))
            }
        }
        Hit::TreeHeader | Hit::TreeBelowRows => Some(MouseAction::Focus(Focus::Tree)),
        Hit::PanelHeader | Hit::PanelLine { .. } => Some(MouseAction::Focus(Focus::Panel)),
        // The composer is hit-tested only when it is on screen, so a press on it
        // is somebody pointing at the field they mean to type in.
        Hit::Composer => Some(MouseAction::Focus(Focus::Composer)),
        Hit::Footer | Hit::Border | Hit::Offscreen => None,
    }
}

#[cfg(test)]
mod tests {
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

        use ratatui::crossterm::event::{
            KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers,
        };
        use ratatui::layout::Size;
        use warlock_engine::NodeState;
        use warlock_tui::{
            Answer, App, Composed, Composer, Edited, Focus, QuitConfirm, Row, ScopeField,
            ScopePrompt, edit_for, panel_height, tree_height,
        };

        use super::super::{Action, Pressed, action_for, press_for as gate_for};

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
                Row::new(0, "/repo", "/repo/WARLOCK.md", NodeState::PactedStale)
                    .with_child_count(12),
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
                Action, Composer, Edited, INERT, KeyCode, KeyEvent, Pressed, QuitConfirm,
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
                        gate_for(key, QuitConfirm::Closed, &scope, &write, None, false, false,),
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

        mod composing {
            use std::time::Instant;

            use super::{
                Action, App, Composed, Composer, Focus, INERT, KeyCode, Pressed, QuitConfirm,
                Round, ScopePrompt, app_in_use, app_on_screen, ctrl_c, offered, press, press_for,
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
        use ratatui::crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
        use ratatui::layout::Size;
        use warlock_engine::NodeState;
        use warlock_tui::{App, Focus, QuitConfirm, Row, ScopePrompt, panel_height, tree_height};

        use super::super::{MouseAction, mouse_action};

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

        fn event(kind: MouseEventKind, column: u16, row: u16) -> MouseEvent {
            MouseEvent {
                kind,
                column,
                row,
                modifiers: KeyModifiers::NONE,
            }
        }

        fn rows() -> Vec<Row> {
            let mut rows = vec![
                Row::new(0, "/repo", "/repo/WARLOCK.md", NodeState::PactedStale)
                    .with_child_count(24),
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
            app
        }

        fn asks(mouse: MouseEvent, app: &App) -> Option<MouseAction> {
            mouse_action(
                mouse,
                SIZE,
                app,
                QuitConfirm::Closed,
                &ScopePrompt::Closed,
                &ScopePrompt::Closed,
                None,
            )
        }

        fn round(app: &mut App, confirm: QuitConfirm, mouse: MouseEvent) {
            round_under(
                app,
                confirm,
                &ScopePrompt::Closed,
                &ScopePrompt::Closed,
                mouse,
            );
        }

        fn round_under(
            app: &mut App,
            confirm: QuitConfirm,
            prompt: &ScopePrompt,
            write: &ScopePrompt,
            mouse: MouseEvent,
        ) {
            match mouse_action(mouse, SIZE, app, confirm, prompt, write, None) {
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
                None => {}
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
            // The footer is nobody's pane; a border is the line between two of
            // them rather than a place a reader means to scroll. The columns are
            // the panel's left border, the two panes' shared edge and the tree's
            // right, and the rows are the panes' top and bottom.
            for (column, row) in [
                (IN_PANEL, FOOTER),
                (IN_TREE, FOOTER),
                (0, FIRST_PANEL_LINE),
                (49, FIRST_TREE_ROW),
                (50, FIRST_TREE_ROW),
                (79, FIRST_TREE_ROW),
                (IN_TREE, 0),
                (IN_PANEL, 20),
            ] {
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
            for (column, row) in [
                (IN_PANEL, FOOTER),
                (IN_TREE, FOOTER),
                (0, FIRST_PANEL_LINE),
                (49, FIRST_TREE_ROW),
                (50, FIRST_TREE_ROW),
                (79, FIRST_TREE_ROW),
                (IN_TREE, 0),
                (IN_PANEL, 20),
            ] {
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
            // Out of scope by decision: hovering, dragging, the release half of
            // a click, the other two buttons and the horizontal wheel. Asked at
            // every kind of point, because dropping them is what keeps a pointer
            // swept across the screen from costing anything — a highlight that
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

            for mouse in [
                wheel_down(IN_TREE, FIRST_TREE_ROW),
                wheel_up(IN_TREE, FIRST_TREE_ROW),
                wheel_down(IN_PANEL, FIRST_PANEL_LINE),
                wheel_up(IN_PANEL, FIRST_PANEL_LINE),
                left_click(IN_TREE, FIRST_TREE_ROW),
                left_click(IN_TREE, FIRST_TREE_ROW + 9),
                left_click(IN_TREE, TREE_HEADER),
                left_click(IN_PANEL, FIRST_PANEL_LINE + 3),
            ] {
                assert_eq!(
                    mouse_action(
                        mouse,
                        SIZE,
                        &app,
                        QuitConfirm::open(),
                        &ScopePrompt::Closed,
                        &ScopePrompt::Closed,
                        None
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
                for mouse in [
                    wheel_down(IN_TREE, FIRST_TREE_ROW),
                    wheel_up(IN_TREE, FIRST_TREE_ROW),
                    wheel_down(IN_PANEL, FIRST_PANEL_LINE),
                    wheel_up(IN_PANEL, FIRST_PANEL_LINE),
                    left_click(IN_TREE, FIRST_TREE_ROW),
                    left_click(IN_TREE, FIRST_TREE_ROW + 9),
                    left_click(IN_TREE, TREE_HEADER),
                    left_click(IN_PANEL, FIRST_PANEL_LINE + 3),
                ] {
                    assert_eq!(
                        mouse_action(
                            mouse,
                            SIZE,
                            &app,
                            QuitConfirm::Closed,
                            &prompt,
                            &ScopePrompt::Closed,
                            None
                        ),
                        None,
                        "{mouse:?} should mean nothing while the prompt is up"
                    );
                    round_under(
                        &mut app,
                        QuitConfirm::Closed,
                        &prompt,
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

            for mouse in [
                wheel_down(IN_TREE, FIRST_TREE_ROW),
                wheel_up(IN_TREE, FIRST_TREE_ROW),
                wheel_down(IN_PANEL, FIRST_PANEL_LINE),
                wheel_up(IN_PANEL, FIRST_PANEL_LINE),
                left_click(IN_TREE, FIRST_TREE_ROW),
                left_click(IN_TREE, FIRST_TREE_ROW + 9),
                left_click(IN_TREE, TREE_HEADER),
                left_click(IN_PANEL, FIRST_PANEL_LINE + 3),
            ] {
                assert_eq!(
                    mouse_action(
                        mouse,
                        SIZE,
                        &app,
                        QuitConfirm::Closed,
                        &ScopePrompt::Closed,
                        &write,
                        None
                    ),
                    None,
                    "{mouse:?} should mean nothing while the path prompt is up"
                );
                round_under(
                    &mut app,
                    QuitConfirm::Closed,
                    &ScopePrompt::Closed,
                    &write,
                    mouse,
                );
            }

            assert_eq!(app, before, "the pointer moved nothing behind the prompt");
        }
    }
}
