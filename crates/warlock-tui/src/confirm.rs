//! The gate on the way out: is the quit confirmation up, and which answer is lit.
//!
//! Esc and `q` used to leave outright, and the accident that costs a session is
//! the reflex second Esc — the first one cancels a pact, the run is still
//! tidying up, and the next press lands in a shell nobody meant to be in. This
//! module is the whole of what stands between a twitch and the end of the
//! session: a mode with two answers, and one pure function saying what a key
//! does to it.
//!
//! ## The shape, because the drawing and the loop both depend on it
//!
//! The mode is a value of its own — [`QuitConfirm`] — and *not* a field on
//! [`App`](crate::App). Two reasons, and both are about what stays true
//! elsewhere. Answering No has to leave the app exactly as it was, and the
//! cheapest way to be sure of that is for the app never to have heard of the
//! dialog: an `App` compared before opening and after closing is equal because
//! nothing about it was touched, rather than because every field was carefully
//! put back. And a confirmation is not state about the tree — it is state about
//! this keystroke and the next one — so keeping it out of `App` keeps `App` the
//! answer to "what is warlock showing" rather than "what is warlock in the
//! middle of asking".
//!
//! Yes is drawn on the left and No on the right, in that reading order, which is
//! what makes [`KeyCode::Left`] and [`KeyCode::Right`] positional here rather
//! than a toggle: Left lights Yes, Right lights No, and pressing the same arrow
//! twice does nothing the second time. A toggle would read as an arrow that
//! moves the highlight *away* from the side it points at as soon as it is
//! already there. Whoever draws the two answers draws them in that order.
//!
//! ## What this deliberately does not answer
//!
//! Ctrl-C. It is a key event and not a signal — raw mode is exactly the mode in
//! which the terminal stops turning it into `SIGINT` — so it has to be answered
//! by the event loop *before* the loop consults this mode, both with the
//! confirmation closed and with it open. If it came through here it would be an
//! ordinary character with a modifier riding along, i.e. one of the keys that
//! change nothing, and the one keystroke every reader trusts to get them out
//! would be the one keystroke the dialog swallowed.
//!
//! Nothing here reads a terminal, draws anything, or takes an
//! [`App`](crate::App): a key event and the lit answer go in, and one of three
//! consequences comes out, so every rule below is one assertion with nothing
//! attached to stdout.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind};

/// One of the two answers the confirmation offers.
///
/// Two variants rather than a boolean, for the reason [`Focus`](crate::Focus) is
/// two variants: "which answer is lit" is the thing the type says, the renderer
/// matches on it exhaustively, and a `bool` would need a comment at every use
/// saying which way round it reads.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum Answer {
    /// Leave warlock, by the path a quit already takes.
    Yes,
    /// Stay, and change nothing: what the confirmation opens on, so the
    /// keystroke that opens it and an Enter straight after it come to nothing at
    /// all. The dangerous answer is never the one under the reader's finger.
    #[default]
    No,
}

/// Whether the confirmation is up, and which answer is lit while it is.
///
/// The lit answer lives inside [`QuitConfirm::Open`] rather than beside a
/// `bool`, so "closed, with Yes highlighted" is not a state that can be written
/// down: there is one place the highlight can be, and it exists exactly as long
/// as the question does.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum QuitConfirm {
    /// No question is being asked. Keys mean what they have always meant, and
    /// there is nothing over the frame.
    #[default]
    Closed,
    /// The question is on screen, with `.0` lit.
    Open(Answer),
}

impl QuitConfirm {
    /// The confirmation as it opens: up, with No lit.
    ///
    /// A named constructor rather than `Open(Answer::No)` written at the call
    /// site, so which answer a fresh dialog starts on is decided here — in the
    /// module that will still be right about it after the drawing changes —
    /// rather than wherever Esc happens to be handled.
    #[must_use]
    pub const fn open() -> Self {
        Self::Open(Answer::No)
    }

    /// Whether a question is being asked right now.
    ///
    /// What the event loop branches on before it consults [`answer_for`], and
    /// what the renderer branches on before it draws anything over the frame.
    #[must_use]
    pub const fn is_open(self) -> bool {
        matches!(self, Self::Open(_))
    }

