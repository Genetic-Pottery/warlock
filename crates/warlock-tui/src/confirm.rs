//! The accident that costs a session is the reflex second Esc — the first
//! cancels a pact, the run is still tidying up, and the next press lands in a
//! shell nobody meant to be in. Which is why Esc answers No here: the key that
//! means "not this" cannot also be the key that leaves. For the same reason the
//! mode is a value of its own and *not* a field on `App`, so an app compared
//! before opening and after closing is equal because nothing about it was
//! touched rather than because every field was carefully put back.
//!
//! Yes is drawn on the left and No on the right, which makes Left and Right
//! positional rather than a toggle — a toggle would move the highlight *away*
//! from the side the arrow points at once it is already there. Ctrl-C is not
//! answered here at all: raw mode is exactly the mode in which the terminal
//! stops turning it into `SIGINT`, so the loop takes it before consulting this
//! mode, and coming through here the dialog would swallow it.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum Answer {
    Yes,
    // The default, so the keystroke that opens the dialog and an Enter straight
    // after it come to nothing at all: the dangerous answer is never the one
    // already under the reader's finger.
    #[default]
    No,
}

// The lit answer lives inside `Open` rather than beside a `bool`, so "closed,
// with Yes highlighted" is not a state that can be written down: the highlight
// exists exactly as long as the question does.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum QuitConfirm {
    #[default]
    Closed,
    Open(Answer),
}

impl QuitConfirm {
    // A named constructor rather than `Open(Answer::No)` at the call site, so
    // which answer a fresh dialog starts on is decided here rather than wherever
    // Esc happens to be handled.
    #[must_use]
    pub const fn open() -> Self {
        Self::Open(Answer::No)
    }

    #[must_use]
    pub const fn is_open(self) -> bool {
        matches!(self, Self::Open(_))
    }

    // The one way into `answer_for`: the `Option` is what keeps the key handler
    // from having to invent an answer for a dialog that is not up.
    #[must_use]
    pub const fn highlighted(self) -> Option<Answer> {
        match self {
            Self::Closed => None,
            Self::Open(answer) => Some(answer),
        }
    }
}

// Three variants is the whole of what can happen to a two-answer question, and
// there is deliberately no variant for "the key meant nothing": a key that means
// nothing here leaves the question where it was, which is `Open` with the same
// answer in it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Answered {
    Open(Answer),
    Close,
    Leave,
}

// Every key not matched below leaves the question byte-for-byte as it was,
// including the tree's own bindings: while this is up the loop consults this
// instead of the app rather than as well as it, so `j`, `k`, `p`, Tab and the
// rest reach nothing underneath.
//
// Only presses count. Crossterm reports releases and auto-repeats on some
// platforms and not others, and a release acted on here would answer the
// question with the release of the very key that opened it — Esc pressed once
// would open the dialog and immediately close it again, which is a gate that is
// not there.
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
#[path = "tests/confirm.rs"]
mod tests;
