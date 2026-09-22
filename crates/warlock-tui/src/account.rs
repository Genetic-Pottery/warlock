//! What one pact did, in the order it did it: an [`Account`] of ordered
//! [`Section`]s, one per directory, each an ordered [`Log`] of clocked lines.
//!
//! Plain data. [`Instant::now`] is never called in this file — every entry
//! point takes the instant it happened at, and every read takes the `now` its
//! clocks are measured against — which is what lets a test drive a whole run
//! off `base + Duration::from_secs` and assert on exact text. Adding a clock
//! read here would make those tests race.
//!
//! The clock rule is `Log::shown_at`: a line shows the instant the line beneath
//! it arrived, or, when it is the newest, `now` while the log is live and the
//! instant it froze once it is not. So the newest clock counts up while a pass
//! is silent and stops where it got to instead of snapping back, which is the
//! only thing on screen telling a slow pass from a hung one. Text is stored
//! whole; cutting a line to a width belongs to whoever knows the width.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::claude::Activity;

pub(crate) const THINKING: &str = "thinking";

/// The placeholder line of a section that has heard nothing yet.
///
/// Drawn by [`Log::rows`] rather than stored, so nothing has to arrive to
/// create it and nothing has to be deleted when the first real line lands.
pub(crate) const WAITING: &str = "waiting";

pub(crate) const WRITING: &str = "writing";

/// Opens the line [`Account::record_describing`] rewords: one file of this
/// directory's per-file passes is in flight.
pub(crate) const DESCRIBING: &str = "describing";

/// Opens the line [`Account::record_rejected`] files: the engine turned an
/// answer down, so the wait that follows is a second one.
const REJECTED: &str = "rejected";

/// Opens the line [`Account::record_repaired`] files: the asking ran out and
/// warlock mended the slot itself, so the document says this much and no more.
const REPAIRED: &str = "repaired";

/// One line and the instant it arrived. Its own arrival is what freezes the
/// line above it, so every entry has to remember it even though a line usually
/// displays the instant belonging to the entry beneath it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Entry {
    at: Instant,
    text: String,
}

/// A run of clocked lines under one heading, and the whole of the clock rule.
///
/// Shared rather than copied because there are two of these in warlock — a
/// [`Section`] of an account and a turn of the panel's thread — which differ in
/// what surrounds the lines, not in how the lines tick. It holds no directory,
/// no outcome and no money, and the only wording it does is [`WAITING`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Log {
    started: Instant,
    entries: Vec<Entry>,
    closed: Option<Instant>,
}

impl Log {
    pub(crate) const fn opened_at(at: Instant) -> Self {
        Self {
            started: at,
            entries: Vec::new(),
            closed: None,
        }
    }

    pub(crate) const fn started(&self) -> Instant {
        self.started
    }

    pub(crate) const fn closed_at(&self) -> Option<Instant> {
        self.closed
    }

    pub(crate) const fn is_closed(&self) -> bool {
        self.closed.is_some()
    }

    /// How many rows this draws as, placeholder included.
    ///
    /// The `max(1)` is the drawn [`WAITING`] line. Scroll offsets are clamped
    /// against this, so a row that is drawn but not counted is one the panel
    /// can never scroll to the edge of.
    pub(crate) fn row_count(&self) -> usize {
        self.entries.len().max(1)
    }

    /// Idempotent: work can be frozen by its own ending, by the next stretch
    /// starting or by the run ending, and the first of those is the honest
    /// instant. A plain assignment would let the end of a run re-date a pass
    /// that stopped nine minutes earlier.
    pub(crate) fn freeze(&mut self, at: Instant) {
        self.closed.get_or_insert(at);
    }

    pub(crate) fn push(&mut self, text: impl Into<String>, at: Instant) {
        self.entries.push(Entry {
            at,
            text: text.into(),
        });
    }

    /// Let a repeated report go on as one line instead of filing a column of
    /// identical ones. The entry already there keeps its own instant, so its
    /// clock counts the whole stretch rather than restarting per report.
    pub(crate) fn extend_or_open(&mut self, text: &str, at: Instant) {
        if self.entries.last().is_none_or(|entry| entry.text != text) {
            self.push(text, at);
        }
    }

