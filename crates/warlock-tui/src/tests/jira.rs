use std::error::Error as _;
use std::io;

use serde_json::{Value, json};
use ureq::http::Method;
use warlock_engine::ScopeRecord;

use super::{
    Calls, Client, EMAIL_TOKEN, JIRA, Jira, REQUEST_TIMEOUT, answer, authorization, base64, pair,
    refusal, url,
};

use crate::board::{Board, Error, NewBrief, NewTicket};
use crate::stubs::Calling;

const EMAIL: &str = "nobody@example.com";
const TOKEN: &str = "ATATT3xFfGF0_a_token_nobody_holds_8f3a1c";
const SITE: &str = "https://example.atlassian.net";
/// The project key the record names, and the key every issue key a verb is
/// handed below starts with, so one assertion against it covers both halves of
/// what a refusal must never carry.
const PROJECT: &str = "PAY";

fn key() -> String {
    format!("{EMAIL}:{TOKEN}")
}

/// The record a board is opened for: a verb is told none of it, so the site and
/// the project key a request carries can only have come from here.
fn a_scope() -> ScopeRecord {
    ScopeRecord::jira("payments", SITE, PROJECT, "In Review", "area/pay")
}

fn variants() -> Vec<Error> {
    vec![
        Error::Key { board: JIRA },
        Error::Mistyped {
            board: JIRA,
            expected: EMAIL_TOKEN,
        },
        Error::Elsewhere {
            board: JIRA,
            scope: "payments".to_owned(),
        },
        Error::Untaught {
            board: JIRA,
            verb: "file a brief",
        },
        Error::Transport {
            board: JIRA,
            source: Box::new(ureq::Error::Io(io::Error::other(
                "connection reset by peer",
            ))),
        },
        Error::Status {
            board: JIRA,
            code: 401,
        },
        Error::Malformed {
            board: JIRA,
            detail: "it was not JSON".to_owned(),
        },
        Error::Refused {
            board: JIRA,
            message: "Issue does not exist".to_owned(),
        },
    ]
}

#[test]
fn one_timeout_of_thirty_seconds_covers_the_whole_call() {
    assert_eq!(REQUEST_TIMEOUT.as_secs(), 30, "thirty seconds, per call");
    // The global timeout, which is the one that covers resolve, connect,
    // handshake, send and body rather than any single phase of them, and the
    // same constant Linear's calls run under rather than a second thirty.
    assert_eq!(
        Client::new(SITE, key()).timeout(),
        Some(crate::linear::REQUEST_TIMEOUT)
    );
}

#[test]
fn the_vendor_is_spelled_once_and_every_error_reads_as_jiras() {
    assert_eq!(JIRA, "Jira");
    // The board's own name is that one spelling and not a second literal, so a
    // decorator that prints `NAME` and an error this module built read alike.
    assert_eq!(<Jira<Calling> as Board>::NAME, JIRA);

    for error in variants() {
        assert!(
            error.to_string().contains("Jira"),
            "the board is not named: {error}"
        );
    }

    let transport = Error::Transport {
        board: JIRA,
        source: Box::new(ureq::Error::Io(io::Error::other("no route to host"))),
    };

    let shown = transport.to_string();

    // ureq's own wording follows the colon, whatever it is for the failure that
    // arrived; warlock's half of the sentence is the half asserted here.
    assert!(shown.starts_with("could not reach Jira: "), "{shown}");
    assert!(shown.contains("no route to host"), "{shown}");
}

#[test]
fn the_key_is_a_parameter_and_the_client_never_prints_it() {
    let client = Client::new(SITE, key());

    let printed = format!("{client:?}");

    assert!(
        !printed.contains(TOKEN) && !printed.contains(EMAIL),
        "the key reached `Debug` output: {printed}"
    );
}

#[test]
fn no_error_variant_prints_the_key() {
    for error in variants() {
        let (shown, printed) = (error.to_string(), format!("{error:?}"));

        assert!(!shown.contains(TOKEN), "the key reached `Display`: {shown}");
        assert!(
            !printed.contains(TOKEN),
            "the key reached `Debug`: {printed}"
        );
    }
}

#[test]
fn only_a_transport_failure_has_a_source_under_it() {
    for error in variants() {
        let expected = matches!(error, Error::Transport { .. });

        assert_eq!(error.source().is_some(), expected, "{error:?}");
    }
}

