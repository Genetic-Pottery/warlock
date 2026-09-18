use std::fs;
use std::path::Path;

use warlock_engine::{
    keys_path, load_key, load_key_binding, load_key_names, load_sigils, save_key, save_key_binding,
    save_sigils, sigils_path,
};

use super::{added, bound, forgotten, listed};
use crate::error::Error;

// Every value stored anywhere below is this one string, so a test asserting
// that nothing printed a key has one needle to look for and a new rendering
// that leaks cannot leak something the assertions do not know about.
const SECRET: &str = "lin_api_this_value_must_never_be_printed";

// Every test builds both its home *and* its repository root out of these, so
// nothing here touches the developer's real `~/.warlock`.
fn a_dir() -> tempfile::TempDir {
    tempfile::tempdir().expect("a temporary directory")
}

fn text(out: Vec<u8>) -> String {
    String::from_utf8(out).expect("warlock writes its own text")
}

// The production composition with the two things a person supplies handed in
// instead: the answer — `None` is EOF — and somewhere to print.
fn add_with(home: &Path, name: &str, line: Option<&str>) -> (Result<(), Error>, String) {
    let answer = line.map(str::to_owned);
    let mut out = Vec::new();
    let outcome = added(home, name, || Ok(answer), &mut out);
    (outcome, text(out))
}

fn listing(home: &Path, json: bool) -> String {
    let mut out = Vec::new();
    listed(home, json, &mut out).expect("a store that was just written lists");
    text(out)
}

fn bind(home: &Path, root: &Path, name: &str) -> (Result<(), Error>, String) {
    let mut out = Vec::new();
    let outcome = bound(home, root, name, &mut out);
    (outcome, text(out))
}

fn forget(home: &Path, root: &Path, name: &str) -> (Result<(), Error>, String) {
    let mut out = Vec::new();
    let outcome = forgotten(home, root, name, &mut out);
    (outcome, text(out))
}

#[test]
fn an_end_of_file_writes_nothing_and_says_so() {
    let home = a_dir();

    let (outcome, said) = add_with(home.path(), "acme", None);

    outcome.expect("an EOF is not a failure");
    assert!(said.ends_with("\nwarlock: nothing changed\n"), "{said:?}");
    assert!(
        !keys_path(home.path()).exists(),
        "a missing store was created by a prompt nobody answered"
    );
}

#[test]
fn a_piped_key_round_trips_and_never_appears_on_the_screen() {
    let home = a_dir();

    // The trailing newline a pipe supplies is not part of the key.
    let (outcome, said) = add_with(home.path(), "acme", Some(&format!("{SECRET}\n")));

    outcome.expect("a line holding a key is stored");
    assert_eq!(
        load_key(home.path(), "acme").expect("the store just written loads"),
        Some(SECRET.to_owned()),
        "what went to disk is not what was piped in"
    );
    assert!(
        said.contains(&keys_path(home.path()).display().to_string()),
        "the confirmation does not name the file it wrote: {said}"
    );
    assert!(!said.contains(SECRET), "the key reached the screen: {said}");
}

fn a_store_of_two(home: &Path) {
    for name in ["acme", "personal"] {
        save_key(home, name, SECRET).expect("a store that saves");
    }
}

#[test]
fn a_listing_prints_the_names_one_per_line_and_nothing_else() {
    let home = a_dir();
    a_store_of_two(home.path());

    let said = listing(home.path(), false);

    assert_eq!(
        said.lines().collect::<Vec<_>>(),
        ["acme", "personal"],
        "a name per line and no line that is not a name: {said:?}"
    );
    assert!(!said.contains(SECRET), "the key reached the screen: {said}");
}

#[test]
fn the_json_listing_is_one_object_carrying_names_only() {
    let home = a_dir();
    a_store_of_two(home.path());

    let said = listing(home.path(), true);

    assert_eq!(said.lines().count(), 1, "not one line: {said:?}");
    assert!(
        said.contains("\"command\":\"key list\""),
        "the envelope does not name the command: {said}"
    );
    assert!(
        said.contains("\"acme\"") && said.contains("\"personal\""),
        "{said}"
    );
    assert!(
        !said.contains(SECRET),
        "the key reached the machine-readable answer too: {said}"
    );
}

