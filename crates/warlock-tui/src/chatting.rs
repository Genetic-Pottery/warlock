//! The conversation the binary keeps: an agent, at most one turn in flight, the
//! draft at the foot of the panel, and the window a written brief opens in.
//!
//! [`Chat`] is one value because the rules between those parts are what this
//! module is. The register the conversation is in is deliberately *not* one of
//! them — it lives on the panel, which is the state the border title is drawn
//! from and the one part of the app a failed pact restores untouched, so a copy
//! here would be a second answer to which mode the reader is in.
//!
//! # A turn is [`pacting`](crate::pacting) over a much smaller job
//!
//! The same four parts in the same order: [`spawn_turn`] starts the worker and
//! hands back the channel, [`start_turn`] is everything the event loop keeps
//! about work it is not doing, [`run_turn`] is the worker's whole body as a free
//! function so a test can drive it with no thread at all, and [`apply_turn`]
//! drains with `try_recv` so no frame is ever spent waiting on a model. The
//! say-when is `pacting`'s own [`CancelGuard`], reused rather than written a
//! second time: dropping a `Chat` takes its `claude` with it, so no exit path
//! has to remember to stop the turn.
//!
//! What is not shared is the ending. Nothing here returns an error — a missing
//! `claude`, a non-zero exit, a timeout, an empty answer and a Ctrl-C are five
//! facts and one consequence, an [`Ending`] on the card and the same sentence on
//! the footer, with the session as usable for the next question as it was for
//! this one. An event loop a bad answer could end would be a chat that takes the
//! tree down with it.
//!
//! # Two orderings that are load-bearing
//!
//! [`Chat::compose`] reads both of `/brief`'s files before it touches the mode.
//! A file that is there and cannot be read is a command that does not happen at
//! all, so a mode set first would be a register entered by a refusal — and
//! neither warlock's own template nor its own default directory is ever quietly
//! put in place of one the repository meant.
//!
//! The mode is then set before the turn is sent, because [`asking`] reads the
//! mode off the app at the moment the worker starts: the instruction that enters
//! brief mode is itself asked at brief mode's level.
//!
//! `/write`, by contrast, reads nothing. The directory it proposes into was
//! settled at the last `/brief` and has been held since, which is the whole of
//! why a document twenty turns in the making cannot arrive at a window that
//! refuses to open.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::thread;
use std::time::Instant;

use warlock_engine::{DEFAULT_BRIEF_DIRECTORY, Manifest, briefs, load_briefs};
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

