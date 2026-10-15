use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use warlock_engine::{
    Manifest, Node, ScopeRecord, ScopedDirectory, Tree, manifest_path, route_facts, save_sigils,
    validate_scope,
};

use super::{Opened, scoped_line, unscoped_line};
use crate::app::App;
// The sentence itself, asked of the one function that writes it rather than
// retyped: the footer and the shell refuse the same boundary in the same
// words, and a test holding a copy of those words is a test that would go on
// passing while the two doors drifted apart.
use crate::boundary::closed_scope_message;
use crate::error::Error;
use crate::error::status_for;
use crate::prompt::ScopePrompt;
use crate::rescope::RecordFields;
// The other door onto the boundary, pressed here so that the two are held to
// one answer in one place.
use crate::scoping::scope_press;
use crate::session::load_manifest;
use crate::settings::Settings;

// Every test here builds both its repository and its home out of one of
// these, so nothing goes near the developer's real home.
fn a_dir() -> tempfile::TempDir {
    tempfile::tempdir().expect("a temporary directory")
}

fn row(module: &str, scope: &str) -> ScopedDirectory {
    ScopedDirectory::new(".", module, scope).expect("a relative module path is inside the root")
}

// `docs` has no row and nothing above it does, so nothing covers it.
fn a_manifest() -> Manifest {
    Manifest::with_directories([
        row("crates", "platform"),
        row("crates/engine", "data-plane"),
    ])
}

fn a_repository() -> tempfile::TempDir {
    let repo = a_dir();
    a_manifest()
        .save(repo.path())
        .expect("a manifest that saves");
    repo
}

fn holding(home: &Path, repo_root: &Path, sigils: &[&str]) {
    let sigils: Vec<String> = sigils.iter().map(|sigil| (*sigil).to_owned()).collect();
    save_sigils(home, repo_root, &sigils).expect("a config that writes");
}

// Bytes rather than a parsed `Manifest`, because what a refusal promises is
// that the file did not change — not that it still parses to something equal.
fn manifest_bytes(repo_root: &Path) -> Option<Vec<u8>> {
    fs::read(manifest_path(repo_root)).ok()
}

// The production road exactly, with what the environment would have settled
// handed in instead. Both writes below reach their edit through this and only
// through this, so there is no way to the second half without the first.
fn open(repo_root: &Path, home: &Path, path: &str) -> Result<Opened, Error> {
    let manifest = load_manifest(repo_root).expect("a manifest that reads");
    Opened::new(
        repo_root.to_path_buf(),
        Some(home),
        manifest,
        repo_root.join(path),
    )
}

fn scope_add(
    repo_root: &Path,
    home: &Path,
    path: &str,
    scope: &str,
    flags: RecordFields<'_>,
) -> Result<String, Error> {
    open(repo_root, home, path)?.scoped(scope, flags)
}

// The three flags a name nothing records has to be given, as one value, so a
// test about something else says what it is about rather than filling a form.
fn a_record() -> RecordFields<'static> {
    RecordFields {
        team_key: Some("Billing"),
        review_state: Some("In Review"),
        label: Some("area/billing"),
    }
}

fn scope_remove(repo_root: &Path, home: &Path, path: &str) -> Result<String, Error> {
    open(repo_root, home, path)?.unscoped()
}

fn stored_scope(repo_root: &Path, module: &str) -> Option<String> {
    load_manifest(repo_root)
        .expect("a manifest that reads")
        .directory(module)
        .map(|directory| directory.scope().to_owned())
}

fn paths(manifest: &Manifest) -> Vec<&str> {
    manifest
        .directories()
        .iter()
        .map(ScopedDirectory::path)
        .collect()
}

