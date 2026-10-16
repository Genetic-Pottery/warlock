//! `warlock brief`: the panel's brief register held at a shell. One agent, one
//! session, a conversation that goes on for as long as somebody is typing into
//! it, and one document at the end of it.
//!
//! The panel's `/brief` is a *mode* of the conversation a reader is already in —
//! see [`mod@crate::chatting`] — and this is the whole conversation instead:
//! there is no chat to come from and none to go back to, which is what keeps the
//! prompt's vocabulary to prose. No `/chat` here, because this command *is*
//! brief mode; and the two register words are not this module's to choose, so
//! [`raised`] names them once and takes them from `claude.rs`, which is where
//! the panel takes them from too.
//!
//! Writing decides nothing about a brief either. The question is asked when a
//! reply ends in the panel's ready marker, the request is the panel's
//! [`WRITE_INSTRUCTION`], the path is [`proposed_path`]'s, and the shape, the
//! bytes and every refusal are [`landed`]'s — see [`mod@crate::writing`] — so the
//! only thing written here is the conversation around them: the question, the
//! offer over the cursor, the empty line that accepts it, and the run ending on
//! the file rather than going round again.
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

use crate::asking::{self, Asks};
use crate::claude::{
    BRIEF_EFFORT, BRIEF_MODEL, ChatAgent, Converses, WRITE_INSTRUCTION, brief_instruction,
    without_marker,
};
use crate::error::{Error, say};
use crate::standing::{FOR_BRIEF, Standing};
use crate::submission::{Submitted, submitted_for};
use crate::template::brief_template;
use crate::thread::ending_for;
use crate::writing::{Landed, landed, proposed_path};

// The cursor a turn is typed on, and `planned.rs`'s prompt down to the bytes: a
// bare mark, because what is being read is prose and not a command word or an
// answer to pick from. What a reader needs before they can type is the line
// above it.
const PROMPT: &str = "> ";

// Said once, before the first turn is sent, because the one thing a reader
// cannot work out from a bare `> ` is what the conversation is for.
const OPENING: &str = "brief mode — this conversation is converging on a document";

// EOF, which is Ctrl-D at a terminal and an exhausted pipe everywhere else.
// Nothing is spent and nothing is kept: a session id this process never wrote
// down is a conversation that ends when the process does, which is a decision
// parked out of scope in
// `docs/warlock-brief-25-give-the-headless-cli-a-voice-questions-in-draft-and-a.md`
// rather than an oversight here.
const OVER: &str = "the conversation is over";

// Every panel command word. `/brief` and `/chat` are a register this session
// cannot leave because it *is* that register; `/push`, `/draft`, `/pull` and
// `/resume` are subcommands of warlock's own, so the shell composes them rather
// than this prompt growing a third spelling of each. Refused rather than sent,
// for `submission.rs`'s reason: a mistyped command costs a line here and a turn
// if it goes.
const NO_COMMANDS: &str = "there are no commands here — every line is the brief, and the \
                           document is offered once the model says it has enough; \
                           `warlock push` and `warlock draft` are commands of their own";

// The panel's "Write the brief?", and its default for the panel's reason: the
// question comes up on no keystroke, and a write is the most expensive turn
// there is.
const WRITE_PROMPT: &str = "write the brief? [y/N] ";

const NOT_WRITTEN: &str = "not written — ask for it when you want it, and this comes back";

