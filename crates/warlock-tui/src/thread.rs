//! The conversation card: an ordered list of turns and of notes, one line each,
//! that warlock says for itself.
//!
//! Plain data, as the account is. [`Instant::now`] is never called in this file,
//! so a whole conversation can be driven off `base + Duration::from_secs` with
//! nothing attached to stdout and no `claude` installed. The clock rule is
//! literally the account's code — both are built on [`Log`] — and tool results,
//! model reasoning and early fragments of an answer are kept out so the answer
//! lands whole.
//!
//! Notes share the sequence with the turns rather than sitting in a side table,
//! because *when* one was said is the whole of what it means: a warning above
//! the turn it is about and the same warning three turns later are different
//! warnings. A pact running behind the conversation puts nothing here, because
//! the account card already carries the passes and the summary. What a turn
//! costs is heard and discarded (see [`Thread::record`]): a price under a single
//! reply belongs to no total.

use std::time::{Duration, Instant};

use warlock_engine::agent;

use crate::account::{Line, Log, THINKING, WRITING, tool_line};
use crate::claude::Activity;

/// Five ways a turn stops short of an answer and no sixth a caller has to word
/// for itself: the four the model seam fails in
/// ([`agent::Error`](warlock_engine::agent::Error), mapped by [`ending_for`]),
/// plus the cancel warlock does on purpose. [`Ending::Broke`] is the catch,
/// because `agent::Error` is `#[non_exhaustive]` and a variant added over there
/// has to arrive as whatever it says about itself rather than as a panic.
///
/// Every line it makes is about *the turn* and never about the model: none of
/// these is a thing the model did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ending {
    Cancelled,
    NoModel { program: String },
    Failed { code: Option<i32>, stderr: String },
    TimedOut { after: Duration },
    NothingSaid,
    Broke { reason: String },
}

impl Ending {
    /// One line and never two, whatever it carries: a stderr with a stack trace
    /// in it is flattened here and cut to width by whoever knows the width.
    ///
    /// Public because a failed turn is said in two places — the row under the
    /// question, and the footer for a reader looking at another card — and those
    /// two have to be one string rather than two spellings that happen to agree.
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

/// The one place the model seam's failure vocabulary becomes the panel's, so
/// whoever runs a turn hands the error over rather than wording it.
///
/// A cancel does not come through here. It reaches a caller as an ordinary I/O
/// error — a killed child is a broken pipe, and nothing about the errno says who
/// killed it — so [`Ending::Cancelled`] is the caller's to choose.
#[must_use]
pub fn ending_for(error: &agent::Error) -> Ending {
    match error {
        agent::Error::NotFound { program } => Ending::NoModel {
            program: program.clone(),
        },
        agent::Error::Failed { code, stderr } => Ending::Failed {
            code: *code,
            stderr: stderr.clone(),
        },
        agent::Error::TimedOut { after } => Ending::TimedOut { after: *after },
        agent::Error::EmptyOutput => Ending::NothingSaid,
        // `agent::Error` is `#[non_exhaustive]`: `Io` lands here today, and so
        // does whatever the seam learns to fail with next. Its own `Display` is
        // one line already, which is exactly what a row needs.
        other => Ending::Broke {
            reason: other.to_string(),
        },
    }
}

/// Opened by [`Thread::ask`] and closed by [`Thread::answer`] or
/// [`Thread::end`]. Between those two it is the live turn, the only one anything
/// can be filed under, and a closed turn never moves again.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Turn {
    message: String,
    log: Log,
    answer: Option<String>,
    ending: Option<Ending>,
}

