use std::fmt;

use warlock_engine::{FiledTo, ScopeRecord};

use crate::jira::{self, Jira};
use crate::linear::{self, Linear, Resolved};

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

    /// The vendor's name for this board as opened, for a sentence warlock words
    /// itself. A method beside [`NAME`](Board::NAME) because [`Boards`] knows
    /// its vendor only once a record has chosen it.
    fn vendor(&self) -> &'static str {
        Self::NAME
    }

    /// Whatever one board has to have in hand before it can create a ticket, in
    /// that board's own terms: a team and a column as ids here, and nothing at
    /// all for a board whose create resolves nothing.
    ///
    /// An associated type rather than a shape written in this module's words,
    /// because there are no words for it: this is the one thing on the seam that
    /// is a board's own protocol, and a flow that could read it would be holding
    /// that protocol again. [`ticket_filing`](Self::ticket_filing) answers one,
    /// [`create_ticket`](Self::create_ticket) takes it back, and between the two
    /// the cut carries a value it has no method to look inside.
    type Filing;

    fn viewer(&self) -> Result<String, Error>;
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
    /// The ticket moved to the board's status of that name, and [`Moved`] saying
    /// whether a status of that name was there to move it to.
    ///
    /// The name and not an id, because what a status is named is the only thing
    /// two boards agree on: a Linear workflow state has a global id and a Jira
    /// status is reachable only through the transitions of the one issue, so the
    /// board is what turns a name into whatever the move needs.
    ///
    /// [`Moved::Unreached`] and not an error, and the callers lean on it: a
    /// board whose workflow nobody has given an `In Progress` or a review state
    /// is a board warlock reports on and works past, and a run that stopped over
    /// it would be the board wagging the pull. An answer and not a silent
    /// success, so a caller cannot mistake "no such column" for a move that
    /// happened.
    fn move_ticket(&self, issue: &str, status: &str) -> Result<Moved, Error>;
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
    /// Whatever this board must settle before a cut files anything, asked once
    /// for the cut and answered before its first ticket exists.
    ///
    /// The brief its tickets file under is the one thing the verb is told, by
    /// the address that board answers one with: a board whose tickets hang off
    /// the brief has to be able to reach it, and a brief nothing is there for is
    /// a cut that cannot file. A board whose tickets carry the brief as a field
    /// of their own reads nothing of it.
    ///
    /// The one verb here whose answer is a value and not a fact: a board
    /// resolves what its own create wants — a team and a column on a board that
    /// writes those as ids, nothing at all on one that does not — and the
    /// [`Filing`](Self::Filing) comes straight back to
    /// [`create_ticket`](Self::create_ticket) unread.
    ///
    /// Both absences are answered rather than raised, as every other absence on
    /// this seam is: the words belong to the caller, which holds the file that
    /// records where a scope files. They are told apart because they are two
    /// different things to fix — a destination the board does not have is a
    /// manifest naming the wrong one, and a destination with no column for a
    /// ticket nobody has started is a board nobody has finished setting up.
    ///
    /// Nothing is created on the way to either answer, and that ordering is the
    /// promise: a destination a cut cannot file into is refused while the slice
    /// is still nothing rather than half filed.
    fn ticket_filing(&self, brief: &str) -> Result<Filable<Self::Filing>, Error>;
    fn create_ticket(
        &self,
        issue: &NewTicket<'_>,
        filing: &Self::Filing,
    ) -> Result<FiledTicket, Error>;
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
/// lets the destination leave every verb — no verb below this seam takes a
/// team, and the label a create carries is a name rather than something a
/// caller resolved.
///
/// Infallible, so a record naming a board this opener cannot speak to is a
/// refusal a verb answers with rather than one open can raise.
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
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Opener;

impl Opens for Opener {
    type Board = Boards;

