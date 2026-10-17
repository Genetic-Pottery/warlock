//! The second module in this library that opens a socket, on the same `ureq`
//! and under `linear.rs`'s rules unchanged: no retry, no backoff and no async
//! runtime. A call that failed is reported and the person decides, because
//! nothing on this side can tell a timeout that arrived before a create ran
//! from one that arrived after.
//!
//! Jira Cloud authenticates an API token with HTTP basic auth, where the
//! username is the account's email: a `Bearer` value there is for an OAuth
//! token and is a 401 for a token. The key store holds one opaque line and
//! judges nothing, so the pair arrives here as [`EMAIL_TOKEN`] and is split at
//! the first colon.

use std::fmt;
use std::time::Duration;

use serde_json::Value;
use ureq::Agent;
use ureq::http::header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE};
use ureq::http::{HeaderValue, Method, Request};

use crate::board::Error;
use crate::linear::REQUEST_TIMEOUT;

/// The one place this vendor is spelled for a reader: every [`Error`] raised
/// here, and `Board::NAME` for the board that answers over this transport.
const JIRA: &str = "Jira";

/// How a Jira key is typed, and the whole of what a refusal of a stored line
/// says about it.
const EMAIL_TOKEN: &str = "email:token";

/// Standard base64, as [RFC 4648] spells it: `+` and `/` for the last two
/// sextets and `=` to pad, which is the alphabet `Authorization: Basic` wants.
///
/// [RFC 4648]: https://datatracker.ietf.org/doc/html/rfc4648#section-4
const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// One REST round trip: a method, a path under the site the client holds, a JSON
/// body, and the answer's JSON.
///
/// The transport under the Jira board, and the seam this module's own tests
/// drive with an in-memory stand-in to check how each operation is spelled.
/// Nothing outside this module is written against it: the flows take a
/// [`Board`](crate::board::Board).
///
/// A method and a path rather than one endpoint and a document, which is the
/// whole difference from [`Posts`](crate::linear::Posts): Jira has a URL per
/// resource and reads a GET, a POST and a PUT as three different verbs on it.
///
/// ```no_run
/// use serde_json::Value;
/// use ureq::http::Method;
/// use warlock_tui::{Calls, JiraClient};
///
/// // Reaches a real Jira site, so this example is not executed by the test suite.
/// let client = JiraClient::new("https://example.atlassian.net", "me@example.com:a-token");
///
/// println!("{}", client.call(Method::GET, "/rest/api/3/myself", Value::Null)?);
/// # Ok::<(), warlock_tui::BoardError>(())
/// ```
pub trait Calls {
    fn call(&self, method: Method, path: &str, body: Value) -> Result<Value, Error>;
}

/// A client for one Jira site, holding the key it authenticates with.
///
/// The site and the key are constructor parameters, as Linear's key is: nothing
/// in this module reads an environment variable, a file or the key store, so no
/// test in this crate can reach the developer's real credentials by accident.
/// The site is a record's `base_url` as recorded, read off the record by
/// whoever builds this.
///
/// ```
/// use warlock_tui::{JiraClient, REQUEST_TIMEOUT};
///
/// let client = JiraClient::new("https://example.atlassian.net", "me@example.com:a-token");
///
/// assert_eq!(client.timeout(), Some(REQUEST_TIMEOUT));
/// // The key is held and never shown.
/// assert!(!format!("{client:?}").contains("a-token"));
/// ```
pub struct Client {
    base_url: String,
    key: String,
    agent: Agent,
}

