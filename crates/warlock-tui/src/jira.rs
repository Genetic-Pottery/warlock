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

use serde_json::{Value, json};
use ureq::Agent;
use ureq::http::header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE};
use ureq::http::{HeaderValue, Method, Request};
use warlock_engine::{FiledTo, ScopeRecord};

use crate::adf::document_for;
use crate::board::{
    Board, Error, FetchedBrief, Filable, FiledBrief, FiledTicket, Listing, NamedTicket, NewBrief,
    NewTicket, Queue, TicketBrief,
};
use crate::linear::REQUEST_TIMEOUT;
use crate::queue::same_name;

/// The one place this vendor is spelled for a reader: every [`Error`] raised
/// here, and `Board::NAME` for the board that answers over this transport.
const JIRA: &str = "Jira";

/// Where a JQL read goes. A POST and not a GET, because [`Calls`] carries a
/// body and a query string would have to be percent-encoded into the path by
/// hand; `/rest/api/2/search` answers 410 Gone and is not an option.
const SEARCH: &str = "/rest/api/3/search/jql";

/// Where one issue is created. The collection and not an issue's own URL: a
/// create is a POST to it, and nothing here addresses an issue by id.
const ISSUE: &str = "/rest/api/3/issue";

/// Jira's own name for the issue type a brief is filed as, read both by the
/// create that writes `issuetype` and by the lookup's JQL. A site that has
/// renamed it refuses both in its own words rather than warlock guessing a
/// second spelling.
const EPIC: &str = "Epic";

/// Issues read per page of a JQL search. Not left off: an answer with no
/// `maxResults` takes whatever default the site has today.
const SEARCH_PAGE: usize = 100;

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

/// The one [`Board`] that speaks REST, over whatever [`Calls`] it holds.
///
/// The record it was opened for is held beside the transport, as
/// [`Linear`](crate::linear::Linear) holds its own, and it is the whole of what
/// this board knows about where work goes: the site and the project key a
/// request is addressed by are read off it by [`site`](Self::site), and the
/// label is there for whatever create reads it, so no verb below [`Board`] is
/// handed any of them. Owned rather than borrowed, because a board is opened on
/// a panel worker and outlives the manifest read that produced the record.
///
/// Every verb but the two a push makes — the brief-by-name lookup and the brief
/// create — refuses as this stands: the requests themselves arrive a slice at a
/// time, and a board that refuses is what keeps a flow which reaches a verb
/// nobody has written yet a sentence rather than a panic.
#[derive(Debug)]
pub(crate) struct Jira<C = Client> {
    calls: C,
    record: ScopeRecord,
}

impl<C: Calls> Jira<C> {
    #[must_use]
    pub(crate) const fn new(calls: C, record: ScopeRecord) -> Self {
        Self { calls, record }
    }

    /// The site and the project key this board files to, read off the record it
    /// was opened for: the two halves of a Jira destination, because a project
    /// key means nothing without the site it is a key on.
    ///
    /// Opening cannot fail, so a record that files somewhere else is refused
    /// here instead — reached only by a board built for a record filing to
    /// Linear, which [`Opener::open`](crate::board::Opener) does not build, and
    /// a `Result` rather than a panic because this is a request that was not
    /// made.
    fn site(&self) -> Result<(&str, &str), Error> {
        match self.record.filed_to() {
            FiledTo::Jira {
                base_url,
                project_key,
            } => Ok((base_url, project_key)),
            FiledTo::Linear => Err(Error::Elsewhere {
                board: JIRA,
                scope: self.record.name().to_owned(),
            }),
        }
    }

