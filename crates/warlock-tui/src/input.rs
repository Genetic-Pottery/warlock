//! What a keystroke or a mouse event asks the app to do.
//!
//! Two pure translations, one per device, and no terminal in either.
//! [`action_for`] turns a key event and a situation — is a pact in flight —
//! into an [`Action`]; [`mouse_action`] turns a mouse event, the size the
//! frame was drawn at and the app into a [`MouseAction`]. Naming the intent
//! apart from the event that produced it is what keeps both testable with
//! nothing attached to stdout, and leaves the event loop in `main.rs` reading
//! as a list of consequences.

use ratatui::crossterm::event::{
    KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::layout::Size;
use warlock_tui::{App, Focus, Hit, hit_test};

/// What a keystroke asks the app to do.
///
/// Naming the intent separately from the key that produced it keeps
/// [`action_for`] a pure function of a key event, testable with no terminal
/// attached, and leaves the loop above reading as a list of consequences.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Action {
    /// Leave the app.
    Quit,
    /// Stop the pact that is running, and stay.
    CancelPact,
    /// Move the keys from one pane to the other: the tree column and the panel
    /// beside it swap which of them is lit and which of them the movement keys
    /// drive.
    ///
    /// One action rather than a focus-the-tree and a focus-the-panel, because
    /// there is one key and two panes: with only two places focus can be, "go to
    /// the other one" is the whole of what a reader can mean by pressing it, and
    /// a pair of actions would be two names for the same keystroke read twice.
    ToggleFocus,
    /// Move the selection one row up.
    SelectPrevious,
    /// Move the selection one row down.
    SelectNext,
    /// Move the selection one screenful up.
    SelectPageUp,
    /// Move the selection one screenful down.
    SelectPageDown,
    /// Select the first row of the tree.
    SelectFirst,
    /// Select the last row of the tree.
    SelectLast,
    /// Hide the selected directory's descendants, or show them again if they
    /// are hidden already.
    ToggleCollapsed,
    /// Draw only the pacted nodes and the ancestors that reach them, or the
    /// whole tree again if that is what is on screen already.
    TogglePactedOnly,
    /// Draw the files inside each directory as well as the directories, or go
    /// back to directories alone if the files are on screen already.
    ToggleFiles,
    /// Pact the selected node, or unpact it if it is pacted already.
    TogglePact,
    /// Stop the terminal reporting its mouse, or ask it to start again if it has
    /// been stopped.
    ///
    /// The one action here that is not about the app at all: what it changes is
    /// what the terminal sends, which is why the loop answers it with an escape
    /// sequence rather than with a method on [`App`]. With capture off the
    /// terminal keeps its own selection — dragging over the screen copies text,
    /// the way it does in any other program — and warlock hears no pointer at
    /// all until the next press.
    ToggleMouseCapture,
}

