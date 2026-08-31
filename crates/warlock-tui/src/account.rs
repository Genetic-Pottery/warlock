//! What one pact did, in the order it did it.
//!
//! A subtree pact is minutes of work happening somewhere else, and the only
//! honest way to show that it is work rather than a hang is to say what it has
//! been doing since it started. This module is that account: an ordered list of
//! sections, one per directory the run reaches, each holding an ordered list of
//! lines, each line one thing a pass was seen doing. It is the state behind the
//! panel, and it is plain data — no terminal, no channel, no clock of its own.
//!
//! # The clock is the point
//!
//! Every line carries an elapsed time measured from the start of the section it
//! sits in, so a run reads as a sequence of passes rather than one undivided
//! stream, and a section that has just opened starts again at `0:00` instead of
//! carrying the whole run's total into a directory that has done nothing yet.
//!
//! The newest line's clock *moves*. A stretch of thinking is one event followed
//! by a minute of nothing at all, and a still screen is exactly what a hang
//! looks like; a line whose clock counts up is the difference between the two.
//! So the newest line's clock is a function of the `now` a caller hands in — the
//! event loop already redraws on a tick, and asking it for the instant on every
//! frame is all the ticking that is needed — and the moment a newer line arrives
//! beneath it, it freezes at the instant that newer line arrived. That is what
//! makes the number continuous: it counts up while the thing is happening and
//! stops at the value it had reached, rather than snapping backwards to the
//! moment the line was first printed.
//!
//! A section that has opened but heard nothing yet has no line to put a clock
//! on, and a bare heading is the still screen all over again — the first thing
//! a pass does is often a minute of silence before its first word arrives. So
//! an empty section shows one placeholder line, `waiting`, clocked like any
//! other: it ticks from the moment the section opens and is replaced by the
//! first real line the pass reports. It is drawn rather than stored, so nothing
//! has to arrive to create it and nothing has to be deleted when it goes.
//!
//! Nothing here reads a clock. [`Instant::now`] is never called in this file,
//! which is what lets a test drive a whole run off `base + Duration::from_secs`
//! and assert on exact text.
//!
//! # What is not here
//!
//! No file contents, no model prose, no tool results, no thought — [`Activity`]
//! has already made those choices and this module only words what it is given.
//! No tokens, no cache statistics, no turn counts, no API durations: the panel
//! is a freshness ledger, not a dashboard. And no truncation — a line is stored
//! whole and cut to the panel's width by whoever is drawing it, because the
//! width is a fact about a terminal and this is not.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::claude::Activity;

/// The whole of what a stretch of thinking is worth saying.
///
/// The bare fact, exactly as [`Activity::Thinking`] carries it: that thinking
/// happened, never what was thought.
pub(crate) const THINKING: &str = "thinking";

/// The placeholder line of a section that has heard nothing yet.
///
/// The pass is running and has not said anything — which, over a stream that
/// reports an assistant message only when it is whole, is most of the first
/// minute of every pass. The word is about warlock rather than the model on
/// purpose: `thinking` would claim to know what the silence is, and this file
/// only words what it is given.
pub(crate) const WAITING: &str = "waiting";

/// The whole of what "the pass is producing its answer" is worth saying.
///
/// The counterpart of [`THINKING`], and on a toolless pass the longer of the
/// two: a pass thinks for a few seconds and then spends the rest of its time
/// writing. The word is about the pass, not about the document — what it is
/// writing is the document, and the document is the outcome line's business.
///
/// The word alone is the start of the line rather than the whole of it: a pact
/// section adds how much of the answer has arrived, per [`writing_line`].
pub(crate) const WRITING: &str = "writing";

/// The whole of what "a big file is being read in pieces" is worth saying.
///
/// One word about warlock rather than about the model, like [`WAITING`]: a
/// summarising pass is warlock's own doing — a file over the per-file cap is
/// read in parts — and the file and the fraction are the only two things a
/// reader can act on. What the pass is *thinking* about that part is the
/// model's business and is not reported here.
const SUMMARISING: &str = "summarising";

/// One line of a section, and the instant it arrived.
///
/// The instant is kept even though a line's *displayed* clock is usually the one
/// belonging to the line beneath it: a line's arrival is what freezes the line
/// above, so every line has to remember its own.
///
/// Private, because the account decides what a line says. Handing these out
/// would be a second place the wording could be decided.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Entry {
    /// When this line arrived, in the caller's clock.
    at: Instant,
    /// What it says, whole and untruncated.
    text: String,
}

/// A run of clocked lines under one heading, and the whole rule their clocks
/// follow.
///
/// Everything the module docs above say about the ticking clock lives in here:
/// where a line's number is measured from, which line is the one that moves,
/// what freezes it, and what an empty one shows in the meantime. It is the
/// mechanism rather than the meaning — it holds no directory, no outcome and no
/// money, and it words nothing except the [`WAITING`] placeholder, which is
/// about the silence rather than about whatever is being waited for.
///
/// Crate-private and shared, because there are two things in warlock that are a
/// list of things a model was seen doing with a clock on each: a [`Section`] of
/// a pact's account, and a turn of the panel's thread. They differ in what
/// surrounds the lines, not in how the lines tick, and a second copy of this
/// rule would be a second clock to keep in step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Log {
    /// When this stretch of work began, which is where its clocks count from.
    started: Instant,
    /// The lines, in arrival order.
    entries: Vec<Entry>,
    /// When this stopped being the live one, if it has. A frozen log's last
    /// line stops here instead of ticking.
    closed: Option<Instant>,
}

impl Log {
    /// A log of work that began at `at` and has heard nothing yet.
    pub(crate) const fn opened_at(at: Instant) -> Self {
        Self {
            started: at,
            entries: Vec::new(),
            closed: None,
        }
    }

    /// When this stretch of work began.
    pub(crate) const fn started(&self) -> Instant {
        self.started
    }

    /// When it stopped moving, or `None` while it is still the live one.
    pub(crate) const fn closed_at(&self) -> Option<Instant> {
        self.closed
    }

    /// Whether it has stopped moving.
    pub(crate) const fn is_closed(&self) -> bool {
        self.closed.is_some()
    }

    /// How many rows it draws as: one per line, and one for the [`WAITING`]
    /// placeholder where there are none.
    ///
    /// Counted here as well as drawn, because this number is what a scroll
    /// offset is clamped against: a row that is drawn but not counted would be
    /// one the panel can never scroll to the edge of.
    pub(crate) fn row_count(&self) -> usize {
        self.entries.len().max(1)
    }

    /// Stop this log moving as of `at`, if it has not stopped already.
    ///
    /// Idempotent on purpose: work can be frozen by its own ending, by the next
    /// stretch starting, or by the run ending, and whichever happens first is
    /// the honest instant to freeze at.
    pub(crate) fn freeze(&mut self, at: Instant) {
        self.closed.get_or_insert(at);
    }

    /// File `text` as a line of its own, arriving at `at`.
    pub(crate) fn push(&mut self, text: impl Into<String>, at: Instant) {
        self.entries.push(Entry {
            at,
            text: text.into(),
        });
    }

    /// Let the stretch of `text` go on if it is already the newest line, or
    /// open one for it at `at` if it is not.
    ///
    /// What makes a repeated report read as one continuing thing rather than as
    /// a column of identical lines: the entry that is already there keeps the
    /// instant it opened at, so its clock counts the whole stretch rather than
    /// restarting on every report the stream happens to send.
    pub(crate) fn extend_or_open(&mut self, text: &str, at: Instant) {
        if self.entries.last().is_none_or(|entry| entry.text != text) {
            self.push(text, at);
        }
    }

    /// Reword the newest line to `text` if it is already a line of the `word`
    /// stretch, or open one for it at `at` if it is not.
    ///
    /// [`Log::extend_or_open`] for a stretch whose wording *changes* while it
    /// goes on: a running count of what has arrived so far. Comparing whole
    /// texts, as that one does, would read every new count as a new stretch and
    /// leave a column of lines one byte apart, each with a clock starting from
    /// nothing; comparing the word in front of the count reads them as the one
    /// thing they are. The entry that is already there keeps the instant it
    /// opened at — so its clock still counts the whole stretch, and the line
    /// above it, frozen by this line's arrival, does not re-freeze later.
    ///
    /// A line belongs to the stretch when it is the bare `word` or the `word`
    /// followed by this file's ` · ` separator, which is exactly the shape
    /// [`writing_line`] gives its two cases and no shape any other line in the
    /// panel has. So a stretch that ends and begins again — anything else
    /// having been filed in between — opens a fresh line and counts its own
    /// bytes from its own instant, by the same rule that gives thinking one
    /// line per stretch.
    pub(crate) fn rewrite_or_open(&mut self, word: &str, text: &str, at: Instant) {
        match self.entries.last_mut() {
            Some(entry) if continues(&entry.text, word) => text.clone_into(&mut entry.text),
            _ => self.push(text, at),
        }
    }

    /// Every line as a drawable row, with clocks measured against `now`.
    ///
    /// A log with nothing in it yields the [`WAITING`] placeholder, clocked as
    /// entry zero, which gives it the right instant by the ordinary rule: no
    /// entry follows it, so it ticks with `now` while the log is live and
    /// freezes where the log froze. The first real entry takes its place — same
    /// rule, nothing special to remove.
    pub(crate) fn rows(&self, now: Instant) -> impl Iterator<Item = Line> + '_ {
        let waiting = self.entries.is_empty().then(|| Line::Clocked {
            clock: self.clock(0, now),
            text: WAITING.to_owned(),
        });
        waiting
            .into_iter()
            .chain(
                self.entries
                    .iter()
                    .enumerate()
                    .map(move |(index, entry)| Line::Clocked {
                        clock: self.clock(index, now),
                        text: entry.text.clone(),
                    }),
            )
    }

    /// The instant the line at `index` shows on its clock, against a caller's
    /// `now`.
    ///
    /// The whole rule of the ticking clock, in one expression. A line with
    /// another beneath it shows the instant that one arrived, so it reads as how
    /// long the thing took. The last line of a live log shows `now`, so it
    /// counts up between events. The last line of a frozen log shows the instant
    /// it froze, so it stops.
    fn shown_at(&self, index: usize, now: Instant) -> Instant {
        self.entries
            .get(index + 1)
            .map_or_else(|| self.closed.unwrap_or(now), |next| next.at)
    }

    /// The clock text for the line at `index`, against a caller's `now`.
    fn clock(&self, index: usize, now: Instant) -> String {
        clock(
            self.shown_at(index, now)
                .saturating_duration_since(self.started),
        )
    }
}

