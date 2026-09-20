use std::collections::VecDeque;
use std::error::Error as _;
use std::io;
use std::sync::Mutex;

use serde_json::{Value, json};

use super::{Client, ENDPOINT, Error, Posts, REQUEST_TIMEOUT, answer, authorization};

const KEY: &str = "lin_api_a_key_nobody_holds_8f3a1c";

// A Linear that answers from memory: it hands back what it was given, in the
// order it was given, and keeps every document it was asked so that a test can
// say which call came first. Answers are popped rather than cloned because
// `Error` is not `Clone` — carrying a refusal that could only happen once is the
// point, not a limitation.
#[derive(Debug, Default)]
struct Posting {
    answers: Mutex<VecDeque<Result<Value, Error>>>,
    asked: Mutex<Vec<(String, Value)>>,
}

impl Posting {
    fn answering(answers: impl IntoIterator<Item = Result<Value, Error>>) -> Self {
        Self {
            answers: Mutex::new(answers.into_iter().collect()),
            asked: Mutex::new(Vec::new()),
        }
    }

    fn documents(&self) -> Vec<String> {
        self.asked
            .lock()
            .expect("the stand-in was not used across a panic")
            .iter()
            .map(|(document, _)| document.clone())
            .collect()
    }

    fn variables(&self) -> Vec<Value> {
        self.asked
            .lock()
            .expect("the stand-in was not used across a panic")
            .iter()
            .map(|(_, variables)| variables.clone())
            .collect()
    }
}

impl Posts for Posting {
    fn post(&self, document: &str, variables: Value) -> Result<Value, Error> {
        self.asked
            .lock()
            .expect("the stand-in was not used across a panic")
            .push((document.to_owned(), variables));

        self.answers
            .lock()
            .expect("the stand-in was not used across a panic")
            .pop_front()
            .unwrap_or_else(|| panic!("the stand-in was asked more times than it was answered"))
    }
}

fn variants() -> Vec<Error> {
    vec![
        Error::Key,
        Error::Transport {
            source: ureq::Error::Io(io::Error::other("connection reset by peer")),
        },
        Error::Status { code: 401 },
        Error::Malformed {
            detail: "it was not JSON".to_owned(),
        },
        Error::Refused {
            message: "Entity not found".to_owned(),
        },
    ]
}

#[test]
fn one_timeout_of_thirty_seconds_covers_the_whole_call() {
    assert_eq!(
        REQUEST_TIMEOUT.as_secs(),
        30,
        "thirty seconds, per Linear call"
    );
    // The global timeout, which is the one that covers resolve, connect,
    // handshake, send and body rather than any single phase of them.
    assert_eq!(Client::new(KEY).timeout(), Some(REQUEST_TIMEOUT));
}

#[test]
fn the_endpoint_is_linears_one_graphql_url() {
    assert_eq!(ENDPOINT, "https://api.linear.app/graphql");
}

#[test]
fn the_key_is_a_parameter_and_the_client_never_prints_it() {
    let client = Client::new(KEY);

    let printed = format!("{client:?}");

    assert!(
        !printed.contains(KEY),
        "the key reached `Debug` output: {printed}"
    );
}

#[test]
fn no_error_variant_prints_the_key() {
    for error in variants() {
        let (shown, printed) = (error.to_string(), format!("{error:?}"));

        assert!(!shown.contains(KEY), "the key reached `Display`: {shown}");
        assert!(!printed.contains(KEY), "the key reached `Debug`: {printed}");
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
fn the_authorization_value_is_the_bare_key() {
    let value = authorization(KEY).expect("an ordinary key is a header value");

    let sent = value.to_str().expect("an ASCII key is a readable value");

    // Not `Bearer <key>`: that is what an OAuth token wants, and it is a 401
    // for a personal API key.
    assert_eq!(sent, KEY);
    assert!(!sent.contains("Bearer"));
    // And marked sensitive, so a header map that gets printed prints
    // `Sensitive` where the key is.
    assert!(value.is_sensitive());
    assert!(!format!("{value:?}").contains(KEY));
}

#[test]
fn a_key_that_cannot_be_a_header_value_is_refused_without_quoting_it() {
    let error = authorization(&format!("{KEY}\nX-Injected: yes"))
        .expect_err("a newline cannot go in a header value");

    assert!(matches!(error, Error::Key));
    assert!(!error.to_string().contains(KEY));
}

#[test]
fn the_data_object_is_what_comes_back() {
    let body = json!({ "data": { "team": { "id": "team-1" } } });

    let data = answer(body).expect("a `data` object is an answer");

    assert_eq!(data, json!({ "team": { "id": "team-1" } }));
}

#[test]
fn a_graphql_errors_array_is_a_refusal_in_linears_words() {
    let body = json!({
        "data": null,
        "errors": [{ "message": "Entity not found" }, { "message": "and another" }],
    });

    let error = answer(body).expect_err("an `errors` array is a refusal");

    assert!(
        matches!(&error, Error::Refused { message } if message == "Entity not found"),
        "{error:?}"
    );
}

#[test]
fn an_errors_entry_with_no_message_still_refuses() {
    let body = json!({ "data": null, "errors": [{ "extensions": {} }] });

    let error = answer(body).expect_err("an `errors` array is a refusal");

    assert!(matches!(&error, Error::Refused { .. }), "{error:?}");
}

#[test]
fn an_answer_with_no_data_is_malformed() {
    for body in [json!({}), json!({ "data": null }), json!("not an object")] {
        let error = answer(body.clone()).expect_err("no `data` is no answer");

        assert!(
            matches!(&error, Error::Malformed { .. }),
            "{body}: {error:?}"
        );
    }
}

#[test]
fn the_seam_drives_an_operation_with_no_socket() {
    let linear = Posting::answering([Ok(json!({ "teams": { "nodes": [] } }))]);

    let data = linear
        .post(
            "query Teams($key: String!) { teams { nodes { id } } }",
            json!({ "key": "WAR" }),
        )
        .expect("the stand-in answered");

    assert_eq!(data, json!({ "teams": { "nodes": [] } }));
    assert_eq!(
        linear.documents(),
        ["query Teams($key: String!) { teams { nodes { id } } }"]
    );
    assert_eq!(linear.variables(), [json!({ "key": "WAR" })]);
}

#[test]
fn the_seam_carries_a_refusal_as_well_as_an_answer() {
    let linear = Posting::answering([Err(Error::Status { code: 401 })]);

    let error = linear
        .post("query { viewer { id } }", json!({}))
        .expect_err("the stand-in refused");

    assert!(matches!(error, Error::Status { code: 401 }), "{error:?}");
}