pub fn brief() -> Result<(), Error> {
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

// Split from `brief` so the environment is a parameter, the line is a script
// and `out` collects what a reader would have seen: the whole conversation — the order the two files are read in, the
// instruction that opens it, the blank line that sends a turn and the EOF that
// ends it — runs against a temporary repository with nothing attached to stdin.
fn briefing<C: Converses, K: Asks, W: Write>(
    standing: &Standing,
    agent: &C,
    ask: &mut K,
    out: &mut W,
) -> Result<(), Error> {
    let root = standing.repo_root();
    // Before the register is raised and before a word is sent, which is the
    // whole of what "a refusal with nothing spent" means here.
    let opening = reading(root)?;
    let agent = raised(agent);

    say(out, OPENING);
    // The instruction is its own turn and not a preface to the reader's first
    // one, exactly as `/brief` sends it: it ends by telling the model to open by
    // asking what the change is, so a turn that carried somebody's own opening
    // underneath it would be answering a question before it was put.
    let mut ready = conversed(&agent, &brief_instruction(&opening.shape), out);
    ask.discard_typed_ahead();

    // Lines until a blank one that nothing is waiting behind, then those lines as
    // one turn. A blank line with more already arrived behind it is a paragraph
    // break: a paste and a script deliver a whole message at once, and a person
    // who pressed Enter to send has nothing queued behind the key. Sending on
    // every blank line was the rule before, and it cut a three-paragraph message
    // into three turns, the later ones answering a question the model had not
    // yet asked.
    let mut typed: Vec<String> = Vec::new();
    // `?` and not a reported line: a stdin that cannot be read is not a fact
    // about this turn, and a conversation that carried on would be asking a
    // sequence of questions into a pipe that is broken.
    loop {
        if ready {
            ready = false;
            let Some(answer) = ask.ask(WRITE_PROMPT)? else {
                break;
            };
            // Neither a yes nor a no is somebody still talking: a line of prose
            // that lands on the question is the first line of their next turn,
            // and reading it as the default would throw it away.
            if !asking::yes(&answer) {
                say(out, NOT_WRITTEN);
                if !asking::no(&answer) {
                    typed.push(answer.trim_end().to_owned());
                }
                continue;
            }
            match written(&agent, root, &opening, ask, out)? {
                // The file is what the conversation was for, so there is nothing
                // to ask after it: the prompt does not come back and the process
                // ends on the line that named the path.
                Landing::Wrote => return Ok(()),
                Landing::Carry => continue,
                Landing::Over => break,
            }
        }
        let Some(line) = ask.ask(PROMPT)? else {
            break;
        };
        // Trimmed at the end alone: a pipe's newline is never part of what
        // somebody meant to say, and leading spaces are — a list or an indented
        // block is prose a brief is argued in.
        let line = line.trim_end();
        // The panel's own parser and not a comparison here, so a line that
        // opens with a path is prose and a command word is refused by the rules
        // the panel refuses it by. See `submission.rs`.
        match submitted_for(line) {
            Submitted::Message => {
                if !line.trim().is_empty() {
                    typed.push(line.to_owned());
                    continue;
                }
                // Enter on a line nobody has typed anything above. Nothing is
                // sent: an empty turn is the model asked to answer silence, and
                // it would cost somebody money to be told so.
                if typed.is_empty() {
                    continue;
                }
                if ask.waiting() {
                    typed.push(String::new());
                    continue;
                }
                while typed.last().is_some_and(String::is_empty) {
                    typed.pop();
                }

                ready = conversed(&agent, &typed.join("\n"), out);
                ask.discard_typed_ahead();
                // Cleared whatever the turn came to: the lines were sent, and a
                // failed turn is not a reason to send them again behind the
                // reader's back.
                typed.clear();
            }
            _ => say(out, NO_COMMANDS),
        }
    }

    // The newline is because the prompt just asked has none and the cursor is
    // still sitting on it.
    drop(writeln!(out));
    say(out, OVER);
    Ok(())
}

/// What the repository says about a brief, read once before the conversation
/// opens and held for as long as it runs.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Opening {
    /// The shape the model is asked for and the document is held to.
    ///
    /// One read and not two, which is the whole of why it is kept: the
    /// instruction that opened the conversation was built from these bytes, so
    /// checking the reply against them is holding the document to the shape the
    /// model was actually given. The panel reads the file again when it writes
    /// because its conversation can outlive an edit to it; a run of this command
    /// is one conversation and cannot.
    shape: String,
    /// Where a written brief goes, from `briefs.toml` and never guessed here.
    directory: String,
}

// Both of `/brief`'s optional files, in the panel's own order and for the
// panel's reason — see `chatting::brief_reading`: the template is read first, so
// a repository with both files broken is one refusal rather than warlock
// reporting its own reading order. Nothing is cached beyond the run, so either
// file edited between two runs takes effect.
fn reading(root: &Path) -> Result<Opening, Error> {
    let shape = brief_template(root).map_err(|source| Error::Template { source })?;
    let directory = load_briefs(root).map_err(|source| Error::Briefs { source })?;

    Ok(Opening { shape, directory })
}

/// What a yes to the write question came to, as the loop has to answer it.
enum Landing {
    /// A document is on disk: the run is over and the exit status is zero.
    Wrote,
    /// Nothing was written and the conversation goes on — a turn that never came
    /// back, or a document the shape turned down. Both are things the next turn
    /// can fix, and neither is the path's fault.
    Carry,
    /// EOF at the path prompt, which ends the run exactly as EOF at the
    /// conversation's own cursor does, with nothing written.
    Over,
}

