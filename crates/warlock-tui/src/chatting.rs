//! The conversation the binary keeps: an agent, at most one turn in flight, the
//! draft at the foot of the panel, and the window a written brief opens in.
//!
//! The register the conversation is in is deliberately not held here. It lives
//! on the panel, the one part of the app a failed pact restores untouched, so a
//! copy would be a second answer to which mode the reader is in. Nothing here
//! returns an error either: a missing `claude`, a non-zero exit, a timeout, an
//! empty answer and a Ctrl-C are five facts and one consequence, because an
//! event loop a bad answer could end would take the tree down with it.
//!
//! [`Chat::compose`] reads both of `/brief`'s files before it touches the mode,
//! so a file that cannot be read is a command that does not happen rather than a
//! register entered by a refusal. The mode is then set before the turn is sent,
//! because [`asking`] reads it off the app at the moment the worker starts.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::thread;
use std::time::Instant;

use warlock_engine::{DEFAULT_BRIEF_DIRECTORY, briefs, load_briefs, to_manifest_path};
use warlock_tui::{
    Activities, Activity, App, BRIEF_EFFORT, BRIEF_MODEL, CHAT_INSTRUCTION, Cancel, ChatAgent,
    Composed, Composer, Converses, Edited, Ending, Focus, Mode, Pasted, ScopePrompt, Submitted,
    TemplateError, WRITE_INSTRUCTION, brief_instruction, brief_template, ending_for, submitted_for,
};

use crate::error::one_line;
use crate::pacting::CancelGuard;
use crate::writing::{write_edit, write_opened};

// The worker reports on every path it takes itself, so a closed channel with no
// `Finished` on it means it panicked. Worded as the tail of `Ending::Broke`'s
// sentence rather than as a sentence of its own, so the line reads like every
// other failed turn.
const TURN_LOST: &str = "it stopped without saying how it went";

// What the commands are shown as on the card — never what is sent, which for
// the three that open a turn is a paragraph warlock wrote. Spelled here rather
// than taken from the draft because `submitted_for` trims, so `"  /brief  "`
// and `"/brief"` are one command and have to draw as one row.
const BRIEF_COMMAND: &str = "/brief";
const CHAT_COMMAND: &str = "/chat";
const WRITE_COMMAND: &str = "/write";
const PUSH_COMMAND: &str = "/push";
const PULL_COMMAND: &str = "/pull";

// Said on a *change* of register only, so a `/brief` typed in brief mode costs a
// turn and no line. Each names the way out, because that is the one thing a
// reader in a mode cannot work out from the screen.
const BRIEF_NOTE: &str =
    "brief mode — this conversation is now converging on a document. /chat leaves it.";

const CHAT_NOTE: &str = "chat mode — the brief is over and nothing is being converged on.";

// The two refusals. Both are a line and no turn: telling the model it is where
// it already is, or asking it to brief a conversation that was never aimed at a
// document, is a question nobody asked and money nobody meant to spend.
const ALREADY_CHATTING: &str = "already in chat mode — /brief is what changes that.";

const NOT_BRIEFING: &str = "/write is only in brief mode — /brief enters it";

/// The two commands that name a brief rather than the conversation, which is
/// the whole of what this value distinguishes: they take the same argument,
/// refuse the same way and hand the same spelling up, and only the word they
/// are typed as and the verb they do to the document differ.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum About {
    Push,
    Pull,
}

impl About {
    // The third refusal, and the same kind of line as the two above it: a bare
    // `/push` or `/pull` is about the document this session wrote, so a session
    // that has written none has nothing to act on — and is told the other road,
    // because a brief committed yesterday is the ordinary thing to be filing or
    // cutting and nothing on the screen says it can be named.
    //
    // Built from the command words rather than spelled around them, so the
    // sentence cannot name a command by a spelling the card does not use, and
    // worded once for the two of them so that the road out of one refusal
    // cannot come to read differently from the road out of the other.
    fn nothing_written(self) -> String {
        let (command, does) = match self {
            Self::Push => (PUSH_COMMAND, "files"),
            Self::Pull => (PULL_COMMAND, "cuts"),
        };
        format!(
            "{command} on its own {does} the brief {WRITE_COMMAND} wrote, and this session has \
             written none — name one, as `{command} docs/a-brief.md`"
        )
    }
}