    /// The client from the key and the destination from the record, which is
    /// the whole of what a board is built from: the record is cloned because
    /// the board outlives the manifest it was read from, and the Jira client
    /// takes the site off it as well, because a project key means nothing
    /// without the site it is a key on.
    ///
    /// The tracker is the only thing read here. Whether the key is this board's
    /// is the board's own business — the store judges nothing on the way in and
    /// this cannot fail — so a Linear key recorded against a Jira scope is a
    /// refusal the first request answers with.
    fn open(&self, record: &ScopeRecord, key: &str) -> Boards {
        match record.filed_to() {
            FiledTo::Linear => {
                Boards::Linear(Linear::new(linear::Client::new(key), record.clone()))
            }
            FiledTo::Jira { base_url, .. } => {
                Boards::Jira(Jira::new(jira::Client::new(base_url, key), record.clone()))
            }
        }
    }
}

/// The [`Board`] [`Opener`] answers with: whichever of the two warlock speaks
/// to, chosen once from the record and held for the run.
///
/// An enum and not a `Box<dyn Board>`. [`Filing`](Board::Filing) is an
/// associated type and [`NAME`](Board::NAME) an associated const, so the trait
/// is not object-safe, and the change that would make it so is erasing the
/// filing — the one thing on this seam that is a board's own protocol, and the
/// reason a cut can carry a value it has no method to read.
#[derive(Debug)]
pub(crate) enum Boards {
    Linear(Linear<linear::Client>),
    Jira(Jira<jira::Client>),
}

impl Board for Boards {
    /// Not a vendor's name: an associated const cannot vary with the variant,
    /// and what reads this is the cancel gate in `puller.rs`, whose sentence is
    /// about a request that was never sent — no socket was opened, so there is
    /// no vendor in it to name.
    const NAME: &'static str = "the board";

    /// Its own, over the two boards' own: each of them settles a different
    /// thing, and neither can be made from the other.
    type Filing = Filings;

