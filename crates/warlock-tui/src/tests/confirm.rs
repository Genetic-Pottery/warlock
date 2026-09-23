use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};

use super::{
    Answer, Answered, Carry, CarryAnswered, Choice, CutAnswered, CutConfirm, PushAnswered,
    PushConfirm, QuitConfirm, Review, Reviewed, answer_for, carry_answer_for, cut_answer_for,
    push_answer_for, review_answer_for,
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
        PushConfirm::open(
            PROJECT,
            warlock_engine::Destination::new(SCOPE, TEAM, "warlock", KEY),
        )
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
        assert_eq!(filing.destination().team(), TEAM);
        // The key by name. There is nowhere in the value for its bytes, which
        // is why nothing downstream can print them.
        assert_eq!(filing.destination().key(), KEY);
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
        assert_eq!(filing.destination().team(), TEAM);
        assert_eq!(filing.destination().key(), KEY);
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

// The cut dialog, which is those same rules answered about a project. Every
// test in here is one of the tests above asked again, for the reason the push
// dialog's are: "answered by the same rules" is a claim about behaviour and not
// about which function the body happens to call.
mod cut {
    use super::{
        Answer, CutAnswered, CutConfirm, INERT, KeyCode, KeyEvent, KeyEventKind, KeyEventState,
        KeyModifiers, cut_answer_for, press,
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

    fn open() -> CutConfirm {
        CutConfirm::open(PROJECT, STATUS, SLICES, TEAM, KEY)
    }

    // The dialog answering one key, as the session answers it: the lit answer
    // comes out of the value that is up rather than from the test.
    fn answered(cut: &CutConfirm, code: KeyCode) -> CutAnswered {
        let cutting = cut.cutting().expect("the dialog under test is up");
        cut_answer_for(press(code), cutting.answer())
    }

    #[test]
    fn a_fresh_dialog_is_up_with_no_highlighted_and_carries_the_five_facts() {
        let cut = open();
        let cutting = cut.cutting().expect("an opened dialog is up");

        assert!(cut.is_open());
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
        assert_eq!(CutConfirm::default(), CutConfirm::Closed);
        assert!(!CutConfirm::Closed.is_open());
        assert!(CutConfirm::Closed.cutting().is_none());
        // An arrow pressed at a window that is not up lights nothing, rather
        // than conjuring a question out of facts nobody fetched.
        assert_eq!(CutConfirm::Closed.lit(Answer::Yes), CutConfirm::Closed);
    }

    #[test]
    fn an_immediate_enter_answers_no() {
        // The round that put this up and the Enter straight after it both come
        // to nothing: No is lit, so no run starts.
        assert_eq!(answered(&open(), KeyCode::Enter), CutAnswered::Cancel);
    }

    #[test]
    fn esc_answers_no_from_either_side() {
        for lit in [Answer::Yes, Answer::No] {
            assert_eq!(
                answered(&open().lit(lit), KeyCode::Esc),
                CutAnswered::Cancel,
                "Esc should answer No with {lit:?} lit"
            );
        }
    }

    #[test]
    fn left_then_enter_cuts_and_right_goes_back_to_no() {
        let cut = open();

        assert_eq!(
            answered(&cut, KeyCode::Left),
            CutAnswered::Open(Answer::Yes)
        );
        let armed = cut.lit(Answer::Yes);
        assert_eq!(answered(&armed, KeyCode::Enter), CutAnswered::Cut);

        assert_eq!(
            answered(&armed, KeyCode::Right),
            CutAnswered::Open(Answer::No)
        );
        assert_eq!(
            answered(&armed.lit(Answer::No), KeyCode::Enter),
            CutAnswered::Cancel
        );
    }

    #[test]
    fn y_and_n_answer_outright_whichever_is_lit() {
        for lit in [Answer::Yes, Answer::No] {
            let cut = open().lit(lit);
            assert_eq!(
                answered(&cut, KeyCode::Char('y')),
                CutAnswered::Cut,
                "y should cut with {lit:?} lit"
            );
            assert_eq!(
                answered(&cut, KeyCode::Char('n')),
                CutAnswered::Cancel,
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
            let cut = open().lit(lit);
            for code in INERT {
                assert_eq!(
                    answered(&cut, code),
                    CutAnswered::Open(lit),
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
            assert_eq!(cut_answer_for(ctrl_c, lit), CutAnswered::Open(lit));
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
                    cut_answer_for(event, Answer::No),
                    CutAnswered::Open(Answer::No),
                    "{kind:?} of {code:?} should answer nothing"
                );
            }
        }
    }

    #[test]
    fn nothing_but_yes_and_enter_on_yes_ever_cuts() {
        for lit in [Answer::Yes, Answer::No] {
            let cut = open().lit(lit);
            for code in INERT.into_iter().chain([
                KeyCode::Esc,
                KeyCode::Char('n'),
                KeyCode::Left,
                KeyCode::Right,
            ]) {
                assert_ne!(
                    answered(&cut, code),
                    CutAnswered::Cut,
                    "{code:?} should not cut with {lit:?} lit"
                );
            }
        }
        assert_eq!(
            answered(&open(), KeyCode::Enter),
            CutAnswered::Cancel,
            "Enter on No answers No, so the default answer starts nothing"
        );
    }
}

// The window one slice's drafts wait behind, which is the only question warlock
// asks that has three answers. What its keys mean is asserted here rather than
// where the run is driven: a test about a run should be able to say `create`
// without also saying which key spells it.
mod review {
    use super::{
        Choice, INERT, KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers, Review,
        Reviewed, press, review_answer_for,
    };

    const SLICE: &str = "slice 1 `The review window`";

    fn titles() -> Vec<String> {
        vec![
            "Gate the drafts behind a window".to_owned(),
            "Report what the create became".to_owned(),
        ]
    }

    // A slice with its redraft still to spend, which is how a window first goes
    // up.
    fn open() -> Review {
        Review::open(SLICE, titles(), true)
    }

    // The same slice once it has been redrafted: two answers, and the third
    // nowhere a key can reach.
    fn spent() -> Review {
        Review::open(SLICE, titles(), false)
    }

    // The window answering one key as the session answers it: what is lit comes
    // off the value that is up, and the whole window goes in because a key's
    // meaning depends on whether the redraft is still there.
    fn answered(review: &Review, code: KeyCode) -> Reviewed {
        review_answer_for(press(code), review)
    }

    // Every key the tree answers to that this window does not, which is INERT
    // without `f`: asking for a redraft is the one thing this window binds a
    // letter to that no other does.
    fn inert() -> Vec<KeyCode> {
        INERT
            .into_iter()
            .filter(|code| !matches!(code, KeyCode::Char('f' | 'F')))
            .collect()
    }

    #[test]
    fn a_fresh_window_is_up_on_skip_with_the_drafts_it_is_about() {
        // Skip is lit for No's reason elsewhere: the round that puts this up
        // and an Enter straight after it file nothing at all.
        let review = open();

        assert_eq!(review.choice(), Choice::Skip);
        assert_eq!(review.slice(), SLICE);
        assert_eq!(review.titles(), titles().as_slice());
        assert!(review.feedback(), "a fresh slice has its redraft to spend");
        assert_eq!(answered(&review, KeyCode::Enter), Reviewed::Skip);
    }

    #[test]
    fn the_arrows_walk_the_three_answers_and_stop_at_both_ends() {
        // A highlight that wrapped would put the answer that files issues under
        // the finger of somebody pressing Right twice.
        let create = open().with_choice(Choice::Create);
        let feedback = open().with_choice(Choice::Feedback);

        assert_eq!(
            answered(&open(), KeyCode::Left),
            Reviewed::Open(Choice::Create)
        );
        assert_eq!(
            answered(&open(), KeyCode::Right),
            Reviewed::Open(Choice::Feedback)
        );
        assert_eq!(
            answered(&create, KeyCode::Left),
            Reviewed::Open(Choice::Create),
            "Left walked off the left end"
        );
        assert_eq!(
            answered(&feedback, KeyCode::Right),
            Reviewed::Open(Choice::Feedback),
            "Right walked off the right end"
        );
        assert_eq!(
            answered(&feedback, KeyCode::Left),
            Reviewed::Open(Choice::Skip)
        );
        assert_eq!(
            answered(&create, KeyCode::Right),
            Reviewed::Open(Choice::Skip)
        );
    }

    #[test]
    fn enter_answers_with_whatever_is_lit() {
        for (choice, answer) in [
            (Choice::Create, Reviewed::Create),
            (Choice::Skip, Reviewed::Skip),
            (Choice::Feedback, Reviewed::Feedback),
        ] {
            assert_eq!(
                answered(&open().with_choice(choice), KeyCode::Enter),
                answer,
                "Enter on {choice:?} answered something else"
            );
        }
    }

    #[test]
    fn the_answers_initials_answer_outright_whichever_is_lit() {
        // Letters rather than `y`/`n`, and matched in either case for
        // `answer_for`'s reason: a reader with caps lock on is still answering.
        for lit in [Choice::Create, Choice::Skip, Choice::Feedback] {
            let review = open().with_choice(lit);
            for (code, answer) in [
                (KeyCode::Char('c'), Reviewed::Create),
                (KeyCode::Char('C'), Reviewed::Create),
                (KeyCode::Char('s'), Reviewed::Skip),
                (KeyCode::Char('S'), Reviewed::Skip),
                (KeyCode::Char('f'), Reviewed::Feedback),
                (KeyCode::Char('F'), Reviewed::Feedback),
            ] {
                assert_eq!(
                    answered(&review, code),
                    answer,
                    "{code:?} with {lit:?} lit answered something else"
                );
            }
        }
    }

    #[test]
    fn esc_skips_rather_than_dismissing_the_window() {
        // There is no cancel: a window that could be dismissed would leave the
        // run holding drafts nobody had decided about, so the least committal
        // key is the answer that files nothing and asks what to do next.
        for lit in [Choice::Create, Choice::Skip, Choice::Feedback] {
            assert_eq!(
                answered(&open().with_choice(lit), KeyCode::Esc),
                Reviewed::Skip,
                "Esc with {lit:?} lit did not skip"
            );
        }
    }

    #[test]
    fn a_spent_redraft_is_neither_pressed_nor_walked_onto() {
        // One redraft each. The third answer is not drawn on a window that has
        // spent it, and a key that is not drawn is not one that can be pressed.
        let spent = spent();

        assert_eq!(
            answered(&spent, KeyCode::Char('f')),
            Reviewed::Open(Choice::Skip),
            "`f` asked for a second redraft"
        );
        assert_eq!(
            answered(&spent, KeyCode::Right),
            Reviewed::Open(Choice::Skip),
            "Right lit an answer that is not on the window"
        );
        assert_eq!(
            answered(&spent.with_choice(Choice::Create), KeyCode::Right),
            Reviewed::Open(Choice::Skip)
        );
    }

    #[test]
    fn every_other_key_leaves_the_window_exactly_as_it_was() {
        for lit in [Choice::Create, Choice::Skip, Choice::Feedback] {
            let review = open().with_choice(lit);
            for code in inert() {
                assert_eq!(
                    answered(&review, code),
                    Reviewed::Open(lit),
                    "{code:?} should leave {lit:?} lit and answer nothing"
                );
            }
        }
    }

    #[test]
    fn releases_and_repeats_answer_nothing() {
        for code in [
            KeyCode::Enter,
            KeyCode::Esc,
            KeyCode::Char('c'),
            KeyCode::Char('s'),
            KeyCode::Char('f'),
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
                    review_answer_for(event, &open()),
                    Reviewed::Open(Choice::Skip),
                    "{kind:?} of {code:?} should answer nothing"
                );
            }
        }
    }

    #[test]
    fn nothing_but_create_and_enter_on_create_ever_files() {
        // The half worth saying outright: of every key this window sees, only
        // two spellings of Create put anything on the board.
        for lit in [Choice::Create, Choice::Skip, Choice::Feedback] {
            let review = open().with_choice(lit);
            for code in inert().into_iter().chain([
                KeyCode::Esc,
                KeyCode::Char('s'),
                KeyCode::Char('f'),
                KeyCode::Left,
                KeyCode::Right,
            ]) {
                assert_ne!(
                    answered(&review, code),
                    Reviewed::Create,
                    "{code:?} should not file with {lit:?} lit"
                );
            }
        }
        assert_eq!(
            answered(&open(), KeyCode::Enter),
            Reviewed::Skip,
            "Enter on Skip skips, so the default answer files nothing"
        );
    }
}

// The question a skipped slice leaves behind, which is the quit dialog's rules
// asked about something else. Every test here is one of this file's own asked
// again, because "answered by the same keys" is a claim about behaviour and not
// about which function the body happens to call.
mod carry {
    use super::{
        Answer, Carry, CarryAnswered, INERT, KeyCode, KeyEvent, KeyEventKind, KeyEventState,
        KeyModifiers, carry_answer_for, press,
    };

    const LEFT: &str = "2 slices";

    fn open() -> Carry {
        Carry::open(LEFT)
    }

    fn answered(carry: &Carry, code: KeyCode) -> CarryAnswered {
        carry_answer_for(press(code), carry.answer())
    }

    #[test]
    fn a_fresh_question_is_up_on_no_and_carries_what_is_left() {
        // No is the answer that stops: a run carries on by somebody saying so.
        let carry = open();

        assert_eq!(carry.answer(), Answer::No);
        assert_eq!(carry.left(), LEFT);
        assert_eq!(
            answered(&carry, KeyCode::Enter),
            CarryAnswered::Stop,
            "an immediate Enter carried on"
        );
    }

    #[test]
    fn esc_and_n_stop_from_either_side() {
        for lit in [Answer::Yes, Answer::No] {
            let carry = open().with_answer(lit);
            for code in [KeyCode::Esc, KeyCode::Char('n'), KeyCode::Char('N')] {
                assert_eq!(
                    answered(&carry, code),
                    CarryAnswered::Stop,
                    "{code:?} with {lit:?} lit did not stop the run"
                );
            }
        }
    }

    #[test]
    fn left_then_enter_carries_on_and_right_goes_back_to_no() {
        let carry = open();

        assert_eq!(
            answered(&carry, KeyCode::Left),
            CarryAnswered::Open(Answer::Yes)
        );
        let lit = open().with_answer(Answer::Yes);
        assert_eq!(lit.left(), LEFT, "moving the highlight lost the count");
        assert_eq!(answered(&lit, KeyCode::Enter), CarryAnswered::Carry);
        assert_eq!(
            answered(&lit, KeyCode::Right),
            CarryAnswered::Open(Answer::No)
        );
        assert_eq!(
            answered(&lit, KeyCode::Char('y')),
            CarryAnswered::Carry,
            "`y` answers outright wherever the highlight is"
        );
    }

    #[test]
    fn every_other_key_leaves_the_question_exactly_as_it_was() {
        for lit in [Answer::Yes, Answer::No] {
            let carry = open().with_answer(lit);
            for code in INERT {
                assert_eq!(
                    answered(&carry, code),
                    CarryAnswered::Open(lit),
                    "{code:?} should leave {lit:?} lit and answer nothing"
                );
            }
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
                    carry_answer_for(event, Answer::No),
                    CarryAnswered::Open(Answer::No),
                    "{kind:?} of {code:?} should answer nothing"
                );
            }
        }
    }

    #[test]
    fn nothing_but_yes_and_enter_on_yes_ever_carries_on() {
        for lit in [Answer::Yes, Answer::No] {
            let carry = open().with_answer(lit);
            for code in INERT.into_iter().chain([
                KeyCode::Esc,
                KeyCode::Char('n'),
                KeyCode::Left,
                KeyCode::Right,
            ]) {
                assert_ne!(
                    answered(&carry, code),
                    CarryAnswered::Carry,
                    "{code:?} should not carry on with {lit:?} lit"
                );
            }
        }
        assert_eq!(
            answered(&open(), KeyCode::Enter),
            CarryAnswered::Stop,
            "Enter on No stops, so the default answer leaves the project alone"
        );
    }
}
