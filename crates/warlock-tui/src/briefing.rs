//! `warlock brief`: the panel's brief register held at a shell. One agent, one
//! session, and a conversation that goes on for as long as somebody is typing
//! into it.
//!
//! The panel's `/brief` is a *mode* of the conversation a reader is already in —
//! see [`mod@crate::chatting`] — and this is the whole conversation instead:
//! there is no chat to come from and none to go back to, which is what keeps the
//! prompt's vocabulary to prose and nothing else. No `/chat` here, because this
//! command *is* brief mode; and the two register words are not this module's to
//! choose, so [`raised`] names them once and takes them from the library's
//! `claude.rs`, which is where the panel takes them from too.
//!
//! Both of `/brief`'s files are read before a word is sent, which is
//! [`mod@crate::chatting`]'s rule at the other door: a template or a
//! `briefs.toml` that will not read is a refusal with nothing spent, rather than
//! a conversation opened and then found to have no shape to converge on.
//!
//! Nothing here takes the terminal, installs a panic hook or enters raw mode.
//! The prompt is a cooked line off stdin through the [`Asks`] seam, which is
//! what lets a whole conversation be driven from a written-down script with no
//! terminal anywhere and no `claude` on the machine. Ctrl-C therefore needs no
//! code at all, and EOF is the way out: see [`Asks::ask`].

use std::io::{self, Write};
use std::path::Path;

use warlock_engine::load_briefs;
use warlock_tui::{
    BRIEF_EFFORT, BRIEF_MODEL, ChatAgent, Converses, brief_instruction, brief_template, ending_for,
};

use crate::asking::{self, Asks};
use crate::error::Error;
use crate::standing::{FOR_BRIEF, Standing};

// The cursor a turn is typed on, and `planned.rs`'s prompt down to the bytes: a
// bare mark, because what is being read is prose and not a command word or an
// answer to pick from. What a reader needs before they can type is the line
// above it.
const PROMPT: &str = "> ";

// Said once, before the first turn is sent, because the one thing a reader
// cannot work out from a bare `> ` is what the conversation is for. Worded as
// the panel's own note is — see `chatting::BRIEF_NOTE` — minus the way out: a
// `/chat` that left brief mode would have nothing to leave it into.
const OPENING: &str = "brief mode — this conversation is converging on a document";

// EOF, which is Ctrl-D at a terminal and an exhausted pipe everywhere else.
// Nothing is spent and nothing is kept: a session id this process never wrote
// down is a conversation that ends when the process does, which is a decision
// parked out of scope in
// `docs/warlock-brief-25-give-the-headless-cli-a-voice-questions-in-draft-and-a.md`
// rather than an oversight here.
const OVER: &str = "the conversation is over";

pub(crate) fn brief() -> Result<(), Error> {
    let standing = Standing::here(FOR_BRIEF)?;

    briefing(
        &standing,
        // One conversation for the whole run, built once: a `ChatAgent` carries
        // the session id that makes the second turn a reply to the first, so an
        // agent per turn would be an argument that forgot itself between two
        // Enters. Cheap to build, and no `claude` exists until the first turn
        // runs.
        &ChatAgent::new(),
        // The real read: the prompt on stdout and one cooked line off stdin,
        // which is what stops the run until somebody answers it.
        &mut asking::Stdin,
        &mut io::stdout(),
    )
}