// What the three commands are shown as on the card — never what is sent, which
// is a paragraph warlock wrote. Spelled here rather than taken from the draft
// because `submitted_for` trims, so `"  /brief  "` and `"/brief"` are one
// command and have to draw as one row.
const BRIEF_COMMAND: &str = "/brief";
const CHAT_COMMAND: &str = "/chat";
const WRITE_COMMAND: &str = "/write";

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

    pub(crate) fn compose(&mut self, app: &mut App, outcome: Composed, now: Instant) {
        match outcome {
            Composed::Typing(next) => self.composer = next,
            Composed::Leave => app.set_focus(Focus::Panel),
            Composed::Submit => {
                // Taken before the field is emptied, and emptied by replacing it
                // outright rather than by unmuting: the muting comes back from
                // `settle_field` on the turn alone.
                let draft = self.composer.draft().to_owned();
                self.composer = Composer::default();

                match submitted_for(&draft) {
                    Submitted::Message => self.ask(app, &draft, now),
                    // The load, then the mode, then the turn. Both files are read
                    // before the mode is touched so a refusal cannot leave the
                    // conversation in a register it never entered; the mode is set
                    // before the turn is sent so the instruction that enters brief
                    // mode is asked at brief mode's level. The note is on the
                    // *change*, so a second `/brief` costs a turn and no line.
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
                    // Asked of the app rather than of anything this value
                    // remembers, because that is the state the border title is
                    // drawn from: two readings of the register would eventually be
                    // two answers, and the refusal would contradict the header.
                    Submitted::Write => {
                        if app.panel().mode() == Mode::Brief {
                            self.say(app, WRITE_COMMAND, WRITE_INSTRUCTION, Asked::Document, now);
                        } else {
                            app.panel_mut().note(NOT_BRIEFING, now);
                        }
                    }
                    // The line is asked of the value rather than restated here, so
                    // the list of commands that exist is written down in one place.
                    said @ Submitted::Refused => {
                        if let Some(line) = said.refusal() {
                            app.panel_mut().note(line, now);
                        }
                    }
                }
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

    pub(crate) fn write(
        &mut self,
        app: &mut App,
        manifest: &Manifest,
        edited: Edited,
        now: Instant,
    ) {
        self.prompt = write_edit(app, manifest, &self.root, &self.prompt, edited, now);
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
    match finished {
        Some(Ok(answer)) => {
            // Cloned only for the turn that asked for a document, and cloned
            // rather than moved because the answer belongs on the card first:
            // the reply is a turn of the conversation whatever is done with it,
            // and a `/write` whose answer went to the loop instead of the thread
            // would be a document nobody could read.
            let document = (asked == Asked::Document).then(|| answer.clone());
            app.panel_mut().answer_turn(answer, now);
            document
        }
        Some(Err(ending)) => {
            end(app, &ending, now);
            None
        }
        None => {
            end(
                app,
                &Ending::Broke {
                    reason: TURN_LOST.to_owned(),
                },
                now,
            );
            None
        }
    }
}

fn end(app: &mut App, ending: &Ending, now: Instant) {
    app.set_message(ending.line());
    app.panel_mut().end_turn(ending, now);
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc::{self, Receiver, Sender};
    use std::time::{Duration, Instant};

    use warlock_tui::{Activity, App, ChatAgent, Ending, INVOCATION_TIMEOUT, Line, Mode};

    use super::{Asked, Chat, Chatting, TurnEvent, apply_turn};
    use crate::pacting::CancelGuard;

    const ASKED: &str = "what is in crates/warlock-engine?";

    fn at(base: Instant, seconds: u64) -> Instant {
        base + Duration::from_secs(seconds)
    }

    fn asking(base: Instant) -> (App, Sender<TurnEvent>, Option<Chatting>) {
        let (events, received) = mpsc::channel();
        let mut app = App::default();
        app.panel_mut().start_turn(ASKED, base);
        (app, events, Some(chatting(received)))
    }

    fn chatting(received: Receiver<TurnEvent>) -> Chatting {
        turn_for(received, Asked::Answer)
    }

    fn writing(received: Receiver<TurnEvent>) -> Chatting {
        turn_for(received, Asked::Document)
    }

    fn turn_for(received: Receiver<TurnEvent>, asked: Asked) -> Chatting {
        Chatting {
            events: received,
            cancel: CancelGuard::new(),
            asked,
        }
    }

    fn drain(chat: &mut Option<Chatting>, app: &mut App, now: Instant) {
        assert_eq!(
            apply_turn(chat, app, now),
            None,
            "an ordinary turn handed something back to the loop"
        );
    }

    fn rows(app: &App, now: Instant) -> Vec<Line> {
        app.panel()
            .thread()
            .expect("a question has been asked")
            .lines(now)
    }

    fn said() -> Line {
        Line::Said {
            text: ASKED.to_owned(),
        }
    }

    fn clocked(seconds: u64, text: &str) -> Line {
        Line::Clocked {
            clock: format!("0:{seconds:02}"),
            text: text.to_owned(),
        }
    }

    #[test]
    fn work_lines_land_as_they_arrive_and_the_turn_stays_in_flight() {
        let base = Instant::now();
        let (mut app, events, mut chat) = asking(base);

        events
            .send(TurnEvent::Doing(Activity::Tool {
                name: "Read".to_owned(),
                detail: Some("src/lib.rs".to_owned()),
            }))
            .expect("the loop is still listening");
        events
            .send(TurnEvent::Doing(Activity::Thinking))
            .expect("the loop is still listening");
        drain(&mut chat, &mut app, at(base, 3));

        // Drained and returned: the worker has said nothing since, and this
        // call did not sit and wait for it to.
        assert!(chat.is_some(), "an unfinished turn was taken down");
        assert_eq!(
            rows(&app, at(base, 3)),
            vec![
                said(),
                clocked(3, "Read src/lib.rs"),
                clocked(3, "thinking")
            ]
        );
        // Nothing on the footer while a turn is going: the panel is where the
        // work is being shown, and the footer has its own things to say.
        assert_eq!(app.message(), None);
    }

    #[test]
    fn an_answer_lands_on_the_thread_and_takes_the_turn_down() {
        let base = Instant::now();
        let (mut app, events, mut chat) = asking(base);

        events
            .send(TurnEvent::Doing(Activity::Writing { bytes: 0 }))
            .expect("the loop is still listening");
        events
            .send(TurnEvent::Finished(Ok(
                "The tree and the manifest.".to_owned()
            )))
            .expect("the loop is still listening");
        drain(&mut chat, &mut app, at(base, 4));

        assert!(chat.is_none(), "an answered turn is still in flight");
        assert_eq!(
            rows(&app, at(base, 9)),
            vec![
                said(),
                // Frozen where the answer landed rather than counting up to
                // the `now` above: the turn is over.
                clocked(4, "writing"),
                Line::Text {
                    text: "The tree and the manifest.".to_owned()
                },
            ]
        );
        // An answer is on screen where it was asked for, so the footer says
        // nothing about it.
        assert_eq!(app.message(), None);
    }

    #[test]
    fn a_write_turn_hands_its_answer_back_and_leaves_it_on_the_card_as_well() {
        // The one difference `/write` makes to the drain, and both halves of it:
        // the loop is handed the document — which is what it opens the path
        // prompt over — and the card keeps the reply, which is what the reader
        // reads and what the write itself takes its bytes from.
        const DOCUMENT: &str = "# Scopes\n\nA boundary somebody drew.";

        let base = Instant::now();
        let (events, received) = mpsc::channel();
        let mut app = App::default();
        app.panel_mut().start_turn("/write", base);
        let mut chat = Some(writing(received));

        events
            .send(TurnEvent::Finished(Ok(DOCUMENT.to_owned())))
            .expect("the loop is still listening");
        let document = apply_turn(&mut chat, &mut app, at(base, 4));

        assert_eq!(document.as_deref(), Some(DOCUMENT));
        assert!(chat.is_none(), "an answered turn is still in flight");
        assert_eq!(
            rows(&app, at(base, 4)),
            vec![
                Line::Said {
                    text: "/write".to_owned()
                },
                clocked(4, "waiting"),
                // The card holds the answer as the lines it is made of, which
                // is how every answer lands on it: what matters here is that
                // the document is still there as well as in the loop's hand.
                Line::Text {
                    text: "# Scopes".to_owned()
                },
                Line::Text {
                    text: String::new()
                },
                Line::Text {
                    text: "A boundary somebody drew.".to_owned()
                },
            ],
            "the reply left the card it was answered on"
        );
        assert_eq!(app.message(), None);
    }

    #[test]
    fn a_write_turn_that_fails_hands_nothing_back_however_it_failed() {
        // Every ending, over the turn that asked for a document: a failure is a
        // failure, so there is nothing for the loop to open a prompt over and
        // nothing anywhere for it to write.
        let endings = [
            Ending::Cancelled,
            Ending::NoModel {
                program: "claude".to_owned(),
            },
            Ending::Failed {
                code: Some(3),
                stderr: "boom".to_owned(),
            },
            Ending::TimedOut {
                after: INVOCATION_TIMEOUT,
            },
            Ending::NothingSaid,
        ];

        for ending in endings {
            let base = Instant::now();
            let (events, received) = mpsc::channel();
            let mut app = App::default();
            app.panel_mut().start_turn("/write", base);
            let mut chat = Some(writing(received));

            events
                .send(TurnEvent::Finished(Err(ending.clone())))
                .expect("the loop is still listening");

            assert_eq!(
                apply_turn(&mut chat, &mut app, at(base, 2)),
                None,
                "{ending:?} handed the loop a document to write"
            );
            assert!(chat.is_none(), "{ending:?} left the turn in flight");
            assert_eq!(app.message(), Some(ending.line().as_str()), "{ending:?}");
        }
    }

    #[test]
    fn a_write_turn_whose_worker_died_hands_nothing_back_either() {
        // The sixth ending, which arrives as a closed channel rather than as a
        // message: the same nothing, by the road that has no `Ending` on it.
        let base = Instant::now();
        let (events, received) = mpsc::channel();
        let mut app = App::default();
        app.panel_mut().start_turn("/write", base);
        let mut chat = Some(writing(received));

        drop(events);

        assert_eq!(apply_turn(&mut chat, &mut app, at(base, 2)), None);
        assert!(chat.is_none(), "a dead worker is still in flight");
        assert!(app.message().is_some(), "the footer says the turn is over");
    }

    #[test]
    fn a_write_turn_still_in_flight_hands_nothing_back() {
        // The round in the middle: work has arrived, the answer has not, and
        // the prompt has nothing to open over yet.
        let base = Instant::now();
        let (events, received) = mpsc::channel();
        let mut app = App::default();
        app.panel_mut().start_turn("/write", base);
        let mut chat = Some(writing(received));

        events
            .send(TurnEvent::Doing(Activity::Thinking))
            .expect("the loop is still listening");

        assert_eq!(apply_turn(&mut chat, &mut app, at(base, 1)), None);
        assert!(chat.is_some(), "an unfinished turn was taken down");
    }

    #[test]
    fn every_ending_is_one_line_on_the_thread_and_the_same_line_on_the_footer() {
        let endings = [
            Ending::Cancelled,
            Ending::NoModel {
                program: "claude".to_owned(),
            },
            Ending::Failed {
                code: Some(3),
                stderr: "boom\nand more boom".to_owned(),
            },
            Ending::TimedOut {
                after: INVOCATION_TIMEOUT,
            },
            Ending::NothingSaid,
            Ending::Broke {
                reason: "a pipe broke".to_owned(),
            },
        ];

        for ending in endings {
            let base = Instant::now();
            let (mut app, events, mut chat) = asking(base);

            events
                .send(TurnEvent::Finished(Err(ending.clone())))
                .expect("the loop is still listening");
            drain(&mut chat, &mut app, at(base, 2));

            assert!(chat.is_none(), "{ending:?} left the turn in flight");
            // One row for the question and one for the ending, whichever
            // ending it is: a failure costs the reader a line, not a screen.
            assert_eq!(
                rows(&app, at(base, 30)),
                vec![said(), clocked(2, &ending.line())],
                "{ending:?}"
            );
            // And the same sentence for a reader who is looking at another
            // card. Same string, from the same place: two spellings of one
            // failure is two things to keep in step.
            assert_eq!(app.message(), Some(ending.line().as_str()), "{ending:?}");
            let turn = app
                .panel()
                .thread()
                .and_then(|thread| thread.turns().last().map(|turn| (**turn).clone()))
                .expect("the turn is on the card");
            assert_eq!(turn.ending(), Some(&ending));
            assert_eq!(turn.answer(), None);
            // Nothing was returned from any of this: the arm above is the
            // whole of what a failed turn does to the loop.
        }
    }

    #[test]
    fn a_cancel_keeps_every_line_that_arrived_before_it_and_adds_one() {
        // What Ctrl-C during a turn leaves behind. The work the model was seen
        // doing really happened, so it stays where it is; the cancel is one more
        // line under it, in the ordinary shape of an ending. A cancel that
        // cleared the turn would throw away the two tool calls the reader was
        // watching, which is the reader's evidence for what they just stopped.
        let base = Instant::now();
        let (mut app, events, mut chat) = asking(base);

        for name in ["Read", "Grep"] {
            events
                .send(TurnEvent::Doing(Activity::Tool {
                    name: name.to_owned(),
                    detail: Some("src/lib.rs".to_owned()),
                }))
                .expect("the loop is still listening");
        }
        drain(&mut chat, &mut app, at(base, 2));
        assert!(chat.is_some(), "the turn is still in flight");

        // And then the cancel, which arrives as the worker's one ending like
        // any other — the loop does not take the turn down at the keystroke.
        events
            .send(TurnEvent::Finished(Err(Ending::Cancelled)))
            .expect("the loop is still listening");
        drain(&mut chat, &mut app, at(base, 4));

        assert!(chat.is_none(), "a cancelled turn is still in flight");
        assert_eq!(
            rows(&app, at(base, 30)),
            vec![
                said(),
                clocked(2, "Read src/lib.rs"),
                // The line that was newest when the cancel landed, frozen at
                // the moment it landed: it had been ticking since the drain
                // above, which is the clock rule the account already follows.
                clocked(4, "Grep src/lib.rs"),
                clocked(4, &Ending::Cancelled.line()),
            ]
        );
    }

    #[test]
    fn a_worker_that_dies_without_saying_how_it_went_still_ends_the_turn() {
        let base = Instant::now();
        let (mut app, events, mut chat) = asking(base);

        // A panicked worker, as this thread sees it: its end of the channel is
        // gone and no ending ever arrived.
        drop(events);
        drain(&mut chat, &mut app, at(base, 5));

        assert!(chat.is_none(), "a dead worker is still in flight");
        let line = app.message().expect("the footer says the turn is over");
        assert!(line.contains("stopped without saying"), "{line}");
        assert_eq!(rows(&app, at(base, 40)), vec![said(), clocked(5, line)]);
    }

    #[test]
    fn a_second_question_runs_as_ordinarily_as_the_first_did() {
        let base = Instant::now();
        let (mut app, events, mut chat) = asking(base);
        events
            .send(TurnEvent::Finished(Err(Ending::NothingSaid)))
            .expect("the loop is still listening");
        drain(&mut chat, &mut app, at(base, 1));

        // The session is as usable as it was: a new turn, a new channel, and
        // the failed one still on the card above it.
        let (again, received) = mpsc::channel();
        let mut chat = Some(chatting(received));
        app.panel_mut()
            .start_turn("and which of those is the biggest?", at(base, 10));
        again
            .send(TurnEvent::Finished(Ok("The engine.".to_owned())))
            .expect("the loop is still listening");
        drain(&mut chat, &mut app, at(base, 12));

        assert!(chat.is_none());
        assert_eq!(
            rows(&app, at(base, 12)),
            vec![
                said(),
                clocked(1, &Ending::NothingSaid.line()),
                Line::Said {
                    text: "and which of those is the biggest?".to_owned()
                },
                // A turn that answered without ever being seen doing anything
                // keeps the placeholder it was drawn with, frozen at the
                // moment the answer landed: two seconds of waiting is what
                // happened, and the thread says so.
                clocked(2, "waiting"),
                Line::Text {
                    text: "The engine.".to_owned()
                },
            ]
        );
    }

    fn words(agent: &warlock_tui::ChatAgent) -> Vec<String> {
        agent
            .args()
            .iter()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect()
    }

    fn value_of<'a>(vector: &'a [String], flag: &str) -> Option<&'a str> {
        let named = vector.iter().position(|word| word == flag)?;
        vector.get(named + 1).map(String::as_str)
    }

    #[test]
    fn a_mode_asks_the_one_conversation_at_a_different_level_and_nothing_else() {
        // What `asking` is allowed to change, asserted on the vector a turn
        // would really run with. Chat mode is the long-lived agent untouched;
        // brief mode is the same agent with one word moved. Nothing here builds
        // a second `ChatAgent`, and neither does the function under test — both
        // vectors name the same session, which is the property the whole design
        // rests on.
        let agent = warlock_tui::ChatAgent::new();
        let question = words(&super::asking(&agent, warlock_tui::Mode::Chat));
        let brief = words(&super::asking(&agent, warlock_tui::Mode::Brief));

        assert_eq!(
            question,
            words(&agent),
            "chat mode is not the agent as built"
        );
        assert_eq!(brief.len(), question.len());
        let moved: Vec<(&String, &String)> = brief
            .iter()
            .zip(&question)
            .filter(|(brief, question)| brief != question)
            .collect();
        assert_eq!(
            moved.len(),
            2,
            "a mode changed something other than how hard the turn thinks and \
             which model thinks it: {moved:?}",
        );
        assert_eq!(
            value_of(&brief, "--effort"),
            Some(warlock_tui::BRIEF_EFFORT)
        );
        assert_eq!(value_of(&brief, "--model"), Some(warlock_tui::BRIEF_MODEL));
        assert_eq!(
            value_of(&brief, "--session-id"),
            value_of(&question, "--session-id"),
            "a mode change started a second conversation",
        );
        assert_eq!(
            value_of(&brief, "--system-prompt"),
            value_of(&question, "--system-prompt"),
        );
        assert_eq!(value_of(&brief, "--tools"), value_of(&question, "--tools"));
    }

    #[test]
    fn a_mode_change_is_a_state_of_the_conversation_and_never_a_second_one() {
        // The same property as the test above, said where a mode change really
        // happens: the composer's own `apply_compose`, over the value the loop
        // keeps, with the commands submitted as they are typed. What is asserted
        // is the agent the conversation still holds four turns later — the one
        // long-lived `ChatAgent` — because *that* is what a second session would
        // show up in. A `/brief` that rebuilt the conversation would name a new
        // session id here, and the turns already said would be turns nothing had
        // ever heard.
        //
        // The `claude` is one that does not exist, so the four turns these
        // commands really start spawn nothing at all: no terminal, no network
        // and no model. The repository is an empty temporary directory, which
        // is a repository that has written no brief template: `/brief` reads
        // for one, finds none, and states warlock's own shape.
        let base = Instant::now();
        let repo = tempfile::tempdir().expect("a temporary directory");
        let mut app = App::default();
        let mut chat = Chat::with_agent(
            repo.path(),
            ChatAgent::new().with_program("/warlock/no/such/program"),
        );
        let opening = words(&chat.agent);
        let session = value_of(&opening, "--session-id").expect("a turn opens a conversation");
        let prompt = value_of(&opening, "--system-prompt").expect("a turn carries the prompt");

        // Where a brief would be written is the conversation's own now, and
        // this test is about the conversation rather than about that setting:
        // `/brief` settling it has its own assertions in `mod submitting`.
        for draft in ["why nine passes?", "/brief", "/brief", "/chat"] {
            chat.compose(
                &mut app,
                warlock_tui::Composed::Typing(warlock_tui::Composer::new(draft)),
                base,
            );
            chat.compose(&mut app, warlock_tui::Composed::Submit, base);
        }

        // The register was really entered and really left — otherwise the
        // vectors below would be equal for the dullest of reasons.
        assert_eq!(app.panel().mode(), Mode::Chat);
        assert_eq!(
            app.panel()
                .thread()
                .map_or(0, |thread| thread.turns().len()),
            4,
            "the commands did not cost the four turns they are supposed to",
        );
        assert_eq!(
            words(&chat.agent),
            opening,
            "a mode change rebuilt the conversation it is a state of",
        );

        // And said one fact at a time, so a failure names which of them went.
        let brief = words(&super::asking(&chat.agent, Mode::Brief));
        let question = words(&super::asking(&chat.agent, Mode::Chat));
        for (mode, vector) in [("brief", &brief), ("chat", &question)] {
            assert_eq!(
                value_of(vector, "--session-id"),
                Some(session),
                "a {mode}-mode turn is asked in another session",
            );
            assert_eq!(
                value_of(vector, "--system-prompt"),
                Some(prompt),
                "a {mode}-mode turn is asked under another prompt",
            );
        }
        // That the one prompt is `CHAT_SYSTEM_PROMPT` and says what it has to
        // say in both registers is `claude.rs`'s own test, on the constant
        // itself; what is asserted here is that a mode change never swaps it.
    }

    #[test]
    fn a_frame_with_no_turn_in_flight_does_nothing_at_all() {
        let base = Instant::now();
        let mut app = App::default();
        let before = app.clone();

        drain(&mut None, &mut app, at(base, 1));

        assert_eq!(app, before, "a frame with nothing running moved something");
    }

    #[cfg(unix)]
    mod unix {
        use std::path::{Path, PathBuf};
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::mpsc::{self, RecvTimeoutError};
        use std::time::{Duration, Instant};
        use std::{env, fs, process, thread};

        use warlock_tui::{Activity, App, Cancel, ChatAgent, Ending, Mode, ScopePrompt};

        use warlock_tui::Converses;

        use super::super::{Asked, Chat, TurnEvent, run_turn, spawn_turn, start_turn, wired};
        use super::{ASKED, at, chatting, clocked, drain, rows, said};

        const AT_MOST: Duration = Duration::from_secs(5);

        const NO_REPOSITORY: &str = "/warlock/no/such/repository";

        const TEARDOWN_TICK: Duration = Duration::from_millis(10);

        const NOT_A_PROGRAM: &str = "/warlock/no/such/program";

        const TURN: [&str; 5] = [
            r#"{"type":"system","subtype":"init","tools":["Read","Grep","Glob"]}"#,
            r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","id":"toolu_1","name":"Read","input":{"file_path":"src/lib.rs"}}]}}"#,
            r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"thinking","thinking":"a thought nobody is entitled to"}]}}"#,
            r#"{"type":"stream_event","event":{"type":"content_block_start","content_block":{"type":"text"}}}"#,
            r#"{"type":"result","subtype":"success","result":"The tree, the manifest and the pact.","total_cost_usd":0.0123}"#,
        ];

        const ANSWER: &str = "The tree, the manifest and the pact.";

        fn reported() -> Vec<Activity> {
            vec![
                Activity::Tool {
                    name: "Read".to_owned(),
                    detail: Some("src/lib.rs".to_owned()),
                },
                Activity::Thinking,
                Activity::Writing { bytes: 0 },
                Activity::Cost { usd: 0.0123 },
            ]
        }

        fn stand_in(script: &str) -> ChatAgent {
            ChatAgent::new()
                .with_program("/bin/sh")
                .with_args(["-c", script])
        }

        fn printing(lines: &[&str]) -> String {
            let arguments: Vec<String> = lines.iter().map(|line| format!("'{line}'")).collect();
            format!("printf '%s\\n' {}", arguments.join(" "))
        }

        fn turned(agent: &ChatAgent, cancel: &Cancel) -> Vec<TurnEvent> {
            let (events, received) = mpsc::channel();
            run_turn(ASKED, &wired(agent, cancel, &events), cancel, &events);
            received.try_iter().collect()
        }

        fn doings(events: &[TurnEvent]) -> Vec<Activity> {
            events
                .iter()
                .filter_map(|event| match event {
                    TurnEvent::Doing(activity) => Some(activity.clone()),
                    TurnEvent::Finished(_) => None,
                })
                .collect()
        }

        fn ended(events: &[TurnEvent]) -> Result<String, Ending> {
            let endings: Vec<_> = events
                .iter()
                .filter_map(|event| match event {
                    TurnEvent::Finished(finished) => Some(finished.clone()),
                    TurnEvent::Doing(_) => None,
                })
                .collect();
            assert_eq!(endings.len(), 1, "not exactly one ending: {events:?}");
            assert!(
                matches!(events.last(), Some(TurnEvent::Finished(_))),
                "something was said after the turn ended: {events:?}"
            );
            endings.into_iter().next().expect("just counted one")
        }

        fn ending(events: &[TurnEvent]) -> Ending {
            ended(events).expect_err("this stand-in never answers")
        }

        fn scratch(name: &str) -> PathBuf {
            static NEXT: AtomicUsize = AtomicUsize::new(0);

            let unique = NEXT.fetch_add(1, Ordering::Relaxed);
            let directory =
                env::temp_dir().join(format!("warlock-chat-{}-{name}-{unique}", process::id()));
            fs::create_dir_all(&directory).expect("a scratch directory under the temp directory");
            directory
        }

        fn clean_up(directory: &Path) {
            let _ = fs::remove_dir_all(directory);
        }

        fn size(path: &Path) -> Option<u64> {
            fs::metadata(path).ok().map(|file| file.len())
        }

        fn settled<C: Converses>(chat: &mut Chat<C>, app: &mut App, now: Instant) -> ScopePrompt {
            let waited = Instant::now();
            while chat.answering() && waited.elapsed() < AT_MOST {
                chat.keep_up(app, now);
                thread::sleep(Duration::from_millis(10));
            }
            assert!(!chat.answering(), "the turn never ended");
            chat.write_prompt().clone()
        }

        fn settle<C: Converses>(chat: &mut Chat<C>, app: &mut App, now: Instant) {
            assert_eq!(
                settled(chat, app, now),
                ScopePrompt::Closed,
                "an ordinary turn opened the write window"
            );
        }

        fn wait_for(path: &Path) {
            let waited = Instant::now();
            while size(path).is_none_or(|bytes| bytes == 0) && waited.elapsed() < AT_MOST {
                thread::sleep(Duration::from_millis(10));
            }
        }

        #[test]
        fn a_turn_that_answers_says_what_it_did_and_then_answers_once() {
            let events = turned(&stand_in(&printing(&TURN)), &Cancel::new());

            // The work as it happened, then the answer: one channel, in the
            // order the turn produced them.
            assert_eq!(doings(&events), reported());
            assert_eq!(ended(&events), Ok(ANSWER.to_owned()));
            // Said once more, because it is the promise the port is for: the
            // thought is not in there, and neither is the answer.
            let seen = format!("{:?}", doings(&events));
            assert!(!seen.contains("entitled"), "{seen}");
            assert!(!seen.contains("manifest"), "{seen}");
        }

        #[test]
        fn no_claude_to_ask_is_one_line_and_not_an_error() {
            let events = turned(
                &stand_in("true").with_program(NOT_A_PROGRAM),
                &Cancel::new(),
            );

            assert_eq!(
                ending(&events),
                Ending::NoModel {
                    program: NOT_A_PROGRAM.to_owned()
                }
            );
        }

        #[test]
        fn a_non_zero_exit_is_one_line_carrying_its_status_and_its_stderr() {
            let events = turned(&stand_in("echo boom >&2; exit 3"), &Cancel::new());

            let ended = ending(&events);
            match &ended {
                Ending::Failed { code, stderr } => {
                    assert_eq!(*code, Some(3));
                    assert_eq!(stderr.trim(), "boom", "stderr is carried, not dropped");
                }
                other => panic!("expected a non-zero exit, got {other:?}"),
            }
            // One row, however many lines the child wrote.
            assert!(ended.line().contains("exit status 3"), "{}", ended.line());
        }

        #[test]
        fn a_turn_that_says_nothing_ends_with_nothing_said() {
            for script in ["exit 0", "printf '\\n  \\n'"] {
                let events = turned(&stand_in(script), &Cancel::new());

                assert_eq!(ending(&events), Ending::NothingSaid, "`{script}`");
            }
        }

        #[test]
        fn a_turn_that_runs_too_long_is_stopped_and_says_so() {
            let agent = stand_in("sleep 30").with_timeout(Duration::from_millis(250));

            let started = Instant::now();
            let events = turned(&agent, &Cancel::new());
            let elapsed = started.elapsed();

            assert_eq!(
                ending(&events),
                Ending::TimedOut {
                    after: Duration::from_millis(250)
                }
            );
            assert!(
                elapsed < Duration::from_secs(10),
                "the turn waited {elapsed:?}, far past its timeout"
            );
        }

        #[test]
        fn a_turn_cancelled_before_it_starts_ends_as_cancelled() {
            let cancel = Cancel::new();
            cancel.cancel();

            let events = turned(&stand_in(&printing(&TURN)), &cancel);

            // The seam has no word for a cancel — a killed child is interrupted
            // I/O and the errno says nothing about who killed it — so this is
            // the fact the worker holds, said in the thread's own words rather
            // than as `the turn could not run — …`.
            assert_eq!(ending(&events), Ending::Cancelled);
        }

        #[test]
        fn a_turn_cancelled_from_another_thread_ends_once_and_promptly() {
            let cancel = super::super::CancelGuard::new();
            // The real five-minute timeout: the only thing that can end this
            // turn in time is the cancel.
            let received = spawn_turn(ASKED, &stand_in("sleep 300"), cancel.handle());

            let started = Instant::now();
            cancel.cancel();
            let first = received
                .recv_timeout(AT_MOST)
                .expect("a cancelled turn still says how it ended");
            let elapsed = started.elapsed();

            assert!(
                matches!(&first, TurnEvent::Finished(Err(Ending::Cancelled))),
                "{first:?}"
            );
            assert!(elapsed < AT_MOST, "the cancel took {elapsed:?}");

            // And nothing after it: the worker sent its one ending and stopped.
            // Two facts, and they fail for different reasons, so they are waited
            // on rather than sampled once. `run_turn` sends the ending as its
            // last statement and the channel goes quiet only when the closure
            // returns and drops the two senders — the worker's and the one
            // `wired` gave the agent — so the gap between the ending arriving
            // here and the disconnect arriving is a thread being scheduled. On a
            // loaded machine that is not bounded by any number worth writing
            // down, and a fixed window here read a slow teardown as a talkative
            // worker: `Timeout` says precisely that nothing was said.
            //
            // So an event is the failure, immediately and in its own words, and
            // silence is waited out to [`AT_MOST`] — the budget every other wait
            // in this module keeps.
            let quiet = Instant::now();
            loop {
                match received.recv_timeout(TEARDOWN_TICK) {
                    Err(RecvTimeoutError::Disconnected) => break,
                    Err(RecvTimeoutError::Timeout) => assert!(
                        quiet.elapsed() < AT_MOST,
                        "the worker held the channel open for {:?} after it ended",
                        quiet.elapsed()
                    ),
                    Ok(event) => panic!("the worker went on talking after it ended: {event:?}"),
                }
            }
        }

        #[test]
        fn dropping_a_turn_kills_the_claude_it_was_waiting_on() {
            let directory = scratch("dropped");
            let ticks = directory.join("ticks");
            // Never exits on its own, and says so in a file: whether it is
            // still running after the drop is a question this test can ask.
            let script = format!(
                "while :; do echo tick >> '{}'; sleep 0.05; done",
                ticks.display()
            );
            let chatting = start_turn(ASKED, &stand_in(&script), Asked::Answer);

            wait_for(&ticks);
            drop(chatting);

            // The guard is the only thing that can have stopped it: nothing
            // above joined a thread, cancelled a handle or waited for a child.
            thread::sleep(Duration::from_millis(300));
            let before = size(&ticks).expect("the child ticked at least once");
            thread::sleep(Duration::from_millis(300));
            assert_eq!(
                Some(before),
                size(&ticks),
                "the child outlived the turn nobody is listening to any more"
            );
            clean_up(&directory);
        }

        #[test]
        fn a_whole_turn_reaches_the_card_the_way_the_loop_drives_it() {
            let base = Instant::now();
            let mut app = App::default();
            app.panel_mut().start_turn(ASKED, base);
            let mut chat = Some(chatting(spawn_turn(
                ASKED,
                &stand_in(&printing(&TURN)),
                Cancel::new(),
            )));

            // The loop's own round, without the terminal: drain, and go round
            // again. Nothing here waits on the worker — `apply_turn` returns
            // whether or not anything has arrived — which is why this is a
            // poll rather than a receive.
            let waited = Instant::now();
            while chat.is_some() && waited.elapsed() < AT_MOST {
                drain(&mut chat, &mut app, at(base, 2));
                thread::sleep(Duration::from_millis(10));
            }

            assert!(chat.is_none(), "the turn never ended");
            assert_eq!(
                rows(&app, at(base, 2)),
                vec![
                    said(),
                    super::clocked(2, "Read src/lib.rs"),
                    super::clocked(2, "thinking"),
                    super::clocked(2, "writing"),
                    warlock_tui::Line::Text {
                        text: ANSWER.to_owned()
                    },
                ]
            );
            // A turn that worked says nothing on the footer.
            assert_eq!(app.message(), None);
        }

        #[test]
        fn stopping_a_turn_ends_it_as_cancelled_and_hands_the_field_back() {
            // Ctrl-C with a question out, end to end and through the value the
            // loop holds: `Chat::stop` is what that key comes to, and it kills
            // the `claude` the turn is waiting on rather than leaving warlock.
            // The stand-in never returns on its own and the agent's timeout is
            // the real five minutes, so the cancel is the only thing that can
            // end this within the test's patience.
            let base = Instant::now();
            let mut app = App::default();
            let mut chat = Chat::with_agent(NO_REPOSITORY, stand_in("sleep 300"));

            chat.ask(&mut app, ASKED, base);
            chat.stop();
            // Still in flight at the keystroke: the worker has one thing left to
            // say, and the drain is where it says it.
            assert!(chat.answering(), "the turn was taken down at the keystroke");
            settle(&mut chat, &mut app, at(base, 3));

            assert_eq!(
                rows(&app, at(base, 30)),
                vec![said(), clocked(3, &Ending::Cancelled.line())]
            );
            assert_eq!(app.message(), Some(Ending::Cancelled.line().as_str()));
            // And the field is live again, on the strength of the drain alone.
            assert!(!chat.answering());
        }

        #[test]
        fn a_synthesized_turn_shows_the_command_and_sends_the_instruction() {
            // The display/send split, end to end and through the value the loop
            // holds: what the reader sees is the word they typed and what the
            // child reads on its stdin is the paragraph warlock wrote. The
            // stand-in copies its stdin to a file before answering, so what was
            // really sent is a thing this test can read rather than infer.
            let directory = scratch("synthesized");
            let sent = directory.join("sent");
            let script = format!("cat > '{}'; {}", sent.display(), printing(&TURN));
            let base = Instant::now();
            let mut app = App::default();
            let mut chat = Chat::with_agent(NO_REPOSITORY, stand_in(&script));
            // Composed here, as the loop composes it: what is sent is a string
            // built around a shape, and this test only cares that whatever was
            // built is what the child read.
            let instruction = warlock_tui::brief_instruction(warlock_tui::DEFAULT_TEMPLATE);

            chat.say(&mut app, "/brief", &instruction, Asked::Answer, base);
            settle(&mut chat, &mut app, at(base, 1));

            // The command, its work lines and its answer: an ordinary turn in
            // every respect but the one word it is shown as.
            assert_eq!(
                rows(&app, at(base, 1)),
                vec![
                    warlock_tui::Line::Said {
                        text: "/brief".to_owned()
                    },
                    clocked(1, "Read src/lib.rs"),
                    clocked(1, "thinking"),
                    clocked(1, "writing"),
                    warlock_tui::Line::Text {
                        text: ANSWER.to_owned()
                    },
                ]
            );
            assert_eq!(
                fs::read_to_string(&sent).expect("the child read something"),
                instruction,
                "the model was not given the instruction",
            );
            clean_up(&directory);
        }

        #[test]
        fn a_failed_turn_leaves_the_conversation_usable_for_the_next_question() {
            // The promise a failure has to keep, driven through the very value
            // the event loop holds: a turn that went wrong is one line on the
            // thread and one on the footer, the turn is taken down — which is
            // what unmutes the field — and the question after it runs as if
            // nothing had happened. One `Chat`, as the loop has one, so the
            // second question is genuinely the next turn of the conversation the
            // first one failed in.
            const AGAIN: &str = "and which of those is the biggest?";

            let directory = scratch("usable");
            let asked_once = directory.join("asked-once");
            // Fails the first time it is asked and answers the second.
            let script = format!(
                "if [ -f '{marker}' ]; then {answer}; else : > '{marker}'; echo boom >&2; exit 3; fi",
                marker = asked_once.display(),
                answer = printing(&TURN),
            );
            let base = Instant::now();
            let mut app = App::default();
            let mut chat = Chat::with_agent(NO_REPOSITORY, stand_in(&script));

            chat.ask(&mut app, ASKED, base);
            assert!(
                chat.answering(),
                "the field is muted for as long as a question is out"
            );
            settle(&mut chat, &mut app, at(base, 1));

            // One line either side, in the same words, and no error anywhere:
            // `keep_up` returns nothing at all, so there is nothing for the loop
            // to have propagated.
            let line = app
                .message()
                .expect("a failed turn says so on the footer")
                .to_owned();
            assert!(line.contains("exit status 3"), "{line}");
            assert_eq!(rows(&app, at(base, 30)), vec![said(), clocked(1, &line)]);

            chat.ask(&mut app, AGAIN, at(base, 10));
            assert!(chat.answering(), "the next question started");
            settle(&mut chat, &mut app, at(base, 12));

            // The failed turn is still on the card above the answered one, and
            // the answered one is whole: the work as it arrived, and the
            // answer.
            assert_eq!(
                rows(&app, at(base, 12)),
                vec![
                    said(),
                    clocked(1, &line),
                    warlock_tui::Line::Said {
                        text: AGAIN.to_owned()
                    },
                    clocked(2, "Read src/lib.rs"),
                    clocked(2, "thinking"),
                    clocked(2, "writing"),
                    warlock_tui::Line::Text {
                        text: ANSWER.to_owned()
                    },
                ]
            );
            clean_up(&directory);
        }

        #[test]
        fn a_write_turn_that_answers_hands_the_document_to_the_loop() {
            // The whole of what `/write` adds to a turn, through the value the
            // loop holds and a `/bin/sh` standing in for the model: the answer
            // comes back to the caller — which is what the path prompt opens
            // over — and the same answer is on the card, where the reader reads
            // it and where the write takes its bytes from.
            let directory = scratch("write-answers");
            let base = Instant::now();
            let mut app = App::default();
            app.panel_mut().set_mode(Mode::Brief);
            let mut chat = Chat::with_agent(NO_REPOSITORY, stand_in(&printing(&TURN)));

            chat.say(
                &mut app,
                "/write",
                warlock_tui::WRITE_INSTRUCTION,
                Asked::Document,
                base,
            );
            let opened = settled(&mut chat, &mut app, at(base, 1));

            // The answer is handed to the window rather than to the loop: the
            // path on it is the one proposed from this very reply, so asserting
            // the field is asserting the document that came back.
            let field = opened.field().expect("the write window opened over it");
            assert_eq!(
                field.text(),
                crate::writing::proposed_path(
                    Path::new(NO_REPOSITORY),
                    warlock_engine::DEFAULT_BRIEF_DIRECTORY,
                    ANSWER
                )
            );
            assert_eq!(
                rows(&app, at(base, 1)),
                vec![
                    warlock_tui::Line::Said {
                        text: "/write".to_owned()
                    },
                    clocked(1, "Read src/lib.rs"),
                    clocked(1, "thinking"),
                    clocked(1, "writing"),
                    warlock_tui::Line::Text {
                        text: ANSWER.to_owned()
                    },
                ],
                "the document left the card it was answered on"
            );
            assert_eq!(
                app.panel().mode(),
                Mode::Brief,
                "the register moved for a write"
            );
            clean_up(&directory);
        }

        #[test]
        fn a_write_turn_that_fails_hands_nothing_back_and_leaves_the_mode_alone() {
            // The failure half of the same thing, and the promise it has to
            // keep: nothing comes back, so the loop has nothing to open a
            // prompt over and nothing to write; the ending is one line in the
            // existing wording; the conversation is still in brief mode and
            // still answers the next question.
            const AGAIN: &str = "what did that leave out?";

            let directory = scratch("write-fails");
            let asked_once = directory.join("asked-once");
            // Fails the first time it is asked and answers the second, exactly
            // as the ordinary failed turn above does.
            let script = format!(
                "if [ -f '{marker}' ]; then {answer}; else : > '{marker}'; echo boom >&2; exit 3; fi",
                marker = asked_once.display(),
                answer = printing(&TURN),
            );
            let base = Instant::now();
            let mut app = App::default();
            app.panel_mut().set_mode(Mode::Brief);
            let mut chat = Chat::with_agent(NO_REPOSITORY, stand_in(&script));

            chat.say(
                &mut app,
                "/write",
                warlock_tui::WRITE_INSTRUCTION,
                Asked::Document,
                base,
            );
            assert_eq!(
                settled(&mut chat, &mut app, at(base, 1)),
                ScopePrompt::Closed,
                "a failed write opened the window anyway"
            );

            let line = app
                .message()
                .expect("a failed turn says so on the footer")
                .to_owned();
            assert!(line.contains("exit status 3"), "{line}");
            assert_eq!(
                rows(&app, at(base, 30)),
                vec![
                    warlock_tui::Line::Said {
                        text: "/write".to_owned()
                    },
                    clocked(1, &line),
                ]
            );
            assert_eq!(
                app.panel().mode(),
                Mode::Brief,
                "a failed write left the register"
            );

            // And the conversation goes on: the next question is asked into the
            // very `Chat` the write failed in.
            chat.ask(&mut app, AGAIN, at(base, 10));
            settle(&mut chat, &mut app, at(base, 12));

            assert_eq!(
                rows(&app, at(base, 12)).last(),
                Some(&warlock_tui::Line::Text {
                    text: ANSWER.to_owned()
                })
            );
            clean_up(&directory);
        }
    }

    mod submitting {
        use std::fs;
        use std::path::{Path, PathBuf};
        use std::thread;
        use std::time::{Duration, Instant};

        use warlock_engine::{DEFAULT_BRIEF_DIRECTORY, briefs_path, load_briefs};
        use warlock_tui::{
            Activity, App, ChatAgent, Composed, Composer, DEFAULT_TEMPLATE, Ending, Line, Mode,
            Submitted, brief_instruction,
        };

        use warlock_tui::Converses;

        use super::super::{
            ALREADY_CHATTING, BRIEF_COMMAND, BRIEF_NOTE, CHAT_COMMAND, CHAT_NOTE, Chat,
            NOT_BRIEFING, WRITE_COMMAND, brief_asking,
        };
        use crate::error::one_line;
        use crate::writing::write_opened;

        const NOT_A_PROGRAM: &str = "/warlock/no/such/program";

        const NO_REPOSITORY: &str = "/warlock/no/such/repository";

        fn conversation() -> Chat<ChatAgent> {
            conversation_in(Path::new(NO_REPOSITORY))
        }

        fn conversation_in(root: &Path) -> Chat<ChatAgent> {
            Chat::with_agent(root, ChatAgent::new().with_program(NOT_A_PROGRAM))
        }

        fn a_root() -> tempfile::TempDir {
            tempfile::tempdir().expect("a temporary directory")
        }

        fn write_template(root: &Path, text: &str) -> PathBuf {
            let directory = root.join(".warlock");
            fs::create_dir_all(&directory).expect("a `.warlock` directory");
            let path = directory.join("brief-template.md");
            fs::write(&path, text).expect("a template file");

            path
        }

        fn write_briefs(root: &Path, text: &str) -> PathBuf {
            let path = briefs_path(root);
            fs::create_dir_all(path.parent().expect("the file has a directory"))
                .expect("a `.warlock` directory");
            fs::write(&path, text).expect("a briefs config");

            path
        }

        fn submit(draft: &str, now: Instant) -> (App, Chat<ChatAgent>) {
            let mut app = App::default();
            let mut chat = conversation();
            submit_into(&mut app, &mut chat, draft, now);

            (app, chat)
        }

        fn submit_into<C: Converses>(app: &mut App, chat: &mut Chat<C>, draft: &str, now: Instant) {
            chat.compose(app, Composed::Typing(Composer::new(draft)), now);
            chat.compose(app, Composed::Submit, now);
        }

        fn note(text: &str) -> Line {
            Line::Note {
                text: text.to_owned(),
            }
        }

        fn asked(shown: &str) -> [Line; 2] {
            [
                Line::Said {
                    text: shown.to_owned(),
                },
                Line::Clocked {
                    clock: "0:00".to_owned(),
                    text: "waiting".to_owned(),
                },
            ]
        }

        fn rows(app: &App, now: Instant) -> Vec<Line> {
            app.panel()
                .thread()
                .map(|thread| thread.lines(now))
                .unwrap_or_default()
        }

        fn turns(app: &App) -> usize {
            app.panel()
                .thread()
                .map_or(0, |thread| thread.turns().len())
        }

        fn proposal(root: &Path, directory: &str) -> String {
            let prompt = write_opened(root, directory, "# A brief\n\nsomething was said.");

            prompt
                .field()
                .expect("the write window opened")
                .text()
                .to_owned()
        }

        #[test]
        fn the_field_is_muted_by_a_turn_and_comes_back_however_it_ends() {
            let now = Instant::now();
            let mut app = App::default();
            let mut chat = conversation();

            assert!(
                !chat.composer().is_muted(),
                "an idle conversation cannot type"
            );

            submit_into(&mut app, &mut chat, "what is a pact?", now);
            assert!(chat.answering(), "the message started no turn");
            assert!(
                chat.composer().is_muted(),
                "a question is out and the field still types"
            );

            // The rounds until the drain takes the turn down. There is no
            // `claude` at that path, so the worker comes back with the ending
            // for it — one of the five, and nothing about that ending says a
            // word about the keyboard.
            let waited = Instant::now();
            while chat.answering() && waited.elapsed() < Duration::from_secs(5) {
                chat.keep_up(&mut app, now);
                thread::sleep(Duration::from_millis(10));
            }
            assert!(!chat.answering(), "the turn never ended");

            assert!(
                !chat.composer().is_muted(),
                "the turn ended and the field is still deaf"
            );
        }

        #[test]
        fn write_outside_brief_mode_is_one_note_and_costs_no_turn() {
            // Nothing is being converged on, so there is nothing to ask for: the
            // command says which register it wants and how to get there, and
            // spends neither a turn nor a `claude`. The line is decided from
            // `App::mode`, which is the state the border title is drawn from, so
            // it cannot say one register while the header says the other.
            let now = Instant::now();

            for draft in ["/write", "  /write  "] {
                let (app, chat) = submit(draft, now);

                assert_eq!(
                    app.panel().mode(),
                    Mode::Chat,
                    "{draft:?} moved the register"
                );
                assert_eq!(
                    rows(&app, now),
                    vec![note(NOT_BRIEFING)],
                    "{draft:?} did not leave exactly one note"
                );
                assert_eq!(turns(&app), 0, "{draft:?} opened a turn");
                assert!(!chat.answering(), "{draft:?} started something");
                assert!(
                    chat.composer().draft().is_empty(),
                    "{draft:?} was left in the field"
                );
            }

            // And after a mode that was entered and left again: the refusal is
            // about the register the conversation is in now, not about whether
            // it was ever in the other one.
            let mut app = App::default();
            let mut chat = conversation();
            submit_into(&mut app, &mut chat, "/brief", now);
            submit_into(&mut app, &mut chat, "/chat", now);
            let before = turns(&app);
            submit_into(&mut app, &mut chat, "/write", now);

            assert_eq!(turns(&app), before, "/write out of the mode cost a turn");
            assert_eq!(rows(&app, now).last(), Some(&note(NOT_BRIEFING)));
        }

        #[test]
        fn write_in_brief_mode_sends_one_turn_shown_as_the_command() {
            // The ask for the artifact, and it is an ordinary turn in every
            // respect but the one word it is shown as: the card carries `/write`
            // and never the paragraph that went to the model, and no note is
            // added because no register changed. `chatting.rs` asserts the
            // instruction really is what reaches the child's stdin.
            let now = Instant::now();
            let mut app = App::default();
            let mut chat = conversation();

            submit_into(&mut app, &mut chat, "/brief", now);
            submit_into(&mut app, &mut chat, "  /write  ", now);

            assert_eq!(app.panel().mode(), Mode::Brief, "/write moved the register");
            assert_eq!(
                rows(&app, now),
                [
                    vec![note(BRIEF_NOTE)],
                    asked(BRIEF_COMMAND).to_vec(),
                    asked(WRITE_COMMAND).to_vec(),
                ]
                .concat(),
                "/write is not one turn shown as the command",
            );
            assert_eq!(turns(&app), 2, "/write did not open one turn");
            assert!(chat.answering(), "/write asked the model nothing");
            assert!(
                chat.composer().draft().is_empty(),
                "/write was left in the field"
            );

            // And again, because asking twice is asking twice: a second document
            // costs a second turn and still says nothing about the mode.
            submit_into(&mut app, &mut chat, "/write", now);

            assert_eq!(turns(&app), 3, "the second /write cost no turn");
            assert_eq!(
                rows(&app, now).len(),
                1 + 2 + 2 + 2,
                "the second /write said something about the register",
            );
        }

        #[test]
        fn brief_notes_the_mode_once_and_sends_one_turn_shown_as_the_command() {
            // What `/brief` costs: one unclocked note where it was typed, and
            // one ordinary turn under it. The card shows the word that was
            // typed and never the paragraph that went to the model — a screen of
            // prose the reader did not write, in the place their own questions
            // go, would be warlock putting words in their mouth.
            let now = Instant::now();

            for draft in ["/brief", "  /brief  "] {
                let (app, chat) = submit(draft, now);

                assert_eq!(
                    app.panel().mode(),
                    Mode::Brief,
                    "{draft:?} did not enter the mode"
                );
                assert_eq!(
                    rows(&app, now),
                    [vec![note(BRIEF_NOTE)], asked(BRIEF_COMMAND).to_vec()].concat(),
                    "{draft:?} is not one note and one turn"
                );
                assert_eq!(turns(&app), 1, "{draft:?} did not open one turn");
                assert!(chat.answering(), "{draft:?} asked the model nothing");
                assert!(
                    chat.composer().draft().is_empty(),
                    "{draft:?} was left in the field"
                );
            }
        }

        #[test]
        fn a_brief_in_a_repository_with_no_template_states_the_built_in_shape() {
            // The ordinary case, and the one nobody configures anything for: a
            // repository that has written no `.warlock/brief-template.md` is a
            // repository that has said nothing about the shape, so the command
            // behaves exactly as it did before there was a file to write —
            // mode, note, one turn — and what it sends is warlock's own
            // skeleton.
            let now = Instant::now();
            let repo = a_root();
            let mut app = App::default();
            let mut chat = conversation_in(repo.path());

            submit_into(&mut app, &mut chat, "/brief", now);

            assert_eq!(app.panel().mode(), Mode::Brief, "the mode was not entered");
            assert_eq!(
                rows(&app, now),
                [vec![note(BRIEF_NOTE)], asked(BRIEF_COMMAND).to_vec()].concat(),
            );
            assert_eq!(turns(&app), 1, "the brief did not open one turn");
            assert!(chat.answering(), "the brief asked the model nothing");
            assert!(chat.composer().draft().is_empty());
            // And the instruction it sent, composed through the very function
            // the arm above composes it with: the built-in shape, because there
            // was nothing else to state.
            assert_eq!(
                brief_asking(repo.path()).expect("a repository with no template"),
                brief_instruction(DEFAULT_TEMPLATE),
            );
        }

        #[test]
        fn a_brief_carries_the_shape_the_repository_wrote_rather_than_the_built_in_one() {
            // A repository that has stated its own shape gets its own: the file
            // is read at this keystroke, and the built-in skeleton is nowhere in
            // what goes out. The card and the register are unchanged by any of
            // that — a template is what the instruction says, not what the
            // command does.
            const SHAPE: &str = "## The only heading we want\n\nOne section, and no others.";

            let now = Instant::now();
            let repo = a_root();
            write_template(repo.path(), SHAPE);
            let mut app = App::default();
            let mut chat = conversation_in(repo.path());

            submit_into(&mut app, &mut chat, "/brief", now);

            assert_eq!(app.panel().mode(), Mode::Brief, "the mode was not entered");
            assert_eq!(turns(&app), 1, "the brief did not open one turn");
            assert!(chat.answering());

            let instruction = brief_asking(repo.path()).expect("a template that reads");
            assert!(
                instruction.contains(SHAPE),
                "the template is not in the instruction: {instruction}"
            );
            assert!(
                !instruction.contains("## Success criteria"),
                "the built-in shape was sent as well: {instruction}"
            );
        }

        #[test]
        fn a_template_that_cannot_be_read_refuses_the_brief_and_changes_nothing_else() {
            // Refused rather than degraded: a file somebody wrote is a shape
            // somebody meant, and quietly aiming twenty turns at warlock's own
            // instead would be the wrong document arrived at slowly. So the
            // command is one line naming the file and what the filesystem said
            // about it, and the session is otherwise exactly as it was — no
            // mode, no turn, no `claude`, nothing on the footer.
            //
            // Bytes that are not UTF-8 are the portable way to have a file that
            // exists and cannot be had; `template.rs` fails the same way for the
            // same reason.
            let now = Instant::now();
            let repo = a_root();
            let path = write_template(repo.path(), "");
            fs::write(&path, [0x23, 0x20, 0xff, 0xfe, 0x0a]).expect("a template file");
            let reason = warlock_tui::brief_template(repo.path())
                .expect_err("a template that cannot be read")
                .to_string();
            let mut app = App::default();
            let mut chat = conversation_in(repo.path());

            submit_into(&mut app, &mut chat, "/brief", now);

            assert_eq!(app.panel().mode(), Mode::Chat, "a refusal entered the mode");
            assert_eq!(turns(&app), 0, "a refusal spent a turn");
            assert!(!chat.answering(), "a refusal asked the model something");
            assert!(chat.composer().draft().is_empty());
            assert_eq!(
                app.message(),
                None,
                "a refusal said something on the footer"
            );

            let said = rows(&app, now);
            assert_eq!(said.len(), 1, "a refusal is one line: {said:?}");
            let Some(Line::Note { text }) = said.first() else {
                panic!("a refusal is a note of warlock's own: {said:?}");
            };
            assert!(
                text.contains(&reason),
                "the loader's own words are not in it: {text}"
            );
            assert!(
                text.contains(&path.display().to_string()),
                "the file is not named: {text}"
            );
            assert!(!text.contains('\n'), "the refusal wrapped: {text}");
            assert!(
                !text.contains("## Success criteria"),
                "the built-in shape leaked into the refusal: {text}"
            );

            // And the next `/brief`, once the file reads again, is an ordinary
            // one: the refusal left nothing behind to recover from.
            fs::write(&path, "## Ours\n\nsay the thing.").expect("a template file");
            submit_into(&mut app, &mut chat, "/brief", now);

            assert_eq!(
                app.panel().mode(),
                Mode::Brief,
                "the mode was still not entered"
            );
            assert_eq!(turns(&app), 1, "the second brief opened no turn");
        }

        #[test]
        fn a_repository_that_says_nothing_briefs_into_the_default_directory() {
            // Where a `/write` in this mode would put the document is a value
            // this command settles and the loop then holds: `/write` reads
            // nothing, so a brief that has taken twenty turns cannot arrive at a
            // window that will not open. A repository with no
            // `.warlock/briefs.toml` has stated no preference, which is the
            // engine's default and nothing said on the card — and it is written
            // here rather than left wherever a previous mode pointed, which is
            // `a_second_brief_re_reads_the_file_and_takes_the_new_directory`'s
            // half of it.
            let now = Instant::now();
            let repo = a_root();
            let mut app = App::default();
            let mut chat = conversation_in(repo.path());

            submit_into(&mut app, &mut chat, "/brief", now);

            assert_eq!(app.panel().mode(), Mode::Brief, "the mode was not entered");
            assert_eq!(turns(&app), 1, "the brief did not open one turn");
            assert_eq!(rows(&app, now).len(), 1 + 2, "something was said about it");
            assert_eq!(chat.directory(), DEFAULT_BRIEF_DIRECTORY);
            // And that is a proposal under `docs/`, through the very function
            // the loop opens the write window with.
            let proposed = proposal(repo.path(), chat.directory());
            assert!(
                proposed.starts_with("docs/"),
                "the default is not where a write would go: {proposed}",
            );
        }

        #[test]
        fn the_repositorys_own_directory_is_what_brief_settles_on() {
            // One key in one hand-written file, read at this keystroke: a
            // repository that keeps its briefs in `plans/` gets `plans/`, and
            // nothing about the command changes — the mode is entered, the note
            // is the note, and the turn is the turn.
            let now = Instant::now();
            let repo = a_root();
            write_briefs(repo.path(), "directory = \"plans\"\n");
            let mut app = App::default();
            let mut chat = conversation_in(repo.path());

            submit_into(&mut app, &mut chat, "/brief", now);

            assert_eq!(app.panel().mode(), Mode::Brief, "the mode was not entered");
            assert_eq!(
                rows(&app, now),
                [vec![note(BRIEF_NOTE)], asked(BRIEF_COMMAND).to_vec()].concat(),
                "a setting that reads said something on the card",
            );
            assert_eq!(chat.directory(), "plans");
            let proposed = proposal(repo.path(), chat.directory());
            assert!(
                proposed.starts_with("plans/"),
                "the setting is not where a write would go: {proposed}",
            );
        }

        #[test]
        fn a_briefs_config_that_cannot_be_read_refuses_the_brief_and_changes_nothing_else() {
            // The template's refusal, over the other file and for the same
            // reason: a `directory` somebody wrote down is a place somebody
            // meant, so warlock will not quietly aim twenty turns at `docs/`
            // instead. One line naming the file and quoting the parser, and the
            // session otherwise exactly as it was — no mode, no turn, no
            // `claude`, nothing on the footer, and the directory the loop was
            // carrying untouched.
            let now = Instant::now();
            let repo = a_root();
            let path = write_briefs(repo.path(), "directory = [\n");
            // The loader's own sentence, flattened the way the note flattens it:
            // a `briefs.toml` that will not parse carries the TOML parser's
            // multi-line diagnostic, and the card is one line a note.
            let reason = one_line(
                &load_briefs(repo.path())
                    .expect_err("a config that is not TOML")
                    .to_string(),
            );
            let mut app = App::default();
            let mut chat = conversation_in(repo.path());

            submit_into(&mut app, &mut chat, "/brief", now);

            assert_eq!(app.panel().mode(), Mode::Chat, "a refusal entered the mode");
            assert_eq!(turns(&app), 0, "a refusal spent a turn");
            assert!(!chat.answering(), "a refusal asked the model something");
            assert!(chat.composer().draft().is_empty());
            assert_eq!(
                app.message(),
                None,
                "a refusal said something on the footer"
            );

            let said = rows(&app, now);
            assert_eq!(said.len(), 1, "a refusal is one line: {said:?}");
            let Some(Line::Note { text }) = said.first() else {
                panic!("a refusal is a note of warlock's own: {said:?}");
            };
            assert!(
                text.contains(&path.display().to_string()),
                "the file is not named: {text}"
            );
            assert!(
                text.contains(&reason),
                "the parser's own words are not in it: {text}"
            );
            assert!(!text.contains('\n'), "the refusal wrapped: {text}");

            // And the next `/brief`, once the file reads again, is an ordinary
            // one: the refusal left nothing behind to recover from, and the
            // setting that now parses is the setting the mode is entered with.
            fs::write(&path, "directory = \"plans\"\n").expect("a briefs config");
            submit_into(&mut app, &mut chat, "/brief", now);

            assert_eq!(
                app.panel().mode(),
                Mode::Brief,
                "the mode was still not entered"
            );
            assert_eq!(turns(&app), 1, "the second brief opened no turn");
            assert_eq!(chat.directory(), "plans");
        }

        #[test]
        fn two_broken_files_are_still_one_line_and_no_turn() {
            // The decision when both files under `.warlock/` are broken: the
            // load stops at the first, so the reader gets one refusal rather
            // than two, and the second file's line is what the next `/brief`
            // says once this one is fixed. Two notes for one keystroke would be
            // warlock reporting its own reading order.
            let now = Instant::now();
            let repo = a_root();
            let template = write_template(repo.path(), "");
            fs::write(&template, [0x23, 0x20, 0xff, 0xfe, 0x0a]).expect("a template file");
            write_briefs(repo.path(), "directroy = \"plans\"\n");
            let mut app = App::default();
            let mut chat = conversation_in(repo.path());

            submit_into(&mut app, &mut chat, "/brief", now);

            assert_eq!(app.panel().mode(), Mode::Chat, "a refusal entered the mode");
            assert_eq!(turns(&app), 0, "a refusal spent a turn");
            let said = rows(&app, now);
            assert_eq!(said.len(), 1, "two files, two lines: {said:?}");
            let Some(Line::Note { text }) = said.first() else {
                panic!("a refusal is a note of warlock's own: {said:?}");
            };
            assert!(
                text.contains(&template.display().to_string()),
                "the first file read is not the one named: {text}"
            );

            // The template fixed, the misspelled key is what the next one says
            // — and it is still one line and still no turn.
            fs::write(&template, "## Ours\n\nsay the thing.").expect("a template file");
            submit_into(&mut app, &mut chat, "/brief", now);

            assert_eq!(
                app.panel().mode(),
                Mode::Chat,
                "the second refusal entered the mode"
            );
            assert_eq!(turns(&app), 0, "the second refusal spent a turn");
            let said = rows(&app, now);
            assert_eq!(
                said.len(),
                2,
                "the second refusal is one more line: {said:?}"
            );
            let Some(Line::Note { text }) = said.last() else {
                panic!("a refusal is a note of warlock's own: {said:?}");
            };
            assert!(
                text.contains(&briefs_path(repo.path()).display().to_string()),
                "the second file is not named: {text}"
            );
            assert!(
                text.contains("directroy"),
                "the offending key is not named: {text}"
            );
            assert_eq!(
                chat.directory(),
                DEFAULT_BRIEF_DIRECTORY,
                "a refusal moved where a brief would go",
            );
        }

        #[test]
        fn a_refusal_leaves_where_a_brief_would_go_where_the_last_good_one_put_it() {
            // The other half of "the directory is written at every `/brief`":
            // it is written at every `/brief` that *happens*, and a refusal is a
            // command that does not happen at all. So a session that has briefed
            // into `plans/` and then meets a `briefs.toml` somebody has broken
            // goes on pointing at `plans/` — it does not fall back to the
            // default, and it does not point at nothing.
            //
            // This wants a conversation that has already briefed, which is why
            // it is its own test rather than a line in the refusal's: that one
            // is about a first `/brief` refusing, and asserts a mode never
            // entered and a turn never spent.
            let now = Instant::now();
            let repo = a_root();
            write_briefs(repo.path(), "directory = \"plans\"\n");
            let mut app = App::default();
            let mut chat = conversation_in(repo.path());

            submit_into(&mut app, &mut chat, "/brief", now);
            assert_eq!(chat.directory(), "plans", "the good brief did not settle");

            // And then the file somebody broke between the two commands.
            write_briefs(repo.path(), "directory = [\n");
            submit_into(&mut app, &mut chat, "/brief", now);

            assert_eq!(
                chat.directory(),
                "plans",
                "a refusal moved where a brief would go"
            );
            assert_eq!(turns(&app), 1, "a refusal spent a turn");
        }

        #[test]
        fn a_second_brief_re_reads_the_file_and_takes_the_new_directory() {
            // The file is read at every `/brief` and never held, so editing it
            // with `e` and typing the command again is the whole of changing
            // where a brief lands — no restart, and nothing on the app or the
            // conversation remembering the old answer.
            let now = Instant::now();
            let repo = a_root();
            write_briefs(repo.path(), "directory = \"plans\"\n");
            let mut app = App::default();
            let mut chat = conversation_in(repo.path());

            submit_into(&mut app, &mut chat, "/brief", now);
            assert_eq!(chat.directory(), "plans");

            write_briefs(repo.path(), "directory = \"notes/adr\"\n");
            submit_into(&mut app, &mut chat, "/brief", now);

            assert_eq!(
                chat.directory(),
                "notes/adr",
                "the second /brief re-read nothing"
            );
            assert_eq!(app.panel().mode(), Mode::Brief);
            assert_eq!(turns(&app), 2, "the second /brief cost no turn");
        }

        #[test]
        fn brief_in_brief_mode_re_sends_the_instruction_and_notes_nothing() {
            // Typing it again is the remedy for a register that has drifted, so
            // it costs a turn every time — and says nothing new about the mode,
            // because the mode did not change.
            let now = Instant::now();
            let mut app = App::default();
            let mut chat = conversation();

            submit_into(&mut app, &mut chat, "/brief", now);
            submit_into(&mut app, &mut chat, "/brief", now);

            assert_eq!(app.panel().mode(), Mode::Brief);
            assert_eq!(
                rows(&app, now),
                [
                    vec![note(BRIEF_NOTE)],
                    asked(BRIEF_COMMAND).to_vec(),
                    asked(BRIEF_COMMAND).to_vec(),
                ]
                .concat(),
            );
            assert_eq!(turns(&app), 2, "the second /brief cost no turn");
        }

        #[test]
        fn chat_leaves_the_mode_with_one_note_and_one_turn() {
            // The way out, and the same shape as the way in: the register is
            // left, warlock says so once, and the model is told the other
            // instruction as one ordinary turn shown as `/chat`.
            let now = Instant::now();
            let mut app = App::default();
            let mut chat = conversation();

            submit_into(&mut app, &mut chat, "/brief", now);
            submit_into(&mut app, &mut chat, "/chat", now);

            assert_eq!(
                app.panel().mode(),
                Mode::Chat,
                "/chat did not leave the mode"
            );
            assert_eq!(
                rows(&app, now),
                [
                    vec![note(BRIEF_NOTE)],
                    asked(BRIEF_COMMAND).to_vec(),
                    vec![note(CHAT_NOTE)],
                    asked(CHAT_COMMAND).to_vec(),
                ]
                .concat(),
            );
            assert_eq!(turns(&app), 2);
            assert!(chat.answering(), "the instruction was never sent");
        }

        #[test]
        fn chat_in_chat_mode_is_one_line_and_costs_no_turn() {
            // There is nothing to leave, so there is nothing to tell the model:
            // a turn spent saying the conversation is where it already was is a
            // question nobody asked and money nobody meant to spend.
            let now = Instant::now();
            let (app, chat) = submit("/chat", now);

            assert_eq!(app.panel().mode(), Mode::Chat);
            assert_eq!(rows(&app, now), vec![note(ALREADY_CHATTING)]);
            assert_eq!(turns(&app), 0, "/chat in chat mode opened a turn");
            assert!(!chat.answering(), "/chat in chat mode asked the model");
            assert!(chat.composer().draft().is_empty());

            // And the same after a mode that really was left: the refusal is
            // about the state, not about how the conversation got into it.
            let mut app = App::default();
            let mut chat = conversation();
            submit_into(&mut app, &mut chat, "/brief", now);
            submit_into(&mut app, &mut chat, "/chat", now);
            let before = turns(&app);
            submit_into(&mut app, &mut chat, "/chat", now);

            assert_eq!(turns(&app), before, "the second /chat cost a turn");
            assert_eq!(
                rows(&app, now).last(),
                Some(&note(ALREADY_CHATTING)),
                "the second /chat said something else",
            );
        }

        #[test]
        fn a_mode_clears_hides_and_reorders_nothing_that_was_already_said() {
            // The property the whole design rests on: the turns already on the
            // card are the material a document is made of. Entering the mode and
            // leaving it again puts rows *under* them and moves none of them.
            let now = Instant::now();
            let mut app = App::default();
            let mut chat = conversation();

            submit_into(&mut app, &mut chat, "why nine passes?", now);
            let before = rows(&app, now);
            submit_into(&mut app, &mut chat, "/brief", now);
            submit_into(&mut app, &mut chat, "/chat", now);

            let after = rows(&app, now);
            assert_eq!(after[..before.len()], before[..], "the card was rewritten");
            assert_eq!(
                after,
                [
                    before,
                    vec![note(BRIEF_NOTE)],
                    asked(BRIEF_COMMAND).to_vec(),
                    vec![note(CHAT_NOTE)],
                    asked(CHAT_COMMAND).to_vec(),
                ]
                .concat(),
            );
            assert_eq!(turns(&app), 3);
        }

        #[test]
        fn a_mode_leaves_every_answer_and_every_work_line_exactly_where_it_was() {
            // The same property with the card full rather than empty, which is
            // the state a `/brief` is actually typed in: the conversation worth
            // converging on is one that has been going for a while, and by then
            // the turns on the card carry the answers and the work lines that
            // are the material a document is made of. Losing those to a mode
            // change would be losing the brief before it started — and it is
            // the failure a second session would show up as, because a session
            // that starts again starts with nothing on the card.
            //
            // The rows come first, because that is what the reader has, and
            // then the turns themselves, because a row that merely *drew* the
            // same is not the same answer.
            let now = Instant::now();
            let later = now + Duration::from_secs(30);
            let mut app = App::default();
            let mut chat = conversation();

            // One turn that was worked at and answered, and one that ended
            // without an answer: both are things a mode change could drop.
            submit_into(&mut app, &mut chat, "why nine passes?", now);
            app.panel_mut().record_turn(
                &Activity::Tool {
                    name: "Read".to_owned(),
                    detail: Some("crates/warlock-engine/src/lib.rs".to_owned()),
                },
                now,
            );
            app.panel_mut().record_turn(&Activity::Thinking, now);
            app.panel_mut()
                .answer_turn("One pass per directory, bottom up.", now);
            submit_into(&mut app, &mut chat, "and the manifest?", now);
            app.panel_mut().end_turn(&Ending::NothingSaid, now);

            let before = rows(&app, later);
            let asked_already: Vec<_> = app
                .panel()
                .thread()
                .expect("two questions were asked")
                .turns()
                .into_iter()
                .cloned()
                .collect();
            // The history is really a history: an answer, work lines and an
            // ending are all on the card before the mode is touched, so the
            // equalities below are about something rather than about nothing.
            assert!(
                before.iter().any(|line| matches!(line, Line::Text { .. })),
                "there is no answer on the card to survive anything: {before:?}"
            );
            assert!(
                before
                    .iter()
                    .filter(|line| matches!(line, Line::Clocked { .. }))
                    .count()
                    >= 3,
                "there is no work on the card to survive anything: {before:?}"
            );

            submit_into(&mut app, &mut chat, "/brief", later);
            submit_into(&mut app, &mut chat, "/chat", later);

            // Every row that was there is still there, at the index it was at:
            // nothing cleared, nothing hidden, nothing reordered, and the two
            // answers and every work line word for word.
            let after = rows(&app, later);
            assert_eq!(
                after[..before.len()],
                before[..],
                "entering and leaving the register rewrote the conversation"
            );
            // And the turns under those rows: the message, the answer and the
            // ending of each, unchanged and in the order they were asked in.
            let asked_now: Vec<_> = app
                .panel()
                .thread()
                .expect("the conversation is still there")
                .turns()
                .into_iter()
                .cloned()
                .collect();
            assert_eq!(
                asked_now[..asked_already.len()],
                asked_already[..],
                "a mode change took a turn of the conversation"
            );
            assert_eq!(
                asked_now.len(),
                asked_already.len() + 2,
                "the two commands did not cost the two turns they are supposed to"
            );
            assert_eq!(
                app.panel().mode(),
                Mode::Chat,
                "the register was never left"
            );
        }

        #[test]
        fn a_refusal_is_exactly_one_note_and_never_a_turn() {
            // The whole of what a missed command costs: one line on the card,
            // in warlock's own voice, and not a question anybody paid for.
            let now = Instant::now();
            let refusal = Submitted::Refused
                .refusal()
                .expect("a refused draft has a line");

            for draft in ["/breif", "/plan", "/BRIEF", "/", "/brief now", "/brief\nx"] {
                let (app, chat) = submit(draft, now);

                assert_eq!(
                    rows(&app, now),
                    vec![Line::Note {
                        text: refusal.to_owned()
                    }],
                    "{draft:?} did not leave exactly one note"
                );
                assert_eq!(turns(&app), 0, "{draft:?} opened a turn");
                assert!(!chat.answering(), "{draft:?} was asked of the model");
                assert!(
                    chat.composer().draft().is_empty(),
                    "{draft:?} was left in the field"
                );
            }
        }

        #[test]
        fn a_message_submits_as_it_always_did() {
            // The behaviour the classifier must not have changed: the words go
            // on the card as the reader's own, one turn is opened, the question
            // is out, and the field is empty behind it. A path is here too,
            // because `/home/cole/notes` is a message and the reader who typed
            // it is talking about a file.
            let now = Instant::now();

            for draft in [
                "why nine passes?",
                "/home/cole/notes",
                "tell me about /brief",
            ] {
                let (app, chat) = submit(draft, now);

                assert_eq!(turns(&app), 1, "{draft:?} did not open one turn");
                // The question as it was typed, and under it the clocked row a
                // turn with nothing back yet draws — a live turn, which is
                // exactly what a command and a refusal never leave.
                assert_eq!(
                    rows(&app, now),
                    vec![
                        Line::Said {
                            text: draft.to_owned()
                        },
                        Line::Clocked {
                            clock: "0:00".to_owned(),
                            text: "waiting".to_owned()
                        }
                    ],
                    "{draft:?} is not on the card as it was typed"
                );
                assert!(chat.answering(), "{draft:?} was never asked");
                assert!(
                    chat.composer().draft().is_empty(),
                    "{draft:?} was left in the field"
                );
            }
        }

        #[test]
        fn the_field_a_submit_leaves_behind_is_empty_with_its_cursor_at_zero() {
            // The field is replaced outright rather than emptied in place, and
            // this is the half of that which is not the draft: an insertion
            // point left where it was in the message that has just gone would
            // be an offset into a string that no longer exists, and the next
            // character typed would go in at it. Submitted from the middle of
            // the draft, because that is the only cursor a submit can leave
            // behind that zero is not already true of.
            let now = Instant::now();
            let mut app = App::default();
            let mut chat = conversation();

            chat.compose(
                &mut app,
                Composed::Typing(Composer::new("why nine passes?").at(4)),
                now,
            );
            assert_eq!(chat.composer().cursor(), 4, "the field took no cursor");

            chat.compose(&mut app, Composed::Submit, now);

            assert!(chat.answering(), "the draft was never asked");
            assert!(
                chat.composer().draft().is_empty(),
                "the draft outlived the submit"
            );
            assert_eq!(
                chat.composer().cursor(),
                0,
                "the cursor outlived the draft it pointed into"
            );
        }
    }
}
