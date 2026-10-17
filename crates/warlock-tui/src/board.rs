use std::fmt;

use warlock_engine::ScopeRecord;

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

/// The operations warlock asks of a board, in the words of the board rather
/// than of GraphQL.
///
/// The flows are written against this and not against `Posts`, so a flow's
/// test fake answers operations instead of recognising query text: Linear's
/// spelling is `Linear`'s business, checked in `linear.rs`'s own tests.
pub(crate) trait Board {
    /// The vendor's name, as every [`Error`] this board builds prints it. An
    /// associated const and not a method, because a decorator that holds no
    /// client still has to name the board it wraps.
    const NAME: &'static str;

    fn viewer(&self) -> Result<String, Error>;
    fn team_id(&self, key: &str) -> Result<Option<String>, Error>;
    /// The brief moved to the board's status of that name, and `true` when a
    /// status of that name was there to move it to.
    ///
    /// The name and not an id, as [`move_ticket`](Self::move_ticket) takes one:
    /// what a status is named is the only thing two boards agree on, so the
    /// board is what turns a name into whatever its own move needs.
    ///
    /// `false` and not an error, because a workspace nobody has given a status
    /// of that name is one warlock reports on and works past: a cut whose last
    /// slice has just settled, and a pull whose ticket was the last one in
    /// review, both say so in a line and carry on. An answer and not a silent
    /// success, so a caller cannot mistake "no such status" for a move that
    /// happened.
    fn move_brief(&self, project: &str, status: &str) -> Result<bool, Error>;
    fn ticket_brief(&self, issue: &str) -> Result<Option<TicketBrief>, Error>;
    fn backlog_state(&self, team: &str) -> Result<Option<String>, Error>;
    /// The ticket moved to the board's status of that name, and `true` when a
    /// status of that name was there to move it to.
    ///
    /// The name and not an id, because what a status is named is the only thing
    /// two boards agree on: a Linear workflow state has a global id and a Jira
    /// status is reachable only through the transitions of the one issue, so the
    /// board is what turns a name into whatever the move needs.
    ///
    /// `false` and not an error, and the callers lean on it: a board whose
    /// workflow nobody has given an `In Progress` or a review state is a board
    /// warlock reports on and works past, and a run that stopped over it would be
    /// the board wagging the pull. An answer and not a silent success, so a
    /// caller cannot mistake "no such column" for a move that happened.
    fn move_ticket(&self, issue: &str, status: &str) -> Result<bool, Error>;
    fn ticket_label_id(&self, name: &str, team: &str) -> Result<String, Error>;
    fn fetch_brief(&self, slug: &str) -> Result<Option<FetchedBrief>, Error>;
    fn planned_briefs(&self) -> Result<Listing, Error>;
    fn brief_named(&self, name: &str) -> Result<Option<String>, Error>;
    fn scope_queue(&self, assignee: &str) -> Result<Queue, Error>;
    fn named_ticket(&self, number: u64) -> Result<Option<NamedTicket>, Error>;
    /// The brief filed, or `None` when the board has nowhere to file it: the
    /// record this board was opened for names a team the workspace does not
    /// have.
    ///
    /// `None` rather than an error, as every other absence on this seam is: a
    /// destination that is not there is worth words about the file that records
    /// it, and nothing below here holds them. Nothing is created on the way to
    /// that answer — the create was never sent.
    ///
    /// Whatever else the board needs is resolved inside here and not by the
    /// caller: the label by the name [`NewBrief`] carries, and the status a
    /// freshly filed brief sits in. A workspace with no status of that name
    /// files the brief with no status at all.
    fn create_brief(&self, project: &NewBrief<'_>) -> Result<Option<FiledBrief>, Error>;
    fn create_ticket(&self, issue: &NewTicket<'_>) -> Result<FiledTicket, Error>;
    fn create_relation(&self, blocker: &str, waiting: &str) -> Result<String, Error>;
    fn comment_on_brief(&self, project: &str, body: &str) -> Result<String, Error>;
    fn comment_on_ticket(&self, issue: &str, body: &str) -> Result<String, Error>;
}

/// Where a board comes from: the scope's record and its key in, a [`Board`]
/// out, and the one seam both the headless verbs and the panel open theirs
/// through.
///
/// Both, because they answer different halves of "which board": the key says
/// which workspace is being spoken to and the record says where in it the work
/// goes. A board that captured its own destination when it was opened is what
/// lets the destination leave every verb — nothing below this seam is told a
/// team or a label.
///
/// Infallible, so a record naming a board this opener cannot speak to is a
/// refusal a verb answers with rather than one open can raise. Every caller
/// refuses such a scope before it opens anything.
///
/// The bounds are what the panel's workers need. A board is built from a record
/// and a key borrowed for as long as the target lives and is then owned by a
/// thread that outlives the press, which is the associated type's side; and the
/// opener itself crosses onto a thread, because the panel's cut resolves its
/// board over there and so opens it there too.
pub(crate) trait Opens: Clone + Send + 'static {
    type Board: Board + Send + 'static;

    fn open(&self, record: &ScopeRecord, key: &str) -> Self::Board;
}

/// The one [`Opens`] that opens a socket, and the only value in warlock that
/// does. Holds nothing: a board is built per request run, from a record and a
/// key read on one line and dropped with whatever used it.
///
/// The impl is in `linear.rs`, beside the [`Board`] it answers with.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Opener;

/// What [`Board::ticket_brief`] answers: the project, and each of its issues'
/// workflow state by name and type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TicketBrief {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) status: Option<String>,
    pub(crate) states: Vec<(String, Category)>,
}

