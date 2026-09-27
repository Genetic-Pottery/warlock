// Every key name and status spelling in this module is a wire format. A run
// record is written by one invocation of a pull and read back by the next one,
// minutes or days later and possibly by a different build, and a person reads it
// beside `.forman/<TICKET>/state.json`, which spells the same keys. Renaming
// `pr_url`, `pulled_at`, `session_id`, `cost_usd` or any status is therefore not
// a refactor: it strands every run in progress on every machine, and the run it
// strands is one holding uncommitted work in somebody's tree.

use std::fmt;

use serde::{Deserialize, Serialize};

/// ```
/// use warlock_engine::{PullRun, PullSubtask, RunStatus, SubtaskStatus};
///
/// let mut run = PullRun::new(
///     "WAR-140",
///     "The Linear queue query",
///     "warlock-team",
///     "war-140/the-linear-queue-query",
///     "2026-09-27T06:21:55+00:00",
/// );
/// run.push_subtask(PullSubtask::new("WAR-140.01", "Add the issues query", ["WAR-140.02"]));
///
/// assert_eq!(run.status(), RunStatus::Pulled);
/// assert_eq!(run.pr_url(), None);
/// assert_eq!(run.subtask("WAR-140.01").map(PullSubtask::status), Some(&SubtaskStatus::Pending));
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PullRun {
    ticket: String,
    title: String,
    scope: String,
    status: RunStatus,
    branch: String,
    // Supplied by the caller and never read from a clock here, as
    // `FiledRecord::filed_at` is and for the same reason: `clock::now_rfc3339`
    // called inside this module would make every test of it depend on the wall
    // clock, and the instant a ticket was pulled belongs to the caller that
    // pulled it.
    pulled_at: String,
    // Written as `null` rather than skipped while there is no pull request, so
    // the object has one shape for its whole life: a record gaining a key
    // halfway through a run is a diff nobody can read, and `state.json` is
    // rewritten before and after every sub-task.
    pr_url: Option<String>,
    subtasks: Vec<PullSubtask>,
}

impl PullRun {
    #[must_use]
    pub fn new(
        ticket: impl Into<String>,
        title: impl Into<String>,
        scope: impl Into<String>,
        branch: impl Into<String>,
        pulled_at: impl Into<String>,
    ) -> Self {
        Self {
            ticket: ticket.into(),
            title: title.into(),
            scope: scope.into(),
            status: RunStatus::Pulled,
            branch: branch.into(),
            pulled_at: pulled_at.into(),
            pr_url: None,
            subtasks: Vec::new(),
        }
    }

    #[must_use]
    pub fn with_subtasks(mut self, subtasks: impl IntoIterator<Item = PullSubtask>) -> Self {
        self.subtasks = subtasks.into_iter().collect();
        self
    }

    #[must_use]
    pub fn ticket(&self) -> &str {
        &self.ticket
    }

    #[must_use]
    pub fn title(&self) -> &str {
        &self.title
    }

    #[must_use]
    pub fn scope(&self) -> &str {
        &self.scope
    }

    #[must_use]
    pub const fn status(&self) -> RunStatus {
        self.status
    }

    #[must_use]
    pub fn branch(&self) -> &str {
        &self.branch
    }

    #[must_use]
    pub fn pulled_at(&self) -> &str {
        &self.pulled_at
    }

    #[must_use]
    pub fn pr_url(&self) -> Option<&str> {
        self.pr_url.as_deref()
    }

    #[must_use]
    pub fn subtasks(&self) -> &[PullSubtask] {
        &self.subtasks
    }

    pub const fn set_status(&mut self, status: RunStatus) {
        self.status = status;
    }

    pub fn set_pr_url(&mut self, url: impl Into<String>) {
        self.pr_url = Some(url.into());
    }

    pub fn push_subtask(&mut self, subtask: PullSubtask) {
        self.subtasks.push(subtask);
    }

    #[must_use]
    pub fn subtask(&self, id: &str) -> Option<&PullSubtask> {
        self.subtasks.iter().find(|subtask| subtask.id == id)
    }

    /// ```
    /// use warlock_engine::{PullRun, PullSubtask, SubtaskStatus};
    ///
    /// let mut run = PullRun::new("WAR-140", "A ticket", "warlock-team", "war-140/a-ticket", "now")
    ///     .with_subtasks([PullSubtask::new("WAR-140.01", "A goal", [] as [&str; 0])]);
    ///
    /// let subtask = run.subtask_mut("WAR-140.01").expect("the sub-task is in the run");
    /// subtask.set_status(SubtaskStatus::Blocked("`crates/control` is scoped control-plane".into()));
    /// subtask.set_session_id("5b80b301-8d38-451e-9f55-0034e0877152");
    /// subtask.set_cost_usd(2.384_245);
    ///
    /// let subtask = run.subtask("WAR-140.01").expect("the sub-task is in the run");
    /// assert_eq!(subtask.status().reason(), Some("`crates/control` is scoped control-plane"));
    /// assert_eq!(subtask.cost_usd(), Some(2.384_245));
    /// ```
    pub fn subtask_mut(&mut self, id: &str) -> Option<&mut PullSubtask> {
        self.subtasks.iter_mut().find(|subtask| subtask.id == id)
    }