    fn vendor(&self) -> &'static str {
        match self {
            Self::Linear(board) => board.vendor(),
            Self::Jira(board) => board.vendor(),
        }
    }

    fn viewer(&self) -> Result<String, Error> {
        match self {
            Self::Linear(board) => board.viewer(),
            Self::Jira(board) => board.viewer(),
        }
    }

    fn move_brief(&self, project: &str, status: &str) -> Result<bool, Error> {
        match self {
            Self::Linear(board) => board.move_brief(project, status),
            Self::Jira(board) => board.move_brief(project, status),
        }
    }

    fn ticket_brief(&self, issue: &str) -> Result<Option<TicketBrief>, Error> {
        match self {
            Self::Linear(board) => board.ticket_brief(issue),
            Self::Jira(board) => board.ticket_brief(issue),
        }
    }

    fn move_ticket(&self, issue: &str, status: &str) -> Result<Moved, Error> {
        match self {
            Self::Linear(board) => board.move_ticket(issue, status),
            Self::Jira(board) => board.move_ticket(issue, status),
        }
    }

    fn fetch_brief(&self, slug: &str) -> Result<Option<FetchedBrief>, Error> {
        match self {
            Self::Linear(board) => board.fetch_brief(slug),
            Self::Jira(board) => board.fetch_brief(slug),
        }
    }

    fn planned_briefs(&self) -> Result<Listing, Error> {
        match self {
            Self::Linear(board) => board.planned_briefs(),
            Self::Jira(board) => board.planned_briefs(),
        }
    }

    fn brief_named(&self, name: &str) -> Result<Option<String>, Error> {
        match self {
            Self::Linear(board) => board.brief_named(name),
            Self::Jira(board) => board.brief_named(name),
        }
    }

    fn scope_queue(&self, assignee: &str) -> Result<Queue, Error> {
        match self {
            Self::Linear(board) => board.scope_queue(assignee),
            Self::Jira(board) => board.scope_queue(assignee),
        }
    }

    fn named_ticket(&self, number: u64) -> Result<Option<NamedTicket>, Error> {
        match self {
            Self::Linear(board) => board.named_ticket(number),
            Self::Jira(board) => board.named_ticket(number),
        }
    }

    fn create_brief(&self, project: &NewBrief<'_>) -> Result<Option<FiledBrief>, Error> {
        match self {
            Self::Linear(board) => board.create_brief(project),
            Self::Jira(board) => board.create_brief(project),
        }
    }

    /// Whichever board this is, and its own answer about the destination
    /// carried through in [`Filings`]: a board that can file says so here, so a
    /// Jira project with a status for a fresh ticket and an Epic to hang them
    /// off arrives at the cut as a destination that is there rather than as one
    /// that is not.
    ///
    /// Nothing is read of what either board settled, as nothing anywhere above
    /// this seam reads one: the variant it is wrapped in is the whole of what
    /// this enum knows about it, and that is only so the create below can hand
    /// it back to the board that made it.
    fn ticket_filing(&self, brief: &str) -> Result<Filable<Filings>, Error> {
        match self {
            Self::Linear(board) => Ok(board.ticket_filing(brief)?.mapped(Filings::Linear)),
            Self::Jira(board) => Ok(board.ticket_filing(brief)?.mapped(Filings::Jira)),
        }
    }

    fn create_ticket(&self, issue: &NewTicket<'_>, filing: &Filings) -> Result<FiledTicket, Error> {
        match (self, filing) {
            (Self::Linear(board), Filings::Linear(filing)) => board.create_ticket(issue, filing),
            (Self::Jira(board), Filings::Jira(filing)) => board.create_ticket(issue, filing),
            // Out of reach: a [`Filings`] is built in `ticket_filing` above, on
            // the same value of this enum the create is then asked of, and
            // nothing outside this module can make one — so the board that
            // settled a filing is the board handed it back. A refusal rather
            // than a panic, because a cut that reaches a create has a slice half
            // on the board behind it, and a sentence is something a person can
            // act on where a panic in the middle of one is not.
            (board, _) => Err(Error::Crossed {
                board: board.vendor(),
            }),
        }
    }

    fn create_relation(&self, blocker: &str, waiting: &str) -> Result<String, Error> {
        match self {
            Self::Linear(board) => board.create_relation(blocker, waiting),
            Self::Jira(board) => board.create_relation(blocker, waiting),
        }
    }

    fn comment_on_brief(&self, project: &str, body: &str) -> Result<String, Error> {
        match self {
            Self::Linear(board) => board.comment_on_brief(project, body),
            Self::Jira(board) => board.comment_on_brief(project, body),
        }
    }

    fn comment_on_ticket(&self, issue: &str, body: &str) -> Result<String, Error> {
        match self {
            Self::Linear(board) => board.comment_on_ticket(issue, body),
            Self::Jira(board) => board.comment_on_ticket(issue, body),
        }
    }
}

/// What [`Board::move_ticket`] came to.
///
/// Two answers rather than a bool, because the half a caller has to word is the
/// one where nothing moved: the person reading that line is looking at a ticket
/// still in the column it was in, and what they need is the name of somewhere it
/// could have gone instead. A bool leaves them with neither that nor where it
/// stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Moved {
    /// A status of that name was reached, and the ticket is in it.
    Reached,
    /// The board offered no way into a status of that name, so nothing was
    /// moved.
    ///
    /// Both facts are the board's own, and a board that cannot say either says
    /// nothing: Jira reads them off the one issue, whose transitions are the
    /// whole of where it can go, where Linear reaches this answer through a team
    /// or a state it looked for and did not find, which says nothing about the
    /// issue.
    Unreached {
        reachable: Vec<String>,
        standing: Option<String>,
    },
}

impl Moved {
    /// Nothing moved, and the board naming neither where the ticket could have
    /// gone nor where it stands: what a board answers when what it failed to
    /// find was a status, so it never asked the ticket anything.
    #[must_use]
    pub(crate) const fn unreached() -> Self {
        Self::Unreached {
            reachable: Vec::new(),
            standing: None,
        }
    }

