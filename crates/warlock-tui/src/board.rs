use std::fmt;

/// A key value held for a run that opens a board later, on a worker.
///
/// `Debug` is hand-written rather than derived: the values holding one are kept
/// across the panel's rounds, and a failing assertion anywhere in the suite may
/// print whatever is in scope.
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct Key(String);

impl Key {
    pub(crate) fn new(value: &str) -> Self {
        Self(value.to_owned())
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Key {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<redacted>")
    }
}

/// What `Board::issue_project` answers: the project, and each of its issues'
/// workflow state by name and type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct IssueProject {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) status: Option<String>,
    pub(crate) states: Vec<(String, StateType)>,
}

impl IssueProject {
    #[must_use]
    #[cfg(test)]
    pub(crate) fn new(status: &str, issues: &[(&str, &str)]) -> Self {
        Self {
            id: "project-1".to_owned(),
            name: "A brief".to_owned(),
            status: Some(status.to_owned()),
            states: issues
                .iter()
                .map(|(name, kind)| ((*name).to_owned(), StateType::new(*kind)))
                .collect(),
        }
    }

    #[must_use]
    pub(crate) fn id(&self) -> &str {
        &self.id
    }

    #[must_use]
    pub(crate) fn name(&self) -> &str {
        &self.name
    }

    #[must_use]
    pub(crate) fn status(&self) -> Option<&str> {
        self.status.as_deref()
    }

    #[must_use]
    pub(crate) fn states(&self) -> &[(String, StateType)] {
        &self.states
    }
}

/// How warlock's notes on a project start, which is what a project read filters
/// comments on and `cut.rs` writes and reads. Changing either orphans every note
/// already on a board: the slices they record are offered again.
pub(crate) const CUT_NOTE: &str = "Warlock cut slice ";
pub(crate) const SKIP_NOTE: &str = "Warlock skipped slice ";

/// A project as `Board::fetch_project` reads it back, which is not the
/// [`Project`] a create answers with: what matters about a project that already
/// exists is what is written on it, and what matters about one that has just
/// been made is where to find it.
///
/// Two of these are allowed to be empty and neither is a broken answer. A
/// project filed into a workspace with no `Backlog` has no status at all, so a
/// gate on the status has to be able to say that as well as name a wrong one;
/// and a description can be emptied in Linear after it was filed, which the
/// caller that parses it will refuse in its own words.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FetchedProject {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) content: String,
    pub(crate) url: String,
    pub(crate) status: Option<String>,
    pub(crate) notes: Vec<String>,
}

impl FetchedProject {
    #[must_use]
    #[cfg(test)]
    pub(crate) fn new(
        name: impl Into<String>,
        content: impl Into<String>,
        url: impl Into<String>,
        status: Option<&str>,
    ) -> Self {
        Self {
            id: "project-1".to_owned(),
            name: name.into(),
            content: content.into(),
            url: url.into(),
            status: status.map(ToOwned::to_owned),
            notes: Vec::new(),
        }
    }

    #[must_use]
    #[cfg(test)]
    pub(crate) fn with_notes(mut self, notes: impl IntoIterator<Item = String>) -> Self {
        self.notes = notes.into_iter().collect();
        self
    }

    /// Linear's own id, which an issue create and a comment are written
    /// against: the slug a draft was typed with reads a project back, and
    /// nothing has said a mutation takes one.
    #[must_use]
    pub(crate) fn id(&self) -> &str {
        &self.id
    }

    #[must_use]
    pub(crate) fn name(&self) -> &str {
        &self.name
    }

    #[must_use]
    pub(crate) fn content(&self) -> &str {
        &self.content
    }

    #[must_use]
    #[cfg(test)]
    pub(crate) fn url(&self) -> &str {
        &self.url
    }

    #[must_use]
    pub(crate) fn status(&self) -> Option<&str> {
        self.status.as_deref()
    }

    /// The bodies of warlock's cut and skip notes, oldest first.
    #[must_use]
    pub(crate) fn notes(&self) -> &[String] {
        &self.notes
    }
}

/// What `Board::planned_projects` answers: slug and name, in Linear's order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Listing {
    pub(crate) projects: Vec<(String, String)>,
}

