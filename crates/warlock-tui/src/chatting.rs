//! One chat turn on a worker thread, from the message somebody typed to the
//! answer that lands under it.
//!
//! [`pacting`](crate::pacting) over a much smaller job, and deliberately the
//! same four parts in the same order: [`spawn_turn`] starts the worker and hands
//! back the channel it reports on, [`start_turn`] is everything the event loop
//! has to keep about work it is not doing, [`run_turn`] is the worker's whole
//! body — a free function, so a test drives it on its own thread with no thread
//! at all — and [`apply_turn`] is the loop's other half, draining with
//! `try_recv` so a frame is never spent waiting on a model. A turn is one
//! `claude` and a pact is a hundred, but the shape of not blocking the screen on
//! either is the same shape, and one of them is enough.
//!
//! What is *not* the same is everything a turn ends in. A run writes documents,
//! saves a manifest and makes the tree on screen a load out of date; a turn
//! writes nothing, saves nothing and reloads nothing, so this module touches no
//! manifest, no [`Scope`](crate::session::Scope) and no [`Tree`](warlock_engine::Tree).
//! What it produces is rows: the work as it happens, and then either the answer
//! or the one line saying why there is none.
//!
//! That is the second difference, and it is the one worth being explicit about.
//! Nothing here comes back as an error. A `claude` that is not installed, a
//! child that exits non-zero, a turn that runs past its timeout, a model that
//! finishes with nothing to say and a reader who pressed Ctrl-C are five
//! different facts and one kind of consequence: exactly one [`Ending`] on the
//! thread and the same sentence on the footer, with the session as usable for
//! the next question as it was for this one. An event loop that could be ended
//! by a bad answer would be a chat that takes the tree down with it.
//!
//! Stopping a turn is the pact's bargain over again, and literally so — the
//! say-when is [`pacting`](crate::pacting)'s own [`CancelGuard`], reused rather
//! than written a second time. A turn cancelled on purpose ends in
//! [`Ending::Cancelled`]; a turn whose value is simply dropped, because warlock
//! is quitting or because the loop returned through a `?`, takes its `claude`
//! with it and reports to nobody, which is the whole reason the handle is a
//! guard rather than a flag somebody has to remember.

use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::thread;
use std::time::Instant;

use warlock_tui::{Activities, Activity, App, Cancel, ChatAgent, Ending, ending_for};

use crate::pacting::CancelGuard;

/// What the thread and the footer say when the worker stopped without reporting
/// anything — which, since it reports on every path it takes itself, means it
/// panicked.
///
/// [`PACT_LOST`](crate::pacting)'s opposite number, and worded as the tail of
/// [`Ending::Broke`]'s sentence rather than as a sentence of its own, so the
/// line reads as every other failed turn reads: `the turn could not run — …`.
/// Nothing was written and nothing was saved, so unlike a lost pact there is
/// nothing to put back — the conversation keeps every line that arrived before
/// the silence, and the next question runs as if this one had merely failed.
const TURN_LOST: &str = "it stopped without saying how it went";

/// The whole of the conversation the event loop keeps: who to ask, and the one
/// question currently out.
///
/// Two things, and the reason they are one value is that neither is any use
/// without the other. The agent is built once for the life of the process —
/// a [`ChatAgent`] carries the session id that makes the second question a reply
/// to the first, so one per turn would be a conversation that forgot itself
/// between two Enters — and the turn is `Some` for the seconds one is in flight.
/// Keeping them together is what lets the loop ask "is a question out?" in one
/// word ([`Chat::answering`]) at the two places that decide it: the field's
/// muting, and what Ctrl-C means.
///
/// It holds no draft and no thread. What has been typed is the loop's
/// [`Composer`](warlock_tui::Composer), which a run must not be able to reach,
/// and the conversation itself is on the app's thread card, where the panel can
/// draw it — this value is the *machinery*, and machinery that also held the
/// text would be a second copy of the conversation for somebody to keep in step.
pub(crate) struct Chat {
    /// Who is asked, and the session every turn belongs to. Never spoken to
    /// directly: each turn works off a copy of it wired to that turn's own
    /// cancel and activity port (see [`wired`]).
    agent: ChatAgent,
    /// The turn being answered, if one is. `None` is the ordinary state and the
    /// state a session starts and ends in.
    turn: Option<Chatting>,
}