    #[must_use]
    pub(crate) const fn reached(&self) -> bool {
        matches!(self, Self::Reached)
    }
}

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
///
/// `draftable` is the board's own answer to whether the brief is open for
/// drafting, and the status beside it is only ever named in a refusal: which
/// of a board's status words mean a brief may be cut is a fact that board
/// states, so no caller reads the name to decide.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FetchedBrief {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) content: String,
    pub(crate) url: String,
    pub(crate) status: Option<String>,
    pub(crate) draftable: bool,
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
        draftable: bool,
    ) -> Self {
        Self {
            id: "project-1".to_owned(),
            name: name.into(),
            content: content.into(),
            url: url.into(),
            status: status.map(ToOwned::to_owned),
            draftable,
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

    #[must_use]
    pub(crate) fn draftable(&self) -> bool {
        self.draftable
    }

    /// The bodies of warlock's cut and skip notes, oldest first.
    #[must_use]
    pub(crate) fn notes(&self) -> &[String] {
        &self.notes
    }
}

/// What [`Board::planned_briefs`] answers, in the order the board listed them:
/// each brief's own address, as that board addresses one, and its name — a slug
/// and a project name on Linear, an issue key and a summary on Jira.
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

    /// In the board's own order, whatever that is, and not the order the queue
    /// is worked: choosing is somebody else's function, and it wants the queue
    /// as the board gave it.
    #[must_use]
    pub fn issues(&self) -> &[QueuedTicket] {
        &self.issues
    }
}

/// One issue in a scope's queue, carrying everything choosing needs and nothing
/// else.
///
/// Both names a board has for an issue, because both are needed: the id writes a
/// state move and the identifier is what a person types and a run record holds.
/// One value can be both, and is on Jira, where the key a person types is also
/// the address every request is built from.
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
/// identifier to name it, whose it is, and the state type to say whether it is
/// still in the way.
///
/// No id and no title: nothing is done to a blocker, it is only reported.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Blocker {
    pub(crate) identifier: String,
    pub(crate) holder: Holder,
    pub(crate) state_type: Category,
}

