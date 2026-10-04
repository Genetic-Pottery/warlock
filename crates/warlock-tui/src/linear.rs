//! The one module in this library that opens a socket, and the reason it is
//! `ureq` rather than `reqwest`: reqwest's blocking client runs a tokio runtime
//! on a background thread, and a terminal program that links one has an
//! executor it never asked for. No async runtime here either, for the reason
//! `claude.rs` has none.
//!
//! No retry and no backoff. A brief filed twice is two projects on the board,
//! and nothing on this side can tell a timeout that arrived before the mutation
//! ran from one that arrived after — so a failed call is reported and the person
//! decides, which is the only honest answer available to a non-idempotent
//! mutation. A mutation is one request; a read that [`paged`] walks is one
//! request per page, and a failed page fails the read.

use std::fmt;
use std::time::Duration;

use serde_json::{Value, json};
use ureq::Agent;
use ureq::http::HeaderValue;
use ureq::http::header::AUTHORIZATION;

/// The clock one call runs under, end to end: the DNS lookup, the connection,
/// the TLS handshake, the request, the answer and its body are all inside it.
///
/// Not [`INVOCATION_TIMEOUT`](crate::INVOCATION_TIMEOUT): five minutes is the
/// measure of a model pass thinking, and a GraphQL call that has not answered in
/// thirty seconds is not about to.
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// Linear has one endpoint and it is GraphQL over `POST`. There is no REST
/// surface to fall back to and no per-resource path to build, so every operation
/// in this module is the same URL with a different document.
const ENDPOINT: &str = "https://api.linear.app/graphql";

/// One GraphQL round trip: a document, its variables, and the answer's `data`
/// object.
///
/// The transport under [`Linear`], and the seam this module's own tests drive
/// with an in-memory stand-in to check how each operation is spelled. Nothing
/// outside this module is written against it: the flows take a [`Board`].
/// Unwrapping the envelope belongs here rather than to each caller: a stand-in
/// hands back the `data` a real answer would have carried, and says a refusal
/// by returning [`Error::Refused`].
///
/// ```no_run
/// use serde_json::json;
/// use warlock_tui::{LinearClient, Posts};
///
/// // Reaches the real Linear, so this example is not executed by the test suite.
/// let client = LinearClient::new("lin_api_example");
///
/// println!("{}", client.post("query { viewer { id } }", json!({}))?);
/// # Ok::<(), warlock_tui::LinearError>(())
/// ```
pub trait Posts {
    fn post(&self, document: &str, variables: Value) -> Result<Value, Error>;
}

/// A client for one workspace, holding the key it authenticates with.
///
/// The key is a constructor parameter, as the home directory is a parameter
/// through `sigils.rs` and `keys.rs`: nothing in this module reads an
/// environment variable, a file or the key store, so no test in this crate can
/// reach the developer's real credentials by accident.
///
/// ```
/// use warlock_tui::{LinearClient, REQUEST_TIMEOUT};
///
/// let client = LinearClient::new("lin_api_example");
///
/// assert_eq!(client.timeout(), Some(REQUEST_TIMEOUT));
/// // The key is held and never shown.
/// assert!(!format!("{client:?}").contains("lin_api_example"));
/// ```
pub struct Client {
    key: String,
    agent: Agent,
}

impl Client {
    #[must_use]
    pub fn new(key: impl Into<String>) -> Self {
        let config = Agent::config_builder()
            // Global rather than any of the per-phase timeouts: what a caller
            // waits on is the whole call, and a resolve, a connect and a body
            // read with thirty seconds each is a minute and a half.
            .timeout_global(Some(REQUEST_TIMEOUT))
            .build();

        Self {
            key: key.into(),
            agent: Agent::new_with_config(config),
        }
    }

    #[must_use]
    pub fn timeout(&self) -> Option<Duration> {
        self.agent.config().timeouts().global
    }
}

// Hand-written because a derive would print the key, and `Debug` is what a
// failed assertion, a panic message and an error chain all print.
impl fmt::Debug for Client {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_struct("Client").finish_non_exhaustive()
    }
}

impl Posts for Client {
    fn post(&self, document: &str, variables: Value) -> Result<Value, Error> {
        let mut response = self
            .agent
            .post(ENDPOINT)
            .header(AUTHORIZATION, authorization(&self.key)?)
            .send_json(json!({ "query": document, "variables": variables }))
            .map_err(failure)?;

        answer(response.body_mut().read_json().map_err(failure)?)
    }
}