impl Chat {
    /// A conversation with nothing asked yet.
    ///
    /// The agent is made here, once, and the session id it names is fixed from
    /// this moment: every turn of this warlock is one conversation, and a second
    /// warlock in another terminal is another.
    pub(crate) fn new() -> Self {
        Self::with_agent(ChatAgent::new())
    }

    /// A conversation with nothing asked yet, over `agent`.
    ///
    /// [`Chat::new`]'s body with the one thing that varies handed in, so that a
    /// test can drive the very value the loop keeps — the agent and the turn
    /// together — against a stand-in program. A conversation whose failures
    /// could only be asserted through the pieces underneath it would be a
    /// conversation nothing had ever run twice.
    fn with_agent(agent: ChatAgent) -> Self {
        Self { agent, turn: None }
    }

    /// Whether a question is out and being answered.
    ///
    /// The one fact the rest of the loop asks for, and it is asked twice a
    /// round: the composer is muted for exactly as long as this is true — one
    /// question at a time — and Ctrl-C stops the turn rather than the session
    /// for exactly as long as it is true. Both readings come from here so that
    /// the key and the field cannot disagree about whether anything is running.
    pub(crate) const fn answering(&self) -> bool {
        self.turn.is_some()
    }

    /// Ask `message`, at `now`: on the thread card, and on a worker thread.
    ///
    /// The two halves of a submitted draft, in the order the reader experiences
    /// them. The question goes on the card first, which is also what brings the
    /// thread to the front, so somebody who has just asked something is looking
    /// at it from the instant they asked rather than from whenever the model
    /// first says anything. Then the worker, which owns its own copy of the
    /// message and of the agent and is never joined (see [`start_turn`]).
    ///
    /// `now` is the caller's clock — the instant the key was pressed — because
    /// that is what the turn's work lines are clocked from: a turn is as old as
    /// the question that asked it, not as old as the first thing the model got
    /// round to saying.
    ///
    /// Nothing stops a caller asking twice, because nothing has to: the field is
    /// muted for the whole of a turn, so the Enter that would ask a second
    /// question never becomes a submit. If one ever did, the second turn would
    /// simply replace the first — and the first's [`CancelGuard`] would drop and
    /// take its `claude` with it, which is the safe way for that accident to go.
    pub(crate) fn ask(&mut self, app: &mut App, message: &str, now: Instant) {
        app.start_turn(message, now);
        self.turn = Some(start_turn(message, &self.agent));
    }

    /// Stop the turn in flight, if there is one.
    ///
    /// What Ctrl-C comes to while a question is out. The turn is deliberately
    /// not taken down here: the worker still has one thing to say — that it was
    /// cancelled — and it says it through [`Chat::keep_up`] like any other
    /// ending, which is what puts the cancelled line under the work that had
    /// already arrived and hands the keyboard back to the field.
    ///
    /// Nothing at all with no turn in flight, which is a state this is never
    /// called in: the key means "leave" then, and the loop reads that off
    /// [`Chat::answering`] before it ever gets here.
    pub(crate) fn stop(&self) {
        if let Some(chatting) = self.turn.as_ref() {
            chatting.cancel.cancel();
        }
    }

    /// Apply everything the turn has said since the last frame.
    ///
    /// [`apply_turn`] against the turn this value holds, so the loop's bottom
    /// end is one call rather than a field reached into. It never blocks and it
    /// is what ends a turn: the moment it takes the turn down, the top of the
    /// next round finds [`Chat::answering`] false and gives the field back —
    /// which is how "the composer is live again the moment the turn ends,
    /// however it ends" is one rule rather than five.
    pub(crate) fn keep_up(&mut self, app: &mut App, now: Instant) {
        apply_turn(&mut self.turn, app, now);
    }
}

