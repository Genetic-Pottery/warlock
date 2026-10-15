//! The rows the panel draws ([`Line`], [`Voice`]), the clocked [`Log`] a turn of
//! the thread is built on, and the wording shared by everything that reports
//! work as it happens: [`THINKING`], [`WRITING`], [`tool_line`], [`size`] and
//! [`plural`].
//!
//! Plain data. [`Instant::now`] is never called in this file — every entry
//! point takes the instant it happened at, and every read takes the `now` its
//! clocks are measured against — which is what lets a test drive a log off
//! `base + Duration::from_secs` and assert on exact text. Adding a clock read
//! here would make those tests race.
//!
//! The clock rule is `Log::shown_at`: a line shows the instant the line beneath
//! it arrived, or, when it is the newest, `now` while the log is live and the
//! instant it froze once it is not. So the newest clock counts up while a turn
//! is silent and stops where it got to instead of snapping back, which is the
//! only thing on screen telling a slow turn from a hung one. Text is stored
//! whole; cutting a line to a width belongs to whoever knows the width.

use std::time::{Duration, Instant};

pub(crate) const THINKING: &str = "thinking";

/// The placeholder line of a log that has heard nothing yet.
///
/// Drawn by [`Log::rows`] rather than stored, so nothing has to arrive to
/// create it and nothing has to be deleted when the first real line lands.
pub(crate) const WAITING: &str = "waiting";

pub(crate) const WRITING: &str = "writing";

/// One line and the instant it arrived. Its own arrival is what freezes the
/// line above it, so every entry has to remember it even though a line usually
/// displays the instant belonging to the entry beneath it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Entry {
    at: Instant,
    text: String,
}

/// A run of clocked lines, and the whole of the clock rule. The only wording it
/// does is [`WAITING`].
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

    /// Whether anything was filed under it at all.
    pub(crate) const fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Idempotent: the first freeze is the honest instant. A plain assignment
    /// would let a later freeze re-date work that stopped minutes earlier.
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

/// One drawable row of the panel.
///
/// Flat rather than nested: the panel is a list and scrolling it is counting,
/// so a heading takes a row like anything else and the rows above and below a
/// window are arithmetic rather than a walk. Both cards — thread and document —
/// yield this one type, so one window rule covers them.
///
/// What goes in *front* of a row is the renderer's: no marker for a question,
/// no bullet for a note. Baking one in here would be a second answer to that
/// question sitting in the value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Line {
    Clocked {
        clock: String,
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
    /// Nothing in this module ever makes one. How many rows a line takes is a
    /// question about a terminal, so these are made in `crate::wrap`. A line
    /// that fits keeps its own variant, and a broken line keeps it on its first
    /// row, so what a row is still reads off the value.
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
/// Not the kind of row: a work line and a note look nothing alike and are both
/// warlock accounting for its own work, while `Text` is the only variant that
/// is the model's own words.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Voice {
    /// What the operator typed.
    Operator,
    /// The model's answer, and only that.
    Model,
    /// Warlock accounting for its own work: work lines and its own notes.
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
            Self::Clocked { .. } | Self::Note { .. } => Voice::Warlock,
            Self::Wrapped { voice, .. } => *voice,
        }
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

/// Shared by the thread and `pull`, because a `Grep` is a `Grep` wherever it
/// turns up. What is not shared is anything around it.
pub(crate) fn tool_line(name: &str, detail: Option<&String>) -> String {
    detail.map_or_else(|| name.to_owned(), |detail| format!("{name} {detail}"))
}

/// `934 bytes`, `1.8 KB`, `403 KB`, `1.6 MB`, `12 MB`.
///
/// Base 1024, spelled `KB` rather than `KiB`, which is the spelling the panel
/// is specified in.
///
/// The unit is chosen by magnitude *before* the rounding happens inside it, so
/// nothing is promoted across a boundary by being rounded: 1048575 bytes is
/// `1024 KB` and not `1.0 MB`, which would claim a megabyte had been reached.
#[must_use]
pub(crate) fn size(bytes: u64) -> String {
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

/// `1 file`, `9 files`, so no line has to be worded twice or read as
/// `1 files`.
pub(crate) fn plural(count: usize, one: &str, many: &str) -> String {
    let noun = if count == 1 { one } else { many };
    format!("{count} {noun}")
}

#[cfg(test)]
#[path = "tests/account.rs"]
mod tests;
