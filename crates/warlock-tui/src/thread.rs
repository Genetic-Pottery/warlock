//! What the reader asked, and what came back.
//!
//! The panel's other two cards are a ledger and a file: an account is one pact
//! writing documents, and a document is what it wrote. This one is the
//! conversation — the third card, and the only place in warlock where a model's
//! prose is ever shown. It is an ordered list of turns, one per message somebody
//! typed at the foot of the panel, each holding that message, the work the model
//! was seen doing while it answered, and the answer. Nothing else is a turn: a
//! pact running behind the conversation is the account card's, whole, and this
//! card has nothing to say about it.
//!
//! It is plain data like the account is: no terminal, no channel, no child
//! process, and no clock of its own. [`Instant::now`] is never called in this
//! file, so a whole conversation can be driven through it in a test off `base +
//! Duration::from_secs` with nothing attached to stdout, no `claude` installed
//! and no thread started.
//!
//! # A turn is a section that somebody asked for
//!
//! The clock rule is the account's, to the letter, and is the account's *code*
//! as well — both are built on [`Log`], so a work line here counts from the
//! start of the turn it belongs to, the newest one ticks against whatever `now`
//! the caller hands in, and it freezes at the instant the next line arrives.
//! A turn that has heard nothing yet shows the same `waiting` placeholder, for
//! the same reason: the first thing a model does with a question is spend a
//! while not answering it, and a still screen is what a hang looks like.
//!
//! The work lines are the account's shape too — a tool call is its name and the
//! one detail it carries, thinking is the word `thinking`, writing is the word
//! `writing` — because a `Grep` is a `Grep` whichever card it turns up on. And
//! what is kept out is kept out just as firmly: no tool results, no model
//! reasoning, no fragment of the answer arriving early. The answer is the one
//! piece of prose a turn has, it is a value of its own, and it lands whole.
//!
//! # A run is not a turn
//!
//! A pact or a refresh started while the conversation is on screen puts nothing
//! here. The panel has a card for a run — the account, one swap away, where the
//! directory headings, the passes and the summary all live in the account's own
//! words — and a conversation that also carried them would be a second copy of
//! the same run for the reader to reconcile with the first, in the middle of
//! what they were reading. So this card is what was asked and what came back,
//! and nothing that nobody typed.
//!
//! # Where this one differs
//!
//! Two things, both deliberate.
//!
//! A question costs money and the card says nothing about it. A pact's spending
//! is the account's business — per pass in `wrote … — 2341 bytes, $0.21`, and
//! totalled in `pact finished — …` — because a pact is a run somebody started
//! for a purpose and the money is part of what it did. A chat turn is a
//! sentence and a reply, and a price under every reply is a number a reader
//! cannot use for anything: it belongs to no total, it is not a budget, and
//! saying so on its own row spent a line of the card explaining a number that
//! should not have been there. So the cost a turn reports is heard and
//! discarded (see [`Thread::record`]).
//!
//! And a turn ends in one line however it ends. A pact has three
//! [`Outcome`](crate::Outcome)s because a pass either writes a document, is
//! refused, or is stopped; a turn has an answer or an [`Ending`] — cancelled,
//! no `claude` to ask, a non-zero exit, a timeout, or a model that finished with
//! nothing to say. Each is one line, said about the turn rather than about the
//! model, so a failed turn costs the reader a row and not a screen. Nothing here
//! returns an error to anybody: a turn that could not be run is a line in the
//! panel, which is the whole reason the endings are values.

use std::time::{Duration, Instant};

use warlock_engine::AgentError;

use crate::account::{Line, Log, THINKING, WRITING, tool_line};
use crate::claude::Activity;

/// One thing that stopped a turn short of an answer, in the words it ends with.
///
/// Five ways and no sixth that a caller has to word for itself: the four the
/// model seam fails in ([`AgentError`], mapped by [`ending_for`]), plus the one
/// warlock does on purpose. [`Ending::Broke`] is the catch — `AgentError` is
/// `#[non_exhaustive]`, so a variant added over there arrives here as whatever
/// it says about itself rather than as a panic or a silent nothing.
///
/// Every line it makes is about *the turn*: `the turn was cancelled`, `the turn
/// failed — …`. Never about the model, because none of these is a thing the
/// model did — it was stopped, or never reached, or not installed — and never in
/// an [`Account`](crate::Account)'s words, because a reader looking at the panel
/// should not have to work out which card they are on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ending {
    /// Somebody stopped the turn before the answer landed.
    ///
    /// Carries nothing: what was said before the cancel is already in the turn,
    /// and what it had spent by then is already the cost line's business.
    Cancelled,
    /// There is no such program to ask, which on a fresh machine is the
    /// ordinary state of the world rather than a fault.
    NoModel {
        /// The command that was looked for, e.g. `claude`.
        program: String,
    },
    /// The turn ran and exited non-zero: something went wrong on the far side.
    Failed {
        /// The exit code, or `None` if it was killed before it could set one.
        code: Option<i32>,
        /// What it wrote to stderr, verbatim and possibly over several lines —
        /// flattened into one line on the way to the panel, because a row is a
        /// row.
        stderr: String,
    },
    /// The turn ran past the time it was given and was stopped.
    TimedOut {
        /// How long it was given.
        after: Duration,
    },
    /// The turn finished cleanly and said nothing, so there is no answer to
    /// draw. Its own ending rather than an empty answer, because a blank turn
    /// on screen is indistinguishable from one that is still going.
    NothingSaid,
    /// The turn could not be run for some other reason: a pipe broke, a handle
    /// could not be opened, or the seam grew a failure this list has not been
    /// taught yet.
    Broke {
        /// What went wrong, in whatever words it came in.
        reason: String,
    },
}