/// How a directory's pass ended, in the words it ends its section with.
///
/// Three ways a pass stops and no fourth: it wrote the document, it was refused,
/// or somebody stopped the run. The cost is not in here — it arrives over the
/// activity port during the pass, is accumulated by the section, and is put into
/// the wording at the moment the section closes, so a caller closing a section
/// never has to know what the pass spent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// The pass wrote a document.
    Wrote {
        /// The document that was written, named however the caller names it —
        /// relative to the tree on screen for preference, since an absolute path
        /// spends the line on the part the reader already knows.
        document: PathBuf,
        /// How big it is, in bytes. Bytes rather than lines or words because
        /// bytes are what a caller can stat without reading the file, and this
        /// module refuses to hold file contents.
        bytes: u64,
    },
    /// The pass was refused, and this is why.
    Refused {
        /// The reason, carried out of whatever refused rather than invented
        /// here.
        reason: String,
    },
    /// The run was stopped while this directory was being worked.
    ///
    /// Carries nothing: what a reader wants to know is what it had spent by
    /// then, and the section already knows that.
    Cancelled,
}

impl Outcome {
    /// The line this outcome closes a section with, given what that section
    /// cost.
    ///
    /// `cost` is `None` when no cost ever arrived for the pass, which is said
    /// out loud rather than printed as `$0.00`: a pass that reported nothing and
    /// a pass that was free are different facts, and only one of them is good
    /// news.
    fn line(&self, cost: Option<f64>) -> String {
        match self {
            Self::Wrote { document, bytes } => {
                let document = document.display();
                let cost = spend(cost);
                format!("wrote {document} — {bytes} bytes, {cost}")
            }
            Self::Refused { reason } => format!("refused — {reason}"),
            Self::Cancelled => {
                let cost = cost.map_or_else(
                    || "nothing reported spent".to_owned(),
                    |usd| format!("{} spent", money(usd)),
                );
                format!("cancelled — {cost}")
            }
        }
    }
}

/// One directory's pass: what it is, when it started, what it has been seen
/// doing, and what it cost.
///
/// Opened by [`Account::open_section`] and closed by
/// [`Account::close_section`]. Between the two it is the live section, which is
/// the one whose newest line ticks; a section that has been closed — or that has
/// had another opened beneath it — is frozen and never moves again, however long
/// the run goes on.
#[derive(Debug, Clone, PartialEq)]
pub struct Section {
    /// The directory being worked, named however the caller named it.
    directory: PathBuf,
    /// When this section opened, what has been filed under it in arrival order
    /// — with the outcome line, if it was closed with one, last — and whether
    /// its clock is still moving.
    log: Log,
    /// What the pass reported spending, summed over however many times it said
    /// so. `None` means it never said, which is not the same as zero.
    cost: Option<f64>,
    /// Whether the outcome line is already under it.
    ///
    /// Not the same question as [`Section::is_closed`], and the difference is
    /// the whole reason both are here: a section stops moving the moment the
    /// next directory opens, which is long before the run says how that
    /// directory went. So a section spends most of a run frozen and unworded,
    /// and this is what [`Account::close_open_sections`] reads to find the ones
    /// still owed an ending.
    has_outcome: bool,
}

impl Section {
    /// The directory this section is about.
    #[must_use]
    pub fn directory(&self) -> &Path {
        &self.directory
    }

    /// What this pass reported spending, or `None` if it never reported.
    ///
    /// `None` and `Some(0.0)` are deliberately different answers: see
    /// [`Account::finish`], where the difference decides whether the run's total
    /// is a total or an understatement.
    #[must_use]
    pub const fn cost(&self) -> Option<f64> {
        self.cost
    }

    /// Whether this section has stopped moving.
    ///
    /// True once it has been closed with an outcome, once another section has
    /// opened beneath it, or once the run has ended — all three of which mean
    /// the same thing to a reader: nothing further will appear here.
    #[must_use]
    pub const fn is_closed(&self) -> bool {
        self.log.is_closed()
    }

    /// How many drawable rows this section is: its heading plus its lines,
    /// where a section with no lines yet draws the one [`WAITING`] placeholder.
    fn line_count(&self) -> usize {
        self.log.row_count() + 1
    }

    /// Put `outcome`'s line under this section, and stop it moving.
    ///
    /// The outcome line is a line like any other and takes a clock like any
    /// other, so a reader can see how long the pass took as well as how it went.
    /// The instant it takes is the instant this section *stopped* rather than
    /// the instant somebody got round to wording it: a section frozen when the
    /// next directory opened ended there, and dating its last line at the end of
    /// the whole run would say that a pass which took thirty seconds took nine
    /// minutes. Only a section still live when it is worded takes `at`.
    ///
    /// Does nothing to a section that has an outcome already. The first ending
    /// wins, because it is the one already on screen.
    fn word(&mut self, outcome: &Outcome, at: Instant) {
        if self.has_outcome {
            return;
        }
        let at = self.log.closed_at().unwrap_or(at);
        self.log.push(outcome.line(self.cost), at);
        self.has_outcome = true;
        self.log.freeze(at);
    }
}

/// One drawable row of the panel: of an account, or of a file somebody asked to
/// read.
///
/// Flat rather than nested, because the panel is a list and scrolling it is
/// counting: a section heading takes a row like anything else, so the number of
/// rows above and below a window is arithmetic rather than a walk.
///
/// The panel's cards — the account, the thread and the document — draw one card
/// at a time and draw it the same way, so the rows of all three are this one
/// type and the window rule over them is one rule, whichever card the reader is
/// looking at. [`Line::Text`] is the document's only shape, because a file's
/// line is a file's line and nothing here knows what any of it means.
///
/// The text is whole. Cutting it to a width belongs to whoever knows the width.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Line {
    /// A section heading, naming the directory whose pass follows.
    Directory {
        /// The directory, as the caller named it.
        path: PathBuf,
    },
    /// One thing that happened under a heading — an activity, or the outcome the
    /// section closed with — and the elapsed time it shows.
    Clocked {
        /// Elapsed since that section started, as `m:ss`. The newest line of the
        /// live section is the only one of these that ever changes.
        clock: String,
        /// What happened, in the fewest words it can be said in.
        text: String,
    },
    /// The one line the whole run ends with.
    Summary {
        /// Directories, wall clock and money — see [`Account::finish`].
        text: String,
    },
    /// One line of a file somebody asked to read, or the one line said about
    /// that reading — that the file was cut at the cap, which is the only thing
    /// ever added to a document.
    ///
    /// No clock: a file did not happen at a time. No path either — which file
    /// this is was answered by the keystroke that asked for it, and repeating it
    /// on every row would spend the panel's width saying the same thing.
    Text {
        /// The line, exactly as the file has it, or the sentence about the cut.
        text: String,
    },
    /// What the reader typed: the message one turn of the thread was asked in,
    /// in their own words.
    ///
    /// Its own variant rather than a [`Line::Text`] with a marker in front of
    /// it, because who said a line is a fact about the line and not a character
    /// somebody decided to store: the renderer is the one place that knows how
    /// the reader's half of a conversation should look, and a `>` baked in here
    /// would be a second answer to that question sitting in the value.
    ///
    /// No clock — a question is not a thing that took time; the work under it is
    /// what has a clock — and no name in front of it, because the panel has one
    /// reader and one model and there is nobody else it could have been. Only a
    /// thread ever yields one.
    Said {
        /// The message, exactly as it was typed, and wrapped by whoever knows
        /// the width.
        text: String,
    },
    /// What warlock itself has to say, in one line of its own voice.
    ///
    /// The third party on the thread's card. A [`Line::Said`] is the reader's
    /// words and a [`Line::Text`] is the model's answer; this is neither — it is
    /// the program saying something about the conversation rather than taking
    /// part in it, which is what a refused command, a file written or a warning
    /// about a stale document is. Only a [`Thread`](crate::Thread) ever yields
    /// one: nothing an [`Account`] holds is warlock talking.
    ///
    /// Unclocked, like [`Line::Said`] and unlike [`Line::Clocked`]: a note is
    /// not work that took time, it is one thing said at one moment, and a `0:00`
    /// beside it would claim a pass had started. It draws distinctly from both
    /// of its neighbours — its own marker rather than the question's, and not
    /// bold — so warlock's own voice is not read as something the model did or
    /// something the reader typed. Which marker that is belongs to the renderer,
    /// for the reason written out at [`Line::Said`]: what is put in front of a
    /// line is not a character stored in it.
    ///
    /// One line, whole. A note that had paragraphs in it would be prose, and
    /// prose on this card is the model's.
    Note {
        /// What warlock says, wrapped by whoever knows the width.
        text: String,
    },
    /// One row of a line too long to draw in one — the whole of it that did not
    /// fit on the row above, already carrying whatever indent keeps it under the
    /// text it continues.
    ///
    /// Nothing here ever makes one: an [`Account`] and a [`Thread`](crate::Thread)
    /// hold what happened, and how many rows that takes is a question about a
    /// terminal. They are made on the way to the screen, by whoever knows the
    /// width (see [`mod@crate::wrap`]), which is why a panel dragged narrower
    /// re-flows the line a reader is looking at rather than cutting its tail
    /// off.
    ///
    /// A line that fits is itself and never one of these, and a line that does
    /// not keeps its own variant on its first row wherever that variant can hold
    /// a piece of it — a question keeps its marker, a clocked line keeps its
    /// clock — so what a row *is* still reads off the value.
    Wrapped {
        /// The row, composed: the prefix on the first row of a broken line, or
        /// blanks the width of it on the rows after, and then the text.
        text: String,
        /// Whether the line this continues is drawn bold — a heading, a summary
        /// or a question — so that one line broken over two rows is not bold on
        /// one of them and plain on the other.
        heading: bool,
    },
}