#[test]
fn the_authorization_value_is_basic_the_encoded_pair() {
    let value = authorization(&key()).expect("an `email:token` line is a header value");

    let sent = value.to_str().expect("base64 is a readable value");

    // Not `Bearer <token>`: that is what an OAuth token wants, and it is a 401
    // for an API token.
    assert_eq!(sent, format!("Basic {}", base64(key().as_bytes())));
    assert!(!sent.contains("Bearer"));
    assert!(!sent.contains(TOKEN));
    // And marked sensitive, so a header map that gets printed prints
    // `Sensitive` where the key is.
    assert!(value.is_sensitive());
    assert!(!format!("{value:?}").contains(TOKEN));
}

#[test]
fn the_pair_is_the_stored_line_split_at_the_first_colon() {
    assert_eq!(
        pair(&key()).expect("an `email:token` line is a pair"),
        (EMAIL, TOKEN)
    );
    // A token carrying a colon keeps it: only the first one divides the halves.
    assert_eq!(
        pair("nobody@example.com:a:token").expect("the first colon divides"),
        ("nobody@example.com", "a:token")
    );
}

#[test]
fn a_line_that_is_not_a_pair_is_refused_in_warlocks_words_without_quoting_it() {
    for stored in [TOKEN.to_owned(), format!(":{TOKEN}"), format!("{EMAIL}:")] {
        let error = pair(&stored).expect_err("the line is not an `email:token` pair");

        assert!(
            matches!(error, Error::Mistyped { .. }),
            "{stored}: {error:?}"
        );

        let shown = error.to_string();

        assert!(shown.contains("email:token"), "{stored}: {shown}");
        assert!(shown.contains("Jira"), "{stored}: {shown}");
        assert!(!shown.contains(TOKEN), "the line was quoted: {shown}");
    }
}

#[test]
fn a_line_that_is_not_a_pair_is_refused_before_a_request_is_built() {
    // A port nothing listens on, so a regression that built the request anyway
    // fails here with a transport failure rather than reaching a real site.
    let client = Client::new("http://127.0.0.1:1", TOKEN);

    let error = client
        .call(Method::GET, "/rest/api/3/myself", Value::Null)
        .expect_err("the stored line is not an `email:token` pair");

    assert!(matches!(error, Error::Mistyped { .. }), "{error:?}");
}

#[test]
fn the_url_is_the_site_as_recorded_with_one_slash_before_the_path() {
    let joined = "https://example.atlassian.net/rest/api/3/myself";

    assert_eq!(url(SITE, "/rest/api/3/myself"), joined);
    // A record keeps the trailing slash somebody typed, and a path spelled
    // without a leading one is the same call.
    assert_eq!(
        url("https://example.atlassian.net/", "/rest/api/3/myself"),
        joined
    );
    assert_eq!(url(SITE, "rest/api/3/myself"), joined);
}

#[test]
fn base64_pads_a_tail_of_one_byte_and_of_two() {
    for (bytes, encoded) in [
        // 3n, so no padding.
        ("", ""),
        ("foo", "Zm9v"),
        ("foobar", "Zm9vYmFy"),
        // 3n+1, so two `=`.
        ("f", "Zg=="),
        ("foob", "Zm9vYg=="),
        // 3n+2, so one `=`.
        ("fo", "Zm8="),
        ("fooba", "Zm9vYmE="),
    ] {
        assert_eq!(base64(bytes.as_bytes()), encoded, "{bytes}");
    }

    assert_eq!(
        base64(key().as_bytes()),
        "bm9ib2R5QGV4YW1wbGUuY29tOkFUQVRUM3hGZkdGMF9hX3Rva2VuX25vYm9keV9ob2xkc184ZjNhMWM=",
    );
}

#[test]
fn base64_spells_the_last_two_sextets_as_plus_and_slash() {
    // The sextets 62 and 63, which a URL-safe alphabet would write `-` and `_`
    // and `Authorization: Basic` would not read.
    assert_eq!(base64(&[0xFB, 0xFF, 0xBF]), "+/+/");
    assert_eq!(base64(&[0xFF]), "/w==");
    assert_eq!(base64(&[0, 0, 0]), "AAAA");
}

#[test]
fn every_base64_quantum_is_four_characters() {
    for length in 0..32 {
        let encoded = base64(&vec![b'x'; length]);

        assert_eq!(encoded.len() % 4, 0, "{length}: {encoded}");
        assert_eq!(encoded.len(), length.div_ceil(3) * 4, "{length}: {encoded}");
    }
}