#[test]
fn a_machine_holding_nothing_is_refused_by_every_scope_it_meets() {
    // No config at all: the ordinary state of a machine nobody has run
    // `warlock sigil add` on, and the one an agent in a fresh checkout is in.
    let repo = a_repository();
    let home = a_dir();
    let before = manifest_bytes(repo.path()).expect("a manifest on disk");

    for path in ["crates", "crates/engine", "crates/engine/src"] {
        let error = scope_remove(repo.path(), home.path(), path)
            .expect_err("holding nothing opens nothing that is scoped");
        assert!(
            matches!(error, Error::ClosedScope { .. }),
            "{path}: {error:?}"
        );
    }
    assert_eq!(manifest_bytes(repo.path()).as_deref(), Some(&before[..]));
}

#[test]
fn anyone_may_draw_the_first_scope_over_a_directory_nothing_covers() {
    // A scope is the only thing that opens a directory to work, so if drawing
    // the first one also needed a scope, no repository could ever be set up.
    let repo = a_repository();
    let home = a_dir();

    let said = scope_add(repo.path(), home.path(), "docs", "billing", a_record())
        .expect("nothing scopes `docs`");

    assert_eq!(said, "docs is scoped `billing`");
    assert_eq!(status_for(&Ok(())), 0);
    assert_eq!(
        stored_scope(repo.path(), "docs").as_deref(),
        Some("billing")
    );

    // Every other row untouched and in the order it was in, and the new one on
    // the end, so the diff against what was there is the one row.
    let after = load_manifest(repo.path()).expect("a manifest that reads");
    let before = a_manifest();
    assert_eq!(paths(&after), ["crates", "crates/engine", "docs"]);
    for module in ["crates", "crates/engine"] {
        assert_eq!(
            after.directory(module),
            before.directory(module),
            "{module}"
        );
    }
}

#[test]
fn a_scope_that_replaces_another_says_whose_boundary_it_moved() {
    let repo = a_repository();
    let home = a_dir();
    holding(home.path(), repo.path(), &["data-plane"]);

    let said = scope_add(
        repo.path(),
        home.path(),
        "crates/engine",
        "billing",
        a_record(),
    )
    .expect("the machine holds the scope covering this directory");

    // A boundary that moved is named, because a script that quietly redrew
    // somebody else's says whose.
    assert_eq!(said, "crates/engine is scoped `billing` — was `data-plane`");
    assert_eq!(
        stored_scope(repo.path(), "crates/engine").as_deref(),
        Some("billing")
    );
    // And re-writing the scope a directory already carries has nothing to
    // report about a boundary nobody moved.
    assert_eq!(
        scoped_line("docs", "billing", Some("billing")),
        "docs is scoped `billing`"
    );
}

#[test]
fn what_was_given_is_folded_before_it_is_judged_and_stored() {
    // `Data-Plane` and `data-plane` are one boundary, and folding belongs to
    // the caller that took the string from a person — the judge refuses a
    // capital outright, as the assertion below shows.
    let repo = a_repository();
    let home = a_dir();

    let said = scope_add(repo.path(), home.path(), "docs", "Data-Plane", a_record())
        .expect("the fold happened before the judge");

    assert!(validate_scope("Data-Plane").is_err());
    assert_eq!(said, "docs is scoped `data-plane`");
    assert_eq!(
        stored_scope(repo.path(), "docs").as_deref(),
        Some("data-plane")
    );
}

#[test]
fn removing_a_scope_drops_its_row_and_leaves_the_one_above() {
    let repo = a_repository();
    let home = a_dir();
    holding(home.path(), repo.path(), &["data-plane"]);

    let said = scope_remove(repo.path(), home.path(), "crates/engine")
        .expect("the machine holds the scope covering this directory");

    assert_eq!(said, "crates/engine is no longer scoped — was `data-plane`");
    assert_eq!(status_for(&Ok(())), 0);
    assert_eq!(stored_scope(repo.path(), "crates/engine"), None);
    assert_eq!(
        stored_scope(repo.path(), "crates").as_deref(),
        Some("platform")
    );
}