    /// Every issue one JQL read answers, page after page, with the `fields`
    /// asked for on each of them.
    ///
    /// Jira's paging is the token an answer hands back going into the next
    /// request, where `linear.rs`'s own `paged` reads a cursor off a `pageInfo`
    /// beside a `hasNextPage` flag: the last page here is the one that carries
    /// no token, and there is no flag beside it to read.
    ///
    /// One request per page, and a page that failed fails the whole read rather
    /// than answering the pages that arrived before it: what a caller asks of
    /// these issues is whether one of them is there, and a page that never
    /// arrived could have been carrying it.
    fn searched(&self, jql: &str, fields: &[&str]) -> Result<Vec<Value>, Error> {
        let mut body = json!({ "jql": jql, "fields": fields, "maxResults": SEARCH_PAGE });
        let mut found = Vec::new();

        loop {
            let answer = self.calls.call(Method::POST, SEARCH, body.clone())?;
            let issues = answer
                .get("issues")
                .and_then(Value::as_array)
                .ok_or_else(|| missing("issues"))?;

            found.extend(issues.iter().cloned());

            let Some(token) = answer["nextPageToken"].as_str().map(str::to_owned) else {
                return Ok(found);
            };

            if let Some(sent) = body.as_object_mut() {
                sent.insert("nextPageToken".to_owned(), json!(token));
            }
        }
    }

    /// The refusal of a verb no slice has written for this board yet, with
    /// nothing sent through [`Calls`].
    ///
    /// The record is read first, so a board opened for a scope that files
    /// somewhere else is told that instead: the two are different things to fix
    /// — a manifest naming the wrong board, against a warlock that has not
    /// learnt this verb — and the one about the record holds whichever verb was
    /// reached.
    fn untaught<T>(&self, verb: &'static str) -> Result<T, Error> {
        self.site()?;
        Err(Error::Untaught { board: JIRA, verb })
    }
}

impl<C: Calls> Board for Jira<C> {
    const NAME: &'static str = JIRA;

    /// Nothing, because the create it would be settled for refuses as well: a
    /// cut that cannot file a ticket has nothing to settle first.
    type Filing = ();

    fn viewer(&self) -> Result<String, Error> {
        self.untaught("read whose key a scope files with")
    }

    fn move_brief(&self, _project: &str, _status: &str) -> Result<bool, Error> {
        self.untaught("move a brief")
    }

    fn ticket_brief(&self, _issue: &str) -> Result<Option<TicketBrief>, Error> {
        self.untaught("read the brief a ticket belongs to")
    }

    fn move_ticket(&self, _issue: &str, _status: &str) -> Result<bool, Error> {
        self.untaught("move a ticket")
    }

    fn fetch_brief(&self, _slug: &str) -> Result<Option<FetchedBrief>, Error> {
        self.untaught("read a brief")
    }

    fn planned_briefs(&self) -> Result<Listing, Error> {
        self.untaught("list the planned briefs")
    }