/// A turn being answered by a worker thread, from the point of view of the
/// thread drawing the screen.
///
/// Two things, and no handle to join: what the worker has to say, and how to
/// tell it to stop. [`Running`](crate::pacting) keeps two more — what the run
/// was asked to do, and the app as it stood before the keystroke painted
/// anything — and a turn needs neither. What it was asked is already on the
/// thread card as the reader's own message, put there by the keystroke that
/// started this; and there is nothing to put back, because a turn changes no
/// row, no colour and no file.
///
/// The say-when is a [`CancelGuard`] rather than a bare [`Cancel`] for exactly
/// the reason a run's is: every way out of the event loop drops this value, and
/// each of them would otherwise have to remember to stop the turn — which is to
/// say one of them eventually would not, and would leave a `claude` burning the
/// user's subscription with nobody left to read what it says.
pub(crate) struct Chatting {
    /// The work as it happens and, once, how the turn ended. Closed by the
    /// worker dropping its end, which is how a panicked worker is noticed.
    pub(crate) events: Receiver<TurnEvent>,
    /// Say-when for the worker: the handle its agent answers to, and the kill
    /// switch for the `claude` it is waiting on.
    pub(crate) cancel: CancelGuard,
}

/// What a turn's worker thread has to say for itself.
///
/// Two things, and the order of them is fixed: any number of
/// [`TurnEvent::Doing`] while the model works, and then exactly one
/// [`TurnEvent::Finished`]. Nothing else is sent and nothing is sent after the
/// end — the worker drops its end of the channel and stops.
///
/// One channel for both, as a run has one channel for its progress and its
/// activities, and for the same reason: they come from the same worker, they are
/// read by the same thread, and a second receiver would be a second thing the
/// event loop has to poll and a second way for the two streams to arrive out of
/// the order the turn produced them in. It is also what makes the answer land
/// *after* the work lines it was produced by, on screen as in life.
#[derive(Debug)]
pub(crate) enum TurnEvent {
    /// The model was seen doing something: a tool call, a stretch of thinking,
    /// a stretch of writing, or what it cost.
    ///
    /// Carries nothing to say which turn it belongs to, because it needs
    /// nothing: one question at a time is the rule the loop keeps, so the live
    /// turn is the only turn anything can be filed under.
    Doing(Activity),
    /// The turn is over, however it went: the answer, or the one [`Ending`] the
    /// thread and the footer are worded from.
    ///
    /// A `Result` because a turn either answered or did not, and never a
    /// `Result` this module hands *out*: see [`apply_turn`], where both arms
    /// come to a row.
    Finished(Result<String, Ending>),
}

/// An activity port that forwards to `events`, for the agent of one turn.
///
/// [`pacting`](crate::pacting)'s port with a different event around it, and the
/// same three properties, which are the whole of why it is made per turn rather
/// than kept on the loop's long-lived agent. It is attached to a copy of the
/// agent that dies with the worker, so a `claude` still writing to a pipe after
/// its turn was abandoned has nowhere to report into the turn after it. It is
/// called on the worker's thread from inside the run, while the run is still
/// going, so it does the least it can: one send and back. And a send that fails
/// is ignored, because a receiver that has gone away is an application that is
/// quitting, and failing a turn that is otherwise going fine for the sake of a
/// screen nobody is looking at would help nobody.
fn activity_port(events: &Sender<TurnEvent>) -> Activities {
    let events = events.clone();
    Activities::new(move |activity| {
        let _ = events.send(TurnEvent::Doing(activity));
    })
}