#[test]
fn removing_a_scope_from_a_directory_that_carries_none_is_success_and_writes_the_same_file() {
    // Idempotence, said as a fact rather than as a refusal: the command's
    // job is to make "this directory carries no scope" true, and it already
    // was.
    let repo = a_repository();
    let home = a_dir();
    let before = manifest_bytes(repo.path()).expect("a manifest on disk");

    let said = scope_remove(repo.path(), home.path(), "docs").expect("nothing scopes `docs`");

    assert_eq!(said, "docs carried no scope");
    assert_eq!(status_for(&Ok(())), 0);
    assert_eq!(
        manifest_bytes(repo.path()).as_deref(),
        Some(&before[..]),
        "an idempotent clear rewrote the file differently"
    );
}

// `third-party` is named by no row at all, and `data-plane` loses the only row
// that named it to the clear below: a writer that pruned the records to what the
// rows spell would drop both, and where work under a scope is filed is not a
// clear's to forget.
fn records() -> Vec<ScopeRecord> {
    vec![
        ScopeRecord::new("data-plane", "Data Plane", "In Review", "area/data-plane"),
        ScopeRecord::new("third-party", "Vendor", "Triage", "area/vendor"),
    ]
}

fn a_repository_of_records() -> tempfile::TempDir {
    let repo = a_dir();
    a_manifest()
        .with_scopes(records())
        .save(repo.path())
        .expect("a manifest that saves");
    repo
}

// The written bytes from the first `[[scope]]` table on, because the order the
// records come back in and the order they are written in are two claims.
fn record_bytes(repo_root: &Path) -> String {
    let text = String::from_utf8(manifest_bytes(repo_root).expect("a manifest on disk"))
        .expect("a manifest is UTF-8");
    let at = text.find("[[scope]]").unwrap_or(text.len());
    text[at..].to_owned()
}

#[test]
fn both_scope_writes_leave_the_records_where_they_found_them() {
    let repo = a_repository_of_records();
    let home = a_dir();
    holding(home.path(), repo.path(), &["platform", "data-plane"]);
    let before = record_bytes(repo.path());
    assert!(before.contains("third-party"), "the fixture has records");

    // A name the fixture already records, so this is the flagless road and the
    // claim is the one the test makes: a write that had no record to create
    // leaves the two that were there where it found them.
    scope_add(
        repo.path(),
        home.path(),
        "docs",
        "data-plane",
        RecordFields::default(),
    )
    .expect("nothing scopes `docs`");
    assert_eq!(
        record_bytes(repo.path()),
        before,
        "`scope add` moved a record"
    );

    scope_remove(repo.path(), home.path(), "crates/engine").expect("the machine holds the scope");
    scope_remove(repo.path(), home.path(), "docs").expect("the machine holds the scope");
    assert_eq!(
        record_bytes(repo.path()),
        before,
        "`scope remove` moved a record",
    );
    let after = load_manifest(repo.path()).expect("a manifest that reads");
    assert_eq!(
        paths(&after),
        ["crates"],
        "no row names `data-plane` any more"
    );
    assert_eq!(after.scopes(), records());
}

// Spaces on both ends and kept there: blank is judged on a trimmed copy while
// the untrimmed string is what gets stored, so a team named as it is named in
// somebody's tracker is written back exactly.
const TEAM: &str = " Billing Team ";