/// Whose a [`Blocker`] is, as far as the board said.
///
/// Three answers and not an `Option`, because "nobody" and "not read" print
/// differently: Jira carries no assignee on the far side of a link, and calling
/// that blocker unassigned would tell a person something false about work that
/// is somebody's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Holder {
    /// The person's name and not their id: a blocker is never matched against
    /// anybody, only printed.
    Named(String),
    /// Assigned to nobody, which a queue filtered by assignee can still be held
    /// up by.
    Nobody,
    /// The board's answer did not say.
    Unread,
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
            holder: assignee.map_or(Holder::Nobody, |name| Holder::Named(name.to_owned())),
            state_type,
        }
    }

    #[must_use]
    pub fn identifier(&self) -> &str {
        &self.identifier
    }

    #[must_use]
    pub const fn holder(&self) -> &Holder {
        &self.holder
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

/// How urgent a ticket is, in the order work is taken rather than in either
/// board's own spelling of it.
///
/// The derived ordering is the whole point: ascending is urgent first and no
/// priority last, so sorting a queue by this sorts it the way the board reads.
/// Linear's own numbers — `0` for none, `1` for urgent — sort the other way
/// round, and Jira's names do not sort at all, which is why nothing outside the
/// parsing that reads them holds either.
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

/// What [`Board::ticket_filing`] answers: a destination a cut can file into,
/// carrying whatever that board's creates need, or which half of the
/// destination is not there.
///
/// Two absences rather than one, because the caller says a different sentence
/// about each and names a different thing in it. The value in
/// [`Ready`](Self::Ready) is a type parameter and not a shape with fields,
/// which is what makes "handed back unread" something the compiler holds: a
/// flow is generic over the board and so over this, and a generic value has
/// nothing to read.
#[derive(Debug)]
pub(crate) enum Filable<F> {
    /// The destination is there and can take a ticket, with whatever the board
    /// settled for the cut.
    Ready(F),
    /// The record this board was opened for names a destination the board does
    /// not have at all.
    NoDestination,
    /// The destination is there, and nothing in it is where a ticket nobody has
    /// started belongs.
    NoColumn,
}

impl<F> Filable<F> {
    /// The same answer about the destination, with whatever the board settled
    /// put into another shape.
    ///
    /// The one thing [`Boards`] does with a filing: say which board's it is
    /// without reading it. Private to this module, because it is the only place
    /// a filing changes shape at all — everywhere else on this seam one is
    /// carried from the verb that answered it to the create that takes it back.
    fn mapped<G>(self, into: impl FnOnce(F) -> G) -> Filable<G> {
        match self {
            Self::Ready(filing) => Filable::Ready(into(filing)),
            Self::NoDestination => Filable::NoDestination,
            Self::NoColumn => Filable::NoColumn,
        }
    }
}

/// The [`Filing`](Board::Filing) [`Boards`] settles: whichever board it opened,
/// holding that board's own.
///
/// An enum for the same reason [`Boards`] is one, and it holds types whose
/// fields are private to the modules that built them — so nothing here can read
/// either half. This is a value on its way back to the board that made it, and
/// this enum is the one thing in the middle that has to name both.
#[derive(Debug)]
pub(crate) enum Filings {
    Linear(Resolved),
    Jira(jira::Settled),
}

/// What [`Board::create_ticket`] is asked for, and the whole of it: priority,
/// estimate, cycle and milestone stay unwritten. A draft says what the work is,
/// and a field warlock would have to invent a value for is a decision taken
/// away from the person who owns the board.
///
/// `assignee` is the exception, and it is not an invented value: the person
/// running `draft` is the person who owns the board, and the assignee is the
/// claim `warlock pull` reads — it only ever works tickets assigned to the
/// operator, so a backlog nothing is assigned in is a queue it can never select
/// from. Handing work to a teammate stays a reassignment a human makes in
/// Linear, which is why there is no way to name anybody else here.
///
/// `label` is the name the label goes by in the workspace and not an id, as
/// [`NewBrief`]'s is: what a label is called is the only thing two boards agree
/// on — one of them has no id for a label at all — so the board is what turns
/// the name into whatever its own create takes.
///
/// No team and no column. Both are a board's own business, settled once for the
/// cut by [`Board::ticket_filing`] and handed back to the create beside this,
/// so nothing a caller resolved reaches here.
#[derive(Debug, Clone, Copy)]
pub(crate) struct NewTicket<'a> {
    pub(crate) title: &'a str,
    pub(crate) body: &'a str,
    pub(crate) project: &'a str,
    pub(crate) label: &'a str,
    pub(crate) assignee: &'a str,
}