/// What a submitted draft hands the loop, which is a brief and what is to be
/// done with it.
///
/// Two variants rather than a second `Option<String>` beside the first: the
/// loop answers them in two different places, and a pair of options would have
/// a fourth state — both at once — that a submit cannot produce and every
/// caller would still have to read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Wanted {
    Filed(String),
    Cut(String),
}

fn unreadable_template(error: &TemplateError) -> String {
    format!("{error} — /brief did nothing, so fix or remove the file and type it again")
}

fn brief_asking(root: &Path) -> Result<String, TemplateError> {
    brief_template(root).map(|template| brief_instruction(&template))
}

// Flattened where `unreadable_template` is not, because the material differs: a
// `briefs.toml` that will not parse carries the TOML parser's multi-line
// diagnostic inside it, and a note on the card is one line.
fn unreadable_briefs(error: &briefs::Error) -> String {
    format!(
        "{} — /brief did nothing, so fix or remove the file and type it again",
        one_line(&error.to_string())
    )
}

// Both of `/brief`'s optional files, re-read at every `/brief` so that either of
// them edited with `e` between two briefs takes effect without a restart.
//
// The `?` is what keeps a repository with both files broken to one note: the
// template is read first and stops the command, and the `briefs.toml` line is
// what the next `/brief` says once that one is fixed. Two notes for one
// keystroke would be warlock reporting its own reading order.
fn brief_reading(root: &Path) -> Result<(String, String), String> {
    let instruction = brief_asking(root).map_err(|error| unreadable_template(&error))?;
    let directory = load_briefs(root).map_err(|error| unreadable_briefs(&error))?;

    Ok((instruction, directory))
}

pub(crate) struct Chat<C> {
    // Built once for the life of the process and never spoken to directly: a
    // `ChatAgent` carries the session id that makes the second question a reply
    // to the first, so one per turn would be a conversation that forgot itself
    // between two Enters. Each turn works off a copy wired to its own cancel and
    // activity port — see `wired`.
    agent: C,
    turn: Option<Chatting>,
    root: PathBuf,
    // Here rather than on the `App` for the reason it was a loop local before: a
    // pact that ends with nothing recorded puts a copy of the app taken before
    // it back over the live one, so a draft stored there would be a draft a run
    // could swallow half a sentence into. Nothing a run does can reach this.
    composer: Composer,
    // Where a `/write` in this conversation would propose to put the document.
    // Written at every `/brief` and read at `/write`, which is why `/write` can
    // never fail for want of a file.
    directory: String,
    prompt: ScopePrompt,
    // What `/write` last put on disk in this session, manifest-relative, and
    // what `/push` would file. The most recent write and not the first: a
    // reader who wrote the document twice meant the second one, and the file
    // they are looking at is the one they just watched land. Nothing but a
    // write that really happened sets this, so a refused path, a missing
    // section and a disk that would not take the file all leave it as it was.
    written: Option<String>,
}

impl Chat<ChatAgent> {
    pub(crate) fn new(root: impl Into<PathBuf>) -> Self {
        Self::with_agent(root, ChatAgent::new())
    }
}

impl<C: Converses> Chat<C> {
    // The seam: a test drives the very value the loop keeps — the agent and the
    // turn together — against a stand-in that answers out of memory, so the
    // loop's own submit path is something that has run more than once.
    pub(crate) fn with_agent(root: impl Into<PathBuf>, agent: C) -> Self {
        Self {
            agent,
            turn: None,
            root: root.into(),
            composer: Composer::default(),
            directory: DEFAULT_BRIEF_DIRECTORY.to_owned(),
            prompt: ScopePrompt::default(),
            written: None,
        }
    }

    pub(crate) const fn composer(&self) -> &Composer {
        &self.composer
    }

