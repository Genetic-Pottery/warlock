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