/// Everything one pact did, from the key press to the summary line.
///
/// One pact, one account: a second run starts a new one rather than appending,
/// because a log is a thing you have to search to find the current thing in.
/// Nothing here is discarded or trimmed while the run it describes is the
/// current one — a finished account stays whole so it can be read afterwards.
///
/// Built by four calls, all of which take the instant they happened at:
/// [`Account::new`] when the run starts, [`Account::open_section`] as each
/// directory comes up, [`Account::record`] for every activity the pass reports,
/// and [`Account::close_section`], [`Account::close_open_sections`] and
/// [`Account::finish`] at the ends. Read back
/// as rows with [`Account::lines`] or [`Account::window`], which take the `now`
/// the newest clock is measured against.
///
/// Holds an `f64` cost, so it is [`PartialEq`] and not [`Eq`] — as is everything
/// that comes to hold one of these.
///
/// ```
/// use std::time::{Duration, Instant};
///
/// use warlock_tui::{Account, Activity, Line};
///
/// let base = Instant::now();
/// let mut account = Account::new(base);
///
/// account.open_section("crates/engine", base);
/// account.record(&Activity::Thinking, base + Duration::from_secs(1));
///
/// // Nothing newer has arrived, so the newest line counts up with `now`.
/// assert_eq!(
///     account.lines(base + Duration::from_secs(9)),
///     vec![
///         Line::Directory { path: "crates/engine".into() },
///         Line::Clocked { clock: "0:09".to_owned(), text: "thinking".to_owned() },
///     ],
/// );
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct Account {
    /// When the run started, which is what the summary's duration counts from.
    started: Instant,
    /// The sections, in the order the run reached them.
    sections: Vec<Section>,
    /// The run's closing line, once it has one.
    summary: Option<String>,
}

impl Account {
    /// An account of a run that has just started at `at` and done nothing yet.
    ///
    /// Empty means empty: no sections, no lines, nothing to draw. The panel
    /// showing nothing at all before the first pact is the same fact said one
    /// level up, where the app has no account rather than an empty one.
    #[must_use]
    pub const fn new(at: Instant) -> Self {
        Self {
            started: at,
            sections: Vec::new(),
            summary: None,
        }
    }

    /// Open a section for `directory` at `at`, and freeze the one above it.
    ///
    /// The clock under this heading counts from `at`, so every directory starts
    /// again at `0:00` — and it is on screen from this call, as the [`WAITING`]
    /// placeholder, rather than from whenever the pass first says something.
    /// Whatever section was live stops there and then — the pass that was
    /// running it is over, whether or not the caller closed it with an outcome,
    /// so its newest line has no business still counting up.
    pub fn open_section(&mut self, directory: impl Into<PathBuf>, at: Instant) {
        if let Some(previous) = self.sections.last_mut() {
            previous.log.freeze(at);
        }
        self.sections.push(Section {
            directory: directory.into(),
            log: Log::opened_at(at),
            cost: None,
            has_outcome: false,
        });
    }

    /// Record what the live pass was seen doing at `at`.
    ///
    /// A tool becomes its name plus the one detail [`Activity`] chose to carry,
    /// or its bare name where there is none. Thinking becomes the word
    /// `thinking` and nothing else. A cost becomes no line at all: it is a fact
    /// about the pass rather than a thing the pass did, so it is added to this
    /// section's spend and appears in the outcome line and the summary instead.
    ///
    /// Does nothing when there is no live section — before the first directory
    /// opens, or after the current one has been closed. A line cannot be filed
    /// under a section that has been worded and frozen without contradicting a
    /// line already on screen, and dropping it is the honest way to fail.
    pub fn record(&mut self, activity: &Activity, at: Instant) {
        let Some(section) = self.sections.last_mut() else {
            return;
        };
        if section.is_closed() {
            return;
        }

        match activity {
            Activity::Cost { usd } => *section.cost.get_or_insert(0.0) += usd,
            // One line per stretch, however many times the stream says the
            // stretch is still going. A pass reports thinking every few seconds
            // while it thinks, and appending each would fill the panel with a
            // column of identical words; keeping the first means the line that
            // is already there goes on ticking, and its clock — measured from
            // when the stretch *started*, which is what this keeps — is the
            // count of how long it has been at it. A stretch that ends and
            // begins again opens a new line, because something else will have
            // been filed in between.
            Activity::Thinking => section.log.extend_or_open(THINKING, at),
            // Writing is the same one-line-per-stretch rule, except that its
            // line is reworded as the answer arrives rather than left alone:
            // the count is the whole point of it, and the entry it rewrites is
            // the one already on screen, instant and all.
            Activity::Writing { bytes } => {
                section
                    .log
                    .rewrite_or_open(WRITING, &writing_line(*bytes), at);
            }
            Activity::Tool { name, detail } => {
                section.log.push(tool_line(name, detail.as_ref()), at);
            }
        }
    }

    /// Record that a summarising pass over `file` — part `part` of `parts` —
    /// started at `at`.
    ///
    /// A file over the per-file cap is read in pieces, and for a directory
    /// holding one those pieces are most of the wait: a run can spend eight
    /// minutes on them behind a single `writing` line that never moves. One
    /// line per pass is what turns that into something a reader can watch.
    ///
    /// The line names the file and the fraction and nothing else — no estimate,
    /// no percentage of its own, no time remaining. The fraction is the honest
    /// half of a progress bar and the rest would be a guess; the clock every
    /// line already carries says how long this part has been going.
    ///
    /// `file` is spelled however the caller spells it — relative to the tree on
    /// screen for preference, as with [`Outcome::Wrote`] — because which tree a
    /// path should be read against is not a fact this module holds.
    ///
    /// Not an [`Activity`]: activities are what the claude stream reports, and
    /// the stream knows nothing about how warlock chose to feed it a file.
    ///
    /// Filed as an ordinary [`Entry`], so it draws, clocks, freezes and counts
    /// exactly like every other line: the line above freezes at `at` and this
    /// one ticks until something newer arrives. Does nothing when there is no
    /// live section, or when the newest one is frozen — the same silence
    /// [`Account::record`] keeps, and for the same reason.
    pub fn record_summarising(
        &mut self,
        file: impl AsRef<Path>,
        part: usize,
        parts: usize,
        at: Instant,
    ) {
        let Some(section) = self.sections.last_mut() else {
            return;
        };
        if section.is_closed() {
            return;
        }

        // Pushed rather than run through `Log::extend_or_open`, which the
        // repeated-report activities use: the two differ only for an event that
        // repeats *identically*, and every pass carries its own `part`, so the
        // ticket is one line per pass either way. Pushing says that outright
        // instead of leaving a reader to work out that the texts never collide.
        let file = file.as_ref().display();
        section
            .log
            .push(format!("{SUMMARISING} {file} ({part}/{parts})"), at);
    }

    /// Record that the live directory's request — `files` files, `bytes` bytes
    /// of them — was handed to a pass at `at`.
    ///
    /// One line, `waiting · 11 files, 1.6 MB`, and it is filed at the handover:
    /// the pass has the request and has said nothing about it yet, and this is
    /// the silence a reader spends most of a directory looking at. The two
    /// numbers are the only thing in warlock that can explain why one directory
    /// is slower than the next, and both are already known at the handover —
    /// nothing is measured for this line.
    ///
    /// # Why this is an entry and not the placeholder
    ///
    /// The drawn [`WAITING`] placeholder is only there while the section has
    /// heard nothing at all, so it can say nothing about a directory that
    /// summarised first — and it carries no numbers, because it is about the
    /// silence rather than about whatever is being waited for. An entry does
    /// both. It is also filed at the handover rather than at the section
    /// opening, so the stretch it covers, from its arrival to whatever arrives
    /// next, is the pass's own silence and not the disk read that came before
    /// it; the placeholder covers that earlier stretch and stops being drawn
    /// once this lands, nothing having been stored for it and nothing deleted.
    /// Its clock is the module's ordinary one — elapsed since the section
    /// opened, moving until something newer arrives.
    ///
    /// On a directory that summarised first there is no placeholder to replace,
    /// and this lands below those lines exactly as it would land below any
    /// other.
    ///
    /// Its text is deliberately not the bare [`WAITING`] constant, so
    /// [`Log::extend_or_open`] cannot fold it into the line above or a later
    /// one into it.
    ///
    /// # What the two numbers count
    ///
    /// `files` is how many files the request carries; `bytes` is everything it
    /// carries counted the way the budget counts it, which includes each child
    /// directory's document. The two do not cover the same set, and that is the
    /// caller's arithmetic rather than this module's — `bytes` is what was sent,
    /// and it is the number the caps are checked against.
    ///
    /// Pushed rather than extended, and silent when there is no live section or
    /// the newest one is frozen, for the reasons
    /// [`Account::record_summarising`] gives.
    pub fn record_waiting(&mut self, files: usize, bytes: u64, at: Instant) {
        let Some(section) = self.sections.last_mut() else {
            return;
        };
        if section.is_closed() {
            return;
        }

        let files = plural(files, "file", "files");
        section
            .log
            .push(format!("{WAITING} · {files}, {}", size(bytes)), at);
    }

    /// Stop whatever section is still live moving as of `at`, without ending
    /// the run.
    ///
    /// The half of [`Account::finish`] that is about clocks rather than about
    /// money: nothing is worded, no summary is written, and a run frozen this
    /// way can still be looked at — it simply stops counting up. Idempotent for
    /// [`Log::freeze`]'s reason, so freezing a run that has already stopped
    /// leaves its last line where it stopped.
    ///
    /// Crate-private, because the caller is `finish` — a run saying what it came
    /// to — and because stopping a run's clocks from anywhere else would be
    /// somebody other than the run deciding it is over.
    pub(crate) fn freeze(&mut self, at: Instant) {
        if let Some(section) = self.sections.last_mut() {
            section.log.freeze(at);
        }
    }

