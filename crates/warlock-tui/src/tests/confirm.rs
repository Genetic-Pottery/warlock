use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};

use super::{
    Answer, Answered, PullAnswered, PullConfirm, PushAnswered, PushConfirm, QuitConfirm,
    answer_for, pull_answer_for, push_answer_for,
};

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

// The push dialog, which is the quit question's rules answered about something
// else. Every test in here is one of the tests above asked again, because
// "answered by the same rules" is a claim about behaviour and not about which
// function the body happens to call.
mod push {
    use super::{
        Answer, INERT, KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers, PushAnswered,
        PushConfirm, press, push_answer_for,
    };

    const PROJECT: &str = "Push a brief to the board";

    const SCOPE: &str = "warlock-team";

    const TEAM: &str = "Warlock";

    const KEY: &str = "work";

    fn open() -> PushConfirm {
        PushConfirm::open(PROJECT, SCOPE, TEAM, KEY)
    }

    // The dialog answering one key, as the session answers it: the lit answer
    // comes out of the value that is up rather than from the test.
    fn answered(push: &PushConfirm, code: KeyCode) -> PushAnswered {
        let filing = push.filing().expect("the dialog under test is up");
        push_answer_for(press(code), filing.answer())
    }

    #[test]
    fn a_fresh_dialog_is_up_with_no_highlighted_and_carries_what_it_asks_about() {
        let push = open();
        let filing = push.filing().expect("an opened dialog is up");

        assert!(push.is_open());
        assert_eq!(filing.answer(), Answer::No);
        assert_eq!(filing.project(), PROJECT);
        assert_eq!(filing.team(), TEAM);
        // The key by name. There is nowhere in the value for its bytes, which
        // is why nothing downstream can print them.
        assert_eq!(filing.key(), KEY);
    }

    #[test]
    fn a_closed_dialog_is_the_default_and_has_nothing_to_answer() {
        assert_eq!(PushConfirm::default(), PushConfirm::Closed);
        assert!(!PushConfirm::Closed.is_open());
        assert!(PushConfirm::Closed.filing().is_none());
        // An arrow pressed at a window that is not up lights nothing, rather
        // than conjuring a question out of three strings nobody supplied.
        assert_eq!(PushConfirm::Closed.lit(Answer::Yes), PushConfirm::Closed);
    }

    #[test]
    fn an_immediate_enter_cancels() {
        // The keystroke that opened this and the Enter straight after it both
        // come to nothing: No is lit, so nothing is sent.
        assert_eq!(answered(&open(), KeyCode::Enter), PushAnswered::Cancel);
    }

    #[test]
    fn esc_cancels_from_either_side() {
        for lit in [Answer::Yes, Answer::No] {
            assert_eq!(
                answered(&open().lit(lit), KeyCode::Esc),
                PushAnswered::Cancel,
                "Esc should cancel with {lit:?} lit"
            );
        }
    }

    #[test]
    fn left_then_enter_sends_and_right_goes_back_to_no() {
        let push = open();

        assert_eq!(
            answered(&push, KeyCode::Left),
            PushAnswered::Open(Answer::Yes)
        );
        let armed = push.lit(Answer::Yes);
        assert_eq!(answered(&armed, KeyCode::Enter), PushAnswered::Send);

        assert_eq!(
            answered(&armed, KeyCode::Right),
            PushAnswered::Open(Answer::No)
        );
        assert_eq!(
            answered(&armed.lit(Answer::No), KeyCode::Enter),
            PushAnswered::Cancel
        );
    }

    #[test]
    fn y_and_n_answer_outright_whichever_is_lit() {
        for lit in [Answer::Yes, Answer::No] {
            let push = open().lit(lit);
            assert_eq!(
                answered(&push, KeyCode::Char('y')),
                PushAnswered::Send,
                "y should send with {lit:?} lit"
            );
            assert_eq!(
                answered(&push, KeyCode::Char('n')),
                PushAnswered::Cancel,
                "n should cancel with {lit:?} lit"
            );
        }
    }

    #[test]
    fn moving_the_highlight_keeps_what_the_question_is_about() {
        // The three strings ride along: the dialog re-lit is the same dialog,
        // not a second one built from whatever the caller still had.
        let moved = open().lit(Answer::Yes);
        let filing = moved.filing().expect("a re-lit dialog is still up");

        assert_eq!(filing.answer(), Answer::Yes);
        assert_eq!(filing.project(), PROJECT);
        assert_eq!(filing.team(), TEAM);
        assert_eq!(filing.key(), KEY);
        assert_eq!(moved.lit(Answer::No), open());
    }