/// The operations warlock asks of a board, in the words of the board rather
/// than of GraphQL.
///
/// The flows are written against this and not against [`Posts`], so a flow's
/// test fake answers operations instead of recognising query text: Linear's
/// spelling is [`Linear`]'s business, checked in this module's own tests.
pub(crate) trait Board {
    fn viewer(&self) -> Result<String, Error>;
    fn team_id(&self, key: &str) -> Result<Option<String>, Error>;
    fn backlog_status(&self) -> Result<Option<String>, Error>;
    fn project_status(&self, name: &str) -> Result<Option<String>, Error>;
    fn move_project(&self, project: &str, status: &str) -> Result<String, Error>;
    fn issue_project(&self, issue: &str) -> Result<Option<IssueProject>, Error>;
    fn backlog_state(&self, team: &str) -> Result<Option<String>, Error>;
    fn workflow_state(&self, team: &str, name: &str) -> Result<Option<String>, Error>;
    fn move_issue(&self, issue: &str, state: &str) -> Result<String, Error>;
    fn issue_label_id(&self, name: &str, team: &str) -> Result<String, Error>;
    fn fetch_project(&self, slug: &str) -> Result<Option<FetchedProject>, Error>;
    fn planned_projects(&self, team: &str, label: &str) -> Result<Listing, Error>;
    fn project_named(&self, team: &str, name: &str) -> Result<Option<String>, Error>;
    fn scope_queue(&self, team: &str, label: &str, assignee: &str) -> Result<Queue, Error>;
    fn named_issue(&self, team: &str, number: u64) -> Result<Option<NamedIssue>, Error>;
    fn create_project(&self, project: &NewProject<'_>) -> Result<Project, Error>;
    fn create_issue(&self, issue: &NewIssue<'_>) -> Result<Issue, Error>;
    fn create_relation(&self, blocker: &str, waiting: &str) -> Result<String, Error>;
    fn comment_on_project(&self, project: &str, body: &str) -> Result<String, Error>;
    fn comment_on_issue(&self, issue: &str, body: &str) -> Result<String, Error>;
}

/// The one [`Board`] that speaks GraphQL, over whatever [`Posts`] it holds.
#[derive(Debug)]
pub(crate) struct Linear<P = Client> {
    posts: P,
}

impl<P: Posts> Linear<P> {
    #[must_use]
    pub(crate) const fn new(posts: P) -> Self {
        Self { posts }
    }
}

impl<P: Posts> Board for Linear<P> {
    fn viewer(&self) -> Result<String, Error> {
        viewer(&self.posts)
    }

    fn team_id(&self, key: &str) -> Result<Option<String>, Error> {
        team_id(&self.posts, key)
    }

    fn backlog_status(&self) -> Result<Option<String>, Error> {
        backlog_status(&self.posts)
    }

    fn project_status(&self, name: &str) -> Result<Option<String>, Error> {
        project_status(&self.posts, name)
    }

    fn move_project(&self, project: &str, status: &str) -> Result<String, Error> {
        move_project(&self.posts, project, status)
    }

    fn issue_project(&self, issue: &str) -> Result<Option<IssueProject>, Error> {
        issue_project(&self.posts, issue)
    }

    fn backlog_state(&self, team: &str) -> Result<Option<String>, Error> {
        backlog_state(&self.posts, team)
    }

    fn workflow_state(&self, team: &str, name: &str) -> Result<Option<String>, Error> {
        workflow_state(&self.posts, team, name)
    }

    fn move_issue(&self, issue: &str, state: &str) -> Result<String, Error> {
        move_issue(&self.posts, issue, state)
    }

    fn issue_label_id(&self, name: &str, team: &str) -> Result<String, Error> {
        issue_label_id(&self.posts, name, team)
    }

    fn fetch_project(&self, slug: &str) -> Result<Option<FetchedProject>, Error> {
        fetch_project(&self.posts, slug)
    }

    fn planned_projects(&self, team: &str, label: &str) -> Result<Listing, Error> {
        planned_projects(&self.posts, team, label)
    }

    fn project_named(&self, team: &str, name: &str) -> Result<Option<String>, Error> {
        project_named(&self.posts, team, name)
    }

    fn scope_queue(&self, team: &str, label: &str, assignee: &str) -> Result<Queue, Error> {
        scope_queue(&self.posts, team, label, assignee)
    }

    fn named_issue(&self, team: &str, number: u64) -> Result<Option<NamedIssue>, Error> {
        named_issue(&self.posts, team, number)
    }

    fn create_project(&self, project: &NewProject<'_>) -> Result<Project, Error> {
        create_project(&self.posts, project)
    }

    fn create_issue(&self, issue: &NewIssue<'_>) -> Result<Issue, Error> {
        create_issue(&self.posts, issue)
    }

    fn create_relation(&self, blocker: &str, waiting: &str) -> Result<String, Error> {
        create_relation(&self.posts, blocker, waiting)
    }

    fn comment_on_project(&self, project: &str, body: &str) -> Result<String, Error> {
        comment_on_project(&self.posts, project, body)
    }

    fn comment_on_issue(&self, issue: &str, body: &str) -> Result<String, Error> {
        comment_on_issue(&self.posts, issue, body)
    }
}

/// Where a board comes from: a key in, a [`Board`] out, and the one seam both
/// the headless verbs and the panel open theirs through.
///
/// The bounds are what the panel's workers need. A board is built from a key
/// borrowed for as long as the target lives and is then owned by a thread that
/// outlives the press, which is the associated type's side; and the opener
/// itself crosses onto a thread, because the panel's cut resolves its board
/// over there and so opens it there too.
pub(crate) trait Opens: Clone + Send + 'static {
    type Board: Board + Send + 'static;

    fn open(&self, key: &str) -> Self::Board;
}

/// The one [`Opens`] that opens a socket, and the only value in warlock that
/// does. Holds nothing: a board is built per request run, from a key read on
/// one line and dropped with whatever used it.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Opener;

impl Opens for Opener {
    type Board = Linear<Client>;

    fn open(&self, key: &str) -> Linear<Client> {
        Linear::new(Client::new(key))
    }
}

/// The name a brief and its issues are filed under: a project status for a
/// project, a team's workflow state for an issue. Two unrelated types in
/// Linear's schema that a workspace spells the same way. A workspace or team
/// with nothing by this name takes the thing with no status at all, rather than
/// whichever one happens to sort first.
const BACKLOG: &str = "Backlog";

/// The user the key belongs to, as Linear's own user id.
///
/// No `Option`, unlike [`team_id`] and [`backlog_state`]: a workspace can be
/// missing a team or a state, but a request Linear answered at all was
/// authenticated as somebody, so an answer carrying no viewer is a malformed one
/// and not an absence for a caller to have words about.
fn viewer(linear: &impl Posts) -> Result<String, Error> {
    let data = linear.post("query Viewer { viewer { id } }", json!({}))?;

    node_id(data.get("viewer").ok_or_else(|| missing("viewer"))?)
}

/// A team key — `WAR` — as Linear's own team id, or `None` when the workspace
/// has no team by that key. Not an error: the caller holds the words about
/// `.warlock/pacts.toml` and this module does not.
fn team_id(linear: &impl Posts, key: &str) -> Result<Option<String>, Error> {
    let data = linear.post(
        "query Team($key: String!) {
            teams(filter: { key: { eq: $key } }, first: 1) { nodes { id } }
        }",
        json!({ "key": key }),
    )?;

    nodes(&data, "teams")?.first().map(node_id).transpose()
}

/// The id of the [`BACKLOG`] status, or `None` when the workspace has no status
/// by that name.
fn backlog_status(linear: &impl Posts) -> Result<Option<String>, Error> {
    // `projectStatuses` takes no filter, so every page is read and the match
    // happens here.
    paged(
        linear,
        "query ProjectStatuses($after: String) {
            projectStatuses(first: 250, after: $after) {
                pageInfo { hasNextPage endCursor }
                nodes { id name }
            }
        }",
        json!({}),
        &["projectStatuses"],
    )?
    .iter()
    .find(|status| status.get("name").and_then(Value::as_str) == Some(BACKLOG))
    .map(node_id)
    .transpose()
}

