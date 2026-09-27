//! Every seam one pull spends arrives in [`Pulling`], and nothing in this module
//! reaches for one: no environment variable is read, no socket is opened, no
//! process is spawned and no [`Standing`](crate::standing::Standing) is
//! resolved. That is what lets a whole run — each halt and the crossing included
//! — be driven by values, with no board, no repository and no `claude` on the
//! machine.
//!
//! The tree check is deliberately not a seam of its own. What a session wrote is
//! read through [`Repository::dirty`] — already a seam — and judged by
//! [`crossings_in`], which is a function over values; what the loop cannot read
//! for itself is the manifest and the sigils this machine holds, so those arrive
//! as fields beside the rest. A trait wrapping the pair was the alternative, and
//! it would have had to answer with an owned mirror of [`Crossings`]: a second
//! record of one finding, which [`pull_request_body`] then wants borrowed again.
//!
//! [`next_runnable`] and [`halt_comment`] are the two decisions a run makes that
//! are not requests, and both are functions of a [`PullRun`] alone. The record is
//! the whole input because the record is the whole state — it is written before
//! and after every sub-task and read back by the next invocation, minutes or days
//! later — so a second source for either would be a second opinion about a run
//! that somebody's working tree is holding.
//!
//! The record is machine-local: it lives under the home directory and never
//! inside the repository, which is why [`Pulling`] carries both paths. A file
//! recording how far a pull got would otherwise turn up in the diff of the very
//! commit it describes.
//!
//! The activity port is the caller's to bridge. [`Activities`] takes a
//! `Fn(Activity) + Send + Sync + 'static` and the progress sink here is a
//! `&mut dyn FnMut`, so a door that wants a session's activities on its
//! [`PullEvent`] stream owns whatever joins the two.
//!
//! [`Activities`]: warlock_tui::Activities
//! [`Crossings`]: warlock_tui::Crossings
//! [`Repository::dirty`]: warlock_tui::Repository::dirty
//! [`crossings_in`]: warlock_tui::crossings_in
//! [`pull_request_body`]: warlock_tui::pull_request_body

// The loop that spends these is not here yet, and a seam nothing calls is dead
// code to the bin target however thoroughly the tests here drive it. An `expect`
// rather than an `allow` so that the day everything below has a caller, this
// line is the compile error that asks to be deleted.
#![expect(
    dead_code,
    reason = "the seams and the decisions land before the loop that spends them"
)]

use std::fmt::Write as _;
use std::path::Path;

use warlock_engine::{Manifest, PullRun, PullSubtask, ScopeRecord, SubtaskStatus};
use warlock_tui::{Activity, Board, Forge, Repository, Split, Worked};

/// Everything one pull is allowed to touch, built by whichever door is pulling.
///
/// Both halves of git are here because only one of them may be missing: a
/// [`Repository`] has to work, and a [`Forge`] may come back with no `gh` at all.
/// The board arrives opened, so no key value reaches this module and none can be
/// printed from it.
///
/// No `Debug`, for `running.rs`'s `Progress`'s reason: a sink is not a value to
/// print, and the rest of this is paths and seams a failing assertion would dump
/// a screenful of.
pub(crate) struct Pulling<'a, B: Board, R: Repository, F: Forge, S: Splits, W: Works> {
    pub(crate) board: &'a B,
    pub(crate) repo: &'a R,
    pub(crate) forge: &'a F,
    pub(crate) split: &'a S,
    pub(crate) sessions: &'a W,
    /// The scope the ticket was pulled under, whole: the team to file against,
    /// the label the queue is read by, and the state a finished run moves the
    /// ticket to are all on it, and asking the manifest again here would be a
    /// second reading of a record the door already resolved.
    pub(crate) scope: &'a ScopeRecord,
    /// What the check after a session judges a changed path against, with
    /// `held`. The manifest in hand rather than one loaded here, as
    /// [`descend`](crate::descent::descend) takes one: reading it again could
    /// disagree with the boundary the door already judged.
    pub(crate) manifest: &'a Manifest,
    /// The flattened sigils this machine holds, as
    /// [`crossings_in`](warlock_tui::crossings_in) and
    /// [`permits`](crate::boundary::permits) take them — a config that would not
    /// parse is the door's to say and not a third answer to give in here.
    pub(crate) held: &'a [String],
    pub(crate) root: &'a Path,
    /// Where the run record goes, never under [`root`](Self::root).
    pub(crate) home: &'a Path,
    pub(crate) progress: &'a mut dyn FnMut(PullEvent),
}