/// This turn's own copy of the agent: the loop's, answering to `cancel` and
/// reporting to `events`.
///
/// The pair of handles that make a turn a thing that can be stopped and
/// watched, attached in one place so that the worker the loop starts and the
/// worker a test drives are wired the same way rather than similarly. A
/// [`ChatAgent`] is a command line, a session id and a timeout, so the copy
/// costs nothing and the loop's own agent keeps the handles it was built
/// with — which nobody holds and nothing listens to, so a cancel can never
/// reach out of the turn it was pressed for.
fn wired(agent: &ChatAgent, cancel: &Cancel, events: &Sender<TurnEvent>) -> ChatAgent {
    agent
        .clone()
        .with_cancel(cancel.clone())
        .with_activities(activity_port(events))
}

/// Put `message` to the model on a thread of its own, and hand back the channel
/// the answer comes back on.
///
/// The worker owns everything it touches — its own copy of the message and its
/// own copy of the agent — so nothing is shared with the event loop but the
/// channel and `cancel`, and that is what makes the thread safe to abandon: the
/// loop can return and the process exit at any moment without waiting for it,
/// because there is no state the two of them are half way through agreeing on.
/// A turn writes no file, so there is not even a half-written one to worry
/// about; the only thing it can leave behind is a child process, and the handle
/// is what takes that with it.
///
/// The [`JoinHandle`](std::thread::JoinHandle) is dropped on purpose, as a
/// run's is: joining is waiting, and this thread exists precisely so that
/// nobody waits for it. The worker reports on every path it takes itself, so
/// the only way the channel closes without a [`TurnEvent::Finished`] is a panic
/// in the worker — which [`apply_turn`] reads as the turn being over rather than
/// as a reason to hang.
pub(crate) fn spawn_turn(message: &str, agent: &ChatAgent, cancel: Cancel) -> Receiver<TurnEvent> {
    let (events, received) = mpsc::channel();
    let message = message.to_owned();
    let agent = wired(agent, &cancel, &events);
    thread::spawn(move || run_turn(&message, &agent, &cancel, &events));
    received
}

/// Everything a submitted message comes to, once the composer has decided there
/// is one.
///
/// [`start_run`](crate::pacting)'s opposite number: the one value the event loop
/// keeps about a turn it is not performing, made in one place so that starting a
/// turn is one call rather than a channel, a handle and an agent assembled
/// correctly at the keystroke.
///
/// The handle is made here and never reused, for the reason a run's is: a cancel
/// is final, so the turn after a cancelled one has to start with a handle nobody
/// has said stop to.
///
/// Nothing is said to the app here. The reader's message goes on the thread card
/// by [`App::start_turn`](warlock_tui::App::start_turn), at the keystroke, so
/// that the question is on screen from the moment it is asked rather than from
/// whenever the model first says something.
pub(crate) fn start_turn(message: &str, agent: &ChatAgent) -> Chatting {
    let cancel = CancelGuard::new();
    Chatting {
        events: spawn_turn(message, agent, cancel.handle()),
        cancel,
    }
}

/// The worker thread's whole body: ask, and say how it went.
///
/// Written as a function of its channel rather than inside the closure so that
/// it can be driven straight from a test — against a stand-in program, on the
/// test's own thread — and what a real turn sends asserted without a terminal, a
/// thread or a `claude`, exactly as [`run_pact`](crate::pacting) is.
///
/// Exactly one [`TurnEvent::Finished`] is sent, always, and it is the last thing
/// this function does. There is no path out of here that says nothing: a failure
/// is an ending like any other, and the wording of every one of them is
/// [`ending_for`]'s rather than this file's, so the panel's vocabulary for a
/// failed turn is written once, beside the thread it is drawn on.
///
/// The cancel is the one thing the seam cannot word for itself, and this is
/// where that fact is held. A killed child comes back as interrupted I/O and
/// nothing about the errno says who killed it, so a failure with the handle
/// latched is read as [`Ending::Cancelled`] here — which is what turns `the turn
/// could not run — …` into `the turn was cancelled` for the one person who
/// already knows why. An answer that beat the cancel by a hair is kept: it is a
/// real answer, and throwing it away would be a lie in the other direction.
fn run_turn(message: &str, agent: &ChatAgent, cancel: &Cancel, events: &Sender<TurnEvent>) {
    let finished = match agent.turn(message) {
        Ok(answer) => Ok(answer),
        Err(_) if cancel.is_cancelled() => Err(Ending::Cancelled),
        Err(error) => Err(ending_for(&error)),
    };
    // Ignored for the reason the activity port's sends are: a receiver that has
    // gone away is an application that is quitting.
    let _ = events.send(TurnEvent::Finished(finished));
}

