use std::error::Error as _;
use std::io;

use serde_json::Value;
use ureq::http::Method;

use super::{Calls, Client, EMAIL_TOKEN, JIRA, REQUEST_TIMEOUT, authorization, base64, pair, url};

use crate::board::Error;

const EMAIL: &str = "nobody@example.com";
const TOKEN: &str = "ATATT3xFfGF0_a_token_nobody_holds_8f3a1c";
const SITE: &str = "https://example.atlassian.net";

fn key() -> String {
    format!("{EMAIL}:{TOKEN}")
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