    /// Close the newest section at `at` with the line `outcome` makes.
    ///
    /// The outcome line is a line like any other and takes a clock like any
    /// other, so a reader can see how long the pass took as well as how it went.
    /// Closing also stops the section moving: this is the instant its last
    /// activity line freezes at.
    ///
    /// The newest section rather than any section, because it is the one a
    /// caller can name without naming it — what stopped is what was running.
    /// Every other section is closed by [`Account::close_open_sections`], which
    /// is where a run that only learns per-directory outcomes at the end does
    /// its wording.
    ///
    /// Does nothing when there is no section to close, or when the newest one
    /// has an outcome already.
    pub fn close_section(&mut self, outcome: &Outcome, at: Instant) {
        if let Some(section) = self.sections.last_mut() {
            section.word(outcome, at);
        }
    }

    /// Close every section still owed an outcome, wording each with what
    /// `outcome` says about it.
    ///
    /// A run does not report itself a directory at a time. The pass for one
    /// directory is over the moment the next one starts, but *how* it went
    /// arrives once, at the end, in a list of failures naming the directories
    /// they are about — so at the end of a run every section is frozen and none
    /// of them has an ending, and this is where they get one. Each is worded at
    /// the instant it stopped rather than at `at`, so the clocks say how long
    /// the passes took; `at` is what the section still live at the end takes.
    ///
    /// The outcome is a caller's judgement, per section, rather than anything
    /// worked out here: what a directory's pass wrote, how big it is and why one
    /// was refused are all facts about a filesystem and a run, and this module
    /// holds neither.
    ///
    /// Sections closed already — the one a cancel worded, say — keep the ending
    /// they have, and `outcome` is never asked about them.
    pub fn close_open_sections(
        &mut self,
        at: Instant,
        mut outcome: impl FnMut(&Section) -> Outcome,
    ) {
        for index in 0..self.sections.len() {
            if self.sections[index].has_outcome {
                continue;
            }
            let ending = outcome(&self.sections[index]);
            self.sections[index].word(&ending, at);
        }
    }

    /// End the run at `at` with the one line that describes the whole of it.
    ///
    /// Directories, wall clock and money: `pact finished — 9 directories, 4:12,
    /// $1.87`. The duration is measured from [`Account::new`] rather than from
    /// the first section, because a run starts when the key is pressed.
    ///
    /// A pass whose cost never arrived is not worth zero. Where any section is
    /// missing one, the line says so — `$1.65 (incomplete: 2 passes reported no
    /// cost)` — rather than quietly under-reporting a number somebody is going to
    /// take at face value.
    ///
    /// Freezes whatever section was still live, since nothing is running any
    /// more.
    pub fn finish(&mut self, at: Instant) {
        self.freeze(at);

        let directories = plural(self.sections.len(), "directory", "directories");
        let elapsed = clock(at.saturating_duration_since(self.started));
        let spent = money(self.spent());
        let unpriced = self.unpriced();
        let total = if unpriced == 0 {
            spent
        } else {
            let passes = plural(unpriced, "pass", "passes");
            format!("{spent} (incomplete: {passes} reported no cost)")
        };

        self.summary = Some(format!("pact finished — {directories}, {elapsed}, {total}"));
    }

    /// The sections, in the order the run reached them.
    #[must_use]
    pub fn sections(&self) -> &[Section] {
        &self.sections
    }

    /// When the run started, which is what its summary's duration counts from.
    ///
    /// Crate-private, and there for one reason: a run drawn into the thread is
    /// a turn, and every turn of a thread has to be able to say when it began —
    /// see [`Turn::started`](crate::Turn). A run began when the key was pressed,
    /// which is the instant [`Account::new`] took.
    pub(crate) const fn started(&self) -> Instant {
        self.started
    }

    /// When the section that is still being worked started, or `None` when none
    /// is.
    ///
    /// The open section is the last one, and only while it is un-frozen: a
    /// section stops being live the moment the next one opens or the run ends
    /// (see [`Section::is_closed`]), so a finished run has no open section even
    /// though its last section still remembers when it started.
    ///
    /// This is the one instant a caller needs to say how long the directory
    /// being worked right now has been going — a renderer measuring `now -
    /// started` for something that has to move while the pass runs. It is an
    /// instant and not a duration on purpose: this file reads no clock, and
    /// handing back a duration would mean picking a `now` here rather than
    /// taking the caller's.
    #[must_use]
    pub fn open_section_started(&self) -> Option<Instant> {
        self.sections
            .last()
            .filter(|section| !section.is_closed())
            .map(|section| section.log.started())
    }

    /// How many rows the whole account draws as.
    ///
    /// A heading per section, a line per thing that happened under it, and the
    /// summary once there is one. What a scroll offset is clamped against, and
    /// what a "how far below the view am I" count is taken from.
    #[must_use]
    pub fn line_count(&self) -> usize {
        self.sections.iter().map(Section::line_count).sum::<usize>()
            + usize::from(self.summary.is_some())
    }

    /// Every row of the account, with clocks measured against `now`.
    ///
    /// `now` is the caller's: this reads no clock, so the same account and the
    /// same instant give the same rows every time. Only the newest line of a
    /// live section depends on it.
    #[must_use]
    pub fn lines(&self, now: Instant) -> Vec<Line> {
        self.window(0, self.line_count(), now)
    }

    /// The `height` rows starting at `offset`, with clocks measured against
    /// `now`.
    ///
    /// What a panel actually draws. Asking for more rows than there are, or
    /// starting past the end, gives back what is there rather than failing:
    /// a viewport is a request, not an assertion about the account's length.
    #[must_use]
    pub fn window(&self, offset: usize, height: usize, now: Instant) -> Vec<Line> {
        self.rows(now).skip(offset).take(height).collect()
    }

    /// Every row, lazily, so a window costs only the rows it takes.
    ///
    /// A heading and then whatever its [`Log`] draws as, which for a section
    /// that has heard nothing yet is the [`WAITING`] placeholder: a pass that
    /// has not said anything still has a clock on screen counting up from the
    /// moment its section opened.
    ///
    /// Crate-private rather than private, because the panel reads a run's rows
    /// out of exactly this iterator: one function words what a run did, so
    /// there is no second spelling of a directory heading or an outcome line
    /// anywhere in warlock to keep in step with this one.
    pub(crate) fn rows(&self, now: Instant) -> impl Iterator<Item = Line> + '_ {
        self.sections
            .iter()
            .flat_map(move |section| {
                std::iter::once(Line::Directory {
                    path: section.directory.clone(),
                })
                .chain(section.log.rows(now))
            })
            .chain(
                self.summary
                    .iter()
                    .map(|text| Line::Summary { text: text.clone() }),
            )
    }

    /// What the run is known to have spent: the sections that reported, summed.
    ///
    /// Folded from a positive zero rather than summed, because [`f64`]'s own
    /// [`Sum`](std::iter::Sum) starts at `-0.0` to keep signed zeroes exact and
    /// a run that spent nothing would print as `$-0.00`.
    fn spent(&self) -> f64 {
        self.sections
            .iter()
            .filter_map(Section::cost)
            .fold(0.0, |total, usd| total + usd)
    }

    /// How many sections never reported a cost, which is how many the total is
    /// short by.
    fn unpriced(&self) -> usize {
        self.sections
            .iter()
            .filter(|section| section.cost.is_none())
            .count()
    }
}

/// An elapsed span as the panel spells it: `m:ss`, counting from `0:00`.
///
/// Minutes are not padded and are allowed to grow past sixty rather than
/// rolling into an hours field: a pact is minutes of work, `73:04` is
/// unambiguous, and an `h:mm:ss` that appears once an hour in is a second format
/// for a reader to parse.
///
/// Sub-second precision is dropped, not rounded. A clock that reads `0:04` and
/// then `0:04` again is a clock; one that jumps forward when a pass is nearly at
/// the next second is a distraction.
fn clock(elapsed: Duration) -> String {
    let seconds = elapsed.as_secs();
    format!("{}:{:02}", seconds / 60, seconds % 60)
}

/// A cost as the panel spells it: two decimal places behind a dollar sign.
///
/// Two places because that is what money looks like, even where a pass costs
/// less than a cent — `$0.00` for a pass that reported almost nothing is a
/// truer statement about the run's total than four decimal places of noise on
/// every line.
///
/// The one place a number becomes money in this crate. What a *line* then says
/// around it is the caller's, and the panel's two ledgers deliberately say very
/// different things: see [`Outcome::line`] and [`Turn`](crate::Turn).
pub(crate) fn money(usd: f64) -> String {
    format!("${usd:.2}")
}

/// A tool call as one line: its name, and the one detail [`Activity`] chose to
/// carry, or its bare name where there is none.
///
/// Shared with the thread, because a `Grep` is a `Grep` whichever card it turns
/// up on and a reader who has learnt to read one of them has learnt to read
/// both. What is *not* shared is anything around it — a pact's line sits under
/// a directory and a turn's under a question.
pub(crate) fn tool_line(name: &str, detail: Option<&String>) -> String {
    detail.map_or_else(|| name.to_owned(), |detail| format!("{name} {detail}"))
}

/// The [`WRITING`] line as it stands, given how much of the answer has arrived.
///
/// `writing · 1.8 KB`, and `writing · 934 bytes` under a kilobyte, per
/// [`size`]. A count and nothing else: no denominator, because nothing knows
/// how long the answer will be until it ends; no percentage and no bar, which
/// are that denominator wearing a hat; no spinner, because the clock the line
/// already carries is the honest thing that moves. What it says is what has
/// happened, and a reader watching it climb knows the pass is alive without
/// being told a guess about when it will stop.
///
/// Zero bytes reads as the bare word. That is the case the block's opening
/// reports, before a single delta has landed, and the bare word is what the
/// panel has always shown at the first token; `writing · 0 bytes` would be a
/// count of nothing, worded as though something had been measured.
fn writing_line(bytes: u64) -> String {
    if bytes == 0 {
        WRITING.to_owned()
    } else {
        format!("{WRITING} · {}", size(bytes))
    }
}