/// The id of the project status by name, or `None` when the workspace has none.
///
/// Matched trimmed and case-insensitively, as [`workflow_state`] matches, unlike
/// [`backlog_status`]'s exact match: a status that cannot be found here costs a
/// project its move and nothing else, so the looser match is the cheaper error.
fn project_status(linear: &impl Posts, name: &str) -> Result<Option<String>, Error> {
    paged(
        linear,
        "query ProjectStatuses($after: String) {
            projectStatuses(first: 250, after: $after) {
                pageInfo { hasNextPage endCursor }
                nodes { id name }
            }
        }",
        json!({}),
        &["projectStatuses"],
    )?
    .iter()
    .find(|status| {
        status
            .get("name")
            .and_then(Value::as_str)
            .is_some_and(|found| found.trim().eq_ignore_ascii_case(name.trim()))
    })
    .map(node_id)
    .transpose()
}

/// Move a project to a status, by ids. One request and no retry, per this
/// module's rule.
fn move_project(linear: &impl Posts, project: &str, status: &str) -> Result<String, Error> {
    let data = linear.post(
        "mutation ProjectUpdate($id: String!, $input: ProjectUpdateInput!) {
            projectUpdate(id: $id, input: $input) { project { id } }
        }",
        json!({ "id": project, "input": { "statusId": status } }),
    )?;

    node_id(payload(&data, "projectUpdate", "project")?)
}

/// The project an issue belongs to, with its status and the state of every
/// issue in it, or `None` for an issue in no project.
fn issue_project(linear: &impl Posts, issue: &str) -> Result<Option<IssueProject>, Error> {
    let data = linear.post(
        "query IssueProject($id: String!) {
            issue(id: $id) { project { id name status { name } } }
        }",
        json!({ "id": issue }),
    )?;

    let project = data
        .get("issue")
        .and_then(|issue| issue.get("project"))
        .ok_or_else(|| missing("project"))?;
    if project.is_null() {
        return Ok(None);
    }
    let id = node_id(project)?;

    let states = paged(
        linear,
        "query ProjectIssues($id: String!, $after: String) {
            project(id: $id) {
                issues(first: 250, after: $after) {
                    pageInfo { hasNextPage endCursor }
                    nodes { state { name type } }
                }
            }
        }",
        json!({ "id": id }),
        &["project", "issues"],
    )?
    .iter()
    .map(|issue| {
        let state = issue.get("state").ok_or_else(|| missing("state"))?;
        Ok((text(state, "name")?, StateType::new(text(state, "type")?)))
    })
    .collect::<Result<_, Error>>()?;

    Ok(Some(IssueProject {
        id,
        name: text(project, "name")?,
        status: nullable(project, "status", |status| text(status, "name"))?,
        states,
    }))
}

/// Every node of a connection, page after page, following `endCursor` while
/// `hasNextPage` says there is more. `path` is the fields from the answer's root
/// to the connection, and `document` takes the cursor as `$after`.
fn paged(
    linear: &impl Posts,
    document: &str,
    mut variables: Value,
    path: &[&str],
) -> Result<Vec<Value>, Error> {
    let mut found = Vec::new();
    loop {
        let data = linear.post(document, variables.clone())?;
        let connection = path
            .iter()
            .try_fold(&data, |node, field| node.get(*field))
            .ok_or_else(|| missing(path.last().copied().unwrap_or("data")))?;
        let (nodes, after) = page_of(connection)?;
        found.extend(nodes.iter().cloned());
        let Some(after) = after else {
            return Ok(found);
        };
        if let Some(fields) = variables.as_object_mut() {
            fields.insert("after".to_owned(), json!(after));
        }
    }
}

/// A connection that arrived inside a larger answer, made whole: its own nodes,
/// then — when its `pageInfo` says there is more — every later page through
/// [`paged`], starting at its `endCursor`.
fn completed(
    linear: &impl Posts,
    connection: &Value,
    document: &str,
    mut variables: Value,
    path: &[&str],
) -> Result<Vec<Value>, Error> {
    let (nodes, after) = page_of(connection)?;
    let mut found = nodes.to_vec();
    if let Some(after) = after {
        if let Some(fields) = variables.as_object_mut() {
            fields.insert("after".to_owned(), json!(after));
        }
        found.extend(paged(linear, document, variables, path)?);
    }
    Ok(found)
}

/// A connection's nodes, and the cursor to read on from when there is a page
/// after them.
fn page_of(connection: &Value) -> Result<(&[Value], Option<String>), Error> {
    let nodes = connection
        .get("nodes")
        .and_then(Value::as_array)
        .ok_or_else(|| missing("nodes"))?;
    let info = connection
        .get("pageInfo")
        .ok_or_else(|| missing("pageInfo"))?;
    let more = info
        .get("hasNextPage")
        .and_then(Value::as_bool)
        .ok_or_else(|| missing("hasNextPage"))?;
    let after = more.then(|| text(info, "endCursor")).transpose()?;
    Ok((nodes, after))
}

/// What [`Board::issue_project`] answers: the project, and each of its issues'
/// workflow state by name and type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct IssueProject {
    id: String,
    name: String,
    status: Option<String>,
    states: Vec<(String, StateType)>,
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

/// The id of the team's workflow state named [`BACKLOG`], or `None` when that
/// team has none. `None` rather than an error for [`team_id`]'s reason: a team
/// that cannot take an issue is worth words about the team, and this module does
/// not hold them.
///
/// [`workflow_state`] under a fixed name, which loosens what this used to do: the
/// match was on `Backlog` exactly and is now trimmed and case-insensitive, so a
/// team whose column reads `backlog` files into it instead of filing with no
/// state at all. Deliberate — the two resolvers reading the same list by
/// different rules is the kind of difference nobody discovers until a board is
/// spelled unusually.
fn backlog_state(linear: &impl Posts, team: &str) -> Result<Option<String>, Error> {
    workflow_state(linear, team, BACKLOG)
}

