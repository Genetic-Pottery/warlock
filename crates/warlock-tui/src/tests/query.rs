use std::path::Path;

use serde_json::{Value, json};

use super::{envelope, spelled, write_object};
use crate::error::Error;
use crate::error::status_for;

#[test]
fn the_root_itself_spells_as_a_dot() {
    assert_eq!(
        spelled(Path::new("/repo"), Path::new("/repo")).expect("the root spells"),
        ".",
        "the manifest's own spelling of the root"
    );
}

#[test]
fn a_path_inside_the_repository_spells_against_the_root() {
    assert_eq!(
        spelled(Path::new("/repo"), Path::new("/repo/crates/engine")).expect("an inner path"),
        "crates/engine"
    );
}

#[test]
fn a_path_outside_the_repository_has_no_spelling() {
    spelled(Path::new("/repo"), Path::new("/elsewhere"))
        .expect_err("nothing outside the repository has a manifest form");
}

#[test]
fn a_path_with_no_repository_relative_spelling_is_a_one_line_refusal() {
    let refused = spelled(Path::new("/repo"), Path::new("/elsewhere")).map(|_| ());
    assert_eq!(status_for(&refused), 1);

    let error = refused.expect_err("a path outside the repository has no manifest form");
    assert!(
        matches!(error, Error::Unspellable { .. }),
        "the engine's own case was rewrapped: {error:?}"
    );
    assert!(!error.to_string().contains('\n'), "`main` prints one line");
}

#[test]
fn the_envelope_names_its_command_first_and_keeps_the_body_in_order() {
    let object = envelope(
        "check",
        [("path", json!("crates")), ("allowed", json!(true))],
    );

    assert_eq!(
        object.to_string(),
        r#"{"command":"check","path":"crates","allowed":true}"#
    );
}

#[test]
fn an_object_is_written_as_one_line() {
    let object = envelope("check", [("nested", json!({"a": [1, 2]}))]);
    let mut out = Vec::new();

    write_object(&mut out, &object);

    let printed = String::from_utf8(out).expect("warlock writes its own text");
    assert_eq!(printed.lines().count(), 1, "{printed:?}");
    assert!(printed.ends_with('\n'), "{printed:?}");
    let parsed: Value = serde_json::from_str(&printed).expect("one parseable object");
    assert_eq!(parsed, object);
}