/// Apply everything the turn has said since the last frame, and take it down
/// once it has said how it ended.
///
/// [`apply_progress`](crate::pacting)'s opposite number and the loop's whole
/// half of a turn. Drained rather than received: the worker is not waiting for
/// this thread, so a burst of tool calls can arrive between two frames, and the
/// loop ends the moment there is nothing left to read — which is the ordinary
/// case, since a turn is seconds and a frame is a tenth of one. Nothing here can
/// block, and that is the point of it: the tree still scrolls and the clocks
/// still tick while the model thinks.
///
/// Every ending is two lines and no error. One goes on the thread, under
/// whatever work had already arrived, so a turn cancelled after two tool calls
/// still shows those two tool calls; the other goes on the footer, in the same
/// words, because the reader may well be looking at the account or a document
/// rather than at the conversation. They are the same sentence by construction
/// rather than by two spellings happening to agree — [`Ending::line`] is asked
/// twice.
///
/// An answer says nothing on the footer. It is already on screen, on the card
/// the question brought to the front, and a footer line announcing it would be
/// warlock telling the reader about something they are looking at.
///
/// A channel that closes without an ending is a worker that panicked. It is
/// treated as the turn having ended, because it has: the panic hook has already
/// printed what happened, this loop is still drawing, and waiting for a message
/// from a thread that no longer exists would hang warlock on the one path where
/// it can least afford to.
///
/// `now` is the caller's clock, and this function reads none of its own: every
/// event drained here is filed under it, so a whole turn is drivable from a base
/// instant in a test exactly as the thread below it is.
pub(crate) fn apply_turn(chat: &mut Option<Chatting>, app: &mut App, now: Instant) {
    // No turn, nothing drained — which is what almost every frame of warlock's
    // life does here.
    let Some(chatting) = chat.as_ref() else {
        return;
    };

    let finished = loop {
        match chatting.events.try_recv() {
            // Filed under the live turn, which is the one this worker is
            // answering: what each activity comes to is the thread's business
            // and not this file's — a tool is its name and its one detail,
            // thinking and writing are the words for them, and a cost is summed
            // rather than drawn. See `Thread::record`.
            Ok(TurnEvent::Doing(activity)) => app.record_turn(&activity, now),
            Ok(TurnEvent::Finished(finished)) => break Some(finished),
            // Still going, and nothing new to say.
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => break None,
        }
    };

    // The turn is over on every path below, so the loop stops holding it before
    // anything is worded: what the reader does next — another question, or the
    // key that cancels — is answered by an empty slot rather than by a receiver
    // nobody will ever hear from again.
    chat.take();
    match finished {
        Some(Ok(answer)) => app.answer_turn(answer, now),
        Some(Err(ending)) => end(app, &ending, now),
        None => end(
            app,
            &Ending::Broke {
                reason: TURN_LOST.to_owned(),
            },
            now,
        ),
    }
}

/// End the live turn with `ending`, and say the same thing on the footer.
///
/// The two halves of a failure, in one place so that they cannot drift: a reader
/// on the thread card sees the line where the turn was, and a reader anywhere
/// else in the panel still hears that the question they asked is not coming
/// back. Neither of them is an error out of the event loop, and neither of them
/// touches anything else the app is holding — the tree, the account and the
/// document card are exactly as they were, which is what makes the next question
/// as ordinary as the last.
fn end(app: &mut App, ending: &Ending, now: Instant) {
    app.set_message(ending.line());
    app.end_turn(ending, now);
}