/// The action `key` asks for with a pact `in_flight` or without one, or `None`
/// for a key that means nothing here.
///
/// One key reads two ways, and it is Esc. With nothing running it quits, which
/// is what it has always done and what the footer has always said. With a pact
/// running it cancels *that* — because the run is the thing in front of the
/// reader, because stopping it is the only thing they can want from a key that
/// means "not this", and because quitting outright on the key nearest to hand
/// would be the one keystroke that costs minutes of somebody else's model time
/// by mistake. Quitting during a run is still one keystroke away, spelled `q` or
/// Ctrl-C, which say what they mean and are not what a hand reaches for to stop
/// something.
///
/// The mode is a parameter rather than something looked up, so this stays a pure
/// function of a key and a situation and both readings are one assertion each.
/// Nothing else in here consults it: every other key means exactly what it meant
/// before, mid-pact included, which is what keeps the tree usable while a run
/// works.
///
/// Only presses count. Crossterm reports key releases and auto-repeats on some
/// platforms (Windows, and on terminals that speak the Kitty keyboard
/// protocol) and not on others, so acting on anything but a press would move
/// the selection twice per keystroke on those platforms and once on the rest —
/// and, since `p` writes the manifest, would toggle a pact straight back off
/// again on the release of the key that turned it on.
///
/// Ctrl-C is a key event, not a signal: raw mode is exactly the mode in which
/// the terminal stops turning it into `SIGINT`, so if this function does not
/// handle it, nothing does — including during a pact, where it is one of the two
/// ways out that also has to take the running `claude` with it.
pub(crate) fn action_for(key: KeyEvent, in_flight: bool) -> Option<Action> {
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
        // Before the quit arm below, and the only thing in here the mode
        // touches: `q` and Ctrl-C keep meaning quit while a pact runs, and Esc
        // stops being a way out for as long as there is a run to stop.
        KeyCode::Esc if in_flight => Some(Action::CancelPact),
        KeyCode::Char('q') | KeyCode::Esc => Some(Action::Quit),
        // Tab is the key every two-pane program moves focus with, and it is the
        // only new binding here: it takes no argument and asks no question, so
        // it means the same thing whether or not a pact is in flight, exactly
        // like every key below it. Shift-Tab is a different keystroke and is not
        // bound — with two panes there is no "backwards" for it to mean.
        KeyCode::Tab => Some(Action::ToggleFocus),
        KeyCode::Up | KeyCode::Char('k') => Some(Action::SelectPrevious),
        KeyCode::Down | KeyCode::Char('j') => Some(Action::SelectNext),
        KeyCode::PageUp => Some(Action::SelectPageUp),
        KeyCode::PageDown => Some(Action::SelectPageDown),
        // `g` and `G` are the pair every pager and vi-like editor has trained
        // hands for, and they are told apart by case alone: matching on the
        // character rather than on `SHIFT` keeps a terminal that reports the
        // upper-case letter without the modifier — or with it, or with caps
        // lock instead — landing on the same action, exactly as Ctrl-C above
        // does not care which of those it is handed.
        KeyCode::Char('g') => Some(Action::SelectFirst),
        KeyCode::Char('G') => Some(Action::SelectLast),
        // Space is the file-tree key everywhere, and crossterm spells it as an
        // ordinary character: there is no `KeyCode::Space`, so `Char(' ')` is
        // the whole of it. Nothing rides along that needs matching — a modifier
        // held with space is a different keystroke, not this one badly spelled.
        KeyCode::Char(' ') => Some(Action::ToggleCollapsed),
        // Lower case only, like `p` below: the upper-case letter is a
        // different keystroke and means nothing here, and a filter that also
        // answered to `O` would take a key that a later binding may want. The
        // mnemonic is "only": what stays on screen is the pacted nodes only.
        KeyCode::Char('o') => Some(Action::TogglePactedOnly),
        // Lower case only, like `o` above and `p` below. The mnemonic is
        // "files": what the key adds to the screen is the files inside each
        // module. It writes nothing and reads nothing — the files came with the
        // tree — so, unlike `p`, there is nothing here that a stray press could
        // cost anybody.
        KeyCode::Char('f') => Some(Action::ToggleFiles),
        // Lower case only, and with no confirmation: the mnemonic is the
        // product's own word (pact, §15), and the action is its own undo —
        // pressing it again removes what it wrote.
        KeyCode::Char('p') => Some(Action::TogglePact),
        // Lower case only, like the three above it. The mnemonic is "mouse",
        // and the key means the same thing whether or not a pact is in flight:
        // giving the terminal its own text selection back is exactly the thing a
        // reader wants during a long run, when there is output on screen worth
        // copying. It moves nothing, selects nothing and writes nothing.
        KeyCode::Char('m') => Some(Action::ToggleMouseCapture),
        _ => None,
    }
}

/// How far one notch of the wheel moves a pane: three rows of the tree, or
/// three lines of the panel.
///
/// One number for both panes, so the two answer at the same speed — the pointer
/// crosses from one to the other and a hand does not expect the gearing to
/// change under it. Three is what terminal programs have settled on: a row a
/// notch is a wheel that has to be spun to get anywhere, and a screenful a notch
/// is a wheel that loses the reader's place on the way.
const WHEEL_NOTCH: usize = 3;