    #[test]
    fn every_other_key_leaves_the_question_exactly_as_it_was() {
        for lit in [Answer::Yes, Answer::No] {
            let push = open().lit(lit);
            for code in INERT {
                assert_eq!(
                    answered(&push, code),
                    PushAnswered::Open(lit),
                    "{code:?} should change nothing with {lit:?} lit"
                );
            }
        }
    }

    #[test]
    fn ctrl_c_is_not_answered_here() {
        // The loop takes it before this window, as it does before the quit
        // question: through here it is an ordinary `c` and changes nothing.
        let ctrl_c = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);

        for lit in [Answer::Yes, Answer::No] {
            assert_eq!(push_answer_for(ctrl_c, lit), PushAnswered::Open(lit));
        }
    }

    #[test]
    fn releases_and_repeats_answer_nothing() {
        // Including the release of the very key that opened the dialog: a
        // question answered by its own opening keystroke is no question.
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
                    push_answer_for(event, Answer::No),
                    PushAnswered::Open(Answer::No),
                    "{kind:?} of {code:?} should answer nothing"
                );
            }
        }
    }

    #[test]
    fn nothing_but_yes_and_enter_on_yes_ever_sends() {
        for lit in [Answer::Yes, Answer::No] {
            let push = open().lit(lit);
            for code in INERT.into_iter().chain([
                KeyCode::Esc,
                KeyCode::Char('n'),
                KeyCode::Left,
                KeyCode::Right,
            ]) {
                assert_ne!(
                    answered(&push, code),
                    PushAnswered::Send,
                    "{code:?} should not send with {lit:?} lit"
                );
            }
        }
        assert_eq!(
            answered(&open(), KeyCode::Enter),
            PushAnswered::Cancel,
            "Enter on No cancels, so the default answer sends nothing"
        );
    }
}

// The pull dialog, which is those same rules answered about a project. Every
// test in here is one of the tests above asked again, for the reason the push
// dialog's are: "answered by the same rules" is a claim about behaviour and not
// about which function the body happens to call.
mod pull {
    use super::{
        Answer, INERT, KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers, PullAnswered,
        PullConfirm, press, pull_answer_for,
    };

    const PROJECT: &str = "Cut a planned project into tickets";

    // The board's own spelling, which is what the fetch found written on the
    // project rather than what the gate folds it to.
    const STATUS: &str = "planned";

    const SLICES: usize = 9;

    const TEAM: &str = "Warlock";

    const KEY: &str = "work";

    // A value no dialog holds and no rendering can therefore contain: the
    // question carries the name a key is held under and nothing else.
    const KEY_VALUE: &str = "not-a-real-key-value";

    fn open() -> PullConfirm {
        PullConfirm::open(PROJECT, STATUS, SLICES, TEAM, KEY)
    }

    // The dialog answering one key, as the session answers it: the lit answer
    // comes out of the value that is up rather than from the test.
    fn answered(pull: &PullConfirm, code: KeyCode) -> PullAnswered {
        let cutting = pull.cutting().expect("the dialog under test is up");
        pull_answer_for(press(code), cutting.answer())
    }

    #[test]
    fn a_fresh_dialog_is_up_with_no_highlighted_and_carries_the_five_facts() {
        let pull = open();
        let cutting = pull.cutting().expect("an opened dialog is up");

        assert!(pull.is_open());
        assert_eq!(cutting.answer(), Answer::No);
        assert_eq!(cutting.project(), PROJECT);
        assert_eq!(cutting.status(), STATUS);
        assert_eq!(cutting.slices(), SLICES);
        assert_eq!(cutting.team(), TEAM);
        // The key by name. There is nowhere in the value for its bytes, which
        // is why nothing downstream can print them.
        assert_eq!(cutting.key(), KEY);
    }

    #[test]
    fn there_is_nowhere_in_the_question_for_a_key_value() {
        // The claim as a reader would check it: the dialog is built from the
        // name alone, so no arrangement of these five strings puts a key into
        // a `Debug` rendering — the one place a value that held one would leak
        // into a panic message or a failing assertion.
        let rendered = format!("{:?}", open().lit(Answer::Yes));

        assert!(rendered.contains(KEY), "{rendered:?} drops the key's name");
        assert!(
            !rendered.contains(KEY_VALUE),
            "{rendered:?} holds a key value"
        );
    }

