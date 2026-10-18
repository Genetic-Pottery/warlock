use std::error::Error as _;
use std::io;

use serde_json::{Value, json};
use ureq::http::Method;
use warlock_engine::{BoardScope, ScopeRecord};

use super::{
    Calls, Client, EMAIL_TOKEN, JIRA, Jira, REQUEST_TIMEOUT, Settled, answer, authorization,
    base64, pair, refusal, standing, url,
};

use crate::adf::document_for;
use crate::board::{
    Assignee, Board, Category, Error, Filable, Holder, Moved, NewBrief, NewTicket, Priority,
};
use crate::brief::{Slice, scope_block_in};
use crate::cut;
use crate::queue::{IN_PROGRESS, Named, Refusal, take_named};
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
        Error::Absent {
            board: JIRA,
            message: None,
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
        400,
        r#"{"errorMessages":["The value 'Epic' does not exist for the field 'issuetype'."],"errors":{}}"#,
    )
    .unwrap_err();

    assert_eq!(
        error.to_string(),
        "Jira refused the request: The value 'Epic' does not exist for the field 'issuetype'."
    );
}

#[test]
fn a_404_is_an_absence_whatever_its_body_says() {
    let said = "Issue does not exist or you do not have permission to see it.";
    let worded = answer(
        404,
        &format!(r#"{{"errorMessages":["{said}"],"errors":{{}}}}"#),
    )
    .unwrap_err();
    let bare = answer(404, "<html>not found</html>").unwrap_err();

    assert!(
        matches!(&worded, Error::Absent { board: JIRA, message: Some(message) } if message == said),
        "{worded:?}"
    );
    assert_eq!(worded.to_string(), format!("Jira answered 404: {said}"));
    assert!(
        matches!(
            &bare,
            Error::Absent {
                board: JIRA,
                message: None
            }
        ),
        "{bare:?}"
    );
    assert_eq!(bare.to_string(), "Jira answered 404");
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
    vec![("ticket_brief", board.ticket_brief("PAY-1").err())]
}

/// Every verb, the ones a slice has taught after the ones it has not: what a
/// board opened for a scope that files somewhere else has to refuse, whether or
/// not warlock knows how to ask it.
fn every_verb(board: &Jira<Calling>) -> Vec<(&'static str, Option<Error>)> {
    let mut verbs = untaught_verbs(board);

    verbs.push(("viewer", board.viewer().err()));
    verbs.push(("fetch_brief", board.fetch_brief(BRIEF).err()));
    verbs.push(("planned_briefs", board.planned_briefs().err()));
    verbs.push(("brief_named", board.brief_named(NAME).err()));
    verbs.push(("scope_queue", board.scope_queue(ACCOUNT).err()));
    verbs.push(("named_ticket", board.named_ticket(1).err()));
    verbs.push(("create_brief", board.create_brief(&a_brief()).err()));
    verbs.push(("ticket_filing", board.ticket_filing(BRIEF).err()));
    verbs.push((
        "create_ticket",
        board.create_ticket(&a_draft(), &a_filing()).err(),
    ));
    verbs.push((
        "create_relation",
        board.create_relation(CHILD, "PAY-21").err(),
    ));
    verbs.push((
        "comment_on_brief",
        board.comment_on_brief(BRIEF, A_CUT).err(),
    ));
    verbs.push((
        "comment_on_ticket",
        board.comment_on_ticket(TICKET, A_HALT).err(),
    ));
    verbs.push(("move_brief", board.move_brief(BRIEF, IN_PROGRESS).err()));
    verbs.push(("move_ticket", board.move_ticket(TICKET, IN_PROGRESS).err()));
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

/// The account the key authenticates as, as Jira spells an `accountId`: an
/// opaque id and not the email the key is typed with, which is the whole reason
/// this is read off the site rather than split out of the stored line.
const ACCOUNT: &str = "5b10a2844c20165700ede21g";

/// The one request the account read makes, as the stand-in records it. Spelled
/// out rather than built from the module's own constant, so a change to what
/// Jira is asked — the path moving, a body appearing, a POST where a GET was —
/// is a failure here and not two expressions that moved together.
fn whoami() -> (Method, String, Value) {
    (Method::GET, "/rest/api/3/myself".to_owned(), Value::Null)
}

#[test]
fn the_account_the_key_belongs_to_is_read_once_from_myself_and_carries_no_body() {
    let board = Jira::new(
        Calling::answering([Ok(json!({
            "accountId": ACCOUNT,
            "emailAddress": EMAIL,
            "displayName": "Nobody At All",
        }))]),
        a_scope(),
    );

    // The `accountId` and nothing else off the answer: the email and the display
    // name beside it are not what a read filters on.
    assert_eq!(board.viewer().expect("the stand-in answered"), ACCOUNT);
    // One request, and it is a GET under `/rest/api/3` with `Value::Null` for a
    // body, so nothing goes out after the headers. A second call would panic the
    // stand-in, which is how "read once per run" is asserted.
    assert_eq!(board.calls.made(), [whoami()]);
}

#[test]
fn an_account_read_answering_no_account_id_is_malformed_and_not_an_absence() {
    // A request the site answered at all was authenticated as somebody, so there
    // is no account for this verb to be missing: an answer with none is bytes a
    // proxy decided to send.
    let board = Jira::new(
        Calling::answering([Ok(json!({ "displayName": "Nobody At All" }))]),
        a_scope(),
    );

    let error = board.viewer().expect_err("not the answer asked for");

    assert!(
        matches!(error, Error::Malformed { board: JIRA, .. }),
        "{error:?}"
    );
    assert!(error.to_string().contains("accountId"), "{error}");
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
fn a_name_with_a_quote_or_a_backslash_searches_on_spaces_and_compares_whole() {
    // Either character would end the quoted JQL value early, and Jira would
    // refuse the search, so the push of such a brief could never get past it.
    let name = r#"Accept "quoted" words \ here"#;
    let board = Jira::new(
        Calling::answering([Ok(page(&[("PAY-3", name)], None))]),
        a_scope(),
    );

    assert_eq!(
        board.brief_named(name).expect("the stand-in answered"),
        Some(format!("{SITE}/browse/PAY-3"))
    );
    let made = board.calls.made();
    assert_eq!(
        made[0].2["jql"],
        json!(format!(
            r#"project = {PROJECT} AND issuetype = Epic AND summary ~ "\"Accept  quoted  words   here\"""#
        ))
    );
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

/// One page of the listing's read, as the stand-in records a call. The JQL is
/// spelled out here rather than built from the module's own pieces, for the
/// reason [`asked`] spells its own: a clause dropped, the category changing or
/// the label leaving the search is a failure here and not two expressions that
/// moved together.
fn listed(token: Option<&str>) -> (Method, String, Value) {
    let mut body = json!({
        // The project key and the label are both the record's: the verb takes
        // no argument at all, so neither can have come from anywhere else.
        "jql": format!(
            r#"project = {PROJECT} AND issuetype = Epic AND labels = "{LABEL}" AND statusCategory = new"#
        ),
        "fields": ["summary"],
        "maxResults": 100,
    });

    if let Some(token) = token {
        body["nextPageToken"] = json!(token);
    }

    (Method::POST, "/rest/api/3/search/jql".to_owned(), body)
}

#[test]
fn the_planned_briefs_are_the_labelled_epics_nobody_has_started_by_key_and_summary() {
    let board = Jira::new(
        Calling::answering([Ok(page(
            &[
                ("PAY-14", "Draft from the board"),
                ("PAY-21", "Give the CLI a voice"),
            ],
            None,
        ))]),
        a_scope(),
    );

    let listing = board.planned_briefs().expect("the stand-in answered");

    // The issue key where the Linear arm answers a slug: it is what a person
    // types back into `warlock draft <scope> <key>`, which is the same key
    // `fetch_brief` reads an Epic by.
    assert_eq!(
        listing.projects(),
        [
            ("PAY-14".to_owned(), "Draft from the board".to_owned()),
            ("PAY-21".to_owned(), "Give the CLI a voice".to_owned()),
        ]
    );
    // One request, a POST with the JQL in its body, narrowed to the project,
    // the label, the issue type and the status category — so an Epic warlock
    // did not file, and one somebody has started or finished, never arrives.
    assert_eq!(board.calls.made(), [listed(None)]);
}

#[test]
fn the_category_a_listing_asks_for_is_the_one_a_fetched_epic_is_draftable_in() {
    let board = Jira::new(Calling::answering([Ok(page(&[], None))]), a_scope());

    board.planned_briefs().expect("the stand-in answered");

    let jql = board.calls.made()[0].2["jql"]
        .as_str()
        .expect("the search carries a JQL string")
        .to_owned();

    // The two halves of the same judgement: the word the search narrows by is
    // the category `standing` calls draftable, and the categories it leaves out
    // are the ones a fetched Epic is refused for. A listing offering a brief
    // the draft after it turns down would be offering work that cannot be cut.
    assert!(jql.contains("statusCategory = new"), "{jql}");
    for (category, draftable) in [("new", true), ("indeterminate", false), ("done", false)] {
        assert_eq!(
            standing(Some(&json!({
                "name": "A column",
                "statusCategory": { "key": category },
            })))
            .1,
            draftable,
            "{category}"
        );
    }
}

#[test]
fn a_listing_reads_every_page_by_the_token_the_page_before_it_gave() {
    let board = Jira::new(
        Calling::answering([
            Ok(page(&[("PAY-14", "Draft from the board")], Some("page-2"))),
            Ok(page(&[("PAY-21", "Give the CLI a voice")], None)),
        ]),
        a_scope(),
    );

    let listing = board.planned_briefs().expect("the stand-in answered");

    assert_eq!(listing.projects().len(), 2, "{:?}", listing.projects());
    // One request per page, the second carrying the token the first answered,
    // and the page with no token is the last.
    assert_eq!(board.calls.made(), [listed(None), listed(Some("page-2"))]);
}

#[test]
fn a_project_holding_no_such_epic_answers_an_empty_listing_and_not_a_failure() {
    // Which is what lets the caller print its own nothing-to-list line: an
    // empty project is an ordinary answer and not a read that went wrong.
    let board = Jira::new(Calling::answering([Ok(page(&[], None))]), a_scope());

    let listing = board.planned_briefs().expect("the stand-in answered");

    assert!(listing.projects().is_empty(), "{:?}", listing.projects());
    assert_eq!(board.calls.made(), [listed(None)]);
}

#[test]
fn a_listing_jira_refuses_is_reported_in_jiras_words_and_is_not_retried() {
    let said = "The value 'area/pay' does not exist for the field 'labels'.";
    let board = Jira::new(
        Calling::answering([Err(Error::Refused {
            board: JIRA,
            message: said.to_owned(),
        })]),
        a_scope(),
    );

    let error = board.planned_briefs().expect_err("the search was refused");

    assert!(error.to_string().contains(said), "{error}");
    assert_eq!(board.calls.made(), [listed(None)], "the search was retried");
}

#[test]
fn a_listing_page_that_failed_fails_the_read_rather_than_answering_the_ones_before_it() {
    // A listing of half a project is a listing a person would draft from
    // believing it whole: the brief they were looking for is the one on the
    // page that never arrived.
    let board = Jira::new(
        Calling::answering([
            Ok(page(&[("PAY-14", "Draft from the board")], Some("page-2"))),
            Err(Error::Status {
                board: JIRA,
                code: 502,
            }),
        ]),
        a_scope(),
    );

    let error = board.planned_briefs().expect_err("a page did not arrive");

    assert!(
        matches!(error, Error::Status { code: 502, .. }),
        "{error:?}"
    );
    assert_eq!(board.calls.made().len(), 2, "{:?}", board.calls.made());
}

#[test]
fn a_listing_answer_missing_its_issues_a_key_or_a_summary_is_malformed() {
    for answer in [
        json!({ "total": 1 }),
        json!({ "issues": [{ "fields": { "summary": "Draft from the board" } }] }),
        json!({ "issues": [{ "key": "PAY-14" }] }),
    ] {
        let board = Jira::new(Calling::answering([Ok(answer.clone())]), a_scope());

        let error = board
            .planned_briefs()
            .expect_err("not the answer asked for");

        assert!(
            matches!(error, Error::Malformed { board: JIRA, .. }),
            "{answer}: {error:?}"
        );
    }
}

/// The key a draft is handed, as a person types one: the whole address of the
/// Epic every read below asks for.
const BRIEF: &str = "PAY-14";

/// A brief as it comes back off the board, which is what a cut parses: the scope
/// heading and one slice under it. Spelled as the Markdown it is, because the
/// assertion is that the ADF the site holds reads back as this and not as
/// something a parse has to be taught.
const BRIEF_BODY: &str = "## Scope\n\n### 1. Read the Epic\n\nBy its key.\n";

/// The status a team-managed project's workflow opens an issue in, and the
/// category Jira files it under: the word is the board's and the category is
/// what a judgement is made on.
const TO_DO: &str = "To Do";

/// The issue read, as the stand-in records it. Spelled out rather than built
/// from the module's own constants, so a change to what Jira is asked — the
/// path moving, the fields asked for changing, a body appearing — is a failure
/// here and not two expressions that moved together.
fn fetched(key: &str) -> (Method, String, Value) {
    (
        Method::GET,
        format!("/rest/api/3/issue/{key}?fields=summary,status,description"),
        Value::Null,
    )
}

/// One page of the notes read, by the key it is under and how many comments the
/// read has already seen. A GET with its window in the path, because [`Calls`]
/// carries a method, a path and a body and nothing else.
fn noted_page(key: &str, at: usize) -> (Method, String, Value) {
    (
        Method::GET,
        format!("/rest/api/3/issue/{key}/comment?startAt={at}&maxResults=100&orderBy=created"),
        Value::Null,
    )
}

/// One Epic as an issue read answers it: the key Jira spells it with, the
/// summary, the status and its category, and the description as an ADF tree.
fn epic(status: &str, category: &str, description: &Value) -> Value {
    json!({
        "id": "10042",
        "key": BRIEF,
        "fields": {
            "summary": NAME,
            "status": { "name": status, "statusCategory": { "key": category } },
            "description": description,
        },
    })
}

/// The Epic a draft is about to read, in the status it is open for drafting in.
fn a_draftable_epic() -> Value {
    epic(TO_DO, "new", &document_for(BRIEF_BODY))
}

/// One page of an issue's comments, each body the ADF a comment arrives as:
/// written through the converter the notes go up as, so what is asserted is the
/// round trip and not a tree typed out by hand to match the reader.
fn commented(bodies: &[&str], at: usize, total: usize) -> Value {
    let comments: Vec<Value> = bodies
        .iter()
        .enumerate()
        .map(|(index, body)| {
            json!({ "id": format!("{}", at + index + 1), "body": document_for(body) })
        })
        .collect();

    json!({ "startAt": at, "maxResults": 100, "total": total, "comments": comments })
}

/// An issue with no comments at all, which is every brief before its first cut.
fn uncommented() -> Value {
    commented(&[], 0, 0)
}

#[test]
fn an_epic_is_read_by_its_key_as_its_name_status_url_and_description() {
    let board = Jira::new(
        Calling::answering([Ok(a_draftable_epic()), Ok(uncommented())]),
        a_scope(),
    );

    let brief = board
        .fetch_brief(BRIEF)
        .expect("the stand-in answered")
        .expect("the site holds the key");

    // The key is the id, as the create answers one: nothing here addresses a
    // brief by a slug or by a numeric id.
    assert_eq!(brief.id(), BRIEF);
    assert_eq!(brief.name(), NAME);
    assert_eq!(brief.status(), Some(TO_DO));
    assert_eq!(brief.url(), format!("{SITE}/browse/{BRIEF}"));
    // The description back through the ADF reader, which is the Markdown it was
    // filed as: a cut parses this for its `## Scope` block.
    assert_eq!(brief.content(), BRIEF_BODY);
    assert!(brief.draftable(), "`To Do` is a `new` category status");
    assert!(brief.notes().is_empty(), "{:?}", brief.notes());
    // Two reads and no mutation: the issue, then its comments. The fields are
    // asked for by name, so a site's custom fields do not ride along.
    assert_eq!(
        board.calls.made(),
        [fetched(BRIEF), noted_page(BRIEF, 0)],
        "{:?}",
        board.calls.made()
    );
}

#[test]
fn the_key_jira_spells_back_is_the_one_recorded_and_the_notes_are_read_under_it() {
    let typed = "pay-14";
    let board = Jira::new(
        Calling::answering([Ok(a_draftable_epic()), Ok(uncommented())]),
        a_scope(),
    );

    let brief = board
        .fetch_brief(typed)
        .expect("the stand-in answered")
        .expect("Jira reads a key case-insensitively");

    assert_eq!(brief.id(), BRIEF);
    assert_eq!(brief.url(), format!("{SITE}/browse/{BRIEF}"));
    // The read goes out under the key that was typed, and everything after it
    // under the key Jira answered with.
    assert_eq!(
        board.calls.made(),
        [fetched(typed), noted_page(BRIEF, 0)],
        "{:?}",
        board.calls.made()
    );
}

#[test]
fn only_a_new_category_status_is_draftable_and_the_boards_own_word_rides_back() {
    for (status, category, draftable) in [
        (TO_DO, "new", true),
        ("In Progress", "indeterminate", false),
        ("Shipped", "done", false),
        // A category key nothing has seen: the corner where a brief is neither
        // offered as fresh work nor treated as finished.
        ("Held", "invented", false),
    ] {
        let board = Jira::new(
            Calling::answering([
                Ok(epic(status, category, &document_for(BRIEF_BODY))),
                Ok(uncommented()),
            ]),
            a_scope(),
        );

        let brief = board
            .fetch_brief(BRIEF)
            .expect("the stand-in answered")
            .expect("the site holds the key");

        assert_eq!(brief.draftable(), draftable, "{status} ({category})");
        // The board's own spelling and not a word warlock has for the category:
        // a refusal names the status a person sees on the board.
        assert_eq!(brief.status(), Some(status), "{status}");
    }
}

#[test]
fn a_status_with_no_category_or_no_status_at_all_is_not_draftable() {
    for status in [json!({ "name": "Held" }), Value::Null] {
        let mut answer = a_draftable_epic();
        answer["fields"]["status"] = status.clone();
        let board = Jira::new(
            Calling::answering([Ok(answer), Ok(uncommented())]),
            a_scope(),
        );

        let brief = board
            .fetch_brief(BRIEF)
            .expect("the stand-in answered")
            .expect("the site holds the key");

        // An Epic warlock cannot place is left alone rather than offered as
        // fresh work, and a status it could not read is no status at all.
        assert!(!brief.draftable(), "{status}");
        assert_eq!(brief.status(), status["name"].as_str(), "{status}");
    }
}

/// A note each of the two shapes `cut.rs` writes, and a comment a person left,
/// in the order they were written.
const A_PERSONS_COMMENT: &str = "Looks right to me, cutting this now.";
const A_CUT: &str = "Warlock cut slice `Read the Epic` into `PAY-20`, `PAY-21`.";
const A_SKIP: &str = "Warlock skipped slice `Read the notes`.";

#[test]
fn only_warlocks_own_notes_are_carried_back_and_a_persons_comment_is_not() {
    let board = Jira::new(
        Calling::answering([
            Ok(a_draftable_epic()),
            Ok(commented(&[A_PERSONS_COMMENT, A_CUT, A_SKIP], 0, 3)),
        ]),
        a_scope(),
    );

    let brief = board
        .fetch_brief(BRIEF)
        .expect("the stand-in answered")
        .expect("the site holds the key");

    // The filter is on this side because JQL has no operator for a comment's
    // text, and it is the two prefixes `board.rs` spells and nothing wider: a
    // comment that starts some other way is a person talking.
    assert_eq!(brief.notes(), [A_CUT, A_SKIP]);
    // And read back by the reader that matters, through the ADF a comment
    // arrives as: a body `cut::noted` cannot read is a slice filed twice.
    assert_eq!(
        cut::noted(brief.notes()),
        [
            (
                "read the epic".to_owned(),
                vec!["PAY-20".to_owned(), "PAY-21".to_owned()]
            ),
            ("read the notes".to_owned(), Vec::new()),
        ]
    );
}

#[test]
fn every_page_of_notes_is_read_oldest_first() {
    let board = Jira::new(
        Calling::answering([
            Ok(a_draftable_epic()),
            Ok(commented(&[A_PERSONS_COMMENT, A_CUT], 0, 3)),
            Ok(commented(&[A_SKIP], 2, 3)),
        ]),
        a_scope(),
    );

    let brief = board
        .fetch_brief(BRIEF)
        .expect("the stand-in answered")
        .expect("the site holds the key");

    assert_eq!(brief.notes(), [A_CUT, A_SKIP]);
    // One request per page, each window starting where the comments that
    // arrived left off, and `orderBy` spelled so the oldest note is the first
    // one read.
    assert_eq!(
        board.calls.made(),
        [fetched(BRIEF), noted_page(BRIEF, 0), noted_page(BRIEF, 2)],
        "{:?}",
        board.calls.made()
    );
}

#[test]
fn a_key_the_site_does_not_hold_is_an_absence_and_no_comments_are_read() {
    for failure in [
        Error::Absent {
            board: JIRA,
            message: Some(
                "Issue does not exist or you do not have permission to see it.".to_owned(),
            ),
        },
        Error::Absent {
            board: JIRA,
            message: None,
        },
    ] {
        let said = failure.to_string();
        let board = Jira::new(Calling::answering([Err(failure)]), a_scope());

        assert_eq!(
            board
                .fetch_brief("PAY-999")
                .expect("an absence, not a failure"),
            None,
            "{said}"
        );
        // One request: there is nothing to read the notes of.
        assert_eq!(board.calls.made(), [fetched("PAY-999")], "{said}");
    }
}

#[test]
fn a_refusal_about_anything_else_is_reported_in_jiras_words() {
    // "does not exist" is also how a site says it has no such field or issue
    // type, and a 401 is a key to fix: neither is a brief that is not there.
    let said = "The value 'Epic' does not exist for the field 'issuetype'.";
    for failure in [
        Error::Refused {
            board: JIRA,
            message: said.to_owned(),
        },
        Error::Status {
            board: JIRA,
            code: 401,
        },
    ] {
        let expected = failure.to_string();
        let board = Jira::new(Calling::answering([Err(failure)]), a_scope());

        let error = board.fetch_brief(BRIEF).expect_err("the read was refused");

        assert_eq!(error.to_string(), expected);
    }
}

#[test]
fn an_epic_with_no_description_is_the_empty_content_a_caller_refuses_itself() {
    for description in [Value::Null, json!({ "type": "doc", "version": 1 })] {
        let mut answer = a_draftable_epic();
        answer["fields"]["description"] = description.clone();
        let board = Jira::new(
            Calling::answering([Ok(answer), Ok(uncommented())]),
            a_scope(),
        );

        let brief = board
            .fetch_brief(BRIEF)
            .expect("the stand-in answered")
            .expect("the site holds the key");

        // An emptied description is not a broken answer: the caller that parses
        // it refuses the brief in its own words.
        assert_eq!(brief.content(), "", "{description}");
    }
}

#[test]
fn a_description_carrying_an_adf_node_the_reader_does_not_know_still_parses() {
    // A brief lightly edited in the Jira UI: a panel beside the headings, which
    // is a node type warlock neither writes nor models.
    let description = json!({
        "type": "doc",
        "version": 1,
        "content": [
            {
                "type": "heading",
                "attrs": { "level": 2 },
                "content": [{ "type": "text", "text": "Scope" }],
            },
            {
                "type": "panel",
                "attrs": { "panelType": "info" },
                "content": [{
                    "type": "paragraph",
                    "content": [{ "type": "text", "text": "Edited here." }],
                }],
            },
            {
                "type": "heading",
                "attrs": { "level": 3 },
                "content": [{ "type": "text", "text": "1. Read the Epic" }],
            },
            {
                "type": "paragraph",
                "content": [{ "type": "text", "text": "By its key." }],
            },
            {
                "type": "heading",
                "attrs": { "level": 3 },
                "content": [{ "type": "text", "text": "2. Read the notes" }],
            },
            {
                "type": "paragraph",
                "content": [{ "type": "text", "text": "Every page." }],
            },
        ],
    });
    let board = Jira::new(
        Calling::answering([Ok(epic(TO_DO, "new", &description)), Ok(uncommented())]),
        a_scope(),
    );

    let brief = board
        .fetch_brief(BRIEF)
        .expect("the stand-in answered")
        .expect("the site holds the key");

    // The panel comes back as the text under it and the headings are still
    // headings, so the block parses and both slices are there.
    let block = scope_block_in(brief.content()).expect("a scope block with slices under it");
    let headings: Vec<&str> = block.slices().iter().map(Slice::heading).collect();
    assert_eq!(headings, ["Read the Epic", "Read the notes"]);
    assert!(
        brief.content().contains("Edited here."),
        "{}",
        brief.content()
    );
}

#[test]
fn an_issue_read_missing_its_key_its_fields_or_its_summary_is_malformed() {
    for answer in [
        json!({ "fields": { "summary": NAME } }),
        json!({ "key": BRIEF }),
        json!({ "key": BRIEF, "fields": { "status": { "name": TO_DO } } }),
    ] {
        let board = Jira::new(Calling::answering([Ok(answer.clone())]), a_scope());

        let error = board
            .fetch_brief(BRIEF)
            .expect_err("not the answer asked for");

        assert!(
            matches!(error, Error::Malformed { board: JIRA, .. }),
            "{answer}: {error:?}"
        );
    }
}

#[test]
fn a_comment_page_with_no_comments_or_no_total_is_malformed() {
    for page in [json!({ "total": 1 }), json!({ "comments": [] })] {
        let board = Jira::new(
            Calling::answering([Ok(a_draftable_epic()), Ok(page.clone())]),
            a_scope(),
        );

        let error = board
            .fetch_brief(BRIEF)
            .expect_err("not the answer asked for");

        // A read that cannot tell how far it got is not a read that has
        // finished: half the notes means the other half's slices filed again.
        assert!(
            matches!(error, Error::Malformed { board: JIRA, .. }),
            "{page}: {error:?}"
        );
    }
}

#[test]
fn a_page_of_notes_that_failed_fails_the_whole_read() {
    let board = Jira::new(
        Calling::answering([
            Ok(a_draftable_epic()),
            Ok(commented(&[A_CUT], 0, 2)),
            Err(Error::Status {
                board: JIRA,
                code: 502,
            }),
        ]),
        a_scope(),
    );

    let error = board
        .fetch_brief(BRIEF)
        .expect_err("a page of notes did not arrive");

    // Not the notes that did arrive: the page that never came is where the note
    // for the slice this run is about to file would be.
    assert!(
        matches!(error, Error::Status { code: 502, .. }),
        "{error:?}"
    );
    assert_eq!(board.calls.made().len(), 3, "{:?}", board.calls.made());
}

#[test]
fn a_comment_page_that_answers_fewer_comments_than_its_total_ends_the_read() {
    // A site that says there are more comments than it hands back: the read
    // ends rather than asking for the same window forever.
    let board = Jira::new(
        Calling::answering([Ok(a_draftable_epic()), Ok(commented(&[], 0, 7))]),
        a_scope(),
    );

    let brief = board
        .fetch_brief(BRIEF)
        .expect("the stand-in answered")
        .expect("the site holds the key");

    assert!(brief.notes().is_empty(), "{:?}", brief.notes());
    assert_eq!(board.calls.made(), [fetched(BRIEF), noted_page(BRIEF, 0)]);
}

#[test]
fn no_read_of_a_brief_goes_under_the_older_api() {
    let board = Jira::new(
        Calling::answering([
            Ok(a_draftable_epic()),
            Ok(commented(&[A_CUT], 0, 2)),
            Ok(commented(&[A_SKIP], 1, 2)),
        ]),
        a_scope(),
    );

    board.fetch_brief(BRIEF).expect("the stand-in answered");

    for (method, path, body) in board.calls.made() {
        // Jira renders a v2 description as wiki markup, which is what makes a
        // `## Scope` parse of one fail; and a read sends no body at all.
        assert!(path.starts_with("/rest/api/3/"), "{path}");
        assert!(!path.contains("/rest/api/2"), "{path}");
        assert_eq!(method, Method::GET, "{path}");
        assert_eq!(body, Value::Null, "{path}");
    }
}

/// One project's statuses as the read answers them: a group per issue type,
/// each carrying the statuses that type's workflow holds, and each status's
/// category key beside its name.
fn project_statuses(types: &[(&str, &[(&str, &str)])]) -> Value {
    types
        .iter()
        .map(|(issue_type, statuses)| {
            let statuses: Vec<Value> = statuses
                .iter()
                .map(|(name, category)| {
                    json!({
                        "id": "10002",
                        "name": name,
                        "statusCategory": { "id": 2, "key": category, "name": name },
                    })
                })
                .collect();

            json!({
                "id": "10001",
                "name": issue_type,
                "subtask": false,
                "statuses": statuses,
            })
        })
        .collect()
}

/// A project a cut can file into: the three statuses a default workflow holds,
/// one of them in the category a ticket nobody has started belongs in.
fn a_filable_project() -> Value {
    project_statuses(&[(
        "Task",
        &[
            (TO_DO, "new"),
            ("In Progress", "indeterminate"),
            ("Done", "done"),
        ],
    )])
}

/// The project read the pre-flight makes, as the stand-in records it. Spelled
/// out rather than built from the module's own constants, for the reason
/// [`asked`] spells its own: the path moving, a body appearing or a POST where a
/// GET was is a failure here and not two expressions that moved together.
fn statuses_of(project_key: &str) -> (Method, String, Value) {
    (
        Method::GET,
        format!("/rest/api/3/project/{project_key}/statuses"),
        Value::Null,
    )
}

/// The Epic read the pre-flight makes, by the key it was handed: one field
/// asked for, because an issue read with no `fields` carries every custom field
/// the site has and nothing of this one is read but the key beside it.
fn reached(key: &str) -> (Method, String, Value) {
    (
        Method::GET,
        format!("/rest/api/3/issue/{key}?fields=summary"),
        Value::Null,
    )
}

/// The Epic as the pre-flight's own read answers it, which is the whole of what
/// that read asks for.
fn reachable() -> Value {
    json!({ "id": "10042", "key": BRIEF, "fields": { "summary": NAME } })
}

/// What the pre-flight settles, as the create of every draft in the slice is
/// handed it back: the Epic by the key Jira spells it with, and the project's
/// status for a ticket nobody has started.
fn a_filing() -> Settled {
    Settled {
        epic: BRIEF.to_owned(),
        status: TO_DO.to_owned(),
    }
}

#[test]
fn the_pre_flight_settles_the_epic_and_the_projects_status_for_a_fresh_ticket() {
    let board = Jira::new(
        Calling::answering([Ok(a_filable_project()), Ok(reachable())]),
        a_scope(),
    );

    let Filable::Ready(settled) = board.ticket_filing(BRIEF).expect("the stand-in answered") else {
        panic!("a project with a status to begin in and an Epic that is there can be filed into");
    };

    // The one `new`-category status of the three, by the board's own name for
    // it: Jira takes a created issue's status from the workflow and not from a
    // create body, so there is no id to settle and the name is what a
    // transition's `to` carries.
    assert_eq!(settled.status, TO_DO);
    // And the Epic the children will hang off, by the key the site spells.
    assert_eq!(settled.epic, BRIEF);
    // Two reads and no mutation, the project before the Epic: the project key
    // is the record's, so a project that cannot be filed into is the manifest's
    // to fix and the Epic is never asked about.
    assert_eq!(
        board.calls.made(),
        [statuses_of(PROJECT), reached(BRIEF)],
        "{:?}",
        board.calls.made()
    );
}

#[test]
fn the_key_jira_spells_back_is_the_one_the_pre_flight_settles() {
    let typed = "pay-14";
    let board = Jira::new(
        Calling::answering([Ok(a_filable_project()), Ok(reachable())]),
        a_scope(),
    );

    let Filable::Ready(settled) = board.ticket_filing(typed).expect("the stand-in answered") else {
        panic!("Jira reads a key case-insensitively");
    };

    // The create writes the parent by the key, so the key it writes is the one
    // the site confirmed rather than the one a person typed.
    assert_eq!(settled.epic, BRIEF);
    assert_eq!(board.calls.made(), [statuses_of(PROJECT), reached(typed)]);
}

#[test]
fn a_status_under_any_of_the_projects_issue_types_is_one_a_fresh_ticket_can_land_in() {
    // What is being asked is whether this project can take a fresh ticket at
    // all: a child's issue type is the project's own default, so every group
    // the read answers is read.
    let board = Jira::new(
        Calling::answering([
            Ok(project_statuses(&[
                ("Epic", &[("In Progress", "indeterminate")]),
                ("Task", &[(TO_DO, "new")]),
            ])),
            Ok(reachable()),
        ]),
        a_scope(),
    );

    let Filable::Ready(settled) = board.ticket_filing(BRIEF).expect("the stand-in answered") else {
        panic!("a project with a status to begin in can be filed into");
    };

    assert_eq!(settled.status, TO_DO);
}

#[test]
fn a_project_answering_a_flat_list_of_statuses_is_read_the_same_way() {
    // The documented answer groups the statuses by issue type, and this is the
    // one read in the module nobody has put a curl against: a site handing back
    // the statuses themselves is read as the same project, because of the two
    // ways to be wrong about the shape, refusing a project that can file is the
    // one that stops a cut.
    let board = Jira::new(
        Calling::answering([
            Ok(json!([
                { "name": "Done", "statusCategory": { "key": "done" } },
                { "name": TO_DO, "statusCategory": { "key": "new" } },
            ])),
            Ok(reachable()),
        ]),
        a_scope(),
    );

    let Filable::Ready(settled) = board.ticket_filing(BRIEF).expect("the stand-in answered") else {
        panic!("a project with a status to begin in can be filed into");
    };

    assert_eq!(settled.status, TO_DO);
}

#[test]
fn a_project_the_site_does_not_hold_has_no_destination_and_the_epic_is_never_read() {
    for message in [
        Some("No project could be found with key 'PAY'.".to_owned()),
        None,
    ] {
        let said = format!("{message:?}");
        let board = Jira::new(
            Calling::answering([Err(Error::Absent {
                board: JIRA,
                message,
            })]),
            a_scope(),
        );

        let filing = board
            .ticket_filing(BRIEF)
            .expect("a project that is not there is an answer");

        // The caller turns this into its own refusal, naming the file the
        // project key is written in: the words about a manifest are not this
        // module's. Nothing is asked about the Epic and no mutation is sent.
        assert!(
            matches!(filing, Filable::NoDestination),
            "{said}: {filing:?}"
        );
        assert_eq!(board.calls.made(), [statuses_of(PROJECT)], "{said}");
    }
}

#[test]
fn a_project_with_no_status_for_a_fresh_ticket_has_no_column_and_the_epic_is_never_read() {
    for statuses in [
        // A project whose workflow has nothing left to begin in.
        project_statuses(&[(
            "Task",
            &[("In Progress", "indeterminate"), ("Done", "done")],
        )]),
        // A category key nothing has seen, which `Category::of` reads as
        // started: the corner where a project is left alone rather than filed
        // into on a guess.
        project_statuses(&[("Task", &[("Held", "invented")])]),
        // A status with no category at all, and a project with no issue types.
        json!([{ "statuses": [{ "name": "Held" }] }]),
        json!([]),
    ] {
        let board = Jira::new(Calling::answering([Ok(statuses.clone())]), a_scope());

        let filing = board
            .ticket_filing(BRIEF)
            .expect("a project with no such status is an answer");

        // Told apart from a project that is not there at all, because the caller
        // says a different sentence about each — and neither reads the Epic or
        // sends a mutation.
        assert!(
            matches!(filing, Filable::NoColumn),
            "{statuses}: {filing:?}"
        );
        assert_eq!(board.calls.made(), [statuses_of(PROJECT)], "{statuses}");
    }
}

#[test]
fn an_epic_the_site_does_not_hold_is_the_absence_the_caller_propagates() {
    // Not one of the two destination answers: the project key is the record's
    // and the Epic's key is the one a person typed, so a key nothing is there
    // for is the 404 every read here answers with and the caller carries it.
    let said = "Issue does not exist or you do not have permission to see it.";
    let board = Jira::new(
        Calling::answering([
            Ok(a_filable_project()),
            Err(Error::Absent {
                board: JIRA,
                message: Some(said.to_owned()),
            }),
        ]),
        a_scope(),
    );

    let error = board
        .ticket_filing("PAY-999")
        .expect_err("the Epic is not there");

    assert!(
        matches!(error, Error::Absent { board: JIRA, .. }),
        "{error:?}"
    );
    assert!(error.to_string().contains(said), "{error}");
    assert_eq!(
        board.calls.made(),
        [statuses_of(PROJECT), reached("PAY-999")]
    );
}

#[test]
fn a_project_read_jira_refuses_is_reported_in_jiras_words_and_the_epic_is_never_read() {
    let said = "You do not have permission to view this project.";
    let board = Jira::new(
        Calling::answering([Err(Error::Refused {
            board: JIRA,
            message: said.to_owned(),
        })]),
        a_scope(),
    );

    let error = board
        .ticket_filing(BRIEF)
        .expect_err("the project read was refused");

    assert!(error.to_string().contains(said), "{error}");
    // One request per read and no retry, as every call on this transport is.
    assert_eq!(
        board.calls.made(),
        [statuses_of(PROJECT)],
        "the read was retried"
    );
}

#[test]
fn a_statuses_answer_that_is_not_a_list_or_an_epic_with_no_key_is_malformed() {
    for answers in [
        vec![Ok(json!({ "values": [] }))],
        vec![Ok(Value::Null)],
        // The Epic's own read: a request the site answered at all answered
        // about an issue, so an answer with no key is bytes a proxy sent.
        vec![
            Ok(a_filable_project()),
            Ok(json!({ "fields": { "summary": NAME } })),
        ],
    ] {
        let board = Jira::new(Calling::answering(answers), a_scope());

        let error = board
            .ticket_filing(BRIEF)
            .expect_err("not the answer asked for");

        assert!(
            matches!(error, Error::Malformed { board: JIRA, .. }),
            "{error:?}"
        );
    }
}

#[test]
fn the_pre_flight_sends_no_mutation_and_reads_only_under_the_newer_api() {
    let board = Jira::new(
        Calling::answering([Ok(a_filable_project()), Ok(reachable())]),
        a_scope(),
    );

    let filing = board.ticket_filing(BRIEF).expect("the stand-in answered");

    assert!(matches!(filing, Filable::Ready(_)), "{filing:?}");
    for (method, path, body) in board.calls.made() {
        // The whole promise of this verb: a destination a cut cannot file into
        // is refused while the slice is still nothing, so every call it makes
        // is a read.
        assert_eq!(method, Method::GET, "{path}");
        assert!(path.starts_with("/rest/api/3/"), "{path}");
        assert!(!path.contains("/rest/api/2"), "{path}");
        assert_eq!(body, Value::Null, "{path}");
    }
}

/// The key the child create answers, which is the whole of how this board names
/// the issue it just filed: the id a link is written with, the identifier a cut
/// record holds and the key a person types are all this.
const CHILD: &str = "PAY-20";

/// The slice's second child, which is the one [`CHILD`] holds up: an edge is
/// written between two keys the creates answered, so the two halves of one are
/// never the same issue.
const HELD_UP: &str = "PAY-21";

/// One draft's title, as a slice hands it over: the summary a child carries.
const DRAFT: &str = "Read the Epic";

/// That draft's body, short enough for the ADF tree it becomes to be spelled
/// out beside it: a heading and a paragraph.
const DRAFT_BODY: &str = "## Goal\n\nBy its key.\n";

/// The draft every create below files. The project is the Epic's key, which is
/// what `planned.rs` takes from `fetch_brief`'s id on a Jira scope, and the
/// assignee is the `accountId` [`Board::viewer`] answered.
fn a_draft() -> NewTicket<'static> {
    NewTicket::new(DRAFT, DRAFT_BODY, BRIEF, LABEL, ACCOUNT)
}

/// The one request that files a child, as the stand-in records it. Spelled out
/// rather than built from `adf::document_for` and the module's own constants,
/// for the reason [`filed`] spells its own: a field added or dropped, the parent
/// moving, the type renamed or the converter's tree changing is a failure here
/// and not two expressions that moved together.
fn filed_child() -> (Method, String, Value) {
    (
        Method::POST,
        "/rest/api/3/issue".to_owned(),
        json!({
            "fields": {
                // The project is the record's key, as the brief's create is.
                "project": { "key": PROJECT },
                // Named because Jira requires it and takes no default from the
                // project: the one value on a child no draft decided.
                "issuetype": { "name": "Task" },
                // What makes the issue the Epic's child, so that the JQL
                // `parent = PAY-14` finds it afterwards.
                "parent": { "key": BRIEF },
                "summary": DRAFT,
                "labels": [LABEL],
                // Jira's `accountId`, which is the only way a user is addressed.
                "assignee": { "accountId": ACCOUNT },
                "description": {
                    "type": "doc",
                    "version": 1,
                    "content": [
                        {
                            "type": "heading",
                            "attrs": { "level": 2 },
                            "content": [{ "type": "text", "text": "Goal" }],
                        },
                        {
                            "type": "paragraph",
                            "content": [{ "type": "text", "text": "By its key." }],
                        },
                    ],
                },
            }
        }),
    )
}

/// The read of where one issue stands, by its key and for that field alone: the
/// read that follows a child's create, because a create answers an `id`, a `key`
/// and a `self` and says nothing about status, and the read a move that reached
/// nothing makes, because a transitions answer says where an issue can go and
/// never where it is.
fn status_of(key: &str) -> (Method, String, Value) {
    (
        Method::GET,
        format!("/rest/api/3/issue/{key}?fields=status"),
        Value::Null,
    )
}

/// A freshly filed child as that read answers it: the status the workflow opened
/// it in, and the category Jira files that status under.
fn landed(status: &str, category: &str) -> Value {
    json!({
        "id": "10050",
        "key": CHILD,
        "fields": { "status": { "name": status, "statusCategory": { "key": category } } },
    })
}

/// Where the moves an issue offers are read and where one of them is spent: the
/// same path, which is why both calls are built from this.
fn moves(key: &str) -> String {
    format!("/rest/api/3/issue/{key}/transitions")
}

/// The read of what the workflow offers from where the child stands.
fn offered(key: &str) -> (Method, String, Value) {
    (Method::GET, moves(key), Value::Null)
}

/// The transitions an issue offers, each by its own id and the status its `to`
/// lands in: what a move is chosen out of.
fn transitions(offers: &[(&str, &str, &str)]) -> Value {
    let transitions: Vec<Value> = offers
        .iter()
        .map(|(id, name, category)| {
            json!({
                "id": id,
                "name": format!("Move to {name}"),
                "to": { "name": name, "statusCategory": { "key": category } },
            })
        })
        .collect();

    json!({ "expand": "transitions", "transitions": transitions })
}

/// The move itself: a POST to the same path, carrying the id of the transition
/// and nothing else.
fn moved(key: &str, transition: &str) -> (Method, String, Value) {
    (
        Method::POST,
        moves(key),
        json!({ "transition": { "id": transition } }),
    )
}

/// A default workflow, from a status a ticket nobody has started belongs in.
fn a_default_workflow() -> Value {
    transitions(&[
        ("11", TO_DO, "new"),
        ("21", "In Progress", "indeterminate"),
        ("31", "Done", "done"),
    ])
}

#[test]
fn a_draft_is_filed_as_a_child_of_the_epic_carrying_the_label_the_assignee_and_adf() {
    let board = Jira::new(
        Calling::answering([
            Ok(
                json!({ "id": "10050", "key": CHILD, "self": "https://example.atlassian.net/rest/api/3/issue/10050" }),
            ),
            Ok(landed(TO_DO, "new")),
        ]),
        a_scope(),
    );

    let ticket = board
        .create_ticket(&a_draft(), &a_filing())
        .expect("the stand-in answered");

    // The key three ways: the id a link is written with, the identifier a cut
    // record holds and the URL a person opens. The numeric id beside it in the
    // answer is read by nothing.
    assert_eq!(ticket.id(), CHILD);
    assert_eq!(ticket.identifier(), CHILD);
    assert_eq!(ticket.url(), format!("{SITE}/browse/{CHILD}"));
    // One request to file it — the label and the assignee are in it, because
    // nothing here can take an issue back — and one read after it, which is the
    // only way to tell where the workflow opened the child. The child landed in
    // a `new` category, so nothing is spent moving it.
    assert_eq!(
        board.calls.made(),
        [filed_child(), status_of(CHILD)],
        "{:?}",
        board.calls.made()
    );
}

#[test]
fn the_parent_is_the_key_the_pre_flight_settled_and_not_the_one_the_draft_carries() {
    // Both name this Epic, and the filing's is the spelling the site itself
    // answered with: a child filed under a key Jira does not hold is a ticket
    // the JQL `parent = PAY-14` never finds.
    let board = Jira::new(
        Calling::answering([Ok(json!({ "key": CHILD })), Ok(landed(TO_DO, "new"))]),
        a_scope(),
    );

    board
        .create_ticket(
            &NewTicket::new(DRAFT, DRAFT_BODY, "pay-14", LABEL, ACCOUNT),
            &a_filing(),
        )
        .expect("the stand-in answered");

    assert_eq!(board.calls.made()[0], filed_child());
}

#[test]
fn a_child_carries_what_the_draft_decided_and_no_field_warlock_invented() {
    let board = Jira::new(
        Calling::answering([Ok(json!({ "key": CHILD })), Ok(landed(TO_DO, "new"))]),
        a_scope(),
    );

    board
        .create_ticket(&a_draft(), &a_filing())
        .expect("the stand-in answered");

    let (_, _, body) = board.calls.made().remove(0);
    let fields = body["fields"]
        .as_object()
        .expect("the create writes a `fields` object");
    let sent: Vec<&str> = fields.keys().map(String::as_str).collect();

    // No priority, no estimate, no sprint, no component and no custom field: a
    // ticket carries what a draft decided and nothing more. No status either —
    // Jira takes a created issue's status from the workflow and ignores one in
    // the body.
    assert_eq!(
        sent,
        [
            "project",
            "issuetype",
            "parent",
            "summary",
            "labels",
            "assignee",
            "description"
        ],
        "{sent:?}"
    );
}

#[test]
fn a_child_that_landed_outside_a_new_category_is_transitioned_into_one() {
    // A workflow whose initial transition opens an issue in progress, which is
    // the whole reason the status is read back at all.
    let board = Jira::new(
        Calling::answering([
            Ok(json!({ "key": CHILD })),
            Ok(landed("In Progress", "indeterminate")),
            Ok(a_default_workflow()),
            // A transition answers 204 with nothing after the headers.
            Ok(Value::Null),
        ]),
        a_scope(),
    );

    let ticket = board
        .create_ticket(&a_draft(), &a_filing())
        .expect("the stand-in answered");

    assert_eq!(ticket.identifier(), CHILD);
    // The transitions read, then the one whose `to` is the status the pre-flight
    // settled, posted by its id. A fifth call would panic the stand-in, which is
    // how "one request per mutation" is asserted.
    assert_eq!(
        board.calls.made(),
        [
            filed_child(),
            status_of(CHILD),
            offered(CHILD),
            moved(CHILD, "11")
        ],
        "{:?}",
        board.calls.made()
    );
}

#[test]
fn the_transition_is_chosen_by_the_status_name_and_never_by_its_category() {
    // Two transitions into the `new` category, which is the corner a choice by
    // category would get wrong: the name is what the pre-flight settled, so the
    // name is what decides. Spelled with different case and spacing, because the
    // word on this side was read off the project's statuses and the word on the
    // wire is the board's own.
    let board = Jira::new(
        Calling::answering([
            Ok(json!({ "key": CHILD })),
            Ok(landed("In Progress", "indeterminate")),
            Ok(transitions(&[
                ("41", "Backlog", "new"),
                ("11", "  to do  ", "new"),
            ])),
            Ok(Value::Null),
        ]),
        a_scope(),
    );

    board
        .create_ticket(&a_draft(), &a_filing())
        .expect("the stand-in answered");

    assert_eq!(board.calls.made()[3], moved(CHILD, "11"));
}

#[test]
fn a_workflow_offering_no_transition_into_that_status_posts_nothing_and_the_child_stands() {
    // The issue exists, it is the Epic's child and it carries the label: there
    // is no second way to ask for a status this workflow cannot reach, so the
    // cut goes on with the ticket it filed.
    let board = Jira::new(
        Calling::answering([
            Ok(json!({ "key": CHILD })),
            Ok(landed("In Progress", "indeterminate")),
            Ok(transitions(&[("31", "Done", "done")])),
        ]),
        a_scope(),
    );

    let ticket = board
        .create_ticket(&a_draft(), &a_filing())
        .expect("a child that cannot be moved is still a child");

    assert_eq!(ticket.identifier(), CHILD);
    assert_eq!(
        board.calls.made(),
        [filed_child(), status_of(CHILD), offered(CHILD)],
        "{:?}",
        board.calls.made()
    );
}

#[test]
fn a_child_create_jira_refuses_is_reported_in_jiras_words_and_is_not_retried() {
    let said = "parent: Issue type Task does not support a parent.";
    let board = Jira::new(
        Calling::answering([Err(Error::Refused {
            board: JIRA,
            message: said.to_owned(),
        })]),
        a_scope(),
    );

    let error = board
        .create_ticket(&a_draft(), &a_filing())
        .expect_err("it was refused");

    assert!(error.to_string().contains(said), "{error}");
    // One request and nothing read after it: nothing on this side can tell a
    // refusal that arrived before the issue existed from one that arrived
    // after, so the person decides.
    assert_eq!(
        board.calls.made(),
        [filed_child()],
        "the create was retried"
    );
}

#[test]
fn a_move_jira_refuses_is_reported_in_jiras_words_and_is_not_retried() {
    let said = "It is not on the appropriate workflow transition.";
    let board = Jira::new(
        Calling::answering([
            Ok(json!({ "key": CHILD })),
            Ok(landed("In Progress", "indeterminate")),
            Ok(a_default_workflow()),
            Err(Error::Refused {
                board: JIRA,
                message: said.to_owned(),
            }),
        ]),
        a_scope(),
    );

    let error = board
        .create_ticket(&a_draft(), &a_filing())
        .expect_err("the transition was refused");

    assert!(error.to_string().contains(said), "{error}");
    assert_eq!(board.calls.made().len(), 4, "{:?}", board.calls.made());
}

#[test]
fn a_child_create_answering_no_key_is_malformed_and_nothing_is_read_after_it() {
    let board = Jira::new(
        Calling::answering([Ok(json!({ "id": "10050" }))]),
        a_scope(),
    );

    let error = board
        .create_ticket(&a_draft(), &a_filing())
        .expect_err("not the answer asked for");

    assert!(
        matches!(error, Error::Malformed { board: JIRA, .. }),
        "{error:?}"
    );
    assert_eq!(board.calls.made(), [filed_child()]);
}

#[test]
fn a_status_read_with_no_fields_or_a_workflow_with_no_transitions_is_malformed() {
    for answers in [
        vec![Ok(json!({ "key": CHILD })), Ok(json!({ "key": CHILD }))],
        // A workflow always offers something from where an issue stands, so an
        // answer with no list in it is bytes a proxy decided to send.
        vec![
            Ok(json!({ "key": CHILD })),
            Ok(landed("In Progress", "indeterminate")),
            Ok(json!({ "expand": "transitions" })),
        ],
    ] {
        let board = Jira::new(Calling::answering(answers), a_scope());

        let error = board
            .create_ticket(&a_draft(), &a_filing())
            .expect_err("not the answer asked for");

        assert!(
            matches!(error, Error::Malformed { board: JIRA, .. }),
            "{error:?}"
        );
    }
}

#[test]
fn every_call_a_child_takes_goes_under_the_newer_api_and_only_the_mutations_post() {
    let board = Jira::new(
        Calling::answering([
            Ok(json!({ "key": CHILD })),
            Ok(landed("In Progress", "indeterminate")),
            Ok(a_default_workflow()),
            Ok(Value::Null),
        ]),
        a_scope(),
    );

    board
        .create_ticket(&a_draft(), &a_filing())
        .expect("the stand-in answered");

    for (method, path, body) in board.calls.made() {
        assert!(path.starts_with("/rest/api/3/"), "{path}");
        assert!(!path.contains("/rest/api/2"), "{path}");
        // A read sends no body at all, and the two that carry one are the
        // create and the move.
        if method == Method::GET {
            assert_eq!(body, Value::Null, "{path}");
        } else {
            assert_eq!(method, Method::POST, "{path}");
            assert_ne!(body, Value::Null, "{path}");
        }
    }
}

/// The one request that writes an edge, as the stand-in records it. Spelled out
/// rather than built from the module's own constants, for the reason
/// [`filed_child`] spells its own — and the two keys are written here in the one
/// place a swap of them is a failure: `inwardIssue` is the issue that blocks and
/// `outwardIssue` the one it holds up, so swapping the two compiles, answers the
/// same shape and inverts every dependency in the slice.
fn linked(blocker: &str, waiting: &str) -> (Method, String, Value) {
    (
        Method::POST,
        "/rest/api/3/issueLink".to_owned(),
        json!({
            // The one default link type that says a dependency, by Jira's own
            // name for it: a site that renamed it refuses the link in its own
            // words.
            "type": { "name": "Blocks" },
            "inwardIssue": { "key": blocker },
            "outwardIssue": { "key": waiting },
        }),
    )
}

/// The one request that says a note on the Epic: a POST to the same comment
/// collection the notes are read out of, carrying the body as the ADF a comment
/// goes up as.
///
/// Built through `adf::document_for` rather than spelled as a tree, where
/// [`filed_child`] spells its own out: what this has to say is that a note
/// leaves as a document and not as the Markdown it was written as, and the
/// converter is asserted in `adf.rs`'s own tests. That the tree reads back as
/// the note is asserted by the round trip below, through the reader that
/// matters.
fn said(key: &str, body: &str) -> (Method, String, Value) {
    (
        Method::POST,
        format!("/rest/api/3/issue/{key}/comment"),
        json!({ "body": document_for(body) }),
    )
}

#[test]
fn an_edge_is_one_blocks_link_with_the_blocker_as_the_inward_issue() {
    // A link create answers 201 with nothing after the headers.
    let board = Jira::new(Calling::answering([Ok(Value::Null)]), a_scope());

    let written = board
        .create_relation(CHILD, HELD_UP)
        .expect("the stand-in answered");

    // Nothing comes back and there is nothing to come back: the answer carries
    // no link id, and the one caller reads the error off this verb and never
    // the value.
    assert_eq!(written, "");
    // One POST to the link collection, both keys in the body and neither in the
    // path. A second call would panic the stand-in, which is how "no mutation
    // is retried" is asserted.
    assert_eq!(
        board.calls.made(),
        [linked(CHILD, HELD_UP)],
        "{:?}",
        board.calls.made()
    );
}

#[test]
fn which_key_is_inward_is_the_whole_of_an_edges_direction() {
    let board = Jira::new(
        Calling::answering([Ok(Value::Null), Ok(Value::Null)]),
        a_scope(),
    );

    board
        .create_relation(CHILD, HELD_UP)
        .expect("the stand-in answered");
    // The same two issues, the other way round, which is a different edge.
    board
        .create_relation(HELD_UP, CHILD)
        .expect("the stand-in answered");

    let made = board.calls.made();

    assert_eq!(made[0], linked(CHILD, HELD_UP));
    assert_eq!(made[1], linked(HELD_UP, CHILD));
    assert_ne!(
        made[0], made[1],
        "the direction reached nothing on the wire"
    );
}

#[test]
fn a_link_jira_turns_down_is_reported_in_jiras_words_and_is_not_retried() {
    let refused = "No issue link type with name 'Blocks' found.";
    let board = Jira::new(
        Calling::answering([Err(Error::Refused {
            board: JIRA,
            message: refused.to_owned(),
        })]),
        a_scope(),
    );

    let error = board
        .create_relation(CHILD, HELD_UP)
        .expect_err("the link was refused");

    assert!(error.to_string().contains(refused), "{error}");
    // One request and nothing after it: `cut::relate` reads the error off this
    // and reports it as one line, so every issue the slice filed stands and a
    // person fixes the edge on the board.
    assert_eq!(
        board.calls.made(),
        [linked(CHILD, HELD_UP)],
        "the link was retried"
    );
}

#[test]
fn a_note_is_one_comment_on_the_epic_carrying_the_body_as_adf() {
    // The two bodies `cut.rs` writes, which are one request apiece and differ
    // only in the prefix they start with: a slice that was cut and a slice
    // somebody said no to are both settled, and this is the whole record of
    // either.
    for note in [A_CUT, A_SKIP] {
        let board = Jira::new(
            Calling::answering([Ok(
                json!({ "id": "10100", "self": "https://example.atlassian.net/rest/api/3/issue/10042/comment/10100" }),
            )]),
            a_scope(),
        );

        let id = board
            .comment_on_brief(BRIEF, note)
            .expect("the stand-in answered");

        // Jira's own id for the comment, which no caller reads.
        assert_eq!(id, "10100", "{note}");
        assert_eq!(
            board.calls.made(),
            [said(BRIEF, note)],
            "{:?}",
            board.calls.made()
        );
    }
}

#[test]
fn the_notes_said_on_the_epic_are_the_ones_a_later_fetch_reads_back() {
    let writing = Jira::new(
        Calling::answering([Ok(json!({ "id": "10100" })), Ok(json!({ "id": "10101" }))]),
        a_scope(),
    );

    writing
        .comment_on_brief(BRIEF, A_CUT)
        .expect("the stand-in answered");
    writing
        .comment_on_brief(BRIEF, A_SKIP)
        .expect("the stand-in answered");

    // The very bodies that went up, handed back as the Epic's comments: the
    // round trip, and not a tree typed out by hand to match the reader.
    let comments: Vec<Value> = writing
        .calls
        .made()
        .into_iter()
        .enumerate()
        .map(|(index, (_, _, sent))| json!({ "id": format!("1010{index}"), "body": sent["body"] }))
        .collect();
    let board = Jira::new(
        Calling::answering([
            Ok(a_draftable_epic()),
            Ok(
                json!({ "startAt": 0, "maxResults": 100, "total": comments.len(), "comments": comments }),
            ),
        ]),
        a_scope(),
    );

    let brief = board
        .fetch_brief(BRIEF)
        .expect("the stand-in answered")
        .expect("the site holds the key");

    assert_eq!(brief.notes(), [A_CUT, A_SKIP]);
    // And read by the reader that decides: a slice one of these names is not
    // offered a second time, and a skip settles its slice the same way a cut
    // does.
    assert_eq!(
        cut::noted(brief.notes()),
        [
            (
                "read the epic".to_owned(),
                vec![CHILD.to_owned(), HELD_UP.to_owned()]
            ),
            ("read the notes".to_owned(), Vec::new()),
        ]
    );
}

#[test]
fn a_note_jira_turns_down_is_reported_in_jiras_words_and_is_not_retried() {
    let refused = "You do not have permission to comment on this issue.";
    let board = Jira::new(
        Calling::answering([Err(Error::Refused {
            board: JIRA,
            message: refused.to_owned(),
        })]),
        a_scope(),
    );

    let error = board
        .comment_on_brief(BRIEF, A_CUT)
        .expect_err("the comment was refused");

    assert!(error.to_string().contains(refused), "{error}");
    // A comment is not idempotent, so a retried note would record the same
    // slice twice on one Epic: the caller gets a line and the person decides.
    assert_eq!(
        board.calls.made(),
        [said(BRIEF, A_CUT)],
        "the note was said twice"
    );
}

#[test]
fn a_note_whose_answer_carries_no_id_is_still_a_note_that_landed() {
    // Jira answers a comment create with the comment it made, id and all. Bytes
    // that carry none are not a reason to report a note nobody can take back as
    // having failed: the next fetch reads it off the Epic and settles the slice.
    let board = Jira::new(Calling::answering([Ok(Value::Null)]), a_scope());

    let id = board
        .comment_on_brief(BRIEF, A_CUT)
        .expect("the note landed");

    assert_eq!(id, "");
}

#[test]
fn the_edge_and_the_note_are_one_post_each_under_the_newer_api() {
    let board = Jira::new(
        Calling::answering([Ok(Value::Null), Ok(json!({ "id": "10100" }))]),
        a_scope(),
    );

    board
        .create_relation(CHILD, HELD_UP)
        .expect("the stand-in answered");
    board
        .comment_on_brief(BRIEF, A_CUT)
        .expect("the stand-in answered");

    let made = board.calls.made();

    assert_eq!(made.len(), 2, "{made:?}");
    for (method, path, body) in made {
        assert_eq!(method, Method::POST, "{path}");
        assert!(path.starts_with("/rest/api/3/"), "{path}");
        assert!(!path.contains("/rest/api/2"), "{path}");
        assert_ne!(body, Value::Null, "{path}");
    }
}

/// A workflow holding the two statuses a choice by category could not tell
/// apart: both land in `indeterminate`, and an Epic a cut has just settled
/// jumping to review would be warlock saying its slices had been read.
fn two_indeterminate_moves() -> Value {
    transitions(&[
        ("51", "In Review", "indeterminate"),
        // Spelled in another case and with spacing around it, because the name
        // on this side is warlock's own word and the name on the wire is the
        // board's.
        ("21", "  in progress  ", "indeterminate"),
    ])
}

/// A workflow offering nothing but the way to done, which is a board that
/// cannot reach the status a settled cut asks for.
fn no_way_to_work() -> Value {
    transitions(&[("31", "Done", "done")])
}

#[test]
fn the_epic_is_moved_by_the_transition_whose_destination_carries_that_name() {
    let board = Jira::new(
        Calling::answering([
            Ok(a_default_workflow()),
            // A transition answers 204 with nothing after the headers.
            Ok(Value::Null),
        ]),
        a_scope(),
    );

    let settled = board
        .move_brief(BRIEF, IN_PROGRESS)
        .expect("the stand-in answered");

    assert!(settled, "the workflow offers a move there");
    // The moves the Epic offers read, then the one whose `to` is called that
    // posted by its id and nothing else in the body. A third call would panic
    // the stand-in, which is how "one request per mutation" is asserted.
    assert_eq!(
        board.calls.made(),
        [offered(BRIEF), moved(BRIEF, "21")],
        "{:?}",
        board.calls.made()
    );
    // And on a default workflow that is the `indeterminate` status, which is
    // where a cut leaves a brief every slice of is settled.
    assert_eq!(
        a_default_workflow()["transitions"][1]["to"],
        json!({ "name": IN_PROGRESS, "statusCategory": { "key": "indeterminate" } })
    );
}

#[test]
fn a_workflow_offering_no_move_of_that_name_answers_so_and_posts_nothing() {
    // An answer and not a failure: the tickets a cut filed exist whatever the
    // Epic's own column does next.
    let board = Jira::new(Calling::answering([Ok(no_way_to_work())]), a_scope());

    let settled = board
        .move_brief(BRIEF, IN_PROGRESS)
        .expect("no such status is an answer");

    assert!(!settled);
    assert_eq!(
        board.calls.made(),
        [offered(BRIEF)],
        "{:?}",
        board.calls.made()
    );
}

#[test]
fn the_epics_move_is_chosen_by_name_and_never_by_the_status_category() {
    let board = Jira::new(
        Calling::answering([Ok(two_indeterminate_moves()), Ok(Value::Null)]),
        a_scope(),
    );

    assert!(
        board
            .move_brief(BRIEF, IN_PROGRESS)
            .expect("the stand-in answered")
    );

    // `21` and never `51`: both land in `indeterminate`, so the name the caller
    // gave is the whole of what tells them apart.
    assert_eq!(board.calls.made()[1], moved(BRIEF, "21"));
}

#[test]
fn a_settled_cut_says_the_line_the_boards_own_answer_earned() {
    let moving = Jira::new(
        Calling::answering([Ok(a_default_workflow()), Ok(Value::Null)]),
        a_scope(),
    );
    let staying = Jira::new(Calling::answering([Ok(no_way_to_work())]), a_scope());

    let moved_on = cut::finish(&moving, BRIEF);
    let stayed = cut::finish(&staying, BRIEF);

    assert_eq!(
        moved_on,
        format!("every slice is settled, so the project moved to `{IN_PROGRESS}`")
    );
    // The very line a Linear workspace with no such column gets: a workflow
    // that cannot reach the status is reported and worked past.
    assert_eq!(
        stayed,
        format!(
            "every slice is settled, and the workspace has no project status called \
             `{IN_PROGRESS}`, so the project was not moved"
        )
    );
}

#[test]
fn an_epics_move_jira_refuses_is_reported_in_jiras_words_and_is_not_retried() {
    let said = "It is not on the appropriate workflow transition.";
    let board = Jira::new(
        Calling::answering([
            Ok(a_default_workflow()),
            Err(Error::Refused {
                board: JIRA,
                message: said.to_owned(),
            }),
        ]),
        a_scope(),
    );

    let error = board
        .move_brief(BRIEF, IN_PROGRESS)
        .expect_err("the transition was refused");

    assert!(error.to_string().contains(said), "{error}");
    // One POST and no second: the caller words the failure and the person
    // decides, as it does for a move of a freshly filed child.
    assert_eq!(
        board.calls.made(),
        [offered(BRIEF), moved(BRIEF, "21")],
        "the move was retried"
    );
}

#[test]
fn an_epics_moves_read_with_no_transitions_is_malformed_and_nothing_is_posted() {
    // A workflow always offers something from where an issue stands, so an
    // answer with no list in it is bytes a proxy decided to send — not an Epic
    // with nowhere to go, which is the `false` above.
    let board = Jira::new(
        Calling::answering([Ok(json!({ "expand": "transitions" }))]),
        a_scope(),
    );

    let error = board
        .move_brief(BRIEF, IN_PROGRESS)
        .expect_err("not the answer asked for");

    assert!(
        matches!(error, Error::Malformed { board: JIRA, .. }),
        "{error:?}"
    );
    assert_eq!(board.calls.made(), [offered(BRIEF)]);
}

/// The key of one ticket a cut filed under the Epic, which is both of the names
/// the queue answers it by: a person types this, and every path a move or a
/// comment is addressed by is built out of it.
const TICKET: &str = "PAY-31";

/// That ticket's summary, which is its title in the queue.
const TITLE: &str = "Read the queue through `search/jql`";

/// And what it says, as the Markdown it was filed as: a sub-task is handed the
/// description, so the assertion below is the round trip through the ADF the
/// site holds and not a tree typed out to match the reader.
const TICKET_BODY: &str = "## Goal\n\nRead the queue.\n";

/// The status a board may spell a ticket somebody is on with, which is no word
/// warlock knows: the category is the judgement and the name is what rides back.
const IN_DEV: &str = "In dev on a branch";

/// An issue holding the queued ticket up, which is no ticket of this queue: a
/// blocker is read whoever holds it, whatever label it carries and whatever
/// status it stands in.
const BLOCKER: &str = "PAY-30";

/// One ticket as a queue read answers it: the key, the summary, the status with
/// the category Jira files it under, the description as an ADF tree, the empty
/// link list of a ticket nothing is linked to, and the priority field — left off
/// entirely for a ticket nobody ranked.
fn a_ticket(status: &str, category: &str, priority: Option<&str>) -> Value {
    let mut fields = json!({
        "summary": TITLE,
        "status": { "name": status, "statusCategory": { "key": category } },
        "description": document_for(TICKET_BODY),
        "issuelinks": [],
    });

    if let Some(priority) = priority {
        // The id rides along because a real answer carries one, and nothing
        // reads it: a priority scheme is the site's to edit, so the ids are one
        // site's own numbering.
        fields["priority"] = json!({ "id": "3", "name": priority });
    }

    json!({ "id": "10031", "key": TICKET, "fields": fields })
}

/// A ticket a queue read would otherwise have parsed, with the field at that
/// JSON pointer taken off it.
fn without(pointer: &str) -> Value {
    let mut ticket = a_ticket(TO_DO, "new", Some("High"));
    // The pointer's last step is the field to take and everything before it
    // addresses the object holding it, which is the whole document for a field
    // on the issue itself.
    let (parent, field) = pointer
        .rsplit_once('/')
        .expect("a JSON pointer, which begins with a slash");

    ticket
        .pointer_mut(parent)
        .and_then(Value::as_object_mut)
        .expect("a ticket to take a field off")
        .remove(field)
        .expect("the field was there to take");

    ticket
}

/// One page of a queue's answer, and the token to read on from when a page
/// follows this one.
fn holding(tickets: &[Value], next: Option<&str>) -> Value {
    let mut answer = json!({ "issues": tickets });

    if let Some(token) = next {
        answer["nextPageToken"] = json!(token);
    }

    answer
}

/// One page of the queue's read, as the stand-in records a call. The JQL is
/// spelled out rather than built from the module's own pieces, for the reason
/// [`asked`] spells its own: a filter dropped, the exclusion changing, the order
/// clause leaving the search or the account not reaching it is a failure here
/// and not two expressions that moved together.
fn pulling(token: Option<&str>) -> (Method, String, Value) {
    let mut body = json!({
        // The project key and the label are the record's — the verb is handed
        // neither — and the account is the whole of what the caller passes in.
        "jql": format!(
            r#"project = {PROJECT} AND labels = "{LABEL}" AND assignee = "{ACCOUNT}" AND statusCategory != done ORDER BY priority DESC, Rank ASC"#
        ),
        // Asked for by name, `issuelinks` among them: a search with no `fields`
        // carries every custom field the site has, and a search that left the
        // links off would answer a queue in which every held-up ticket is ready.
        "fields": ["summary", "status", "description", "priority", "issuelinks"],
        "maxResults": 100,
    });

    if let Some(token) = token {
        body["nextPageToken"] = json!(token);
    }

    (Method::POST, "/rest/api/3/search/jql".to_owned(), body)
}

#[test]
fn a_queued_ticket_carries_the_key_as_both_names_with_its_summary_status_and_body() {
    let board = Jira::new(
        Calling::answering([Ok(holding(&[a_ticket(TO_DO, "new", Some("High"))], None))]),
        a_scope(),
    );

    let queue = board.scope_queue(ACCOUNT).expect("the stand-in answered");

    let [ticket] = queue.issues() else {
        panic!("one ticket: {:?}", queue.issues())
    };
    // The key twice over: the id a move and a comment are addressed by and the
    // identifier a person types are one value on this board, where Linear has
    // two.
    assert_eq!(ticket.id(), TICKET);
    assert_eq!(ticket.identifier(), TICKET);
    assert_eq!(ticket.title(), TITLE);
    // The description back through the ADF reader, which is the Markdown it was
    // filed as: this is what a sub-task session is handed as the parent ticket.
    assert_eq!(ticket.description(), TICKET_BODY);
    // The board's own status word, which is only ever printed, beside the
    // category every judgement is made on.
    assert_eq!(ticket.state(), TO_DO);
    assert_eq!(ticket.state_type(), Category::New);
    assert_eq!(ticket.priority(), Priority::High);
    // And nothing holds it up, because the links the answer carried are none:
    // an unlinked ticket is ready, and the field was asked for to know that.
    assert!(ticket.blockers().is_empty(), "{:?}", ticket.blockers());
    // One request, a POST with the JQL in its body, narrowed to the project, the
    // label and the account — so a ticket of another scope, and one somebody
    // else holds, never arrives.
    assert_eq!(board.calls.made(), [pulling(None)]);
}

#[test]
fn the_queue_is_ordered_by_priority_then_rank_and_excludes_the_done_category() {
    let board = Jira::new(Calling::answering([Ok(holding(&[], None))]), a_scope());

    let queue = board
        .scope_queue(ACCOUNT)
        .expect("an empty queue is an answer");

    assert!(queue.issues().is_empty(), "{:?}", queue.issues());

    let jql = board.calls.made()[0].2["jql"]
        .as_str()
        .expect("the search carries a JQL string")
        .to_owned();

    // The board's own order, asked for rather than taken as read: a search with
    // no clause comes back in whatever order the site settles on today.
    assert!(jql.ends_with(" ORDER BY priority DESC, Rank ASC"), "{jql}");
    // And the exclusion is by category, which is the vocabulary both boards
    // share: the word left out is the one `Category::of` reads as settled, so no
    // status *name* is in the search at all — `Done` on this board is `Shipped`
    // on the next, and only the category tells either from live work.
    assert!(jql.contains("statusCategory != done"), "{jql}");
    assert_eq!(Category::of("done"), Category::Settled);
    for name in ["Done", "Shipped", "Closed", "Cancelled"] {
        assert!(!jql.contains(name), "the status `{name}` is in: {jql}");
    }
}

#[test]
fn a_tickets_category_is_its_status_categorys_key_and_the_boards_word_rides_back() {
    for (status, category, kind) in [
        (TO_DO, "new", Category::New),
        (IN_DEV, "indeterminate", Category::Started),
        // Left out by the search, and read the same way when one arrives all the
        // same: nothing here may answer a category the judgement downstream
        // would not have made of the word on the wire.
        ("Shipped to production", "done", Category::Settled),
    ] {
        let board = Jira::new(
            Calling::answering([Ok(holding(
                &[a_ticket(status, category, Some("High"))],
                None,
            ))]),
            a_scope(),
        );

        let queue = board.scope_queue(ACCOUNT).expect("the stand-in answered");

        assert_eq!(queue.issues()[0].state(), status, "{category}");
        assert_eq!(queue.issues()[0].state_type(), kind, "{category}");
    }
}

#[test]
fn the_five_priority_names_are_the_ranks_and_anything_else_is_no_priority() {
    for (name, rank) in [
        (Some("Highest"), Priority::Urgent),
        (Some("High"), Priority::High),
        (Some("Medium"), Priority::Medium),
        (Some("Low"), Priority::Low),
        // The bottom of the five lands where nothing at all does: a ticket
        // somebody ranked lowest is not worked ahead of one nobody ranked.
        (Some("Lowest"), Priority::None),
        // A site that renamed its scheme or added a sixth priority.
        (Some("Blocker"), Priority::None),
        // And a project whose scheme leaves the field off a ticket nobody
        // ranked, which is not a ticket to read as urgent.
        (None, Priority::None),
    ] {
        let board = Jira::new(
            Calling::answering([Ok(holding(&[a_ticket(TO_DO, "new", name)], None))]),
            a_scope(),
        );

        let queue = board.scope_queue(ACCOUNT).expect("the stand-in answered");

        assert_eq!(queue.issues()[0].priority(), rank, "{name:?}");
    }
}

#[test]
fn a_ticket_nobody_described_is_the_empty_description_a_subtask_is_handed() {
    let board = Jira::new(
        Calling::answering([Ok(holding(&[without("/fields/description")], None))]),
        a_scope(),
    );

    let queue = board
        .scope_queue(ACCOUNT)
        .expect("a ticket with nothing more to say than its title is a ticket");

    assert_eq!(queue.issues()[0].description(), "");
}

#[test]
fn the_queue_reads_every_page_by_the_token_the_page_before_it_gave() {
    let board = Jira::new(
        Calling::answering([
            Ok(holding(
                &[a_ticket(TO_DO, "new", Some("High"))],
                Some("page-2"),
            )),
            Ok(holding(&[a_ticket(IN_DEV, "indeterminate", None)], None)),
        ]),
        a_scope(),
    );

    let queue = board.scope_queue(ACCOUNT).expect("the stand-in answered");

    assert_eq!(queue.issues().len(), 2, "{:?}", queue.issues());
    // One request per page, the second carrying the token the first answered,
    // and the page with no token is the last.
    assert_eq!(board.calls.made(), [pulling(None), pulling(Some("page-2"))]);
}

#[test]
fn a_queue_jira_refuses_is_reported_in_jiras_words_with_its_order_clause_kept() {
    let said = "Field 'Rank' does not exist or you do not have permission to view it.";
    let board = Jira::new(
        Calling::answering([Err(Error::Refused {
            board: JIRA,
            message: said.to_owned(),
        })]),
        a_scope(),
    );

    let error = board
        .scope_queue(ACCOUNT)
        .expect_err("the search was refused");

    assert!(error.to_string().contains(said), "{error}");
    // One request and no second: a search asked again without the clause the
    // site turned down would answer a queue in some other order, which is a
    // different queue and not a repaired one.
    assert_eq!(
        board.calls.made(),
        [pulling(None)],
        "the search was retried or asked again"
    );
}

#[test]
fn a_queue_page_that_failed_fails_the_read_rather_than_answering_the_ones_before_it() {
    // Half a queue is a queue a pull would choose out of believing it whole: the
    // ticket that should have been taken is the one on the page that never
    // arrived, and nothing downstream can tell that from an empty board.
    let board = Jira::new(
        Calling::answering([
            Ok(holding(
                &[a_ticket(TO_DO, "new", Some("High"))],
                Some("page-2"),
            )),
            Err(Error::Status {
                board: JIRA,
                code: 502,
            }),
        ]),
        a_scope(),
    );

    let error = board
        .scope_queue(ACCOUNT)
        .expect_err("a page did not arrive");

    assert!(
        matches!(error, Error::Status { code: 502, .. }),
        "{error:?}"
    );
    assert_eq!(board.calls.made().len(), 2, "{:?}", board.calls.made());
}

#[test]
fn a_queue_answer_missing_any_field_a_ticket_is_read_out_of_is_malformed() {
    for pointer in [
        "/key",
        "/fields",
        "/fields/summary",
        "/fields/status",
        "/fields/status/name",
        "/fields/status/statusCategory",
    ] {
        let board = Jira::new(
            Calling::answering([Ok(holding(&[without(pointer)], None))]),
            a_scope(),
        );

        let error = board
            .scope_queue(ACCOUNT)
            .expect_err("not the answer asked for");

        // Parsed and not described: a ticket a field is missing from is bytes a
        // proxy decided to send, and a queue read that guessed at one would hand
        // a pull a ticket nobody filed.
        assert!(
            matches!(error, Error::Malformed { board: JIRA, .. }),
            "{pointer}: {error:?}"
        );
    }
}

/// The queued ticket, carrying those links in its `issuelinks`: the field the
/// read asks for beside the four the ticket itself is read out of.
fn with_links(links: &[Value]) -> Value {
    let mut ticket = a_ticket(TO_DO, "new", Some("High"));

    ticket["fields"]["issuelinks"] = json!(links);

    ticket
}

/// One of the queued ticket's links, of that type, with the far side under that
/// field — which is the whole of a link's direction and so is never defaulted
/// by a test. The phrases ride along because a real answer carries them and
/// nothing reads them: `Blocks` is matched by the type's name, never by the
/// sentence a board shows beside it.
fn link(relation: &str, side: &str, far: Value) -> Value {
    let mut link = json!({
        "id": "10101",
        "type": { "id": "10000", "name": relation, "inward": "is blocked by", "outward": "blocks" },
    });

    link[side] = far;

    link
}

/// The far side of a link as Jira hands it back on an issue: the key, the id
/// beside it that nothing reads, and the four fields a link carries its far
/// side's — no `assignee` among them, which is why no blocker read off this
/// board says whose it is.
fn far_side(key: &str, status: &str, category: &str) -> Value {
    json!({
        "id": "10020",
        "key": key,
        "fields": {
            "summary": "Write the thing this one waits on",
            "status": { "name": status, "statusCategory": { "key": category } },
            "priority": { "id": "3", "name": "High" },
            "issuetype": { "name": "Task" },
        },
    })
}

#[test]
fn a_blocker_is_read_off_the_very_side_a_link_create_writes_one_into() {
    let writer = Jira::new(Calling::answering([Ok(Value::Null)]), a_scope());

    // `CHILD` blocks `TICKET`, written the way a cut writes a slice's edge.
    writer
        .create_relation(CHILD, TICKET)
        .expect("the stand-in answered");

    let made = writer.calls.made();
    let [(_, _, wrote)] = made.as_slice() else {
        panic!("one request: {made:?}")
    };
    // Which field holds the blocker is not named by this test: it is whichever
    // one the create put `CHILD` in, and the link read below is built by
    // putting that same value back under that same field. A swap on either
    // side fails here — a create that wrote the held-up issue inward has
    // `TICKET` reported as its own blocker, and a read that took the other
    // field finds no blocker at all.
    let (side, far) = wrote
        .as_object()
        .expect("a link create writes an object")
        .iter()
        .find(|(_, issue)| issue["key"] == json!(CHILD))
        .expect("the create wrote the blocking issue under some field");
    let blocking = far["key"].as_str().expect("the key the create wrote");
    let reader = Jira::new(
        Calling::answering([Ok(holding(
            &[with_links(&[link(
                "Blocks",
                side,
                far_side(blocking, IN_DEV, "indeterminate"),
            )])],
            None,
        ))]),
        a_scope(),
    );

    let queue = reader.scope_queue(ACCOUNT).expect("the stand-in answered");

    let [held_up] = queue.issues() else {
        panic!("one ticket: {:?}", queue.issues())
    };
    assert_eq!(held_up.identifier(), TICKET);

    let [blocker] = held_up.blockers() else {
        panic!("one blocker: {:?}", held_up.blockers())
    };

    assert_eq!(blocker.identifier(), CHILD);
    // The category of the blocker's own status, which is what says whether it
    // is still in the way, and the board's word for it is never read.
    assert_eq!(blocker.state_type(), Category::Started);
    // Unread rather than nobody, because the far side of a link carries no
    // `assignee`: a queue's refusal names a Jira blocker by its key alone
    // rather than spending a read per link to print one word.
    assert_eq!(blocker.holder(), &Holder::Unread);
    // And the links cost no request of their own: they ride back in the fields
    // the queue's own search asked for.
    assert_eq!(reader.calls.made(), [pulling(None)]);
}

#[test]
fn only_a_blocks_link_holds_a_ticket_up_and_only_from_the_side_it_arrives_on() {
    let dropped = [
        // This ticket blocks that one, which holds nothing up on this side.
        link("Blocks", "outwardIssue", far_side(CHILD, TO_DO, "new")),
        // And the link types that say no dependency at all, whichever side of
        // either the far issue arrives on.
        link("Relates", "inwardIssue", far_side(CHILD, TO_DO, "new")),
        link("Duplicate", "inwardIssue", far_side(CHILD, TO_DO, "new")),
        link("Cloners", "outwardIssue", far_side(CHILD, TO_DO, "new")),
    ];

    for link in &dropped {
        let board = Jira::new(
            Calling::answering([Ok(holding(&[with_links(std::slice::from_ref(link))], None))]),
            a_scope(),
        );

        let queue = board.scope_queue(ACCOUNT).expect("the stand-in answered");

        assert!(
            queue.issues()[0].blockers().is_empty(),
            "{link}: {:?}",
            queue.issues()[0].blockers()
        );
    }

    // The same four beside the one link that does hold it up, so the blocker is
    // picked out of a list rather than read because it is the only entry.
    let mut links = dropped.to_vec();
    links.push(link(
        "Blocks",
        "inwardIssue",
        far_side(BLOCKER, TO_DO, "new"),
    ));
    let board = Jira::new(
        Calling::answering([Ok(holding(&[with_links(&links)], None))]),
        a_scope(),
    );

    let queue = board.scope_queue(ACCOUNT).expect("the stand-in answered");

    let [blocker] = queue.issues()[0].blockers() else {
        panic!("one blocker: {:?}", queue.issues()[0].blockers())
    };

    assert_eq!(blocker.identifier(), BLOCKER);
    assert_eq!(blocker.state_type(), Category::New);
}

#[test]
fn a_blocker_arrives_whatever_the_queues_own_filters_would_have_dropped() {
    // A ticket is held up by whatever blocks it: an issue of another project,
    // one somebody is on, and one already finished — none of which the queue's
    // own four clauses would have let through. What a blocker's category means
    // is choosing's judgement to make and not this read's, so all three arrive.
    let blockers = [
        far_side("OPS-9", TO_DO, "new"),
        far_side(BLOCKER, IN_DEV, "indeterminate"),
        far_side(CHILD, "Shipped to production", "done"),
    ];
    let board = Jira::new(
        Calling::answering([Ok(holding(
            &[with_links(
                &blockers
                    .iter()
                    .map(|far| link("Blocks", "inwardIssue", far.clone()))
                    .collect::<Vec<_>>(),
            )],
            None,
        ))]),
        a_scope(),
    );

    let queue = board.scope_queue(ACCOUNT).expect("the stand-in answered");

    let read: Vec<_> = queue.issues()[0]
        .blockers()
        .iter()
        .map(|blocker| (blocker.identifier(), blocker.state_type()))
        .collect();

    assert_eq!(
        read,
        [
            ("OPS-9", Category::New),
            (BLOCKER, Category::Started),
            (CHILD, Category::Settled),
        ]
    );
    // One request, and no second one narrowed any further: the links were in it.
    assert_eq!(board.calls.made(), [pulling(None)]);
}

#[test]
fn a_ticket_whose_answer_carries_no_links_is_a_ticket_nothing_holds_up() {
    // A ticket linked to nothing: whether a site spells that an empty list or
    // leaves the field out of the answer, it is not a ticket to refuse — the
    // field was asked for by name, so there is nothing more to ask.
    for ticket in [with_links(&[]), without("/fields/issuelinks")] {
        let board = Jira::new(
            Calling::answering([Ok(holding(&[ticket], None))]),
            a_scope(),
        );

        let queue = board
            .scope_queue(ACCOUNT)
            .expect("a ticket nothing links to is a ticket");

        assert!(
            queue.issues()[0].blockers().is_empty(),
            "{:?}",
            queue.issues()[0].blockers()
        );
    }
}

#[test]
fn a_link_answer_missing_any_field_a_blocker_is_read_out_of_is_malformed() {
    let blocks = link("Blocks", "inwardIssue", far_side(BLOCKER, TO_DO, "new"));

    for pointer in [
        // A link carrying no type is an unreadable answer and not one more link
        // to drop: dropping it quietly is how a held-up ticket looks ready.
        "/type",
        "/type/name",
        "/inwardIssue/key",
        // And a blocker whose category cannot be read is not a blocker to guess
        // at, for the reason a ticket's own status is not guessed at either: the
        // guess is about whether the work can start.
        "/inwardIssue/fields",
        "/inwardIssue/fields/status",
        "/inwardIssue/fields/status/statusCategory",
    ] {
        let mut link = blocks.clone();
        let (parent, field) = pointer
            .rsplit_once('/')
            .expect("a JSON pointer, which begins with a slash");

        link.pointer_mut(parent)
            .and_then(Value::as_object_mut)
            .expect("a link to take a field off")
            .remove(field)
            .expect("the field was there to take");

        let board = Jira::new(
            Calling::answering([Ok(holding(&[with_links(&[link])], None))]),
            a_scope(),
        );

        let error = board
            .scope_queue(ACCOUNT)
            .expect_err("not the answer asked for");

        assert!(
            matches!(error, Error::Malformed { board: JIRA, .. }),
            "{pointer}: {error:?}"
        );
    }

    // And a links field that is no list at all, which is the one way its
    // absence is not read as a ticket with no links.
    let mut ticket = with_links(&[]);
    ticket["fields"]["issuelinks"] = json!("none");
    let board = Jira::new(
        Calling::answering([Ok(holding(&[ticket], None))]),
        a_scope(),
    );

    let error = board
        .scope_queue(ACCOUNT)
        .expect_err("not the answer asked for");

    assert!(
        matches!(error, Error::Malformed { board: JIRA, .. }),
        "{error:?}"
    );
}

/// The number a caller split off `PAY-31`, which is the whole of what the
/// named read is told: the prefix was checked against the scope before anything
/// was asked, so the project the key is built from can only come off the record.
const NUMBER: u64 = 31;

/// The name the site has for the account the key authenticates as: the half of
/// an assignee a refusal prints, where the `accountId` beside it is the half a
/// check is made on.
const HOLDER: &str = "Nobody At All";

/// One named ticket as the issue read answers it: the very ticket a queue read
/// would have answered, plus the three facts the queue answered with a filter.
fn a_named_ticket(project: &str, labels: Value, assignee: Value) -> Value {
    let mut issue = a_ticket(TO_DO, "new", Some("High"));

    // The id rides along because a real answer carries one, and nothing reads
    // it: a scope record names a project by its key.
    issue["fields"]["project"] = json!({ "id": "10000", "key": project });
    issue["fields"]["labels"] = labels;
    issue["fields"]["assignee"] = assignee;
    issue
}

/// That ticket inside the scope: the record's own project, the record's label
/// among the ones it carries, and the account the key belongs to on it.
fn a_ticket_of_this_scope() -> Value {
    a_named_ticket(
        PROJECT,
        json!([LABEL, "backend"]),
        json!({ "accountId": ACCOUNT, "displayName": HOLDER }),
    )
}

/// A named ticket the read would otherwise have parsed, with the field at that
/// JSON pointer taken off it.
fn lacking(pointer: &str) -> Value {
    let mut issue = a_ticket_of_this_scope();
    let (parent, field) = pointer
        .rsplit_once('/')
        .expect("a JSON pointer, which begins with a slash");

    issue
        .pointer_mut(parent)
        .and_then(Value::as_object_mut)
        .expect("a ticket to take a field off")
        .remove(field)
        .expect("the field was there to take");

    issue
}

/// The named read, as the stand-in records it: a GET on the issue itself, with
/// the key and the fields spelled out rather than built from the module's own
/// pieces — for the reason [`asked`] spells its own. A filter arriving in the
/// path, a field dropping out of the list, the key being built from anything
/// but the record's project, or the read becoming a search is a failure here
/// and not two expressions that moved together.
fn by_key(key: &str) -> (Method, String, Value) {
    (
        Method::GET,
        format!(
            "/rest/api/3/issue/{key}\
             ?fields=summary,status,description,priority,issuelinks,project,labels,assignee"
        ),
        Value::Null,
    )
}

/// The board reading that answer, and the ticket it answered with.
fn named(issue: Value) -> (Jira<Calling>, crate::board::NamedTicket) {
    let board = Jira::new(Calling::answering([Ok(issue)]), a_scope());
    let found = board
        .named_ticket(NUMBER)
        .expect("the stand-in answered")
        .expect("the stand-in answered with the ticket");

    (board, found)
}

#[test]
fn a_named_ticket_is_read_by_the_key_the_records_project_and_the_number_make() {
    let (board, found) = named(a_ticket_of_this_scope());

    // The key the record's project and the number make, which is the whole
    // address: `PAY-31` and not a search for a ticket numbered 31.
    assert_eq!(found.issue().identifier(), TICKET);
    // One request, a GET with no body, and a second would panic the stand-in.
    assert_eq!(board.calls.made(), [by_key(TICKET)]);
}

#[test]
fn the_named_read_carries_none_of_the_queues_filters() {
    let (board, _) = named(a_ticket_of_this_scope());

    let made = board.calls.made();
    let [(method, path, body)] = made.as_slice() else {
        panic!("one request: {made:?}")
    };
    let (addressed, query) = path.split_once('?').expect("the fields ride in the path");

    assert_eq!(*method, Method::GET);
    assert_eq!(*body, Value::Null);
    assert_eq!(addressed, format!("/rest/api/3/issue/{TICKET}"));
    // The fields asked for and nothing else: no second query parameter is a
    // place a filter could be hiding.
    assert!(query.starts_with("fields="), "{query}");
    assert!(!query.contains('&'), "{query}");
    // Label, project and assignee are read as fields and never filtered on,
    // which is the whole point of this read: a ticket dropped by a filter
    // cannot say which filter dropped it, so the three arrive as facts to be
    // checked. The field names are in the path; the record's label and the
    // caller's account — the values a filter would be written against — are
    // not.
    assert!(!path.contains(LABEL), "the label is filtered on: {path}");
    assert!(
        !path.contains(ACCOUNT),
        "the account is filtered on: {path}"
    );
    // And no status clause, for the reason the state filter is off the Linear
    // arm's own named read: a ticket that shipped last week is absent from the
    // queue too, and refusing it as another project's work would be a lie.
    assert!(!path.contains("statusCategory"), "{path}");
    assert!(!path.contains("ORDER BY"), "{path}");
    // Not a search at all, so none of the queue's JQL can reach it.
    assert!(!path.contains("jql"), "{path}");
    assert!(!path.contains("search"), "{path}");
}

#[test]
fn a_named_ticket_is_the_same_value_a_queue_would_have_given() {
    let queued = Jira::new(
        Calling::answering([Ok(holding(&[a_ticket(TO_DO, "new", Some("High"))], None))]),
        a_scope(),
    );
    let chosen = queued.scope_queue(ACCOUNT).expect("the stand-in answered");

    let (_, found) = named(a_ticket_of_this_scope());

    // Parsed by the queue's own function, so a named ticket and a chosen one
    // are one type and the rules over them cannot drift.
    assert_eq!(found.issue(), &chosen.issues()[0]);
    assert_eq!(found.issue().title(), TITLE);
    assert_eq!(found.issue().description(), TICKET_BODY);
    assert_eq!(found.issue().state(), TO_DO);
    assert_eq!(found.issue().state_type(), Category::New);
    assert_eq!(found.issue().priority(), Priority::High);
}

#[test]
fn a_named_ticket_carries_the_project_labels_and_assignee_the_queue_filtered_on() {
    let (_, found) = named(a_ticket_of_this_scope());

    // The project key and not its id: a scope record names a project `PAY`, so
    // the two are comparable without resolving either.
    assert_eq!(found.team(), PROJECT);
    assert_eq!(found.labels(), [LABEL, "backend"]);
    assert_eq!(found.assignee(), Some(&Assignee::new(ACCOUNT, HOLDER)));
    // The id is what says whether a ticket is yours, and the name is what a
    // refusal prints: two people on a site can share a display name.
    assert_eq!(found.assignee().map(Assignee::id), Some(ACCOUNT));
    assert_eq!(found.assignee().map(Assignee::name), Some(HOLDER));
}

#[test]
fn the_three_facts_are_read_off_the_answer_and_never_off_the_record() {
    // The site answering about another project, with neither the record's label
    // nor its account on it: everything a check is made against is on the wire,
    // so a reader that filled any of the three in from the record would answer
    // the check with the request.
    let (_, found) = named(a_named_ticket(
        "ENG",
        json!(["area/web"]),
        json!({ "accountId": "5b10a2844c20165700ede000", "displayName": "Somebody Else" }),
    ));

    assert_eq!(found.team(), "ENG");
    assert_eq!(found.labels(), ["area/web"]);
    assert_eq!(
        found.assignee().map(Assignee::name),
        Some("Somebody Else"),
        "the holder is the site's and not the key's"
    );
}

#[test]
fn a_ticket_with_no_labels_and_nobody_on_it_is_an_ordinary_answer() {
    let (_, found) = named(a_named_ticket(PROJECT, json!([]), Value::Null));

    // Neither is malformed: both are refusals the caller words, not answers
    // this module cannot read.
    assert_eq!(found.labels(), [] as [String; 0]);
    assert_eq!(found.assignee(), None);
}

#[test]
fn a_ticket_that_shipped_is_still_answered() {
    let mut issue = a_ticket_of_this_scope();
    issue["fields"]["status"] = json!({
        "name": "Shipped to production",
        "statusCategory": { "key": "done" },
    });

    let (_, found) = named(issue);

    // No status filter is applied here on purpose: a finished ticket is absent
    // from the queue too, and a refusal calling that "not in your project"
    // would be a lie about the board. It comes back, and the caller refuses it
    // for the fact that is true.
    assert_eq!(found.issue().state(), "Shipped to production");
    assert_eq!(found.issue().state_type(), Category::Settled);
}

#[test]
fn a_key_the_site_has_no_issue_for_is_an_absence_rather_than_an_error() {
    let board = Jira::new(
        Calling::answering([Err(Error::Absent {
            board: JIRA,
            message: Some(
                "Issue does not exist or you do not have permission to see it.".to_owned(),
            ),
        })]),
        a_scope(),
    );

    // A number the project has not reached, as the Linear arm answers one: the
    // caller has its own words for a ticket that is not there, and a 404 is not
    // a failure to report in Jira's.
    assert_eq!(
        board
            .named_ticket(9_999)
            .expect("a 404 is an answer to this question"),
        None
    );
    assert_eq!(board.calls.made(), [by_key("PAY-9999")]);
}

#[test]
fn a_named_read_jira_refuses_is_reported_in_jiras_words_and_is_not_retried() {
    let said = "You do not have the permission to see the specified issue.";
    let board = Jira::new(
        Calling::answering([Err(Error::Refused {
            board: JIRA,
            message: said.to_owned(),
        })]),
        a_scope(),
    );

    let error = board
        .named_ticket(NUMBER)
        .expect_err("a refusal is not an absence");

    assert!(error.to_string().contains(said), "{error}");
    // One request and no second: nothing here asks again to repair a refusal.
    assert_eq!(board.calls.made(), [by_key(TICKET)], "the read was retried");
}

#[test]
fn a_named_answer_missing_any_fact_a_refusal_names_is_malformed() {
    let mut answers: Vec<Value> = [
        // The three the queue answered with a filter: a missing one is an
        // answer no check can be decided from, so it is never read as an
        // absence.
        "/fields/project",
        "/fields/project/key",
        "/fields/labels",
        "/fields/assignee",
        "/fields/assignee/accountId",
        "/fields/assignee/displayName",
        // And the queue's own fields, since the ticket is parsed by its
        // function.
        "/key",
        "/fields/status",
    ]
    .into_iter()
    .map(lacking)
    .collect();

    // A labels field that is no list at all, which is the one way its absence
    // is not read as a ticket carrying none.
    let mut scrambled = a_ticket_of_this_scope();
    scrambled["fields"]["labels"] = json!("area/pay");
    answers.push(scrambled);
    // And a label that is not a name.
    let mut numbered = a_ticket_of_this_scope();
    numbered["fields"]["labels"] = json!([7]);
    answers.push(numbered);
    // The whole of `fields`, which every one of the four is read out of.
    answers.push(json!({ "key": TICKET }));

    for answer in answers {
        let board = Jira::new(Calling::answering([Ok(answer.clone())]), a_scope());

        let error = board
            .named_ticket(NUMBER)
            .expect_err("not the answer asked for");

        assert!(
            matches!(error, Error::Malformed { board: JIRA, .. }),
            "{answer}: {error:?}"
        );
        assert_eq!(board.calls.made().len(), 1, "no retry and no second read");
    }
}

/// `PAY-31` taken or turned down, with the scope read off the very record the
/// board was opened for.
///
/// The whole point of the three reads below is that nothing about the project,
/// the label or the account reaches the request: the read is unfiltered, and
/// the judgement is made here over facts the site stated. So the same
/// [`a_scope`] record is both halves — what the board builds a key from and
/// what a refusal names — and the account is the one the key authenticates as.
fn taking(issue: Value) -> (Jira<Calling>, Named) {
    let record = a_scope();
    let board = Jira::new(Calling::answering([Ok(issue)]), record.clone());

    let named = take_named(&board, BoardScope::of(&record), ACCOUNT, TICKET, &[])
        .expect("the stand-in answered");

    (board, named)
}

/// The printable reason `PAY-31` was turned down, which is what each refusal
/// below is asserted as: the words are [`crate::queue`]'s own, shared with the
/// Linear arm, and no Jira spelling of them exists to drift.
fn turned_down(named: &Named) -> String {
    match named {
        Named::Refused(refused) => refused.to_string(),
        Named::Taken(issue) => panic!("{} was taken", issue.identifier()),
    }
}

#[test]
fn a_named_jira_ticket_inside_the_scope_is_taken() {
    let (board, named) = taking(a_ticket_of_this_scope());

    // The control the three refusals below are read against: everything about
    // this ticket is the record's, so each of them fails on the one fact it
    // changes.
    assert!(
        matches!(&named, Named::Taken(issue) if issue.identifier() == TICKET),
        "{named:?}"
    );
    assert_eq!(board.calls.made(), [by_key(TICKET)], "one request");
}

#[test]
fn a_named_ticket_in_another_project_is_refused_naming_both_projects() {
    let mut elsewhere = a_ticket_of_this_scope();
    // The project the answer states, not the prefix that was typed: `PAY-31`
    // is what was asked for, and the site answered with an issue filed under
    // `ENG`.
    elsewhere["fields"]["project"] = json!({ "id": "10001", "key": "ENG" });

    let (board, named) = taking(elsewhere);

    assert!(
        matches!(&named, Named::Refused(Refusal::NotOnTeam { team, found })
            if team == PROJECT && found == "ENG"),
        "{named:?}"
    );
    assert_eq!(
        turned_down(&named),
        "on team `ENG`, and this scope routes to `PAY`"
    );
    assert_eq!(board.calls.made(), [by_key(TICKET)], "no second request");
}

#[test]
fn a_named_ticket_without_the_records_label_is_refused_naming_the_label_it_wants() {
    let mut unlabelled = a_ticket_of_this_scope();
    unlabelled["fields"]["labels"] = json!(["area/web", "chore"]);

    let (board, named) = taking(unlabelled);

    // What it carries as well as what it is missing: the usual cause is a near
    // miss rather than a ticket nobody labelled.
    assert!(
        matches!(&named, Named::Refused(Refusal::Unlabelled { label, carried })
            if label == LABEL && carried == &["area/web", "chore"]),
        "{named:?}"
    );
    assert_eq!(
        turned_down(&named),
        "not labelled `area/pay` — it carries area/web, chore"
    );
    assert_eq!(board.calls.made(), [by_key(TICKET)], "no second request");

    let (bare, carrying_none) = taking(a_named_ticket(
        PROJECT,
        json!([]),
        json!({ "accountId": ACCOUNT, "displayName": HOLDER }),
    ));

    assert_eq!(
        turned_down(&carrying_none),
        "not labelled `area/pay` — it carries no labels at all"
    );
    assert_eq!(bare.calls.made(), [by_key(TICKET)], "no second request");
}

#[test]
fn a_named_ticket_somebody_else_holds_is_refused_naming_them() {
    let mut theirs = a_ticket_of_this_scope();
    // A different `accountId` under a display name the site holds: the id is
    // what the check is made on, and the name is the half that gets printed.
    theirs["fields"]["assignee"] = json!({
        "accountId": "5b10a2844c20165700ede000",
        "displayName": "Somebody Else",
    });

    let (board, named) = taking(theirs);

    assert!(
        matches!(&named, Named::Refused(Refusal::NotYours { holder })
            if holder.as_deref() == Some("Somebody Else")),
        "{named:?}"
    );
    assert_eq!(
        turned_down(&named),
        "assigned to Somebody Else and not to you"
    );
    assert_eq!(board.calls.made(), [by_key(TICKET)], "no second request");
}

/// The status a record names as the one a ticket in review belongs in, which is
/// the name a pull's second move asks for. No default workflow reaches it, which
/// is the whole of why the answer to a move has to say more than `false`.
const REVIEW: &str = "In Review";

/// Where the ticket stands as the status read answers it: the column it was
/// moved into when the run started, which is where it stays when nothing can
/// reach the one that was asked for.
fn stood_in(status: &str, category: &str) -> Value {
    json!({
        "id": "10031",
        "key": TICKET,
        "fields": { "status": { "name": status, "statusCategory": { "key": category } } },
    })
}

#[test]
fn the_ticket_is_moved_by_the_transition_whose_destination_carries_that_name() {
    let board = Jira::new(
        Calling::answering([
            Ok(a_default_workflow()),
            // A transition answers 204 with nothing after the headers.
            Ok(Value::Null),
        ]),
        a_scope(),
    );

    let answered = board
        .move_ticket(TICKET, IN_PROGRESS)
        .expect("the stand-in answered");

    assert_eq!(answered, Moved::Reached);
    // The moves the ticket offers read, then the one whose `to` is called that
    // posted by its id and nothing else in the body — and no status read, which
    // is a read bought for a line nobody prints when the move happened. A third
    // call would panic the stand-in, which is how "one request per read and one
    // per post" is asserted.
    assert_eq!(
        board.calls.made(),
        [offered(TICKET), moved(TICKET, "21")],
        "{:?}",
        board.calls.made()
    );
}

#[test]
fn a_workflow_reaching_no_status_of_that_name_names_what_it_reaches_and_where_the_ticket_stands() {
    // A default workflow and a review state, which is the pair a pull meets on
    // an unedited board: nothing it offers is called `In Review`, and a run that
    // said only that the move had not happened would leave a person looking at a
    // ticket in `In Progress` with nothing on screen to act on.
    let board = Jira::new(
        Calling::answering([
            Ok(a_default_workflow()),
            Ok(stood_in(IN_PROGRESS, "indeterminate")),
        ]),
        a_scope(),
    );

    let answered = board
        .move_ticket(TICKET, REVIEW)
        .expect("no such status is an answer");

    // Every status the workflow does reach, in the order the site listed them,
    // and the one the ticket is in now: the two facts the caller's line is made
    // of.
    assert_eq!(
        answered,
        Moved::Unreached {
            reachable: vec![TO_DO.to_owned(), IN_PROGRESS.to_owned(), "Done".to_owned()],
            standing: Some(IN_PROGRESS.to_owned()),
        }
    );
    // The transitions read and the status read, and no POST at all: no second
    // attempt at a status of another name, because the caller asked for one
    // column and warlock moving the ticket to a different one would be a move
    // nobody asked for.
    assert_eq!(
        board.calls.made(),
        [offered(TICKET), status_of(TICKET)],
        "{:?}",
        board.calls.made()
    );
}

#[test]
fn a_status_read_that_fails_after_no_move_was_found_leaves_the_status_unnamed() {
    let board = Jira::new(
        Calling::answering([
            Ok(a_default_workflow()),
            Err(Error::Status {
                board: JIRA,
                code: 503,
            }),
        ]),
        a_scope(),
    );

    let answered = board
        .move_ticket(TICKET, REVIEW)
        .expect("the status read is not worth failing the run over");

    assert_eq!(
        answered,
        Moved::Unreached {
            reachable: vec![TO_DO.to_owned(), IN_PROGRESS.to_owned(), "Done".to_owned()],
            standing: None,
        }
    );
    assert_eq!(
        board.calls.made(),
        [offered(TICKET), status_of(TICKET)],
        "{:?}",
        board.calls.made()
    );
}

/// The account of a stop a pull says on the ticket it ran, shortened but shaped
/// as `pulling.rs` writes one: prose, a heading and a list, which is Markdown
/// and is why the conversion stands between it and the wire.
const A_HALT: &str = "This pull halted, so the ticket stays in progress.\n\n\
     ### Finished\n\n- `WAR-193.01` Teach the ticket comment\n";

#[test]
fn a_halt_is_one_comment_on_the_ticket_carrying_the_body_as_adf() {
    let board = Jira::new(
        Calling::answering([Ok(
            json!({ "id": "10200", "self": "https://example.atlassian.net/rest/api/3/issue/10031/comment/10200" }),
        )]),
        a_scope(),
    );

    let id = board
        .comment_on_ticket(TICKET, A_HALT)
        .expect("the stand-in answered");

    // Jira's own id for the comment, which no caller reads.
    assert_eq!(id, "10200");
    // One POST to the ticket's own comment collection, the body as the document
    // the conversion made of it. A second call would panic the stand-in, which
    // is how "one request" is asserted.
    assert_eq!(
        board.calls.made(),
        [said(TICKET, A_HALT)],
        "{:?}",
        board.calls.made()
    );

    let (_, path, sent) = board.calls.made().remove(0);

    assert_eq!(path, format!("/rest/api/3/issue/{TICKET}/comment"));
    // And a document rather than the Markdown it was written as: the newer API
    // takes no string there, so a body that went up as text would be a halt
    // nobody is told about.
    assert_eq!(sent["body"]["type"], "doc");
    assert_ne!(sent["body"], json!(A_HALT));
}

#[test]
fn a_ticket_comment_jira_turns_down_is_reported_in_jiras_words_and_is_not_retried() {
    let refused = "You do not have permission to comment on this issue.";
    let board = Jira::new(
        Calling::answering([Err(Error::Refused {
            board: JIRA,
            message: refused.to_owned(),
        })]),
        a_scope(),
    );

    let error = board
        .comment_on_ticket(TICKET, A_HALT)
        .expect_err("the comment was refused");

    assert!(error.to_string().contains(refused), "{error}");
    // One request and nothing after it: the run's record is already on disk, and
    // a retried comment is how one halt gets explained twice on one ticket.
    assert_eq!(
        board.calls.made(),
        [said(TICKET, A_HALT)],
        "the comment was said twice"
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