    /// The URL of the Epic in this project with this name, or `None` when the
    /// project holds none: the only thing a push can tell a second push of the
    /// same brief by, now that nothing on the machine remembers the first.
    ///
    /// `~` is a fuzzy text match and the only operator JQL has for a summary, so
    /// the search narrows and the comparison decides: a quoted phrase inside the
    /// quoted value matches the words in order, and `A brief about pay` still
    /// comes back for `A brief`. Every page is read and each summary is then
    /// compared as [`same_name`] compares, which is how a brief whose name
    /// differs in case or in spacing is the same brief — the same comparison
    /// `project_named` makes on Linear, where the filter did the matching.
    ///
    /// A name carrying a double quote is a JQL string Jira refuses in its own
    /// words. It is not escaped here because a brief's name is a heading in a
    /// document somebody wrote, and a refusal naming the search is a better
    /// answer than a lookup that quietly matched something else.
    fn brief_named(&self, name: &str) -> Result<Option<String>, Error> {
        let (base_url, project_key) = self.site()?;
        // The [`EPIC`] the create writes as well, and the escaped quotes are a
        // quoted phrase inside the quoted value `~` takes.
        let jql =
            format!(r#"project = {project_key} AND issuetype = {EPIC} AND summary ~ "\"{name}\"""#);

        for issue in self.searched(&jql, &["summary"])? {
            let summary = issue
                .get("fields")
                .and_then(|fields| fields.get("summary"))
                .and_then(Value::as_str)
                .ok_or_else(|| missing("summary"))?;

            if same_name(summary, name) {
                let key = issue
                    .get("key")
                    .and_then(Value::as_str)
                    .ok_or_else(|| missing("key"))?;

                return Ok(Some(browse(base_url, key)));
            }
        }

        Ok(None)
    }

    fn scope_queue(&self, _assignee: &str) -> Result<Queue, Error> {
        self.untaught("read a scope's queue")
    }

    fn named_ticket(&self, _number: u64) -> Result<Option<NamedTicket>, Error> {
        self.untaught("read a ticket by number")
    }

    /// File the brief as an Epic in the project the record names, carrying the
    /// label and its body as an ADF document, and answer where a person opens
    /// it.
    ///
    /// One request, and the label is in it: it is the only mark on an Epic
    /// saying warlock filed it, nothing here can take an Epic back, and a
    /// create that landed before a label that then failed would be a brief no
    /// cut ever reads. Jira writes a label by its own name on the issue, so
    /// there is nothing to resolve first — where `create_project` on Linear
    /// resolves an id before it creates anything, this spends no read at all.
    /// It is the label [`NewBrief`] carries, which is the one the record
    /// records: a push reads it off the scope and every create on this seam is
    /// handed it the same way.
    ///
    /// No status field and no transition after it: the Epic opens in whatever
    /// status the project's workflow starts an issue in, which is the site's
    /// decision and not warlock's.
    ///
    /// Never `None`, which on this seam is "the board has nowhere to file it" —
    /// a Linear workspace with no team by the recorded key. A Jira project key
    /// the site does not hold is something the create itself refuses in Jira's
    /// own words, and telling the two apart would cost a read before a
    /// mutation that is one request.
    fn create_brief(&self, project: &NewBrief<'_>) -> Result<Option<FiledBrief>, Error> {
        let (base_url, project_key) = self.site()?;
        let created = self.calls.call(
            Method::POST,
            ISSUE,
            json!({
                "fields": {
                    "project": { "key": project_key },
                    "issuetype": { "name": EPIC },
                    "summary": project.name,
                    "labels": [project.label],
                    "description": document_for(project.content),
                }
            }),
        )?;

        // The key and not the numeric `id` beside it: every path this module
        // addresses an issue by takes either, and the key is the one a person
        // reads in a refusal and types into a search.
        let key = created
            .get("key")
            .and_then(Value::as_str)
            .ok_or_else(|| missing("key"))?;

        Ok(Some(FiledBrief {
            id: key.to_owned(),
            url: browse(base_url, key),
        }))
    }

    fn ticket_filing(&self) -> Result<Filable<()>, Error> {
        self.untaught("settle where a cut's tickets file")
    }

    fn create_ticket(&self, _issue: &NewTicket<'_>, _filing: &()) -> Result<FiledTicket, Error> {
        self.untaught("file a ticket")
    }

    fn create_relation(&self, _blocker: &str, _waiting: &str) -> Result<String, Error> {
        self.untaught("record that one ticket blocks another")
    }

    fn comment_on_brief(&self, _project: &str, _body: &str) -> Result<String, Error> {
        self.untaught("comment on a brief")
    }

    fn comment_on_ticket(&self, _issue: &str, _body: &str) -> Result<String, Error> {
        self.untaught("comment on a ticket")
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

/// Where a person opens one issue, from the site and the issue's key.
///
/// The one address this board answers with, for an issue it filed and for one
/// it found: `/browse/<key>` is the path Jira itself redirects to whatever view
/// the issue belongs to, so no project id, no board and no issue id is needed to
/// build it.
fn browse(base_url: &str, key: &str) -> String {
    url(base_url, &format!("/browse/{key}"))
}

/// Warlock's own words for an answer that left out a field this module reads,
/// which is all a reader is told about it: the bytes that arrived are whatever a
/// captive portal or a proxy decided to send.
fn missing(field: &str) -> Error {
    Error::Malformed {
        board: JIRA,
        detail: format!("it carried no `{field}`"),
    }
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