    // Handed over once a round from the draw, off the same measurement the frame
    // is cut by: Home, End, Up and Down move by display row, and a row is only a
    // row once something has said how wide the field is. Told every round rather
    // than at a resize, because a width somebody had to notice and re-send is a
    // width somebody can forget to send.
    pub(crate) const fn set_composer_width(&mut self, width: u16) {
        self.composer.set_width(width);
    }

    pub(crate) const fn write_prompt(&self) -> &ScopePrompt {
        &self.prompt
    }

    #[cfg(test)]
    pub(crate) fn directory(&self) -> &str {
        &self.directory
    }

    // Read twice a round, and both readings come from here so the key and the
    // field cannot disagree: the composer is muted for exactly as long as this is
    // true, and Ctrl-C stops the turn rather than the session for exactly as long
    // as it is true.
    pub(crate) const fn answering(&self) -> bool {
        self.turn.is_some()
    }

    pub(crate) fn ask(&mut self, app: &mut App, message: &str, now: Instant) {
        self.say(app, message, message, Asked::Answer, now);
    }

    // `shown` and `sent` are two strings rather than one so a synthesized turn
    // can put the word that was typed on the card while the model gets the
    // paragraph: a screen of warlock's prose in the place the reader's own
    // questions go would be warlock putting words in their mouth.
    //
    // The question goes on the card before the worker starts, which is also what
    // brings the thread to the front, so somebody who has just asked something is
    // looking at it from the instant they asked. `now` is the caller's clock —
    // the instant the key was pressed — because a turn is as old as the question,
    // not as old as the first thing the model got round to saying.
    //
    // The mode is read off the app here, at the moment the worker starts, and is
    // the whole of what `asking` needs.
    pub(crate) fn say(
        &mut self,
        app: &mut App,
        shown: &str,
        sent: &str,
        asked: Asked,
        now: Instant,
    ) {
        app.panel_mut().start_turn(shown, now);
        self.turn = Some(start_turn(
            sent,
            &asking(&self.agent, app.panel().mode()),
            asked,
        ));
        self.settle_field();
    }

    // Derived from the turn rather than set and cleared, and called at exactly
    // the two points a turn can change: `say` starts one, `keep_up` is the only
    // thing that ends one. A turn ends five ways and all five arrive in
    // `keep_up`, so "however it ends, the field comes back" is one line here
    // rather than a flag five endings have to remember to unset.
    //
    // A pact is deliberately not a reason to mute. The two workers share nothing,
    // and a reader watching a long run is exactly who most wants to ask something
    // about the repository it is walking.
    fn settle_field(&mut self) {
        self.composer.set_muted(self.turn.is_some());
    }

    // The turn is deliberately not taken down here: the worker still has one
    // thing to say — that it was cancelled — and it says it through `keep_up`
    // like any other ending, which is what puts the cancelled line under the work
    // that had already arrived and hands the keyboard back.
    pub(crate) fn stop(&self) {
        if let Some(chatting) = self.turn.as_ref() {
            chatting.cancel.cancel();
        }
    }

    // The one thing a draft can hand back to the loop: the brief a `/push`
    // asks to file or a `/pull` asks to cut. Which board either of them reaches
    // is a manifest, a home and a key store away, and none of the three is this
    // value's — so the commands are recognised here and answered there.
    pub(crate) fn compose(
        &mut self,
        app: &mut App,
        outcome: Composed,
        now: Instant,
    ) -> Option<Wanted> {
        match outcome {
            Composed::Typing(next) => self.composer = next,
            Composed::Leave => app.set_focus(Focus::Panel),
            Composed::Submit => return self.submit(app, now),
        }

        None
    }