/// Whether `text` is a line of the `word` stretch: the bare word, or the word
/// in front of a ` · ` detail.
///
/// The half of [`Log::rewrite_or_open`] that decides what "the same stretch"
/// means. Split out so the shape it looks for sits next to [`writing_line`],
/// which is what produces it — the two have to agree, and agreeing at a
/// distance is how they would stop.
fn continues(text: &str, word: &str) -> bool {
    text.strip_prefix(word)
        .is_some_and(|rest| rest.is_empty() || rest.starts_with(" · "))
}

/// What an outcome line says about a pass's cost, including when there is none.
///
/// A pass that never reported is said in words rather than as a number, because
/// every number here would be a lie about a thing that was never measured.
fn spend(cost: Option<f64>) -> String {
    cost.map_or_else(|| "no cost reported".to_owned(), money)
}

/// A number of bytes as the panel spells it: `934 bytes`, `1.8 KB`, `403 KB`,
/// `1.6 MB`, `12 MB`.
///
/// Base 1024, spelled `KB` rather than `KiB`, which is the spelling the panel
/// is specified in. The engine's own prose writes `KiB` for the same
/// arithmetic, so the two disagree in spelling while agreeing in the number;
/// settling that is not this module's call.
///
/// # The tiers
///
/// Under a kilobyte the count is exact, because a small request is a fact worth
/// stating precisely and `0.9 KB` says less than `934 bytes` does. Above it,
/// one decimal while the number is under ten — `1.8 KB` and `1.6 MB`, where the
/// first digit alone would throw away most of what the reader wanted — and
/// whole units from ten up, where that digit is noise on a number nobody reads
/// to three significant figures.
///
/// The unit is chosen by magnitude first and the rounding happens inside it, so
/// nothing is ever promoted across a boundary by being rounded: 1048575 bytes
/// is `1024 KB` and not `1.0 MB`, which would claim the request had reached a
/// megabyte when it had not. `MB` is the largest unit, since the request cap is
/// measured in megabytes and a `GB` here would be a unit for a number that
/// cannot occur.
///
/// Integer arithmetic throughout: these are exact counts, and the halfway cases
/// are the ones the tests pin.
///
/// Public because it is the panel's spelling of a size and there is now more
/// than one line that needs it — the account's request lines, and the line the
/// thread says when `/write` puts a file on disk. Shared rather than copied: two
/// formatters would be two spellings the day either of them changed.
#[must_use]
pub fn size(bytes: u64) -> String {
    /// One kilobyte, as this file counts them.
    const KB: u64 = 1024;
    /// One megabyte, likewise.
    const MB: u64 = KB * KB;

    if bytes < KB {
        // Worded here rather than through `plural`, which counts in `usize`: a
        // byte total is the one count in the panel that is not a number of
        // things held in memory.
        let noun = if bytes == 1 { "byte" } else { "bytes" };
        format!("{bytes} {noun}")
    } else if bytes < MB {
        scaled(bytes, KB, "KB")
    } else {
        scaled(bytes, MB, "MB")
    }
}

/// `bytes` in units of `unit`, spelled with the digits that unit deserves.
///
/// The half of [`size`] that is arithmetic rather than choice: one decimal
/// below ten of the unit, whole units at ten and above, rounding half up in
/// both. The multiplication by ten only happens on the tenths branch, where
/// the value is under ten units and so nowhere near overflowing.
fn scaled(bytes: u64, unit: u64, name: &str) -> String {
    if bytes < 10 * unit {
        let tenths = (bytes * 10 + unit / 2) / unit;
        format!("{}.{} {name}", tenths / 10, tenths % 10)
    } else {
        format!("{} {name}", (bytes + unit / 2) / unit)
    }
}

