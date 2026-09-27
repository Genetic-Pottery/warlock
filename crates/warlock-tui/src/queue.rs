//! Which ticket a scope's queue gives up next, decided as a value rather than a
//! request.
//!
//! [`choose`] is the whole module: the queue [`scope_queue`] read, the state the
//! scope record reserves for review, and the run records this machine holds go
//! in, and one issue or none comes out with a reason for every issue passed
//! over. Nothing here posts, spawns or reads a path — the run records arrive as
//! a slice rather than being loaded from a home directory, which is what lets a
//! test drive every skip and the whole ordering with no socket and no temporary
//! home.
//!
//! No HTTP vocabulary crosses into it either. The choice is made over the types
//! [`mod@crate::linear`] already parsed, so a change to how the board spells an
//! answer cannot change which ticket is next.
//!
//! [`scope_queue`]: crate::Board::scope_queue

use std::cmp::Reverse;
use std::collections::HashMap;
use std::fmt;

use warlock_engine::{PullRun, RunStatus};

use crate::linear::{Blocker, Priority, Queue, QueuedIssue};

/// The workflow state a ticket sits in while a run works it.
///
/// Matched the way the board move matches it — trimmed and case-insensitively —
/// because `In progress` and `In Progress` are the same column to everyone
/// except a string comparison, and a team that renamed the case would otherwise
/// have every one of its started tickets read as somebody else's work.
pub const IN_PROGRESS: &str = "In Progress";

/// What choosing came to: the ticket to work, and every issue it walked past.
///
/// A ready issue that simply lost the ordering is not in [`skipped`]: those four
/// reasons are the ones a person can act on, and listing the rest of the queue
/// under "not chosen" would bury them. So an empty [`taken`] beside an empty
/// [`skipped`] means the queue itself was empty.
///
/// [`taken`]: Chosen::taken
/// [`skipped`]: Chosen::skipped
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chosen {
    taken: Option<QueuedIssue>,
    skipped: Vec<Skipped>,
}

impl Chosen {
    #[must_use]
    pub const fn taken(&self) -> Option<&QueuedIssue> {
        self.taken.as_ref()
    }

    /// In the queue's order rather than the reason's, so the list reads as a pass
    /// down the board.
    #[must_use]
    pub fn skipped(&self) -> &[Skipped] {
        &self.skipped
    }
}

/// One issue that was not available, with the reason it was not.
///
/// The issue is carried whole rather than as an identifier: what is printed
/// about a skip is the identifier, the title and the reason, and an output line
/// that had to look the title back up would be a second pass over the queue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Skipped {
    issue: QueuedIssue,
    reason: Reason,
}

impl Skipped {
    #[must_use]
    pub const fn issue(&self) -> &QueuedIssue {
        &self.issue
    }

    #[must_use]
    pub const fn reason(&self) -> &Reason {
        &self.reason
    }
}

/// Why an issue in the queue was not available, in the four ways it can happen.
///
/// Each variant carries everything its own sentence needs, so the reason prints
/// on its own: a halt names the command that releases it without the printer
/// having to know which ticket it was about, and a block names its blockers
/// without a second look at the queue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reason {
    /// A run for it on this machine stopped and is waiting for the operator to
    /// say they have looked.
    Halted { ticket: String },
    /// It is in the state the scope record reserves for review, where it is
    /// waiting on a human. The state is the board's own spelling of it, not the
    /// record's.
    InReview { state: String },
    /// It is in [`IN_PROGRESS`] and this machine holds no run record for it, so
    /// the work is somewhere warlock cannot see.
    InProgressElsewhere,
    /// At least one issue blocking it is neither `completed` nor `canceled`.
    /// Only the open blockers are carried; a settled one is not part of the
    /// reason.
    Blocked { blockers: Vec<Blocker> },
}

impl fmt::Display for Reason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Halted { ticket } => {
                write!(f, "halted — `warlock resume {ticket}` releases it")
            }
            Self::InReview { state } => {
                write!(f, "in `{state}`, which is waiting on a human")
            }
            Self::InProgressElsewhere => {
                f.write_str("in progress elsewhere — this machine holds no run record for it")
            }
            Self::Blocked { blockers } => write!(f, "blocked by {}", listed(blockers)),
        }
    }
}