impl Listing {
    #[must_use]
    #[cfg(test)]
    pub(crate) fn new(projects: &[(&str, &str)]) -> Self {
        Self {
            projects: projects
                .iter()
                .map(|(slug, name)| ((*slug).to_owned(), (*name).to_owned()))
                .collect(),
        }
    }

    #[must_use]
    pub(crate) fn projects(&self) -> &[(String, String)] {
        &self.projects
    }
}

/// A scope's queue, all of it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Queue {
    pub(crate) issues: Vec<QueuedIssue>,
}

impl Queue {
    #[must_use]
    pub fn new(issues: Vec<QueuedIssue>) -> Self {
        Self { issues }
    }

    /// In Linear's own order, which is not the order the queue is worked:
    /// choosing is somebody else's function, and it wants the queue as the board
    /// gave it.
    #[must_use]
    pub fn issues(&self) -> &[QueuedIssue] {
        &self.issues
    }
}

/// One issue in a scope's queue, carrying everything choosing needs and nothing
/// else.
///
/// Both names Linear has for an issue, because both are needed: the id writes a
/// state move and the identifier is what a person types and a run record holds.
/// The state arrives twice for the same reason — the name is what the record's
/// `review_state` and `In Progress` are matched against and what a skip prints,
/// the type is what says whether a blocker is out of the way.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueuedIssue {
    pub(crate) id: String,
    pub(crate) identifier: String,
    pub(crate) title: String,
    pub(crate) description: String,
    pub(crate) state: String,
    pub(crate) state_type: StateType,
    pub(crate) priority: Priority,
    pub(crate) blockers: Vec<Blocker>,
}

impl QueuedIssue {
    #[must_use]
    pub fn new(
        id: impl Into<String>,
        identifier: impl Into<String>,
        title: impl Into<String>,
        state: impl Into<String>,
        state_type: StateType,
        priority: Priority,
        blockers: Vec<Blocker>,
    ) -> Self {
        Self {
            id: id.into(),
            identifier: identifier.into(),
            title: title.into(),
            description: String::new(),
            state: state.into(),
            state_type,
            priority,
            blockers,
        }
    }

    #[must_use]
    pub fn with_description(self, description: impl Into<String>) -> Self {
        Self {
            description: description.into(),
            ..self
        }
    }

    /// What the ticket says, which `warlock pull` hands every sub-task as the
    /// parent ticket.
    #[must_use]
    pub fn description(&self) -> &str {
        &self.description
    }

    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }

    #[must_use]
    pub fn identifier(&self) -> &str {
        &self.identifier
    }

    #[must_use]
    pub fn title(&self) -> &str {
        &self.title
    }

    /// The team's own name for the state — `In Review`, `Doing` — which is what
    /// a record names and a skipped issue prints.
    #[must_use]
    pub fn state(&self) -> &str {
        &self.state
    }

    #[must_use]
    pub const fn state_type(&self) -> &StateType {
        &self.state_type
    }

    #[must_use]
    pub const fn priority(&self) -> Priority {
        self.priority
    }

    /// The issues blocking this one, whoever owns them.
    #[must_use]
    pub fn blockers(&self) -> &[Blocker] {
        &self.blockers
    }
}

/// An issue holding up one in the queue, in the three facts a refusal needs: the
/// identifier to name it, the assignee to say whose it is, and the state type to
/// say whether it is still in the way.
///
/// No id and no title: nothing is done to a blocker, it is only reported.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Blocker {
    pub(crate) identifier: String,
    pub(crate) assignee: Option<String>,
    pub(crate) state_type: StateType,
}

impl Blocker {
    #[must_use]
    pub fn new(
        identifier: impl Into<String>,
        assignee: Option<&str>,
        state_type: StateType,
    ) -> Self {
        Self {
            identifier: identifier.into(),
            assignee: assignee.map(ToOwned::to_owned),
            state_type,
        }
    }

    #[must_use]
    pub fn identifier(&self) -> &str {
        &self.identifier
    }

    /// The person's name and not their id: a blocker is never matched against
    /// anybody, only printed. `None` is an unassigned blocker, which a queue
    /// filtered by assignee can still be held up by.
    #[must_use]
    pub fn assignee(&self) -> Option<&str> {
        self.assignee.as_deref()
    }

    #[must_use]
    pub const fn state_type(&self) -> &StateType {
        &self.state_type
    }
}