impl Ending {
    /// The one line a turn ends with.
    ///
    /// One line and never two, whatever it carries: a stderr with a stack trace
    /// in it is flattened to a single row here and cut to the panel's width by
    /// whoever knows the width, the same division of labour the account keeps.
    ///
    /// Public because a failed turn is said in two places — the row under the
    /// question, and the footer, for a reader who is looking at another card —
    /// and those two have to be one sentence rather than two spellings that
    /// happen to agree. Whoever ends a turn puts this on the footer and hands
    /// the [`Ending`] itself to [`Thread::end`], which files the very same
    /// string as a line.
    ///
    /// ```
    /// use warlock_tui::Ending;
    ///
    /// assert_eq!(Ending::Cancelled.line(), "the turn was cancelled");
    /// ```
    #[must_use]
    pub fn line(&self) -> String {
        match self {
            Self::Cancelled => "the turn was cancelled".to_owned(),
            Self::NoModel { program } => {
                format!("the turn could not run — no {program} on PATH")
            }
            Self::Failed { code, stderr } => {
                let said = one_line(stderr);
                let said = if said.is_empty() {
                    String::new()
                } else {
                    format!(": {said}")
                };
                match code {
                    Some(code) => format!("the turn failed — exit status {code}{said}"),
                    None => format!("the turn failed — stopped before it exited{said}"),
                }
            }
            Self::TimedOut { after } => {
                format!("the turn ran past {}s and was stopped", after.as_secs())
            }
            Self::NothingSaid => "the turn ended with nothing to say".to_owned(),
            Self::Broke { reason } => format!("the turn could not run — {reason}"),
        }
    }
}

/// Which [`Ending`] a failed turn is.
///
/// The one place the model seam's failure vocabulary becomes the panel's, so
/// whoever runs a turn hands the error over rather than wording it — the same
/// arrangement as [`answer_for`](crate::answer_for) and
/// [`compose_for`](crate::compose_for), a value in and a value out.
///
/// A cancel does not come through here. It reaches a caller as an ordinary I/O
/// error — a killed child is a broken pipe, and nothing about the errno says who
/// killed it — so whether a turn was cancelled is a fact the caller holds and
/// [`Ending::Cancelled`] is theirs to choose.
#[must_use]
pub fn ending_for(error: &AgentError) -> Ending {
    match error {
        AgentError::NotFound { program } => Ending::NoModel {
            program: program.clone(),
        },
        AgentError::Failed { code, stderr } => Ending::Failed {
            code: *code,
            stderr: stderr.clone(),
        },
        AgentError::TimedOut { after } => Ending::TimedOut { after: *after },
        AgentError::EmptyOutput => Ending::NothingSaid,
        // `AgentError` is `#[non_exhaustive]`: `Io` lands here today, and so
        // does whatever the seam learns to fail with next. Its own `Display` is
        // one line already, which is exactly what a row needs.
        other => Ending::Broke {
            reason: other.to_string(),
        },
    }
}

/// One entry of the conversation: a question somebody typed, everything the
/// model was seen doing about it, and what came back.
///
/// Opened by [`Thread::ask`] and closed by [`Thread::answer`] or
/// [`Thread::end`]. Between those two it is the live turn — the one whose newest
/// work line ticks and the only one anything can be filed under — and a closed
/// turn never moves again, however long the session goes on.
///
/// One kind of turn and no second: a pact running behind the conversation is the
/// account card's, whole, and nothing anybody did not type is an entry here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Turn {
    /// The message the reader typed, exactly as they typed it, newlines and
    /// all. Never wrapped here: the width is a fact about a terminal.
    message: String,
    /// When the turn was asked, what has been filed under it, and whether its
    /// clock is still moving.
    log: Log,
    /// What came back, whole and unwrapped, once it has.
    answer: Option<String>,
    /// How it ended instead, if it did. Never both this and an answer: the
    /// first of the two to arrive closes the turn.
    ending: Option<Ending>,
}

