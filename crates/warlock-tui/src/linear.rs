//! The one module in this library that opens a socket, and the reason it is
//! `ureq` rather than `reqwest`: reqwest's blocking client runs a tokio runtime
//! on a background thread, and a terminal program that links one has an
//! executor it never asked for. No async runtime here either, for the reason
//! `claude.rs` has none.
//!
//! One request per call, no retry and no backoff. A brief filed twice is two
//! projects on the board, and nothing on this side can tell a timeout that
//! arrived before the mutation ran from one that arrived after — so a failed
//! call is reported and the person decides, which is the only honest answer
//! available to a non-idempotent mutation.

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
/// The seam every operation is written against, which is what lets them be
/// driven by an in-memory stand-in and keeps every test in this crate off the
/// network. Unwrapping the envelope belongs here rather than to each caller: a
/// stand-in hands back the `data` a real answer would have carried, and says a
/// refusal by returning [`Error::Refused`].
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

/// The project status a brief is filed into. A workspace with no status by this
/// name takes the project with no status at all, rather than whichever one
/// happens to sort first.
const BACKLOG: &str = "Backlog";

/// A team key — `WAR` — as Linear's own team id, or `None` when the workspace
/// has no team by that key. Not an error: the caller holds the words about
/// `.warlock/pacts.toml` and this module does not.
pub fn team_id(linear: &impl Posts, key: &str) -> Result<Option<String>, Error> {
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
pub fn backlog_status(linear: &impl Posts) -> Result<Option<String>, Error> {
    // `projectStatuses` takes no filter, so the one request this operation is
    // allowed asks for Linear's largest page and the match happens here. A
    // workspace with more than 250 project statuses would need a second page;
    // paginating for that is a loop around a create path, which this module
    // does not have.
    let data = linear.post(
        "query ProjectStatuses { projectStatuses(first: 250) { nodes { id name } } }",
        json!({}),
    )?;

    nodes(&data, "projectStatuses")?
        .iter()
        .find(|status| status.get("name").and_then(Value::as_str) == Some(BACKLOG))
        .map(node_id)
        .transpose()
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
pub fn label_id(linear: &impl Posts, name: &str) -> Result<String, Error> {
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

/// Create the project, with its label resolved first.
///
/// The order is the point and not an implementation detail: the label is the
/// only mark on a project saying warlock filed it, nothing here can take a
/// project back, and a create that landed before a label that then failed is a
/// project no pull will ever read. Resolving first means a project that exists
/// is a project that carries the label.
pub fn create_project(linear: &impl Posts, project: &NewProject<'_>) -> Result<Project, Error> {
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

/// What [`create_project`] is asked for: the label is the name it goes by in the
/// workspace rather than an id, because the create path is what resolves it.
#[derive(Debug, Clone, Copy)]
pub struct NewProject<'a> {
    name: &'a str,
    content: &'a str,
    team: &'a str,
    status: Option<&'a str>,
    label: &'a str,
}

impl<'a> NewProject<'a> {
    #[must_use]
    pub const fn new(name: &'a str, content: &'a str, team: &'a str, label: &'a str) -> Self {
        Self {
            name,
            content,
            team,
            status: None,
            label,
        }
    }

    #[must_use]
    pub const fn with_status(mut self, status: Option<&'a str>) -> Self {
        self.status = status;
        self
    }
}

/// A project that now exists. The URL is the one thing a failure downstream must
/// never lose, so it comes back from the create rather than being built from the
/// id here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Project {
    id: String,
    url: String,
}

impl Project {
    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }

    #[must_use]
    pub fn url(&self) -> &str {
        &self.url
    }
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

fn refusal(body: &Value) -> Option<String> {
    let first = body.get("errors")?.as_array()?.first()?;

    Some(
        first
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("no reason given")
            .to_owned(),
    )
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

fn text(node: &Value, field: &str) -> Result<String, Error> {
    node.get(field)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| Error::Malformed {
            detail: format!("a node carried no `{field}`"),
        })
}

/// Nothing here carries the key, and two variants say why they carry what they
/// do instead.
#[derive(Debug)]
pub enum Error {
    /// The key cannot be a header value at all — a newline or a byte outside
    /// ASCII in whatever was stored. It is named and not quoted: this type is
    /// printed.
    Key,
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
            Self::Key | Self::Status { .. } | Self::Malformed { .. } | Self::Refused { .. } => None,
        }
    }
}

#[cfg(test)]
#[path = "tests/linear.rs"]
mod tests;