/// The id of the team's workflow state by name, or `None` when that team has no
/// state by it.
///
/// `None` rather than an error, and here it is load-bearing rather than tidy: the
/// caller moving an issue prints a missing column as one line and works the
/// ticket anyway. A workflow column is a courtesy to people who are not watching
/// the terminal, and nothing about the work depends on it.
///
/// Matched trimmed and case-insensitively, because `In progress` and
/// `In Progress` are the same column to everyone except a string comparison.
/// That rule is spelled out again in `queue.rs`'s own `named`, rather than
/// shared: this module is below that one — `queue.rs` reads the types parsed
/// here — and a helper pulled up from there would point the dependency
/// backwards for four words of code.
///
/// Workflow states belong to a team and not to the workspace, so this takes the
/// id [`team_id`] answered rather than the team key. Every page is read and the
/// match happens here, as [`backlog_status`] does.
fn workflow_state(linear: &impl Posts, team: &str, name: &str) -> Result<Option<String>, Error> {
    paged(
        linear,
        "query WorkflowStates($team: ID!, $after: String) {
            workflowStates(filter: { team: { id: { eq: $team } } }, first: 250, after: $after) {
                pageInfo { hasNextPage endCursor }
                nodes { id name }
            }
        }",
        json!({ "team": team }),
        &["workflowStates"],
    )?
    .iter()
    .find(|state| {
        state
            .get("name")
            .and_then(Value::as_str)
            .is_some_and(|found| found.trim().eq_ignore_ascii_case(name.trim()))
    })
    .map(node_id)
    .transpose()
}

/// A project already on the board, by the slug at the end of its URL, or `None`
/// when the workspace has no project with it.
///
/// `None` rather than an error for [`team_id`]'s reason: a slug the board does
/// not know is worth words about the command that typed it, and this module
/// does not hold them.
///
/// Only the comments that are warlock's cut and skip notes come back, filtered
/// on the wire: they are the whole of what records which slices are cut, and a
/// project's other talk would spend the pages on nothing. Every page is read,
/// because a note missed is a slice filed twice.
fn fetch_project(linear: &impl Posts, slug: &str) -> Result<Option<FetchedProject>, Error> {
    let data = match linear.post(
        "query Project($id: String!, $cut: String!, $skipped: String!) {
            project(id: $id) {
                id name content url status { name }
                comments(filter: { or: [
                    { body: { startsWith: $cut } }
                    { body: { startsWith: $skipped } }
                ] }, first: 250) {
                    pageInfo { hasNextPage endCursor }
                    nodes { body }
                }
            }
        }",
        json!({ "id": slug, "cut": CUT_NOTE, "skipped": SKIP_NOTE }),
    ) {
        Ok(data) => data,
        // `project(id:)` answers a `Project!` in Linear's schema, so an id the
        // workspace does not have arrives as a GraphQL error and never as a
        // null node. Both shapes are folded into the absence here, because a
        // caller that had to recognise "Entity not found" itself would be
        // reading Linear's prose in a second place.
        Err(Error::Refused { message }) if unknown_entity(&message) => return Ok(None),
        Err(error) => return Err(error),
    };

    let Some(project) = data.get("project") else {
        return Err(Error::Malformed {
            detail: "the answer carried no `project`".to_owned(),
        });
    };
    if project.is_null() {
        return Ok(None);
    }

    let id = node_id(project)?;
    let notes = completed(
        linear,
        project.get("comments").ok_or_else(|| missing("comments"))?,
        "query ProjectNotes($id: String!, $cut: String!, $skipped: String!, $after: String) {
            project(id: $id) {
                comments(filter: { or: [
                    { body: { startsWith: $cut } }
                    { body: { startsWith: $skipped } }
                ] }, first: 250, after: $after) {
                    pageInfo { hasNextPage endCursor }
                    nodes { body }
                }
            }
        }",
        json!({ "id": id, "cut": CUT_NOTE, "skipped": SKIP_NOTE }),
        &["project", "comments"],
    )?
    .iter()
    .map(|comment| text(comment, "body"))
    .collect::<Result<_, _>>()?;

    Ok(Some(FetchedProject {
        id,
        name: text(project, "name")?,
        content: nullable(project, "content", |_| text(project, "content"))?.unwrap_or_default(),
        url: text(project, "url")?,
        status: nullable(project, "status", |status| text(status, "name"))?,
        notes,
    }))
}

/// How warlock's notes on a project start, which is what [`fetch_project`]
/// filters on and `cut.rs` writes and reads. Changing either orphans every note
/// already on a board: the slices they record are offered again.
pub(crate) const CUT_NOTE: &str = "Warlock cut slice ";
pub(crate) const SKIP_NOTE: &str = "Warlock skipped slice ";

/// A project as [`Board::fetch_project`] reads it back, which is not the
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
    id: String,
    name: String,
    content: String,
    url: String,
    status: Option<String>,
    notes: Vec<String>,
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

/// The projects in the team that are `Planned` and carry warlock's label, by
/// slug and name, every page of them.
///
/// The label narrows it to what a push filed, which is what a draft can read:
/// a project somebody made by hand has no `## Scope` warlock wrote the shape of.
fn planned_projects(linear: &impl Posts, team: &str, label: &str) -> Result<Listing, Error> {
    let projects = paged(
        linear,
        r#"query PlannedProjects($team: String!, $label: String!, $after: String) {
            projects(
                filter: {
                    accessibleTeams: { some: { key: { eq: $team } } }
                    status: { name: { eqIgnoreCase: "Planned" } }
                    labels: { some: { name: { eq: $label } } }
                }
                first: 250
                after: $after
            ) {
                pageInfo { hasNextPage endCursor }
                nodes { slugId name }
            }
        }"#,
        json!({ "team": team, "label": label }),
        &["projects"],
    )?
    .iter()
    .map(|project| Ok((text(project, "slugId")?, text(project, "name")?)))
    .collect::<Result<_, Error>>()?;

    Ok(Listing { projects })
}

/// What [`Board::planned_projects`] answers: slug and name, in Linear's order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Listing {
    projects: Vec<(String, String)>,
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

/// The URL of a project in the team with this name, compared ignoring case, or
/// `None` when the team has none: the only thing a push can tell a second push
/// of the same brief by, now that nothing on the machine remembers the first.
fn project_named(linear: &impl Posts, team: &str, name: &str) -> Result<Option<String>, Error> {
    let data = linear.post(
        "query ProjectNamed($team: String!, $name: String!) {
            projects(
                filter: {
                    accessibleTeams: { some: { key: { eq: $team } } }
                    name: { eqIgnoreCase: $name }
                }
                first: 1
            ) { nodes { url } }
        }",
        json!({ "team": team, "name": name }),
    )?;

    nodes(&data, "projects")?
        .first()
        .map(|project| text(project, "url"))
        .transpose()
}

