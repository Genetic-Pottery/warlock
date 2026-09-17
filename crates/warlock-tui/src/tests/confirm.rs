use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};

use super::{Answer, Answered, QuitConfirm, answer_for};

fn press(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

// Every key the tree answers to, plus a character bound to nothing anywhere:
// the list the confirmation has to swallow whole.
const INERT: [KeyCode; 15] = [
    KeyCode::Char('j'),
    KeyCode::Char('k'),
    KeyCode::Char('g'),
    KeyCode::Char('G'),
    KeyCode::Char(' '),
    KeyCode::Char('o'),
    KeyCode::Char('f'),
    KeyCode::Char('p'),
    KeyCode::Char('m'),
    KeyCode::Tab,
    KeyCode::PageUp,
    KeyCode::PageDown,
    KeyCode::Up,
    KeyCode::Down,
    KeyCode::Char('x'),
];

#[test]
fn a_fresh_confirmation_is_up_with_no_highlighted() {
    // The dangerous answer is never the one under the reader's finger: an
    // Enter straight after the key that opened this comes to nothing.
    assert_eq!(QuitConfirm::open(), QuitConfirm::Open(Answer::No));
    assert!(QuitConfirm::open().is_open());
    assert_eq!(QuitConfirm::open().highlighted(), Some(Answer::No));
    assert_eq!(
        answer_for(press(KeyCode::Enter), Answer::No),
        Answered::Close
    );
}

#[test]
fn a_closed_confirmation_is_the_default_and_lights_nothing() {
    // `Default` is what the event loop starts a session on, and a closed
    // question has no highlight to draw or to hand to `answer_for`.
    assert_eq!(QuitConfirm::default(), QuitConfirm::Closed);
    assert!(!QuitConfirm::Closed.is_open());
    assert_eq!(QuitConfirm::Closed.highlighted(), None);
}

#[test]
fn left_lights_yes_and_right_lights_no_from_either_side() {
    // Positional, not a toggle: an arrow points at a side, and pressing it
    // twice cannot walk the highlight off the side it points at.
    for lit in [Answer::Yes, Answer::No] {
        assert_eq!(
            answer_for(press(KeyCode::Left), lit),
            Answered::Open(Answer::Yes),
            "Left should light Yes with {lit:?} lit"
        );
        assert_eq!(
            answer_for(press(KeyCode::Right), lit),
            Answered::Open(Answer::No),
            "Right should light No with {lit:?} lit"
        );
    }
}

#[test]
fn enter_takes_the_answer_that_is_lit() {
    assert_eq!(
        answer_for(press(KeyCode::Enter), Answer::Yes),
        Answered::Leave
    );
    assert_eq!(
        answer_for(press(KeyCode::Enter), Answer::No),
        Answered::Close
    );
}

#[test]
fn y_and_n_answer_outright_whichever_is_lit() {
    // The reader who already knows the answer should not have to look at
    // the screen to find out which way the highlight is facing.
    for lit in [Answer::Yes, Answer::No] {
        assert_eq!(
            answer_for(press(KeyCode::Char('y')), lit),
            Answered::Leave,
            "y should leave with {lit:?} lit"
        );
        assert_eq!(
            answer_for(press(KeyCode::Char('n')), lit),
            Answered::Close,
            "n should stay with {lit:?} lit"
        );
    }
}

#[test]
fn upper_case_y_and_n_answer_the_same_way() {
    // Caps lock, or shift held out of habit, is not a reason for the way
    // out to stop working: the pair is matched by character, like `g`/`G`.
    assert_eq!(
        answer_for(press(KeyCode::Char('Y')), Answer::No),
        Answered::Leave
    );
    assert_eq!(
        answer_for(press(KeyCode::Char('N')), Answer::Yes),
        Answered::Close
    );
}

#[test]
fn esc_answers_no_from_either_side() {
    // The whole point of the gate: the reflex second Esc closes the
    // question rather than the session, even with Yes lit.
    for lit in [Answer::Yes, Answer::No] {
        assert_eq!(
            answer_for(press(KeyCode::Esc), lit),
            Answered::Close,
            "Esc should answer No with {lit:?} lit"
        );
    }
}

#[test]
fn every_other_key_leaves_the_question_exactly_as_it_was() {
    // Asserted at both highlight positions, because "nothing happens" has
    // to include the highlight not moving as well as the question staying
    // up: a stray `j` that shuffled the answer under the reader's finger
    // would be the same accident by another route.
    for lit in [Answer::Yes, Answer::No] {
        for code in INERT {
            assert_eq!(
                answer_for(press(code), lit),
                Answered::Open(lit),
                "{code:?} should change nothing with {lit:?} lit"
            );
        }
    }
}

#[test]
fn ctrl_c_is_not_answered_here() {
    // It is a key event, not a signal, and the loop has to take it before
    // it consults this mode. Through here it is an ordinary `c` with a
    // modifier riding along, i.e. one of the keys that change nothing —
    // which is why the loop answering it first is load-bearing rather than
    // tidy.
    let ctrl_c = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);

    for lit in [Answer::Yes, Answer::No] {
        assert_eq!(answer_for(ctrl_c, lit), Answered::Open(lit));
    }
}

#[test]
fn releases_and_repeats_answer_nothing() {
    // The same rule as `action_for`, and here it is the difference between
    // a gate and no gate: acting on a release would answer the question
    // with the release of the very key that opened it, and a held `y`
    // would leave as fast as the terminal repeats.
    for code in [
        KeyCode::Enter,
        KeyCode::Esc,
        KeyCode::Char('y'),
        KeyCode::Char('n'),
        KeyCode::Left,
        KeyCode::Right,
    ] {
        for kind in [KeyEventKind::Release, KeyEventKind::Repeat] {
            let event = KeyEvent::new_with_kind_and_state(
                code,
                KeyModifiers::NONE,
                kind,
                KeyEventState::NONE,
            );

            assert_eq!(
                answer_for(event, Answer::No),
                Answered::Open(Answer::No),
                "{kind:?} of {code:?} should answer nothing"
            );
        }
    }
}

#[test]
fn nothing_but_yes_and_enter_on_yes_ever_leaves() {
    // Said the other way round, because what must not happen is the
    // interesting half: of every key this module sees, only two spellings
    // of Yes get anybody out.
    for lit in [Answer::Yes, Answer::No] {
        for code in INERT.into_iter().chain([
            KeyCode::Esc,
            KeyCode::Char('n'),
            KeyCode::Left,
            KeyCode::Right,
        ]) {
            assert_ne!(
                answer_for(press(code), lit),
                Answered::Leave,
                "{code:?} should not leave with {lit:?} lit"
            );
        }
    }
    assert_eq!(
        answer_for(press(KeyCode::Enter), Answer::No),
        Answered::Close,
        "Enter on No stays, so the default answer is the safe one"
    );
}