/// A workflow state's type, which is the part of a state that is Linear's rather
/// than the team's: `triage`, `backlog`, `unstarted`, `started`, `completed`,
/// `canceled`.
///
/// Kept as the string it arrived as instead of an enum over those six, because
/// the only question anything asks of it is [`StateType::settled`] and a seventh
/// type invented in Linear next year should not make a queue unreadable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StateType(pub(crate) String);

impl StateType {
    #[must_use]
    pub fn new(name: impl Into<String>) -> Self {
        Self(name.into())
    }

    /// Out of the way: the one judgement this type exists to make. An issue is
    /// ready when every issue blocking it is settled, and these two types are
    /// what Linear calls settled — a state named `Done`, `Shipped` or `Won't do`
    /// is one of them whatever the team called it.
    #[must_use]
    pub fn settled(&self) -> bool {
        self.0 == "completed" || self.0 == "canceled"
    }
}

/// Linear's `priority`, in the order work is taken rather than the order the
/// numbers come in.
///
/// The derived ordering is the whole point: ascending is urgent first and no
/// priority last, so sorting a queue by this sorts it the way the board reads.
/// Linear's own numbers — `0` for none, `1` for urgent — sort the other way
/// round, which is why nothing outside the parsing that reads them holds one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Priority {
    Urgent,
    High,
    Medium,
    Low,
    None,
}

/// A named ticket, and the three facts that say whether it is this scope's work
/// at all.
///
/// An issue in a [`Queue`] answers those three by construction: the query
/// filtered on them, so it is on the team, carries the label and belongs to the
/// key's user. A named ticket is read with none of them applied, so they arrive
/// as facts here — which is what lets a refusal name which one failed instead of
/// saying only that the ticket is not in the queue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NamedIssue {
    pub(crate) issue: QueuedIssue,
    pub(crate) team: String,
    pub(crate) labels: Vec<String>,
    pub(crate) assignee: Option<Assignee>,
}

impl NamedIssue {
    #[must_use]
    #[cfg(test)]
    pub(crate) fn new(
        issue: QueuedIssue,
        team: impl Into<String>,
        labels: Vec<String>,
        assignee: Option<Assignee>,
    ) -> Self {
        Self {
            issue,
            team: team.into(),
            labels,
            assignee,
        }
    }

    #[must_use]
    pub(crate) const fn issue(&self) -> &QueuedIssue {
        &self.issue
    }

    /// The ticket itself, once the checks on the rest are through: what is worked
    /// is an issue and not a membership.
    #[must_use]
    pub(crate) fn into_issue(self) -> QueuedIssue {
        self.issue
    }

    /// The team's key — `WAR` — which is how a scope record names a team, so the
    /// two are comparable without resolving either to an id.
    #[must_use]
    pub(crate) fn team(&self) -> &str {
        &self.team
    }

    /// Every label on the ticket, because a refusal says what it carries as well
    /// as what it is missing.
    #[must_use]
    pub(crate) fn labels(&self) -> &[String] {
        &self.labels
    }

    #[must_use]
    pub(crate) const fn assignee(&self) -> Option<&Assignee> {
        self.assignee.as_ref()
    }
}

/// Who holds a ticket: the id it is matched by and the name it is named by.
///
/// Both, because one of each is needed and neither does the other's work. Ids are
/// what `Board::viewer` answers and the only honest way to ask whether a ticket
/// is yours — two people in a workspace can share a display name. The name is the
/// half a person reads in a refusal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Assignee {
    pub(crate) id: String,
    pub(crate) name: String,
}

impl Assignee {
    #[must_use]
    #[cfg(test)]
    pub(crate) fn new(id: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
        }
    }

    #[must_use]
    pub(crate) fn id(&self) -> &str {
        &self.id
    }

    #[must_use]
    pub(crate) fn name(&self) -> &str {
        &self.name
    }
}

/// What `Board::create_project` is asked for: the label is the name it goes by
/// in the workspace rather than an id, because the create path is what resolves
/// it.
#[derive(Debug, Clone, Copy)]
pub(crate) struct NewProject<'a> {
    pub(crate) name: &'a str,
    pub(crate) content: &'a str,
    pub(crate) team: &'a str,
    pub(crate) status: Option<&'a str>,
    pub(crate) label: &'a str,
}