#[test]
fn binding_a_name_the_store_has_never_heard_of_writes_no_byte() {
    let (home, repo) = (a_dir(), a_dir());
    save_key(home.path(), "acme", SECRET).expect("a store that saves");
    let path = sigils_path(home.path(), repo.path());

    let (outcome, said) = bind(home.path(), repo.path(), "acne");

    let error = outcome.expect_err("a name nobody stored is refused");
    assert!(matches!(error, Error::UnknownKey { .. }), "{error:?}");
    assert!(
        said.is_empty(),
        "a refusal printed a confirmation: {said:?}"
    );
    assert!(!path.exists(), "a missing config was created by a refusal");

    // And an existing one is byte for byte what it was.
    save_sigils(home.path(), repo.path(), &["billing".to_owned()]).expect("a config that saves");
    let before = fs::read(&path).expect("the config just written reads");
    assert!(bind(home.path(), repo.path(), "acne").0.is_err());
    assert_eq!(fs::read(&path).expect("reads"), before);
}

#[test]
fn binding_keeps_the_sigils_the_config_already_held() {
    let (home, repo) = (a_dir(), a_dir());
    save_key(home.path(), "acme", SECRET).expect("a store that saves");
    save_sigils(
        home.path(),
        repo.path(),
        &["billing".to_owned(), "web".to_owned()],
    )
    .expect("a config that saves");

    let (outcome, said) = bind(home.path(), repo.path(), "acme");

    outcome.expect("a stored name binds");
    assert_eq!(
        load_sigils(home.path(), repo.path()).expect("the config still loads"),
        ["billing", "web"],
        "binding a key cost this checkout its sigils"
    );
    assert_eq!(
        load_key_binding(home.path(), repo.path()).expect("the config still loads"),
        Some("acme".to_owned())
    );
    assert!(
        said.contains(&sigils_path(home.path(), repo.path()).display().to_string()),
        "the confirmation does not name the file it wrote: {said}"
    );
    assert!(!said.contains(SECRET), "the key reached the screen: {said}");
}

#[test]
fn forgetting_a_name_the_store_has_never_heard_of_removes_nothing() {
    let (home, repo) = (a_dir(), a_dir());

    // A store that is not there at all is the same refusal as one that simply
    // holds no such name, and neither writes.
    let (outcome, said) = forget(home.path(), repo.path(), "acme");
    assert!(
        matches!(outcome, Err(Error::UnknownKey { .. })),
        "{outcome:?}"
    );
    assert!(
        said.is_empty(),
        "a refusal printed a confirmation: {said:?}"
    );
    assert!(!keys_path(home.path()).exists());

    save_key(home.path(), "acme", SECRET).expect("a store that saves");
    let before = fs::read(keys_path(home.path())).expect("the store just written reads");

    let (outcome, said) = forget(home.path(), repo.path(), "acne");

    assert!(
        matches!(outcome, Err(Error::UnknownKey { .. })),
        "{outcome:?}"
    );
    assert!(said.is_empty(), "{said:?}");
    assert_eq!(
        fs::read(keys_path(home.path())).expect("reads"),
        before,
        "a refused removal still rewrote the store"
    );
}

#[test]
fn forgetting_the_bound_name_says_this_checkout_is_now_unbound() {
    let (home, repo) = (a_dir(), a_dir());
    a_store_of_two(home.path());
    save_key_binding(home.path(), repo.path(), "acme").expect("a config that saves");

    // The name this checkout is not bound to: removed, with nothing said about
    // a binding that never was.
    let (outcome, said) = forget(home.path(), repo.path(), "personal");
    outcome.expect("a stored name is forgotten");
    assert!(!said.contains("unbound"), "{said}");

    let (outcome, said) = forget(home.path(), repo.path(), "acme");

    outcome.expect("a stored name is forgotten");
    assert!(
        said.contains("unbound"),
        "the checkout lost its key and was not told: {said}"
    );
    assert!(
        said.contains(&repo.path().display().to_string()),
        "the sentence does not name the checkout it is about: {said}"
    );
    assert_eq!(said.lines().count(), 1, "not one line: {said:?}");
    assert_eq!(
        load_key_names(home.path()).expect("the store still loads"),
        Vec::<String>::new()
    );
    assert!(!said.contains(SECRET), "the key reached the screen: {said}");
}