    fn submit(&mut self, app: &mut App, now: Instant) -> Option<Wanted> {
        // Taken before the field is emptied, and emptied by replacing it
        // outright rather than by unmuting: the muting comes back from
        // `settle_field` on the turn alone.
        let draft = self.composer.draft().to_owned();
        self.composer = Composer::default();

        match submitted_for(&draft) {
            Submitted::Message => self.ask(app, &draft, now),
            // The load, then the mode, then the turn. Both files are read before
            // the mode is touched so a refusal cannot leave the conversation in a
            // register it never entered; the mode is set before the turn is sent
            // so the instruction that enters brief mode is asked at brief mode's
            // level. The note is on the *change*, so a second `/brief` costs a
            // turn and no line.
            Submitted::Brief => match brief_reading(&self.root) {
                Ok((instruction, directory)) => {
                    self.directory = directory;
                    if app.panel_mut().set_mode(Mode::Brief) {
                        app.panel_mut().note(BRIEF_NOTE, now);
                    }
                    self.say(app, BRIEF_COMMAND, &instruction, Asked::Answer, now);
                }
                Err(line) => app.panel_mut().note(line, now),
            },
            Submitted::Chat => {
                if app.panel_mut().set_mode(Mode::Chat) {
                    app.panel_mut().note(CHAT_NOTE, now);
                    self.say(app, CHAT_COMMAND, CHAT_INSTRUCTION, Asked::Answer, now);
                } else {
                    app.panel_mut().note(ALREADY_CHATTING, now);
                }
            }
            // Asked of the app rather than of anything this value remembers,
            // because that is the state the border title is drawn from: two
            // readings of the register would eventually be two answers, and the
            // refusal would contradict the header.
            Submitted::Write => {
                if app.panel().mode() == Mode::Brief {
                    self.say(app, WRITE_COMMAND, WRITE_INSTRUCTION, Asked::Document, now);
                } else {
                    app.panel_mut().note(NOT_BRIEFING, now);
                }
            }
            // Two commands about a file rather than about the conversation, so
            // neither asks anything of the model and neither says anything
            // about the mode: each is about the brief it was handed, or the one
            // `/write` wrote, whichever register the reader has since gone back
            // to. The refusals are the whole of what this value decides about
            // them; the brief goes up to the loop, which holds the manifest
            // that says which board it reaches.
            Submitted::Push(named) => {
                return self
                    .brief_for(app, named, About::Push, now)
                    .map(Wanted::Filed);
            }
            Submitted::Pull(named) => {
                return self
                    .brief_for(app, named, About::Pull, now)
                    .map(Wanted::Cut);
            }
            // The line is asked of the value rather than restated here, so the
            // list of commands that exist is written down in one place.
            said @ Submitted::Refused => {
                if let Some(line) = said.refusal() {
                    app.panel_mut().note(line, now);
                }
            }
        }

        None
    }

    // The one spelling of a brief's path, made here for the reason
    // `write_submit` makes it before it writes: what goes up to the loop is the
    // manifest's own spelling, so a path somebody typed and a path remembered
    // from `/write` cannot arrive as two different strings naming one file.
    //
    // A typed path is read against the repository root rather than the working
    // directory, which is what the thread already names files by, and the
    // engine's own sentence is what refuses one that climbs out of it.
    //
    // One call for both commands, with `about` deciding nothing but the words
    // of the refusal: a `/pull` that resolved its path a second way would be a
    // pull of a document a `/push` would have filed somewhere else.
    fn brief_for(
        &self,
        app: &mut App,
        named: Option<&str>,
        about: About,
        now: Instant,
    ) -> Option<String> {
        let Some(named) = named else {
            if self.written.is_none() {
                app.panel_mut().note(about.nothing_written(), now);
            }
            return self.written.clone();
        };

        match to_manifest_path(&self.root, named) {
            Ok(stored) => Some(stored),
            Err(source) => {
                app.panel_mut().note(one_line(&source.to_string()), now);
                None
            }
        }
    }

    pub(crate) fn paste(&mut self, outcome: Pasted) {
        let Pasted::Typing(next) = outcome;
        self.composer = next;
    }

    // `apply_turn` hands back a document only on the round a `/write` turn
    // answered in, so every other ending leaves the window closed and the line
    // already on the thread is the whole report. The path is proposed into a
    // directory settled turns ago, so nothing here reads a file or can fail.
    pub(crate) fn keep_up(&mut self, app: &mut App, now: Instant) {
        if let Some(document) = apply_turn(&mut self.turn, app, now) {
            self.prompt = write_opened(&self.root, &self.directory, &document);
        }
        self.settle_field();
    }