impl<'a> NewProject<'a> {
    #[must_use]
    pub(crate) const fn new(
        name: &'a str,
        content: &'a str,
        team: &'a str,
        label: &'a str,
    ) -> Self {
        Self {
            name,
            content,
            team,
            status: None,
            label,
        }
    }

    #[must_use]
    pub(crate) const fn with_status(mut self, status: Option<&'a str>) -> Self {
        self.status = status;
        self
    }

    #[must_use]
    #[cfg(test)]
    pub(crate) const fn name(&self) -> &'a str {
        self.name
    }

    #[must_use]
    #[cfg(test)]
    pub(crate) const fn content(&self) -> &'a str {
        self.content
    }

    #[must_use]
    #[cfg(test)]
    pub(crate) const fn team(&self) -> &'a str {
        self.team
    }

    #[must_use]
    #[cfg(test)]
    pub(crate) const fn status(&self) -> Option<&'a str> {
        self.status
    }

    #[must_use]
    #[cfg(test)]
    pub(crate) const fn label(&self) -> &'a str {
        self.label
    }
}

/// A project that now exists. The URL is the one thing a failure downstream must
/// never lose, so it comes back from the create rather than being built from the
/// id here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Project {
    pub(crate) id: String,
    pub(crate) url: String,
}

impl Project {
    #[must_use]
    #[cfg(test)]
    pub(crate) fn new(id: impl Into<String>, url: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            url: url.into(),
        }
    }

    #[must_use]
    #[cfg(test)]
    pub(crate) fn id(&self) -> &str {
        &self.id
    }

    #[must_use]
    pub(crate) fn url(&self) -> &str {
        &self.url
    }
}

/// What `Board::create_issue` is asked for, and the whole of it: every field
/// here is an id the caller resolved, and priority, estimate, cycle and
/// milestone stay unwritten. A draft says what the work is, and a field warlock
/// would have to invent a value for is a decision taken away from the person who
/// owns the board.
///
/// `assignee` is the exception, and it is not an invented value: the person
/// running `draft` is the person who owns the board, and the assignee is the
/// claim `warlock pull` reads — it only ever works tickets assigned to the
/// operator, so a backlog nothing is assigned in is a queue it can never select
/// from. Handing work to a teammate stays a reassignment a human makes in
/// Linear, which is why there is no way to name anybody else here.
///
/// `state` is a *team workflow state* id from `Board::backlog_state` and
/// `label` an *issue label* id from `Board::issue_label_id`; neither a project
/// status nor a project label is usable here.
#[derive(Debug, Clone, Copy)]
pub(crate) struct NewIssue<'a> {
    pub(crate) title: &'a str,
    pub(crate) body: &'a str,
    pub(crate) team: &'a str,
    pub(crate) project: &'a str,
    pub(crate) label: &'a str,
    pub(crate) state: &'a str,
    pub(crate) assignee: &'a str,
}

impl<'a> NewIssue<'a> {
    #[must_use]
    pub(crate) const fn new(
        title: &'a str,
        body: &'a str,
        team: &'a str,
        project: &'a str,
        label: &'a str,
        state: &'a str,
        assignee: &'a str,
    ) -> Self {
        Self {
            title,
            body,
            team,
            project,
            label,
            state,
            assignee,
        }
    }

    #[must_use]
    #[cfg(test)]
    pub(crate) const fn title(&self) -> &'a str {
        self.title
    }

    #[must_use]
    #[cfg(test)]
    pub(crate) const fn body(&self) -> &'a str {
        self.body
    }

    #[must_use]
    #[cfg(test)]
    pub(crate) const fn team(&self) -> &'a str {
        self.team
    }

    #[must_use]
    #[cfg(test)]
    pub(crate) const fn project(&self) -> &'a str {
        self.project
    }

    #[must_use]
    #[cfg(test)]
    pub(crate) const fn label(&self) -> &'a str {
        self.label
    }

    #[must_use]
    #[cfg(test)]
    pub(crate) const fn state(&self) -> &'a str {
        self.state
    }

    #[must_use]
    #[cfg(test)]
    pub(crate) const fn assignee(&self) -> &'a str {
        self.assignee
    }
}