    /// The lit answer, or `None` when nothing is being asked.
    ///
    /// The one way into [`answer_for`]: an `Option` here is what keeps the key
    /// handler from having to invent an answer for a dialog that is not up.
    #[must_use]
    pub const fn highlighted(self) -> Option<Answer> {
        match self {
            Self::Closed => None,
            Self::Open(answer) => Some(answer),
        }
    }
}

/// What a keystroke comes to while the confirmation is open.
///
/// Named apart from the keys that produce it for the reason the binary's
/// `Action` is: it keeps [`answer_for`] a pure function of a key event, and
/// leaves the loop above reading as a list of consequences. Three variants is
/// the whole of what can happen to a two-answer question — it moves on, it is
/// answered No, or it is answered Yes — and there is deliberately no variant for
/// "the key meant nothing", because a key that means nothing here leaves the
/// question exactly where it was, which is [`Answered::Open`] with the same
/// answer in it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Answered {
    /// Not answered: the question stays up with `.0` lit, which is either where
    /// the highlight already was or the one place an arrow key moved it to.
    Open(Answer),
    /// Answered No: take the question down and change nothing else. The app was
    /// never touched, so there is nothing to put back.
    Close,
    /// Answered Yes: leave warlock, by the path a quit already takes, so the
    /// terminal is restored through the guard that already exists.
    Leave,
}

/// What `key` does to a confirmation with `highlighted` lit.
///
/// The counterpart of the binary's `action_for`, and the same shape: a key and a
/// situation in, one intention out, no terminal and no [`App`](crate::App).
///
/// Four keys answer the question and two move it. Enter takes whichever answer
/// is lit, which is what Enter means in every dialog anyone has used. `y` and
/// `n` answer outright, whichever is lit, because a reader who knows the answer
/// should not have to look at the screen to give it — and they are matched by
/// character in either case, so caps lock does not take the way out away. Esc
/// answers No: the key that means "not this" cannot be the key that leaves, or
/// the reflex second press this whole gate exists for would go straight through
/// it. Left lights Yes and Right lights No, positionally, per the order recorded
/// in the module docs.
///
/// Every other key leaves the question byte-for-byte as it was, including the
/// tree's own bindings: while this is up, `j`, `k`, `g`, `G`, space, `o`, `f`,
/// `p`, `m` and Tab reach nothing underneath, because the loop consults this
/// instead of the app rather than as well as it.
///
/// Only presses count, exactly as `action_for` has it. Crossterm reports
/// releases and auto-repeats on some platforms and not on others, and a release
/// acted on here would answer the question with the release of the very key that
/// opened it — Esc pressed once would open the dialog and immediately close it
/// again, which is a gate that is not there.
#[must_use]
pub fn answer_for(key: KeyEvent, highlighted: Answer) -> Answered {
    if key.kind != KeyEventKind::Press {
        return Answered::Open(highlighted);
    }

    match key.code {
        KeyCode::Left => Answered::Open(Answer::Yes),
        KeyCode::Right => Answered::Open(Answer::No),
        KeyCode::Enter => match highlighted {
            Answer::Yes => Answered::Leave,
            Answer::No => Answered::Close,
        },
        // By character rather than by `SHIFT`, like the tree's `g`/`G` pair:
        // terminals disagree about whether the modifier rides along with an
        // upper-case letter, and a reader with caps lock on is still answering.
        KeyCode::Char('y' | 'Y') => Answered::Leave,
        KeyCode::Char('n' | 'N') | KeyCode::Esc => Answered::Close,
        _ => Answered::Open(highlighted),
    }
}

#[cfg(test)]
mod tests {
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};

    use super::{Answer, Answered, QuitConfirm, answer_for};

    /// A plain press of `code`, as crossterm reports one with no modifiers.
    fn press(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    /// Every key the tree answers to, plus a character bound to nothing
    /// anywhere: the list the confirmation has to swallow whole, so no
    /// keystroke reaches the app underneath while the question is up.
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
}