    // Assigned rather than replaced: a keystroke that wrote nothing — which is
    // every keystroke but the Enter that lands the file — leaves the session
    // remembering the document written before it.
    pub(crate) fn write(&mut self, app: &mut App, edited: Edited, now: Instant) {
        let wrote = write_edit(app, &self.root, &self.prompt, edited, now);
        self.prompt = wrote.prompt;
        if let Some(written) = wrote.written {
            self.written = Some(written);
        }
    }
}

// A turn from the point of view of the thread drawing the screen: what the
// worker has to say, and how to tell it to stop. No join handle — `Running`
// keeps the app as it stood before the keystroke so a failed pact can put it
// back, and a turn needs no such thing because it changes no row and no file.
pub(crate) struct Chatting {
    // Closed by the worker dropping its end, which is how a panic is noticed.
    pub(crate) events: Receiver<TurnEvent>,
    pub(crate) cancel: CancelGuard,
    // Never read by the worker: this is the drain's, and it decides whether the
    // answer is handed back as well as put on the card.
    pub(crate) asked: Asked,
}

// What a turn was started to get, written down on the turn rather than as a flag
// beside it in the event loop. A loop-side flag would be a second record of which
// question is out, and the two would disagree the first time a `/write` ended in
// a way nobody remembered to clear it on — a cancel, a missing `claude`, a
// panicked worker. This rides on the turn, so it goes down exactly when the turn
// does. It changes nothing about how the turn is run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Asked {
    Answer,
    Document,
}

// Any number of `Doing` and then exactly one `Finished`; nothing after the end.
// One channel for both, so the answer cannot arrive before the work it came out
// of and the loop has one thing to poll.
#[derive(Debug)]
pub(crate) enum TurnEvent {
    Doing(Activity),
    Finished(Result<String, Ending>),
}

// Made per turn, attached to a copy of the agent that dies with the worker, so a
// `claude` still writing to a pipe after its turn was abandoned has nowhere to
// report into the turn after it. Called on the worker's thread from inside the
// run, so it does the least it can: one send and back. A failed send is ignored —
// a receiver that has gone away is an application that is quitting.
fn activity_port(events: &Sender<TurnEvent>) -> Activities {
    let events = events.clone();
    Activities::new(move |activity| {
        let _ = events.send(TurnEvent::Doing(activity));
    })
}

fn wired<C: Converses>(agent: &C, cancel: &Cancel, events: &Sender<TurnEvent>) -> C {
    agent.wired(cancel.clone(), activity_port(events))
}

// A clone of the one long-lived agent, never a rebuild — that is the property a
// mode change has to have. The turns already said are the material a brief is
// made of, and a second `ChatAgent` would be a second conversation that had never
// heard any of them. Everything but two words of the argument vector is identical
// between the arms, down to the latch the session id is claimed in.
//
// Chat mode is the agent untouched rather than the agent asked for its own
// defaults again, so a `WARLOCK_MODEL` or `WARLOCK_EFFORT` set for the session is
// carried rather than restated here.
fn asking<C: Converses>(agent: &C, mode: Mode) -> C {
    match mode {
        Mode::Brief => agent.raised(BRIEF_MODEL, BRIEF_EFFORT),
        Mode::Chat => agent.clone(),
    }
}

// The worker owns its own copy of the message and of the agent, so nothing is
// shared with the event loop but the channel and `cancel` — which is what makes
// the thread safe to abandon. The `JoinHandle` is dropped on purpose: joining is
// waiting, and this thread exists precisely so nobody waits for it.
pub(crate) fn spawn_turn<C: Converses>(
    message: &str,
    agent: &C,
    cancel: Cancel,
) -> Receiver<TurnEvent> {
    let (events, received) = mpsc::channel();
    let message = message.to_owned();
    let agent = wired(agent, &cancel, &events);
    thread::spawn(move || run_turn(&message, &agent, &cancel, &events));
    received
}