/// What a mouse event asks the app to do.
///
/// [`Action`]'s counterpart for the pointer, and separate from the event for the
/// same reason: naming the intent apart from what produced it keeps
/// [`mouse_action`] a pure function of an event, a terminal size and the app —
/// testable with nothing attached to stdout — and leaves the loop above reading
/// as a list of consequences.
///
/// There is no variant for hovering, for dragging, or for a button other than
/// the left one. Those events are read and dropped ([`mouse_action`]), and a
/// name here for any of them would be an invitation to behaviour warlock has
/// decided against: a highlight that follows the pointer costs a redraw per
/// pointer move to say what the selection already says.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MouseAction {
    /// Move the tree's selection `.0` rows down, whichever pane has the focus.
    SelectNextBy(usize),
    /// Move the tree's selection `.0` rows up, whichever pane has the focus.
    SelectPreviousBy(usize),
    /// Scroll the panel's window `.0` lines towards the newest line, whichever
    /// pane has the focus.
    ScrollPanelDown(usize),
    /// Scroll the panel's window `.0` lines back, whichever pane has the focus.
    ScrollPanelUp(usize),
    /// Select the row at `.0` in [`App::rows`](warlock_tui::App::rows), and give
    /// the tree the keys.
    SelectRow(usize),
    /// Expand or collapse the selected row, and give the tree the keys: what a
    /// click on the row that is already selected comes to, which is what space
    /// comes to.
    ToggleCollapsed,
    /// Give the named pane the keys, and do nothing else.
    Focus(Focus),
}