impl Turn {
    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }

    /// Whole and unwrapped, as it arrived. Breaking it into rows happens on the
    /// way to the screen, so a terminal made narrower re-flows an answer the
    /// reader is looking at rather than asking the model again.
    #[must_use]
    pub fn answer(&self) -> Option<&str> {
        self.answer.as_deref()
    }

    #[must_use]
    pub const fn ending(&self) -> Option<&Ending> {
        self.ending.as_ref()
    }

    #[must_use]
    pub const fn started(&self) -> Instant {
        self.log.started()
    }

    /// Answered, ended, or overtaken by a newer turn.
    #[must_use]
    pub const fn is_closed(&self) -> bool {
        self.log.is_closed()
    }

    /// At least one work row, because a turn that has heard nothing still draws
    /// the `waiting` placeholder.
    fn line_count(&self) -> usize {
        broken(&self.message).count()
            + self.log.row_count()
            + self
                .answer
                .as_deref()
                .map_or(0, |answer| broken(answer).count())
    }

    /// `piece` is where this turn's message sits in [`Thread::pieces`], so the
    /// answer's is the one after it. Everything a turn draws is made here and
    /// [`Entry::rows`] throws the tags away: two flattenings would be a row
    /// tagged with text that is not under it.
    ///
    /// An ending needs no arm of its own: it is filed as an ordinary line when
    /// the turn closes, so it clocks and freezes like everything else.
    fn sourced(&self, piece: usize, now: Instant) -> Vec<Sourced> {
        let mut rows: Vec<Sourced> = broken(&self.message)
            .map(|(offset, text)| Sourced {
                line: Line::Said {
                    text: text.to_owned(),
                },
                piece,
                offset,
                work: false,
            })
            .collect();

        // A work row is nobody's text — the live one's clock is recomputed every
        // frame — so it stands for the end of the question above it, which is
        // there before the answer is.
        rows.extend(self.log.rows(now).map(|line| Sourced {
            line,
            piece,
            offset: self.message.len(),
            work: true,
        }));

        rows.extend(
            self.answer
                .iter()
                .flat_map(|answer| broken(answer))
                .map(|(offset, text)| Sourced {
                    line: Line::Text {
                        text: text.to_owned(),
                    },
                    piece: piece + 1,
                    offset,
                    work: false,
                }),
        );

        rows
    }

    fn pieces(&self) -> Vec<&str> {
        let mut pieces = vec![self.message.as_str()];
        pieces.extend(self.answer.as_deref());
        pieces
    }

    /// What a newer turn does to the one above it. It adds no line: an
    /// overtaken turn is frozen where it got to, with what it had said still on
    /// screen.
    fn freeze(&mut self, at: Instant) {
        self.log.freeze(at);
    }

    /// The one gate everything a conversation does goes through: activity, an
    /// answer and an ending are all filed here or dropped.
    fn live(&mut self) -> Option<&mut Self> {
        (!self.is_closed()).then_some(self)
    }

    /// Reached only through [`Turn::live`], which is what makes the first ending
    /// win: a cancel landing a moment before the answer does is still the cancel
    /// on screen.
    fn word(&mut self, ending: &Ending, at: Instant) {
        self.log.push(ending.line(), at);
        self.ending = Some(ending.clone());
        self.log.freeze(at);
    }
}

/// A row of the card and the text it was drawn from: which of
/// [`Thread::pieces`] it came out of, where in that piece its own text starts,
/// and whether the row is one that stands for a position rather than holding
/// text of its own.
///
/// [`Thread::window`] flattens entries to [`Line`]s and keeps none of this,
/// which is why a cell cannot be read back into the conversation from a
/// [`Line`] alone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Sourced {
    pub(crate) line: Line,
    pub(crate) piece: usize,
    pub(crate) offset: usize,
    pub(crate) work: bool,
}

/// One sequence and not two lists, because a note's whole meaning is where it
/// sits — see the module docs. Private, and it stays private: what a caller
/// writes is [`Thread::ask`] or [`Thread::note`], and what it reads back is rows
/// or [`Thread::turns`].
#[derive(Debug, Clone, PartialEq, Eq)]
enum Entry {
    Turn(Turn),
    Note { text: String, at: Instant },
}

impl Entry {
    const fn at(&self) -> Instant {
        match self {
            Self::Turn(turn) => turn.started(),
            Self::Note { at, .. } => *at,
        }
    }

    fn line_count(&self) -> usize {
        match self {
            Self::Turn(turn) => turn.line_count(),
            Self::Note { .. } => 1,
        }
    }

