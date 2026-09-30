use std::io::{self, BufReader, Read};

use super::{Asks, line_in};
use crate::error::Error;
use crate::stubs::Typing;

// The three answers a read can give, asked of a reader a test can hold rather
// than of stdin: nothing here opens a terminal or waits on a pipe, which is the
// whole reason the line read is generic over its reader.
fn read(bytes: &'static [u8]) -> Result<Option<String>, Error> {
    line_in(&mut &bytes[..])
}

// A reader that fails on the first byte, for the one answer neither a line nor an
// empty pipe can produce. `BufRead` is reached through `BufReader`, so what is
// written here is the one thing being stood in for: the read itself going wrong.
struct Failing;

impl Read for Failing {
    fn read(&mut self, _into: &mut [u8]) -> io::Result<usize> {
        Err(io::Error::other("the pipe broke"))
    }
}

#[test]
fn a_typed_line_comes_back_exactly_as_it_was_typed() {
    assert_eq!(
        read(b"data-plane web\n").expect("a line"),
        Some("data-plane web\n".to_owned())
    );
    // The newline is kept rather than trimmed here: what a trailing newline means
    // is the caller's — sigils are words on a line, a key is the line trimmed.
    assert_eq!(
        read(b"lin_api_x").expect("a line"),
        Some("lin_api_x".to_owned())
    );
}

#[test]
fn an_empty_pipe_is_end_of_file_and_a_bare_newline_is_an_answer() {
    assert_eq!(read(b"").expect("an EOF is not a failure"), None);
    // The distinction the whole module exists to keep: somebody pressed Enter,
    // which is a blank line that clears a set, and not nobody answering at all.
    assert_eq!(read(b"\n").expect("a line"), Some("\n".to_owned()));
}

#[test]
fn a_read_that_goes_wrong_is_a_prompt_failure_and_never_an_end_of_file() {
    let mut failing = BufReader::new(Failing);

    let error = line_in(&mut failing).expect_err("a broken pipe is a failure");

    assert!(matches!(error, Error::Prompt { .. }), "{error:?}");
}

#[test]
fn the_stand_in_answers_one_line_per_question_and_keeps_every_prompt() {
    let mut typing = Typing::lines(["billing\n", "web\n"]);

    assert_eq!(
        typing.ask("first> ").expect("a line"),
        Some("billing\n".to_owned())
    );
    assert_eq!(
        typing.ask("second> ").expect("a line"),
        Some("web\n".to_owned())
    );
    // The script has run out, which is a pipe read to the end.
    assert_eq!(
        typing.ask("third> ").expect("an EOF is not a failure"),
        None
    );
    assert_eq!(typing.asked(), ["first> ", "second> ", "third> "]);
}

#[test]
fn a_stand_in_holding_nothing_is_end_of_file_from_the_first_question() {
    let mut typing = Typing::nothing();

    assert_eq!(typing.ask("> ").expect("an EOF is not a failure"), None);
    assert_eq!(typing.ask("> ").expect("an EOF is not a failure"), None);
    assert_eq!(typing.asked().len(), 2, "both questions were asked");
}