impl TicketBrief {
    #[must_use]
    #[cfg(test)]
    pub(crate) fn new(status: &str, issues: &[(&str, &str)]) -> Self {
        Self {
            id: "project-1".to_owned(),
            name: "A brief".to_owned(),
            status: Some(status.to_owned()),
            states: issues
                .iter()
                .map(|(name, kind)| ((*name).to_owned(), Category::of(kind)))
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
    pub(crate) fn states(&self) -> &[(String, Category)] {
        &self.states
    }
}

/// How warlock's notes on a project start, which is what a project read filters
/// comments on and `cut.rs` writes and reads. Changing either orphans every note
/// already on a board: the slices they record are offered again.
pub(crate) const CUT_NOTE: &str = "Warlock cut slice ";
pub(crate) const SKIP_NOTE: &str = "Warlock skipped slice ";

/// A project as [`Board::fetch_brief`] reads it back, which is not the
/// [`FiledBrief`] a create answers with: what matters about a project that
/// already exists is what is written on it, and what matters about one that
/// has just been made is where to find it.
///
/// Two of these are allowed to be empty and neither is a broken answer. A
/// project filed into a workspace with no `Backlog` has no status at all, so a
/// gate on the status has to be able to say that as well as name a wrong one;
/// and a description can be emptied in Linear after it was filed, which the
/// caller that parses it will refuse in its own words.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FetchedBrief {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) content: String,
    pub(crate) url: String,
    pub(crate) status: Option<String>,
    pub(crate) notes: Vec<String>,
}

impl FetchedBrief {
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

/// What [`Board::planned_briefs`] answers: slug and name, in Linear's order.
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
    pub(crate) issues: Vec<QueuedTicket>,
}

impl Queue {
    #[must_use]
    pub fn new(issues: Vec<QueuedTicket>) -> Self {
        Self { issues }
    }