/// `count` with the right noun after it: `1 directory`, `9 directories`.
///
/// English, in the one place these lines need it, so no line has to be worded
/// twice or read as `1 directories`.
fn plural(count: usize, one: &str, many: &str) -> String {
    let noun = if count == 1 { one } else { many };
    format!("{count} {noun}")
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::time::{Duration, Instant};

    use super::{Account, Line, Outcome, Section, clock, size};
    use crate::claude::Activity;

    /// The instant `seconds` after `base`, so a whole run can be driven without
    /// anything ever reading a clock.
    fn at(base: Instant, seconds: u64) -> Instant {
        base + Duration::from_secs(seconds)
    }

    /// The activity a pass reports for a tool call with one detail worth saying.
    fn tool(name: &str, detail: &str) -> Activity {
        Activity::Tool {
            name: name.to_owned(),
            detail: Some(detail.to_owned()),
        }
    }

    /// What a run's rows say, as plain text, for the tests that care about the
    /// wording rather than the shape.
    fn said(account: &Account, now: Instant) -> Vec<String> {
        account
            .lines(now)
            .into_iter()
            .map(|line| match line {
                Line::Directory { path } => path.display().to_string(),
                Line::Clocked { clock, text } => format!("{clock} {text}"),
                // An account never yields a document's line, a question or a
                // note; they are here so this helper words every row of the
                // panel and not most of them. Nor a wrapped row: an account
                // holds what happened, and how many rows that takes is the
                // panel's question.
                Line::Summary { text }
                | Line::Text { text }
                | Line::Said { text }
                | Line::Note { text }
                | Line::Wrapped { text, .. } => text,
            })
            .collect()
    }

    #[test]
    fn a_run_that_has_done_nothing_draws_nothing() {
        let base = Instant::now();
        let account = Account::new(base);

        assert_eq!(account.line_count(), 0);
        assert_eq!(account.lines(at(base, 30)), Vec::new());
        assert!(account.sections().is_empty());
    }

    #[test]
    fn every_section_starts_its_clock_again_at_zero() {
        let base = Instant::now();
        let mut account = Account::new(base);

        account.open_section("crates/engine", at(base, 10));
        account.record(&Activity::Thinking, at(base, 12));
        account.open_section("crates/tui", at(base, 70));
        account.record(&Activity::Thinking, at(base, 71));

        // The first section's line froze when the second section opened, at one
        // minute in; the second's counts from its own start, not the run's.
        assert_eq!(
            said(&account, at(base, 75)),
            vec![
                "crates/engine".to_owned(),
                "1:00 thinking".to_owned(),
                "crates/tui".to_owned(),
                "0:05 thinking".to_owned(),
            ],
        );
    }

    #[test]
    fn a_section_that_has_heard_nothing_ticks_waiting_until_the_pass_speaks() {
        let base = Instant::now();
        let mut account = Account::new(base);

        account.open_section("crates/engine", base);

        // On screen from the moment the section opens: the clock is already
        // moving with nothing recorded, which is what says the silence is a
        // pass and not a hang.
        assert_eq!(account.line_count(), 2);
        assert_eq!(
            said(&account, at(base, 12)),
            vec!["crates/engine".to_owned(), "0:12 waiting".to_owned()],
        );

        // The first real line takes its place, on the same clock: the pass
        // was quiet for fifty-eight seconds and then it was thinking.
        account.record(&Activity::Thinking, at(base, 58));
        assert_eq!(
            said(&account, at(base, 60)),
            vec!["crates/engine".to_owned(), "1:00 thinking".to_owned()],
        );
    }

    #[test]
    fn a_stretch_of_thinking_is_one_line_however_often_it_is_reported() {
        let base = Instant::now();
        let mut account = Account::new(base);

        account.open_section("crates/engine", base);
        // The stream says "still thinking" every few seconds; the panel says it
        // once, and lets the clock do the rest.
        for second in [2, 5, 9, 40] {
            account.record(&Activity::Thinking, at(base, second));
        }

        assert_eq!(
            said(&account, at(base, 55)),
            vec!["crates/engine".to_owned(), "0:55 thinking".to_owned()],
            "one line, counting from when thinking started",
        );

        // Something else happening ends the stretch, and thinking after it is a
        // new one — the two are separated by what came between them.
        account.record(&tool("Read", "src/lib.rs"), at(base, 60));
        account.record(&Activity::Thinking, at(base, 61));
        account.record(&Activity::Thinking, at(base, 65));

        assert_eq!(
            said(&account, at(base, 70)),
            vec![
                "crates/engine".to_owned(),
                "1:00 thinking".to_owned(),
                "1:01 Read src/lib.rs".to_owned(),
                "1:10 thinking".to_owned(),
            ],
        );
    }

    #[test]
    fn a_pass_reads_as_thinking_and_then_as_writing() {
        // The whole of a toolless pass, which is what a pact runs now: a few
        // seconds of thinking, then the rest of the time producing the
        // document. Two lines, each clocked from when its own stretch began, so
        // the panel says which half the wait is being spent in.
        let base = Instant::now();
        let mut account = Account::new(base);

        account.open_section("crates/engine", base);
        account.record(&Activity::Thinking, at(base, 2));
        account.record(&Activity::Thinking, at(base, 3));
        account.record(&Activity::Writing { bytes: 0 }, at(base, 4));
        account.record(&Activity::Writing { bytes: 0 }, at(base, 9));

        assert_eq!(
            said(&account, at(base, 25)),
            vec![
                "crates/engine".to_owned(),
                // Thinking froze when writing began, four seconds in.
                "0:04 thinking".to_owned(),
                // And writing is the live line, counting from when it started.
                "0:25 writing".to_owned(),
            ],
        );
    }

    #[test]
    fn the_writing_line_is_reworded_as_the_answer_arrives() {
        // The long half of a toolless pass, with the count on it: one line,
        // reworded where it stands, so the panel moves while the answer is
        // being produced instead of holding one motionless word for minutes.
        let base = Instant::now();
        let mut account = Account::new(base);

        account.open_section("crates/engine", base);
        account.record(&Activity::Thinking, at(base, 2));
        // The text block opening, before a single delta: the bare word, which
        // is what the panel showed at the first token before there was a count
        // to show.
        account.record(&Activity::Writing { bytes: 0 }, at(base, 8));
        assert_eq!(
            said(&account, at(base, 8)),
            vec![
                "crates/engine".to_owned(),
                "0:08 thinking".to_owned(),
                "0:08 writing".to_owned(),
            ],
        );

        for (bytes, second) in [(212, 20), (1_843, 60), (48_000, 200)] {
            account.record(&Activity::Writing { bytes }, at(base, second));
        }

        assert_eq!(
            said(&account, at(base, 260)),
            vec![
                "crates/engine".to_owned(),
                // Thinking still froze when writing began, and nothing since
                // has moved it: the rewrites are not arrivals.
                "0:08 thinking".to_owned(),
                // One line, the newest count, and a clock counting from the
                // first report rather than from the last one.
                "4:20 writing · 47 KB".to_owned(),
            ],
        );
    }

    #[test]
    fn a_writing_line_under_a_kilobyte_counts_in_bytes() {
        // The short answer, worded exactly rather than rounded to `0.9 KB`.
        let base = Instant::now();
        let mut account = Account::new(base);

        account.open_section("crates/engine", base);
        account.record(&Activity::Writing { bytes: 0 }, at(base, 3));
        account.record(&Activity::Writing { bytes: 934 }, at(base, 5));

        assert_eq!(said(&account, at(base, 6))[1], "0:06 writing · 934 bytes");
    }

    #[test]
    fn writing_after_something_else_opens_a_line_with_its_own_count() {
        // A stretch that ends and begins again — a tool call between two text
        // blocks, and a summarising pass between two directories' worth of
        // work — counts its own block's bytes from its own instant, rather
        // than continuing the earlier line's total.
        let base = Instant::now();
        let mut account = Account::new(base);

        account.open_section("crates/engine", base);
        account.record(&Activity::Writing { bytes: 2_048 }, at(base, 10));
        account.record(&tool("Read", "src/lib.rs"), at(base, 30));
        account.record(&Activity::Writing { bytes: 512 }, at(base, 40));
        account.record_summarising("src/big.rs", 1, 3, at(base, 50));
        account.record(&Activity::Writing { bytes: 100 }, at(base, 60));

        assert_eq!(
            said(&account, at(base, 90)),
            vec![
                "crates/engine".to_owned(),
                "0:30 writing · 2.0 KB".to_owned(),
                "0:40 Read src/lib.rs".to_owned(),
                "0:50 writing · 512 bytes".to_owned(),
                "1:00 summarising src/big.rs (1/3)".to_owned(),
                "1:30 writing · 100 bytes".to_owned(),
            ],
            "each stretch its own line, its own count and its own clock",
        );
    }

    #[test]
    fn a_waiting_line_freezes_with_the_section_it_waits_in() {
        let base = Instant::now();
        let mut account = Account::new(base);

        account.open_section("crates/engine", base);
        account.open_section("crates/tui", at(base, 30));

        // The first section froze when the second opened, still with nothing
        // heard, so its waiting line stops where the pass stopped — however
        // late the frame is drawn.
        for now in [at(base, 30), at(base, 900)] {
            assert_eq!(said(&account, now)[1], "0:30 waiting");
        }
        // While the live section's own waiting line goes on ticking.
        assert_eq!(said(&account, at(base, 45))[3], "0:15 waiting");
    }

    #[test]
    fn the_newest_line_counts_up_with_the_now_it_is_asked_about() {
        let base = Instant::now();
        let mut account = Account::new(base);

        account.open_section("crates/engine", base);
        account.record(&Activity::Thinking, at(base, 1));

        let ticking: Vec<_> = [1, 9, 65, 3_600]
            .into_iter()
            .map(|seconds| said(&account, at(base, seconds)).join("|"))
            .collect();

        assert_eq!(
            ticking,
            vec![
                "crates/engine|0:01 thinking".to_owned(),
                "crates/engine|0:09 thinking".to_owned(),
                "crates/engine|1:05 thinking".to_owned(),
                "crates/engine|60:00 thinking".to_owned(),
            ],
        );
    }

    #[test]
    fn a_line_freezes_at_the_instant_the_line_beneath_it_arrived() {
        let base = Instant::now();
        let mut account = Account::new(base);

        account.open_section("crates/engine", base);
        account.record(&Activity::Thinking, at(base, 2));
        account.record(&tool("Read", "src/lib.rs"), at(base, 20));

        // The thinking line stopped where it had got to — 0:20, where the read
        // arrived — and stays there however late the frame is drawn.
        for now in [at(base, 20), at(base, 40), at(base, 900)] {
            assert_eq!(said(&account, now)[1], "0:20 thinking");
        }

        // While the line beneath it, being the newest, goes on ticking.
        assert_eq!(said(&account, at(base, 40))[2], "0:40 Read src/lib.rs");
        assert_eq!(said(&account, at(base, 41))[2], "0:41 Read src/lib.rs");
    }

    #[test]
    fn a_closed_section_stops_ticking_for_good() {
        let base = Instant::now();
        let mut account = Account::new(base);

        account.open_section("crates/engine", base);
        account.record(&Activity::Thinking, at(base, 2));
        account.close_section(
            &Outcome::Wrote {
                document: "crates/engine/WARLOCK.md".into(),
                bytes: 2_341,
            },
            at(base, 30),
        );

        let early = said(&account, at(base, 30));
        assert_eq!(early, said(&account, at(base, 4_000)));
        assert_eq!(early[1], "0:30 thinking");
        assert!(early[2].starts_with("0:30 wrote"));
        assert!(account.sections()[0].is_closed());
    }

    #[test]
    fn ending_a_run_freezes_the_section_that_was_still_live() {
        let base = Instant::now();
        let mut account = Account::new(base);

        account.open_section("crates/engine", base);
        account.record(&Activity::Thinking, at(base, 1));
        account.finish(at(base, 12));

        assert_eq!(said(&account, at(base, 999))[1], "0:12 thinking");
    }

    #[test]
    fn a_tool_is_its_name_and_the_one_detail_it_carries() {
        let base = Instant::now();
        let mut account = Account::new(base);

        account.open_section("crates/engine", base);
        account.record(&tool("Read", "src/lib.rs"), at(base, 1));
        account.record(
            &Activity::Tool {
                name: "WebFetch".to_owned(),
                detail: None,
            },
            at(base, 2),
        );
        account.record(&Activity::Thinking, at(base, 3));

        assert_eq!(
            said(&account, at(base, 3)),
            vec![
                "crates/engine".to_owned(),
                "0:02 Read src/lib.rs".to_owned(),
                "0:03 WebFetch".to_owned(),
                "0:03 thinking".to_owned(),
            ],
        );
    }

    #[test]
    fn a_summarising_pass_is_one_line_naming_the_file_and_its_fraction() {
        let base = Instant::now();
        let mut account = Account::new(base);

        // The shape of a directory holding one file over the cap: a little
        // thinking, then pass after pass over the pieces of that file, then the
        // document. Each pass is its own line, in arrival order among the rest.
        account.open_section("crates/engine", base);
        account.record(&Activity::Thinking, at(base, 2));
        account.record_summarising("crates/engine/Cargo.lock", 1, 3, at(base, 10));
        account.record_summarising("crates/engine/Cargo.lock", 2, 3, at(base, 70));
        account.record(&tool("Read", "src/lib.rs"), at(base, 130));
        account.record_summarising("crates/engine/Cargo.lock", 3, 3, at(base, 131));
        account.record(&Activity::Writing { bytes: 0 }, at(base, 190));

        assert_eq!(
            said(&account, at(base, 200)),
            vec![
                "crates/engine".to_owned(),
                "0:10 thinking".to_owned(),
                "1:10 summarising crates/engine/Cargo.lock (1/3)".to_owned(),
                "2:10 summarising crates/engine/Cargo.lock (2/3)".to_owned(),
                "2:11 Read src/lib.rs".to_owned(),
                "3:10 summarising crates/engine/Cargo.lock (3/3)".to_owned(),
                "3:20 writing".to_owned(),
            ],
        );
        // Counted as well as drawn: a heading and six lines.
        assert_eq!(account.line_count(), 7);
    }

    #[test]
    fn a_run_of_summarising_passes_is_a_line_apiece_however_long_it_goes_on() {
        let base = Instant::now();
        let mut account = Account::new(base);

        // Fourteen passes at half a minute each: the wait that draws as one
        // motionless line today draws as fourteen, and none of them coalesces
        // into the one above it.
        account.open_section("crates/engine", base);
        for part in 1..=14 {
            account.record_summarising("Cargo.lock", part, 14, at(base, part as u64 * 30));
        }

        let rows = said(&account, at(base, 500));
        assert_eq!(rows.len(), 15);
        assert_eq!(rows[1], "1:00 summarising Cargo.lock (1/14)");
        assert_eq!(rows[13], "7:00 summarising Cargo.lock (13/14)");
        // The newest is the only one still moving.
        assert_eq!(rows[14], "8:20 summarising Cargo.lock (14/14)");
        assert_eq!(
            said(&account, at(base, 900))[14],
            "15:00 summarising Cargo.lock (14/14)",
        );
    }

    #[test]
    fn a_summarising_line_freezes_and_ticks_like_any_other() {
        let base = Instant::now();
        let mut account = Account::new(base);

        account.open_section("crates/engine", base);
        account.record(&Activity::Thinking, at(base, 1));

        // Its arrival freezes the line above it, at the instant it arrived.
        account.record_summarising("Cargo.lock", 1, 2, at(base, 20));
        for now in [at(base, 20), at(base, 60), at(base, 900)] {
            assert_eq!(said(&account, now)[1], "0:20 thinking");
        }

        // And, being the newest, it counts up until something newer lands.
        assert_eq!(
            said(&account, at(base, 40))[2],
            "0:40 summarising Cargo.lock (1/2)"
        );
        assert_eq!(
            said(&account, at(base, 41))[2],
            "0:41 summarising Cargo.lock (1/2)"
        );
        account.record(&Activity::Writing { bytes: 0 }, at(base, 50));
        for now in [at(base, 50), at(base, 900)] {
            assert_eq!(said(&account, now)[2], "0:50 summarising Cargo.lock (1/2)");
        }

        // A section frozen by the next one opening stops with a summarising
        // line under it exactly as it stops with anything else.
        account.open_section("crates/tui", at(base, 80));
        assert_eq!(said(&account, at(base, 4_000))[3], "1:20 writing");
    }

    #[test]
    fn no_summarising_line_is_filed_where_there_is_no_live_section() {
        let base = Instant::now();
        let mut account = Account::new(base);

        // Before the first directory.
        account.record_summarising("Cargo.lock", 1, 2, at(base, 1));
        assert_eq!(account.line_count(), 0);

        // And after the current one has been worded and frozen: swallowed
        // whole, exactly as an activity is.
        account.open_section("crates/engine", at(base, 2));
        account.close_section(&Outcome::Cancelled, at(base, 3));
        account.record_summarising("Cargo.lock", 1, 2, at(base, 4));

        assert_eq!(
            said(&account, at(base, 9)),
            vec![
                "crates/engine".to_owned(),
                "0:01 cancelled — nothing reported spent".to_owned(),
            ],
        );
    }

    #[test]
    fn a_size_is_spelled_with_the_digits_its_magnitude_deserves() {
        // Exact under a kilobyte, one decimal under ten of a unit, whole units
        // above that.
        assert_eq!(size(0), "0 bytes");
        assert_eq!(size(1), "1 byte");
        assert_eq!(size(934), "934 bytes");
        assert_eq!(size(1_843), "1.8 KB");
        assert_eq!(size(34 * 1024), "34 KB");
        assert_eq!(size(403 * 1024), "403 KB");
        assert_eq!(size(1_677_722), "1.6 MB");
        assert_eq!(size(12 * 1024 * 1024), "12 MB");
    }

    #[test]
    fn a_size_never_rounds_itself_across_a_boundary() {
        // The unit is chosen by magnitude and the rounding happens inside it,
        // so no number ever claims to have reached the next unit or the next
        // tier by being rounded into it.
        assert_eq!(size(1_023), "1023 bytes");
        assert_eq!(size(1_024), "1.0 KB");
        assert_eq!(size(10_239), "10.0 KB", "one decimal still, not 10 KB");
        assert_eq!(size(10_240), "10 KB");
        assert_eq!(size(1_048_575), "1024 KB", "not 1.0 MB");
        assert_eq!(size(1_048_576), "1.0 MB");
    }

    #[test]
    fn the_handed_over_request_replaces_the_waiting_placeholder() {
        let base = Instant::now();
        let mut account = Account::new(base);

        // The section opens when the directory comes up, and the placeholder
        // covers reading it off disk: one second of ticking with nothing filed.
        account.open_section("crates/warlock-tui/src", base);
        assert_eq!(
            said(&account, at(base, 1)),
            vec![
                "crates/warlock-tui/src".to_owned(),
                "0:01 waiting".to_owned(),
            ],
        );

        // Then the request is handed over, and the wait that follows is the
        // pass's own rather than the disk's. The placeholder is drawn rather
        // than stored, so it is not left above the entry: there is an entry now,
        // and `entries.is_empty()` being false is the whole of what stops it
        // being drawn. Nothing was stored for it and nothing was deleted — the
        // account is the same two rows it always was.
        account.record_waiting(11, 1_677_722, at(base, 1));
        assert_eq!(
            said(&account, at(base, 60)),
            vec![
                "crates/warlock-tui/src".to_owned(),
                "1:00 waiting · 11 files, 1.6 MB".to_owned(),
            ],
            "one row, the entry, ticking by the ordinary rule",
        );
        assert_eq!(account.line_count(), 2);

        // And the stretch this line covers is the pass's silence: it froze when
        // the pass first spoke, a minute after the handover a second in.
        account.record(&Activity::Thinking, at(base, 61));
        assert_eq!(
            said(&account, at(base, 900))[1],
            "1:01 waiting · 11 files, 1.6 MB",
        );
    }

    #[test]
    fn a_handed_over_request_lands_below_the_summarising_passes_before_it() {
        let base = Instant::now();
        let mut account = Account::new(base);

        // A directory holding a file over the cap: the summarising passes speak
        // first, so the placeholder never appears at all, and the request line
        // lands under them like any other line.
        account.open_section("crates/engine", base);
        account.record_summarising("crates/engine/Cargo.lock", 1, 2, at(base, 10));
        account.record_summarising("crates/engine/Cargo.lock", 2, 2, at(base, 70));
        account.record_waiting(11, 34 * 1024, at(base, 130));
        account.record(&Activity::Thinking, at(base, 190));

        assert_eq!(
            said(&account, at(base, 200)),
            vec![
                "crates/engine".to_owned(),
                "1:10 summarising crates/engine/Cargo.lock (1/2)".to_owned(),
                "2:10 summarising crates/engine/Cargo.lock (2/2)".to_owned(),
                // Frozen where thinking began: the wait for this pass's first
                // word was a minute.
                "3:10 waiting · 11 files, 34 KB".to_owned(),
                "3:20 thinking".to_owned(),
            ],
        );
    }

    #[test]
    fn a_handed_over_request_is_collapsed_into_nothing_and_swallows_nothing() {
        let base = Instant::now();
        let mut account = Account::new(base);

        // Its text differs from the bare `waiting` of the placeholder and from
        // whatever follows it, so `extend_or_open` has nothing to fold: the
        // repeated report after it opens its own line, and a second request
        // line would too.
        account.open_section("crates/engine", base);
        account.record_waiting(1, 934, at(base, 1));
        account.record(&Activity::Thinking, at(base, 2));
        account.record(&Activity::Thinking, at(base, 3));

        assert_eq!(
            said(&account, at(base, 10)),
            vec![
                "crates/engine".to_owned(),
                // One file, said as one file.
                "0:02 waiting · 1 file, 934 bytes".to_owned(),
                "0:10 thinking".to_owned(),
            ],
        );
    }

    #[test]
    fn no_request_line_is_filed_where_there_is_no_live_section() {
        let base = Instant::now();
        let mut account = Account::new(base);

        // Before the first directory, and after the current one has been worded
        // and frozen: the same silence `record` and `record_summarising` keep.
        account.record_waiting(11, 34 * 1024, at(base, 1));
        assert_eq!(account.line_count(), 0);

        account.open_section("crates/engine", at(base, 2));
        account.close_section(&Outcome::Cancelled, at(base, 3));
        account.record_waiting(11, 34 * 1024, at(base, 4));

        assert_eq!(
            said(&account, at(base, 9)),
            vec![
                "crates/engine".to_owned(),
                "0:01 cancelled — nothing reported spent".to_owned(),
            ],
        );
    }

    #[test]
    fn a_cost_is_arithmetic_rather_than_a_line() {
        let base = Instant::now();
        let mut account = Account::new(base);

        account.open_section("crates/engine", base);
        account.record(&Activity::Cost { usd: 0.21 }, at(base, 1));
        account.record(&Activity::Cost { usd: 0.04 }, at(base, 2));

        // No line of its own: the section still reads as waiting for the pass
        // to be seen doing something, and the money was kept.
        assert_eq!(account.line_count(), 2);
        assert_eq!(
            said(&account, at(base, 2)),
            vec!["crates/engine".to_owned(), "0:02 waiting".to_owned()]
        );
        assert_eq!(account.sections()[0].cost(), Some(0.25));
    }

    #[test]
    fn nothing_is_filed_where_there_is_no_live_section() {
        let base = Instant::now();
        let mut account = Account::new(base);

        // Before the first directory.
        account.record(&Activity::Thinking, at(base, 1));
        assert_eq!(account.line_count(), 0);

        // And after the current one has been worded and frozen.
        account.open_section("crates/engine", at(base, 2));
        account.close_section(&Outcome::Cancelled, at(base, 3));
        account.record(&Activity::Thinking, at(base, 4));
        account.record(&Activity::Cost { usd: 9.99 }, at(base, 5));
        account.close_section(&Outcome::Cancelled, at(base, 6));

        assert_eq!(
            said(&account, at(base, 7)),
            vec![
                "crates/engine".to_owned(),
                "0:01 cancelled — nothing reported spent".to_owned(),
            ],
        );
        assert_eq!(account.sections()[0].cost(), None);
    }

    #[test]
    fn a_written_document_says_what_it_wrote_how_big_and_what_it_cost() {
        let base = Instant::now();
        let mut account = Account::new(base);

        account.open_section("crates/engine", base);
        account.record(&Activity::Cost { usd: 0.2149 }, at(base, 1));
        account.close_section(
            &Outcome::Wrote {
                document: "crates/engine/WARLOCK.md".into(),
                bytes: 2_341,
            },
            at(base, 42),
        );

        assert_eq!(
            said(&account, at(base, 42))[1],
            "0:42 wrote crates/engine/WARLOCK.md — 2341 bytes, $0.21",
        );
    }

    #[test]
    fn a_written_document_whose_pass_never_priced_itself_says_so() {
        let base = Instant::now();
        let mut account = Account::new(base);

        account.open_section("crates/engine", base);
        account.close_section(
            &Outcome::Wrote {
                document: "crates/engine/WARLOCK.md".into(),
                bytes: 12,
            },
            at(base, 5),
        );

        assert_eq!(
            said(&account, at(base, 5))[1],
            "0:05 wrote crates/engine/WARLOCK.md — 12 bytes, no cost reported",
        );
    }

    #[test]
    fn a_refusal_says_why_it_was_refused() {
        let base = Instant::now();
        let mut account = Account::new(base);

        account.open_section("crates/engine", base);
        account.close_section(
            &Outcome::Refused {
                reason: "the model returned an empty document".to_owned(),
            },
            at(base, 7),
        );

        assert_eq!(
            said(&account, at(base, 7))[1],
            "0:07 refused — the model returned an empty document",
        );
    }

    #[test]
    fn a_cancellation_says_what_it_had_spent_by_then() {
        let base = Instant::now();
        let mut account = Account::new(base);

        account.open_section("crates/engine", base);
        account.record(&Activity::Cost { usd: 0.03 }, at(base, 1));
        account.close_section(&Outcome::Cancelled, at(base, 65));

        assert_eq!(
            said(&account, at(base, 65))[1],
            "1:05 cancelled — $0.03 spent"
        );
    }

    #[test]
    fn the_summary_counts_directories_wall_clock_and_money() {
        let base = Instant::now();
        let mut account = Account::new(base);

        for (index, directory) in ["crates/engine", "crates/tui"].into_iter().enumerate() {
            let opened = at(base, index as u64 * 60);
            account.open_section(directory, opened);
            account.record(&Activity::Cost { usd: 0.935 }, opened);
            account.close_section(
                &Outcome::Wrote {
                    document: "WARLOCK.md".into(),
                    bytes: 10,
                },
                opened + Duration::from_secs(30),
            );
        }
        account.finish(at(base, 252));

        assert_eq!(
            said(&account, at(base, 252)).last().map(String::as_str),
            Some("pact finished — 2 directories, 4:12, $1.87"),
        );
    }

    #[test]
    fn one_directory_is_said_in_the_singular() {
        let base = Instant::now();
        let mut account = Account::new(base);

        account.open_section("crates/engine", base);
        account.record(&Activity::Cost { usd: 1.0 }, at(base, 1));
        account.finish(at(base, 9));

        assert_eq!(
            said(&account, at(base, 9)).last().map(String::as_str),
            Some("pact finished — 1 directory, 0:09, $1.00"),
        );
    }

    #[test]
    fn a_pass_that_never_reported_a_cost_makes_the_total_incomplete() {
        let base = Instant::now();
        let mut account = Account::new(base);

        account.open_section("crates/engine", base);
        account.record(&Activity::Cost { usd: 1.65 }, at(base, 1));
        account.open_section("crates/tui", at(base, 2));
        account.open_section("crates/cli", at(base, 3));
        account.finish(at(base, 60));

        // Two passes said nothing, so the number is a floor rather than a total,
        // and the line says which.
        assert_eq!(
            said(&account, at(base, 60)).last().map(String::as_str),
            Some(
                "pact finished — 3 directories, 1:00, $1.65 (incomplete: 2 passes reported no cost)"
            ),
        );
    }

    #[test]
    fn one_pass_missing_a_cost_is_said_in_the_singular_too() {
        let base = Instant::now();
        let mut account = Account::new(base);

        account.open_section("crates/engine", base);
        account.finish(at(base, 1));

        assert_eq!(
            said(&account, at(base, 1)).last().map(String::as_str),
            Some("pact finished — 1 directory, 0:01, $0.00 (incomplete: 1 pass reported no cost)"),
        );
    }

    #[test]
    fn every_section_is_closed_at_the_end_with_what_is_said_about_it() {
        let base = Instant::now();
        let mut account = Account::new(base);

        // Two passes, one after the other, with nothing closing either of them
        // at the time: how a directory went is not known until the run ends.
        account.open_section("crates/engine", base);
        account.record(&Activity::Thinking, at(base, 1));
        account.open_section("crates/tui", at(base, 30));
        account.record(&Activity::Thinking, at(base, 31));

        account.close_open_sections(at(base, 90), |section| {
            if section.directory() == Path::new("crates/engine") {
                Outcome::Wrote {
                    document: "crates/engine/WARLOCK.md".into(),
                    bytes: 2_341,
                }
            } else {
                Outcome::Refused {
                    reason: "the model returned an empty document".to_owned(),
                }
            }
        });

        // The first section's ending is dated where that pass stopped — thirty
        // seconds in, where the next directory opened — and the second's, being
        // the one still live, is dated now.
        assert_eq!(
            said(&account, at(base, 900)),
            vec![
                "crates/engine".to_owned(),
                "0:30 thinking".to_owned(),
                "0:30 wrote crates/engine/WARLOCK.md — 2341 bytes, no cost reported".to_owned(),
                "crates/tui".to_owned(),
                "1:00 thinking".to_owned(),
                "1:00 refused — the model returned an empty document".to_owned(),
            ],
        );
        assert!(account.sections().iter().all(Section::is_closed));
    }

    #[test]
    fn a_section_that_has_an_ending_already_is_not_asked_for_a_second_one() {
        let base = Instant::now();
        let mut account = Account::new(base);

        // How a cancelled run is worded: the section it was stopped in is
        // closed on the spot, and the ones above it are closed with what the
        // run went on to say about them.
        account.open_section("crates/engine", base);
        account.record(&Activity::Cost { usd: 0.21 }, at(base, 1));
        account.open_section("crates/tui", at(base, 10));
        account.record(&Activity::Cost { usd: 0.03 }, at(base, 11));
        account.close_section(&Outcome::Cancelled, at(base, 20));

        let mut asked = Vec::new();
        account.close_open_sections(at(base, 20), |section| {
            asked.push(section.directory().display().to_string());
            Outcome::Wrote {
                document: "WARLOCK.md".into(),
                bytes: 12,
            }
        });

        assert_eq!(asked, vec!["crates/engine".to_owned()]);
        assert_eq!(
            said(&account, at(base, 20)),
            vec![
                "crates/engine".to_owned(),
                "0:10 wrote WARLOCK.md — 12 bytes, $0.21".to_owned(),
                "crates/tui".to_owned(),
                "0:10 cancelled — $0.03 spent".to_owned(),
            ],
        );
    }

    #[test]
    fn a_window_is_the_slice_of_rows_the_panel_asked_for() {
        let base = Instant::now();
        let mut account = Account::new(base);

        account.open_section("crates/engine", base);
        account.record(&Activity::Thinking, at(base, 1));
        account.record(&tool("Read", "src/lib.rs"), at(base, 2));
        account.open_section("crates/tui", at(base, 3));
        account.record(&Activity::Thinking, at(base, 4));
        account.finish(at(base, 5));

        let now = at(base, 5);
        assert_eq!(account.line_count(), 6);
        assert_eq!(account.window(0, 6, now), account.lines(now));
        assert_eq!(
            account.window(2, 2, now),
            vec![
                Line::Clocked {
                    clock: "0:03".to_owned(),
                    text: "Read src/lib.rs".to_owned(),
                },
                Line::Directory {
                    path: "crates/tui".into(),
                },
            ],
        );

        // Asking for more than there is, or starting past the end, is a request
        // rather than a claim about the account's length.
        assert_eq!(account.window(5, 99, now).len(), 1);
        assert!(account.window(6, 99, now).is_empty());
    }

    #[test]
    fn the_clock_pads_seconds_and_lets_minutes_grow() {
        assert_eq!(clock(Duration::ZERO), "0:00");
        assert_eq!(clock(Duration::from_millis(1_999)), "0:01");
        assert_eq!(clock(Duration::from_secs(59)), "0:59");
        assert_eq!(clock(Duration::from_mins(1)), "1:00");
        assert_eq!(clock(Duration::from_secs(252)), "4:12");
        assert_eq!(clock(Duration::from_secs(4_384)), "73:04");
    }

    #[test]
    fn a_now_from_before_a_section_started_reads_as_zero_rather_than_panicking() {
        let base = Instant::now();
        let mut account = Account::new(at(base, 10));

        account.open_section("crates/engine", at(base, 10));
        account.record(&Activity::Thinking, at(base, 11));

        assert_eq!(said(&account, base)[1], "0:00 thinking");

        account.finish(base);
        assert_eq!(
            said(&account, base).last().map(String::as_str),
            Some("pact finished — 1 directory, 0:00, $0.00 (incomplete: 1 pass reported no cost)"),
        );
    }

    #[test]
    fn the_open_section_hands_back_the_instant_it_started() {
        let base = Instant::now();
        let mut account = Account::new(base);

        // Nothing has opened yet, so there is nothing being worked.
        assert_eq!(account.open_section_started(), None);

        account.open_section("crates/engine", at(base, 10));
        assert_eq!(account.open_section_started(), Some(at(base, 10)));

        // Lines under it do not move the start: the section began when it began.
        account.record(&Activity::Thinking, at(base, 12));
        assert_eq!(account.open_section_started(), Some(at(base, 10)));
    }

    #[test]
    fn the_start_moves_to_the_second_section_the_moment_it_opens() {
        let base = Instant::now();
        let mut account = Account::new(base);

        account.open_section("crates/engine", at(base, 10));
        account.open_section("crates/tui", at(base, 70));

        // The first section froze when the second opened, so the answer is the
        // second's start and the handover is one step rather than a fade.
        assert_eq!(account.open_section_started(), Some(at(base, 70)));
        assert!(account.sections()[0].is_closed());
        assert!(!account.sections()[1].is_closed());
    }

    #[test]
    fn a_closed_section_is_not_an_open_one() {
        let base = Instant::now();
        let mut account = Account::new(base);

        account.open_section("crates/engine", at(base, 10));
        account.close_section(
            &Outcome::Refused {
                reason: "no".into(),
            },
            at(base, 30),
        );

        // Closed with an outcome: the directory is done, so nothing is being
        // worked even though the section is still the last one there is.
        assert_eq!(account.open_section_started(), None);

        // And a run that ended freezes whatever was still live, so a finished
        // run has no open section either.
        account.open_section("crates/tui", at(base, 40));
        assert_eq!(account.open_section_started(), Some(at(base, 40)));
        account.finish(at(base, 90));
        assert_eq!(account.open_section_started(), None);
    }
}