/// What one turn on a worker thread does: what it says on the way, what it ends
/// with, and what all of that comes to on the app the loop is drawing.
///
/// Two halves, and the seam between them is the channel. The tests below the
/// [`unix`](tests::unix) module drive [`apply_turn`] over a channel this file
/// writes to by hand, so what the loop does with an event is asserted without
/// running anything at all; the ones inside it drive [`run_turn`] and
/// [`spawn_turn`] against `/bin/sh` stand-ins, one per way a turn can fail, so
/// what a real `claude` would come back as is asserted without a `claude`. No
/// terminal, no network and no model in either half.
#[cfg(test)]
mod tests {
    use std::sync::mpsc::{self, Receiver, Sender};
    use std::time::{Duration, Instant};

    use warlock_tui::{Activity, App, Ending, INVOCATION_TIMEOUT, Line};

    use super::{Chatting, TurnEvent, apply_turn};
    use crate::pacting::CancelGuard;

    /// The question every test asks, so a row that quotes it is recognisable.
    const ASKED: &str = "what is in crates/warlock-engine?";

    /// `base` plus `seconds`, so a turn's whole timeline is one instant and
    /// some arithmetic rather than a sleep.
    fn at(base: Instant, seconds: u64) -> Instant {
        base + Duration::from_secs(seconds)
    }

    /// An app with [`ASKED`] on its thread card and a turn in flight under it,
    /// and the loop's end of the channel that turn reports on.
    ///
    /// Everything a submitted message leaves behind at the keystroke: the
    /// question is on the card already — [`App::start_turn`] put it there — and
    /// the worker is represented by a receiver this test sends down itself.
    fn asking(base: Instant) -> (App, Sender<TurnEvent>, Option<Chatting>) {
        let (events, received) = mpsc::channel();
        let mut app = App::default();
        app.start_turn(ASKED, base);
        (app, events, Some(chatting(received)))
    }

    /// The value the loop keeps for a turn reporting on `received`.
    fn chatting(received: Receiver<TurnEvent>) -> Chatting {
        Chatting {
            events: received,
            cancel: CancelGuard::new(),
        }
    }

    /// Every row of the app's thread card, clocked against `now`.
    fn rows(app: &App, now: Instant) -> Vec<Line> {
        app.thread().expect("a question has been asked").lines(now)
    }

    /// The reader's own message as the card draws it.
    fn said() -> Line {
        Line::Said {
            text: ASKED.to_owned(),
        }
    }

    /// A work line, clocked at `seconds` past the question.
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
        apply_turn(&mut chat, &mut app, at(base, 3));

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
        apply_turn(&mut chat, &mut app, at(base, 4));

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
            apply_turn(&mut chat, &mut app, at(base, 2));

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
                .thread()
                .and_then(|thread| thread.turns().last().cloned())
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
        apply_turn(&mut chat, &mut app, at(base, 2));
        assert!(chat.is_some(), "the turn is still in flight");

        // And then the cancel, which arrives as the worker's one ending like
        // any other — the loop does not take the turn down at the keystroke.
        events
            .send(TurnEvent::Finished(Err(Ending::Cancelled)))
            .expect("the loop is still listening");
        apply_turn(&mut chat, &mut app, at(base, 4));

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
        apply_turn(&mut chat, &mut app, at(base, 5));

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
        apply_turn(&mut chat, &mut app, at(base, 1));

        // The session is as usable as it was: a new turn, a new channel, and
        // the failed one still on the card above it.
        let (again, received) = mpsc::channel();
        let mut chat = Some(chatting(received));
        app.start_turn("and which of those is the biggest?", at(base, 10));
        again
            .send(TurnEvent::Finished(Ok("The engine.".to_owned())))
            .expect("the loop is still listening");
        apply_turn(&mut chat, &mut app, at(base, 12));

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