#[test]
fn a_success_with_no_body_answers_null() {
    // A transition answers 204 and an issue link 201, each with nothing after
    // the headers.
    assert_eq!(answer(204, "").unwrap(), Value::Null);
    assert_eq!(answer(201, " \n").unwrap(), Value::Null);
}

#[test]
fn a_success_with_a_body_answers_its_json() {
    assert_eq!(
        answer(201, r#"{"key":"KAN-2"}"#).unwrap(),
        json!({ "key": "KAN-2" })
    );
}

#[test]
fn a_success_whose_body_is_not_json_is_malformed() {
    assert!(matches!(
        answer(200, "<html>sign in</html>"),
        Err(Error::Malformed { board: JIRA, .. })
    ));
}

#[test]
fn a_refusal_carries_jiras_reasons() {
    let error = answer(
        404,
        r#"{"errorMessages":["Issue does not exist or you do not have permission to see it."],"errors":{}}"#,
    )
    .unwrap_err();

    assert_eq!(
        error.to_string(),
        "Jira refused the request: Issue does not exist or you do not have permission to see it."
    );
}

#[test]
fn field_errors_are_named_after_the_messages() {
    let body = json!({
        "errorMessages": ["The request was invalid."],
        "errors": { "summary": "You must specify a summary of the issue." }
    });

    assert_eq!(
        refusal(&body).unwrap(),
        "The request was invalid.; summary: You must specify a summary of the issue."
    );
}

/// Every [`Board`] verb warlock has no request for on this board yet, driven
/// once against one board, as the name a reader would look the method up by and
/// the refusal it answered.
///
/// Written out by hand and in one place, because every test below asks the same
/// thing of all of them: a verb added to the trait is added here as well, and a
/// verb a slice teaches moves out of here and into [`every_verb`]; nothing but a
/// reader checks either.
fn untaught_verbs(board: &Jira<Calling>) -> Vec<(&'static str, Option<Error>)> {
    vec![
        ("viewer", board.viewer().err()),
        ("move_brief", board.move_brief("10001", "In Review").err()),
        ("ticket_brief", board.ticket_brief("PAY-1").err()),
        ("move_ticket", board.move_ticket("PAY-1", "In Review").err()),
        ("fetch_brief", board.fetch_brief("a-brief").err()),
        ("planned_briefs", board.planned_briefs().err()),
        ("scope_queue", board.scope_queue("me").err()),
        ("named_ticket", board.named_ticket(1).err()),
        ("ticket_filing", board.ticket_filing().err()),
        (
            "create_ticket",
            board
                .create_ticket(
                    &NewTicket::new("A slice", "## Goal", "10001", "area/pay", "me"),
                    &(),
                )
                .err(),
        ),
        (
            "create_relation",
            board.create_relation("PAY-1", "PAY-2").err(),
        ),
        (
            "comment_on_brief",
            board.comment_on_brief("10001", "a note").err(),
        ),
        (
            "comment_on_ticket",
            board.comment_on_ticket("PAY-1", "a note").err(),
        ),
    ]
}

/// Every verb, the ones a slice has taught after the ones it has not: what a
/// board opened for a scope that files somewhere else has to refuse, whether or
/// not warlock knows how to ask it.
fn every_verb(board: &Jira<Calling>) -> Vec<(&'static str, Option<Error>)> {
    let mut verbs = untaught_verbs(board);

    verbs.push(("brief_named", board.brief_named(NAME).err()));
    verbs.push(("create_brief", board.create_brief(&a_brief()).err()));
    verbs
}

#[test]
fn every_unwritten_verb_refuses_with_a_sentence_and_sends_nothing() {
    // Nothing queued, so a verb that reached the transport panics the stand-in
    // rather than answering anything.
    let board = Jira::new(Calling::answering([]), a_scope());

    for (verb, refusal) in untaught_verbs(&board) {
        let error =
            refusal.unwrap_or_else(|| panic!("`{verb}` answered instead of refusing the request"));

        assert!(
            matches!(error, Error::Untaught { board: JIRA, .. }),
            "{verb}: {error:?}"
        );

        let shown = error.to_string();

        // A sentence rather than a panic, naming the board and what warlock
        // cannot do with it yet.
        assert!(shown.contains("Jira"), "{verb}: {shown}");
        assert!(shown.contains("yet"), "{verb}: {shown}");
        // And carrying no key value: not the project the record names, and not
        // the issue key a verb was handed.
        assert!(
            !shown.contains(PROJECT),
            "{verb}: a refusal carries a key: {shown}"
        );
    }

    assert!(board.calls.made().is_empty(), "{:?}", board.calls.made());
}

#[test]
fn each_verb_is_refused_in_its_own_words() {
    let board = Jira::new(Calling::answering([]), a_scope());

    let mut said: Vec<String> = untaught_verbs(&board)
        .into_iter()
        .map(|(verb, refusal)| {
            refusal
                .unwrap_or_else(|| panic!("`{verb}` answered instead of refusing"))
                .to_string()
        })
        .collect();
    let verbs = said.len();

    said.sort_unstable();
    said.dedup();

    // One sentence per verb, so a reader who sees a refusal knows which one
    // they reached rather than only that something is unwritten.
    assert_eq!(said.len(), verbs, "{said:?}");
}

#[test]
fn a_board_opened_for_a_scope_that_files_elsewhere_sends_nothing() {
    // Out of reach through warlock's own verbs, each of which refuses such a
    // scope before it opens anything — but opening cannot fail, so the refusal
    // has to be a verb's and it has to arrive with no request made.
    let board = Jira::new(
        Calling::answering([]),
        ScopeRecord::new("data-plane", "WAR", "In Review", "area/data"),
    );

    for (verb, refusal) in every_verb(&board) {
        let error = refusal.unwrap_or_else(|| panic!("`{verb}` answered for a Linear record"));

        // The record and not the unwritten verb, because the two are different
        // things to fix.
        assert!(
            matches!(&error, Error::Elsewhere { scope, .. } if scope == "data-plane"),
            "{verb}: {error:?}"
        );
    }

    assert!(board.calls.made().is_empty(), "{:?}", board.calls.made());
}

/// The brief a push is about to file, which is the name every lookup below
/// asks by.
const NAME: &str = "Push a brief to the board";

/// One page of a JQL answer: an issue per key and summary, and the token to
/// read on from when a page follows this one.
fn page(issues: &[(&str, &str)], next: Option<&str>) -> Value {
    let issues: Vec<Value> = issues
        .iter()
        .map(|(key, summary)| json!({ "key": key, "fields": { "summary": summary } }))
        .collect();
    let mut answer = json!({ "issues": issues });

    if let Some(token) = next {
        answer["nextPageToken"] = json!(token);
    }

    answer
}

/// One page of the lookup's read, as the stand-in records a call: the JQL is
/// spelled out here rather than built from the module's own pieces, so a change
/// to what Jira is asked shows up as a failure and not as two expressions that
/// moved together.
fn asked(token: Option<&str>) -> (Method, String, Value) {
    let mut body = json!({
        // The project key is the record's: no verb is handed one.
        "jql": format!(r#"project = {PROJECT} AND issuetype = Epic AND summary ~ "\"{NAME}\"""#),
        "fields": ["summary"],
        "maxResults": 100,
    });

    if let Some(token) = token {
        body["nextPageToken"] = json!(token);
    }

    (Method::POST, "/rest/api/3/search/jql".to_owned(), body)
}

#[test]
fn a_brief_of_the_same_name_is_answered_by_the_url_a_person_opens_it_at() {
    let board = Jira::new(
        Calling::answering([Ok(page(&[("PAY-14", NAME)], None))]),
        a_scope(),
    );

    assert_eq!(
        board.brief_named(NAME).expect("the stand-in answered"),
        Some(format!("{SITE}/browse/PAY-14"))
    );
    // A POST and not a GET: the JQL goes in the body, because nothing in the
    // crate percent-encodes a query string into a path.
    assert_eq!(board.calls.made(), [asked(None)]);
}

#[test]
fn a_project_holding_no_brief_of_that_name_answers_none() {
    let board = Jira::new(Calling::answering([Ok(page(&[], None))]), a_scope());

    assert_eq!(
        board.brief_named(NAME).expect("the stand-in answered"),
        None
    );
    assert_eq!(board.calls.made(), [asked(None)]);
}

#[test]
fn a_near_name_is_not_the_brief_already_filed() {
    // `~` is a fuzzy text match, so a summary carrying the name's words comes
    // back whether or not it is the name: the comparison and not the search is
    // what answers, or a push of `Push a brief to the board` would be refused
    // over a brief somebody else filed.
    let longer = format!("{NAME} over Calls");
    let board = Jira::new(
        Calling::answering([Ok(page(
            &[("PAY-9", longer.as_str()), ("PAY-11", "Push a brief")],
            None,
        ))]),
        a_scope(),
    );

    assert_eq!(
        board.brief_named(NAME).expect("the stand-in answered"),
        None
    );
    assert_eq!(board.calls.made(), [asked(None)]);
}

#[test]
fn a_name_differing_in_case_or_in_spacing_is_the_same_brief() {
    // `queue::same_name`, which is the comparison `project_named` makes on
    // Linear: a brief filed from a heading somebody retyped is the same brief.
    let shouted = format!("  {}  ", NAME.to_uppercase());
    let board = Jira::new(
        Calling::answering([Ok(page(&[("PAY-14", shouted.as_str())], None))]),
        a_scope(),
    );

    assert_eq!(
        board.brief_named(NAME).expect("the stand-in answered"),
        Some(format!("{SITE}/browse/PAY-14"))
    );
}

#[test]
fn a_match_on_the_second_page_is_found_by_the_token_the_first_page_gave() {
    let longer = format!("{NAME} over Calls");
    let board = Jira::new(
        Calling::answering([
            Ok(page(&[("PAY-9", longer.as_str())], Some("page-2"))),
            Ok(page(&[("PAY-14", NAME)], None)),
        ]),
        a_scope(),
    );

    assert_eq!(
        board.brief_named(NAME).expect("the stand-in answered"),
        Some(format!("{SITE}/browse/PAY-14"))
    );
    // One request per page, the second carrying the token the first answered,
    // and the page carrying no token is the last one: Jira has no `hasNextPage`
    // flag beside it to read.
    assert_eq!(board.calls.made(), [asked(None), asked(Some("page-2"))]);
}

#[test]
fn a_search_jira_refuses_is_reported_in_jiras_words_and_is_not_retried() {
    let said = "The value 'Epic' does not exist for the field 'issuetype'.";
    let board = Jira::new(
        Calling::answering([Err(Error::Refused {
            board: JIRA,
            message: said.to_owned(),
        })]),
        a_scope(),
    );

    let error = board.brief_named(NAME).expect_err("the search was refused");

    assert!(error.to_string().contains(said), "{error}");
    assert_eq!(board.calls.made(), [asked(None)], "the search was retried");
}

#[test]
fn a_page_that_failed_fails_the_read_rather_than_answering_the_pages_before_it() {
    // The match is on the page that arrived and the read fails all the same: a
    // lookup that answered here would be answering about a project it only read
    // part of, and the next page is where a second brief of this name would be.
    let board = Jira::new(
        Calling::answering([
            Ok(page(&[("PAY-14", NAME)], Some("page-2"))),
            Err(Error::Status {
                board: JIRA,
                code: 502,
            }),
        ]),
        a_scope(),
    );

    let error = board.brief_named(NAME).expect_err("a page did not arrive");

    assert!(
        matches!(error, Error::Status { code: 502, .. }),
        "{error:?}"
    );
    assert_eq!(board.calls.made().len(), 2, "{:?}", board.calls.made());
}

#[test]
fn an_answer_missing_its_issues_its_summary_or_a_matchs_key_is_malformed() {
    for answer in [
        json!({ "total": 1 }),
        json!({ "issues": [{ "key": "PAY-14" }] }),
        json!({ "issues": [{ "fields": { "summary": NAME } }] }),
    ] {
        let board = Jira::new(Calling::answering([Ok(answer.clone())]), a_scope());

        let error = board
            .brief_named(NAME)
            .expect_err("not the answer asked for");

        assert!(
            matches!(error, Error::Malformed { board: JIRA, .. }),
            "{answer}: {error:?}"
        );
    }
}

/// The label the record records, which is the one a push hands the create: no
/// verb is told a label either, so a label in a request can only have come
/// through [`NewBrief`].
const LABEL: &str = "area/pay";

/// A brief's body as somebody wrote it, short enough for the ADF tree it
/// becomes to be spelled out beside it: a heading, a paragraph and one bullet.
const BODY: &str = "## Problem\n\nPush has nowhere to file.\n\n- One request\n";

/// The brief every create below files.
fn a_brief() -> NewBrief<'static> {
    NewBrief::new(NAME, BODY, LABEL)
}

/// The one request a create makes, as the stand-in records it. Spelled out
/// rather than built from `adf::document_for` and the module's own constants,
/// so a change to what Jira is sent — a field added, the type renamed, the
/// converter's own tree moving — is a failure here and not two expressions that
/// moved together.
fn filed() -> (Method, String, Value) {
    (
        Method::POST,
        "/rest/api/3/issue".to_owned(),
        json!({
            "fields": {
                // The project is the record's key, as the lookup's JQL is.
                "project": { "key": PROJECT },
                "issuetype": { "name": "Epic" },
                "summary": NAME,
                "labels": [LABEL],
                "description": {
                    "type": "doc",
                    "version": 1,
                    "content": [
                        {
                            "type": "heading",
                            "attrs": { "level": 2 },
                            "content": [{ "type": "text", "text": "Problem" }],
                        },
                        {
                            "type": "paragraph",
                            "content": [
                                { "type": "text", "text": "Push has nowhere to file." },
                            ],
                        },
                        {
                            "type": "bulletList",
                            "content": [{
                                "type": "listItem",
                                "content": [{
                                    "type": "paragraph",
                                    "content": [
                                        { "type": "text", "text": "One request" },
                                    ],
                                }],
                            }],
                        },
                    ],
                },
            }
        }),
    )
}

#[test]
fn a_brief_is_filed_as_one_epic_carrying_the_label_and_its_body_as_adf() {
    let board = Jira::new(
        Calling::answering([Ok(json!({ "id": "10042", "key": "PAY-14" }))]),
        a_scope(),
    );

    let filed_brief = board
        .create_brief(&a_brief())
        .expect("the stand-in answered")
        .expect("a Jira create either files or is refused");

    // The key and not the numeric id beside it: every path addressing an issue
    // takes either, and the key is the one a person reads.
    assert_eq!(filed_brief.id(), "PAY-14");
    // Built from the key the create answered and the site the record names.
    assert_eq!(filed_brief.url(), format!("{SITE}/browse/PAY-14"));
    // One request, carrying the whole of what warlock asks for: the label is in
    // the create because it is the only mark saying warlock filed this Epic and
    // nothing here can take an Epic back.
    assert_eq!(board.calls.made(), [filed()]);
}

#[test]
fn the_create_sends_no_status_and_transitions_nothing_after_it() {
    let board = Jira::new(
        Calling::answering([Ok(json!({ "key": "PAY-14" }))]),
        a_scope(),
    );

    board
        .create_brief(&a_brief())
        .expect("the stand-in answered");

    let made = board.calls.made();

    // The Epic opens in whatever status the project's workflow starts an issue
    // in: no status is asked for on the way in, and nothing reads the issue's
    // transitions or spends one afterwards. One call, and it is the create.
    assert_eq!(made.len(), 1, "{made:?}");
    let (_, path, body) = &made[0];
    assert!(!path.contains("transitions"), "{path}");
    let fields = body["fields"]
        .as_object()
        .expect("the create writes a `fields` object");
    let sent: Vec<&str> = fields.keys().map(String::as_str).collect();
    assert_eq!(
        sent,
        ["project", "issuetype", "summary", "labels", "description"],
        "{sent:?}"
    );
}

#[test]
fn a_create_jira_refuses_is_reported_in_jiras_words_and_is_not_retried() {
    let said = "project: project is required";
    let board = Jira::new(
        Calling::answering([Err(Error::Refused {
            board: JIRA,
            message: said.to_owned(),
        })]),
        a_scope(),
    );

    let error = board.create_brief(&a_brief()).expect_err("it was refused");

    assert!(error.to_string().contains(said), "{error}");
    // A mutation is one request: a create that failed is reported and the
    // person decides, because nothing here can tell a refusal that arrived
    // before the Epic existed from one that arrived after.
    assert_eq!(board.calls.made(), [filed()], "the create was retried");
}

#[test]
fn a_create_answering_no_key_is_malformed() {
    let board = Jira::new(
        Calling::answering([Ok(json!({ "id": "10042" }))]),
        a_scope(),
    );

    let error = board
        .create_brief(&a_brief())
        .expect_err("not the answer asked for");

    assert!(
        matches!(error, Error::Malformed { board: JIRA, .. }),
        "{error:?}"
    );
}

#[test]
fn a_refusal_with_no_reasons_is_its_status() {
    // A 401 from Jira, a proxy's HTML and an envelope with empty reasons all
    // name nothing warlock can pass on.
    for text in [
        "",
        "<html>bad gateway</html>",
        r#"{"errorMessages":[],"errors":{}}"#,
    ] {
        assert!(matches!(
            answer(401, text),
            Err(Error::Status {
                board: JIRA,
                code: 401
            })
        ));
    }
}