#[test]
fn every_scope_refusal_is_one_line_a_1_and_nothing_written() {
    // The rules are `rescope`'s and tested there; what the shell adds is that
    // each refusal reaches `main` as one line, the ordinary 1 rather than
    // clap's 2 or the boundary's 3, with the file left as it was.
    let repo = a_repository_of_records();
    let home = a_dir();
    let before = manifest_bytes(repo.path()).expect("a manifest on disk");

    for refused in [
        scope_add(
            repo.path(),
            home.path(),
            "docs",
            "Control Plane",
            a_record(),
        ),
        scope_add(repo.path(), home.path(), "docs", "", a_record()),
        scope_add(
            repo.path(),
            home.path(),
            "docs",
            "billing",
            RecordFields::default(),
        ),
        scope_add(
            repo.path(),
            home.path(),
            "docs",
            "billing",
            RecordFields {
                team_key: Some(" "),
                ..a_record()
            },
        ),
        scope_add(repo.path(), home.path(), "docs", "data-plane", a_record()),
    ] {
        let error = refused.expect_err("each of these is refused");
        let Error::Scope { refusal } = &error else {
            panic!("a scope rule was refused as something else: {error:?}");
        };
        assert!(
            error.to_string().starts_with(&refusal.to_string()),
            "{error}"
        );
        assert!(!error.to_string().contains('\n'), "`main` prints one line");
        assert_eq!(status_for(&Err(error)), 1);
    }

    assert_eq!(manifest_bytes(repo.path()).as_deref(), Some(&before[..]));
}

#[test]
fn a_name_nothing_records_gets_its_scope_and_its_record_from_one_write() {
    let repo = a_repository_of_records();
    let home = a_dir();
    let before = record_bytes(repo.path());

    let said = scope_add(
        repo.path(),
        home.path(),
        "docs",
        "billing",
        RecordFields {
            team_key: Some(TEAM),
            review_state: Some("In Review"),
            label: Some("area/billing"),
        },
    )
    .expect("nothing records `billing`");

    assert_eq!(said, "docs is scoped `billing`");
    assert_eq!(status_for(&Ok(())), 0);
    let after = load_manifest(repo.path()).expect("a manifest that reads");
    assert_eq!(
        after.directory("docs").map(ScopedDirectory::scope),
        Some("billing")
    );
    // The name on the record is the folded name on the row, and the three
    // values are what was typed — including the spaces around the team, which
    // nothing here is entitled to correct.
    assert_eq!(
        after.scopes(),
        [
            records(),
            vec![ScopeRecord::new(
                "billing",
                TEAM,
                "In Review",
                "area/billing"
            )]
        ]
        .concat()
    );
    // The two records the fixture had, still first and still byte for byte
    // what they were: a record already in the file is neither rewritten nor
    // moved by one being created beside it.
    assert!(
        record_bytes(repo.path()).starts_with(&before),
        "a record that was there moved or changed"
    );

    let untouched = a_manifest();
    for module in ["crates", "crates/engine"] {
        assert_eq!(
            after.directory(module),
            untouched.directory(module),
            "{module}"
        );
    }
}

#[test]
fn a_name_something_records_is_written_flagless_and_gains_no_second_record() {
    let repo = a_repository_of_records();
    let home = a_dir();
    let before = record_bytes(repo.path());

    let said = scope_add(
        repo.path(),
        home.path(),
        "docs",
        "data-plane",
        RecordFields::default(),
    )
    .expect("`data-plane` already has a record");

    assert_eq!(said, "docs is scoped `data-plane`");
    assert_eq!(status_for(&Ok(())), 0);
    let after = load_manifest(repo.path()).expect("a manifest that reads");
    assert_eq!(
        after.directory("docs").map(ScopedDirectory::scope),
        Some("data-plane")
    );
    assert_eq!(after.scopes(), records());
    assert_eq!(record_bytes(repo.path()), before);

    // And the road is chosen by the folded name rather than by what was
    // typed, so a capital does not send an already-recorded scope down the
    // recording road to be refused for flags it should not be asked for. The
    // sigil is held now because the write above drew the boundary: `docs` is
    // scoped `data-plane` from here on, and a second write at it is a write
    // inside somebody's scope.
    holding(home.path(), repo.path(), &["data-plane"]);
    assert_eq!(
        scope_add(
            repo.path(),
            home.path(),
            "docs",
            "Data-Plane",
            RecordFields::default(),
        )
        .expect("`data-plane` is the same name folded"),
        "docs is scoped `data-plane`"
    );
    assert_eq!(record_bytes(repo.path()), before);
}

