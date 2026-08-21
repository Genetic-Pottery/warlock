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
const THINKING: &str = "thinking";

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
    /// When this section opened, which is where its clock counts from.
    started: Instant,
    /// The lines under it, in arrival order, with the outcome line — if the
    /// section was closed with one — last.
    entries: Vec<Entry>,
    /// What the pass reported spending, summed over however many times it said
    /// so. `None` means it never said, which is not the same as zero.
    cost: Option<f64>,
    /// When this section stopped being the live one, if it has. A closed
    /// section's last line is frozen here instead of ticking.
    closed: Option<Instant>,
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
        self.closed.is_some()
    }

    /// How many drawable rows this section is: its heading plus its lines.
    fn line_count(&self) -> usize {
        self.entries.len() + 1
    }

    /// The instant the line at `index` shows on its clock, against a caller's
    /// `now`.
    ///
    /// The whole rule of the ticking clock, in one expression. A line with
    /// another beneath it shows the instant that one arrived, so it reads as how
    /// long the thing took. The last line of a live section shows `now`, so it
    /// counts up between events. The last line of a closed section shows the
    /// instant the section closed, so it stops.
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

    /// Stop this section moving as of `at`, if it has not stopped already.
    ///
    /// Idempotent on purpose: a section can be frozen by its own outcome, by the
    /// next section opening, or by the run ending, and whichever happens first
    /// is the honest instant to freeze at.
    fn freeze(&mut self, at: Instant) {
        self.closed.get_or_insert(at);
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
        let at = self.closed.unwrap_or(at);
        self.entries.push(Entry {
            at,
            text: outcome.line(self.cost),
        });
        self.has_outcome = true;
        self.freeze(at);
    }
}

/// One drawable row of an account.
///
/// Flat rather than nested, because the panel is a list and scrolling it is
/// counting: a section heading takes a row like anything else, so the number of
/// rows above and below a window is arithmetic rather than a walk.
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
    /// again at `0:00`. Whatever section was live stops there and then — the
    /// pass that was running it is over, whether or not the caller closed it
    /// with an outcome, so its newest line has no business still counting up.
    pub fn open_section(&mut self, directory: impl Into<PathBuf>, at: Instant) {
        if let Some(previous) = self.sections.last_mut() {
            previous.freeze(at);
        }
        self.sections.push(Section {
            directory: directory.into(),
            started: at,
            entries: Vec::new(),
            cost: None,
            closed: None,
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
            Activity::Thinking => section.entries.push(Entry {
                at,
                text: THINKING.to_owned(),
            }),
            Activity::Tool { name, detail } => section.entries.push(Entry {
                at,
                text: detail
                    .as_ref()
                    .map_or_else(|| name.clone(), |detail| format!("{name} {detail}")),
            }),
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
        if let Some(section) = self.sections.last_mut() {
            section.freeze(at);
        }

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
    fn rows(&self, now: Instant) -> impl Iterator<Item = Line> + '_ {
        self.sections
            .iter()
            .flat_map(move |section| {
                std::iter::once(Line::Directory {
                    path: section.directory.clone(),
                })
                .chain(section.entries.iter().enumerate().map(
                    move |(index, entry)| Line::Clocked {
                        clock: section.clock(index, now),
                        text: entry.text.clone(),
                    },
                ))
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
fn money(usd: f64) -> String {
    format!("${usd:.2}")
}

/// What an outcome line says about a pass's cost, including when there is none.
///
/// A pass that never reported is said in words rather than as a number, because
/// every number here would be a lie about a thing that was never measured.
fn spend(cost: Option<f64>) -> String {
    cost.map_or_else(|| "no cost reported".to_owned(), money)
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

    use super::{Account, Line, Outcome, Section, clock};
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
                Line::Summary { text } => text,
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
    fn a_cost_is_arithmetic_rather_than_a_line() {
        let base = Instant::now();
        let mut account = Account::new(base);

        account.open_section("crates/engine", base);
        account.record(&Activity::Cost { usd: 0.21 }, at(base, 1));
        account.record(&Activity::Cost { usd: 0.04 }, at(base, 2));

        // Nothing but the heading was drawn, and the money was kept.
        assert_eq!(account.line_count(), 1);
        assert_eq!(
            said(&account, at(base, 2)),
            vec!["crates/engine".to_owned()]
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
}