/// Issues read per page of a queue, which is a hundred rather than the 250 the
/// other reads ask for: every issue carries a page of relations under it, and
/// the two multiply into the size of one answer.
const QUEUE_PAGE: usize = 100;

/// Blocking relations read per issue in the page that carries it. Not left off:
/// a nested connection with no `first` takes whatever default Linear has today.
/// An issue with more is read on through [`issue_blockers`].
const BLOCKERS_PAGE: usize = 25;

/// The fields every issue a queue or a name reads is parsed from, and its first
/// page of blocking relations. One spelling, so the two reads cannot drift.
const QUEUED_FIELDS: &str = "
    id
    identifier
    title
    description
    priority
    state { name type }
    inverseRelations(first: $blockers) {
        pageInfo { hasNextPage endCursor }
        nodes {
            type
            issue { identifier state { type } assignee { name } }
        }
    }";

/// Every issue a scope's queue holds, every page of them.
///
/// The three filters are the whole of what makes an issue this scope's work: the
/// record's team, the record's label, and the user the key belongs to. The
/// assignee is a filter rather than a check afterwards because `pull` only ever
/// works tickets assigned to the operator — an issue somebody else holds is not
/// a ticket this queue has an opinion about.
///
/// The fourth filter is on the state's *type* and not its name: a team names its
/// workflow states what it likes, `Done` on one board is `Shipped` on the next,
/// and only the type says which of those names means finished. Filtering on the
/// wire rather than here is the difference between a page of live work and a page
/// that may be all history.
///
/// Blockers are read unfiltered, and that asymmetry is the point: an issue is
/// held up by whatever blocks it, whoever owns that and whatever label it
/// carries, so the relations must not inherit the queue's own filters. They are
/// read to the end, because a blocker list cut off midway makes a held-up issue
/// look ready.
fn scope_queue(
    linear: &impl Posts,
    team: &str,
    label: &str,
    assignee: &str,
) -> Result<Queue, Error> {
    let document = format!(
        r#"query ScopeQueue(
            $team: String!
            $label: String!
            $assignee: ID!
            $first: Int!
            $blockers: Int!
            $after: String
        ) {{
            issues(
                filter: {{
                    team: {{ key: {{ eq: $team }} }}
                    labels: {{ name: {{ eq: $label }} }}
                    assignee: {{ id: {{ eq: $assignee }} }}
                    state: {{ type: {{ nin: ["completed", "canceled"] }} }}
                }}
                first: $first
                after: $after
            ) {{
                pageInfo {{ hasNextPage endCursor }}
                nodes {{ {QUEUED_FIELDS} }}
            }}
        }}"#
    );
    let found = paged(
        linear,
        &document,
        json!({
            "team": team,
            "label": label,
            "assignee": assignee,
            "first": QUEUE_PAGE,
            "blockers": BLOCKERS_PAGE,
        }),
        &["issues"],
    )?;

    let issues = found
        .iter()
        .map(|node| queued_issue(linear, node))
        .collect::<Result<_, _>>()?;

    Ok(Queue { issues })
}

/// The blocking relations of one issue past the page its read carried.
fn issue_blockers(linear: &impl Posts, node: &Value) -> Result<Vec<Value>, Error> {
    completed(
        linear,
        node.get("inverseRelations")
            .ok_or_else(|| missing("inverseRelations"))?,
        "query IssueBlockers($id: String!, $after: String) {
            issue(id: $id) {
                inverseRelations(first: 250, after: $after) {
                    pageInfo { hasNextPage endCursor }
                    nodes {
                        type
                        issue { identifier state { type } assignee { name } }
                    }
                }
            }
        }",
        json!({ "id": node_id(node)? }),
        &["issue", "inverseRelations"],
    )
}

/// One issue, with every one of its blocking relations.
fn queued_issue(linear: &impl Posts, node: &Value) -> Result<QueuedIssue, Error> {
    let state = node.get("state").ok_or_else(|| missing("state"))?;
    let relations = issue_blockers(linear, node)?;

    let mut blockers = Vec::new();

    for relation in &relations {
        // A relation carrying no type at all is an unreadable answer rather than
        // one more relation to drop: skipping it quietly is how a held-up issue
        // comes to look ready.
        if text(relation, "type")? == BLOCKS {
            blockers.push(blocker(relation)?);
        }
    }

    let issue = QueuedIssue {
        id: node_id(node)?,
        identifier: text(node, "identifier")?,
        title: text(node, "title")?,
        // Null on a ticket nobody described, which is a ticket with nothing more
        // to say than its title rather than an unreadable answer.
        description: node
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        state: text(state, "name")?,
        state_type: StateType(text(state, "type")?),
        priority: priority(node)?,
        blockers,
    };

    Ok(issue)
}

/// The far side of one `blocks` relation on an issue's `inverseRelations`, which
/// is the issue doing the blocking.
///
/// The direction is the whole of what this reads, and it lives in which field is
/// taken from which connection: in `relations` an issue is the `issue` of the
/// edge and the far side is `relatedIssue`, so those are the issues it blocks;
/// in `inverseRelations` it is the `relatedIssue` and the far side is `issue`, so
/// those are the issues blocking it. Reading the wrong one of the two compiles,
/// answers the same shape, and inverts every dependency in the queue.
fn blocker(relation: &Value) -> Result<Blocker, Error> {
    let blocker = relation.get("issue").ok_or_else(|| missing("issue"))?;
    let state = blocker.get("state").ok_or_else(|| missing("state"))?;

    Ok(Blocker {
        identifier: text(blocker, "identifier")?,
        assignee: nullable(blocker, "assignee", |assignee| text(assignee, "name"))?,
        state_type: StateType(text(state, "type")?),
    })
}

/// Linear's word for the one relation type that holds work up, and the one
/// [`create_relation`] writes. The others — `related`, `duplicate`, `similar` —
/// block nothing and are read and dropped.
const BLOCKS: &str = "blocks";

