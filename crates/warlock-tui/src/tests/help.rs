use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use super::*;

fn press(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

#[test]
fn esc_and_question_mark_close_it() {
    for code in [KeyCode::Esc, KeyCode::Char('?')] {
        assert_eq!(help_for(press(code), 4), HelpSheet::Closed, "{code:?}");
    }
}

#[test]
fn q_does_not_close_it() {
    assert_eq!(help_for(press(KeyCode::Char('q')), 4), HelpSheet::Open(4));
}

#[test]
fn the_movement_keys_scroll_it() {
    for (code, scroll) in [
        (KeyCode::Down, 5),
        (KeyCode::Char('j'), 5),
        (KeyCode::Up, 3),
        (KeyCode::Char('k'), 3),
        (KeyCode::PageDown, 14),
        (KeyCode::PageUp, 0),
        (KeyCode::Char('g'), 0),
        (KeyCode::Char('G'), u16::MAX),
    ] {
        assert_eq!(
            help_for(press(code), 4),
            HelpSheet::Open(scroll),
            "{code:?}"
        );
    }
}

#[test]
fn a_release_changes_nothing() {
    let mut esc = press(KeyCode::Esc);
    esc.kind = KeyEventKind::Release;
    assert_eq!(help_for(esc, 4), HelpSheet::Open(4));
}

#[test]
fn clamping_holds_the_scroll_at_the_ceiling_and_leaves_closed_closed() {
    assert_eq!(HelpSheet::Open(u16::MAX).clamped(12), HelpSheet::Open(12));
    assert_eq!(HelpSheet::Open(3).clamped(12), HelpSheet::Open(3));
    assert_eq!(HelpSheet::Closed.clamped(12), HelpSheet::Closed);
}