    /// [`Log::extend_or_open`] for a stretch whose wording changes as it goes
    /// on. Matching on `word` through [`continues`] rather than on the whole
    /// text is what makes a running byte count one stretch; comparing texts
    /// would read every new count as a new line with a clock starting at zero.
    pub(crate) fn rewrite_or_open(&mut self, word: &str, text: &str, at: Instant) {
        match self.entries.last_mut() {
            Some(entry) if continues(&entry.text, word) => text.clone_into(&mut entry.text),
            _ => self.push(text, at),
        }
    }

    pub(crate) fn rows(&self, now: Instant) -> impl Iterator<Item = Line> + '_ {
        // Clocked as entry zero, which gives it the ordinary rule with nothing
        // special added: no entry follows it, so it ticks while the log is live
        // and freezes where the log froze.
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

    /// The clock rule, in one expression: the next line's arrival, else the
    /// instant this log froze, else the caller's `now`.
    fn shown_at(&self, index: usize, now: Instant) -> Instant {
        self.entries
            .get(index + 1)
            .map_or_else(|| self.closed.unwrap_or(now), |next| next.at)
    }

    fn clock(&self, index: usize, now: Instant) -> String {
        clock(
            self.shown_at(index, now)
                .saturating_duration_since(self.started),
        )
    }
}

/// How a directory's pass ended.
///
/// Carries no cost. The spend arrives over the activity port during the pass
/// and is accumulated by the [`Section`], so a caller closing one never has to
/// know what it cost.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Wrote { document: PathBuf, bytes: u64 },
    Refused { reason: String },
    Unchanged { document: PathBuf },
    Skipped { below: PathBuf },
    Cancelled,
}

impl Outcome {
    /// Whether a model pass ran for this directory at all.
    ///
    /// Two of these ran none: a carried document needed no pass and a skipped
    /// directory was never going to earn a grant. A section with no cost under
    /// either of them is not a pass whose cost went missing, which is the
    /// difference [`Account::finish`] reports as `incomplete`.
    const fn ran_a_pass(&self) -> bool {
        match self {
            Self::Unchanged { .. } | Self::Skipped { .. } => false,
            Self::Wrote { .. } | Self::Refused { .. } | Self::Cancelled => true,
        }
    }