/// Linear's `priority`, read into the order a queue is worked.
///
/// The number on the wire is not that order: `0` is *no* priority and `1` is
/// urgent, so an integer sorted as an integer puts the issue nobody has ranked
/// ahead of the one that is on fire. Nothing outside this function sees the
/// number.
fn priority(node: &Value) -> Result<Priority, Error> {
    let number = node
        .get("priority")
        .and_then(Value::as_f64)
        .ok_or_else(|| missing("priority"))?;

    // `priority` is a `Float!` in Linear's schema carrying one of five whole
    // numbers, so each rank is the interval around its number rather than an
    // equality on a float.
    Ok(if (0.5..1.5).contains(&number) {
        Priority::Urgent
    } else if (1.5..2.5).contains(&number) {
        Priority::High
    } else if (2.5..3.5).contains(&number) {
        Priority::Medium
    } else if (3.5..4.5).contains(&number) {
        Priority::Low
    } else {
        // `0` is Linear's own "no priority", and so is anything outside the
        // five: a rank this side cannot read is not a reason to work something
        // first.
        Priority::None
    })
}

/// A scope's queue, all of it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Queue {
    issues: Vec<QueuedIssue>,
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
    id: String,
    identifier: String,
    title: String,
    description: String,
    state: String,
    state_type: StateType,
    priority: Priority,
    blockers: Vec<Blocker>,
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

    /// What the ticket says, which `forman pull` hands every sub-task as the
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
    identifier: String,
    assignee: Option<String>,
    state_type: StateType,
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
pub struct StateType(String);

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

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Linear's `priority`, in the order work is taken rather than the order the
/// numbers come in.
///
/// The derived ordering is the whole point: ascending is urgent first and no
/// priority last, so sorting a queue by this sorts it the way the board reads.
/// Linear's own numbers — `0` for none, `1` for urgent — sort the other way
/// round, which is why nothing outside [`priority`] holds one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Priority {
    Urgent,
    High,
    Medium,
    Low,
    None,
}

/// Labels read on a named ticket in the page that carries it; more are read on.
const LABELS_PAGE: usize = 50;

/// One ticket a person named, by the two facts an identifier is made of.
///
/// Not by the identifier itself, because Linear's `IssueFilter` has no
/// `identifier`: the identifier is a display name made of the team's key and the
/// issue's number, and those two are what can be filtered on. Splitting it is the
/// caller's, which is also where a string that is no identifier at all gets its
/// own words rather than a request.
///
/// The node selection is [`scope_queue`]'s — the ticket comes back as the same
/// [`QueuedIssue`], parsed by the same function, so a named ticket and a chosen
/// one are the same value and the rules cannot drift — plus the three facts the
/// queue's filters stood for. Which is the whole point of reading it this way:
/// the filters are what make a queue, and an issue *missing* from a filtered
/// queue cannot say which filter dropped it. Here the three arrive as facts, and
/// the caller's check on them can name the one that failed.
///
/// The state filter is left off for the same reason, and one more: a ticket that
/// shipped last week is absent from the queue too, and a refusal calling that
/// "not on your team" would be a lie about the board.
///
/// `None` when the workspace has no such issue — a team key nobody uses, or a
/// number that team has not reached. Not an error, for [`team_id`]'s reason.
fn named_issue(linear: &impl Posts, team: &str, number: u64) -> Result<Option<NamedIssue>, Error> {
    let document = format!(
        r"query NamedIssue($team: String!, $number: Float!, $labels: Int!, $blockers: Int!) {{
            issues(
                filter: {{ team: {{ key: {{ eq: $team }} }}, number: {{ eq: $number }} }}
                first: 1
            ) {{
                nodes {{
                    {QUEUED_FIELDS}
                    team {{ key }}
                    labels(first: $labels) {{
                        pageInfo {{ hasNextPage endCursor }}
                        nodes {{ name }}
                    }}
                    assignee {{ id name }}
                }}
            }}
        }}"
    );
    let data = linear.post(
        &document,
        json!({
            // Upper cased because Linear's team keys are, and `WAR-133` is
            // something a person types: `war-133` names the same ticket to
            // everyone except an `eq` on the key.
            "team": team.trim().to_uppercase(),
            "number": number,
            "labels": LABELS_PAGE,
            "blockers": BLOCKERS_PAGE,
        }),
    )?;

    let Some(node) = nodes(&data, "issues")?.first() else {
        return Ok(None);
    };
    let team = node.get("team").ok_or_else(|| missing("team"))?;
    let issue = queued_issue(linear, node)?;

    Ok(Some(NamedIssue {
        issue,
        team: text(team, "key")?,
        labels: label_names(linear, node)?,
        assignee: nullable(node, "assignee", |assignee| {
            Ok(Assignee {
                id: node_id(assignee)?,
                name: text(assignee, "name")?,
            })
        })?,
    }))
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
    issue: QueuedIssue,
    team: String,
    labels: Vec<String>,
    assignee: Option<Assignee>,
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
/// what [`Board::viewer`] answers and the only honest way to ask whether a ticket
/// is yours — two people in a workspace can share a display name. The name is the
/// half a person reads in a refusal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Assignee {
    id: String,
    name: String,
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

/// The id of the label by that name, creating it when the workspace has none.
///
/// Two requests at most, one per thing asked, and the create only ever runs
/// against an empty answer — so a second push finds the label the first one made
/// rather than adding another of the same name.
///
/// A project label is its own type in Linear: `issueLabels` and
/// `issueLabelCreate` are a different set of labels, and an id from there is not
/// one a project can carry.
fn label_id(linear: &impl Posts, name: &str) -> Result<String, Error> {
    let data = linear.post(
        "query ProjectLabel($name: String!) {
            projectLabels(filter: { name: { eq: $name } }, first: 1) { nodes { id } }
        }",
        json!({ "name": name }),
    )?;

    if let Some(existing) = nodes(&data, "projectLabels")?.first() {
        return node_id(existing);
    }

    let created = linear.post(
        "mutation ProjectLabelCreate($input: ProjectLabelCreateInput!) {
            projectLabelCreate(input: $input) { projectLabel { id } }
        }",
        json!({ "input": { "name": name } }),
    )?;

    node_id(payload(&created, "projectLabelCreate", "projectLabel")?)
}