impl Turn {
    /// The message this turn was asked in, in the reader's own words.
    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }

    /// What came back, or `None` while the turn is still going — and for good
    /// on a turn that ended without answering.
    ///
    /// Whole and unwrapped, as it arrived. Breaking it into the rows a panel of
    /// some width can draw it in happens on the way to the screen, so a terminal
    /// made narrower re-flows an answer the reader is looking at rather than
    /// asking the model again.
    #[must_use]
    pub fn answer(&self) -> Option<&str> {
        self.answer.as_deref()
    }

    /// How this turn ended short of an answer, or `None` if it did not.
    #[must_use]
    pub const fn ending(&self) -> Option<&Ending> {
        self.ending.as_ref()
    }

    /// When this turn was asked, which is where its clocks count from.
    #[must_use]
    pub const fn started(&self) -> Instant {
        self.log.started()
    }

    /// Whether this turn has stopped moving: answered, ended, or overtaken by a
    /// newer turn.
    #[must_use]
    pub const fn is_closed(&self) -> bool {
        self.log.is_closed()
    }

    /// How many rows this turn draws as, before anything is wrapped to a width.
    ///
    /// The message's own lines, the work lines — at least one, since a turn that
    /// has heard nothing draws the `waiting` placeholder — and the answer's own
    /// lines.
    fn line_count(&self) -> usize {
        broken(&self.message).count()
            + self.log.row_count()
            + self
                .answer
                .as_deref()
                .map_or(0, |answer| broken(answer).count())
    }

    /// Every row of this turn, in the order a reader reads them, with clocks
    /// measured against `now`.
    ///
    /// The question, then the work, then the answer, and nothing after it. An
    /// ending needs no arm of its own: it is filed as an ordinary line when the
    /// turn closes, so it clocks and freezes like everything else and sits
    /// exactly where a reader is already looking.
    fn rows(&self, now: Instant) -> impl Iterator<Item = Line> + '_ {
        let said = broken(&self.message).map(|text| Line::Said {
            text: text.to_owned(),
        });
        let answer = self
            .answer
            .iter()
            .flat_map(|answer| broken(answer))
            .map(|text| Line::Text {
                text: text.to_owned(),
            });

        said.chain(self.log.rows(now)).chain(answer)
    }

    /// Stop this turn moving as of `at`, if it has not stopped already.
    ///
    /// What a newer turn does to the one above it. It adds no line: a turn
    /// overtaken by a newer question is simply frozen where it got to, with
    /// whatever it had said still on screen.
    fn freeze(&mut self, at: Instant) {
        self.log.freeze(at);
    }

    /// This turn while it is still live, or `None` once it is over.
    ///
    /// The one gate everything a conversation does goes through: a message's
    /// activity, its answer and its ending are all filed here or dropped. A turn
    /// that is over is never it, for the reason [`Thread::record`] gives.
    fn live(&mut self) -> Option<&mut Self> {
        (!self.is_closed()).then_some(self)
    }

    /// Close this turn at `at` with the line `ending` makes.
    ///
    /// Reached only through [`Turn::live`], which is what makes the first
    /// ending win: a turn that is closed already is not a live one, so a cancel
    /// that lands a moment before the answer does is still the cancel that is on
    /// screen.
    fn word(&mut self, ending: &Ending, at: Instant) {
        self.log.push(ending.line(), at);
        self.ending = Some(ending.clone());
        self.log.freeze(at);
    }
}

/// The conversation, from the first question to the last answer — and every run
/// that happened while it was going on.
///
/// One session, one thread: warlock's chat is one conversation for the life of
/// the process, so turns are appended and nothing is ever dropped or trimmed —
/// a reader can scroll back to what they asked ten minutes ago, and the model
/// remembers it too, which is [`ChatAgent`](crate::ChatAgent)'s half of the same
/// arrangement.
///
/// Driven by four calls, all of which take the instant they happened at:
/// [`Thread::ask`] when a message is submitted, [`Thread::record`] for every
/// activity the turn reports, and [`Thread::answer`] or [`Thread::end`] when it
/// is over. Read back as rows with [`Thread::lines`] or [`Thread::window`],
/// which take the `now` the newest clock is measured against. There is no fifth
/// call and nothing for a pact to say here: a run belongs to the account card.
///
/// ```
/// use std::time::{Duration, Instant};
///
/// use warlock_tui::{Activity, Line, Thread};
///
/// let base = Instant::now();
/// let mut thread = Thread::new();
///
/// thread.ask("what is in crates/warlock-engine?", base);
/// thread.record(&Activity::Thinking, base + Duration::from_secs(1));
///
/// // Nothing newer has arrived, so the newest line counts up with `now`.
/// assert_eq!(
///     thread.lines(base + Duration::from_secs(9)),
///     vec![
///         Line::Said { text: "what is in crates/warlock-engine?".to_owned() },
///         Line::Clocked { clock: "0:09".to_owned(), text: "thinking".to_owned() },
///     ],
/// );
/// ```
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Thread {
    /// The turns, in the order they were asked.
    turns: Vec<Turn>,
}

impl Thread {
    /// A conversation nobody has said anything in yet.
    ///
    /// Empty means empty: no turns, no lines, nothing to draw. A panel whose
    /// thread card has never been asked a question draws warlock's mark, which
    /// is the same fact said one level up.
    #[must_use]
    pub const fn new() -> Self {
        Self { turns: Vec::new() }
    }

    /// Ask `message` at `at`, and freeze the turn above it.
    ///
    /// The clock under this question counts from `at`, so every turn starts
    /// again at `0:00` — and it is on screen from this call, as the `waiting`
    /// placeholder, rather than from whenever the model first says something.
    /// Whatever turn was live stops here: one question at a time is the rule
    /// the event loop keeps, and a turn still ticking under a newer one would
    /// be a second answer to that question.
    pub fn ask(&mut self, message: impl Into<String>, at: Instant) {
        self.freeze_last(at);
        self.turns.push(Turn {
            message: message.into(),
            log: Log::opened_at(at),
            answer: None,
            ending: None,
        });
    }