    #[test]
    fn a_frame_with_no_turn_in_flight_does_nothing_at_all() {
        let base = Instant::now();
        let mut app = App::default();
        let before = app.clone();

        apply_turn(&mut None, &mut app, at(base, 1));

        assert_eq!(app, before, "a frame with nothing running moved something");
    }

    /// What a turn comes back as when a real child really runs it: one test per
    /// way a turn can fail, and one for a turn that works.
    ///
    /// Unix-only for `claude.rs`'s reason — `/bin/sh` is the stand-in, and a
    /// shell script is the cheapest possible model — and driven through
    /// [`run_turn`] on the test's own thread, so what is asserted is the
    /// sequence of events a real turn sends rather than the timing of one. The
    /// two that go through [`spawn_turn`] and [`start_turn`] are the two that
    /// are *about* the thread: stopping a turn from another thread, and
    /// dropping one.
    #[cfg(unix)]
    mod unix {
        use std::path::{Path, PathBuf};
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::mpsc::{self, RecvTimeoutError};
        use std::time::{Duration, Instant};
        use std::{env, fs, process, thread};

        use warlock_tui::{Activity, App, Cancel, ChatAgent, Ending};

        use super::super::{Chat, TurnEvent, apply_turn, run_turn, spawn_turn, start_turn, wired};
        use super::{ASKED, at, chatting, clocked, rows, said};

        /// How long a test waits for a child to say it is running, or for a
        /// turn to come back, before giving up. Generous, because it is only
        /// reached when something is already wrong; every wait ends as soon as
        /// what it is waiting for happens.
        const AT_MOST: Duration = Duration::from_secs(5);

        /// A program that is not there, so a spawn fails the one way that has
        /// its own ending.
        const NOT_A_PROGRAM: &str = "/warlock/no/such/program";