// Split from `brief` for `config::prompted`'s reason: the environment is a
// parameter, the line is a script and `out` collects what a reader would have
// seen, so the whole conversation — the order the two files are read in, the
// instruction that opens it, the blank line that sends a turn and the EOF that
// ends it — runs against a temporary repository with nothing attached to stdin.
fn briefing<C: Converses, K: Asks, W: Write>(
    standing: &Standing,
    agent: &C,
    ask: &mut K,
    out: &mut W,
) -> Result<(), Error> {
    // Before the register is raised and before a word is sent, which is the
    // whole of what "a refusal with nothing spent" means here.
    let instruction = reading(standing.repo_root())?;
    let agent = raised(agent);

    say(out, OPENING);
    // The instruction is its own turn and not a preface to the reader's first
    // one, exactly as `/brief` sends it: it ends by telling the model to open by
    // asking what the change is, so a turn that carried somebody's own opening
    // underneath it would be answering a question before it was put.
    turned(&agent, &instruction, out);

    // Lines until a blank one, then those lines as one turn. Multi-line prose is
    // what a brief is argued in, and a terminal has no other way to say "I have
    // not finished typing" — the cost is that a turn cannot itself contain a
    // blank line, which is written down as accepted in the brief.
    let mut typed: Vec<String> = Vec::new();
    // `?` and not a reported line: a stdin that cannot be read is not a fact
    // about this turn, and a conversation that carried on would be asking a
    // sequence of questions into a pipe that is broken.
    while let Some(line) = ask.ask(PROMPT)? {
        // Trimmed at the end alone: a pipe's newline is never part of what
        // somebody meant to say, and leading spaces are — a list or an indented
        // block is prose a brief is argued in.
        let line = line.trim_end();
        if !line.trim().is_empty() {
            typed.push(line.to_owned());
            continue;
        }
        // Enter on a line nobody has typed anything above. Nothing is sent: an
        // empty turn is the model asked to answer silence, and it would cost
        // somebody money to be told so.
        if typed.is_empty() {
            continue;
        }

        turned(&agent, &typed.join("\n"), out);
        // Cleared whatever the turn came to: the lines were sent, and a failed
        // turn is not a reason to send them again behind the reader's back.
        typed.clear();
    }

    // The newline is because the prompt just asked has none and the cursor is
    // still sitting on it.
    drop(writeln!(out));
    say(out, OVER);
    Ok(())
}

// Both of `/brief`'s optional files, in the panel's own order and for the
// panel's reason — see `chatting::brief_reading`: the template is read first, so
// a repository with both files broken is one refusal rather than warlock
// reporting its own reading order.
//
// The directory `briefs.toml` names is read and not kept, which is deliberate
// rather than waste: the refusal it can make is owed before the first turn, and
// where a written brief goes is a question this road cannot yet be asked.
// Nothing is cached, so either file edited between two runs takes effect.
fn reading(root: &Path) -> Result<String, Error> {
    let instruction = brief_template(root)
        .map(|template| brief_instruction(&template))
        .map_err(|source| Error::Template { source })?;
    load_briefs(root).map_err(|source| Error::Briefs { source })?;

    Ok(instruction)
}

// The register, named once here rather than at the call: the panel raises the
// same two words over the same conversation — see `chatting::asking` — and a
// brief argued at a shell is the same work at the same level. `raised` and not a
// second `ChatAgent`, so the session id is the one the agent was built with.
fn raised<C: Converses>(agent: &C) -> C {
    Converses::raised(agent, BRIEF_MODEL, BRIEF_EFFORT)
}

// One turn sent and what came back said, which is the whole of what this road
// does with the model.
//
// A turn that failed is a line and the prompt again rather than the end of the
// run: what reaches here is a missing binary, a timeout and a cancel, and ending
// an argument that may be twenty turns old over any of the three would throw
// away the one thing this command exists to produce. The session id has not
// moved, so the turn after a failed one is still the same conversation. The
// wording is `Ending`'s, so a failed turn reads the same here as on the panel.
fn turned<C: Converses, W: Write>(agent: &C, message: &str, out: &mut W) {
    match agent.turn(message) {
        // Whole and unflattened, which is the point of the command: a brief is
        // argued in paragraphs, and `one_line` is for a refusal rather than for
        // something somebody is meant to read.
        Ok(reply) => say(out, &reply),
        Err(error) => say(out, &ending_for(&error).line()),
    }
}

// A failed write is ignored, for `planned::say`'s reason: `warlock brief | head`
// is a closed stdout, and there is nothing useful to say to somebody whose
// screen has gone away.
fn say<W: Write>(out: &mut W, fact: &str) {
    drop(writeln!(out, "warlock: {fact}"));
}

// Every test drives a temporary repository, a scripted line and a stand-in
// model, so none of them reads the terminal the developer is sitting at or
// spawns a `claude`.
#[cfg(test)]
#[path = "tests/briefing.rs"]
mod tests;