    /// Record what the live turn was seen doing at `at`.
    ///
    /// A tool becomes its name plus the one detail [`Activity`] chose to carry,
    /// thinking becomes the word `thinking`, writing the word `writing` — the
    /// account's wording, from the account's own function, because they are the
    /// same facts. A cost becomes nothing at all — not a line, not a total, not
    /// a field: what a chat turn spends is not a thing the turn did, it belongs
    /// to no total, and a price the reader can do nothing with is a row of the
    /// card spent on nothing.
    ///
    /// Does nothing when there is no live turn — before the first question, and
    /// after the current one has been answered or ended. A line cannot be filed
    /// under a turn that is already over without contradicting a line already on
    /// screen; dropping it is the honest way to fail.
    pub fn record(&mut self, activity: &Activity, at: Instant) {
        let Some(turn) = self.live() else {
            return;
        };

        match activity {
            // Heard and dropped: see above.
            Activity::Cost { .. } => {}
            // One line per stretch, however many times the stream says the
            // stretch is still going: the line already there goes on ticking,
            // and its clock is the count of how long the model has been at it.
            Activity::Thinking => turn.log.extend_or_open(THINKING, at),
            Activity::Writing { .. } => turn.log.extend_or_open(WRITING, at),
            Activity::Tool { name, detail } => {
                turn.log.push(tool_line(name, detail.as_ref()), at);
            }
        }
    }

    /// Land `answer` on the live turn at `at`, and close it.
    ///
    /// The answer is stored as it arrived, whole and unwrapped, and the turn
    /// stops moving: `at` is the instant its last work line freezes at.
    ///
    /// An answer with nothing in it is not an answer. A model that finished and
    /// said nothing ends the turn with [`Ending::NothingSaid`] instead, because
    /// a turn drawn as a question with nothing under it is indistinguishable
    /// from one still going, and a reader would sit and wait for it.
    ///
    /// Does nothing when there is no live turn, for [`Thread::record`]'s reason.
    pub fn answer(&mut self, answer: impl Into<String>, at: Instant) {
        let Some(turn) = self.live() else {
            return;
        };

        let answer = answer.into();
        if answer.trim().is_empty() {
            turn.word(&Ending::NothingSaid, at);
            return;
        }
        turn.answer = Some(answer);
        turn.log.freeze(at);
    }

    /// End the live turn at `at` with the one line `ending` makes.
    ///
    /// The cancel and every failure come through here, and every one of them is
    /// one line: whatever arrived before it stays exactly where it was, so a
    /// turn cancelled after two tool calls still shows those two tool calls.
    ///
    /// Does nothing when there is no live turn, or when the newest one is over
    /// already — the first ending wins, and a failure reported twice is still
    /// one line.
    pub fn end(&mut self, ending: &Ending, at: Instant) {
        if let Some(turn) = self.live() {
            turn.word(ending, at);
        }
    }

    /// The turns, in the order they were asked.
    #[must_use]
    pub fn turns(&self) -> &[Turn] {
        &self.turns
    }

    /// Whether nothing has been asked.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.turns.is_empty()
    }

    /// The turn still going, or `None` when none is.
    ///
    /// The live turn is the last one, and only while it is un-frozen: a turn
    /// stops being live the moment it answers, ends, or is overtaken by a newer
    /// one. What a caller does with it is decide whether the composer is muted —
    /// one question at a time — without keeping a second flag that could
    /// disagree with the thread.
    #[must_use]
    pub fn in_flight(&self) -> Option<&Turn> {
        self.turns.last().filter(|turn| !turn.is_closed())
    }

    /// Stop the newest turn moving as of `at`: what every turn opening does to
    /// the one above it. See [`Turn::freeze`].
    fn freeze_last(&mut self, at: Instant) {
        if let Some(previous) = self.turns.last_mut() {
            previous.freeze(at);
        }
    }

    /// The live turn, which is the only turn anything can be filed under.
    fn live(&mut self) -> Option<&mut Turn> {
        self.turns.last_mut().and_then(Turn::live)
    }

    /// How many rows the whole thread draws as, before anything is wrapped.
    ///
    /// What a scroll offset is clamped against — at a width of nothing, which is
    /// every card's count until the frame says otherwise. The card is the one
    /// that knows the width, and it is the one that wraps.
    #[must_use]
    pub fn line_count(&self) -> usize {
        self.turns.iter().map(Turn::line_count).sum()
    }

    /// Every row of the thread, with clocks measured against `now`.
    ///
    /// `now` is the caller's: this reads no clock, so the same thread and the
    /// same instant give the same rows every time. Only the newest work line of
    /// a live turn depends on it.
    #[must_use]
    pub fn lines(&self, now: Instant) -> Vec<Line> {
        self.window(0, self.line_count(), now)
    }

    /// The `height` rows starting at `offset`, with clocks measured against
    /// `now`.
    ///
    /// Asking for more rows than there are, or starting past the end, gives back
    /// what is there rather than failing: a viewport is a request, not an
    /// assertion about the conversation's length.
    #[must_use]
    pub fn window(&self, offset: usize, height: usize, now: Instant) -> Vec<Line> {
        self.turns
            .iter()
            .flat_map(move |turn| turn.rows(now))
            .skip(offset)
            .take(height)
            .collect()
    }
}

/// `text` as the rows it is drawn in before any wrapping: its own lines, and one
/// empty row for a text with nothing in it.
///
/// Never empty, for [`wrapped`](crate::wrap)'s reason: a blank line is a
/// paragraph break, and a message or an answer that came to no rows at all would
/// be a turn with a hole in it.
fn broken(text: &str) -> impl Iterator<Item = &str> {
    let mut lines = text.lines().peekable();
    let empty = lines.peek().is_none().then_some("");
    empty.into_iter().chain(lines)
}