// One write: the request the panel sends, the path warlock proposes for the
// reply, and the line that accepts or replaces it.
//
// The read blocks, which is what makes the offer worth something: the run waits
// at the cursor for as long as whoever said yes takes, and nothing here decides
// on their behalf. A path refused is the offer again and another line —
// a path that is taken is the one mistake a reader fixes by typing — while a
// document the shape turned down goes back to the conversation, because another
// path would be refused in exactly the same words.
fn written<C: Converses, K: Asks, W: Write>(
    agent: &C,
    root: &Path,
    opening: &Opening,
    ask: &mut K,
    out: &mut W,
) -> Result<Landing, Error> {
    // The panel's own request and not a second wording of it, so the document
    // asked for at a shell is the document asked for in the panel.
    let Some(reply) = replied(agent, WRITE_INSTRUCTION, out) else {
        // The ending is already said. There is no reply to write and none to
        // propose a path from, and the conversation is still there to ask again.
        return Ok(Landing::Carry);
    };
    // Off the document for the panel's reason — see `chatting::apply_turn` —
    // and otherwise said whole: it is the one reply that becomes bytes, and a
    // reader watching it go by is the only sight of it they get before the file
    // lands.
    let (reply, _) = without_marker(reply);
    say(out, &reply);

    // Proposed once, from the reply that just landed, and offered again under
    // every refusal: the proposal is a fact about the document rather than about
    // what somebody typed over it.
    let proposal = proposed_path(root, &opening.directory, &reply);
    ask.discard_typed_ahead();
    loop {
        say(out, &proposing(&proposal));
        let Some(line) = ask.ask(PROMPT)? else {
            return Ok(Landing::Over);
        };

        // An empty line is the proposal accepted, which is the whole of what
        // makes it an offer; anything else replaces it entirely, and nothing
        // here reads the line for anything else.
        let typed = line.trim();
        let typed = if typed.is_empty() {
            proposal.as_str()
        } else {
            typed
        };

        match landed(root, typed, &opening.shape, &reply) {
            Landed::Wrote(line) => {
                say(out, &line);
                return Ok(Landing::Wrote);
            }
            Landed::Path(rule) => say(out, &rule),
            Landed::Document(line) => {
                say(out, &line);
                return Ok(Landing::Carry);
            }
        }
    }
}

// The offer over the cursor, in `planned.rs`'s shape: warlock's own answer on
// the line above a bare prompt, with what Enter does said on it — there is no
// border here to draw a rule on, and a reader who has to guess what an empty
// line means will type the path out again every time.
fn proposing(path: &str) -> String {
    format!("the document goes to `{path}` — Enter writes it there, another path replaces it")
}

// The register, named once here rather than at the call: the panel raises the
// same two words over the same conversation — see `chatting::asking` — and a
// brief argued at a shell is the same work at the same level. `raised` and not a
// second `ChatAgent`, so the session id is the one the agent was built with.
fn raised<C: Converses>(agent: &C) -> C {
    Converses::raised(agent, BRIEF_MODEL, BRIEF_EFFORT)
}

// A conversation turn: the reply said with the ready marker taken off, and
// whether it was there.
fn conversed<C: Converses, W: Write>(agent: &C, message: &str, out: &mut W) -> bool {
    let Some(reply) = replied(agent, message, out) else {
        return false;
    };
    let (reply, ready) = without_marker(reply);
    if !reply.is_empty() {
        say(out, &reply);
    }
    ready
}

// One turn sent, which is the whole of what this road does with the model.
// `None` is a turn that failed, with the line that says so already printed.
//
// A turn that failed is a line and the prompt again rather than the end of the
// run: what reaches here is a missing binary, a timeout and a cancel, and ending
// an argument that may be twenty turns old over any of the three would throw
// away the one thing this command exists to produce. The session id has not
// moved, so the turn after a failed one is still the same conversation. The
// wording is `Ending`'s, so a failed turn reads the same here as on the panel.
fn replied<C: Converses, W: Write>(agent: &C, message: &str, out: &mut W) -> Option<String> {
    agent
        .turn(message)
        .map_err(|error| say(out, &ending_for(&error).line()))
        .ok()
}

// A failed write is ignored, for `planned::say`'s reason: `warlock brief | head`
// is a closed stdout, and there is nothing useful to say to somebody whose
// screen has gone away.
// Every test drives a temporary repository, a scripted line and a stand-in
// model, so none of them reads the terminal the developer is sitting at or
// spawns a `claude`.
#[cfg(test)]
#[path = "tests/briefing.rs"]
mod tests;
