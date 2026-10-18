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

use crate::adf::{body_for, document_for};
use crate::board::{
    Board, CUT_NOTE, Category, Error, FetchedBrief, Filable, FiledBrief, FiledTicket, Listing,
    NamedTicket, NewBrief, NewTicket, Queue, SKIP_NOTE, TicketBrief,
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

/// The issue collection: a create is a POST to it, and one issue and its
/// comments are a key under it. The same root for both, because an issue's own
/// URL is this path and its key and nothing else is needed to build one.
const ISSUE: &str = "/rest/api/3/issue";

/// The project collection, which one project's own statuses hang under by its
/// key: the only read in this module addressed to a project rather than to an
/// issue, and the one place the record's project key is asked about on its own.
const PROJECT: &str = "/rest/api/3/project";

/// Where the account a key authenticates as is read. The account the key
/// belongs to and not a user looked up by id or by email: whose key it is is
/// the one thing nothing on this side knows before the site says it, and
/// `/rest/api/3/user` would need the answer to ask the question.
const MYSELF: &str = "/rest/api/3/myself";

/// Jira's own name for the issue type a brief is filed as, read both by the
/// create that writes `issuetype` and by the lookup's JQL. A site that has
/// renamed it refuses both in its own words rather than warlock guessing a
/// second spelling.
const EPIC: &str = "Epic";

/// Jira's own name for the issue type a cut's children are filed as, written by
/// [`Board::create_ticket`] as [`EPIC`] is written by the brief's create.
///
/// Named because Jira requires `issuetype` on a create and takes no default
/// from the project, so there is no way to leave the decision to the site: this
/// is the type every project template is created with, and the one an Epic is
/// ordinarily the parent of. A site that has renamed it refuses the create in
/// its own words, which is the same answer [`EPIC`] gets and for the same
/// reason — warlock guessing a second spelling would file the slice somewhere
/// nobody asked for.
const TASK: &str = "Task";

/// Jira's own key for the status category an Epic is still open for drafting
/// in, written into the JQL a listing narrows by.
///
/// The key and not a status name: what a site calls the column — `To Do` on one
/// board and `Triage` on the next — is the site's word, and the category Jira
/// files that status under is the one vocabulary every board shares. It is the
/// word [`Category::of`] reads as [`Category::New`], which is the same
/// judgement [`standing`] makes of an Epic a draft has already fetched, so the
/// listing offers exactly the briefs the fetch after it calls draftable.
const PLANNED: &str = "new";

/// Issues read per page of a JQL search. Not left off: an answer with no
/// `maxResults` takes whatever default the site has today.
const SEARCH_PAGE: usize = 100;

/// Comments read per page of an issue's own comment list, for the same reason
/// [`SEARCH_PAGE`] is sent.
const COMMENTS_PAGE: usize = 100;

/// The fields an Epic is read back for, asked for by name: an issue read with no
/// `fields` carries every field the site has, every custom one included, and
/// three of them are the whole of what a draft reads.
const BRIEF_FIELDS: &str = "summary,status,description";

/// The field a cut's pre-flight reads the Epic back for, which is one field
/// because nothing of it is read: what the read answers is whether the Epic is
/// there at all, and the key a child is filed under rides outside `fields`
/// whatever is asked for. Asked for all the same, so an issue with two hundred
/// custom fields does not come back whole to be thrown away.
const PARENT_FIELDS: &str = "summary";

/// The field a freshly filed child is read back for, and the whole of it: where
/// the workflow's initial transition put the issue, which a create's own answer
/// says nothing about.
const STATUS_FIELDS: &str = "status";

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
/// Every verb but seven refuses as this stands: the two a push makes, the
/// brief-by-name lookup and the brief create; the two a cut reads first, for
/// whose key it is and for the brief itself; the pre-flight a cut settles
/// before it files anything and the create that files each of a slice's drafts;
/// and the listing a bare `warlock draft` prints. The requests themselves
/// arrive a slice at a time, and a board that refuses is what keeps a flow
/// which reaches a verb nobody has written yet a sentence rather than a panic.
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

    /// Warlock's own notes on one issue, oldest first, as Markdown: the whole
    /// record of which of a brief's slices a run has already cut or been told to
    /// pass over.
    ///
    /// The prefixes are matched here and not on the wire, which is the one
    /// difference from `linear.rs`'s own read: JQL has no operator for the text
    /// of a comment, so every comment on the Epic arrives and a person's own
    /// comment is dropped on this side. Every page of them, because a note
    /// missed is a slice filed twice.
    ///
    /// Each body arrives as an ADF tree — that is what a comment is under
    /// `/rest/api/3` — so the match runs on what [`body_for`] reads back out of
    /// it, which is also the Markdown a note was written as.
    ///
    /// A page that failed fails the whole read rather than answering the notes
    /// that arrived before it, as a search's pages do: a cut that read half the
    /// notes would file the slices in the other half a second time.
    fn notes(&self, key: &str) -> Result<Vec<String>, Error> {
        let mut notes = Vec::new();
        let mut read = 0;

        loop {
            let answer = self
                .calls
                .call(Method::GET, &comments(key, read), Value::Null)?;
            let page = answer
                .get("comments")
                .and_then(Value::as_array)
                .ok_or_else(|| missing("comments"))?;
            // Read rather than counted off the pages that arrived: an answer
            // that says nothing about how many comments there are is one this
            // read cannot tell the end of, and guessing it has ended there is
            // the slice-filed-twice failure again.
            let total = answer
                .get("total")
                .and_then(Value::as_u64)
                .ok_or_else(|| missing("total"))?;

            read += page.len();
            for comment in page {
                let body = body_for(comment.get("body").unwrap_or(&Value::Null));
                let body = body.trim();
                if body.starts_with(CUT_NOTE) || body.starts_with(SKIP_NOTE) {
                    notes.push(body.to_owned());
                }
            }

            // The empty page is checked first, so a `total` that outruns the
            // comments a site hands back ends the read rather than asking for
            // the same window forever.
            if page.is_empty() || read >= usize::try_from(total).unwrap_or(usize::MAX) {
                return Ok(notes);
            }
        }
    }

    /// Put a freshly filed child in a status for work nobody has started, when
    /// the workflow did not open it in one.
    ///
    /// One read and nothing more on an ordinary board, which is the whole cost
    /// of it: a create answers an `id`, a `key` and a `self` and says nothing
    /// about status, so where the workflow's initial transition put the child
    /// has to be read back — and a child already in a [`Category::New`] status,
    /// which is what every default workflow answers, is left exactly as it
    /// landed.
    ///
    /// A move is a transition and there is nothing else it could be: Jira takes
    /// no status in a create body and none in an edit either. The transition is
    /// chosen on the name its `to` carries, which is the name the pre-flight
    /// read off the project's own statuses, and never on the category — a
    /// workflow holding two `new` statuses would otherwise put the child in
    /// whichever one the site happened to list a transition to first.
    ///
    /// A workflow offering no transition into that status posts nothing and
    /// leaves the child where it landed: the issue exists, it is the Epic's
    /// child and it carries the label, and there is no second way to ask for a
    /// status this one cannot reach. A call that failed is the caller's to
    /// report, as every call in this module is — which on this one means a cut
    /// that refuses with an issue standing, the same as a create that failed
    /// partway through a slice.
    fn begun(&self, key: &str, status: &str) -> Result<(), Error> {
        let child = self.calls.call(
            Method::GET,
            &format!("{ISSUE}/{key}?fields={STATUS_FIELDS}"),
            Value::Null,
        )?;
        let fields = child.get("fields").ok_or_else(|| missing("fields"))?;

        // The same judgement a fetched brief's own status gets, on the category
        // key and never on the name: `To Do` on one project is `Triage` on the
        // next, and what is being asked is whether this is work nobody has
        // started.
        if standing(fields.get("status")).1 {
            return Ok(());
        }

        let offered = self
            .calls
            .call(Method::GET, &transitions(key), Value::Null)?;
        let Some(transition) = chosen(&offered, status)? else {
            return Ok(());
        };

        self.calls
            .call(
                Method::POST,
                &transitions(key),
                json!({ "transition": { "id": transition } }),
            )
            .map(drop)
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

/// What a cut on this board has settled before its first child issue: the Epic
/// every one of them hangs off, by the key Jira spells it with, and a status in
/// the project that a ticket nobody has started belongs in.
///
/// This board's [`Filing`](Board::Filing), so the fields are private to this
/// module as Linear's [`Resolved`](crate::linear::Resolved) are: the cut carries
/// one of these from [`Board::ticket_filing`] to every [`Board::create_ticket`]
/// and has no way to read it.
///
/// The status by name and not as an id, which is the whole difference from
/// Linear's filing. Jira takes an issue's opening status from the workflow's
/// initial transition and not from the create body, so there is no id for a
/// create to write: this is the status a child is moved into when it did not
/// land in a `new` category, and a transition is chosen on the name its `to`
/// carries.
#[derive(Debug, Clone)]
pub(crate) struct Settled {
    epic: String,
    status: String,
}

impl<C: Calls> Board for Jira<C> {
    const NAME: &'static str = JIRA;

    type Filing = Settled;

    /// The account the key authenticates as, as Jira's own `accountId`: the one
    /// value every read that filters on a person is written against, since a
    /// Jira user is addressed by that id and not by the email the key is typed
    /// with.
    ///
    /// No `Option`, as `linear.rs`'s own `viewer` answers none: a request the
    /// site answered at all was authenticated as somebody, so an answer carrying
    /// no account is malformed and not an absence for a caller to have words
    /// about.
    ///
    /// One request, and a read: the body is [`Value::Null`], so the GET leaves
    /// with nothing after its headers. The record is read first all the same, so
    /// a board opened for a scope that files somewhere else is told that with
    /// nothing sent.
    fn viewer(&self) -> Result<String, Error> {
        self.site()?;
        let account = self.calls.call(Method::GET, MYSELF, Value::Null)?;

        account
            .get("accountId")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| missing("accountId"))
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

    /// The Epic this key names — its summary as the brief's name, its status,
    /// where a person opens it, its description as Markdown and warlock's own
    /// notes on it — or `None` when the site holds no such issue.
    ///
    /// The key is the whole address, as [`create_brief`](Self::create_brief)
    /// answers one: nothing here looks a brief up by a slug, a numeric id or a
    /// name.
    ///
    /// Two reads and not one. Asking for the issue's `comment` field would
    /// carry its first page of comments along with the issue and no more, and
    /// reading the rest means the comment collection anyway — so the notes are
    /// read there from the start, by [`notes`](Self::notes).
    ///
    /// The description goes through [`body_for`], which is total: an ADF node
    /// type warlock does not model contributes the text under it, so a brief
    /// somebody edited in the Jira UI comes back plainer with its headings
    /// intact rather than not at all. An Epic with no description at all is the
    /// empty content [`FetchedBrief`] allows, which the caller that parses it
    /// refuses in its own words.
    fn fetch_brief(&self, slug: &str) -> Result<Option<FetchedBrief>, Error> {
        let (base_url, _) = self.site()?;

        let issue = match self.calls.call(
            Method::GET,
            &format!("{ISSUE}/{slug}?fields={BRIEF_FIELDS}"),
            Value::Null,
        ) {
            Ok(issue) => issue,
            Err(Error::Absent { .. }) => return Ok(None),
            Err(error) => return Err(error),
        };

        // Jira's own spelling of the key and not the one that was asked for, so
        // a key typed in lower case is recorded as the board writes it: this is
        // the brief's id for the rest of the cut, the key its notes are read
        // under and the key a person is shown.
        let key = issue
            .get("key")
            .and_then(Value::as_str)
            .ok_or_else(|| missing("key"))?
            .to_owned();
        let fields = issue.get("fields").ok_or_else(|| missing("fields"))?;
        let (status, draftable) = standing(fields.get("status"));

        Ok(Some(FetchedBrief {
            name: fields
                .get("summary")
                .and_then(Value::as_str)
                .ok_or_else(|| missing("summary"))?
                .to_owned(),
            content: body_for(fields.get("description").unwrap_or(&Value::Null)),
            url: browse(base_url, &key),
            status,
            draftable,
            notes: self.notes(&key)?,
            id: key,
        }))
    }

    /// The Epics in the project that carry the record's label and sit in a
    /// status category nobody has started, by key and summary, every page of
    /// them: what a bare `warlock draft <scope>` prints as the briefs there are
    /// slices left to cut from.
    ///
    /// The key where the Linear arm answers a slug, because the key is the
    /// whole address of an Epic: what a listing prints is what
    /// [`fetch_brief`](Self::fetch_brief) takes next, so the word on the line is
    /// the word a person types into `warlock draft <scope> <key>`.
    ///
    /// The JQL is the whole filter and nothing re-reads it here, as the Linear
    /// arm's own filter is its query. The label narrows it to what a push
    /// filed, which is what a draft can read: an Epic somebody made by hand has
    /// no `## Scope` warlock wrote the shape of. [`PLANNED`] narrows it to work
    /// nobody has begun — an Epic in `indeterminate` or in `done` is not fresh
    /// work to file slices against, and neither ever arrives.
    ///
    /// A label carrying a double quote would end the quoted JQL value early and
    /// Jira would refuse the search in its own words. Nothing is escaped on the
    /// way: Jira holds no such label, so a record naming one is a manifest to
    /// fix rather than a search to repair.
    fn planned_briefs(&self) -> Result<Listing, Error> {
        let (_, project_key) = self.site()?;
        let label = self.record.label();
        let jql = format!(
            r#"project = {project_key} AND issuetype = {EPIC} AND labels = "{label}" AND statusCategory = {PLANNED}"#
        );

        let projects = self
            .searched(&jql, &["summary"])?
            .iter()
            .map(|issue| {
                let key = issue
                    .get("key")
                    .and_then(Value::as_str)
                    .ok_or_else(|| missing("key"))?;
                let summary = issue
                    .get("fields")
                    .and_then(|fields| fields.get("summary"))
                    .and_then(Value::as_str)
                    .ok_or_else(|| missing("summary"))?;

                Ok((key.to_owned(), summary.to_owned()))
            })
            .collect::<Result<_, Error>>()?;

        Ok(Listing { projects })
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
    /// A double quote or a backslash in the name is a space in the phrase. Left
    /// in, either ends the quoted value early and Jira refuses the search, so a
    /// brief with a quoted word in its title could never be pushed. The text
    /// match drops punctuation anyway, and the comparison against the full
    /// name is what decides, so the space narrows no less and matches nothing
    /// [`same_name`] would not.
    fn brief_named(&self, name: &str) -> Result<Option<String>, Error> {
        let (base_url, project_key) = self.site()?;
        // The [`EPIC`] the create writes as well, and the escaped quotes are a
        // quoted phrase inside the quoted value `~` takes.
        let phrase = name.replace(['"', '\\'], " ");
        let jql = format!(
            r#"project = {project_key} AND issuetype = {EPIC} AND summary ~ "\"{phrase}\"""#
        );

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

    /// What this board must have in hand before a cut files anything: a status
    /// in the record's project for a ticket nobody has started, and the Epic the
    /// children will hang off.
    ///
    /// No status id is resolved and none could be. Jira takes a created issue's
    /// status from the workflow's initial transition and ignores a status in the
    /// create body, so what the project read buys is not a value for the create
    /// but the answer to whether this project can take a fresh ticket at all —
    /// which is the whole of why the refusal lives here rather than in the
    /// create, where the slice would already be half filed.
    ///
    /// Two reads, no mutation, and the order is the refusal a caller can act
    /// on. The project first, because its key is the record's: a project the
    /// site does not hold is [`Filable::NoDestination`], which the caller words
    /// as the manifest to fix, and a project holding no `new`-category status is
    /// [`Filable::NoColumn`], which it words as the board nobody has finished
    /// setting up. Neither asks about the Epic. The Epic last, because its key
    /// is the one a person typed: an Epic the site holds nothing for is the
    /// [`Error::Absent`] every read here answers a 404 with, and the caller
    /// propagates it rather than wording a destination.
    ///
    /// Jira's own spelling of the key rides back and not the one that was asked
    /// for, as [`fetch_brief`](Self::fetch_brief) answers one: the create writes
    /// the parent by the key, so the key it writes is the one the site confirmed.
    fn ticket_filing(&self, brief: &str) -> Result<Filable<Settled>, Error> {
        let (_, project_key) = self.site()?;

        let answered = match self.calls.call(
            Method::GET,
            &format!("{PROJECT}/{project_key}/statuses"),
            Value::Null,
        ) {
            Ok(answered) => answered,
            Err(Error::Absent { .. }) => return Ok(Filable::NoDestination),
            Err(error) => return Err(error),
        };
        let Some(groups) = answered.as_array() else {
            return Err(missing("statuses"));
        };
        let Some(status) = fresh(groups) else {
            return Ok(Filable::NoColumn);
        };

        let epic = self.calls.call(
            Method::GET,
            &format!("{ISSUE}/{brief}?fields={PARENT_FIELDS}"),
            Value::Null,
        )?;
        let epic = epic
            .get("key")
            .and_then(Value::as_str)
            .ok_or_else(|| missing("key"))?
            .to_owned();

        Ok(Filable::Ready(Settled { epic, status }))
    }

    /// File one draft as an issue in the record's project, under the Epic the
    /// pre-flight settled, carrying the label, the assignee and its body as an
    /// ADF document — then put it in a status for work nobody has started, if
    /// the workflow did not.
    ///
    /// `fields.parent` is what makes it the Epic's child, which is the whole of
    /// how a cut's tickets hang off a brief on this board: the JQL
    /// `parent = <epic key>` finds them afterwards, and nothing else on the
    /// issue says which brief it came from. The key written is the filing's and
    /// not [`NewTicket::project`]'s — both are this Epic, and the filing's is
    /// the spelling the site itself answered with.
    ///
    /// One request for the issue, as [`create_brief`](Self::create_brief) is
    /// one: the label and the assignee ride in it rather than being written
    /// after, because nothing here can take an issue back and a child that lost
    /// either is a ticket no later run finds or no queue ever offers. The
    /// assignee is Jira's `accountId`, which is what [`viewer`](Self::viewer)
    /// answered and the only way a Jira user is addressed.
    ///
    /// [`TASK`] is the one value warlock names that no draft decided, and Jira
    /// requires it. Nothing else is written: no priority, no estimate, no
    /// sprint, no component and no custom field.
    ///
    /// The id and the identifier are both the key, which is the whole of how
    /// this board names an issue — a link is written by key, a cut record holds
    /// the key and a person types the key — so the numeric `id` beside it in the
    /// answer is read by nothing.
    fn create_ticket(&self, issue: &NewTicket<'_>, filing: &Settled) -> Result<FiledTicket, Error> {
        let (base_url, project_key) = self.site()?;
        let created = self.calls.call(
            Method::POST,
            ISSUE,
            json!({
                "fields": {
                    "project": { "key": project_key },
                    "issuetype": { "name": TASK },
                    "parent": { "key": filing.epic },
                    "summary": issue.title,
                    "labels": [issue.label],
                    "assignee": { "accountId": issue.assignee },
                    "description": document_for(issue.body),
                }
            }),
        )?;
        let key = created
            .get("key")
            .and_then(Value::as_str)
            .ok_or_else(|| missing("key"))?
            .to_owned();

        self.begun(&key, &filing.status)?;

        Ok(FiledTicket {
            id: key.clone(),
            identifier: key.clone(),
            url: browse(base_url, &key),
        })
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
/// as JSON would fail a call that worked. A 404 is [`Error::Absent`] whatever
/// the body says, so the status survives Jira's prose. Any other refusal is
/// Jira's own words when the body carries any, and the bare status when it
/// carries none.
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

    let message = serde_json::from_str::<Value>(text)
        .ok()
        .and_then(|body| refusal(&body));
    if status == 404 {
        return Err(Error::Absent {
            board: JIRA,
            message,
        });
    }
    match message {
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

/// Where one issue's comments are read, from the issue's key and how many of
/// them this read has already seen.
///
/// The window is in the path because [`Calls`] carries a method, a path and a
/// body and nothing else: a GET's query parameters are part of its path here,
/// which is the same reason a JQL read is a POST.
///
/// `startAt` and not a token, which is the whole difference from the search's
/// paging: Jira's comment collection answers a window and the `total` beside it,
/// and the next page is the window after the comments that arrived.
///
/// `orderBy` is spelled rather than taken as read, because the one caller needs
/// the notes oldest first: a cut note and a later skip note for the same slice
/// are read in the order a person wrote them.
fn comments(key: &str, read: usize) -> String {
    format!("{ISSUE}/{key}/comment?startAt={read}&maxResults={COMMENTS_PAGE}&orderBy=created")
}

/// Where one issue's moves are read and where one of them is spent: the same
/// path for both, a GET answering what the workflow offers from where the issue
/// stands and a POST taking one of them by its id.
fn transitions(key: &str) -> String {
    format!("{ISSUE}/{key}/transitions")
}

/// The first transition the issue offers into a status of that name, by the id
/// a POST spends it with, or `None` when it offers none.
///
/// The name and not the category, for the reason [`Jira::begun`] states: a
/// workflow can hold two statuses in one category, and which of them a caller
/// meant is a thing only the name says. Compared as [`same_name`] compares —
/// trimmed and case-folded — because the name on this side was read off a
/// status and the one on the wire is the board's own word for where a
/// transition lands.
///
/// An answer carrying no `transitions` at all is malformed and not an issue with
/// nowhere to go: a workflow always offers something from where an issue stands,
/// so an answer with no list in it is bytes a proxy decided to send.
fn chosen(offered: &Value, status: &str) -> Result<Option<String>, Error> {
    let offered = offered
        .get("transitions")
        .and_then(Value::as_array)
        .ok_or_else(|| missing("transitions"))?;

    Ok(offered.iter().find_map(|transition| {
        let name = transition.get("to")?.get("name")?.as_str()?;
        let id = transition.get("id")?.as_str()?;

        same_name(name, status).then(|| id.to_owned())
    }))
}

/// One issue's status as a read answers it: the name a refusal prints, and
/// whether nobody has started it.
///
/// The two come apart on purpose. The name is a site's own word — `To Do` on one
/// board and `Triage` on the next — so the judgement is made on the category
/// Jira files that status under, and the name only ever rides back to be printed.
/// [`Category::New`] is the one category a brief may be cut in: an Epic somebody
/// has started or finished is not work to file slices against. It is the same
/// judgement [`Jira::begun`] makes of a child that was just filed, which is why
/// the flag is a category and not a word about drafting.
///
/// A status with no category, or one spelling a word [`Category::of`] does not
/// know, is not one: an Epic warlock cannot place is left alone rather than
/// offered as fresh work, and a child that landed in one is moved.
fn standing(status: Option<&Value>) -> (Option<String>, bool) {
    let name = status
        .and_then(|status| status.get("name"))
        .and_then(Value::as_str)
        .map(str::to_owned);
    let category = status
        .and_then(|status| status.get("statusCategory"))
        .and_then(|category| category.get("key"))
        .and_then(Value::as_str)
        .unwrap_or_default();

    (name, Category::of(category) == Category::New)
}

/// A status in the project that a ticket nobody has started belongs in, by the
/// board's own name for it, or `None` when the project offers none.
///
/// The judgement is [`Category::of`]'s, on the category key and never on the
/// name, for the reason [`standing`] states: `To Do` on one project is `Triage`
/// on the next, and the category is the one vocabulary every board shares. The
/// first such status in the order the site listed them — a project with a
/// `Backlog` and a `To Do` offers two, and neither is more a beginning than the
/// other.
///
/// A project's statuses arrive grouped by issue type, one group per type
/// carrying the statuses that type's workflow holds, and every group is read:
/// what is being asked is whether this project can take a fresh ticket at all,
/// and a child's issue type is the project's own default rather than a type
/// warlock names.
///
/// A group carrying no `statuses` is read as a status itself, so an answer that
/// is a flat list of statuses is read the same way. This is the one read in this
/// module nobody has put a curl against: of the two ways to be wrong about its
/// shape, refusing a project that can file is the one that stops a cut, and
/// reading a status out of either shape costs a line.
fn fresh(groups: &[Value]) -> Option<String> {
    groups
        .iter()
        .flat_map(|group| {
            group
                .get("statuses")
                .and_then(Value::as_array)
                .map_or_else(|| std::slice::from_ref(group), Vec::as_slice)
        })
        .find_map(|status| {
            let category = status
                .get("statusCategory")
                .and_then(|category| category.get("key"))
                .and_then(Value::as_str)?;
            let name = status.get("name").and_then(Value::as_str)?;

            (Category::of(category) == Category::New).then(|| name.to_owned())
        })
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