/// `text` as one line: runs of whitespace collapsed to single spaces.
///
/// A row is a row, and a stderr with a stack trace in it would otherwise be
/// several. Nothing is cut — the account refuses to truncate for the same
/// reason, since the width is a fact about a terminal and this is not.
fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use warlock_engine::AgentError;

    use super::{Ending, Thread, Turn, ending_for};
    use crate::account::{Account, Line, Outcome};
    use crate::claude::Activity;

    /// The instant `seconds` after `base`, so a whole conversation can be driven
    /// without anything ever reading a clock.
    fn at(base: Instant, seconds: u64) -> Instant {
        base + Duration::from_secs(seconds)
    }

    /// The activity a turn reports for a tool call with one detail worth saying.
    fn tool(name: &str, detail: &str) -> Activity {
        Activity::Tool {
            name: name.to_owned(),
            detail: Some(detail.to_owned()),
        }
    }

    /// What the thread's rows say, as plain text, for the tests that care about
    /// the wording rather than the shape.
    fn said(thread: &Thread, now: Instant) -> Vec<String> {
        thread
            .lines(now)
            .into_iter()
            .map(|line| match line {
                // Never a directory heading, which is an account's row: it is
                // here so this helper words every row of the panel and not
                // most of them.
                Line::Directory { path } => path.display().to_string(),
                Line::Clocked { clock, text } => format!("{clock} {text}"),
                // A thread holds a conversation, not rows: a wrapped row is
                // made on the way to a panel of some width.
                Line::Summary { text }
                | Line::Text { text }
                | Line::Said { text }
                | Line::Wrapped { text, .. } => text,
            })
            .collect()
    }

    /// Only the clocked rows: the work lines, which is where the promises about
    /// what never reaches the panel are kept.
    fn work(thread: &Thread, now: Instant) -> Vec<String> {
        thread
            .lines(now)
            .into_iter()
            .filter_map(|line| match line {
                Line::Clocked { text, .. } => Some(text),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_conversation_nobody_has_started_draws_nothing() {
        let base = Instant::now();
        let thread = Thread::new();

        assert!(thread.is_empty());
        assert_eq!(thread.line_count(), 0);
        assert_eq!(thread.lines(at(base, 30)), Vec::new());
        assert!(thread.turns().is_empty());
        assert!(thread.in_flight().is_none());
    }

    #[test]
    fn a_whole_turn_reads_as_question_then_work_then_answer() {
        let base = Instant::now();
        let mut thread = Thread::new();

        thread.ask("what does the engine do?", base);
        thread.record(&Activity::Thinking, at(base, 2));
        thread.record(&tool("grep", "pact_directory"), at(base, 8));
        thread.record(
            &tool("read", "crates/warlock-engine/src/pact.rs"),
            at(base, 11),
        );
        thread.record(&Activity::Writing { bytes: 0 }, at(base, 30));
        thread.record(&Activity::Cost { usd: 0.02 }, at(base, 31));
        thread.answer("It pacts a tree.", at(base, 42));

        assert_eq!(
            said(&thread, at(base, 90)),
            vec![
                "what does the engine do?".to_owned(),
                // Each clock is the instant the next line arrived, measured
                // from the question rather than from the session.
                "0:08 thinking".to_owned(),
                "0:11 grep pact_directory".to_owned(),
                "0:30 read crates/warlock-engine/src/pact.rs".to_owned(),
                "0:42 writing".to_owned(),
                "It pacts a tree.".to_owned(),
            ],
        );
        assert_eq!(thread.line_count(), 6);
        assert_eq!(thread.lines(at(base, 90)).len(), thread.line_count());
    }

    #[test]
    fn a_turn_that_has_heard_nothing_ticks_waiting_until_the_model_speaks() {
        let base = Instant::now();
        let mut thread = Thread::new();

        thread.ask("why?", base);

        // On screen from the moment the question is submitted: the clock is
        // already moving with nothing recorded, which is what says the silence
        // is a model and not a hang.
        assert_eq!(thread.line_count(), 2);
        assert_eq!(
            said(&thread, at(base, 12)),
            vec!["why?".to_owned(), "0:12 waiting".to_owned()],
        );

        // The first real line takes its place, on the same clock.
        thread.record(&Activity::Thinking, at(base, 58));
        assert_eq!(
            said(&thread, at(base, 60)),
            vec!["why?".to_owned(), "1:00 thinking".to_owned()],
        );
    }

    #[test]
    fn the_newest_work_line_ticks_with_now_and_freezes_when_the_answer_lands() {
        let base = Instant::now();
        let mut thread = Thread::new();

        thread.ask("go on", base);
        thread.record(&Activity::Thinking, at(base, 1));

        assert_eq!(work(&thread, at(base, 5)), vec!["thinking".to_owned()]);
        assert_eq!(
            said(&thread, at(base, 5))[1],
            "0:05 thinking",
            "a live turn's newest line counts up with the caller's now",
        );
        assert_eq!(said(&thread, at(base, 9))[1], "0:09 thinking");

        thread.answer("done", at(base, 20));

        // Frozen where the answer landed, however long the session runs on.
        assert_eq!(said(&thread, at(base, 600))[1], "0:20 thinking");
        assert!(thread.in_flight().is_none());
    }

    #[test]
    fn every_turn_starts_its_clock_again_at_zero() {
        let base = Instant::now();
        let mut thread = Thread::new();

        thread.ask("first", at(base, 10));
        thread.record(&Activity::Thinking, at(base, 12));
        thread.ask("second", at(base, 70));
        thread.record(&Activity::Thinking, at(base, 71));

        // The first turn's line froze when the second question was asked, one
        // minute in; the second's counts from its own question.
        assert_eq!(
            said(&thread, at(base, 75)),
            vec![
                "first".to_owned(),
                "1:00 thinking".to_owned(),
                "second".to_owned(),
                "0:05 thinking".to_owned(),
            ],
        );
        assert_eq!(thread.turns().len(), 2);
        assert_eq!(thread.turns()[0].message(), "first");
        assert_eq!(thread.in_flight().map(Turn::started), Some(at(base, 70)));
    }

    #[test]
    fn a_stretch_of_thinking_is_one_line_however_often_it_is_reported() {
        let base = Instant::now();
        let mut thread = Thread::new();

        thread.ask("hello", base);
        for second in [2, 5, 9, 40] {
            thread.record(&Activity::Thinking, at(base, second));
        }

        assert_eq!(
            said(&thread, at(base, 55)),
            vec!["hello".to_owned(), "0:55 thinking".to_owned()],
            "one line, counting from when thinking started",
        );

        // Something else happening ends the stretch, and thinking after it is a
        // new one — the two are separated by what came between them.
        thread.record(&tool("read", "src/lib.rs"), at(base, 60));
        thread.record(&Activity::Thinking, at(base, 61));
        thread.record(&Activity::Thinking, at(base, 65));

        assert_eq!(
            work(&thread, at(base, 70)),
            vec![
                "thinking".to_owned(),
                "read src/lib.rs".to_owned(),
                "thinking".to_owned(),
            ],
        );
    }

    #[test]
    fn a_tool_without_a_detail_is_its_bare_name() {
        let base = Instant::now();
        let mut thread = Thread::new();

        thread.ask("look around", base);
        thread.record(
            &Activity::Tool {
                name: "glob".to_owned(),
                detail: None,
            },
            at(base, 3),
        );

        assert_eq!(work(&thread, at(base, 4)), vec!["glob".to_owned()]);
    }

    #[test]
    fn what_a_turn_cost_is_heard_and_never_drawn() {
        let base = Instant::now();
        let mut thread = Thread::new();

        thread.ask("what is this?", base);
        thread.record(&Activity::Cost { usd: 0.01 }, at(base, 1));
        thread.record(&Activity::Cost { usd: 0.014 }, at(base, 2));

        // Not a work line, and not a line anywhere else either: a report of
        // what a chat turn spent belongs to no total a reader can use, so the
        // card is the same card whether it arrives or not.
        assert_eq!(work(&thread, at(base, 3)), vec!["waiting".to_owned()]);

        thread.answer("this.", at(base, 4));
        assert_eq!(
            said(&thread, at(base, 5)),
            vec![
                "what is this?".to_owned(),
                "0:04 waiting".to_owned(),
                "this.".to_owned(),
            ],
        );

        // The same rows a turn that reported nothing draws.
        let mut silent = Thread::new();
        silent.ask("what is this?", base);
        silent.answer("this.", at(base, 4));
        assert_eq!(silent.lines(at(base, 5)), thread.lines(at(base, 5)));
    }

    #[test]
    fn no_money_reaches_this_card_however_much_a_turn_reports() {
        let base = Instant::now();

        let mut thread = Thread::new();
        thread.ask("how much?", base);
        thread.record(&Activity::Cost { usd: 0.21 }, at(base, 1));
        thread.answer("that much.", at(base, 2));

        // The same twenty-one cents on an account, which is the card money is
        // said on: what a pass produced and what it cost, totalled.
        let mut account = Account::new(base);
        account.open_section("crates/engine", base);
        account.record(&Activity::Cost { usd: 0.21 }, at(base, 1));
        account.close_section(
            &Outcome::Wrote {
                document: "crates/engine/WARLOCK.md".into(),
                bytes: 2341,
            },
            at(base, 2),
        );
        account.finish(at(base, 3));

        let asked = said(&thread, at(base, 4));
        let pact_money: Vec<String> = account
            .lines(at(base, 4))
            .into_iter()
            .filter_map(|line| match line {
                Line::Clocked { text, .. } | Line::Summary { text } => {
                    text.contains("$0.21").then_some(text)
                }
                _ => None,
            })
            .collect();

        // The account says what a pass produced and totals those; the question
        // says nothing about money at all, so there are not two numbers on one
        // screen for a reader to add up.
        assert_eq!(
            pact_money,
            vec![
                "wrote crates/engine/WARLOCK.md — 2341 bytes, $0.21".to_owned(),
                "pact finished — 1 directory, 0:03, $0.21".to_owned(),
            ],
        );
        assert_eq!(
            asked,
            vec![
                "how much?".to_owned(),
                "0:02 waiting".to_owned(),
                "that much.".to_owned(),
            ],
        );
        assert!(!asked.iter().any(|line| line.contains('$')));
    }

    #[test]
    fn no_tool_result_and_no_model_prose_ever_reaches_a_work_line() {
        let base = Instant::now();
        let mut thread = Thread::new();

        // Everything a turn can be told, including a tool whose detail is the
        // only thing it carries: none of it is a result, and none of it is
        // reasoning. The answer is the one piece of prose, and it is a text row
        // rather than a clocked one.
        thread.ask("summarise the engine", base);
        thread.record(&Activity::Thinking, at(base, 1));
        thread.record(&tool("grep", "fn pact_directory"), at(base, 4));
        thread.record(&tool("read", "src/pact.rs"), at(base, 6));
        thread.record(&Activity::Writing { bytes: 0 }, at(base, 9));
        thread.answer(
            "The engine walks a tree and writes a document per directory.",
            at(base, 20),
        );

        assert_eq!(
            work(&thread, at(base, 21)),
            vec![
                "thinking".to_owned(),
                "grep fn pact_directory".to_owned(),
                "read src/pact.rs".to_owned(),
                "writing".to_owned(),
            ],
        );
        for line in work(&thread, at(base, 21)) {
            assert!(
                !line.contains("engine walks"),
                "the answer's prose never appears among the work lines: {line}",
            );
        }
    }

    #[test]
    fn an_answer_is_kept_whole_and_unwrapped() {
        let base = Instant::now();
        let mut thread = Thread::new();

        let long = "a ".repeat(120);
        thread.ask("say something long", base);
        thread.answer(format!("{long}\n\nand a second paragraph"), at(base, 5));

        // Stored exactly as it arrived: wrapping is the frame's business, and a
        // terminal made narrower should re-flow this rather than ask again.
        let answer = thread.turns()[0]
            .answer()
            .expect("the turn answered")
            .to_owned();
        assert_eq!(answer, format!("{long}\n\nand a second paragraph"));

        // Its own newlines are rows, though, because they are the answer's.
        assert_eq!(
            thread
                .lines(at(base, 6))
                .into_iter()
                .filter(|line| matches!(line, Line::Text { .. }))
                .count(),
            3,
        );
    }

    #[test]
    fn a_cancelled_turn_keeps_what_arrived_and_gains_one_line() {
        let base = Instant::now();
        let mut thread = Thread::new();

        thread.ask("read the tree", base);
        thread.record(&tool("read", "a.rs"), at(base, 2));
        thread.record(&tool("read", "b.rs"), at(base, 4));
        thread.end(&Ending::Cancelled, at(base, 6));

        assert_eq!(
            said(&thread, at(base, 60)),
            vec![
                "read the tree".to_owned(),
                "0:04 read a.rs".to_owned(),
                "0:06 read b.rs".to_owned(),
                "0:06 the turn was cancelled".to_owned(),
            ],
        );
        assert_eq!(thread.turns()[0].ending(), Some(&Ending::Cancelled));
        assert!(thread.turns()[0].answer().is_none());
        assert!(thread.in_flight().is_none());
    }

    #[test]
    fn every_failure_ends_the_turn_in_exactly_one_line() {
        let base = Instant::now();
        let endings = [
            (
                Ending::NoModel {
                    program: "claude".to_owned(),
                },
                "the turn could not run — no claude on PATH",
            ),
            (
                Ending::Failed {
                    code: Some(2),
                    stderr: "not logged in\n  try `claude login`".to_owned(),
                },
                "the turn failed — exit status 2: not logged in try `claude login`",
            ),
            (
                Ending::Failed {
                    code: None,
                    stderr: String::new(),
                },
                "the turn failed — stopped before it exited",
            ),
            (
                Ending::TimedOut {
                    after: Duration::from_secs(90),
                },
                "the turn ran past 90s and was stopped",
            ),
            (Ending::NothingSaid, "the turn ended with nothing to say"),
            (
                Ending::Broke {
                    reason: "broken pipe".to_owned(),
                },
                "the turn could not run — broken pipe",
            ),
        ];

        for (ending, wording) in endings {
            let mut thread = Thread::new();
            thread.ask("go", base);
            thread.end(&ending, at(base, 3));

            assert_eq!(
                said(&thread, at(base, 30)),
                vec!["go".to_owned(), format!("0:03 {wording}")],
                "one question, one line, and nothing else",
            );
            assert_eq!(thread.turns()[0].ending(), Some(&ending));
            assert!(thread.in_flight().is_none(), "the session is free again");
        }
    }

    #[test]
    fn a_model_that_finished_with_nothing_to_say_ends_rather_than_answers() {
        let base = Instant::now();
        let mut thread = Thread::new();

        thread.ask("well?", base);
        thread.answer("   \n  ", at(base, 4));

        // A blank turn on screen is indistinguishable from one still going, so
        // an empty answer is a failure with a line of its own.
        assert!(thread.turns()[0].answer().is_none());
        assert_eq!(thread.turns()[0].ending(), Some(&Ending::NothingSaid));
        assert_eq!(
            said(&thread, at(base, 30)),
            vec![
                "well?".to_owned(),
                "0:04 the turn ended with nothing to say".to_owned()
            ],
        );
    }

    #[test]
    fn the_first_ending_wins_and_nothing_lands_on_a_closed_turn() {
        let base = Instant::now();
        let mut thread = Thread::new();

        thread.ask("stop", base);
        thread.end(&Ending::Cancelled, at(base, 2));

        // Everything after the cancel is dropped: a second ending, an answer
        // that was already on its way, and any activity still in the channel.
        thread.end(&Ending::NothingSaid, at(base, 3));
        thread.answer("too late", at(base, 4));
        thread.record(&Activity::Thinking, at(base, 5));
        thread.record(&Activity::Cost { usd: 9.99 }, at(base, 6));

        assert_eq!(
            said(&thread, at(base, 30)),
            vec!["stop".to_owned(), "0:02 the turn was cancelled".to_owned()],
        );
        assert_eq!(thread.turns()[0].ending(), Some(&Ending::Cancelled));
    }

    #[test]
    fn nothing_can_be_filed_before_the_first_question() {
        let base = Instant::now();
        let mut thread = Thread::new();

        thread.record(&Activity::Thinking, base);
        thread.answer("out of nowhere", at(base, 1));
        thread.end(&Ending::Cancelled, at(base, 2));

        assert!(thread.is_empty());
        assert_eq!(thread.lines(at(base, 3)), Vec::new());
    }

    #[test]
    fn a_cancelled_turn_ends_in_the_line_that_cancelled_it_and_nothing_after() {
        let base = Instant::now();
        let mut thread = Thread::new();

        thread.ask("expensive?", base);
        thread.record(&Activity::Cost { usd: 0.4 }, at(base, 1));
        thread.end(&Ending::Cancelled, at(base, 2));

        assert_eq!(
            said(&thread, at(base, 30)),
            vec![
                "expensive?".to_owned(),
                "0:02 the turn was cancelled".to_owned(),
            ],
        );
    }

    #[test]
    fn a_message_with_newlines_in_it_is_the_rows_it_was_typed_as() {
        let base = Instant::now();
        let mut thread = Thread::new();

        thread.ask("first line\nsecond line", base);

        assert_eq!(
            thread.lines(at(base, 1))[..2],
            [
                Line::Said {
                    text: "first line".to_owned()
                },
                Line::Said {
                    text: "second line".to_owned()
                },
            ],
        );
        assert_eq!(thread.line_count(), 3);
    }

    #[test]
    fn a_window_gives_back_what_is_there_rather_than_what_was_asked_for() {
        let base = Instant::now();
        let mut thread = Thread::new();

        thread.ask("one", base);
        thread.answer("uno", at(base, 1));
        thread.ask("two", at(base, 2));
        thread.answer("dos", at(base, 3));

        assert_eq!(thread.line_count(), 6);
        assert_eq!(
            thread.window(3, 2, at(base, 4)),
            vec![
                Line::Said {
                    text: "two".to_owned()
                },
                Line::Clocked {
                    clock: "0:01".to_owned(),
                    text: "waiting".to_owned()
                },
            ],
        );
        assert_eq!(thread.window(0, 99, at(base, 4)).len(), 6);
        assert_eq!(thread.window(99, 4, at(base, 4)), Vec::new());
    }

    #[test]
    fn a_run_is_no_part_of_this_card_however_it_is_driven() {
        // The account is the run's card and this one has no idea it exists:
        // nothing here opens, feeds or closes a run, and there is no call that
        // could. What the conversation holds is what was typed into it.
        let base = Instant::now();
        let mut thread = Thread::new();

        thread.ask("what is in crates?", base);
        thread.record(&Activity::Thinking, at(base, 1));
        thread.answer("The engine and the front end.", at(base, 2));

        let mut account = Account::new(base);
        account.open_section("crates/engine", base);
        account.record(&Activity::Thinking, at(base, 2));
        account.close_section(
            &Outcome::Wrote {
                document: "crates/engine/WARLOCK.md".into(),
                bytes: 2341,
            },
            at(base, 3),
        );
        account.finish(at(base, 4));

        // A whole run happened beside the conversation, and the conversation is
        // the one turn somebody asked for.
        assert_eq!(thread.turns().len(), 1);
        assert_eq!(
            said(&thread, at(base, 900)),
            vec![
                "what is in crates?".to_owned(),
                "0:02 thinking".to_owned(),
                "The engine and the front end.".to_owned(),
            ],
        );
        assert!(
            thread.in_flight().is_none(),
            "a run cannot mute a conversation it is no part of"
        );
    }

    #[test]
    fn the_seams_failures_become_the_panels_endings() {
        assert_eq!(
            ending_for(&AgentError::NotFound {
                program: "claude".to_owned()
            }),
            Ending::NoModel {
                program: "claude".to_owned()
            },
        );
        assert_eq!(
            ending_for(&AgentError::Failed {
                code: Some(1),
                stderr: "boom".to_owned(),
            }),
            Ending::Failed {
                code: Some(1),
                stderr: "boom".to_owned(),
            },
        );
        assert_eq!(
            ending_for(&AgentError::TimedOut {
                after: Duration::from_secs(30)
            }),
            Ending::TimedOut {
                after: Duration::from_secs(30)
            },
        );
        assert_eq!(ending_for(&AgentError::EmptyOutput), Ending::NothingSaid);

        // Anything the seam grows later arrives as whatever it says about
        // itself, rather than as a panic or a silent nothing.
        let broken = AgentError::Io {
            source: std::io::Error::other("broken pipe"),
        };
        assert_eq!(
            ending_for(&broken),
            Ending::Broke {
                reason: broken.to_string()
            },
        );
    }
}
