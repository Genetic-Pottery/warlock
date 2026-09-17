use std::fs;
use std::path::{Path, PathBuf};

use warlock_engine::{Manifest, PactEntry, save_sigils, sigils_path};
use warlock_tui::Sigils;

use super::{Checked, checked, checked_onto, object, prose};
use crate::error::Error;
use crate::standing::Standing;
use crate::status_for;

fn standing_in(repo: &Path) -> Standing {
    Standing::at(repo.to_path_buf(), repo.to_path_buf())
}

fn said(repo: &Path, path: &str, json: bool) -> String {
    let mut out = Vec::new();
    checked_onto(&standing_in(repo), PathBuf::from(path), json, &mut out)
        .expect("a check inside a repository answers");
    String::from_utf8(out).expect("warlock writes its own text")
}

#[test]
fn the_composition_answers_about_a_path_the_manifest_has_never_heard_of() {
    let repo = tempfile::tempdir().expect("a temporary directory");

    let line = said(repo.path(), "src", false);

    assert!(
        line.contains("nothing scopes `src`"),
        "a repository that never pacted anything covers nothing: {line}"
    );
    assert!(
        line.contains("open to anyone"),
        "an unscoped path is open, and the answer should say so: {line}"
    );
    assert!(line.ends_with('\n'), "the answer is terminated: {line:?}");
}

#[test]
fn the_composition_reads_the_scope_the_manifest_holds() {
    let repo = tempfile::tempdir().expect("a temporary directory");
    Manifest::with_entries([PactEntry::new(".", "src", "src/WARLOCK.md")
        .expect("a relative module path is inside the root")
        .with_scope("data-plane")])
    .save(repo.path())
    .expect("a manifest that saves");

    let line = said(repo.path(), "src", false);

    assert!(
        line.contains("data-plane"),
        "the scope on disk is not in the answer: {line}"
    );
}

#[test]
fn a_missing_manifest_answers_and_an_unparsable_one_refuses() {
    let repo = tempfile::tempdir().expect("a temporary directory");
    // Missing: an answer, not a failure.
    let mut out = Vec::new();
    checked_onto(
        &standing_in(repo.path()),
        PathBuf::from("src"),
        false,
        &mut out,
    )
    .expect("a repository with no manifest still answers");

    // There and broken: a failure, and not one word printed first.
    let warlock = repo.path().join(".warlock");
    fs::create_dir_all(&warlock).expect("the bookkeeping directory");
    fs::write(warlock.join("pacts.toml"), "not toml {{{").expect("a broken manifest");

    let mut out = Vec::new();
    let error = checked_onto(
        &standing_in(repo.path()),
        PathBuf::from("src"),
        false,
        &mut out,
    )
    .expect_err("a manifest that will not parse is a failure");

    assert!(matches!(error, Error::Manifest { .. }), "{error:?}");
    assert!(
        out.is_empty(),
        "a partial answer was printed before the refusal: {out:?}"
    );
    assert_eq!(status_for(&Err(error)), 1);
}

#[test]
fn the_json_answer_is_one_line_and_names_the_command() {
    let repo = tempfile::tempdir().expect("a temporary directory");

    let line = said(repo.path(), "src", true);

    assert_eq!(line.lines().count(), 1, "not one line: {line:?}");
    assert!(
        line.contains("\"command\":\"check\""),
        "the envelope does not name the command: {line}"
    );
}

// A path rather than a directory on disk, deliberately: coverage is a walk
// up the manifest's stored paths and never a walk of the filesystem, so
// nothing here has to exist for the answer to be the answer.
const REPO: &str = "/repo";

// Every test that reads or writes a config builds its home out of one of
// these, so nothing here goes near the developer's real home.
fn a_dir() -> tempfile::TempDir {
    tempfile::tempdir().expect("a temporary directory")
}

fn entry(module: &str) -> PactEntry {
    PactEntry::new(".", module, format!("{module}/WARLOCK.md"))
        .expect("a relative module path is inside the root")
}

fn a_manifest() -> Manifest {
    Manifest::with_entries([
        entry("crates").with_scope("platform"),
        entry("crates/engine").with_scope("data-plane"),
        entry("docs"),
    ])
}

