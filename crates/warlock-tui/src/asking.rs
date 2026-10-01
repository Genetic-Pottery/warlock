//! The one answer warlock reads from stdin.
//!
//! The headless verbs print what a reader needs on the ordinary screen and then
//! read an answer back, and this is where that read lives: the prompt written,
//! stdout flushed while the cursor is still on it, end of file told apart from
//! an answer somebody just pressed Enter on, and an io failure raised as
//! [`Error::Prompt`]. At a terminal the answer is read raw, so a multi-line
//! paste arrives whole (see [`Stdin`]); from a pipe it is one cooked line.
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
use std::process;
use std::time::Duration;

use ratatui::crossterm::event::{
    self, DisableBracketedPaste, EnableBracketedPaste, Event, KeyCode, KeyEvent, KeyEventKind,
    KeyModifiers,
};
use ratatui::crossterm::execute;
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
    /// A step of its own and never part of [`Asks::ask`]: a verb that asks
    /// several times with no model between, as `warlock brief` does line by
    /// line, must keep what was typed between its questions.
    fn discard_typed_ahead(&mut self) {}
}

/// The real one: the prompt on stdout, the line off stdin.
pub(crate) struct Stdin;

impl Asks for Stdin {
    // At a terminal the answer is read raw, with bracketed paste on, so a
    // multi-line paste is one answer: a cooked read hands back the paste's first
    // line as the whole answer, and the rest is thrown away as type-ahead before
    // the next question. That cut-off is the one thing wrong with Red and
    // Forman's prompts, and this is where it is not copied. A pipe keeps the
    // cooked line, because its lines were each written as an answer.
    fn ask(&mut self, prompt: &str) -> Result<Option<String>, Error> {
        show(prompt);
        if at_terminal() && enable_raw_mode().is_ok() {
            let cooked = Cooked;
            let answer = edited();
            drop(cooked);
            return match answer {
                // What Ctrl-C did at a cooked prompt, with the terminal put
                // back first: the process ends, as SIGINT ends it.
                Err(error) if error.kind() == io::ErrorKind::Interrupted => process::exit(130),
                answer => answer.map_err(|source| Error::Prompt { source }),
            };
        }
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

// The terminal half of [`Stdin::ask`]: events in, echo out, until [`step`]
// says the answer is done. Bracketed paste is turned on for the read and off
// again whatever happens, like raw mode, so a shell is never left with it.
fn edited() -> io::Result<Option<String>> {
    let mut out = io::stdout();
    let _pasting = Pasting::on(&mut out);
    let mut answer = String::new();
    loop {
        let event = event::read()?;
        // An Enter with more input already waiting behind it is a newline
        // inside a paste, on a terminal that does not bracket pastes: a person
        // pressing Enter has nothing queued behind the key.
        let queued = matches!(event::poll(PASTE_GAP), Ok(true));
        match step(&mut answer, &event, queued) {
            Step::Echo(text) => {
                drop(write!(out, "{text}"));
                drop(out.flush());
            }
            Step::Done => {
                drop(write!(out, "\r\n"));
                drop(out.flush());
                return Ok(Some(answer));
            }
            Step::End => {
                drop(write!(out, "\r\n"));
                drop(out.flush());
                return Ok(None);
            }
            // Raw mode turns Ctrl-C into a key; `Stdin::ask` ends the process
            // on it once the terminal is restored.
            Step::Interrupt => {
                drop(write!(out, "^C\r\n"));
                drop(out.flush());
                return Err(io::Error::from(io::ErrorKind::Interrupted));
            }
        }
    }
}

// How long an Enter waits to see whether more input follows it. Pasted bytes
// arrive together, so a few milliseconds tells a paste from a person.
const PASTE_GAP: Duration = Duration::from_millis(5);

struct Pasting;

impl Pasting {
    fn on(out: &mut io::Stdout) -> Self {
        drop(execute!(out, EnableBracketedPaste));
        Self
    }
}

impl Drop for Pasting {
    fn drop(&mut self) {
        drop(execute!(io::stdout(), DisableBracketedPaste));
    }
}

/// What one terminal event does to the answer being typed.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Step {
    /// Keep reading, after putting this on the screen.
    Echo(String),
    /// The answer is finished.
    Done,
    /// Ctrl-D on an empty answer: end of file, as at a cooked prompt.
    End,
    /// Ctrl-C.
    Interrupt,
}

/// The line editor, without a terminal: `queued` is whether more input was
/// already waiting when this event was read. Kept to what a cooked prompt
/// offers — characters, Backspace, Enter, Ctrl-D and Ctrl-C — plus a paste
/// that keeps its newlines.
pub(crate) fn step(answer: &mut String, event: &Event, queued: bool) -> Step {
    match event {
        Event::Paste(text) => {
            let text = text.replace("\r\n", "\n").replace('\r', "\n");
            answer.push_str(&text);
            Step::Echo(text.replace('\n', "\r\n"))
        }
        Event::Key(KeyEvent {
            code,
            modifiers,
            kind: KeyEventKind::Press,
            ..
        }) => {
            let control = modifiers.contains(KeyModifiers::CONTROL);
            match code {
                KeyCode::Char('c') if control => Step::Interrupt,
                KeyCode::Char('d') if control && answer.is_empty() => Step::End,
                KeyCode::Enter if queued => {
                    answer.push('\n');
                    Step::Echo("\r\n".to_owned())
                }
                KeyCode::Enter => Step::Done,
                // Never back over a newline: the cursor cannot follow it up a
                // line without redrawing everything above, so what was pasted
                // above stays as it was pasted.
                KeyCode::Backspace => match answer.chars().last() {
                    Some(last) if last != '\n' => {
                        answer.pop();
                        Step::Echo("\u{8} \u{8}".to_owned())
                    }
                    _ => Step::Echo(String::new()),
                },
                KeyCode::Char(character) if !control => {
                    answer.push(*character);
                    Step::Echo(character.to_string())
                }
                _ => Step::Echo(String::new()),
            }
        }
        _ => Step::Echo(String::new()),
    }
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