#[test]
fn a_closed_boundary_answers_before_a_single_flag_is_looked_at() {
    // Whatever the flags said, a scope this machine does not hold is the same
    // refusal. The four shapes below are the four different answers the record
    // checks give, none of which is reached from out here — a refusal naming a
    // record would be a fact about the inside of a manifest the reader has just
    // been told they may not work in.
    let repo = a_repository_of_records();
    let home = a_dir();
    holding(home.path(), repo.path(), &["platform"]);
    let before = manifest_bytes(repo.path()).expect("a manifest on disk");

    for (scope, flags) in [
        ("billing", RecordFields::default()),
        ("billing", a_record()),
        (
            "billing",
            RecordFields {
                team_key: Some("  "),
                ..a_record()
            },
        ),
        ("data-plane", a_record()),
    ] {
        let error = scope_add(repo.path(), home.path(), "crates/engine", scope, flags)
            .expect_err("a scope this machine does not hold refuses an add");

        assert!(
            matches!(error, Error::ClosedScope { .. }),
            "{scope} {flags:?}: {error:?}"
        );
        assert_eq!(
            error.to_string(),
            closed_scope_message("crates/engine", "data-plane")
        );
        assert_eq!(status_for(&Err(error)), 3, "{scope} {flags:?}");
    }

    assert_eq!(manifest_bytes(repo.path()).as_deref(), Some(&before[..]));
}

#[test]
fn the_router_finds_the_team_review_state_and_label_the_flags_wrote() {
    // What `warlock check` prints for the directory just scoped, asked of the
    // router itself rather than of the manifest: a record filed under a name
    // the row does not carry would route nowhere, which is the whole reason
    // the flags are asked for in the same act that writes the name.
    let repo = a_repository_of_records();
    let home = a_dir();

    scope_add(repo.path(), home.path(), "docs", "billing", a_record())
        .expect("nothing records `billing`");

    let written = load_manifest(repo.path()).expect("a manifest that reads");
    let facts = route_facts("docs", repo.path(), &written, None)
        .expect("the module path is inside the root");
    assert_eq!(facts.scope(), Some("billing"));
    let record = facts.record().expect("the router found no record");
    assert_eq!(record.team_key(), "Billing");
    assert_eq!(record.review_state(), "In Review");
    assert_eq!(record.label(), "area/billing");
}

#[test]
fn a_closed_boundary_refuses_both_scope_writes_and_leaves_the_manifest_byte_identical() {
    let repo = a_repository();
    let home = a_dir();
    // The nearest scope wins, so the machine holding the outer boundary is
    // still outside the inner one.
    holding(home.path(), repo.path(), &["platform"]);
    let before = manifest_bytes(repo.path()).expect("a manifest on disk");

    let refusals = [
        scope_add(
            repo.path(),
            home.path(),
            "crates/engine",
            "billing",
            a_record(),
        )
        .expect_err("a scope this machine does not hold refuses an add"),
        scope_remove(repo.path(), home.path(), "crates/engine").expect_err("and refuses a remove"),
    ];

    for error in refusals {
        assert!(
            matches!(error, Error::ClosedScope { .. }),
            "the boundary was refused as something else: {error:?}"
        );
        // The same sentence the footer puts up, asked of the one function
        // that writes it.
        assert_eq!(
            error.to_string(),
            closed_scope_message("crates/engine", "data-plane")
        );
        assert!(!error.to_string().contains('\n'), "`main` prints one line");
        // The refusal's own status: not the 1 warlock spends on something it
        // could not do, because nothing here can be retried into working.
        assert_eq!(status_for(&Err(error)), 3);
    }
    assert_eq!(manifest_bytes(repo.path()).as_deref(), Some(&before[..]));
}