    /// The rows [`Entry::sourced`] makes, with the tags dropped: one flattening,
    /// so a row on screen and the row a cell is resolved against cannot come to
    /// disagree about what an entry draws.
    fn rows(&self, now: Instant) -> impl Iterator<Item = Line> + '_ {
        self.sourced(0, now).into_iter().map(|row| row.line)
    }

    /// A note is one row and one piece, whole: [`Thread::note`] flattened it on
    /// the way in, so the row is the piece rather than a line of it.
    fn sourced(&self, piece: usize, now: Instant) -> Vec<Sourced> {
        match self {
            Self::Turn(turn) => turn.sourced(piece, now),
            Self::Note { text, .. } => vec![Sourced {
                line: Line::Note { text: text.clone() },
                piece,
                offset: 0,
                work: false,
            }],
        }
    }

    fn pieces(&self) -> Vec<&str> {
        match self {
            Self::Turn(turn) => turn.pieces(),
            Self::Note { text, .. } => vec![text.as_str()],
        }
    }

    const fn turn(&self) -> Option<&Turn> {
        match self {
            Self::Turn(turn) => Some(turn),
            Self::Note { .. } => None,
        }
    }

    const fn turn_mut(&mut self) -> Option<&mut Turn> {
        match self {
            Self::Turn(turn) => Some(turn),
            Self::Note { .. } => None,
        }
    }
}

/// One session, one thread: warlock's chat is one conversation for the life of
/// the process, so turns are appended and nothing is ever dropped or trimmed.
/// [`ChatAgent`](crate::ChatAgent) holds the other half of that arrangement.
///
/// Driven by five calls, all of which take the instant they happened at:
/// [`Thread::ask`], [`Thread::record`], [`Thread::answer`] or [`Thread::end`],
/// and [`Thread::note`]. Read back with [`Thread::lines`] or [`Thread::window`],
/// which take the `now` the newest clock is measured against.
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
    entries: Vec<Entry>,
}

impl Thread {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    /// The clock under this question counts from `at`, and the turn is on screen
    /// from this call as the `waiting` placeholder rather than from whenever the
    /// model first says something. Whatever turn was live stops here: one
    /// question at a time is the rule the event loop keeps, and a turn still
    /// ticking under a newer one would be a second answer to that question.
    pub fn ask(&mut self, message: impl Into<String>, at: Instant) {
        self.freeze_last(at);
        self.entries.push(Entry::Turn(Turn {
            message: message.into(),
            log: Log::opened_at(at),
            answer: None,
            ending: None,
        }));
    }

    /// It touches no turn at all: nothing is opened, closed or frozen, so a note
    /// arriving while an answer is on its way leaves that turn live and
    /// [`Thread::in_flight`] saying what it said a moment before.
    ///
    /// Flattened to one row first, for [`Ending::line`]'s reason.
    pub fn note(&mut self, text: impl Into<String>, at: Instant) {
        let text = text.into();
        self.entries.push(Entry::Note {
            text: one_line(&text),
            at,
        });
    }

    /// A cost becomes nothing at all — not a line, not a total, not a field. See
    /// the module docs.
    ///
    /// Does nothing when there is no live turn. A line filed under a turn that
    /// is already over would contradict a line already on screen, so dropping it
    /// is the honest way to fail.
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

    /// An answer with nothing in it is not an answer: the turn ends with
    /// [`Ending::NothingSaid`] instead, because a question with nothing under it
    /// is indistinguishable from one still going and a reader would sit and wait
    /// for it.
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

    /// Whatever arrived before the ending stays exactly where it was, so a turn
    /// cancelled after two tool calls still shows those two tool calls. Does
    /// nothing once the newest turn is over: the first ending wins, and a
    /// failure reported twice is still one line.
    pub fn end(&mut self, ending: &Ending, at: Instant) {
        if let Some(turn) = self.live() {
            turn.word(ending, at);
        }
    }

    /// Borrowed turns rather than a slice, since the turns are no longer stored
    /// end to end: what is stored is the sequence of everything that happened,
    /// and this is the turns picked out of it.
    #[must_use]
    pub fn turns(&self) -> Vec<&Turn> {
        self.entries.iter().filter_map(Entry::turn).collect()
    }