    #[test]
    fn a_closed_dialog_is_the_default_and_has_nothing_to_answer() {
        assert_eq!(PullConfirm::default(), PullConfirm::Closed);
        assert!(!PullConfirm::Closed.is_open());
        assert!(PullConfirm::Closed.cutting().is_none());
        // An arrow pressed at a window that is not up lights nothing, rather
        // than conjuring a question out of facts nobody fetched.
        assert_eq!(PullConfirm::Closed.lit(Answer::Yes), PullConfirm::Closed);
    }

    #[test]
    fn an_immediate_enter_answers_no() {
        // The round that put this up and the Enter straight after it both come
        // to nothing: No is lit, so no run starts.
        assert_eq!(answered(&open(), KeyCode::Enter), PullAnswered::Cancel);
    }

    #[test]
    fn esc_answers_no_from_either_side() {
        for lit in [Answer::Yes, Answer::No] {
            assert_eq!(
                answered(&open().lit(lit), KeyCode::Esc),
                PullAnswered::Cancel,
                "Esc should answer No with {lit:?} lit"
            );
        }
    }

    #[test]
    fn left_then_enter_cuts_and_right_goes_back_to_no() {
        let pull = open();

        assert_eq!(
            answered(&pull, KeyCode::Left),
            PullAnswered::Open(Answer::Yes)
        );
        let armed = pull.lit(Answer::Yes);
        assert_eq!(answered(&armed, KeyCode::Enter), PullAnswered::Cut);

        assert_eq!(
            answered(&armed, KeyCode::Right),
            PullAnswered::Open(Answer::No)
        );
        assert_eq!(
            answered(&armed.lit(Answer::No), KeyCode::Enter),
            PullAnswered::Cancel
        );
    }

    #[test]
    fn y_and_n_answer_outright_whichever_is_lit() {
        for lit in [Answer::Yes, Answer::No] {
            let pull = open().lit(lit);
            assert_eq!(
                answered(&pull, KeyCode::Char('y')),
                PullAnswered::Cut,
                "y should cut with {lit:?} lit"
            );
            assert_eq!(
                answered(&pull, KeyCode::Char('n')),
                PullAnswered::Cancel,
                "n should answer No with {lit:?} lit"
            );
        }
    }

    #[test]
    fn moving_the_highlight_keeps_what_the_question_is_about() {
        // The five facts ride along: the dialog re-lit is the same dialog, not
        // a second one built from whatever a second request might answer.
        let moved = open().lit(Answer::Yes);
        let cutting = moved.cutting().expect("a re-lit dialog is still up");

        assert_eq!(cutting.answer(), Answer::Yes);
        assert_eq!(cutting.project(), PROJECT);
        assert_eq!(cutting.status(), STATUS);
        assert_eq!(cutting.slices(), SLICES);
        assert_eq!(cutting.team(), TEAM);
        assert_eq!(cutting.key(), KEY);
        assert_eq!(moved.lit(Answer::No), open());
    }

    #[test]
    fn every_other_key_leaves_the_question_exactly_as_it_was() {
        for lit in [Answer::Yes, Answer::No] {
            let pull = open().lit(lit);
            for code in INERT {
                assert_eq!(
                    answered(&pull, code),
                    PullAnswered::Open(lit),
                    "{code:?} should change nothing with {lit:?} lit"
                );
            }
        }
    }

    #[test]
    fn ctrl_c_is_not_answered_here() {
        // The loop takes it before this window, as it does before the other
        // two: through here it is an ordinary `c` and changes nothing.
        let ctrl_c = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);

        for lit in [Answer::Yes, Answer::No] {
            assert_eq!(pull_answer_for(ctrl_c, lit), PullAnswered::Open(lit));
        }
    }

    #[test]
    fn releases_and_repeats_answer_nothing() {
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
                    pull_answer_for(event, Answer::No),
                    PullAnswered::Open(Answer::No),
                    "{kind:?} of {code:?} should answer nothing"
                );
            }
        }
    }

    #[test]
    fn nothing_but_yes_and_enter_on_yes_ever_cuts() {
        for lit in [Answer::Yes, Answer::No] {
            let pull = open().lit(lit);
            for code in INERT.into_iter().chain([
                KeyCode::Esc,
                KeyCode::Char('n'),
                KeyCode::Left,
                KeyCode::Right,
            ]) {
                assert_ne!(
                    answered(&pull, code),
                    PullAnswered::Cut,
                    "{code:?} should not cut with {lit:?} lit"
                );
            }
        }
        assert_eq!(
            answered(&open(), KeyCode::Enter),
            PullAnswered::Cancel,
            "Enter on No answers No, so the default answer starts nothing"
        );
    }
}