    /// In Linear's own order, which is not the order the queue is worked:
    /// choosing is somebody else's function, and it wants the queue as the board
    /// gave it.
    #[must_use]
    pub fn issues(&self) -> &[QueuedTicket] {
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
pub struct QueuedTicket {
    pub(crate) id: String,
    pub(crate) identifier: String,
    pub(crate) title: String,
    pub(crate) description: String,
    pub(crate) state: String,
    pub(crate) state_type: Category,
    pub(crate) priority: Priority,
    pub(crate) blockers: Vec<Blocker>,
}

impl QueuedTicket {
    #[must_use]
    pub fn new(
        id: impl Into<String>,
        identifier: impl Into<String>,
        title: impl Into<String>,
        state: impl Into<String>,
        state_type: Category,
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
    pub const fn state_type(&self) -> Category {
        self.state_type
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
    pub(crate) state_type: Category,
}

impl Blocker {
    #[must_use]
    pub fn new(
        identifier: impl Into<String>,
        assignee: Option<&str>,
        state_type: Category,
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
    pub const fn state_type(&self) -> Category {
        self.state_type
    }
}

/// How far along a workflow state is, in the three answers warlock acts on: an
/// issue nobody has begun, one somebody is on, and one that is out of the way.
///
/// [`Settled`](Self::Settled) is the judgement everything downstream asks for —
/// a blocker in it no longer holds anything up, and a named ticket in it is
/// refused as finished — so a word warlock does not recognise must not land
/// there. [`Category::of`] puts an unknown word in [`Started`](Self::Started)
/// instead: the corner where the issue is neither offered as fresh work nor
/// treated as done, which is what a seventh Linear type or a Jira category
/// nobody has seen should cost.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Category {
    New,
    Started,
    Settled,
}

impl Category {
    /// Both boards' vocabularies in one match, because they do not collide:
    /// Linear answers a state's `type` and Jira a status category's `key`, and
    /// no word means one thing on one board and another on the other.
    #[must_use]
    pub fn of(word: &str) -> Self {
        match word {
            "triage" | "backlog" | "unstarted" | "new" => Self::New,
            "completed" | "canceled" | "done" => Self::Settled,
            _ => Self::Started,
        }
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
pub(crate) struct NamedTicket {
    pub(crate) issue: QueuedTicket,
    pub(crate) team: String,
    pub(crate) labels: Vec<String>,
    pub(crate) assignee: Option<Assignee>,
}

impl NamedTicket {
    #[must_use]
    #[cfg(test)]
    pub(crate) fn new(
        issue: QueuedTicket,
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
    pub(crate) const fn issue(&self) -> &QueuedTicket {
        &self.issue
    }

    /// The ticket itself, once the checks on the rest are through: what is worked
    /// is an issue and not a membership.
    #[must_use]
    pub(crate) fn into_issue(self) -> QueuedTicket {
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
/// what [`Board::viewer`] answers and the only honest way to ask whether a ticket
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

/// What [`Board::create_brief`] is asked for, and the whole of it: what the
/// brief is called and what it says, and the label by the name it goes by in
/// the workspace rather than as an id, because the create path is what resolves
/// it.
///
/// No team and no status. The team is the one on the record the board was
/// opened for, which is where every other verb's destination comes from too;
/// the status a freshly filed brief sits in is the board's own word for
/// "nobody has started this", and a caller that resolved one would be naming a
/// Linear project status to a board that may have no such thing.
#[derive(Debug, Clone, Copy)]
pub(crate) struct NewBrief<'a> {
    pub(crate) name: &'a str,
    pub(crate) content: &'a str,
    pub(crate) label: &'a str,
}

impl<'a> NewBrief<'a> {
    #[must_use]
    pub(crate) const fn new(name: &'a str, content: &'a str, label: &'a str) -> Self {
        Self {
            name,
            content,
            label,
        }
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
    pub(crate) const fn label(&self) -> &'a str {
        self.label
    }
}

/// A project that now exists. The URL is the one thing a failure downstream must
/// never lose, so it comes back from the create rather than being built from the
/// id here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FiledBrief {
    pub(crate) id: String,
    pub(crate) url: String,
}

impl FiledBrief {
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

/// What [`Board::create_ticket`] is asked for, and the whole of it: every field
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
/// `state` is a *team workflow state* id from [`Board::backlog_state`] and
/// `label` an *issue label* id from [`Board::ticket_label_id`]; neither a project
/// status nor a project label is usable here.
#[derive(Debug, Clone, Copy)]
pub(crate) struct NewTicket<'a> {
    pub(crate) title: &'a str,
    pub(crate) body: &'a str,
    pub(crate) team: &'a str,
    pub(crate) project: &'a str,
    pub(crate) label: &'a str,
    pub(crate) state: &'a str,
    pub(crate) assignee: &'a str,
}

impl<'a> NewTicket<'a> {
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
pub(crate) struct FiledTicket {
    pub(crate) id: String,
    pub(crate) identifier: String,
    pub(crate) url: String,
}

impl FiledTicket {
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
    pub(crate) fn listed(open: &QueuedTicket) -> Self {
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
    Key {
        board: &'static str,
    },
    /// Nothing was sent, because nothing is left to hear the answer: the run the
    /// request belonged to was stopped before it went out. Warlock's own word
    /// rather than the board's — no socket was opened, so the board knows
    /// nothing about it and there is nothing to undo.
    Stopped {
        board: &'static str,
    },
    /// Nothing was sent, because the scope the board was opened for files
    /// somewhere else: a verb that reads its destination off the record has
    /// none to read there. Warlock's own word, as [`Stopped`](Self::Stopped)
    /// is.
    ///
    /// Out of reach through warlock's own verbs, each of which refuses a scope
    /// it cannot file to before it opens anything. It is a refusal here and not
    /// a panic because [`Opens::open`] cannot fail, so the check cannot live
    /// where the board is built.
    Elsewhere {
        board: &'static str,
        scope: String,
    },
    /// Type-erased rather than the HTTP client's own error, which is what keeps
    /// this module free of one: a board that speaks over a socket boxes whatever
    /// its transport failed with, and a second board's transport need not be the
    /// first's. Naming a concrete error here points the dependency back at one
    /// board's wire.
    Transport {
        board: &'static str,
        source: Box<dyn std::error::Error + Send + Sync>,
    },
    Status {
        board: &'static str,
        code: u16,
    },
    /// Warlock's own words about the answer's shape, never the answer's bytes:
    /// what comes back from an endpoint that is not the board's is whatever a
    /// captive portal or a proxy decided to send, and printing it is printing
    /// that.
    Malformed {
        board: &'static str,
        detail: String,
    },
    /// The board's words, for a request it understood and refused. Worth keeping
    /// as they are — "Entity not found" and "A project with that name exists"
    /// are the two the caller can act on.
    Refused {
        board: &'static str,
        message: String,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Key { board } => write!(f, "the {board} key cannot be sent as a header value"),
            Self::Stopped { board } => {
                write!(f, "the run was stopped, so nothing was sent to {board}")
            }
            Self::Elsewhere { board, scope } => {
                write!(f, "`{scope}` does not file to {board}, so nothing was sent")
            }
            Self::Transport { board, source } => write!(f, "could not reach {board}: {source}"),
            Self::Status { board, code } => write!(f, "{board} answered {code}"),
            Self::Malformed { board, detail } => {
                write!(f, "{board}'s answer was unreadable: {detail}")
            }
            Self::Refused { board, message } => {
                write!(f, "{board} refused the request: {message}")
            }
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Transport { source, .. } => Some(&**source),
            Self::Key { .. }
            | Self::Stopped { .. }
            | Self::Elsewhere { .. }
            | Self::Status { .. }
            | Self::Malformed { .. }
            | Self::Refused { .. } => None,
        }
    }
}