fn answer(home: &Path, path: &str) -> Checked {
    checked(
        Path::new(REPO),
        Some(home),
        &a_manifest(),
        &Path::new(REPO).join(path),
    )
    .expect("a path inside the repository has a manifest form")
}

fn holding(home: &Path, sigils: &[&str]) {
    let sigils: Vec<String> = sigils.iter().map(|sigil| (*sigil).to_owned()).collect();
    save_sigils(home, Path::new(REPO), &sigils).expect("a config that writes");
}

fn a_broken_config(home: &Path) {
    let path = sigils_path(home, Path::new(REPO));
    fs::create_dir_all(
        path.parent()
            .expect("the config lives in a project directory"),
    )
    .expect("the project directory");
    fs::write(&path, "not a config\n").expect("a file that is not TOML");
}

#[test]
fn a_covered_path_is_named_by_the_nearest_scope_above_it() {
    // The engine's nearest-wins walk, read back out here: a file deep under
    // `crates/engine` answers with the inner scope, and a sibling that has
    // said nothing answers with the outer one.
    let home = a_dir();

    assert_eq!(
        answer(home.path(), "crates/engine/src/lib.rs")
            .scope
            .as_deref(),
        Some("data-plane")
    );
    assert_eq!(
        answer(home.path(), "crates/engine").scope.as_deref(),
        Some("data-plane")
    );
    assert_eq!(
        answer(home.path(), "crates/tui/src").scope.as_deref(),
        Some("platform")
    );
}

#[test]
fn a_path_nothing_covers_says_so_rather_than_naming_a_scope() {
    let home = a_dir();

    // Pacted and unscoped is not covered, and neither is the root itself.
    assert_eq!(answer(home.path(), "docs/adr").scope, None);
    assert_eq!(answer(home.path(), ".").scope, None);
    // And an unscoped path is open to a machine holding nothing at all:
    // the permissive default is on the directory and only there.
    assert!(answer(home.path(), "docs/adr").opens);
    assert_eq!(
        prose(&answer(home.path(), "docs/adr")),
        "nothing scopes `docs/adr`\n\
             holding nothing\n\
             an unscoped path is open to anyone, so this machine may work here"
    );
}

#[test]
fn a_held_sigil_opens_the_scope_it_matches_and_nothing_else() {
    let home = a_dir();
    holding(home.path(), &["platform"]);

    let open = answer(home.path(), "crates/tui/src");
    assert_eq!(open.sigils, Sigils::Held(vec!["platform".to_owned()]));
    assert!(open.opens);
    assert_eq!(
        prose(&open),
        "`crates/tui/src` is scoped `platform`\n\
             holding `platform`\n\
             `platform` is open to this machine"
    );

    // The nearer scope replaces the outer one outright, so holding
    // `platform` does not open what `data-plane` covers.
    let closed = answer(home.path(), "crates/engine");
    assert!(!closed.opens);
    // A closed scope is the answer and not a failure to reach one, so what
    // `main` makes of it is a 0: the verdict is in the output, and the
    // non-zero status is left for `jq -e '.opens'` to spend. Asked a second
    // time for its `Result` alone, which the helper above unwraps — a check
    // reads one config file and walks a manifest already in hand, so asking
    // twice costs nothing.
    assert_eq!(
        status_for(
            &checked(
                Path::new(REPO),
                Some(home.path()),
                &a_manifest(),
                &Path::new(REPO).join("crates/engine"),
            )
            .map(|_| ())
        ),
        0
    );
    assert_eq!(
        prose(&closed),
        "`crates/engine` is scoped `data-plane`\n\
             holding `platform`\n\
             `data-plane` is closed to this machine — hold that sigil to work \
             here, with `warlock config`"
    );
}

#[test]
fn a_machine_that_holds_nothing_is_closed_by_every_scope_and_still_answers() {
    // No config at all: the ordinary state of a machine nobody has run
    // `warlock config` on. It is an answer, not a failure.
    let home = a_dir();
    let closed = answer(home.path(), "crates/engine");

    assert_eq!(closed.sigils, Sigils::Nothing);
    assert!(!closed.opens);
    assert_eq!(
        object(&closed).to_string(),
        r#"{"command":"check","path":"crates/engine","scope":"data-plane","sigils":[],"opens":false}"#
    );
}

