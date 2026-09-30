//! The one line warlock reads from stdin.
//!
//! `warlock config` and `warlock key add` both print what a reader needs on the
//! ordinary screen and then read one cooked line back, and this is where that
//! read lives: the prompt written, stdout flushed while the cursor is still on
//! it, `Ok(0)` told apart from a line somebody just pressed Enter on, and an io
//! failure raised as [`Error::Prompt`]. Those four decisions were a byte-for-byte
//! copy in each of the two subcommands, which is one copy too many — the way one
//! of them goes wrong is by being EOF-blind on its own.
//!
//! [`Asks`] is the seam. A headless verb takes `&mut impl Asks` rather than
//! reaching for stdin itself, so a whole run is driven off a written-down script
//! with nothing attached to stdin and no terminal anywhere. It is `&mut self` and
//! not a closure because a run may ask more than once and each answer is the next
//! line — a one-shot closure would have to be rebuilt per question by every
//! caller.
//!
//! Nothing here echoes the line it read, and that is the decision rather than an
//! omission. At a terminal the line discipline has already echoed it; when stdin
//! is not a terminal nothing echoes it and warlock does not make up for it,
//! because the line that arrives that way is `warlock key add acme < key.txt`'s
//! and a key printed back would be on the screen, in the scrollback and in the CI
//! log. [`at_terminal`] is the other half of the same question and the only reason
//! this module asks it: a terminal on stdin is a person typing, so `key add` takes
//! the terminal and masks the read itself; see [`mod@crate::key`].

use std::io::{self, BufRead, IsTerminal, Write};
use std::time::Duration;

use ratatui::crossterm::event;
use ratatui::crossterm::terminal::{disable_raw_mode, enable_raw_mode};

use crate::error::Error;

/// One question, asked and answered.
///
/// `None` is EOF — Ctrl-D at a terminal, an empty or exhausted pipe everywhere
/// else — and never an empty line: a caller that cannot tell those two apart
/// writes for somebody who never answered.
pub(crate) trait Asks {
    fn ask(&mut self, prompt: &str) -> Result<Option<String>, Error>;

    /// Throw away whatever was typed while a model was working, called by a
    /// verb after a session has answered and before it asks again.
    ///
    /// A step of its own and never part of [`Asks::ask`]: `warlock brief` asks
    /// once per line, so a paragraph pasted at its prompt arrives as lines typed
    /// ahead of every ask after the first, and discarding at each ask would keep
    /// the first line of the paste and drop the rest.
    fn discard_typed_ahead(&mut self) {}
}

/// The real one: the prompt on stdout, the line off stdin.
pub(crate) struct Stdin;

impl Asks for Stdin {
    fn ask(&mut self, prompt: &str) -> Result<Option<String>, Error> {
        show(prompt);
        line_in(&mut io::stdin().lock())
    }

    // Whatever a person typed while a session was working sits in the terminal's
    // line buffer, and a cooked read would take it as the answer to a question it
    // was typed before: an `accept` pressed during drafting would file drafts
    // nobody read. Raw mode makes that buffer readable without waiting, so it is
    // read and thrown away. Forman drops type-ahead before each of its gates for
    // the same reason. Only at a terminal, because a pipe's lines were all
    // written ahead on purpose. Best effort: a terminal that will not go raw
    // keeps what was typed, which is how every read behaved before this.
    fn discard_typed_ahead(&mut self) {
        if !at_terminal() || enable_raw_mode().is_err() {
            return;
        }
        let _cooked = Cooked;
        while matches!(event::poll(Duration::ZERO), Ok(true)) {
            if event::read().is_err() {
                break;
            }
        }
    }
}

// The restore, as a `Drop` rather than a line after the raw work: a `Drop` also
// runs while a panic unwinds and on every early return. A terminal left in raw
// mode outlives the process — the person gets a shell with no echo and no line
// editing, and has to know to type `reset`.
pub(crate) struct Cooked;

impl Drop for Cooked {
    fn drop(&mut self) {
        drop(disable_raw_mode());
    }
}

/// Whether stdin is a terminal.
///
/// Stdin and not stdout, because stdin is what is about to be read: a terminal
/// there means a person typing, and anything else means a script that wants the
/// cooked line it has always had.
pub(crate) fn at_terminal() -> bool {
    io::stdin().is_terminal()
}

/// The prompt, on the screen and out of the buffer, with the cursor left sitting
/// on the end of it.
///
/// Public to the crate for the one caller that reads a line without reading
/// stdin: `key add`'s masked terminal read, which has to put the same bytes on
/// the screen before it waits. The flush is the whole point and cannot be left to
/// the caller — a prompt carries no newline of its own, so line-buffered stdout
/// holds it back and a person is left looking at a blank line below whatever was
/// printed before it.
///
/// Best effort, and the only thing that could be done about it: a stdout that
/// will not flush has nothing useful to say about itself, and the read that
/// follows reports anything that really goes wrong.
pub(crate) fn show(prompt: &str) {
    let mut out = io::stdout();
    drop(write!(out, "{prompt}"));
    drop(out.flush());
}

// `Ok(0)` is EOF and nothing else. It is told apart from an empty line here
// rather than further up, because everything above treats a line as text and only
// a read can tell "they pressed Enter" from "there is no line and never will be".
//
// Generic over the reader so those three answers are tested without stdin: the
// one caller hands it the real thing.
fn line_in<R: BufRead>(from: &mut R) -> Result<Option<String>, Error> {
    let mut line = String::new();
    match from.read_line(&mut line) {
        Ok(0) => Ok(None),
        Ok(_) => Ok(Some(line)),
        Err(source) => Err(Error::Prompt { source }),
    }
}

#[cfg(test)]
#[path = "tests/asking.rs"]
mod tests;
