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
//! is silent and stops where it got to instead of snapping back — which is the
//! only thing on screen distinguishing a slow pass from a hung one.
//!
//! Text is stored whole. Cutting a line to a width belongs to whoever knows the
//! width, and wrapping to [`Line::Wrapped`] happens in [`mod@crate::wrap`] on
//! the way to the screen.


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

/// Opens the line [`Account::record_rejected`] files: the engine turned an
/// answer down, so the wait that follows is a second one.
const REJECTED: &str = "rejected";

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
    Wrote {
        document: PathBuf,
        bytes: u64,
    },
    Refused {
        reason: String,
    },
    Unchanged {
        document: PathBuf,
    },
    Cancelled,
}

impl Outcome {
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
    },
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

    /// `waiting · 11 files, 1.6 MB`, filed at the handover to the pass. Both
    /// numbers are already known there, so nothing is measured for this line.
    ///
    /// The text is deliberately not the bare `waiting` constant, so
    /// `Log::extend_or_open` cannot fold it into a neighbouring line. Pushed
    /// rather than extended because a handover happens once per pass. Same
    /// silence as [`Account::record`] when there is no live section.
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
        let Some(section) = self.sections.last_mut() else {
            return;
        };
        if section.is_closed() {
            return;
        }

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
        for index in 0..self.sections.len() {
            if self.sections[index].has_outcome {
                continue;
            }
            let ending = outcome(&self.sections[index]);
            self.sections[index].word(&ending, at);
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
            .filter(|section| section.cost.is_none())
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
/// Base 1024, spelled `KB` rather than `KiB`, which is the spelling the panel
/// is specified in; the engine writes `KiB` for the same arithmetic and
/// settling that is not this module's call.
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
mod tests {
    use std::path::Path;
    use std::time::{Duration, Instant};

    use super::{Account, Line, Outcome, Section, clock, size};
    use crate::claude::Activity;

    fn at(base: Instant, seconds: u64) -> Instant {
        base + Duration::from_secs(seconds)
    }

    fn tool(name: &str, detail: &str) -> Activity {
        Activity::Tool {
            name: name.to_owned(),
            detail: Some(detail.to_owned()),
        }
    }

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
        // blocks — counts its own block's bytes from its own instant, rather
        // than continuing the earlier line's total.
        let base = Instant::now();
        let mut account = Account::new(base);

        account.open_section("crates/engine", base);
        account.record(&Activity::Writing { bytes: 2_048 }, at(base, 10));
        account.record(&tool("Read", "src/lib.rs"), at(base, 30));
        account.record(&Activity::Writing { bytes: 512 }, at(base, 40));
        account.record(&tool("Read", "src/big.rs"), at(base, 50));
        account.record(&Activity::Writing { bytes: 100 }, at(base, 60));

        assert_eq!(
            said(&account, at(base, 90)),
            vec![
                "crates/engine".to_owned(),
                "0:30 writing · 2.0 KB".to_owned(),
                "0:40 Read src/lib.rs".to_owned(),
                "0:50 writing · 512 bytes".to_owned(),
                "1:00 Read src/big.rs".to_owned(),
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
    fn a_carried_section_says_unchanged_and_names_no_write_and_no_cost() {
        let base = Instant::now();
        let mut account = Account::new(base);

        // A directory the run carried forward: it opened a section, because the
        // engine announced it like any other, and then closed it having spent
        // nothing and written nothing.
        account.open_section("crates/engine", base);
        account.close_section(
            &Outcome::Unchanged {
                document: "crates/engine/WARLOCK.md".into(),
            },
            at(base, 0),
        );

        let line = said(&account, at(base, 5))
            .into_iter()
            .find(|line| line.contains("crates/engine/WARLOCK.md"))
            .expect("the section closed with a line naming its document");
        assert!(
            line.contains("unchanged"),
            "the word a reader learns this from, and the one `wrote` would \
             have lied about: {line}",
        );
        assert!(
            !line.contains("wrote"),
            "nothing was written, so nothing may say it was: {line}",
        );
        assert!(
            !line.contains('$') && !line.contains("cost"),
            "and a carried directory has no spend to report at all: {line}",
        );
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
    fn a_pushed_line_freezes_and_ticks_like_any_other() {
        let base = Instant::now();
        let mut account = Account::new(base);

        account.open_section("crates/engine", base);
        account.record(&Activity::Thinking, at(base, 1));

        // Its arrival freezes the line above it, at the instant it arrived.
        account.record_waiting(11, 34 * 1024, at(base, 20));
        for now in [at(base, 20), at(base, 60), at(base, 900)] {
            assert_eq!(said(&account, now)[1], "0:20 thinking");
        }

        // And, being the newest, it counts up until something newer lands.
        assert_eq!(
            said(&account, at(base, 40))[2],
            "0:40 waiting · 11 files, 34 KB"
        );
        assert_eq!(
            said(&account, at(base, 41))[2],
            "0:41 waiting · 11 files, 34 KB"
        );
        account.record(&Activity::Writing { bytes: 0 }, at(base, 50));
        for now in [at(base, 50), at(base, 900)] {
            assert_eq!(said(&account, now)[2], "0:50 waiting · 11 files, 34 KB");
        }

        // A section frozen by the next one opening stops with a pushed line
        // under it exactly as it stops with anything else.
        account.open_section("crates/tui", at(base, 80));
        assert_eq!(said(&account, at(base, 4_000))[3], "1:20 writing");
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
    fn a_handed_over_request_lands_below_whatever_spoke_before_it() {
        let base = Instant::now();
        let mut account = Account::new(base);

        // Anything filed before the handover means the placeholder never
        // appears at all, and the request line lands under it like any other.
        account.open_section("crates/engine", base);
        account.record(&tool("Read", "Cargo.lock"), at(base, 10));
        account.record(&tool("Read", "src/lib.rs"), at(base, 70));
        account.record_waiting(11, 34 * 1024, at(base, 130));
        account.record(&Activity::Thinking, at(base, 190));

        assert_eq!(
            said(&account, at(base, 200)),
            vec![
                "crates/engine".to_owned(),
                "1:10 Read Cargo.lock".to_owned(),
                "2:10 Read src/lib.rs".to_owned(),
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
        // and frozen: the same silence `record` keeps.
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