    /// `cost` of `None` is said in words rather than printed as `$0.00`: a
    /// pass that reported nothing and a pass that was free are different facts.
    fn line(&self, cost: Option<f64>) -> String {
        match self {
            Self::Wrote { document, bytes } => {
                let document = document.display();
                let cost = spend(cost);
                format!("wrote {document} — {bytes} bytes, {cost}")
            }
            Self::Unchanged { document } => {
                let document = document.display();
                format!("unchanged — {document} kept, no pass needed")
            }
            // Says which directory below cost it the pass, because on its own
            // "skipped" reads as a run losing interest. Nothing was written and
            // nothing was spent: the document this directory has is the one it
            // had.
            Self::Skipped { below } => {
                let below = below.display();
                format!(
                    "skipped — {below} below it was not documented, so no pass would have been granted"
                )
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

#[derive(Debug, Clone, PartialEq)]
pub struct Section {
    directory: PathBuf,
    log: Log,
    cost: Option<f64>,
    /// Whether the outcome line is already under this section.
    ///
    /// Not the same question as [`Section::is_closed`]: a section stops moving
    /// the moment the next directory opens, which is long before the run says
    /// how it went, so most sections spend a run frozen and unworded.
    /// [`Account::close_open_sections`] reads this to find the ones still owed
    /// an ending.
    has_outcome: bool,
    /// Whether any pass ran under this section, taken from the outcome that
    /// closed it. A section that ran none has no cost to be missing, and
    /// [`Account::finish`] leaves it out of the `incomplete` count rather than
    /// reporting a pass nobody made as one that forgot to say what it cost.
    passless: bool,
    /// What this directory's per-file passes have been handed so far, summed as
    /// each `describing` arrives. Kept on the section rather than recomputed
    /// from the line already on screen, which would mean parsing back a string
    /// this module just formatted.
    described_bytes: u64,
}

impl Section {
    #[must_use]
    pub fn directory(&self) -> &Path {
        &self.directory
    }

    #[must_use]
    pub const fn cost(&self) -> Option<f64> {
        self.cost
    }

    #[must_use]
    pub const fn is_closed(&self) -> bool {
        self.log.is_closed()
    }

    fn line_count(&self) -> usize {
        self.log.row_count() + 1
    }

    /// The outcome takes the instant the section *stopped*, not the instant
    /// somebody got round to wording it — otherwise a pass that took thirty
    /// seconds ends up dated at the end of the whole run. Only a section still
    /// live when it is worded takes `at`. The first ending wins, because it is
    /// the one already on screen.
    fn word(&mut self, outcome: &Outcome, at: Instant) {
        if self.has_outcome {
            return;
        }
        let at = self.log.closed_at().unwrap_or(at);
        self.log.push(outcome.line(self.cost), at);
        self.has_outcome = true;
        self.passless = !outcome.ran_a_pass();
        self.log.freeze(at);
    }
}

/// One drawable row of the panel.
///
/// Flat rather than nested: the panel is a list and scrolling it is counting,
/// so a heading takes a row like anything else and the rows above and below a
/// window are arithmetic rather than a walk. All three cards — account, thread
/// and document — yield this one type, so one window rule covers them.
///
/// What goes in *front* of a row is the renderer's: no marker for a question,
/// no bullet for a note. Baking one in here would be a second answer to that
/// question sitting in the value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Line {
    Directory {
        path: PathBuf,
    },
    Clocked {
        clock: String,
        text: String,
    },
    Summary {
        text: String,
    },
    Text {
        text: String,
    },
    Said {
        text: String,
    },
    Note {
        text: String,
    },
    /// Nothing in this module ever makes one. An [`Account`] holds what
    /// happened; how many rows that takes is a question about a terminal, so
    /// these are made in `crate::wrap`. A line that fits keeps its own
    /// variant, and a broken line keeps it on its first row, so what a row is
    /// still reads off the value.
    Wrapped {
        text: String,
        /// Whether the line this continues is drawn bold, so one line broken
        /// over two rows is not bold on one and plain on the other.
        heading: bool,
        /// Whose line this continues, so one line broken over two rows is not
        /// two colours. `heading` cannot answer this: it separates bold from
        /// plain, which is two of the three voices on one side of it.
        voice: Voice,
    },
}

/// Who a row belongs to, which is what decides its colour.
///
/// Not the kind of row: a work line and a summary line look nothing alike and
/// are both warlock accounting for its own work, while `Text` is the only
/// variant that is the model's own words.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Voice {
    /// What the operator typed.
    Operator,
    /// The model's answer, and only that.
    Model,
    /// Warlock accounting for its own work: directories, work lines,
    /// summaries, its own notes.
    Warlock,
}

impl Line {
    /// ```
    /// use warlock_tui::{Line, Voice};
    ///
    /// let said = Line::Said { text: "pact this".to_owned() };
    /// let work = Line::Clocked { clock: "0:04".to_owned(), text: "Reading".to_owned() };
    ///
    /// assert_eq!(said.voice(), Voice::Operator);
    /// assert_eq!(work.voice(), Voice::Warlock);
    /// ```
    #[must_use]
    pub const fn voice(&self) -> Voice {
        // No catch-all arm, for the reason `colour_for` has none: a variant
        // added here has to break this match rather than be drawn in whichever
        // voice a fallback picked.
        match self {
            Self::Said { .. } => Voice::Operator,
            Self::Text { .. } => Voice::Model,
            Self::Directory { .. }
            | Self::Clocked { .. }
            | Self::Summary { .. }
            | Self::Note { .. } => Voice::Warlock,
            Self::Wrapped { voice, .. } => *voice,
        }
    }
}

/// Everything one pact did, from the key press to the summary line.
///
/// One pact, one account: a second run starts a new one rather than appending.
/// Nothing is trimmed while the run is the current one.
///
/// Holds an `f64` cost, so it is [`PartialEq`] and not [`Eq`] — as is
/// everything that comes to hold one.
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
    started: Instant,
    sections: Vec<Section>,
    summary: Option<String>,
}

impl Account {
    #[must_use]
    pub const fn new(at: Instant) -> Self {
        Self {
            started: at,
            sections: Vec::new(),
            summary: None,
        }
    }