/// The id of the issue label by that name on the team, creating it there when
/// the team has none.
///
/// Issue labels and project labels are different types in Linear's schema,
/// reached by different queries: `issueLabels`/`issueLabelCreate` here,
/// `projectLabels`/`projectLabelCreate` in [`label_id`]. An id from one is not
/// usable by the other — a project cannot carry an issue label and an issue
/// cannot carry a project label — so the two resolvers stay separate and neither
/// answer may be handed to the other's create, however alike the two names look
/// at the call site.
///
/// Two requests at most, one per thing asked, and the create only ever runs
/// against an empty answer — so a second cut finds the label the first one made
/// rather than adding another of the same name.
fn issue_label_id(linear: &impl Posts, name: &str, team: &str) -> Result<String, Error> {
    let data = linear.post(
        "query IssueLabel($name: String!, $team: ID!) {
            issueLabels(
                filter: { name: { eq: $name }, team: { id: { eq: $team } } }
                first: 1
            ) { nodes { id } }
        }",
        json!({ "name": name, "team": team }),
    )?;

    if let Some(existing) = nodes(&data, "issueLabels")?.first() {
        return node_id(existing);
    }

    let created = linear.post(
        "mutation IssueLabelCreate($input: IssueLabelCreateInput!) {
            issueLabelCreate(input: $input) { issueLabel { id } }
        }",
        json!({ "input": { "name": name, "teamId": team } }),
    )?;

    node_id(payload(&created, "issueLabelCreate", "issueLabel")?)
}

/// Create the project, with its label resolved first.
///
/// The order is the point and not an implementation detail: the label is the
/// only mark on a project saying warlock filed it, nothing here can take a
/// project back, and a create that landed before a label that then failed is a
/// project no cut will ever read. Resolving first means a project that exists
/// is a project that carries the label.
fn create_project(linear: &impl Posts, project: &NewProject<'_>) -> Result<Project, Error> {
    let label = label_id(linear, project.label)?;

    let mut input = json!({
        "name": project.name,
        "content": project.content,
        "teamIds": [project.team],
        "labelIds": [label],
    });

    // Absent rather than `null`: a status the workspace does not have is a
    // project filed with no status, and the field is left out entirely to say
    // that.
    if let Some(status) = project.status
        && let Some(fields) = input.as_object_mut()
    {
        fields.insert("statusId".to_owned(), json!(status));
    }

    let data = linear.post(
        "mutation ProjectCreate($input: ProjectCreateInput!) {
            projectCreate(input: $input) { project { id url } }
        }",
        json!({ "input": input }),
    )?;
    let created = payload(&data, "projectCreate", "project")?;

    Ok(Project {
        id: node_id(created)?,
        url: text(created, "url")?,
    })
}

/// What [`Board::create_project`] is asked for: the label is the name it goes by
/// in the workspace rather than an id, because the create path is what resolves
/// it.
#[derive(Debug, Clone, Copy)]
pub(crate) struct NewProject<'a> {
    name: &'a str,
    content: &'a str,
    team: &'a str,
    status: Option<&'a str>,
    label: &'a str,
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
    id: String,
    url: String,
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

/// Create one issue, in the team, project, label and workflow state the caller
/// already resolved.
///
/// Nothing is resolved here: an issue create that had to look up its own state
/// would be a second request per draft, and the team that has no `Backlog` has
/// to be refused before any issue exists rather than once a slice is half filed.
fn create_issue(linear: &impl Posts, issue: &NewIssue<'_>) -> Result<Issue, Error> {
    let data = linear.post(
        "mutation IssueCreate($input: IssueCreateInput!) {
            issueCreate(input: $input) { issue { id identifier url } }
        }",
        json!({
            "input": {
                "title": issue.title,
                "description": issue.body,
                "teamId": issue.team,
                "projectId": issue.project,
                "labelIds": [issue.label],
                "stateId": issue.state,
                "assigneeId": issue.assignee,
            },
        }),
    )?;
    let created = payload(&data, "issueCreate", "issue")?;

    Ok(Issue {
        id: node_id(created)?,
        identifier: text(created, "identifier")?,
        url: text(created, "url")?,
    })
}

/// What [`Board::create_issue`] is asked for, and the whole of it: every field
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
/// `label` an *issue label* id from [`Board::issue_label_id`]; neither a project
/// status nor a project label is usable here.
#[derive(Debug, Clone, Copy)]
pub(crate) struct NewIssue<'a> {
    title: &'a str,
    body: &'a str,
    team: &'a str,
    project: &'a str,
    label: &'a str,
    state: &'a str,
    assignee: &'a str,
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
    id: String,
    identifier: String,
    url: String,
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

/// Write the edge saying `blocker` blocks `waiting`, by issue id.
fn create_relation(linear: &impl Posts, blocker: &str, waiting: &str) -> Result<String, Error> {
    let data = linear.post(
        "mutation IssueRelationCreate($input: IssueRelationCreateInput!) {
            issueRelationCreate(input: $input) { issueRelation { id } }
        }",
        // The direction lives entirely in which id goes in which field: with
        // type `blocks`, `issueId` is the issue that blocks and
        // `relatedIssueId` the one held up. Swapping the two is a change that
        // compiles, passes anything not asserting the input, and inverts every
        // dependency in the slice.
        json!({
            "input": {
                "issueId": blocker,
                "relatedIssueId": waiting,
                "type": "blocks",
            },
        }),
    )?;

    node_id(payload(&data, "issueRelationCreate", "issueRelation")?)
}

/// Move one issue into a workflow state, by the two ids the caller already
/// resolved.
///
/// The input carries `stateId` and nothing else, which is the whole of what this
/// is allowed to write: an assignee is a human's decision about whose work a
/// ticket is, and a project's status is a different type on a different object
/// that no issue move has any business touching. A state named `completed` or
/// `canceled` is not refused here — a team names its columns what it likes and
/// this module cannot tell which id is which without a second request — it is
/// refused by the caller, which knows the name it asked [`workflow_state`] for.
///
/// One request and no retry, per this module's rule. A move that failed leaves a
/// ticket in the column it was in, which is a line to print rather than a run to
/// abandon: the board is a courtesy and the work is the point.
fn move_issue(linear: &impl Posts, issue: &str, state: &str) -> Result<String, Error> {
    let data = linear.post(
        "mutation IssueUpdate($id: String!, $input: IssueUpdateInput!) {
            issueUpdate(id: $id, input: $input) { issue { id } }
        }",
        json!({ "id": issue, "input": { "stateId": state } }),
    )?;

    node_id(payload(&data, "issueUpdate", "issue")?)
}