    /// The addressable texts, in thread order: a turn's message, that turn's
    /// answer once there is one, and a note. Work rows are left out, because the
    /// live one's clock is recomputed every frame and no stable position can sit
    /// inside it.
    ///
    /// An answer follows its own message and nothing filed later, which is what
    /// keeps a [`Position`](crate::Position) taken while a turn was still
    /// running pointing at the same text once the answer lands. Reordering this
    /// moves every position taken before the answer arrived.
    #[must_use]
    pub fn pieces(&self) -> Vec<&str> {
        self.entries.iter().flat_map(Entry::pieces).collect()
    }

    /// [`Thread::lines`] again, row for row and in the same order, with each row
    /// tagged with the piece it was drawn from. Unwrapped, as `lines` is: the
    /// width is the panel's business, and the piece a row came from is the same
    /// whatever width it is broken at.
    #[must_use]
    pub(crate) fn sourced(&self, now: Instant) -> Vec<Sourced> {
        let mut piece = 0;
        let mut rows = Vec::new();
        for entry in &self.entries {
            rows.extend(entry.sourced(piece, now));
            piece += entry.pieces().len();
        }

        rows
    }

    /// A card with one note on it is not empty — a refusal before the first
    /// question is exactly that case.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The one instant a thread with rows in it can always name, whether it
    /// opens with a question or with a note.
    #[must_use]
    pub fn started(&self) -> Option<Instant> {
        self.entries.first().map(Entry::at)
    }

    /// Notes are stepped over — one landing under a question does not end it —
    /// so a caller can decide whether the composer is muted without keeping a
    /// second flag that could disagree with the thread.
    #[must_use]
    pub fn in_flight(&self) -> Option<&Turn> {
        self.last_turn().filter(|turn| !turn.is_closed())
    }

    fn last_turn(&self) -> Option<&Turn> {
        self.entries.iter().rev().find_map(Entry::turn)
    }

    fn last_turn_mut(&mut self) -> Option<&mut Turn> {
        self.entries.iter_mut().rev().find_map(Entry::turn_mut)
    }

    fn freeze_last(&mut self, at: Instant) {
        if let Some(previous) = self.last_turn_mut() {
            previous.freeze(at);
        }
    }

    fn live(&mut self) -> Option<&mut Turn> {
        self.last_turn_mut().and_then(Turn::live)
    }

    /// Before any wrapping, which is the card's business: the card is the one
    /// that knows the width.
    #[must_use]
    pub fn line_count(&self) -> usize {
        self.entries.iter().map(Entry::line_count).sum()
    }

    /// `now` is the caller's: this reads no clock, so the same thread and the
    /// same instant give the same rows every time.
    #[must_use]
    pub fn lines(&self, now: Instant) -> Vec<Line> {
        self.window(0, self.line_count(), now)
    }

    /// Asking for more rows than there are, or starting past the end, gives back
    /// what is there rather than failing: a viewport is a request, not an
    /// assertion about the conversation's length.
    #[must_use]
    pub fn window(&self, offset: usize, height: usize, now: Instant) -> Vec<Line> {
        self.entries
            .iter()
            .flat_map(move |entry| entry.rows(now))
            .skip(offset)
            .take(height)
            .collect()
    }
}

/// Never empty, for [`wrapped`](crate::wrap)'s reason: a blank line is a
/// paragraph break, and a message or an answer that came to no rows at all would
/// be a turn with a hole in it.
///
/// Each row carries where it starts in `text`, which is the only record of which
/// of the text's own bytes a row on screen is drawn from.
fn broken(text: &str) -> impl Iterator<Item = (usize, &str)> {
    let mut rows = Vec::new();
    let mut offset = 0;
    for line in text.lines() {
        rows.push((offset, line));
        offset += line.len();
        // The break itself belongs to no row: `lines` splits at `\n` and takes
        // the `\r` of a `\r\n` with it, so both have to be stepped over for the
        // next row's offset to be the text's own.
        if text[offset..].starts_with('\r') {
            offset += 1;
        }
        if text[offset..].starts_with('\n') {
            offset += 1;
        }
    }
    if rows.is_empty() {
        rows.push((0, ""));
    }

    rows.into_iter()
}

/// Nothing is cut. The account refuses to truncate for the same reason: the
/// width is a fact about a terminal, and this is not.
fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
#[path = "tests/thread.rs"]
mod tests;
