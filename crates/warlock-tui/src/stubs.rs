//! No agent here stops a turn on the say-when, reports activities, or records
//! the model it was raised at, and that is what says which tests they are not
//! for: cancelling a run, attaching the activity port, and the argv a brief
//! runs at are facts about a child process, asserted on the real adapter in
//! `pacting.rs` and `chatting.rs`. A second record of them here would be a copy
//! for the two to disagree over.
//!
//! [`Scripted`] keeping the handles it was wired to is not that record: what it
//! answers never depends on them, and what they buy is the one question the
//! real adapter cannot be asked from the panel's side — whether the session in
//! flight was told to stop.

use std::collections::VecDeque;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};

use warlock_engine::splitting::Numbered;
use warlock_engine::{Agent, agent, drafting, stub_answer, working};
use warlock_tui::{
    Activities, Activity, Board, Cancel, Commit, Converses, Dirty, FetchedProject, Forge, GitError,
    LinearError, LinearIssue, LinearProject, NamedIssue, NewIssue, NewProject, Opened, Opens,
    PullRequest, Queue, Repository, Split, Wired, Worked,
};

use crate::clipboard::Clip;
use crate::pulling::{Splits, Works};

// A clipboard nothing on the machine has to provide. The refusal is kept as the
// description rather than as an `arboard::Error`, because that type is not
// `Clone` and a stand-in that could only fail once would be a stand-in that
// silently succeeded on the second copy.
#[derive(Debug, Default)]
pub(crate) struct Copying {
    refusing: Option<String>,
    copied: Vec<String>,
}

impl Copying {
    pub(crate) fn taking() -> Self {
        Self::default()
    }

    pub(crate) fn refusing(description: impl Into<String>) -> Self {
        Self {
            refusing: Some(description.into()),
            copied: Vec::new(),
        }
    }

    pub(crate) fn copied(&self) -> &[String] {
        &self.copied
    }
}