// A fresh guard every turn, because a cancel is final: the turn after a cancelled
// one has to start with a handle nobody has said stop to.
pub(crate) fn start_turn<C: Converses>(message: &str, agent: &C, asked: Asked) -> Chatting {
    let cancel = CancelGuard::new();
    Chatting {
        events: spawn_turn(message, agent, cancel.handle()),
        cancel,
        asked,
    }
}

// A free function rather than the body of the closure, so a test drives a real
// turn against a stand-in program on its own thread. Exactly one `Finished` is
// sent on every path out, and the wording of every ending is `ending_for`'s.
//
// The cancel arm is the one thing the seam cannot word for itself: a killed child
// comes back as interrupted I/O and nothing in the errno says who killed it, so a
// failure with the handle latched is read as a cancel here. An answer that beat
// the cancel by a hair is kept — it is a real answer, and discarding it would be
// a lie in the other direction.
fn run_turn<C: Converses>(message: &str, agent: &C, cancel: &Cancel, events: &Sender<TurnEvent>) {
    let finished = match agent.turn(message) {
        Ok(answer) => Ok(answer),
        Err(_) if cancel.is_cancelled() => Err(Ending::Cancelled),
        Err(error) => Err(ending_for(&error)),
    };
    // Ignored for the reason the activity port's sends are: a receiver that has
    // gone away is an application that is quitting.
    let _ = events.send(TurnEvent::Finished(finished));
}

// Drained rather than received, so a burst of tool calls between two frames all
// arrives and nothing here can block: the tree still scrolls and the clocks still
// tick while the model thinks. `now` is the caller's clock and this reads none of
// its own, so a whole turn is drivable from a base instant in a test.
//
// Every ending is two lines and no error — one on the thread under whatever work
// had already arrived, one on the footer, both from `Ending::line` so they cannot
// drift. An answer says nothing on the footer: it is already on the card the
// question brought to the front.
//
// The one thing handed back is the document, on the one round a `/write` turn
// answers in. It comes back from here rather than being read off the card
// afterwards because this is where the turn is known to have been the write
// request; a loop that went looking for the newest answer would be a second
// opinion about which turn just ended.
pub(crate) fn apply_turn(
    chat: &mut Option<Chatting>,
    app: &mut App,
    now: Instant,
) -> Option<String> {
    let chatting = chat.as_ref()?;
    let asked = chatting.asked;

    let finished = loop {
        match chatting.events.try_recv() {
            // Filed under the live turn, which is the one this worker is
            // answering: what each activity comes to is the thread's business
            // and not this file's — a tool is its name and its one detail,
            // thinking and writing are the words for them, and a cost is summed
            // rather than drawn. See `Thread::record`.
            Ok(TurnEvent::Doing(activity)) => app.panel_mut().record_turn(&activity, now),
            Ok(TurnEvent::Finished(finished)) => break Some(finished),
            // Still going, and nothing new to say.
            Err(TryRecvError::Empty) => return None,
            Err(TryRecvError::Disconnected) => break None,
        }
    };

    // The turn is over on every path below, so the loop stops holding it before
    // anything is worded: what the reader does next — another question, or the
    // key that cancels — is answered by an empty slot rather than by a receiver
    // nobody will ever hear from again.
    chat.take();
    let finished = finished.unwrap_or_else(|| {
        Err(Ending::Broke {
            reason: TURN_LOST.to_owned(),
        })
    });

    match finished {
        Ok(answer) => {
            // Cloned only for the turn that asked for a document, and cloned
            // rather than moved because the answer belongs on the card first:
            // the reply is a turn of the conversation whatever is done with it,
            // and a `/write` whose answer went to the loop instead of the thread
            // would be a document nobody could read.
            let document = (asked == Asked::Document).then(|| answer.clone());
            app.panel_mut().answer_turn(answer, now);
            document
        }
        Err(ending) => {
            end(app, &ending, now);
            None
        }
    }
}

fn end(app: &mut App, ending: &Ending, now: Instant) {
    app.set_message(ending.line());
    app.panel_mut().end_turn(ending, now);
}

#[cfg(test)]
#[path = "tests/chatting.rs"]
mod tests;
