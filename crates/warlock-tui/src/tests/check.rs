use std::fs;
use std::path::{Path, PathBuf};

use serde_json::json;
use warlock_engine::{
    Manifest, ScopeRecord, ScopedDirectory, config_path, save_key, save_key_binding,
    save_open_scopes,
};

use super::{
    Checked, Filed, Recorded, checked, checked_onto, gated_onto, hooked_onto, object, prose,
};
use crate::app::OpenScopes;
use crate::error::Error;
use crate::error::status_for;
use crate::standing::Standing;

fn standing_in(repo: &Path) -> Standing {
    Standing::at(repo.to_path_buf(), repo.to_path_buf())
}

// The home is handed in for the same reason the repository is: the composition
// reads the config under whatever home it is given, and one resolved in here
// would be the developer's own.
fn said(repo: &Path, home: &Path, path: &str, json: bool) -> String {
    let mut out = Vec::new();
    checked_onto(
        &standing_in(repo),
        Some(home),
        PathBuf::from(path),
        json,
        &mut out,
    )
    .expect("a check inside a repository answers");
    String::from_utf8(out).expect("warlock writes its own text")
}

#[test]
fn the_composition_answers_about_a_path_the_manifest_has_never_heard_of() {
    let (repo, home) = (a_dir(), a_dir());

    let line = said(repo.path(), home.path(), "src", false);

    assert!(
        line.contains("nothing scopes `src`"),
        "a repository that never scoped anything covers nothing: {line}"
    );
    assert!(
        line.contains("an unscoped path is closed to everyone"),
        "an unscoped path is closed, and the answer should say so: {line}"
    );
    assert!(line.ends_with('\n'), "the answer is terminated: {line:?}");
}

#[test]
fn the_composition_reads_the_scope_the_manifest_holds() {
    let (repo, home) = (a_dir(), a_dir());
    Manifest::with_directories([ScopedDirectory::new(".", "src", "data-plane")
        .expect("a relative module path is inside the root")])
    .save(repo.path())
    .expect("a manifest that saves");

    let line = said(repo.path(), home.path(), "src", false);

    assert!(
        line.contains("data-plane"),
        "the scope on disk is not in the answer: {line}"
    );
}