#[test]
fn the_boundary_is_asked_before_the_manifest_is_read_for_a_row() {
    // `crates/tui` has no row and sits inside the boundary `crates` draws, so
    // from outside that boundary both writes answer with the scope refusal —
    // never with what the manifest holds about `crates/tui`.
    let repo = a_repository();
    let home = a_dir();
    let before = manifest_bytes(repo.path()).expect("a manifest on disk");

    for refused in [
        scope_add(
            repo.path(),
            home.path(),
            "crates/tui",
            "billing",
            a_record(),
        ),
        scope_remove(repo.path(), home.path(), "crates/tui"),
    ] {
        let error = refused.expect_err("holding nothing opens nothing that is scoped");
        assert!(
            matches!(error, Error::ClosedScope { .. }),
            "the manifest's shape leaked past a closed boundary: {error:?}"
        );
        assert_eq!(status_for(&Err(error)), 3);
    }
    assert_eq!(manifest_bytes(repo.path()).as_deref(), Some(&before[..]));

    // And past the same boundary held, the clear says there was nothing to
    // take away and the add gives the directory a row of its own.
    holding(home.path(), repo.path(), &["platform"]);
    assert_eq!(
        scope_remove(repo.path(), home.path(), "crates/tui")
            .expect("a directory with no row carries no scope"),
        "crates/tui carried no scope"
    );
    assert_eq!(manifest_bytes(repo.path()).as_deref(), Some(&before[..]));
    assert_eq!(
        scope_add(
            repo.path(),
            home.path(),
            "crates/tui",
            "billing",
            a_record()
        )
        .expect("the machine holds the scope covering `crates/tui`"),
        "crates/tui is scoped `billing`"
    );
    assert_eq!(
        stored_scope(repo.path(), "crates/tui").as_deref(),
        Some("billing")
    );
}

#[test]
fn a_path_with_no_manifest_form_is_refused_by_both_scope_writes() {
    let repo = a_repository();
    let home = a_dir();
    let before = manifest_bytes(repo.path()).expect("a manifest on disk");

    for outside in [PathBuf::from("/elsewhere"), repo.path().join("..")] {
        let manifest = load_manifest(repo.path()).expect("a manifest that reads");
        let opened = || {
            Opened::new(
                repo.path().to_path_buf(),
                Some(home.path()),
                manifest.clone(),
                outside.clone(),
            )
        };

        for refused in [
            opened().and_then(|opened| opened.scoped("billing", a_record())),
            opened().and_then(|opened| opened.unscoped()),
        ] {
            let error = refused.expect_err("a path outside the repository has no manifest form");
            assert!(
                matches!(error, Error::Unspellable { .. }),
                "{}: {error:?}",
                outside.display()
            );
            assert!(!error.to_string().contains('\n'), "`main` prints one line");
        }
        assert_eq!(manifest_bytes(repo.path()).as_deref(), Some(&before[..]));
    }
}

#[test]
fn a_clear_that_took_a_boundary_away_names_it_and_one_that_took_nothing_says_so() {
    assert_eq!(
        unscoped_line("crates/engine", Some("data-plane")),
        "crates/engine is no longer scoped — was `data-plane`"
    );
    assert_eq!(unscoped_line("docs", None), "docs carried no scope");
}

// The one sigil this machine holds, on both doors and in every case below.
const HELD: &str = "platform";

// The scope it does not, which is the one every refusal here is by.
const CLOSED: &str = "data-plane";

// Every shape the two doors have to answer alike, in one manifest: an unscoped
// root, a boundary this machine holds on `crates`, one it does not on
// `crates/engine` below that, and an unscoped `docs` over a held `docs/api`.
// One manifest rather than one per case, because a parity test over several
// fixtures would be showing that the two doors agree about several different
// repositories.
fn a_repository_of_boundaries() -> (tempfile::TempDir, tempfile::TempDir) {
    let repo = a_dir();
    let home = a_dir();
    Manifest::with_directories([
        row("crates", HELD),
        row("crates/engine", CLOSED),
        row("docs/api", HELD),
    ])
    .save(repo.path())
    .expect("a manifest that saves");
    holding(home.path(), repo.path(), &[HELD]);

    (repo, home)
}

