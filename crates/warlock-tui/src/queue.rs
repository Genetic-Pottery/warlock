//! Which ticket a scope's queue gives up next, decided as a value rather than a
//! request.
//!
//! [`choose`] is most of the module: the queue [`scope_queue`] read, the state
//! the scope record reserves for review, and the run records this machine holds
//! go in, and one issue or none comes out with a reason for every issue passed
//! over. Nothing on that path posts, spawns or reads a path — the run records
//! arrive as a slice rather than being loaded from a home directory, which is what
//! lets a test drive every skip and the whole ordering with no socket and no
//! temporary home.
//!
//! No HTTP vocabulary crosses into it either. The choice is made over the types
//! [`mod@crate::linear`] already parsed, so a change to how the board spells an
//! answer cannot change which ticket is next.
//!
//! [`take_named`] is the other way in, for when a person says which ticket, and
//! it is the one thing here that needs the board: a ticket nobody chose still has
//! to be read before anything can be judged about it. What is judged is the same
//! four rules, in the same function, so the two doors lead to the same room.
//!
//! [`scope_queue`]: crate::Board::scope_queue

use std::cmp::Reverse;
use std::collections::HashMap;
use std::fmt;

use warlock_engine::{PullRun, RunStatus, ScopeRecord};

use crate::linear::{Assignee, Blocker, Board, Error as LinearError, Priority, Queue, QueuedIssue};

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

        if let Some(reason) = unavailable(issue, review_state, status) {
            skipped.push(skip(issue, reason));
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

/// Why this issue cannot be worked, or `None` when it can: the four rules, in
/// the one place both doors read them from.
///
/// Shared rather than copied because the promise the brief makes about a named
/// ticket is that it is held to the queue's rules — a second spelling of them
/// beside [`take_named`] would be a second opinion to drift, and the drift would
/// show up as warlock working a ticket somebody else's run is holding.
///
/// The order of the four is the promise, for the reasons on [`choose`].
fn unavailable(
    issue: &QueuedIssue,
    review_state: &str,
    status: Option<RunStatus>,
) -> Option<Reason> {
    if status == Some(RunStatus::Halted) {
        return Some(Reason::Halted {
            ticket: issue.identifier().to_owned(),
        });
    }

    if named(issue.state(), review_state) {
        return Some(Reason::InReview {
            state: issue.state().to_owned(),
        });
    }

    if status.is_none() && named(issue.state(), IN_PROGRESS) {
        return Some(Reason::InProgressElsewhere);
    }

    let open: Vec<Blocker> = issue
        .blockers()
        .iter()
        .filter(|blocker| !blocker.state_type().settled())
        .cloned()
        .collect();

    (!open.is_empty()).then_some(Reason::Blocked { blockers: open })
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
    split(identifier).map(|(_, number)| number)
}

/// A named ticket, taken or turned down.
///
/// Two outcomes and not a `Result`, because neither of these is a failure: the
/// board answered, and the answer was either the ticket or a reason. A
/// [`LinearError`] is what failure looks like, and it stays in the `Result` around
/// this.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Named {
    /// The ticket, held to every rule [`choose`] holds a queue to.
    Taken(QueuedIssue),
    Refused(Refusal),
}

/// Why a named ticket cannot be worked.
///
/// The first three are what the queue's filters would have answered silently by
/// simply not returning it, which is the whole reason the named path reads the
/// ticket unfiltered: each one names what failed. The rest are the queue's own
/// rules, which a named ticket is held to exactly as a chosen one is — so
/// [`Refusal::NotReady`] carries a [`Reason`] rather than restating it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// Not something a ticket could be called, so nothing was asked of the board.
    NotATicket { ticket: String },
    /// The board has no ticket by that name: a team key nobody uses, or a number
    /// that team has not reached.
    Unknown { ticket: String },
    /// On another team than the one the scope record routes to, which is the
    /// record's `team` against the ticket's own team key.
    NotOnTeam { team: String, found: String },
    /// On the right team without the record's `label`, so it is not this scope's
    /// work. Carries every label it does have: the usual cause is a ticket nobody
    /// labelled, and the next is a near miss.
    Unlabelled { label: String, carried: Vec<String> },
    /// Somebody else's, or nobody's. `pull` works the operator's own tickets and
    /// there is no way past this: taking a teammate's ticket is a reassignment a
    /// human makes on the board.
    NotYours { holder: Option<String> },
    /// Already `completed` or `canceled`. Named separately from the queue's rules
    /// because a finished ticket is missing from the queue rather than skipped in
    /// it, and "not on your team" would be a lie about it.
    Finished { state: String },
    /// In the queue and not available, for one of the four reasons anything in a
    /// queue is not available.
    NotReady(Reason),
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotATicket { ticket } => {
                write!(
                    f,
                    "`{ticket}` is not a ticket identifier, which reads like `WAR-9`"
                )
            }
            Self::Unknown { ticket } => write!(f, "the board has no `{ticket}`"),
            Self::NotOnTeam { team, found } => {
                write!(f, "on team `{found}`, and this scope routes to `{team}`")
            }
            Self::Unlabelled { label, carried } => write!(
                f,
                "not labelled `{label}` — it carries {}",
                if carried.is_empty() {
                    "no labels at all".to_owned()
                } else {
                    carried.join(", ")
                }
            ),
            Self::NotYours { holder } => match holder {
                Some(holder) => write!(f, "assigned to {holder} and not to you"),
                None => f.write_str("assigned to nobody, and `pull` works your own tickets"),
            },
            Self::Finished { state } => write!(f, "in `{state}`, which is finished"),
            Self::NotReady(reason) => reason.fmt(f),
        }
    }
}