    pub fn subtasks_mut(&mut self) -> &mut [PullSubtask] {
        &mut self.subtasks
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(into = "WireSubtask", try_from = "WireSubtask")]
pub struct PullSubtask {
    id: String,
    goal: String,
    status: SubtaskStatus,
    depends_on: Vec<String>,
    log: Option<String>,
    started_at: Option<String>,
    finished_at: Option<String>,
    // Set through `set_session_id` and `set_cost_usd` whatever the outcome, not
    // only on a `done` sub-task: a session that spent four dollars before
    // reporting itself blocked spent them, and a record that drops the cost of
    // every failure under-reports exactly the runs worth looking at.
    session_id: Option<String>,
    cost_usd: Option<f64>,
}

impl PullSubtask {
    #[must_use]
    pub fn new(
        id: impl Into<String>,
        goal: impl Into<String>,
        depends_on: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        Self {
            id: id.into(),
            goal: goal.into(),
            status: SubtaskStatus::Pending,
            depends_on: depends_on.into_iter().map(Into::into).collect(),
            log: None,
            started_at: None,
            finished_at: None,
            session_id: None,
            cost_usd: None,
        }
    }

    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }

    #[must_use]
    pub fn goal(&self) -> &str {
        &self.goal
    }

    #[must_use]
    pub const fn status(&self) -> &SubtaskStatus {
        &self.status
    }

    #[must_use]
    pub fn depends_on(&self) -> &[String] {
        &self.depends_on
    }

    #[must_use]
    pub fn log(&self) -> Option<&str> {
        self.log.as_deref()
    }

    #[must_use]
    pub fn started_at(&self) -> Option<&str> {
        self.started_at.as_deref()
    }

    #[must_use]
    pub fn finished_at(&self) -> Option<&str> {
        self.finished_at.as_deref()
    }

    #[must_use]
    pub fn session_id(&self) -> Option<&str> {
        self.session_id.as_deref()
    }

    #[must_use]
    pub const fn cost_usd(&self) -> Option<f64> {
        self.cost_usd
    }

    pub fn set_status(&mut self, status: SubtaskStatus) {
        self.status = status;
    }

    pub fn set_log(&mut self, log: impl Into<String>) {
        self.log = Some(log.into());
    }

    pub fn set_started_at(&mut self, at: impl Into<String>) {
        self.started_at = Some(at.into());
    }

    pub fn set_finished_at(&mut self, at: impl Into<String>) {
        self.finished_at = Some(at.into());
    }

    pub fn set_session_id(&mut self, session_id: impl Into<String>) {
        self.session_id = Some(session_id.into());
    }

    pub const fn set_cost_usd(&mut self, cost: f64) {
        self.cost_usd = Some(cost);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Pulled,
    InProgress,
    Halted,
    Resumed,
    InReview,
}

impl RunStatus {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pulled => "pulled",
            Self::InProgress => "in_progress",
            Self::Halted => "halted",
            Self::Resumed => "resumed",
            Self::InReview => "in_review",
        }
    }
}

impl fmt::Display for RunStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// ```
/// use warlock_engine::SubtaskStatus;
///
/// let crossed = SubtaskStatus::Crossed("wrote crates/control/src/lib.rs".to_owned());
///
/// assert_eq!(crossed.as_str(), "crossed");
/// assert_eq!(crossed.reason(), Some("wrote crates/control/src/lib.rs"));
/// assert_eq!(SubtaskStatus::Done.reason(), None);
/// ```
// The reason is carried by the variant rather than sitting in a field beside the
// status, so `blocked` with nothing to say cannot be constructed at all — no
// builder, no setter and no deserialised file can produce one. A
// `{ status, reason: Option<String> }` pair was the alternative, and it makes
// the halt comment on the ticket the place the omission shows up, hours after
// the session that knew the reason has gone.
//
// It carries no `Serialize`/`Deserialize` of its own on purpose: on the wire the
// status and its reason are two sibling keys of the sub-task object, `status`
// and `blocked_reason`, so this enum has no standalone form. `WireSubtask`
// below is where the two halves are joined and split.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum SubtaskStatus {
    Pending,
    InProgress,
    Done,
    Blocked(String),
    Failed(String),
    Crossed(String),
}