/// Comment on a project, by id.
///
/// Linear has one comment mutation for issues and projects both, told apart by
/// which id the input carries — so a `projectId` here is the whole of what makes
/// this a project comment.
fn comment_on_project(linear: &impl Posts, project: &str, body: &str) -> Result<String, Error> {
    comment(linear, &json!({ "projectId": project, "body": body }))
}

/// Comment on an issue, by id.
///
/// The same mutation [`comment_on_project`] sends, and `issueId` in place of
/// `projectId` is the entire difference: the two functions exist so that which
/// object gets commented on is decided by the name a caller types rather than by
/// the key it spells in a JSON literal.
///
/// One request and no retry, per this module's rule, and here that rule has teeth:
/// a create is not idempotent, so a retried comment is how a halt gets explained
/// twice on the same ticket. A comment that did not land is a line to print.
fn comment_on_issue(linear: &impl Posts, issue: &str, body: &str) -> Result<String, Error> {
    comment(linear, &json!({ "issueId": issue, "body": body }))
}

fn comment(linear: &impl Posts, input: &Value) -> Result<String, Error> {
    let data = linear.post(
        "mutation CommentCreate($input: CommentCreateInput!) {
            commentCreate(input: $input) { comment { id } }
        }",
        json!({ "input": input }),
    )?;

    node_id(payload(&data, "commentCreate", "comment")?)
}

/// The key as the whole of the `Authorization` value, with no `Bearer ` prefix:
/// Linear takes a personal API key bare there, and the prefix an OAuth token
/// wants is a 401 for a key. This is the only place the key is read.
fn authorization(key: &str) -> Result<HeaderValue, Error> {
    let mut value = HeaderValue::from_str(key).map_err(|_| Error::Key)?;
    // So that anything which prints a header map — ureq's own logging, a
    // middleware added later — prints `Sensitive` where the key is.
    value.set_sensitive(true);
    Ok(value)
}

fn failure(error: ureq::Error) -> Error {
    match error {
        // ureq turns 4xx and 5xx into this by default. A 401 is the key and a
        // 400 is the document; neither is an I/O failure and neither should
        // read like one.
        ureq::Error::StatusCode(code) => Error::Status { code },
        ureq::Error::Json(_) => Error::Malformed {
            detail: "it was not JSON".to_owned(),
        },
        source => Error::Transport { source },
    }
}

/// GraphQL answers a request it understood and turned down with a 200 and an
/// `errors` array, so the envelope decides whether the call worked and the
/// status does not.
fn answer(mut body: Value) -> Result<Value, Error> {
    if let Some(message) = refusal(&body) {
        return Err(Error::Refused { message });
    }

    body.get_mut("data")
        .map(Value::take)
        .filter(|data| !data.is_null())
        .ok_or_else(|| Error::Malformed {
            detail: "it carried no `data`".to_owned(),
        })
}

// `message` stays first and whole, because `unknown_entity` matches on it. For
// a validation failure it is only "Argument Validation Error", and the reason a
// person can act on is in `userPresentableMessage`.
fn refusal(body: &Value) -> Option<String> {
    let first = body.get("errors")?.as_array()?.first()?;

    let message = first
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or("no reason given");
    let presentable = first
        .pointer("/extensions/userPresentableMessage")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|presentable| !presentable.is_empty() && *presentable != message);

    Some(match presentable {
        Some(presentable) => format!("{message}: {presentable}"),
        None => message.to_owned(),
    })
}

fn nodes<'a>(data: &'a Value, connection: &str) -> Result<&'a [Value], Error> {
    data.get(connection)
        .and_then(|connection| connection.get("nodes"))
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .ok_or_else(|| Error::Malformed {
            detail: format!("`{connection}` carried no nodes"),
        })
}

fn payload<'a>(data: &'a Value, mutation: &str, created: &str) -> Result<&'a Value, Error> {
    data.get(mutation)
        .and_then(|payload| payload.get(created))
        .filter(|created| !created.is_null())
        .ok_or_else(|| Error::Malformed {
            detail: format!("`{mutation}` carried no `{created}`"),
        })
}

fn node_id(node: &Value) -> Result<String, Error> {
    text(node, "id")
}

/// A field that was asked for and may be answered `null`. Missing from the
/// answer altogether is still malformed: a document that named the field and an
/// answer that does not carry it are not the same call.
fn nullable<T>(
    node: &Value,
    field: &str,
    read: impl FnOnce(&Value) -> Result<T, Error>,
) -> Result<Option<T>, Error> {
    match node.get(field) {
        Some(Value::Null) => Ok(None),
        Some(value) => read(value).map(Some),
        None => Err(missing(field)),
    }
}

/// Every label on an issue, by name. An issue with none is an empty list and not
/// an absence: no labels is a perfectly ordinary issue, and it is a refusal for
/// the caller rather than a malformed answer.
fn label_names(linear: &impl Posts, issue: &Value) -> Result<Vec<String>, Error> {
    completed(
        linear,
        issue.get("labels").ok_or_else(|| missing("labels"))?,
        "query IssueLabels($id: String!, $after: String) {
            issue(id: $id) {
                labels(first: 250, after: $after) {
                    pageInfo { hasNextPage endCursor }
                    nodes { name }
                }
            }
        }",
        json!({ "id": node_id(issue)? }),
        &["issue", "labels"],
    )?
    .iter()
    .map(|label| text(label, "name"))
    .collect()
}

fn unknown_entity(message: &str) -> bool {
    message.to_lowercase().contains("entity not found")
}

fn text(node: &Value, field: &str) -> Result<String, Error> {
    node.get(field)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| missing(field))
}

fn missing(field: &str) -> Error {
    Error::Malformed {
        detail: format!("a node carried no `{field}`"),
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
    Transport {
        source: ureq::Error,
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
            Self::Transport { source } => Some(source),
            Self::Key
            | Self::Stopped
            | Self::Status { .. }
            | Self::Malformed { .. }
            | Self::Refused { .. } => None,
        }
    }
}

#[cfg(test)]
#[path = "tests/linear.rs"]
mod tests;