impl Clip for Copying {
    fn put(&mut self, text: &str) -> Result<(), arboard::Error> {
        if let Some(description) = &self.refusing {
            return Err(arboard::Error::Unknown {
                description: description.clone(),
            });
        }
        self.copied.push(text.to_owned());
        Ok(())
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct Passing;

impl Passing {
    pub(crate) fn filling() -> Self {
        Self
    }
}

impl Agent for Passing {
    fn run(&self, request: &agent::Request) -> Result<agent::Response, agent::Error> {
        Ok(agent::Response::new(stub_answer(request)))
    }
}

impl Wired for Passing {
    fn wired(&self, _cancel: Cancel, _activities: Activities) -> Self {
        *self
    }
}

#[derive(Debug, Clone)]
pub(crate) struct Saying {
    answer: String,
}

impl Saying {
    pub(crate) fn answering(answer: impl Into<String>) -> Self {
        Self {
            answer: answer.into(),
        }
    }
}

impl Wired for Saying {
    fn wired(&self, _cancel: Cancel, _activities: Activities) -> Self {
        self.clone()
    }
}

impl Converses for Saying {
    fn turn(&self, _message: &str) -> Result<String, agent::Error> {
        Ok(self.answer.clone())
    }

    fn raised(&self, _model: &str, _effort: &str) -> Self {
        self.clone()
    }
}

/// One turn's answer, written down before the run starts.
///
/// Prose and the drafts object are one variant because the session decides
/// which is which — prose is a question while a round is left and a failed
/// attempt once the asking is over — and a stand-in that named the two apart
/// would be a second opinion about what it had just said.
#[derive(Debug, Clone)]
pub(crate) enum Answering {
    Says(String),
    /// No `claude` on the machine, which is one of the three ways a session's
    /// turn ends in a failure rather than an answer.
    Missing,
}

impl Answering {
    pub(crate) fn says(text: impl Into<String>) -> Self {
        Self::Says(text.into())
    }

    /// The drafting road's own stub object, named for the slice it stands in
    /// for: two drafts inside every cap, so it is accepted rather than
    /// repaired.
    pub(crate) fn drafts(slice: &str) -> Self {
        Self::Says(drafting::stub_answer(slice))
    }

    pub(crate) const fn missing() -> Self {
        Self::Missing
    }
}

/// A model that answers a written-down sequence, one entry per turn, and keeps
/// what it was asked and every cancel handle it was wired to.
///
/// The cancels are the point of the record: a drafting session mints its own
/// and wires its agent to it, so a test that wants to know whether quitting
/// reached the turn in flight has nowhere else to look.
///
/// `Arc<Mutex<_>>` because a slice's turn runs on a worker thread, so a model
/// that could not cross one would not stand in for the thing being tested.
#[derive(Debug, Clone)]
pub(crate) struct Scripted {
    answers: Arc<Mutex<VecDeque<Answering>>>,
    said: Arc<Mutex<Vec<String>>>,
    cancels: Arc<Mutex<Vec<Cancel>>>,
    held: Option<Arc<Gate>>,
}

impl Scripted {
    pub(crate) fn saying(answers: impl IntoIterator<Item = Answering>) -> Self {
        Self {
            answers: Arc::new(Mutex::new(answers.into_iter().collect())),
            said: Arc::new(Mutex::new(Vec::new())),
            cancels: Arc::new(Mutex::new(Vec::new())),
            held: None,
        }
    }

    /// The same model, answering nothing until the gate is opened: what a turn
    /// that takes minutes looks like from the loop's side, without a clock.
    pub(crate) fn held_at(mut self, gate: &Arc<Gate>) -> Self {
        self.held = Some(Arc::clone(gate));
        self
    }

    pub(crate) fn turns(&self) -> usize {
        self.said.lock().expect("a stand-in nothing poisoned").len()
    }

    /// Everything this model was asked, in the order it was asked: what a test
    /// reads to say that an answer somebody sent reached the session that had
    /// asked for it, in the words it was sent in.
    pub(crate) fn said(&self) -> Vec<String> {
        self.said
            .lock()
            .expect("a stand-in nothing poisoned")
            .clone()
    }

    /// Whether anything that was given a handle on a turn of this model has been
    /// told to stop.
    pub(crate) fn cancelled(&self) -> bool {
        self.cancels
            .lock()
            .expect("a stand-in nothing poisoned")
            .iter()
            .any(Cancel::is_cancelled)
    }
}

impl Wired for Scripted {
    fn wired(&self, cancel: Cancel, _activities: Activities) -> Self {
        self.cancels
            .lock()
            .expect("a stand-in nothing poisoned")
            .push(cancel);
        self.clone()
    }
}

impl Converses for Scripted {
    fn turn(&self, message: &str) -> Result<String, agent::Error> {
        self.said
            .lock()
            .expect("a stand-in nothing poisoned")
            .push(message.to_owned());
        if let Some(gate) = &self.held {
            gate.wait();
        }

        let answer = self
            .answers
            .lock()
            .expect("a stand-in nothing poisoned")
            .pop_front();
        match answer {
            Some(Answering::Says(text)) => Ok(text),
            Some(Answering::Missing) => Err(agent::Error::NotFound {
                program: "claude".into(),
            }),
            // A turn the script has no answer for is a session opened where the
            // test meant none to be, which is worth failing over rather than
            // answering.
            None => panic!("a turn this model was not scripted for: {message}"),
        }
    }

    fn raised(&self, _model: &str, _effort: &str) -> Self {
        self.clone()
    }
}

/// A board that answers every operation out of memory, and the seam it arrives
/// through: a `Boarding` is its own [`Opens::Board`], so a test keeps a handle on
/// the very board a flow opened and reads afterwards what was asked of it.
///
/// One stand-in for every flow rather than one per test file: a push, a cut's
/// fetch and a cut are one conversation with one workspace, and the panel's
/// session opens all three through a single type parameter.
///
/// `Arc<Mutex<_>>` rather than `Rc<RefCell<_>>`: the panel's push and cut run
/// on worker threads, so a board that could not cross one would not stand in
/// for the thing being tested.
///
/// `Debug` is written by hand because the keys it was opened with are kept, and
/// a derived rendering would put them back into the `Debug` output every test on
/// this path asserts carries no key.
#[derive(Clone)]
pub(crate) struct Boarding {
    log: Arc<Mutex<Log>>,
    viewer: String,
    team: Option<String>,
    status: Option<String>,
    state: Option<String>,
    label: String,
    project: Option<FetchedProject>,
    /// What [`Board::scope_queue`] answers with, empty until a test says
    /// otherwise: most flows here never read a queue.
    queue: Queue,
    /// What [`Board::named_issue`] answers with, `None` being a board that has no
    /// such ticket.
    named: Option<NamedIssue>,
    created: LinearProject,
    first_issue: u32,
    refusals: Vec<Refusal>,
    refusing_everything: Option<String>,
    watching: Option<PathBuf>,
    unopened: bool,
    unreachable: bool,
    held: Option<Arc<Gate>>,
}

#[derive(Debug, Default)]
struct Log {
    calls: Vec<Call>,
    recorded: Vec<bool>,
    keys: Vec<String>,
    issued: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Call {
    Viewer,
    Team(String),
    BacklogStatus,
    BacklogState(String),
    WorkflowState { team: String, name: String },
    MoveIssue { issue: String, state: String },
    IssueLabel { name: String, team: String },
    FetchProject(String),
    ScopeQueue(QueueAsked),
    NamedIssue { team: String, number: u64 },
    CreateProject(ProjectAsked),
    CreateIssue(IssueAsked),
    Relation { blocker: String, waiting: String },
    Comment { project: String, body: String },
    IssueComment { issue: String, body: String },
}

impl Call {
    pub(crate) const fn op(&self) -> Op {
        match self {
            Self::Viewer => Op::Viewer,
            Self::Team(_) => Op::Team,
            Self::BacklogStatus => Op::BacklogStatus,
            Self::BacklogState(_) => Op::BacklogState,
            Self::WorkflowState { .. } => Op::WorkflowState,
            Self::MoveIssue { .. } => Op::MoveIssue,
            Self::IssueLabel { .. } => Op::IssueLabel,
            Self::FetchProject(_) => Op::FetchProject,
            Self::ScopeQueue(_) => Op::ScopeQueue,
            Self::NamedIssue { .. } => Op::NamedIssue,
            Self::CreateProject(_) => Op::CreateProject,
            Self::CreateIssue(_) => Op::CreateIssue,
            Self::Relation { .. } => Op::Relation,
            Self::Comment { .. } => Op::Comment,
            Self::IssueComment { .. } => Op::IssueComment,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Op {
    Viewer,
    Team,
    BacklogStatus,
    BacklogState,
    WorkflowState,
    MoveIssue,
    IssueLabel,
    FetchProject,
    ScopeQueue,
    NamedIssue,
    CreateProject,
    CreateIssue,
    Relation,
    Comment,
    IssueComment,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct QueueAsked {
    pub(crate) team: String,
    pub(crate) label: String,
    pub(crate) assignee: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProjectAsked {
    pub(crate) name: String,
    pub(crate) content: String,
    pub(crate) team: String,
    pub(crate) status: Option<String>,
    pub(crate) label: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct IssueAsked {
    pub(crate) title: String,
    pub(crate) body: String,
    pub(crate) team: String,
    pub(crate) project: String,
    pub(crate) label: String,
    pub(crate) state: String,
    pub(crate) assignee: String,
}

// Which calls of one operation are turned down, counted from the first call of
// that operation: `until` of `None` is every call from `from` on.
#[derive(Debug, Clone)]
struct Refusal {
    op: Op,
    from: usize,
    until: Option<usize>,
    message: String,
}

/// The user every [`Boarding`] answers [`Board::viewer`] with, so a test
/// asserting where the id went can name it rather than repeat a literal.
pub(crate) const VIEWER: &str = "user-viewer";

impl Boarding {
    /// A workspace that has the team, both `Backlog`s and the label already,
    /// creates the project at `url`, numbers issues from 1, and holds no project
    /// to read back.
    pub(crate) fn filing(url: impl Into<String>) -> Self {
        Self {
            log: Arc::new(Mutex::new(Log::default())),
            viewer: VIEWER.to_owned(),
            team: Some("team-1".to_owned()),
            status: Some("status-backlog".to_owned()),
            state: Some("state-backlog".to_owned()),
            label: "label-held".to_owned(),
            project: None,
            queue: Queue::new(Vec::new(), false),
            named: None,
            created: LinearProject::new("project-filed", url),
            first_issue: 1,
            refusals: Vec::new(),
            refusing_everything: None,
            watching: None,
            unopened: false,
            unreachable: false,
            held: None,
        }
    }

    /// A workspace holding one project, under that name, in that status, with
    /// that description — which is where the scope block a cut parses lives.
    pub(crate) fn holding(
        name: impl Into<String>,
        status: Option<&str>,
        content: impl Into<String>,
    ) -> Self {
        Self::filing("").reading(FetchedProject::new(
            name,
            content,
            "https://linear.app/acme/project/drafted-1a2b3c",
            status,
        ))
    }

    /// Linear's own words for a request it understood and would not do, on
    /// whichever operation comes first: the failure the panel has to survive.
    pub(crate) fn refusing(message: impl Into<String>) -> Self {
        Self {
            refusing_everything: Some(message.into()),
            ..Self::filing("")
        }
    }

    /// A board no call may reach: being asked anything at all is the failure a
    /// test holding one is about, so it panics rather than recording a flag.
    pub(crate) fn unreachable() -> Self {
        Self {
            unreachable: true,
            ..Self::filing("")
        }
    }

    /// The same, one step earlier: the key being read and a board being opened
    /// at all is the failure.
    pub(crate) fn unopened() -> Self {
        Self {
            unopened: true,
            ..Self::unreachable()
        }
    }

    /// The queue this scope's own filters came back with, which is what a pull's
    /// selection is made over.
    pub(crate) fn queueing(mut self, queue: Queue) -> Self {
        self.queue = queue;
        self
    }

    /// The one ticket this board answers `named_issue` with, whatever name is
    /// asked: which name that was is in the recorded call, so a test asserting
    /// that `--ticket` reached the board reads it there rather than from the
    /// answer.
    pub(crate) fn naming(mut self, named: NamedIssue) -> Self {
        self.named = Some(named);
        self
    }

    pub(crate) fn reading(mut self, project: FetchedProject) -> Self {
        self.project = Some(project);
        self
    }

    pub(crate) fn creating_project(mut self, id: &str, url: &str) -> Self {
        self.created = LinearProject::new(id, url);
        self
    }

    pub(crate) fn without_team(mut self) -> Self {
        self.team = None;
        self
    }

    pub(crate) fn without_backlog_state(mut self) -> Self {
        self.state = None;
        self
    }

    pub(crate) fn labelled(mut self, id: &str) -> Self {
        id.clone_into(&mut self.label);
        self
    }

    /// Issues are numbered as they are created — `issue-N`, `WAR-N` — so the
    /// identifiers a test reads back are this workspace's own answers in the
    /// order it gave them.
    pub(crate) const fn numbering_from(mut self, first: u32) -> Self {
        self.first_issue = first;
        self
    }

    pub(crate) fn refuse(self, op: Op, message: &str) -> Self {
        self.refusal(op, 0, None, message)
    }

    /// Every call of `op` from the `from`th on (counting from nothing) turned
    /// down: what a run that dies partway is built out of.
    pub(crate) fn refuse_from(self, op: Op, from: usize, message: &str) -> Self {
        self.refusal(op, from, None, message)
    }

    pub(crate) fn refuse_at(self, op: Op, at: usize, message: &str) -> Self {
        self.refusal(op, at, Some(at + 1), message)
    }

    fn refusal(mut self, op: Op, from: usize, until: Option<usize>, message: &str) -> Self {
        self.refusals.push(Refusal {
            op,
            from,
            until,
            message: message.to_owned(),
        });
        self
    }

    /// Whether `path` exists is noted at every call, which is how a test says
    /// that a record was written after the requests rather than before one.
    pub(crate) fn watching(mut self, path: PathBuf) -> Self {
        self.watching = Some(path);
        self
    }

    /// The same workspace, answering nothing until the gate is opened: what a
    /// slow request looks like from the loop's side, without a clock.
    pub(crate) fn held_at(mut self, gate: &Arc<Gate>) -> Self {
        self.held = Some(Arc::clone(gate));
        self
    }

    pub(crate) fn calls(&self) -> Vec<Call> {
        self.log().calls.clone()
    }

    pub(crate) fn ops(&self) -> Vec<Op> {
        self.log().calls.iter().map(Call::op).collect()
    }

    /// How many calls reached the workspace, which is how a test says that a
    /// refusal or a second press sent nothing.
    pub(crate) fn requests(&self) -> usize {
        self.log().calls.len()
    }

    /// Where in the whole conversation each call of `op` was asked, so an
    /// ordering promise is an assertion about positions.
    pub(crate) fn positions_of(&self, op: Op) -> Vec<usize> {
        self.log()
            .calls
            .iter()
            .enumerate()
            .filter(|(_, call)| call.op() == op)
            .map(|(at, _)| at)
            .collect()
    }

    pub(crate) fn projects_created(&self) -> Vec<ProjectAsked> {
        self.log()
            .calls
            .iter()
            .filter_map(|call| match call {
                Call::CreateProject(asked) => Some(asked.clone()),
                _ => None,
            })
            .collect()
    }

    pub(crate) fn issues_created(&self) -> Vec<IssueAsked> {
        self.log()
            .calls
            .iter()
            .filter_map(|call| match call {
                Call::CreateIssue(asked) => Some(asked.clone()),
                _ => None,
            })
            .collect()
    }

    /// Every edge asked for, blocker first.
    pub(crate) fn relations(&self) -> Vec<(String, String)> {
        self.log()
            .calls
            .iter()
            .filter_map(|call| match call {
                Call::Relation { blocker, waiting } => Some((blocker.clone(), waiting.clone())),
                _ => None,
            })
            .collect()
    }

    /// Every project comment asked for, as the project id and the body.
    pub(crate) fn comments(&self) -> Vec<(String, String)> {
        self.log()
            .calls
            .iter()
            .filter_map(|call| match call {
                Call::Comment { project, body } => Some((project.clone(), body.clone())),
                _ => None,
            })
            .collect()
    }

    /// Whether the watched path existed at each call, in the order of the calls.
    pub(crate) fn recorded_when_asked(&self) -> Vec<bool> {
        self.log().recorded.clone()
    }

    /// Every key this board was opened with, which is how a test says the value
    /// out of the key store reached the one line that reads it.
    pub(crate) fn opened_with(&self) -> Vec<String> {
        self.log().keys.clone()
    }

    fn log(&self) -> MutexGuard<'_, Log> {
        self.log.lock().expect("no test panics holding this")
    }

    fn ask(&self, call: Call) -> Result<(), LinearError> {
        assert!(!self.unreachable, "a request was sent: {call:?}");
        let op = call.op();
        let index = {
            let mut log = self.log();
            let index = log.calls.iter().filter(|asked| asked.op() == op).count();
            let recorded = self.watching.as_deref().is_some_and(Path::exists);
            log.recorded.push(recorded);
            log.calls.push(call);
            index
        };
        if let Some(gate) = &self.held {
            gate.wait();
        }

        let refused = self.refusing_everything.as_ref().or_else(|| {
            self.refusals
                .iter()
                .find(|refusal| {
                    refusal.op == op
                        && index >= refusal.from
                        && refusal.until.is_none_or(|until| index < until)
                })
                .map(|refusal| &refusal.message)
        });
        match refused {
            Some(message) => Err(LinearError::Refused {
                message: message.clone(),
            }),
            None => Ok(()),
        }
    }
}

impl fmt::Debug for Boarding {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Boarding")
            .field("requests", &self.requests())
            .finish_non_exhaustive()
    }
}

impl Opens for Boarding {
    type Board = Self;

    fn open(&self, key: &str) -> Self {
        assert!(!self.unopened, "the key was read and a board was opened");
        self.log().keys.push(key.to_owned());
        self.clone()
    }
}

impl Board for Boarding {
    fn viewer(&self) -> Result<String, LinearError> {
        self.ask(Call::Viewer)?;
        Ok(self.viewer.clone())
    }

    fn team_id(&self, key: &str) -> Result<Option<String>, LinearError> {
        self.ask(Call::Team(key.to_owned()))?;
        Ok(self.team.clone())
    }

    fn backlog_status(&self) -> Result<Option<String>, LinearError> {
        self.ask(Call::BacklogStatus)?;
        Ok(self.status.clone())
    }

    fn backlog_state(&self, team: &str) -> Result<Option<String>, LinearError> {
        self.ask(Call::BacklogState(team.to_owned()))?;
        Ok(self.state.clone())
    }

    /// The one workflow state this workspace holds an id for, whatever name is
    /// asked: which name that was is in the recorded call, so a test asserting
    /// that a flow asked for `In Progress` reads it there rather than from the
    /// answer. A board built [`without_backlog_state`] has no states at all, so
    /// a move's resolve comes back empty too.
    ///
    /// [`without_backlog_state`]: Boarding::without_backlog_state
    fn workflow_state(&self, team: &str, name: &str) -> Result<Option<String>, LinearError> {
        self.ask(Call::WorkflowState {
            team: team.to_owned(),
            name: name.to_owned(),
        })?;
        Ok(self.state.clone())
    }

    /// The move recorded and the issue handed back, as the real board answers
    /// with the issue it updated.
    fn move_issue(&self, issue: &str, state: &str) -> Result<String, LinearError> {
        self.ask(Call::MoveIssue {
            issue: issue.to_owned(),
            state: state.to_owned(),
        })?;
        Ok(issue.to_owned())
    }

    fn issue_label_id(&self, name: &str, team: &str) -> Result<String, LinearError> {
        self.ask(Call::IssueLabel {
            name: name.to_owned(),
            team: team.to_owned(),
        })?;
        Ok(self.label.clone())
    }

    fn fetch_project(&self, id: &str) -> Result<Option<FetchedProject>, LinearError> {
        self.ask(Call::FetchProject(id.to_owned()))?;
        Ok(self.project.clone())
    }

    /// Whatever queue the test wrote down, and the call recorded: the flows that
    /// file and cut read none, so the default is an empty one.
    fn scope_queue(&self, team: &str, label: &str, assignee: &str) -> Result<Queue, LinearError> {
        self.ask(Call::ScopeQueue(QueueAsked {
            team: team.to_owned(),
            label: label.to_owned(),
            assignee: assignee.to_owned(),
        }))?;
        Ok(self.queue.clone())
    }

    /// Whatever ticket the test wrote down — `None`, a board with no such ticket,
    /// being the default — and the call recorded, which is where a test reads the
    /// team and number that were asked for.
    fn named_issue(&self, team: &str, number: u64) -> Result<Option<NamedIssue>, LinearError> {
        self.ask(Call::NamedIssue {
            team: team.to_owned(),
            number,
        })?;
        Ok(self.named.clone())
    }

    fn create_project(&self, project: &NewProject<'_>) -> Result<LinearProject, LinearError> {
        self.ask(Call::CreateProject(ProjectAsked {
            name: project.name().to_owned(),
            content: project.content().to_owned(),
            team: project.team().to_owned(),
            status: project.status().map(ToOwned::to_owned),
            label: project.label().to_owned(),
        }))?;
        Ok(self.created.clone())
    }

    fn create_issue(&self, issue: &NewIssue<'_>) -> Result<LinearIssue, LinearError> {
        self.ask(Call::CreateIssue(IssueAsked {
            title: issue.title().to_owned(),
            body: issue.body().to_owned(),
            team: issue.team().to_owned(),
            project: issue.project().to_owned(),
            label: issue.label().to_owned(),
            state: issue.state().to_owned(),
            assignee: issue.assignee().to_owned(),
        }))?;
        let number = {
            let mut log = self.log();
            let number = self.first_issue + log.issued;
            log.issued += 1;
            number
        };
        Ok(LinearIssue::new(
            format!("issue-{number}"),
            format!("WAR-{number}"),
            format!("https://linear.app/acme/issue/WAR-{number}"),
        ))
    }

    fn create_relation(&self, blocker: &str, waiting: &str) -> Result<String, LinearError> {
        self.ask(Call::Relation {
            blocker: blocker.to_owned(),
            waiting: waiting.to_owned(),
        })?;
        Ok(format!(
            "relation-{}",
            self.positions_of(Op::Relation).len()
        ))
    }

    fn comment_on_project(&self, project: &str, body: &str) -> Result<String, LinearError> {
        self.ask(Call::Comment {
            project: project.to_owned(),
            body: body.to_owned(),
        })?;
        Ok("comment-1".to_owned())
    }

    /// Recorded as its own call, not as a [`Call::Comment`] with an issue id in
    /// it: a flow that meant to explain a halt on the ticket and commented on the
    /// project instead is a bug a shared variant would hide.
    fn comment_on_issue(&self, issue: &str, body: &str) -> Result<String, LinearError> {
        self.ask(Call::IssueComment {
            issue: issue.to_owned(),
            body: body.to_owned(),
        })?;
        Ok("comment-1".to_owned())
    }
}

/// A request held open, for the one thing about a worker a finished push cannot
/// show: that the loop goes round — drawing, answering keys — while it is in
/// flight. A test holds the gate shut, counts rounds, and opens it.
#[derive(Debug, Default)]
pub(crate) struct Gate {
    open: Mutex<bool>,
    changed: Condvar,
}

impl Gate {
    pub(crate) fn shut() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Notified rather than dropped, so the worker goes on to answer and the
    /// test can then assert what the push said: a gate nobody opens is a thread
    /// parked until the test binary exits.
    pub(crate) fn open(&self) {
        *self.open.lock().expect("no test panics holding this") = true;
        self.changed.notify_all();
    }

    // Looped for the spurious wakeup `Condvar` is allowed, which is the whole of
    // why this is not a bare `park`.
    fn wait(&self) {
        let mut open = self.open.lock().expect("no test panics holding this");
        while !*open {
            open = self
                .changed
                .wait(open)
                .expect("no test panics holding this");
        }
    }
}

/// A checkout that answers out of memory and writes down every call, so a whole
/// pull is driven with no repository and no `git` on the machine.
///
/// What it answers is scripted per call and the last answer repeats, which is what
/// lets one value stand for a run of several sessions: `HEAD` holding still across
/// a session is the ordinary case, and a second answer is how a test says a session
/// committed behind warlock's back.
///
/// `Arc<Mutex<_>>` for the reason [`Boarding`]'s log is one: a pull runs on a
/// worker thread in the panel, so a checkout that could not cross one would not
/// stand in for the thing being tested.
#[derive(Debug, Clone)]
pub(crate) struct Checkout {
    log: Arc<Mutex<Vec<GitCall>>>,
    default: String,
    trees: Vec<Vec<Dirty>>,
    heads: Vec<String>,
}

/// Every call a pull makes on a checkout, in the pull's words rather than in
/// `git`'s: the argv is [`Git`](warlock_tui::Git)'s business and is asserted on the
/// real adapter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum GitCall {
    Dirty,
    DefaultBranch,
    SwitchTo(String),
    CatchUp(String),
    CutBranch { branch: String, from: String },
    Head,
    CommitAll(String),
    Publish(String),
}

impl Checkout {
    /// On the default branch, clean, and holding one commit — the checkout a fresh
    /// pull starts in.
    pub(crate) fn clean(default: &str) -> Self {
        Self {
            log: Arc::new(Mutex::new(Vec::new())),
            default: default.to_owned(),
            trees: vec![Vec::new()],
            heads: vec!["4e72482258".to_owned()],
        }
    }

    /// What `git status` answers, one entry per call, the last repeating: the tree
    /// a session is to be found to have left.
    pub(crate) fn trees(mut self, trees: impl IntoIterator<Item = Vec<Dirty>>) -> Self {
        self.trees = trees.into_iter().collect();
        assert!(!self.trees.is_empty(), "a checkout answers every status");
        self
    }

    /// The commits `HEAD` is on, one per call, the last repeating. Two different
    /// ones across a session is a session that committed.
    pub(crate) fn heads(mut self, heads: impl IntoIterator<Item = &'static str>) -> Self {
        self.heads = heads.into_iter().map(ToOwned::to_owned).collect();
        assert!(!self.heads.is_empty(), "a checkout is always on a commit");
        self
    }

    pub(crate) fn calls(&self) -> Vec<GitCall> {
        self.log
            .lock()
            .expect("no test panics holding this")
            .clone()
    }

    /// Every commit message this checkout was asked to make, in order: what a halt
    /// is asserted to have left empty.
    pub(crate) fn commits(&self) -> Vec<String> {
        self.calls()
            .into_iter()
            .filter_map(|call| match call {
                GitCall::CommitAll(message) => Some(message),
                _ => None,
            })
            .collect()
    }

    // The nth call of one kind, saturating at the last written-down answer.
    fn answer<T: Clone>(&self, answers: &[T], of: &GitCall) -> T {
        let at = self
            .calls()
            .iter()
            .filter(|call| std::mem::discriminant(*call) == std::mem::discriminant(of))
            .count();
        answers[at.min(answers.len() - 1)].clone()
    }

    fn note(&self, call: GitCall) {
        self.log
            .lock()
            .expect("no test panics holding this")
            .push(call);
    }
}

impl Repository for Checkout {
    fn dirty(&self) -> Result<Vec<Dirty>, GitError> {
        let answer = self.answer(&self.trees, &GitCall::Dirty);
        self.note(GitCall::Dirty);
        Ok(answer)
    }

    fn default_branch(&self) -> Result<String, GitError> {
        self.note(GitCall::DefaultBranch);
        Ok(self.default.clone())
    }

    fn switch_to(&self, branch: &str) -> Result<(), GitError> {
        self.note(GitCall::SwitchTo(branch.to_owned()));
        Ok(())
    }

    fn catch_up(&self, branch: &str) -> Result<(), GitError> {
        self.note(GitCall::CatchUp(branch.to_owned()));
        Ok(())
    }

    fn cut_branch(&self, branch: &str, from: &str) -> Result<(), GitError> {
        self.note(GitCall::CutBranch {
            branch: branch.to_owned(),
            from: from.to_owned(),
        });
        Ok(())
    }

    fn head(&self) -> Result<Commit, GitError> {
        let answer = self.answer(&self.heads, &GitCall::Head);
        self.note(GitCall::Head);
        Ok(Commit::new(answer))
    }

    fn commit_all(&self, message: &str) -> Result<(), GitError> {
        self.note(GitCall::CommitAll(message.to_owned()));
        Ok(())
    }

    fn publish(&self, branch: &str) -> Result<(), GitError> {
        self.note(GitCall::Publish(branch.to_owned()));
        Ok(())
    }
}

/// A forge that opens whatever it is asked for and keeps the request.
///
/// The request is kept whole rather than as a flag, because the two branches on it
/// are the pair a call swapping them would still compile: what a test asserts is
/// that the base is the detected default branch and the head is the run's own.
#[derive(Debug, Clone)]
pub(crate) struct Forging {
    log: Arc<Mutex<Vec<PullRequestAsked>>>,
    opened: Opened,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PullRequestAsked {
    pub(crate) base: String,
    pub(crate) head: String,
    pub(crate) title: String,
    pub(crate) body: String,
}

impl Forging {
    pub(crate) fn opening(url: &str) -> Self {
        Self {
            log: Arc::new(Mutex::new(Vec::new())),
            opened: Opened::At {
                url: url.to_owned(),
            },
        }
    }

    /// A machine with no `gh` on it, which answers the request rather than
    /// refusing it: the request is still recorded, because what a caller has to
    /// do about [`Opened::NoGh`] is decided from the body it built.
    pub(crate) fn without_gh() -> Self {
        Self {
            log: Arc::new(Mutex::new(Vec::new())),
            opened: Opened::NoGh,
        }
    }

    pub(crate) fn asked(&self) -> Vec<PullRequestAsked> {
        self.log
            .lock()
            .expect("no test panics holding this")
            .clone()
    }
}

impl Forge for Forging {
    fn open_pull_request(&self, request: PullRequest<'_>) -> Result<Opened, GitError> {
        self.log
            .lock()
            .expect("no test panics holding this")
            .push(PullRequestAsked {
                base: request.base.to_owned(),
                head: request.head.to_owned(),
                title: request.title.to_owned(),
                body: request.body.to_owned(),
            });
        Ok(self.opened.clone())
    }
}

/// The split of a ticket, written down before the run starts, and what it was
/// asked about.
///
/// One answer and not a sequence: a run splits its ticket once, and a second
/// answer would be a promise about a road nothing takes.
#[derive(Debug, Clone)]
pub(crate) struct Slicing {
    log: Arc<Mutex<Vec<SplitAsked>>>,
    answer: Split,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SplitAsked {
    pub(crate) ticket: String,
    pub(crate) title: String,
    pub(crate) description: String,
}

impl Slicing {
    /// A split that answered with these goals, each waiting on the one before it
    /// and numbered `<TICKET>.01` upward, as [`number`] leaves them.
    ///
    /// [`number`]: warlock_engine::splitting::number
    pub(crate) fn into_chain(ticket: &str, goals: &[&str]) -> Self {
        let subtasks = goals
            .iter()
            .enumerate()
            .map(|(at, goal)| Numbered {
                id: format!("{ticket}.{:02}", at + 1),
                goal: (*goal).to_owned(),
                depends_on: if at == 0 {
                    Vec::new()
                } else {
                    vec![format!("{ticket}.{at:02}")]
                },
                definition_of_done: vec![format!("{goal} is done")],
                likely_files: Vec::new(),
                test_plan: String::new(),
                notes: String::new(),
            })
            .collect();
        Self::answering(Split::Subtasks {
            subtasks,
            repairs: Vec::new(),
        })
    }

    pub(crate) fn answering(answer: Split) -> Self {
        Self {
            log: Arc::new(Mutex::new(Vec::new())),
            answer,
        }
    }

    pub(crate) fn asked(&self) -> Vec<SplitAsked> {
        self.log
            .lock()
            .expect("no test panics holding this")
            .clone()
    }
}

impl Splits for Slicing {
    fn split(&self, ticket: &str, title: &str, description: &str) -> Split {
        self.log
            .lock()
            .expect("no test panics holding this")
            .push(SplitAsked {
                ticket: ticket.to_owned(),
                title: title.to_owned(),
                description: description.to_owned(),
            });
        self.answer.clone()
    }
}

/// The sub-task sessions, written down before the run starts: one answer per
/// session in the order the run raises them, and every opening kept.
///
/// A session the test wrote no answer for panics rather than answering something,
/// because the number of sessions a run raises is exactly what most of these tests
/// are about.
#[derive(Debug, Clone)]
pub(crate) struct Sessions {
    log: Arc<Mutex<Vec<String>>>,
    answers: Arc<Mutex<VecDeque<Worked>>>,
    watching: Option<PathBuf>,
    seen: Arc<Mutex<Vec<String>>>,
    /// Where this session reports what it is seen doing, and what it reports —
    /// the same port the real one is wired with, so a door that bridges
    /// activities onto its own progress is driven the way it runs.
    activities: Activities,
    doing: Vec<Activity>,
}

impl Sessions {
    pub(crate) fn answering(answers: impl IntoIterator<Item = Worked>) -> Self {
        Self {
            log: Arc::new(Mutex::new(Vec::new())),
            answers: Arc::new(Mutex::new(answers.into_iter().collect())),
            watching: None,
            seen: Arc::new(Mutex::new(Vec::new())),
            activities: Activities::none(),
            doing: Vec::new(),
        }
    }

    /// The same sessions, reporting `doing` into `activities` before each answers:
    /// what a caller watching a session sees, without a `claude` on the machine.
    pub(crate) fn reporting(
        mut self,
        activities: Activities,
        doing: impl IntoIterator<Item = Activity>,
    ) -> Self {
        self.activities = activities;
        self.doing = doing.into_iter().collect();
        self
    }

    /// The file as it stood when each session was raised, which is the only way to
    /// ask what a run had written down *before* it spent a session: everything
    /// after the last save looks the same from outside.
    pub(crate) fn watching(mut self, path: PathBuf) -> Self {
        self.watching = Some(path);
        self
    }

    /// Every opening a session was raised on, in order: how a test says the brief,
    /// the ticket and the finished siblings reached the session.
    pub(crate) fn openings(&self) -> Vec<String> {
        self.log
            .lock()
            .expect("no test panics holding this")
            .clone()
    }

    /// What the watched file held at each session, in order. A file that was not
    /// there reads as empty, which is a failure every test asserting on this has a
    /// sentence for.
    pub(crate) fn seen(&self) -> Vec<String> {
        self.seen
            .lock()
            .expect("no test panics holding this")
            .clone()
    }
}

impl Works for Sessions {
    fn work(&self, opening: &str) -> Worked {
        self.log
            .lock()
            .expect("no test panics holding this")
            .push(opening.to_owned());
        for activity in &self.doing {
            self.activities.report(activity.clone());
        }
        if let Some(path) = &self.watching {
            let held = std::fs::read_to_string(path).unwrap_or_default();
            self.seen
                .lock()
                .expect("no test panics holding this")
                .push(held);
        }
        self.answers
            .lock()
            .expect("no test panics holding this")
            .pop_front()
            .expect("a session was raised that this test wrote no answer for")
    }
}

/// One session's last message, read the way a real one is: through the engine's
/// own contract, so a test writes what a session said and never builds an
/// [`Accepted`](warlock_engine::working::Accepted) by hand.
pub(crate) fn said(status: &str, summary: &str, reason: Option<&str>) -> Worked {
    let object = serde_json::json!({
        "status": status,
        "summary": summary,
        "blocked_reason": reason,
    });
    Worked::Answered(working::accept(&object.to_string()))
}