/// The next ticket in a scope's queue, and why each of the others was passed
/// over.
///
/// The order the rules are applied in is the promise rather than an
/// arrangement. A halted run is named as halted even
/// though its ticket is usually sitting in [`IN_PROGRESS`], because "release it
/// with `warlock resume`" is the thing to do about it and "in progress
/// elsewhere" would be a lie about a run this machine is holding. Review comes
/// next, because an issue waiting on a human is waiting whatever else is true of
/// it. Blocking is asked last, so an issue that is both blocked and in review
/// reads as in review.
///
/// A run that was resumed is taken ahead of any issue with no record at all,
/// however the ordering would otherwise fall: the operator has already looked at
/// that run, and there is a branch and a tree waiting on it. Several of them
/// come out in the queue's order, since nothing here knows which was resumed
/// first.
///
/// ```
/// use warlock_tui::{Priority, Queue, QueuedIssue, StateType, choose};
///
/// let queue = Queue::new(
///     vec![
///         QueuedIssue::new(
///             "id-10", "WAR-10", "Ten", "Todo",
///             StateType::new("unstarted"), Priority::High, vec![],
///         ),
///         QueuedIssue::new(
///             "id-9", "WAR-9", "Nine", "In Review",
///             StateType::new("started"), Priority::Urgent, vec![],
///         ),
///     ],
///     false,
/// );
///
/// let choice = choose(&queue, "In Review", &[]);
///
/// // The urgent one is waiting on a human, so the high one is next.
/// assert_eq!(choice.taken().map(QueuedIssue::identifier), Some("WAR-10"));
/// assert_eq!(choice.skipped().len(), 1);
/// assert_eq!(
///     choice.skipped()[0].reason().to_string(),
///     "in `In Review`, which is waiting on a human"
/// );
/// ```
#[must_use]
pub fn choose(queue: &Queue, review_state: &str, runs: &[PullRun]) -> Chosen {
    let mut skipped = Vec::new();
    let mut resumed = Vec::new();
    let mut ready = Vec::new();

    for issue in queue.issues() {
        let status = run_for(runs, issue.identifier()).map(PullRun::status);

        if status == Some(RunStatus::Halted) {
            skipped.push(skip(
                issue,
                Reason::Halted {
                    ticket: issue.identifier().to_owned(),
                },
            ));
            continue;
        }

        if named(issue.state(), review_state) {
            skipped.push(skip(
                issue,
                Reason::InReview {
                    state: issue.state().to_owned(),
                },
            ));
            continue;
        }

        if status.is_none() && named(issue.state(), IN_PROGRESS) {
            skipped.push(skip(issue, Reason::InProgressElsewhere));
            continue;
        }

        let open: Vec<Blocker> = issue
            .blockers()
            .iter()
            .filter(|blocker| !blocker.state_type().settled())
            .cloned()
            .collect();
        if !open.is_empty() {
            skipped.push(skip(issue, Reason::Blocked { blockers: open }));
            continue;
        }

        if status == Some(RunStatus::Resumed) {
            resumed.push(issue);
        } else {
            ready.push(issue);
        }
    }

    let blocks = blocking_counts(queue);
    // Stable, so the queue's order is what settles anything the key leaves
    // equal — which the key does not, since it ends in the identifier.
    ready.sort_by_key(|issue| order_of(issue, &blocks));

    Chosen {
        taken: resumed
            .first()
            .or(ready.first())
            .map(|&issue| issue.clone()),
        skipped,
    }
}

/// The run record for that ticket, if this machine holds one.
///
/// Trimmed and case-insensitive for the reason [`IN_PROGRESS`] is: the record's
/// ticket was written from an identifier the board gave, but a record a person
/// typed or a future `--ticket war-9` should still find its own run rather than
/// starting a second one over the same branch.
fn run_for<'a>(runs: &'a [PullRun], identifier: &str) -> Option<&'a PullRun> {
    runs.iter()
        .find(|run| run.ticket().trim().eq_ignore_ascii_case(identifier.trim()))
}

fn named(state: &str, name: &str) -> bool {
    state.trim().eq_ignore_ascii_case(name.trim())
}

fn skip(issue: &QueuedIssue, reason: Reason) -> Skipped {
    Skipped {
        issue: issue.clone(),
        reason,
    }
}

/// How many issues in the queue each identifier blocks.
///
/// Counted over the whole queue rather than over the ready issues alone: an
/// issue holding up three blocked ones is worth taking first, and the three are
/// exactly the issues that are not ready yet.
fn blocking_counts(queue: &Queue) -> HashMap<&str, usize> {
    let mut counts = HashMap::new();
    for issue in queue.issues() {
        for blocker in issue.blockers() {
            *counts.entry(blocker.identifier()).or_insert(0) += 1;
        }
    }
    counts
}

/// The sort key, and the whole of the ordering: priority, then how much of the
/// queue this issue is holding up, then the number in its identifier.
///
/// Total and deterministic down to the last tuple element, because two runs of
/// the same queue choosing different tickets is not something a person could
/// diagnose. The trailing identifier is what makes it so: it is unique on the
/// board, so nothing after it could matter.
fn order_of(issue: &QueuedIssue, blocks: &HashMap<&str, usize>) -> OrderKey {
    let identifier = issue.identifier();
    (
        issue.priority(),
        Reverse(blocks.get(identifier).copied().unwrap_or(0)),
        // The flag carries the fallback, and it is a flag rather than an
        // `Option` because `None` sorts *before* `Some` and the answer wanted is
        // the other way round: an identifier whose suffix is not a number is
        // ordered after every one that is, and then by its own text. Refusing
        // it, or panicking on it, would be warlock deciding what Linear is
        // allowed to call an issue.
        number_in(identifier).map_or((true, 0), |number| (false, number)),
        identifier.to_owned(),
    )
}

type OrderKey = (Priority, Reverse<usize>, (bool, u64), String);

/// `WAR-9` is 9, so it comes before `WAR-10`. Comparing the suffixes as text is
/// what puts `WAR-10` first, and the queue is read in identifier order often
/// enough that the wrong answer looks deliberate.
fn number_in(identifier: &str) -> Option<u64> {
    identifier
        .rsplit_once('-')
        .and_then(|(_, number)| number.trim().parse().ok())
}

/// `WAR-12 (Cole)`, `WAR-13 (unassigned)`, joined — the blockers as a refusal
/// names them, with whose each one is.
fn listed(blockers: &[Blocker]) -> String {
    blockers
        .iter()
        .map(|blocker| {
            format!(
                "{} ({})",
                blocker.identifier(),
                blocker.assignee().unwrap_or("unassigned")
            )
        })
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
#[path = "tests/queue.rs"]
mod tests;