impl Client {
    #[must_use]
    pub fn new(base_url: impl Into<String>, key: impl Into<String>) -> Self {
        let config = Agent::config_builder()
            // Global rather than any of the per-phase timeouts, and the same
            // one Linear's calls run under: what a caller waits on is the whole
            // call, and a resolve, a connect and a body read with thirty
            // seconds each is a minute and a half.
            .timeout_global(Some(REQUEST_TIMEOUT))
            // Off so a 4xx keeps its body: Jira says why it refused in
            // `errorMessages` and `errors`, and ureq's own error for a status
            // carries the code alone. See `answer`.
            .http_status_as_error(false)
            .build();

        Self {
            base_url: base_url.into(),
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

impl Calls for Client {
    fn call(&self, method: Method, path: &str, body: Value) -> Result<Value, Error> {
        // Read first, so a stored line that is not a pair is refused with no URL
        // built and nothing sent.
        let authorization = authorization(&self.key)?;
        // A read takes `Value::Null` and sends no body: a four-byte `null` on a
        // GET is a body nothing asked for.
        let body = if body.is_null() {
            String::new()
        } else {
            body.to_string()
        };

        let request = Request::builder()
            .method(method)
            .uri(url(&self.base_url, path))
            .header(AUTHORIZATION, authorization)
            .header(ACCEPT, "application/json")
            .header(CONTENT_TYPE, "application/json")
            .body(body)
            .map_err(unreachable)?;

        let mut response = self.agent.run(request).map_err(failure)?;
        let status = response.status().as_u16();
        let text = response.body_mut().read_to_string().map_err(failure)?;

        answer(status, &text)
    }
}

/// The answer to one call, from its status and its body as text.
///
/// A success with an empty body is `Value::Null`: a transition answers 204 and
/// an issue link 201, both with nothing after the headers, and reading either
/// as JSON would fail a call that worked. A refusal is Jira's own words when
/// the body carries any, and the bare status when it carries none.
fn answer(status: u16, text: &str) -> Result<Value, Error> {
    if (200..300).contains(&status) {
        if text.trim().is_empty() {
            return Ok(Value::Null);
        }
        return serde_json::from_str(text).map_err(|_| Error::Malformed {
            board: JIRA,
            detail: "it was not JSON".to_owned(),
        });
    }

    match serde_json::from_str::<Value>(text)
        .ok()
        .and_then(|body| refusal(&body))
    {
        Some(message) => Err(Error::Refused {
            board: JIRA,
            message,
        }),
        None => Err(Error::Status {
            board: JIRA,
            code: status,
        }),
    }
}

/// Jira's reasons for a refusal, joined: every `errorMessages` entry, then each
/// `errors` field as `field: reason`. `None` when the body names none, so a
/// proxy's error page reads as a status and never as Jira's words.
fn refusal(body: &Value) -> Option<String> {
    let messages = body["errorMessages"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_owned);
    let fields = body["errors"]
        .as_object()
        .into_iter()
        .flatten()
        .filter_map(|(field, reason)| Some(format!("{field}: {}", reason.as_str()?)));
    let reasons: Vec<String> = messages
        .chain(fields)
        .filter(|reason| !reason.trim().is_empty())
        .collect();

    (!reasons.is_empty()).then(|| reasons.join("; "))
}

/// The site joined to one path.
// A recorded `base_url` may end in a slash, because a record keeps whatever was
// typed and nothing normalises it: the join is where the two spellings meet, so
// it trims rather than the record being rewritten anywhere.
fn url(base_url: &str, path: &str) -> String {
    format!(
        "{}/{}",
        base_url.trim_end_matches('/'),
        path.trim_start_matches('/')
    )
}

/// `Basic <base64(email:token)>`, and the only place the key is read.
fn authorization(stored: &str) -> Result<HeaderValue, Error> {
    let (email, token) = pair(stored)?;
    let credentials = base64(format!("{email}:{token}").as_bytes());

    // The refusal is out of reach through `base64`, whose output is ASCII
    // whatever the stored line's bytes were. A refusal rather than an `expect`
    // all the same, because a panic here is a panic on a value off a disk.
    let mut value = HeaderValue::from_str(&format!("Basic {credentials}"))
        .map_err(|_| Error::Key { board: JIRA })?;
    // So that anything which prints a header map — ureq's own logging, a
    // middleware added later — prints `Sensitive` where the key is.
    value.set_sensitive(true);
    Ok(value)
}

/// The stored line as its two halves, split at the first colon so a token
/// carrying one keeps it.
///
/// The one check warlock makes of a key value, and it is here rather than where
/// the client is built because [`Opens::open`](crate::board::Opens::open)
/// cannot fail. The store judges nothing on the way in.
fn pair(stored: &str) -> Result<(&str, &str), Error> {
    match stored.split_once(':') {
        Some((email, token)) if !email.is_empty() && !token.is_empty() => Ok((email, token)),
        _ => Err(Error::Mistyped {
            board: JIRA,
            expected: EMAIL_TOKEN,
        }),
    }
}

fn failure(error: ureq::Error) -> Error {
    match error {
        // Out of reach with `http_status_as_error` off, and kept so a config
        // change that turns it back on still reads a 401 as a status rather
        // than as an I/O failure.
        ureq::Error::StatusCode(code) => Error::Status { board: JIRA, code },
        source => Error::Transport {
            board: JIRA,
            source: Box::new(source),
        },
    }
}

/// Everything `http` refuses a request for: a method, a URI and two header
/// values it already accepted, so what is left is a `base_url` no URI can be
/// made of.
// Boxed as a transport failure because that is what it is to a reader — the
// site recorded for the scope cannot be reached — and because this type carries
// no variant for a request that was never built.
fn unreachable(error: ureq::http::Error) -> Error {
    Error::Transport {
        board: JIRA,
        source: Box::new(error),
    }
}

/// Base64, padded.
// Hand-written because the root `Cargo.toml` asks every dependency to justify
// its supply chain and twenty lines of encoder cannot: this is the only base64
// in warlock, and there is no decoder because nothing here reads one back.
fn base64(bytes: &[u8]) -> String {
    let mut encoded = String::with_capacity(bytes.len().div_ceil(3) * 4);

    for chunk in bytes.chunks(3) {
        let mut bits = 0_usize;
        for (index, byte) in chunk.iter().enumerate() {
            bits |= usize::from(*byte) << (16 - 8 * index);
        }

        // One sextet more than the chunk has bytes, which is where the 3n+1 and
        // 3n+2 tails get their two and three characters, and the `=` below
        // makes the quantum four either way.
        for index in 0..=chunk.len() {
            encoded.push(char::from(ALPHABET[(bits >> (18 - 6 * index)) & 0b11_1111]));
        }
        for _ in chunk.len()..3 {
            encoded.push('=');
        }
    }

    encoded
}

#[cfg(test)]
#[path = "tests/jira.rs"]
mod tests;