    /// Freezing the section above is what stops two clocks running at once:
    /// the pass that was running it is over whether or not the caller closed it
    /// with an outcome.
    pub fn open_section(&mut self, directory: impl Into<PathBuf>, at: Instant) {
        if let Some(previous) = self.sections.last_mut() {
            previous.log.freeze(at);
        }
        self.sections.push(Section {
            directory: directory.into(),
            log: Log::opened_at(at),
            cost: None,
            has_outcome: false,
            passless: false,
            described_bytes: 0,
        });
    }

    /// A cost becomes no line at all — it is a fact about the pass rather than
    /// something the pass did, and it reaches the reader through the outcome
    /// line and the summary instead.
    ///
    /// Silently drops everything when there is no live section, or when the
    /// newest one is frozen: a line filed under a worded section would
    /// contradict a line already on screen.
    pub fn record(&mut self, activity: &Activity, at: Instant) {
        let Some(section) = self.live_section() else {
            return;
        };

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

    /// `waiting · 11 files, 1.6 MB`, filed at the handover to the synthesis
    /// pass. Both numbers are already known there, so nothing is measured for
    /// this line.
    ///
    /// Pushed rather than reworded, and this is the line that separates the two
    /// halves of a per-file run: the [`DESCRIBING`] line above it counts the
    /// files being read, this one counts the lines they came to. A directory
    /// reaches this once, after its last file. Same silence as
    /// [`Account::record`] when there is no live section.
    pub fn record_waiting(&mut self, files: usize, bytes: u64, at: Instant) {
        let Some(section) = self.live_section() else {
            return;
        };

        let files = plural(files, "file", "files");
        section
            .log
            .push(format!("{WAITING} · {files}, {}", size(bytes)), at);
    }

    /// `describing · 4/18 files, 12 KB`, reworded in place as each file's pass
    /// goes out rather than filed one line per file.
    ///
    /// A directory of eighteen moved files is eighteen passes and would be
    /// eighteen identical-looking lines in the run's record, which is a column
    /// of noise standing in for one fact. Reworded the way
    /// [`Activity::Writing`] is, so the clock on it counts the whole stretch of
    /// file passes from the first, and what it freezes at is what the directory
    /// came to. Same silence as [`Account::record`] when there is no live
    /// section.
    pub fn record_describing(&mut self, position: usize, total: usize, bytes: u64, at: Instant) {
        let Some(section) = self.live_section() else {
            return;
        };

        section.described_bytes = section.described_bytes.saturating_add(bytes);
        let text = format!(
            "{DESCRIBING} · {position}/{total} {}, {}",
            if total == 1 { "file" } else { "files" },
            size(section.described_bytes)
        );
        section.log.rewrite_or_open(DESCRIBING, &text, at);
    }

    /// Not an [`Activity`]: the stream reported an answer and it was warlock
    /// that refused it. Same silence as [`Account::record`] when there is no
    /// live section.
    pub fn record_rejected(
        &mut self,
        defects: &[String],
        attempt: usize,
        attempts: usize,
        at: Instant,
    ) {
        let Some(section) = self.live_section() else {
            return;
        };

        // The first defect and a count of the rest: a line, not the list. The
        // engine's own refusal carries the whole list, and the footer shows
        // that when the directory fails.
        let first = defects.first().map_or("no answer", String::as_str);
        let rest = match defects.len() {
            0 | 1 => String::new(),
            more => format!(" (+{})", more - 1),
        };
        section.log.push(
            format!("{REJECTED} · attempt {attempt}/{attempts}: {first}{rest}"),
            at,
        );
    }

    /// The other end of [`Account::record_rejected`]: the attempts ran out and
    /// warlock mended the slot itself rather than refusing the document. One
    /// line per mend, in the engine's own words — which name the slot the way
    /// the defect behind it was named, and what was done to it — because that
    /// is the difference between the model's prose and warlock's fallback, and
    /// this panel is where a reader of the document finds it out. Same silence
    /// as [`Account::record`] when there is no live section.
    pub fn record_repaired(&mut self, mend: &str, at: Instant) {
        let Some(section) = self.live_section() else {
            return;
        };

        section.log.push(format!("{REPAIRED} · {mend}"), at);
    }

    fn live_section(&mut self) -> Option<&mut Section> {
        self.sections
            .last_mut()
            .filter(|section| !section.is_closed())
    }

    /// [`Account::finish`] without the wording or the money. Crate-private
    /// because the caller is `finish`; stopping a run's clocks from anywhere
    /// else would be somebody other than the run deciding it is over.
    pub(crate) fn freeze(&mut self, at: Instant) {
        if let Some(section) = self.sections.last_mut() {
            section.log.freeze(at);
        }
    }

    pub fn close_section(&mut self, outcome: &Outcome, at: Instant) {
        if let Some(section) = self.sections.last_mut() {
            section.word(outcome, at);
        }
    }

    /// A run does not report itself a directory at a time: a pass is over when
    /// the next one starts, but *how* it went arrives once at the end, so by
    /// then every section is frozen and none has an ending. Each is worded at
    /// the instant it stopped rather than at `at`; sections closed already keep
    /// the ending they have and `outcome` is never asked about them.
    pub fn close_open_sections(
        &mut self,
        at: Instant,
        mut outcome: impl FnMut(&Section) -> Outcome,
    ) {
        for section in &mut self.sections {
            if section.has_outcome {
                continue;
            }
            let ending = outcome(section);
            section.word(&ending, at);
        }
    }

    /// `pact finished — 9 directories, 4:12, $1.87`, measured from
    /// [`Account::new`] rather than from the first section, because a run
    /// starts when the key is pressed.
    ///
    /// A pass whose cost never arrived is not worth zero, so a run missing any
    /// says `(incomplete: 2 passes reported no cost)` rather than quietly
    /// under-reporting a number somebody will take at face value.
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

    #[must_use]
    pub fn sections(&self) -> &[Section] {
        &self.sections
    }

    pub(crate) const fn started(&self) -> Instant {
        self.started
    }

    /// An instant and not a duration: this file reads no clock, and answering
    /// with a duration would mean picking a `now` here rather than taking the
    /// caller's.
    #[must_use]
    pub fn open_section_started(&self) -> Option<Instant> {
        self.sections
            .last()
            .filter(|section| !section.is_closed())
            .map(|section| section.log.started())
    }

    #[must_use]
    pub fn line_count(&self) -> usize {
        self.sections.iter().map(Section::line_count).sum::<usize>()
            + usize::from(self.summary.is_some())
    }

    #[must_use]
    pub fn lines(&self, now: Instant) -> Vec<Line> {
        self.window(0, self.line_count(), now)
    }

    /// Asking for more rows than there are, or starting past the end, gives
    /// back what is there: a viewport is a request, not an assertion about the
    /// account's length.
    #[must_use]
    pub fn window(&self, offset: usize, height: usize, now: Instant) -> Vec<Line> {
        self.rows(now).skip(offset).take(height).collect()
    }

    /// Crate-private rather than private because the panel draws a run's rows
    /// out of exactly this iterator: one function words what a run did, so
    /// there is no second spelling of a heading or an outcome line to keep in
    /// step with this one.
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

    /// Folded from `0.0` rather than `sum()`, because [`f64`]'s own
    /// [`Sum`](std::iter::Sum) starts at `-0.0` to keep signed zeroes exact and
    /// a run that spent nothing would print as `$-0.00`.
    fn spent(&self) -> f64 {
        self.sections
            .iter()
            .filter_map(Section::cost)
            .fold(0.0, |total, usd| total + usd)
    }

    fn unpriced(&self) -> usize {
        self.sections
            .iter()
            .filter(|section| section.cost.is_none() && !section.passless)
            .count()
    }
}

/// `m:ss`, counting from `0:00`.
///
/// Minutes are unpadded and allowed to grow past sixty rather than rolling into
/// an hours field, which would be a second format appearing once an hour in.
/// Sub-second precision is dropped, not rounded: a clock that reads `0:04`
/// twice is a clock, one that jumps ahead near the next second is a
/// distraction.
fn clock(elapsed: Duration) -> String {
    let seconds = elapsed.as_secs();
    format!("{}:{:02}", seconds / 60, seconds % 60)
}

/// The one place a number becomes money in this crate. Two decimals even where
/// a pass cost less than a cent — four places of noise on every line says less
/// about the run's total than `$0.00` does.
pub(crate) fn money(usd: f64) -> String {
    format!("${usd:.2}")
}

/// Shared with the thread, because a `Grep` is a `Grep` whichever card it
/// turns up on. What is not shared is anything around it.
pub(crate) fn tool_line(name: &str, detail: Option<&String>) -> String {
    detail.map_or_else(|| name.to_owned(), |detail| format!("{name} {detail}"))
}

/// `writing · 1.8 KB`. A count and nothing else: no denominator, because
/// nothing knows how long an answer will be until it ends, and no spinner,
/// because the clock the line already carries is the honest thing that moves.
///
/// Zero bytes is the bare word — the case the block's opening reports, before a
/// single delta has landed. `writing · 0 bytes` would word a measurement that
/// had not happened.
fn writing_line(bytes: u64) -> String {
    if bytes == 0 {
        WRITING.to_owned()
    } else {
        format!("{WRITING} · {}", size(bytes))
    }
}

/// What "the same stretch" means to [`Log::rewrite_or_open`]. It has to agree
/// with the shape [`writing_line`] produces, which is why it sits next to it.
fn continues(text: &str, word: &str) -> bool {
    text.strip_prefix(word)
        .is_some_and(|rest| rest.is_empty() || rest.starts_with(" · "))
}

fn spend(cost: Option<f64>) -> String {
    cost.map_or_else(|| "no cost reported".to_owned(), money)
}

/// `934 bytes`, `1.8 KB`, `403 KB`, `1.6 MB`, `12 MB`.
///
/// Base 1024, spelled `KB` rather than `KiB`.
///
/// Not the same formatter as the engine's `document::human`, which renders the
/// sizes inside a `WARLOCK.md`, and the two do not agree: the engine truncates
/// where this rounds half up and stays on one decimal at every magnitude, so
/// 1587 bytes is `1.6 KB` here and `1.5 KB` in the document the panel just
/// wrote. Unifying them is a real decision and not a cleanup — the engine's
/// spelling is pinned by a doctest and sits in every document on disk, so
/// changing it restales them — but it is a divergence, not a design.
///
/// The unit is chosen by magnitude *before* the rounding happens inside it, so
/// nothing is promoted across a boundary by being rounded: 1048575 bytes is
/// `1024 KB` and not `1.0 MB`, which would claim a megabyte had been reached.
/// `MB` is the largest unit because the request cap is measured in megabytes.
///
/// Public because two ledgers need the panel's spelling of a size — the
/// account's request lines and the line the thread says when `/write` puts a
/// file on disk — and two formatters would be two spellings the day either
/// changed.
#[must_use]
pub fn size(bytes: u64) -> String {
    const KB: u64 = 1024;
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

/// One decimal below ten of the unit, whole units at ten and above, rounding
/// half up in both. The multiply by ten is only on the tenths branch, where the
/// value is under ten units and nowhere near overflowing.
fn scaled(bytes: u64, unit: u64, name: &str) -> String {
    if bytes < 10 * unit {
        let tenths = (bytes * 10 + unit / 2) / unit;
        format!("{}.{} {name}", tenths / 10, tenths % 10)
    } else {
        format!("{} {name}", (bytes + unit / 2) / unit)
    }
}

/// `1 directory`, `9 directories`, so no line has to be worded twice or read
/// as `1 directories`.
fn plural(count: usize, one: &str, many: &str) -> String {
    let noun = if count == 1 { one } else { many };
    format!("{count} {noun}")
}

#[cfg(test)]
#[path = "tests/account.rs"]
mod tests;