/// An issue that now exists, in all three of the ways the rest of warlock has to
/// name one: the id relations are written with, the identifier a cut record
/// stores and a person types, and the URL a failure downstream must not lose.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Issue {
    pub(crate) id: String,
    pub(crate) identifier: String,
    pub(crate) url: String,
}

impl Issue {
    #[must_use]
    #[cfg(test)]
    pub(crate) fn new(
        id: impl Into<String>,
        identifier: impl Into<String>,
        url: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            identifier: identifier.into(),
            url: url.into(),
        }
    }

    /// An open ticket the queue listed, with the id a relation is written
    /// with: what a draft that waits on `WAR-142` is blocked by.
    #[must_use]
    pub(crate) fn listed(open: &QueuedIssue) -> Self {
        Self {
            id: open.id().to_owned(),
            identifier: open.identifier().to_owned(),
            url: String::new(),
        }
    }

    /// An issue a cut record names, which is the one an earlier run filed: the
    /// identifier is everything such a record keeps, so it stands as the id as
    /// well and there is no URL.
    ///
    /// For the one thing a resumed run does with a slice it cut last time —
    /// name its issues as the blockers of a slice being filed now. The
    /// alternative is a request per recorded issue to turn identifiers back
    /// into ids, which would make a slice nothing is being sent for the reason
    /// something was sent.
    ///
    /// Whether Linear resolves an identifier where a relation wants an id is
    /// Linear's to say, and this promises nothing about it: an edge the API
    /// turns down is one reported line and leaves every issue filed, so the
    /// worst this can come to is the ordering a person adds on the board.
    #[must_use]
    pub(crate) fn recorded(identifier: &str) -> Self {
        Self {
            id: identifier.to_owned(),
            identifier: identifier.to_owned(),
            url: String::new(),
        }
    }

    #[must_use]
    pub(crate) fn id(&self) -> &str {
        &self.id
    }

    /// `WAR-125`, which is what a cut record holds: the id is a UUID that means
    /// nothing to a reader and changes nothing about which issue was filed.
    #[must_use]
    pub(crate) fn identifier(&self) -> &str {
        &self.identifier
    }

    #[must_use]
    #[cfg(test)]
    pub(crate) fn url(&self) -> &str {
        &self.url
    }
}

/// Nothing here carries the key, and two variants say why they carry what they
/// do instead.
#[derive(Debug)]
pub enum Error {
    /// The key cannot be a header value at all — a newline or a byte outside
    /// ASCII in whatever was stored. It is named and not quoted: this type is
    /// printed.
    Key,
    /// Nothing was sent, because nothing is left to hear the answer: the run the
    /// request belonged to was stopped before it went out. Warlock's own word
    /// rather than Linear's — no socket was opened, so the board knows nothing
    /// about it and there is nothing to undo.
    Stopped,
    /// Type-erased rather than the HTTP client's own error, which is what keeps
    /// this module free of one: a board that speaks over a socket boxes whatever
    /// its transport failed with, and a second board's transport need not be the
    /// first's. Naming a concrete error here points the dependency back at one
    /// board's wire.
    Transport {
        source: Box<dyn std::error::Error + Send + Sync>,
    },
    Status {
        code: u16,
    },
    /// Warlock's own words about the answer's shape, never the answer's bytes:
    /// what comes back from an endpoint that is not Linear is whatever a captive
    /// portal or a proxy decided to send, and printing it is printing that.
    Malformed {
        detail: String,
    },
    /// Linear's words, for a request it understood and refused. Worth keeping as
    /// they are — "Entity not found" and "A project with that name exists" are
    /// the two the caller can act on.
    Refused {
        message: String,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Key => write!(f, "the Linear key cannot be sent as a header value"),
            Self::Stopped => write!(f, "the run was stopped, so nothing was sent to Linear"),
            Self::Transport { source } => write!(f, "could not reach Linear: {source}"),
            Self::Status { code } => write!(f, "Linear answered {code}"),
            Self::Malformed { detail } => write!(f, "Linear's answer was unreadable: {detail}"),
            Self::Refused { message } => write!(f, "Linear refused the request: {message}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Transport { source } => Some(&**source),
            Self::Key
            | Self::Stopped
            | Self::Status { .. }
            | Self::Malformed { .. }
            | Self::Refused { .. } => None,
        }
    }
}