#[test]
fn a_missing_manifest_answers_and_an_unparsable_one_refuses() {
    let (repo, home) = (a_dir(), a_dir());
    // Missing: an answer, not a failure.
    let mut out = Vec::new();
    checked_onto(
        &standing_in(repo.path()),
        Some(home.path()),
        PathBuf::from("src"),
        false,
        &mut out,
    )
    .expect("a repository with no manifest still answers");

    // There and broken: a failure, and not one word printed first.
    let warlock = repo.path().join(".warlock");
    fs::create_dir_all(&warlock).expect("the bookkeeping directory");
    fs::write(warlock.join("scopes.toml"), "not toml {{{").expect("a broken manifest");

    let mut out = Vec::new();
    let error = checked_onto(
        &standing_in(repo.path()),
        Some(home.path()),
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
    let (repo, home) = (a_dir(), a_dir());

    let line = said(repo.path(), home.path(), "src", true);

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

fn entry(module: &str, scope: &str) -> ScopedDirectory {
    ScopedDirectory::new(".", module, scope).expect("a relative module path is inside the root")
}

// `data-plane` has a `[[scope]]` record and `platform` deliberately has none:
// a scope somebody drew before anybody filed a routing record for it is the
// ordinary half-finished state, and every test below can reach both from one
// manifest.
fn a_manifest() -> Manifest {
    Manifest::with_directories([
        entry("crates", "platform"),
        entry("crates/engine", "data-plane"),
    ])
    .with_scopes([ScopeRecord::new(
        "data-plane",
        "Data Plane",
        "In Review",
        "area/data-plane",
    )])
}

// The same manifest with the recorded scope filing to Jira: a check reports and
// never refuses, so the board changes which fields the route is said in and
// nothing else about the answer.
fn a_jira_manifest() -> Manifest {
    Manifest::with_directories([
        entry("crates", "platform"),
        entry("crates/engine", "data-plane"),
    ])
    .with_scopes([ScopeRecord::jira(
        "data-plane",
        "https://acme.atlassian.net",
        "PAY",
        "In Review",
        "area/data-plane",
    )])
}

// Not a key, and named so nothing reading this file mistakes it for one. It is
// written only so a bound name can be made to resolve, and the tests below
// assert it is nowhere in what warlock prints.
const NOT_A_KEY: &str = "not-a-real-key-value";

fn answer(home: &Path, path: &str) -> Checked {
    checked(
        Path::new(REPO),
        Some(home),
        &a_manifest(),
        &Path::new(REPO).join(path),
    )
    .expect("a path inside the repository has a manifest form")
}

// `answer`'s twin against the Jira manifest, so the Jira cases below stand
// beside the Linear ones they are read against.
fn jira_answer(home: &Path, path: &str) -> Checked {
    checked(
        Path::new(REPO),
        Some(home),
        &a_jira_manifest(),
        &Path::new(REPO).join(path),
    )
    .expect("a path inside the repository has a manifest form")
}

// The two manifests' `[[scope]]` records as `Checked` holds them: the board
// carries its own addressing fields, so the Linear record has a team and no
// site and the Jira one has a site and a project and no team. Neither can be
// built half-filled, which is what the object's `null`s are rendered from.
fn the_linear_record() -> Recorded {
    Recorded {
        filed: Filed::Linear {
            team: "Data Plane".to_owned(),
        },
        review_state: "In Review".to_owned(),
        label: "area/data-plane".to_owned(),
    }
}

fn the_jira_record() -> Recorded {
    Recorded {
        filed: Filed::Jira {
            base_url: "https://acme.atlassian.net".to_owned(),
            project_key: "PAY".to_owned(),
        },
        review_state: "In Review".to_owned(),
        label: "area/data-plane".to_owned(),
    }
}

fn opening(home: &Path, open: &[&str]) {
    opening_in(home, Path::new(REPO), open);
}

// The repository is a parameter as well as the home, because the config is keyed
// by root: the gate tests below stand in a temporary repository on disk, and
// open scopes written against `REPO` would not be the ones they read.
fn opening_in(home: &Path, repo: &Path, open: &[&str]) {
    let open: Vec<String> = open.iter().map(|scope| (*scope).to_owned()).collect();
    save_open_scopes(home, repo, &open).expect("a config that writes");
}

// The scope is a parameter because the config holds a key name per scope, and
// `check` reads the binding of the scope covering the path it was asked about:
// each caller below passes the scope its own path resolves to, so a binding
// written for the other scope of `a_manifest` would not be the one read.
fn bound(home: &Path, scope: &str, name: &str) {
    save_key_binding(home, Path::new(REPO), scope, name).expect("a binding that writes");
}

fn keeping(home: &Path, name: &str) {
    save_key(home, name, NOT_A_KEY).expect("a key store that writes");
}

fn a_broken_config(home: &Path) {
    let path = config_path(home, Path::new(REPO));
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

    assert_eq!(answer(home.path(), "docs/adr").scope, None);
    assert_eq!(answer(home.path(), ".").scope, None);
    // Work happens only inside a scope, so an unscoped path is closed even to
    // a machine with the wildcard open.
    assert!(!answer(home.path(), "docs/adr").opens);
    opening(home.path(), &["*"]);
    assert!(!answer(home.path(), "docs/adr").opens);
}

#[test]
fn an_open_scope_opens_the_scope_it_matches_and_nothing_else() {
    let home = a_dir();
    opening(home.path(), &["platform"]);

    let open = answer(home.path(), "crates/tui/src");
    assert_eq!(open.open, OpenScopes::Named(vec!["platform".to_owned()]));
    assert!(open.opens);
    assert!(prose(&open).contains("`platform` is open on this machine"));

    // The nearer scope replaces the outer one outright, so `platform` being
    // open does not open what `data-plane` covers.
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
    assert!(prose(&closed).contains(
        "`data-plane` is closed on this machine — `warlock open data-plane` lets AI work here"
    ));
}

#[test]
fn a_machine_with_no_scope_open_is_closed_by_every_scope_and_still_answers() {
    // No config at all: the ordinary state of a machine nobody has run
    // `warlock open` on. It is an answer, not a failure.
    let home = a_dir();
    let closed = answer(home.path(), "crates/engine");

    assert_eq!(closed.open, OpenScopes::Nothing);
    assert!(!closed.opens);
    assert_eq!(
        object(&closed).to_string(),
        r#"{"command":"check","path":"crates/engine","scope":"data-plane","open":[],"opens":false,"tracker":"linear","team":"Data Plane","base_url":null,"project_key":null,"review_state":"In Review","label":"area/data-plane","key":null,"key_found":false}"#
    );
}

#[test]
fn a_config_that_will_not_parse_is_unknown_and_never_an_empty_open_list() {
    // The case `[]` would lie about: an operator told no scope is open when
    // the truth is that warlock could not read which ones are.
    let home = a_dir();
    a_broken_config(home.path());
    let unknown = answer(home.path(), "crates/engine");

    assert_eq!(unknown.open, OpenScopes::Unknown);
    assert!(!unknown.opens, "a config warlock cannot read opens nothing");
    assert_eq!(
        object(&unknown).to_string(),
        r#"{"command":"check","path":"crates/engine","scope":"data-plane","open":null,"opens":false,"tracker":"linear","team":"Data Plane","base_url":null,"project_key":null,"review_state":"In Review","label":"area/data-plane","key":null,"key_found":false}"#
    );
    // And the prose names the file, which is the one thing a reader needs
    // in order to fix it.
    let path = config_path(home.path(), Path::new(REPO));
    assert!(
        prose(&unknown).contains(&format!("`{}` could not be read", path.display())),
        "the unreadable file is not named: {}",
        prose(&unknown)
    );
}

#[test]
fn a_missing_home_is_nothing_open_rather_than_a_config_that_would_not_read() {
    // `Unknown` claims a file on disk is broken. With no home there is no
    // file and no path to name one by, so there is nothing broken to report.
    let nowhere = checked(
        Path::new(REPO),
        None,
        &a_manifest(),
        &Path::new(REPO).join("crates/engine"),
    )
    .expect("a path inside the repository has a manifest form");

    assert_eq!(nowhere.open, OpenScopes::Nothing);
    assert_eq!(nowhere.config, None);
    assert!(!nowhere.opens);
    assert!(prose(&nowhere).contains("AI can work in no scope"));
    // No home is nothing bound rather than unknown, for a reason of its own: a
    // machine with no home has no key store to bind a name to either, so
    // `warlock key use` is the fix here as it is for a home that simply has
    // no binding in it yet. The route the manifest knows is still answered.
    assert_eq!(nowhere.key, None);
    assert!(!nowhere.key_found);
    assert_eq!(nowhere.record, Some(the_linear_record()));
}

#[test]
fn a_path_outside_the_repository_is_refused_rather_than_reported_as_unscoped() {
    // `scope_covering`'s own doc: such a path is not unscoped, it is one
    // this manifest has nothing whatever to say about.
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
    opening(home.path(), &["billing"]);

    assert_eq!(
        object(&answer(home.path(), "crates/engine")).to_string(),
        r#"{"command":"check","path":"crates/engine","scope":"data-plane","open":["billing"],"opens":false,"tracker":"linear","team":"Data Plane","base_url":null,"project_key":null,"review_state":"In Review","label":"area/data-plane","key":null,"key_found":false}"#
    );
}

#[test]
fn nothing_covering_the_path_is_a_null_scope_rather_than_a_missing_field() {
    let home = a_dir();

    assert_eq!(
        object(&answer(home.path(), "docs/adr")).to_string(),
        r#"{"command":"check","path":"docs/adr","scope":null,"open":[],"opens":false,"tracker":null,"team":null,"base_url":null,"project_key":null,"review_state":null,"label":null,"key":null,"key_found":false}"#
    );
}

#[test]
fn a_binding_on_another_scope_of_the_repository_is_not_reported_for_an_unscoped_path() {
    // The case the test above cannot reach, because it binds nothing at all: a
    // home that binds `data-plane` to a name this machine really stores, asked
    // about a path no scope covers. A key is bound per scope, so there is no
    // scope here to read a binding for, and reporting the one `data-plane`
    // holds would name a key that has nothing to do with this path.
    let home = a_dir();
    bound(home.path(), "data-plane", "work");
    keeping(home.path(), "work");

    let printed = object(&answer(home.path(), "docs/adr")).to_string();

    assert_eq!(
        printed,
        r#"{"command":"check","path":"docs/adr","scope":null,"open":[],"opens":false,"tracker":null,"team":null,"base_url":null,"project_key":null,"review_state":null,"label":null,"key":null,"key_found":false}"#
    );
    // Neither the name nor, ever, the value behind it.
    assert!(!printed.contains("work"), "{printed}");
    assert!(!printed.contains(NOT_A_KEY), "{printed}");
}

#[test]
fn no_object_carries_a_root_or_a_home() {
    // Deliberate: an absolute machine path is the one non-reproducible thing
    // in a document meant to be diffable across machines. The home is worse
    // — it names a person.
    let home = a_dir();
    opening(home.path(), &["billing"]);
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

#[test]
fn the_whole_answer_for_a_scoped_recorded_and_bound_checkout_is_one_flat_object() {
    // The shape a consumer is promised, asserted whole rather than field by
    // field: the six record fields and the two key ones sit flat beside the four
    // that were here first, with no nested object to reach through, and
    // `review_state` is spelled as the `[[scope]]` record spells it. The two
    // fields a Linear record has no place for are `null` and not absent, so a
    // script reads one shape whichever board answered.
    let home = a_dir();
    opening(home.path(), &["data-plane"]);
    bound(home.path(), "data-plane", "work");
    keeping(home.path(), "work");

    let printed = object(&answer(home.path(), "crates/engine/src")).to_string();

    assert_eq!(
        printed,
        r#"{"command":"check","path":"crates/engine/src","scope":"data-plane","open":["data-plane"],"opens":true,"tracker":"linear","team":"Data Plane","base_url":null,"project_key":null,"review_state":"In Review","label":"area/data-plane","key":"work","key_found":true}"#
    );
    // The name is printed and the value it stands for never is. Asserted on
    // the one answer that had to read the store to say `true`, because that is
    // the answer a leak could come through.
    assert!(!printed.contains(NOT_A_KEY), "{printed}");
}

#[test]
fn a_jira_record_answers_in_its_own_fields_and_exits_as_a_linear_one_does() {
    // A check reports, so the board warlock cannot file to is said in that
    // board's own fields rather than refused: `tracker` names it, `base_url` and
    // `project_key` carry it, `team` is the `null` of a key Jira has no place
    // for, and the status is the 0 a recorded scope always answers with. The
    // object is the Linear one's shape to the key and the order — only the
    // values move.
    let home = a_dir();
    opening(home.path(), &["data-plane"]);
    bound(home.path(), "data-plane", "work");
    keeping(home.path(), "work");

    let answered = jira_answer(home.path(), "crates/engine/src");

    assert_eq!(answered.record, Some(the_jira_record()));
    assert_eq!(
        object(&answered).to_string(),
        r#"{"command":"check","path":"crates/engine/src","scope":"data-plane","open":["data-plane"],"opens":true,"tracker":"jira","team":null,"base_url":"https://acme.atlassian.net","project_key":"PAY","review_state":"In Review","label":"area/data-plane","key":"work","key_found":true}"#
    );
    assert_eq!(
        status_for(
            &checked(
                Path::new(REPO),
                Some(home.path()),
                &a_jira_manifest(),
                &Path::new(REPO).join("crates/engine/src"),
            )
            .map(|_| ())
        ),
        0,
        "a check of a Jira scope refused instead of reporting"
    );
}

#[test]
fn a_covering_scope_with_no_record_routes_to_null_rather_than_to_empty_strings() {
    // `platform` covers this and nobody has filed a `[[scope]]` record for it.
    // `""` would tell a script the work is filed to a team whose name is the
    // empty string, which is a thing a tracker will accept.
    let home = a_dir();
    bound(home.path(), "platform", "work");
    keeping(home.path(), "work");

    assert_eq!(
        object(&answer(home.path(), "crates/tui/src")).to_string(),
        r#"{"command":"check","path":"crates/tui/src","scope":"platform","open":[],"opens":false,"tracker":null,"team":null,"base_url":null,"project_key":null,"review_state":null,"label":null,"key":"work","key_found":true}"#
    );
}

#[test]
fn the_three_key_states_are_told_apart_by_the_name_and_never_by_the_boolean() {
    // Nothing bound and a name nothing answers to are both `key_found: false`
    // and are fixed in different places — `warlock key use` against the
    // checkout's config, `warlock key add` against the store — so `key` is
    // what tells them apart.
    let unbound = a_dir();
    let checked = answer(unbound.path(), "crates/engine");
    assert_eq!(checked.key, None);
    assert!(!checked.key_found);

    // Bound to a name this machine has never stored: the store is not there at
    // all, which is the same answer as a store without that name in it.
    let dangling = a_dir();
    bound(dangling.path(), "data-plane", "work");
    let checked = answer(dangling.path(), "crates/engine");
    assert_eq!(checked.key.as_deref(), Some("work"));
    assert!(!checked.key_found);

    // A store that holds other names and not this one.
    keeping(dangling.path(), "personal");
    let checked = answer(dangling.path(), "crates/engine");
    assert_eq!(checked.key.as_deref(), Some("work"));
    assert!(!checked.key_found);

    keeping(dangling.path(), "work");
    let checked = answer(dangling.path(), "crates/engine");
    assert_eq!(checked.key.as_deref(), Some("work"));
    assert!(checked.key_found);
}

#[test]
fn a_closed_scope_still_carries_the_whole_route_beside_the_closed_verdict() {
    // The decision this implements: warlock says what is being crossed and
    // goes on saying everything else it knows. Blanking the route because the
    // scope is closed on this machine would leave somebody covering for a
    // colleague with a verdict and nowhere to file.
    let home = a_dir();
    opening(home.path(), &["platform"]);
    bound(home.path(), "data-plane", "work");
    keeping(home.path(), "work");

    let closed = answer(home.path(), "crates/engine");

    assert!(!closed.opens);
    assert_eq!(closed.record, Some(the_linear_record()));
    assert_eq!(closed.key.as_deref(), Some("work"));
    assert!(closed.key_found);
}

// The seven prose cases below assert the whole answer rather than a line of it,
// because the composition is the thing under test: five lines, always in that
// order, with the repository's two facts above this checkout's two. A
// `contains` would pass on an answer that had silently lost a line.
#[test]
fn the_prose_for_a_scoped_recorded_and_bound_checkout_says_the_route_and_the_key() {
    let home = a_dir();
    opening(home.path(), &["data-plane"]);
    bound(home.path(), "data-plane", "work");
    keeping(home.path(), "work");

    let answered = answer(home.path(), "crates/engine/src");

    assert_eq!(
        prose(&answered),
        "`crates/engine/src` is scoped `data-plane`\n\
         work here is filed to `Data Plane`, labelled `area/data-plane`, and a finished pull \
         moves the ticket to `In Review`\n\
         AI can work in `data-plane`\n\
         `data-plane` is open on this machine\n\
         filing to `Data Plane` would use the key `work`, which this machine stores"
    );
    // The one answer that had to read the key store to say the name resolves,
    // which is the answer a leak would come through.
    assert!(
        !prose(&answered).contains(NOT_A_KEY),
        "{}",
        prose(&answered)
    );
}

#[test]
fn the_prose_for_a_jira_record_names_the_project_and_the_site_and_never_says_no_record() {
    // The bug this fixes: a Jira record has no team, and a route line that
    // matched the team as part of one group fell through to "`data-plane` has no
    // `[[scope]]` record", which is false. The line names both halves of the
    // address — one project key names a different project on every site — and
    // the key line pairs the key with the project where the Linear case above
    // pairs it with the team. The same five lines in the same order:
    // nothing but the route line and the name on the key line differs.
    let home = a_dir();
    opening(home.path(), &["data-plane"]);
    bound(home.path(), "data-plane", "work");
    keeping(home.path(), "work");

    let said = prose(&jira_answer(home.path(), "crates/engine/src"));

    assert_eq!(
        said,
        "`crates/engine/src` is scoped `data-plane`\n\
         work here is filed to `PAY` at `https://acme.atlassian.net`, labelled \
         `area/data-plane`, and a finished pull moves the ticket to `In Review`\n\
         AI can work in `data-plane`\n\
         `data-plane` is open on this machine\n\
         filing to `PAY` would use the key `work`, which this machine stores"
    );
    assert!(!said.contains("no `[[scope]]` record"), "{said}");
    assert!(!said.contains(NOT_A_KEY), "{said}");
}

#[test]
fn a_closed_scope_keeps_its_whole_route_in_the_prose_beside_the_closed_line() {
    // The decision this implements: the closed line sits beside the route and
    // never in place of it. Somebody covering for a colleague is told what they
    // would be crossing and still where the work files.
    let home = a_dir();
    opening(home.path(), &["platform"]);
    bound(home.path(), "data-plane", "work");
    keeping(home.path(), "work");

    assert_eq!(
        prose(&answer(home.path(), "crates/engine")),
        "`crates/engine` is scoped `data-plane`\n\
         work here is filed to `Data Plane`, labelled `area/data-plane`, and a finished pull \
         moves the ticket to `In Review`\n\
         AI can work in `platform`\n\
         `data-plane` is closed on this machine — `warlock open data-plane` lets AI work here\n\
         filing to `Data Plane` would use the key `work`, which this machine stores"
    );
}

#[test]
fn a_path_no_scope_covers_says_there_is_nothing_to_route_to() {
    let home = a_dir();

    assert_eq!(
        prose(&answer(home.path(), "docs/adr")),
        "nothing scopes `docs/adr`\n\
         there is nothing to route to: a scope covering this path is what would fix it, \
         with `warlock scope add`\n\
         AI can work in no scope\n\
         an unscoped path is closed to everyone, so no AI may work here\n\
         a key is bound to a scope, so there is none to report and none to bind until a scope \
         covers this path"
    );
    // No scope to name and no `warlock key use` to spell: the command takes a
    // scope argument, and there is none here to put in it.
    let said = prose(&answer(home.path(), "docs/adr"));
    assert!(!said.contains("warlock key use"), "{said}");
}

#[test]
fn a_covering_scope_nobody_recorded_says_which_scope_is_missing_a_record() {
    // `platform` covers this and has no `[[scope]]` record. The line names the
    // scope and the file, because those are the two things somebody filing the
    // record needs and neither is in the rest of the answer.
    let home = a_dir();
    bound(home.path(), "platform", "work");
    keeping(home.path(), "work");

    assert_eq!(
        prose(&answer(home.path(), "crates/tui/src")),
        "`crates/tui/src` is scoped `platform`\n\
         `platform` has no `[[scope]]` record, so there is nothing to route to: a record \
         in `.warlock/scopes.toml` is what would fix it\n\
         AI can work in no scope\n\
         `platform` is closed on this machine — `warlock open platform` lets AI work here\n\
         the key `work` is bound here and stored on this machine"
    );
}

#[test]
fn an_unbound_checkout_is_sent_to_key_use_and_key_add_and_a_dangling_name_to_key_add() {
    // The two absences are fixed in different files by different commands, so
    // the lines differ: nothing bound is the checkout's config, a name the
    // store has never heard of is the store. Both are exit 0 and both print the
    // route above them, and both name the covering scope — the binding is per
    // scope, so `warlock key use` is spelled with the scope it would bind.
    let unbound = a_dir();
    opening(unbound.path(), &["data-plane"]);

    assert_eq!(
        prose(&answer(unbound.path(), "crates/engine")),
        "`crates/engine` is scoped `data-plane`\n\
         work here is filed to `Data Plane`, labelled `area/data-plane`, and a finished pull \
         moves the ticket to `In Review`\n\
         AI can work in `data-plane`\n\
         `data-plane` is open on this machine\n\
         no key is bound to the scope `data-plane`: `warlock key add <name>` stores a new key \
         on this machine, and a name it already stores is bound with `warlock key use \
         data-plane <name>`"
    );

    let dangling = a_dir();
    opening(dangling.path(), &["data-plane"]);
    bound(dangling.path(), "data-plane", "work");
    keeping(dangling.path(), "personal");

    assert_eq!(
        prose(&answer(dangling.path(), "crates/engine")),
        "`crates/engine` is scoped `data-plane`\n\
         work here is filed to `Data Plane`, labelled `area/data-plane`, and a finished pull \
         moves the ticket to `In Review`\n\
         AI can work in `data-plane`\n\
         `data-plane` is open on this machine\n\
         the key `work` is bound to the scope `data-plane` and this machine has not stored it: \
         `warlock key add work` stores it, `warlock key use data-plane <name>` binds another"
    );
}

// The gate stands in a temporary repository on disk rather than at `REPO`,
// because it loads the manifest itself: that is the composition under test —
// root, home, manifest and joined path becoming parameters in one place — and a
// gate that read the developer's own home would be answering about their open
// scopes. No socket is opened and no model is run; the manifest and one config
// are the whole of what is read.
fn gating(repo: &Path, home: &Path, path: &str) -> Result<(), Error> {
    a_manifest().save(repo).expect("a manifest that saves");
    gated_onto(&standing_in(repo), Some(home), PathBuf::from(path))
}

#[test]
fn a_gate_over_a_closed_scope_is_the_boundarys_three_and_names_the_scope() {
    // The outer scope open and not the nearer one: the ordinary way to be
    // closed while something is open.
    let (repo, home) = (a_dir(), a_dir());
    opening_in(home.path(), repo.path(), &["platform"]);

    let refused = gating(repo.path(), home.path(), "crates/engine/src/lib.rs");

    // The register the headless writes already refuse in, and no new status:
    // one line and a **3**, which re-running will never turn into a 0.
    assert_eq!(status_for(&refused), 3);
    let error = refused.expect_err("a scope closed on this machine is refused");
    assert!(
        matches!(
            &error,
            Error::ClosedScope { path, scope }
                if path == "crates/engine/src/lib.rs" && scope == "data-plane"
        ),
        "{error:?}"
    );
    assert!(!error.to_string().contains('\n'), "`main` prints one line");
    // Repository-relative and never the absolute path handed in: a hook shows
    // this line to somebody, and an absolute one names their home back at them.
    assert!(
        !error
            .to_string()
            .contains(&repo.path().display().to_string()),
        "{error}"
    );

    // And a machine with no scope open at all, which is every machine nobody
    // has run `warlock open` on.
    let nothing = a_dir();
    assert_eq!(
        status_for(&gating(repo.path(), nothing.path(), "crates/engine")),
        3
    );
}

#[test]
fn a_gate_over_an_open_scope_is_a_zero() {
    // Nothing is printed — there is no writer to print through — so a hook's
    // stdout stays empty and a shell gets no envelope it would have to parse.
    let (repo, home) = (a_dir(), a_dir());
    opening_in(home.path(), repo.path(), &["data-plane"]);

    for path in ["crates/engine", "crates/engine/src/lib.rs"] {
        let gated = gating(repo.path(), home.path(), path);
        assert_eq!(status_for(&gated), 0, "{path}");
        gated.unwrap_or_else(|error| panic!("{path} is open: {error}"));
    }
}

#[test]
fn a_gate_over_an_unscoped_path_is_the_boundarys_three_and_says_to_draw_a_scope() {
    // The wildcard opens every scope and still not a path under none: work
    // happens only inside a scope.
    let (repo, home) = (a_dir(), a_dir());
    opening_in(home.path(), repo.path(), &["*"]);

    for path in ["docs/adr", "vendor/thing/file.rs"] {
        let refused = gating(repo.path(), home.path(), path);

        assert_eq!(status_for(&refused), 3, "{path}");
        let error = refused.expect_err("an unscoped path is closed to everyone");
        assert!(
            matches!(&error, Error::Unscoped { path: refused } if refused == path),
            "{error:?}"
        );
        assert!(
            error.to_string().contains("warlock scope add"),
            "the way out is drawing a scope: {error}"
        );
    }
}

#[test]
fn every_half_finished_route_is_still_a_zero() {
    // A check answers; it does not gate. The non-zero status is left for `jq
    // -e '.opens and .key_found'` to spend, so each of these four — a scope
    // closed on this machine, nothing bound, a name the store has never
    // heard of, a path no scope covers — is output and a 0.
    let closed = a_dir();
    opening(closed.path(), &["platform"]);

    let dangling = a_dir();
    bound(dangling.path(), "data-plane", "work");

    for (home, path) in [
        (&closed, "crates/engine"),
        (&a_dir(), "crates/engine"),
        (&dangling, "crates/engine"),
        (&a_dir(), "docs/adr"),
    ] {
        assert_eq!(
            status_for(
                &checked(
                    Path::new(REPO),
                    Some(home.path()),
                    &a_manifest(),
                    &Path::new(REPO).join(path),
                )
                .map(|_| ())
            ),
            0,
            "{path} under {}",
            home.path().display()
        );
    }
}

// The hook form stands where the gate form stands and for the same reasons:
// `hooked_onto` is what `check::hook` calls once it has read stdin, so a canned
// payload here runs the code a `PreToolUse` hook runs. What is read is the
// manifest in the temporary repository and one config under the temporary
// home; no socket is opened, no model is run and no real home is touched.
fn hooking(repo: &Path, home: &Path, payload: &str) -> (String, Result<(), Error>) {
    a_manifest().save(repo).expect("a manifest that saves");

    let mut out = Vec::new();
    let outcome = hooked_onto(&standing_in(repo), Some(home), payload.as_bytes(), &mut out);
    (
        String::from_utf8(out).expect("warlock writes its own text"),
        outcome,
    )
}

// The payload Claude Code sends, with the fields the gate does not read left in
// rather than trimmed to the two it does: that is what makes the read a walk down
// somebody else's schema instead of a deserialisation of a type warlock owns.
// `file_path` arrives absolute, as a hook sends it.
fn a_payload(repo: &Path, path: &str) -> String {
    json!({
        "session_id": "0f6b",
        "cwd": repo,
        "hook_event_name": "PreToolUse",
        "tool_name": "Edit",
        "tool_input": {
            "file_path": repo.join(path),
            "old_string": "before",
            "new_string": "after",
        },
    })
    .to_string()
}

#[test]
fn a_hook_over_a_closed_scope_writes_the_deny_object_and_still_exits_zero() {
    // The gate form's closed case, asked the hook's way: the outer scope open
    // and not the nearer one.
    let (repo, home) = (a_dir(), a_dir());
    opening_in(home.path(), repo.path(), &["platform"]);

    let (written, outcome) = hooking(
        repo.path(),
        home.path(),
        &a_payload(repo.path(), "crates/engine/src/lib.rs"),
    );

    // To the byte, because all four of these field names are Claude Code's: an
    // object one letter out is not a weaker refusal, it is a hook that permits
    // every write silently. One line, nested under `hookSpecificOutput`, with no
    // `command` field — this is not warlock's envelope — and in insertion order.
    assert_eq!(
        written,
        "{\"hookSpecificOutput\":{\"hookEventName\":\"PreToolUse\",\
         \"permissionDecision\":\"deny\",\"permissionDecisionReason\":\"warlock: \
         `crates/engine/src/lib.rs` is scoped `data-plane`, which is closed on this machine — \
         `warlock open data-plane` lets AI work here\"}}\n"
    );
    // The reason names the path the repository's way and not the machine's: it
    // is shown to somebody, and the absolute one names their home back at them.
    assert!(
        !written.contains(&repo.path().display().to_string()),
        "{written}"
    );
    // A 0 and not the path form's **3**: 2 is the only status Claude Code honours
    // from a hook, so the refusal travels in the object and the status says
    // nothing about the boundary.
    assert_eq!(status_for(&outcome), 0);
    outcome.expect("a hook denies by writing, not by failing");
}

#[test]
fn a_hook_over_an_open_scope_writes_nothing() {
    // Saying nothing is how a `PreToolUse` hook permits, and a stdout with an
    // object of any kind on it is a write stopped.
    let (repo, home) = (a_dir(), a_dir());
    opening_in(home.path(), repo.path(), &["data-plane"]);

    for path in ["crates/engine", "crates/engine/src/lib.rs"] {
        let (written, outcome) = hooking(repo.path(), home.path(), &a_payload(repo.path(), path));

        assert!(written.is_empty(), "{path} was refused: {written}");
        assert_eq!(status_for(&outcome), 0, "{path}");
        outcome.unwrap_or_else(|error| panic!("{path} is open: {error}"));
    }
}

#[test]
fn a_hook_over_an_unscoped_path_writes_the_deny_object_that_says_to_draw_a_scope() {
    let (repo, home) = (a_dir(), a_dir());
    opening_in(home.path(), repo.path(), &["*"]);

    let (written, outcome) = hooking(
        repo.path(),
        home.path(),
        &a_payload(repo.path(), "docs/adr/0001.md"),
    );

    assert_eq!(
        written,
        "{\"hookSpecificOutput\":{\"hookEventName\":\"PreToolUse\",\
         \"permissionDecision\":\"deny\",\"permissionDecisionReason\":\"warlock: \
         nothing scopes `docs/adr/0001.md`, so no AI may work there — draw a scope with \
         `warlock scope add`\"}}\n"
    );
    assert_eq!(status_for(&outcome), 0);
    outcome.expect("a hook denies by writing, not by failing");
}

#[test]
fn a_payload_with_no_path_in_it_writes_nothing_and_exits_zero() {
    // No scope open, so every path in this manifest is closed: a payload the
    // read *did* understand would deny, which is what leaves the read itself as
    // the only thing under test.
    let (repo, home) = (a_dir(), a_dir());

    for payload in [
        // Not JSON at all, which is a hook wired to the wrong stream.
        "not json {{{".to_owned(),
        // JSON and not an object.
        "[]".to_owned(),
        // An object with no `tool_input`: a tool that writes nothing, near
        // enough, and one every `PreToolUse` hook is handed.
        json!({ "hook_event_name": "PreToolUse", "tool_name": "WebFetch" }).to_string(),
        // An input with no path in it. `Bash` is the one that can write past
        // this gate anyway, which is accepted.
        json!({ "tool_input": { "command": "rm -rf crates/engine" } }).to_string(),
        // A `file_path` that is not a string, which no schema promises it will
        // never become.
        json!({ "tool_input": { "file_path": 7 } }).to_string(),
    ] {
        let (written, outcome) = hooking(repo.path(), home.path(), &payload);

        assert!(written.is_empty(), "{payload} was refused: {written}");
        assert_eq!(status_for(&outcome), 0, "{payload}");
        outcome.unwrap_or_else(|error| panic!("{payload}: {error}"));
    }
}

#[test]
fn an_empty_file_path_is_not_a_write_to_the_repository_root() {
    // The one absence with a consequence of its own, so it gets a repository of
    // its own: the payload's path is joined onto the working directory, so an
    // empty one would be the directory itself, and here that directory carries a
    // scope closed on this machine. A read that let the empty string through
    // would deny a write nobody can find.
    let (repo, home) = (a_dir(), a_dir());
    Manifest::with_directories([entry(".", "warlock-team")])
        .save(repo.path())
        .expect("a manifest that saves");

    let empty = json!({ "tool_input": { "file_path": "" } }).to_string();
    let mut out = Vec::new();
    hooked_onto(
        &standing_in(repo.path()),
        Some(home.path()),
        empty.as_bytes(),
        &mut out,
    )
    .expect("a payload with no path in it permits");

    assert!(out.is_empty(), "the root was refused: {out:?}");

    // And the same repository denies a path it can name, so the silence above is
    // the empty string's doing.
    let named =
        json!({ "tool_input": { "file_path": repo.path().join("src/lib.rs") } }).to_string();
    let mut out = Vec::new();
    hooked_onto(
        &standing_in(repo.path()),
        Some(home.path()),
        named.as_bytes(),
        &mut out,
    )
    .expect("a hook denies by writing, not by failing");

    assert!(
        String::from_utf8(out)
            .expect("warlock writes its own text")
            .contains("\"permissionDecision\":\"deny\""),
        "a scope closed on this machine should have refused"
    );
}

#[test]
fn a_write_warlock_cannot_name_a_scope_for_writes_nothing() {
    // The two absences that are not the payload's: a path outside the repository
    // the hook is standing in, which has no repository-relative spelling at all,
    // and a manifest that will not parse, which is a repository whose scopes are
    // unknown. `warlock check` refuses the second with a **1**; a hook has no
    // status to refuse with and nowhere to put a sentence, and a gate that cannot
    // name the scope covering a write cannot refuse it either.
    let (repo, home) = (a_dir(), a_dir());

    let outside = json!({ "tool_input": { "file_path": "/elsewhere/file.rs" } }).to_string();
    let (written, outcome) = hooking(repo.path(), home.path(), &outside);
    assert!(written.is_empty(), "{written}");
    outcome.expect("a path outside the repository permits");

    let warlock = repo.path().join(".warlock");
    fs::write(warlock.join("scopes.toml"), "not toml {{{").expect("a broken manifest");

    let mut out = Vec::new();
    hooked_onto(
        &standing_in(repo.path()),
        Some(home.path()),
        a_payload(repo.path(), "crates/engine/src/lib.rs").as_bytes(),
        &mut out,
    )
    .expect("a manifest that will not parse permits");

    assert!(out.is_empty(), "a broken manifest refused: {out:?}");
}