impl<B: Board, R: Repository, F: Forge, S: Splits, W: Works> Pulling<'_, B, R, F, S, W> {
    pub(crate) fn report(&mut self, event: PullEvent) {
        (self.progress)(event);
    }
}

/// The splitting session, behind a seam.
///
/// Takes the ticket rather than being built around it, because the ticket is a
/// parameter of the run and the seams are built before one is chosen. One call,
/// one [`Split`], and no `Result`: a session that never answered is already one
/// of [`Split::Halted`]'s endings.
pub(crate) trait Splits {
    fn split(&self, ticket: &str, title: &str, description: &str) -> Split;
}

/// One sub-task session per call.
///
/// A factory and not a session: a run opens one session per sub-task and drops it
/// at the end of the sub-task, so nothing one sub-task said reaches the next —
/// which is what [`working_opening`](warlock_tui::working_opening) is written on
/// the assumption of.
///
/// The retries are the implementation's, not the caller's:
/// [`Working::run`](warlock_tui::Working::run) already owns which stopping earns
/// another attempt and what a turn limit does to the next one, and a loop that
/// asked again itself would be a second retry policy on top of that one.
pub(crate) trait Works {
    fn work(&self, opening: &str) -> Worked;
}

/// What a whole pull came to, in the three endings that cost different money.
///
/// The [`status`](Pulled::status) is here rather than worked out at the door for
/// the reason the enum exists: a crossing and a halt both leave a branch with
/// commits on it and a ticket with a comment, and the only thing that tells them
/// apart afterwards is which variant was answered with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Pulled {
    /// The run reached a pull request. `None` is
    /// [`Opened::NoGh`](warlock_tui::Opened::NoGh), which is still a run that
    /// did the work: the branch is pushed and the body is on the ticket.
    Opened { ticket: String, url: Option<String> },
    /// Nothing was runnable, and the ticket already carries
    /// [`halt_comment`]'s account of why.
    Halted { ticket: String },
    /// A session wrote under a scope this machine does not hold. Named apart
    /// from a halt because the tree still holds that work uncommitted, and
    /// because it is the one halt that is a boundary rather than a failure.
    Crossed { ticket: String, subtask: String },
}

impl Pulled {
    /// What the shell spends on it.
    ///
    /// The numbers are `main.rs`'s `status_for` ones and mean what they mean
    /// there: **0** the question was answered, **1** warlock could not do it,
    /// **3** a boundary this machine's sigils do not open. A crossing is a **3**
    /// for that last reason and not because it is worse than a halt.
    pub(crate) const fn status(&self) -> u8 {
        match self {
            Self::Opened { .. } => 0,
            Self::Halted { .. } => 1,
            Self::Crossed { .. } => 3,
        }
    }

    pub(crate) fn ticket(&self) -> &str {
        match self {
            Self::Opened { ticket, .. }
            | Self::Halted { ticket }
            | Self::Crossed { ticket, .. } => ticket,
        }
    }
}

/// What a run is seen doing, for whichever door is watching.
///
/// A [`Heading`] opens a section and everything else belongs to the section open
/// at the time, so which events start one is a fact about the type rather than a
/// convention each door has to learn again.
///
/// The two board lines are variants rather than failures because neither is one:
/// a team with no state to move the ticket to has a ticket that does not move,
/// and a run that stopped over it would be the board wagging the pull.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum PullEvent {
    Heading(Heading),
    /// The session's own, carried rather than worded: two doors word one
    /// differently — a line on a pipe and a
    /// [`Section`](warlock_tui::Section) on the panel's account card — and a
    /// line rendered in here would be the shell's wording sent to both.
    Activity(Activity),
    /// The team has no workflow state named
    /// [`IN_PROGRESS`](warlock_tui::IN_PROGRESS). No state is carried because
    /// there is only one spelling this looks for; the review state's is the
    /// scope record's, so that one is said.
    NoStartState {
        team: String,
    },
    NoReviewState {
        team: String,
        state: String,
    },
}