impl SubtaskStatus {
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::InProgress => "in_progress",
            Self::Done => "done",
            Self::Blocked(_) => "blocked",
            Self::Failed(_) => "failed",
            Self::Crossed(_) => "crossed",
        }
    }

    #[must_use]
    pub fn reason(&self) -> Option<&str> {
        match self {
            Self::Pending | Self::InProgress | Self::Done => None,
            Self::Blocked(reason) | Self::Failed(reason) | Self::Crossed(reason) => Some(reason),
        }
    }
}

impl fmt::Display for SubtaskStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())?;
        match self.reason() {
            Some(reason) => write!(f, ": {reason}"),
            None => Ok(()),
        }
    }
}

/// A `blocked`, `failed` or `crossed` sub-task in a file with no reason beside
/// it. Refused rather than read as reasonless, which is the whole point of the
/// data-carrying variants above: a status the type cannot hold is a record the
/// operator has to be told about, not one to quietly downgrade.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReasonMissing {
    id: String,
    status: &'static str,
}

impl ReasonMissing {
    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }

    #[must_use]
    pub const fn status(&self) -> &str {
        self.status
    }
}

impl fmt::Display for ReasonMissing {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "sub-task `{}` is `{}` with no `blocked_reason`",
            self.id, self.status
        )
    }
}

impl std::error::Error for ReasonMissing {}

// The three reason-carrying statuses share one key rather than gaining a
// `failed_reason` and a `crossed_reason`: `blocked_reason` is what a sub-task
// session's own last message answers with, and what `.forman/<TICKET>/state.json`
// spells, so one key keeps the record and the answer it came from readable
// against each other.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct WireSubtask {
    id: String,
    goal: String,
    status: WireStatus,
    #[serde(default)]
    depends_on: Vec<String>,
    blocked_reason: Option<String>,
    log: Option<String>,
    started_at: Option<String>,
    finished_at: Option<String>,
    session_id: Option<String>,
    cost_usd: Option<f64>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum WireStatus {
    Pending,
    InProgress,
    Done,
    Blocked,
    Failed,
    Crossed,
}

impl WireStatus {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::InProgress => "in_progress",
            Self::Done => "done",
            Self::Blocked => "blocked",
            Self::Failed => "failed",
            Self::Crossed => "crossed",
        }
    }
}

impl From<PullSubtask> for WireSubtask {
    fn from(subtask: PullSubtask) -> Self {
        let (status, blocked_reason) = match subtask.status {
            SubtaskStatus::Pending => (WireStatus::Pending, None),
            SubtaskStatus::InProgress => (WireStatus::InProgress, None),
            SubtaskStatus::Done => (WireStatus::Done, None),
            SubtaskStatus::Blocked(reason) => (WireStatus::Blocked, Some(reason)),
            SubtaskStatus::Failed(reason) => (WireStatus::Failed, Some(reason)),
            SubtaskStatus::Crossed(reason) => (WireStatus::Crossed, Some(reason)),
        };
        Self {
            id: subtask.id,
            goal: subtask.goal,
            status,
            depends_on: subtask.depends_on,
            blocked_reason,
            log: subtask.log,
            started_at: subtask.started_at,
            finished_at: subtask.finished_at,
            session_id: subtask.session_id,
            cost_usd: subtask.cost_usd,
        }
    }
}

impl TryFrom<WireSubtask> for PullSubtask {
    type Error = ReasonMissing;

    fn try_from(wire: WireSubtask) -> Result<Self, Self::Error> {
        let WireSubtask {
            id,
            goal,
            status,
            depends_on,
            blocked_reason,
            log,
            started_at,
            finished_at,
            session_id,
            cost_usd,
        } = wire;

        let status = match (status, blocked_reason) {
            // A reason beside a status that does not carry one is dropped
            // rather than refused: the key is written as `null` for every
            // sub-task, so a stray reason is most likely a hand edit or the
            // leftover of a status somebody reset, and there is nowhere in the
            // type to put it. Refusing would strand a record mid-run over a
            // field nothing reads.
            (WireStatus::Pending, _) => SubtaskStatus::Pending,
            (WireStatus::InProgress, _) => SubtaskStatus::InProgress,
            (WireStatus::Done, _) => SubtaskStatus::Done,
            (WireStatus::Blocked, Some(reason)) => SubtaskStatus::Blocked(reason),
            (WireStatus::Failed, Some(reason)) => SubtaskStatus::Failed(reason),
            (WireStatus::Crossed, Some(reason)) => SubtaskStatus::Crossed(reason),
            (status @ (WireStatus::Blocked | WireStatus::Failed | WireStatus::Crossed), None) => {
                return Err(ReasonMissing {
                    id,
                    status: status.as_str(),
                });
            }
        };

        Ok(Self {
            id,
            goal,
            status,
            depends_on,
            log,
            started_at,
            finished_at,
            session_id,
            cost_usd,
        })
    }
}

#[cfg(test)]
#[path = "tests/pulls.rs"]
mod tests;