/// What `mouse` over a terminal of `size` asks `app` to do, or `None` for an
/// event that means nothing here.
///
/// [`action_for`]'s counterpart, and the same shape: three things in, one
/// intention out, no terminal read and nothing drawn. The size is the one the
/// round measured before it drew, so the hit test agrees with the frame the
/// reader is pointing at rather than with a second opinion about the layout; the
/// app is here because a screen point alone cannot say which row it landed on —
/// [`Hit::TreeRow`] counts from the top of the tree's window, and only the app
/// knows where that window is and how many rows are under it.
///
/// Two events count and the rest do not. The wheel drives whichever pane the
/// pointer is *over*, focus notwithstanding — that is the whole convention of a
/// pointer, and a wheel that scrolled the focused pane instead would scroll the
/// half of the screen the reader is not looking at. The left button selects and
/// focuses. Drags, moves, releases, the other buttons and the horizontal wheel
/// are read and dropped: they are out of scope by decision, not by omission,
/// and dropping them here is what keeps a pointer swept across the screen from
/// changing anything at all.
pub(crate) fn mouse_action(mouse: MouseEvent, size: Size, app: &App) -> Option<MouseAction> {
    let hit = hit_test(mouse.column, mouse.row, size);
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

/// One notch of the wheel at `hit`: `tree` when the pointer is over the tree
/// column, `panel` when it is over the panel, and nothing anywhere else.
///
/// Which way the notch went is the caller's, because that is the only thing
/// that differs between the two directions; what this owns is the rule that the
/// pointer picks the pane. Every part of a pane's inside answers for that pane,
/// the tree's header included: a wheel is aimed at a column rather than at a
/// row, and a notch that did nothing because the pointer happened to be on the
/// one line naming the tree would read as a wheel that sticks.
///
/// The footer and the borders answer nothing, and they are the whole of what
/// does not: the footer is nobody's pane, and a border is the line between two
/// of them rather than a place a reader means to scroll.
fn wheel(hit: Hit, tree: MouseAction, panel: MouseAction) -> Option<MouseAction> {
    match hit {
        Hit::TreeHeader | Hit::TreeRow { .. } | Hit::TreeBelowRows => Some(tree),
        Hit::PanelLine { .. } => Some(panel),
        Hit::Footer | Hit::Border | Hit::Offscreen => None,
    }
}

/// One press of the left button at `hit`, given where `app` has its window and
/// its selection.
///
/// A click inside a pane always gives that pane the keys, and on the tree it may
/// do one thing more. The window offset the hit carries is turned into a row of
/// [`App::rows`](warlock_tui::App::rows) by adding
/// [`App::scroll_offset`](warlock_tui::App::scroll_offset), which is the only
/// arithmetic in here, and an offset past the last row is a point on nothing:
/// the window can be taller than the tree in it, and a click on the blank part
/// of a half-full pane is a click in the pane and no more.
///
/// A click on the row that is already selected is the reader asking for
/// something other than the selection they already have, and the thing a file
/// tree does with a second click is open or close the row. So it goes through
/// [`App::toggle_collapsed`](warlock_tui::App::toggle_collapsed) — the very
/// method space goes through, so a directory opens and closes and a row with
/// nothing under it does nothing at all, without this file having to know which
/// is which.
fn click(hit: Hit, app: &App) -> Option<MouseAction> {
    match hit {
        Hit::TreeRow { offset } => {
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
        Hit::PanelLine { .. } => Some(MouseAction::Focus(Focus::Panel)),
        Hit::Footer | Hit::Border | Hit::Offscreen => None,
    }
}

#[cfg(test)]
mod tests {
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};

    use super::{Action, action_for};

    /// A plain press of `code`, as crossterm reports one with no modifiers.
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
            KeyCode::Tab,
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
        // which is a keystroke of its own and is not bound.
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

    /// What the pointer comes to: which move a notch of the wheel or a press of
    /// the left button at a named point on a named screen asks the app for.
    ///
    /// Every test here builds its own event, names its own [`Size`] and builds
    /// its app out of rows written down in this file. No terminal is entered, no
    /// frame is drawn and nothing is attached to stdout — which is the whole
    /// reason the pointer's answer is a function of an event, a size and an app
    /// rather than something the event loop does inline.
    ///
    /// Two layers are asserted, and they are different things. Most tests ask
    /// [`mouse_action`] what a point *means*, which is the pure part. The few
    /// that care what the reader would see also go through [`round`], which is
    /// the event loop's arms written out a second time, so
    /// that "three rows a notch, clamped" and "a click on the row already
    /// selected opens it" are asserted about an app rather than about a variant
    /// name. What each of those moves does on its own is `app.rs`'s to test, and
    /// is not restated here.
    mod pointer {
        use ratatui::crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
        use ratatui::layout::Size;
        use warlock_engine::NodeState;
        use warlock_tui::{App, Focus, Row, panel_height, tree_height};

        use super::super::{MouseAction, mouse_action};

        /// The terminal every test here points at, and the layout it comes to.
        ///
        /// Eighty by twenty-four is the terminal every other program's defaults
        /// assume, and it is wide enough that the tree takes its floor of thirty
        /// columns rather than its share — the even-split branch of the layout is
        /// `ui.rs`'s to test, and what is being tested here is what a point
        /// means, not where the panes are.
        ///
        /// ```text
        /// columns  0        panel        49 50       tree        79
        /// row  0   ┌───────────────────────┐┌───────────────────────┐
        /// row  1   │ panel line 0          ││ tree header           │
        /// row  2   │ panel line 1          ││ tree row 0            │
        ///  ...     │  ...                  ││  ...                  │
        /// row 19   │ panel line 18         ││ tree row 17           │
        /// row 20   └───────────────────────┘└───────────────────────┘
        /// rows 21-23                     the footer
        /// ```
        const SIZE: Size = Size {
            width: 80,
            height: 24,
        };

        /// A column inside the tree pane, well clear of either border.
        const IN_TREE: u16 = 65;

        /// A column inside the panel, likewise.
        const IN_PANEL: u16 = 10;

        /// The screen row the tree's first drawn row is on: the pane's top
        /// border, then its header.
        const FIRST_TREE_ROW: u16 = 2;

        /// The screen row the panel's first drawn line is on: the pane's top
        /// border and no header, because the panel has none.
        const FIRST_PANEL_LINE: u16 = 1;

        /// The one line inside the tree pane's border that names the tree.
        const TREE_HEADER: u16 = 1;

        /// A row of the footer — the middle of its three.
        const FOOTER: u16 = 22;

        /// How many rows of tree this screen has room for, which is what the
        /// event loop tells the app before it draws. Asserted rather than
        /// assumed, so a layout that ever changed shape says so here.
        fn viewport() -> usize {
            usize::from(tree_height(SIZE))
        }

        /// One notch of the wheel towards the newest line, at a point.
        fn wheel_down(column: u16, row: u16) -> MouseEvent {
            event(MouseEventKind::ScrollDown, column, row)
        }

        /// One notch of the wheel back, at a point.
        fn wheel_up(column: u16, row: u16) -> MouseEvent {
            event(MouseEventKind::ScrollUp, column, row)
        }

        /// The left button going down at a point, which is the half of a click
        /// warlock answers.
        fn left_click(column: u16, row: u16) -> MouseEvent {
            event(MouseEventKind::Down(MouseButton::Left), column, row)
        }

        /// A mouse event of `kind` at a point, as crossterm reports one with no
        /// modifiers held.
        fn event(kind: MouseEventKind, column: u16, row: u16) -> MouseEvent {
            MouseEvent {
                kind,
                column,
                row,
                modifiers: KeyModifiers::NONE,
            }
        }

        /// Twenty-five rows: a root, then twenty-four directories with a file
        /// apiece.
        ///
        /// More rows than the screen above has room for, so a window that has
        /// been scrolled is a state these tests can get into; and each directory
        /// claims the one child it is given, because a row with no children is a
        /// row [`App::toggle_collapsed`] refuses, and a click on the selected
        /// row has to be tested against a row it does not refuse as well as
        /// against one it does.
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

        /// Those rows, in an app told how big the screen above is — which is
        /// what the top of the event loop does every round, and what the hit
        /// test's answers have to agree with.
        ///
        /// The file rows are hidden, as the file toggle starts, so the drawn
        /// list is the root and its twenty-four directories.
        fn app_on_screen() -> App {
            let mut app = App::from_rows(rows());
            app.set_viewport_height(tree_height(SIZE));
            app.set_panel_height(panel_height(SIZE));
            app
        }

        /// One round of the event loop with `mouse` arriving in it: the answer
        /// worked out and then done, which is the loop's arms written out again.
        ///
        /// Here so that a test can assert about a selection and a focus rather
        /// than about the name of a variant. It is the pointer's whole road, and
        /// a change to the loop that this stopped matching would be a change one
        /// of the tests below is asserting the old shape of.
        fn round(app: &mut App, mouse: MouseEvent) {
            match mouse_action(mouse, SIZE, app) {
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
                usize::from(panel_height(SIZE)),
                19,
                "nineteen lines of panel: no header of its own"
            );
        }

        #[test]
        fn a_notch_over_the_tree_moves_the_selection_three_rows() {
            let app = app_on_screen();

            assert_eq!(
                mouse_action(wheel_down(IN_TREE, FIRST_TREE_ROW + 4), SIZE, &app),
                Some(MouseAction::SelectNextBy(3)),
            );
            assert_eq!(
                mouse_action(wheel_up(IN_TREE, FIRST_TREE_ROW + 4), SIZE, &app),
                Some(MouseAction::SelectPreviousBy(3)),
            );
            // Every part of the pane's inside answers for the pane, the header
            // included: a wheel is aimed at a column, and a notch that did
            // nothing because the pointer sat on the naming line would read as a
            // wheel that sticks.
            assert_eq!(
                mouse_action(wheel_down(IN_TREE, TREE_HEADER), SIZE, &app),
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

            round(&mut app, wheel_down(IN_TREE, FIRST_TREE_ROW));
            assert_eq!(app, pressed, "a notch is three presses of the movement key");

            // Clamped at both ends rather than wrapping or running off: the
            // wheel is spun past the end far more easily than a key is held
            // there.
            for _ in 0..20 {
                round(&mut app, wheel_up(IN_TREE, FIRST_TREE_ROW));
            }
            assert_eq!(app.selected(), 0, "stopped at the first row");
            for _ in 0..20 {
                round(&mut app, wheel_down(IN_TREE, FIRST_TREE_ROW));
            }
            assert_eq!(app.selected(), app.rows().len() - 1, "stopped at the last");
        }

        #[test]
        fn a_notch_over_the_panel_scrolls_it_three_lines() {
            let app = app_on_screen();

            assert_eq!(
                mouse_action(wheel_down(IN_PANEL, FIRST_PANEL_LINE + 7), SIZE, &app),
                Some(MouseAction::ScrollPanelDown(3)),
            );
            assert_eq!(
                mouse_action(wheel_up(IN_PANEL, FIRST_PANEL_LINE), SIZE, &app),
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
            round(&mut app, wheel_down(IN_TREE, FIRST_TREE_ROW + 2));

            assert_eq!(app.selected(), 3, "the tree moved under the pointer");
            assert_eq!(app.focus(), Focus::Panel, "the keys did not follow");

            // And the other way round: the tree has the keys, the pointer is
            // over the panel, and the notch is the panel's.
            let mut app = app_on_screen();
            let selected = app.selected();
            round(&mut app, wheel_up(IN_PANEL, FIRST_PANEL_LINE));

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
                    mouse_action(wheel_down(column, row), SIZE, &app),
                    None,
                    "a notch at {column},{row} should change nothing"
                );
                assert_eq!(
                    mouse_action(wheel_up(column, row), SIZE, &app),
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
                mouse_action(left_click(IN_TREE, FIRST_TREE_ROW + 5), SIZE, &app),
                Some(MouseAction::SelectRow(5)),
                "the sixth row of a window that has not scrolled"
            );

            round(&mut app, left_click(IN_TREE, FIRST_TREE_ROW + 5));
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
                mouse_action(left_click(IN_TREE, FIRST_TREE_ROW + 3), SIZE, &app),
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
            round(&mut app, point);
            let path = app.selected_row().expect("a row is selected").path.clone();
            assert!(!app.is_collapsed(&path), "nothing collapsed by selecting");

            assert_eq!(
                mouse_action(point, SIZE, &app),
                Some(MouseAction::ToggleCollapsed),
            );
            round(&mut app, point);
            assert!(app.is_collapsed(&path), "the second click closed it");

            // And back open, which is what space does on the third press too.
            round(&mut app, point);
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
            round(&mut app, point);
            assert!(
                app.selected_row().expect("a row is selected").is_file(),
                "the third drawn row is a file"
            );

            let before = app.clone();
            round(&mut app, point);
            assert_eq!(app, before, "a file row has nothing to open");
        }

        #[test]
        fn a_click_in_the_panel_takes_the_keys_and_no_more() {
            let mut app = app_on_screen();
            let before = app.clone();

            assert_eq!(
                mouse_action(left_click(IN_PANEL, FIRST_PANEL_LINE + 9), SIZE, &app),
                Some(MouseAction::Focus(Focus::Panel)),
                "the panel has no selection, so focus is the whole of it"
            );

            round(&mut app, left_click(IN_PANEL, FIRST_PANEL_LINE + 9));
            assert_eq!(app.focus(), Focus::Panel);
            assert_eq!(app.selected(), before.selected(), "the tree did not move");
            assert_eq!(
                app.panel_scroll_offset(),
                before.panel_scroll_offset(),
                "the panel's window did not move either"
            );
        }

        #[test]
        fn a_click_on_the_tree_header_takes_the_keys_and_no_more() {
            let mut app = app_on_screen();
            app.set_focus(Focus::Panel);
            let selected = app.selected();

            assert_eq!(
                mouse_action(left_click(IN_TREE, TREE_HEADER), SIZE, &app),
                Some(MouseAction::Focus(Focus::Tree)),
            );

            round(&mut app, left_click(IN_TREE, TREE_HEADER));
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
                mouse_action(left_click(IN_TREE, FIRST_TREE_ROW + 6), SIZE, &app),
                Some(MouseAction::Focus(Focus::Tree)),
            );

            let before = app.clone();
            round(&mut app, left_click(IN_TREE, FIRST_TREE_ROW + 6));
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
                    mouse_action(left_click(column, row), SIZE, &app),
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
                        mouse_action(event(kind, column, row), SIZE, &app),
                        None,
                        "{kind:?} at {column},{row} should mean nothing"
                    );
                }
            }
        }
    }
}