#[test]
fn a_config_that_will_not_parse_is_unknown_and_never_an_empty_holding() {
    // The case `[]` would lie about: an operator told they hold nothing when
    // the truth is that warlock could not read what they hold.
    let home = a_dir();
    a_broken_config(home.path());
    let unknown = answer(home.path(), "crates/engine");

    assert_eq!(unknown.sigils, Sigils::Unknown);
    assert!(
        !unknown.opens,
        "a holding warlock cannot read opens nothing"
    );
    assert_eq!(
        object(&unknown).to_string(),
        r#"{"command":"check","path":"crates/engine","scope":"data-plane","sigils":null,"opens":false}"#
    );
    // And the prose names the file, which is the one thing a reader needs
    // in order to fix it.
    let path = sigils_path(home.path(), Path::new(REPO));
    assert!(
        prose(&unknown).contains(&format!("`{}` could not be read", path.display())),
        "the unreadable file is not named: {}",
        prose(&unknown)
    );
}

#[test]
fn a_missing_home_is_nothing_held_rather_than_a_config_that_would_not_read() {
    // `Unknown` claims a file on disk is broken. With no home there is no
    // file and no path to name one by, so there is nothing broken to report.
    let nowhere = checked(
        Path::new(REPO),
        None,
        &a_manifest(),
        &Path::new(REPO).join("crates/engine"),
    )
    .expect("a path inside the repository has a manifest form");

    assert_eq!(nowhere.sigils, Sigils::Nothing);
    assert_eq!(nowhere.config, None);
    assert!(!nowhere.opens);
    assert!(prose(&nowhere).contains("holding nothing"));
}

#[test]
fn a_path_outside_the_repository_is_refused_rather_than_reported_as_unscoped() {
    // `scope_covering`'s own doc: such a path is not unscoped, it is one
    // this manifest has nothing whatever to say about. Answering
    // `scope: null` would tell a script it is open to anyone.
    let home = a_dir();

    for outside in [
        Path::new("/elsewhere").to_path_buf(),
        Path::new(REPO).join("..").join("elsewhere"),
    ] {
        let refused =
            checked(Path::new(REPO), Some(home.path()), &a_manifest(), &outside).map(|_| ());
        // A question warlock could not answer, so 1 — the status a closed
        // scope above deliberately does not take.
        assert_eq!(status_for(&refused), 1, "{}", outside.display());

        let error = refused.expect_err("a path outside the repository has no manifest form");
        assert!(
            matches!(error, Error::Unspellable { .. }),
            "the engine's own case was rewrapped: {error:?}"
        );
        assert!(!error.to_string().contains('\n'), "`main` prints one line");
    }
}

#[test]
fn the_object_is_the_shape_a_consumer_was_promised() {
    let home = a_dir();
    holding(home.path(), &["billing"]);

    assert_eq!(
        object(&answer(home.path(), "crates/engine")).to_string(),
        r#"{"command":"check","path":"crates/engine","scope":"data-plane","sigils":["billing"],"opens":false}"#
    );
}

#[test]
fn nothing_covering_the_path_is_a_null_scope_rather_than_a_missing_field() {
    let home = a_dir();

    assert_eq!(
        object(&answer(home.path(), "docs/adr")).to_string(),
        r#"{"command":"check","path":"docs/adr","scope":null,"sigils":[],"opens":true}"#
    );
}

#[test]
fn no_object_carries_a_root_or_a_home() {
    // Deliberate: an absolute machine path is the one non-reproducible thing
    // in a document meant to be diffable across machines. The home is worse
    // — it names a person.
    let home = a_dir();
    holding(home.path(), &["billing"]);
    let printed = object(&answer(home.path(), "crates/engine")).to_string();

    assert!(!printed.contains("root"), "{printed}");
    assert!(!printed.contains(REPO), "{printed}");
    assert!(
        !printed.contains(&home.path().display().to_string()),
        "{printed}"
    );

    // Including the one answer that has a file to name: the prose says
    // which file, and the object still does not.
    let broken = a_dir();
    a_broken_config(broken.path());
    let printed = object(&answer(broken.path(), "crates/engine")).to_string();
    assert!(
        !printed.contains(&broken.path().display().to_string()),
        "{printed}"
    );
}