/// The three sections a run has, and the whole of what opens one.
///
/// The refresh has no heading here. It is one clearly named call site in the
/// finish, with nothing behind it yet, and a section announced for a pass that
/// does not run would be a run reporting work it did not do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Heading {
    Split {
        ticket: String,
        title: String,
    },
    /// The fraction is one-based and its denominator is the split's answer, as
    /// [`RunEvent::Starting`](crate::descent::RunEvent)'s is: a sub-task that is
    /// re-attempted does not move it, because the run's size is not a running
    /// total.
    Subtask {
        id: String,
        goal: String,
        position: usize,
        total: usize,
    },
    PullRequest {
        branch: String,
    },
}

/// The sub-task to work next: the first [`Pending`](SubtaskStatus::Pending) one
/// every dependency of which is [`Done`](SubtaskStatus::Done), or `None` when
/// nothing is runnable.
///
/// In the record's own order, which is the split's: `number` put every sub-task
/// after everything it waits on, so the first runnable one found walking forward
/// is the one the split meant to come next.
///
/// A `depends_on` naming a sub-task the run does not hold counts as unmet. The
/// record is hand-editable — it is read by a person beside a halted run — and the
/// two readings differ only for a file somebody has mistyped, where refusing to
/// run is a halt the operator is told about and the other road is warlock
/// building on work that was never done.
pub(crate) fn next_runnable(run: &PullRun) -> Option<&PullSubtask> {
    run.subtasks().iter().find(|subtask| {
        *subtask.status() == SubtaskStatus::Pending
            && subtask.depends_on().iter().all(|id| {
                run.subtask(id)
                    .is_some_and(|needed| *needed.status() == SubtaskStatus::Done)
            })
    })
}

/// The one comment a halted run leaves on its ticket: what finished, what
/// stopped and why, what never started, and the two commands that carry the run
/// on.
///
/// The two commands are both said, in order, because neither does the other's
/// job: `resume` is the human saying they have looked, and it puts the stopped
/// sub-tasks back to `pending` without working any of them; `pull --ticket` is
/// what works them. A comment naming only the first would leave a released run
/// waiting for a queue pass that will not choose it ahead of anything.
///
/// A section with nothing in it is absent rather than an empty heading, as
/// [`pull_request_body`](warlock_tui::pull_request_body) leaves one out: a run
/// that halted on its first sub-task has nothing finished, and a heading saying
/// so is the bulk of the comment.
pub(crate) fn halt_comment(run: &PullRun) -> String {
    let mut comment = format!(
        "This pull halted, so the ticket has not moved. The branch `{}` holds one \
         commit per finished sub-task and nothing else was committed.",
        run.branch()
    );

    section(&mut comment, "Finished", run, |status| {
        *status == SubtaskStatus::Done
    });
    // Everything that is neither finished nor waiting, which is `blocked`,
    // `failed`, `crossed` — and `in_progress`, the status a run killed mid-session
    // leaves behind. That one has no reason to give, and it belongs here anyway:
    // a sub-task warlock started and cannot account for is a thing to look at,
    // not a thing to list as never started.
    section(&mut comment, "Stopped", run, |status| {
        !matches!(status, SubtaskStatus::Done | SubtaskStatus::Pending)
    });
    section(&mut comment, "Not started", run, |status| {
        *status == SubtaskStatus::Pending
    });

    let _ = write!(
        comment,
        "\n\n`warlock resume {ticket}` puts the stopped sub-tasks back, and then \
         `warlock pull {scope} --ticket {ticket}` works the ticket again.",
        ticket = run.ticket(),
        scope = run.scope(),
    );
    comment
}

fn section(
    comment: &mut String,
    heading: &str,
    run: &PullRun,
    wanted: impl Fn(&SubtaskStatus) -> bool,
) {
    let listed: Vec<&PullSubtask> = run
        .subtasks()
        .iter()
        .filter(|subtask| wanted(subtask.status()))
        .collect();
    if listed.is_empty() {
        return;
    }

    let _ = write!(comment, "\n\n## {heading}");
    for subtask in listed {
        let _ = write!(
            comment,
            "\n\n- `{}` {}",
            subtask.id(),
            subtask.goal().trim()
        );
        // The status is spelled on the stopped lines alone. On a finished line it
        // would be the heading said twice, and on a not-started line `pending` is
        // what "not started" means.
        let status = subtask.status();
        if !matches!(status, SubtaskStatus::Done | SubtaskStatus::Pending) {
            let _ = write!(comment, " — `{}`", status.as_str());
            if let Some(reason) = status.reason() {
                let _ = write!(comment, ": {}", reason.trim());
            }
        }
    }
}

#[cfg(test)]
#[path = "tests/pulling.rs"]
mod tests;