fn a_panel_over(repo_root: &Path) -> App {
    let node =
        |name: &str, children: Vec<Node>| Node::new(repo_root.join(name)).with_children(children);

    App::from_tree(&Tree::new(Node::new(repo_root).with_children([
        node(
            "crates",
            vec![node(
                "crates/engine",
                vec![node("crates/engine/src", Vec::new())],
            )],
        ),
        node("docs", vec![node("docs/api", Vec::new())]),
    ])))
}

// The answer, and deliberately not the mechanism: the panel refuses by
// opening no window and putting a line on the footer, the shell by handing
// `main` an error to print. Those are two shapes of one rule, and this is
// what the two of them have to be equal in.
#[derive(Debug, PartialEq, Eq)]
enum Answer {
    WentAhead,
    Refused(String),
}

// The one difference between the doors that is not about the rule:
// `App::label_for` spells the root row by its absolute path, where the shell
// spells that row `.`. Every other row is named by the manifest's own spelling
// on both sides, so this is a no-op for them.
fn as_the_shell_says_it(sentence: &str, repo_root: &Path) -> String {
    sentence.replace(&repo_root.display().to_string(), ".")
}

fn panel_answer(repo_root: &Path, home: &Path, path: &str) -> Answer {
    let manifest = load_manifest(repo_root).expect("a manifest that reads");
    let sigils = Settings::at(Some(home.to_path_buf()), repo_root).sigils_or_unknown();
    let mut app = a_panel_over(repo_root);
    let target = if path == "." {
        repo_root.to_path_buf()
    } else {
        repo_root.join(path)
    };
    let index = app
        .rows()
        .iter()
        .position(|row| row.path == target)
        .expect("the panel draws a row for this directory");
    app.select_row(index);

    match scope_press(
        &mut app,
        &manifest,
        repo_root,
        &sigils,
        None,
        Instant::now(),
    ) {
        ScopePrompt::Open(_) => Answer::WentAhead,
        ScopePrompt::Closed => Answer::Refused(as_the_shell_says_it(
            app.message().expect("a refused press says why"),
            repo_root,
        )),
    }
}

fn shell_answer(repo_root: &Path, home: &Path, path: &str) -> Answer {
    match open(repo_root, home, path) {
        Ok(_) => Answer::WentAhead,
        Err(error) => Answer::Refused(error.to_string()),
    }
}

#[test]
fn a_key_press_and_a_shell_prompt_answer_the_same_scope_write_alike() {
    // There is no path by which one door refuses and the other permits. Both
    // are pressed over one manifest, by one machine holding one sigil, and the
    // answers are held against each other *and* against what the answer is
    // supposed to be — so a change to either door alone fails here, and so
    // does a change to both that moves the rule.
    let refused_here = closed_scope_message("crates/engine", CLOSED);

    for (path, expected) in [
        ("crates/engine", Answer::Refused(refused_here)),
        // Covered from above by the scope this machine does not hold.
        (
            "crates/engine/src",
            Answer::Refused(closed_scope_message("crates/engine/src", CLOSED)),
        ),
        ("crates", Answer::WentAhead),
        ("docs/api", Answer::WentAhead),
        // Nothing covers these, so anyone may draw the first scope.
        ("docs", Answer::WentAhead),
        (".", Answer::WentAhead),
    ] {
        let (repo, home) = a_repository_of_boundaries();

        let panel = panel_answer(repo.path(), home.path(), path);
        let shell = shell_answer(repo.path(), home.path(), path);

        assert_eq!(
            panel, shell,
            "`s` and `warlock scope add` disagree over {path}"
        );
        assert_eq!(panel, expected, "the answer over {path} has changed");
    }
}