impl<'a> NewTicket<'a> {
    #[must_use]
    pub(crate) const fn new(
        title: &'a str,
        body: &'a str,
        project: &'a str,
        label: &'a str,
        assignee: &'a str,
    ) -> Self {
        Self {
            title,
            body,
            project,
            label,
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

/// Nothing here carries the key, and the variants carrying words say why they
/// carry those.
#[derive(Debug)]
pub enum Error {
    /// The key cannot be a header value at all — a newline or a byte outside
    /// ASCII in whatever was stored. It is named and not quoted: this type is
    /// printed.
    Key {
        board: &'static str,
    },
    /// Nothing was sent, because the stored key is not shaped like this board's:
    /// a Jira key is an account email and an API token on one line, and a line
    /// missing either half authenticates as nobody. The shape is warlock's own
    /// word for what a person types, which is why it is static and why nothing
    /// of the stored value is here beside it.
    Mistyped {
        board: &'static str,
        expected: &'static str,
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
    /// Out of reach through [`Opener`], which reads the record's tracker and
    /// builds the variant it names, so the board and the record it holds agree
    /// by construction. It is a refusal here and not a panic because
    /// [`Opens::open`] cannot fail, so the check cannot live where the board is
    /// built: the verb that goes to read the destination is the first thing
    /// that could see a mismatch, and a request not made is worth a sentence.
    Elsewhere {
        board: &'static str,
        scope: String,
    },
    /// Nothing was sent, because warlock has no request for this verb on this
    /// board yet: the board can do the thing and warlock cannot ask it. The
    /// refusal a board implemented a slice at a time answers with until the
    /// slice that teaches it arrives, so a flow that reaches such a verb fails
    /// with a sentence rather than a panic.
    ///
    /// Warlock's own words, as [`Stopped`](Self::Stopped) and
    /// [`Elsewhere`](Self::Elsewhere) are, and both halves static: the verb is
    /// named in this crate's words, so nothing the request was carrying — a
    /// project key, an issue key, a label — can reach the sentence.
    Untaught {
        board: &'static str,
        verb: &'static str,
    },
    /// Nothing was sent, because the value a cut settled for its filing is the
    /// other board's: a create can only be spelled by the board that resolved
    /// what goes in it.
    ///
    /// Out of reach through [`Boards`], which settles a filing on the same
    /// value it is then asked to create against and is the only thing that can
    /// build a [`Filings`] at all. It is a refusal here and not a panic because
    /// a cut that reaches a create has already filed part of a slice, and a
    /// sentence is something a person can act on. Warlock's own words, as
    /// [`Stopped`](Self::Stopped) and [`Elsewhere`](Self::Elsewhere) are.
    Crossed {
        board: &'static str,
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
    /// A 404, kept apart from [`Error::Refused`] so a caller asking whether the
    /// thing it addressed is there reads the status and not the vendor's prose,
    /// which also says "does not exist" about a field or an issue type. The
    /// board's words ride along for the sentence when the body carried any.
    Absent {
        board: &'static str,
        message: Option<String>,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Key { board } => write!(f, "the {board} key cannot be sent as a header value"),
            Self::Mistyped { board, expected } => {
                write!(
                    f,
                    "the {board} key must be typed `{expected}`, and the stored one is not"
                )
            }
            Self::Stopped { board } => {
                write!(f, "the run was stopped, so nothing was sent to {board}")
            }
            Self::Elsewhere { board, scope } => {
                write!(f, "`{scope}` does not file to {board}, so nothing was sent")
            }
            Self::Untaught { board, verb } => {
                write!(
                    f,
                    "warlock cannot {verb} on a {board} scope yet, so nothing was sent"
                )
            }
            Self::Crossed { board } => write!(
                f,
                "the filing this cut settled is not {board}'s, so nothing was sent"
            ),
            Self::Transport { board, source } => write!(f, "could not reach {board}: {source}"),
            Self::Status { board, code } => write!(f, "{board} answered {code}"),
            Self::Malformed { board, detail } => {
                write!(f, "{board}'s answer was unreadable: {detail}")
            }
            Self::Refused { board, message } => {
                write!(f, "{board} refused the request: {message}")
            }
            Self::Absent {
                board,
                message: Some(message),
            } => write!(f, "{board} answered 404: {message}"),
            Self::Absent {
                board,
                message: None,
            } => write!(f, "{board} answered 404"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Transport { source, .. } => Some(&**source),
            Self::Key { .. }
            | Self::Mistyped { .. }
            | Self::Stopped { .. }
            | Self::Elsewhere { .. }
            | Self::Untaught { .. }
            | Self::Crossed { .. }
            | Self::Status { .. }
            | Self::Malformed { .. }
            | Self::Refused { .. }
            | Self::Absent { .. } => None,
        }
    }
}

#[cfg(test)]
#[path = "tests/board.rs"]
mod tests;