/// The ticket a person named, if the scope's rules leave it workable.
///
/// One request, and the design tension it settles is worth writing down. The
/// queue's three filters — team, label, assignee — run on Linear's side, so a
/// ticket that is merely *absent* from the queue cannot say which of them it
/// failed: "not in your queue" is equally true of a teammate's ticket, an
/// unlabelled one, and one on another team. There were two honest ways out. Read
/// the queue and then look the missing identifier up to diagnose it, which is two
/// requests and still cannot work a ticket sitting past the queue's one page; or
/// read the named ticket on its own with the queue's node selection and none of
/// its filters, and check the three here — which is this. The diagnosis is then a
/// pure check over facts the board stated, one request answers one question, and
/// a ticket beyond the first page of a busy queue is still nameable. No retry, no
/// second page.
///
/// What is checked after the three is [`unavailable`], the same function
/// [`choose`] skips by, so a named ticket can only be refused for a reason the
/// chooser would have skipped it for. The three come first, in this order: an
/// issue label belongs to a team in Linear, so a ticket on the wrong team cannot
/// be carrying this team's label either, and naming the label would send somebody
/// to fix the wrong thing.
///
/// There is no way here to work somebody else's ticket. The assignee is checked
/// by id against the user the key belongs to, and nothing takes a flag past it.
pub fn take_named(
    board: &impl Board,
    record: &ScopeRecord,
    assignee: &str,
    ticket: &str,
    runs: &[PullRun],
) -> Result<Named, LinearError> {
    let Some((team, number)) = split(ticket) else {
        return Ok(Named::Refused(Refusal::NotATicket {
            ticket: ticket.to_owned(),
        }));
    };

    let Some(found) = board.named_issue(team, number)? else {
        return Ok(Named::Refused(Refusal::Unknown {
            ticket: ticket.to_owned(),
        }));
    };

    if !named(found.team(), record.team()) {
        return Ok(Named::Refused(Refusal::NotOnTeam {
            team: record.team().to_owned(),
            found: found.team().to_owned(),
        }));
    }

    if !found
        .labels()
        .iter()
        .any(|label| named(label, record.label()))
    {
        return Ok(Named::Refused(Refusal::Unlabelled {
            label: record.label().to_owned(),
            carried: found.labels().to_vec(),
        }));
    }

    // By id and never by name: two people in a workspace can be called the same
    // thing, and the id is what the key itself answered. Compared exactly for the
    // same reason — an opaque id that needs folding to match is not a match.
    if found.assignee().map(Assignee::id) != Some(assignee) {
        return Ok(Named::Refused(Refusal::NotYours {
            holder: found.assignee().map(|holder| holder.name().to_owned()),
        }));
    }

    if found.issue().state_type().settled() {
        return Ok(Named::Refused(Refusal::Finished {
            state: found.issue().state().to_owned(),
        }));
    }

    let status = run_for(runs, found.issue().identifier()).map(PullRun::status);

    if let Some(reason) = unavailable(found.issue(), record.review_state(), status) {
        return Ok(Named::Refused(Refusal::NotReady(reason)));
    }

    Ok(Named::Taken(found.into_issue()))
}

/// `WAR-133` as the team key and the number, which is the shape Linear can be
/// asked for one ticket in: its `IssueFilter` has no `identifier`, because an
/// identifier is a display name made of those two.
///
/// `None` for anything that is not built that way, which is a refusal rather than
/// a request: `banana` is not a ticket, and asking the board about it would only
/// turn a typo into a round trip and a vaguer answer.
fn split(ticket: &str) -> Option<(&str, u64)> {
    let (team, number) = ticket.trim().rsplit_once('-')?;
    let team = team.trim();

    (!team.is_empty()).then_some((team, number.trim().parse().ok()?))
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