        /// The lines of a turn, in miniature: the session's opening line, a
        /// tool call, a thought, the model starting to write, and the result
        /// line carrying the answer and what the turn cost.
        ///
        /// `claude.rs`'s `PASS` for the other kind of run, and deliberately the
        /// same shape — the transport is the same transport — with a chat's
        /// tools in the opening line and an answer where a document would be.
        const TURN: [&str; 5] = [
            r#"{"type":"system","subtype":"init","tools":["Read","Grep","Glob"]}"#,
            r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","id":"toolu_1","name":"Read","input":{"file_path":"src/lib.rs"}}]}}"#,
            r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"thinking","thinking":"a thought nobody is entitled to"}]}}"#,
            r#"{"type":"stream_event","event":{"type":"content_block_start","content_block":{"type":"text"}}}"#,
            r#"{"type":"result","subtype":"success","result":"The tree, the manifest and the pact.","total_cost_usd":0.0123}"#,
        ];

        /// The answer [`TURN`]'s result line carries.
        const ANSWER: &str = "The tree, the manifest and the pact.";

        /// What [`TURN`] reports, in order: the tool with its one whitelisted
        /// argument, the bare fact of the thought, the model starting to write,
        /// and the cost. Not the thought's text and not a word of the answer —
        /// the answer arrives once, as the turn's ending, and never in pieces
        /// among the work.
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

        /// An agent whose `claude` is `sh -c script`, as `claude.rs`'s tests
        /// build one.
        fn stand_in(script: &str) -> ChatAgent {
            ChatAgent::new()
                .with_program("/bin/sh")
                .with_args(["-c", script])
        }

        /// A shell script that prints `lines`, one per line, and exits.
        fn printing(lines: &[&str]) -> String {
            let arguments: Vec<String> = lines.iter().map(|line| format!("'{line}'")).collect();
            format!("printf '%s\\n' {}", arguments.join(" "))
        }

        /// Everything one turn of `agent` sends, in order, driven on this
        /// thread.
        ///
        /// The worker's body over the worker's own wiring: the agent is given
        /// this turn's cancel and this turn's activity port by the very
        /// function [`spawn_turn`] uses, so what a test drives and what the
        /// loop starts are the same run without a thread between them.
        fn turned(agent: &ChatAgent, cancel: &Cancel) -> Vec<TurnEvent> {
            let (events, received) = mpsc::channel();
            run_turn(ASKED, &wired(agent, cancel, &events), cancel, &events);
            received.try_iter().collect()
        }

        /// The work `events` reported, in order.
        fn doings(events: &[TurnEvent]) -> Vec<Activity> {
            events
                .iter()
                .filter_map(|event| match event {
                    TurnEvent::Doing(activity) => Some(activity.clone()),
                    TurnEvent::Finished(_) => None,
                })
                .collect()
        }

        /// How the turn ended, having asserted that it said so exactly once and
        /// last.
        ///
        /// The promise every test in here is about, checked in one place: a
        /// turn ends once, whatever it was doing when it ended, and nothing is
        /// sent after the end.
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

        /// How `events` ended, having asserted it did not answer.
        fn ending(events: &[TurnEvent]) -> Ending {
            ended(events).expect_err("this stand-in never answers")
        }

        /// A directory of this test's own, removed at the end of the test that
        /// made it — `claude.rs`'s helper, for the one test here that needs a
        /// child to write a file.
        fn scratch(name: &str) -> PathBuf {
            static NEXT: AtomicUsize = AtomicUsize::new(0);

            let unique = NEXT.fetch_add(1, Ordering::Relaxed);
            let directory =
                env::temp_dir().join(format!("warlock-chat-{}-{name}-{unique}", process::id()));
            fs::create_dir_all(&directory).expect("a scratch directory under the temp directory");
            directory
        }

        /// Best effort: a leftover under `/tmp` is untidy, not a test failure.
        fn clean_up(directory: &Path) {
            let _ = fs::remove_dir_all(directory);
        }

        /// How big `path` is, or nothing if it is not there yet.
        fn size(path: &Path) -> Option<u64> {
            fs::metadata(path).ok().map(|file| file.len())
        }

        /// Go round the loop's bottom end until the turn has ended, or give up
        /// after [`AT_MOST`].
        ///
        /// [`Chat::keep_up`] and nothing else, which is exactly what `keep_up`
        /// in the event loop calls: nothing here waits on the worker, joins a
        /// thread or receives from a channel — the round polls and comes back,
        /// and the turn ending is the drain taking it down.
        fn settle(chat: &mut Chat, app: &mut App, now: Instant) {
            let waited = Instant::now();
            while chat.answering() && waited.elapsed() < AT_MOST {
                chat.keep_up(app, now);
                thread::sleep(Duration::from_millis(10));
            }
            assert!(!chat.answering(), "the turn never ended");
        }

        /// Wait until `path` exists, or give up after [`AT_MOST`].
        ///
        /// How a test waits for a child to be genuinely running rather than
        /// guessing at a sleep.
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
            assert!(
                matches!(
                    received.recv_timeout(Duration::from_millis(200)),
                    Err(RecvTimeoutError::Disconnected)
                ),
                "the worker went on talking after it ended"
            );
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
            let chatting = start_turn(ASKED, &stand_in(&script));

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
            app.start_turn(ASKED, base);
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
                apply_turn(&mut chat, &mut app, at(base, 2));
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
                    warlock_tui::Line::Summary {
                        text: "this turn cost $0.01 — chat, never added to a pact's total"
                            .to_owned()
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
            let mut chat = Chat::with_agent(stand_in("sleep 300"));

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
            let mut chat = Chat::with_agent(stand_in(&script));

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
            // the answered one is whole: the work as it arrived, the answer, and
            // what this turn cost.
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
                    warlock_tui::Line::Summary {
                        text: "this turn cost $0.01 — chat, never added to a pact's total"
                            .to_owned()
                    },
                ]
            );
            clean_up(&directory);
        }
    }
}
