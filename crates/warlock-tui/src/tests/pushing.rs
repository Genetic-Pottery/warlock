use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use tempfile::TempDir;
use warlock_engine::{
    Filed, FiledRecord, Manifest, PactEntry, ScopeRecord, from_manifest_path, save_key,
    save_key_binding, save_sigils,
};
use warlock_tui::{App, Edited, Line, PushAnswered, ScopeField, ScopePrompt, edit_for};

use super::{FILING_HEADING, NO_SCOPE, Pushes, Pushing};
use crate::error::one_line;
use crate::push::prepare;
use crate::stubs::Boarding;

// Not a key, and named so that nothing reading this file mistakes it for one:
// `tests/push.rs` keeps the same rule.
const NOT_A_KEY: &str = "not-a-real-key-value";

// A name no real key store would be holding, so a dialog that named the bound
// key of the machine these tests run on could not pass for one that named the
// temporary home's.
const KEY_NAME: &str = "this-tests-own-name";

const SCOPE: &str = "data-plane";

const OTHER_SCOPE: &str = "web";

const TEAM: &str = "Data Plane";

const OTHER_TEAM: &str = "Web";

const LABEL: &str = "warlock";

// What `/write` handed back: the manifest-relative spelling the thread line was
// worded from, which is the one thing the session remembers about the document.
const WRITTEN: &str = "docs/brief.md";

const TITLE: &str = "Push a brief to the board";

const PROJECT_ID: &str = "b229262b-22aa-444a-a8af-0a2a3f4ef100";

const URL: &str = "https://linear.app/acme/project/push-a-brief-1a2b3c";

// Every section the built-in shape asks for, so a repository that has written
// no template of its own holds this document to something it satisfies.
const BRIEF: &str = "# Push a brief to the board\n\n\
                     Nothing turns a document on disk into a project.\n\n\
                     ## Outcome\n\n`/push` files it.\n\n\
                     ## Success criteria\n\n**The reader**\n\n- sees a URL\n\n\
                     ## Constraints\n\nNo new dependency.\n\n\
                     ## Out of scope\n\nPulling anything back.\n\n\
                     ## Scope\n\n### 1. Read the file\n\ndepends_on: []\n";

fn a_dir() -> TempDir {
    tempfile::tempdir().expect("a temporary directory")
}

fn now() -> Instant {
    Instant::now()
}

fn a_record(name: &str, team: &str) -> ScopeRecord {
    ScopeRecord::new(name, team, "In Review", LABEL)
}

// Both records, in every repository below: which of them is a candidate is the
// machine's sigils' to say, which is the rule these tests are about.
fn a_manifest() -> Manifest {
    Manifest::with_entries([PactEntry::new(".", "docs", "docs/WARLOCK.md")
        .expect("a relative module path is inside the root")
        .with_scope(SCOPE)])
    .with_scopes([a_record(SCOPE, TEAM), a_record(OTHER_SCOPE, OTHER_TEAM)])
}

// A repository with the manifest saved and the brief `/write` wrote on disk.
fn a_repository() -> TempDir {
    let repo = a_dir();
    a_manifest()
        .save(repo.path())
        .expect("a manifest that saves");
    let path = repo.path().join(WRITTEN);
    fs::create_dir_all(path.parent().expect("a `docs` directory")).expect("a `docs` directory");
    fs::write(&path, BRIEF).expect("a brief file");
    repo
}

// A home of this test's own, every time: the sigils that pick the board, the
// binding and the key store are all under it, and the module under test is
// only ever asked about the one handed in below. Nothing here reads `HOME`.
fn a_home(root: &Path, sigils: &[&str]) -> TempDir {
    let home = a_dir();
    let held: Vec<String> = sigils.iter().map(|sigil| (*sigil).to_owned()).collect();
    save_sigils(home.path(), root, &held).expect("a config that writes");
    save_key_binding(home.path(), root, KEY_NAME).expect("a binding that writes");
    save_key(home.path(), KEY_NAME, NOT_A_KEY).expect("a key store that writes");
    home
}

// The record a first `/push` of this brief would have left behind.
fn already_filed(root: &Path) {
    Filed::with_records([FiledRecord::new(
        root,
        root.join(WRITTEN),
        PROJECT_ID,
        URL,
        SCOPE,
        TEAM,
        "2026-09-20T07:32:00Z",
    )
    .expect("a path inside the repository")])
    .save(root)
    .expect("a record file that saves");
}

fn notes(app: &App) -> Vec<String> {
    app.panel()
        .thread()
        .map(|thread| thread.lines(now()))
        .unwrap_or_default()
        .into_iter()
        .filter_map(|line| match line {
            Line::Note { text } => Some(text),
            _ => None,
        })
        .collect()
}

// A `Pushes` built with a home of the test's own: one built by `new` reads the
// machine's, and a test that went through there would resolve the board of the
// machine it runs on. The board panics if it is opened, because nothing before
// a Yes may open one.
fn pushes(home: &Path) -> Pushes<Boarding> {
    Pushes::with_client(Boarding::unopened(), Some(home.to_path_buf()))
}

// `/push` with nothing typed after it, over the brief `/write` wrote.
fn pressing(app: &mut App, repo: &Path, home: &Path) -> Pushing {
    let mut pushes = pushes(home);
    pushes.press(app, &a_manifest(), repo, WRITTEN, now());
    pushes.window
}

// Enter on the scope field, with `field` as what was typed into it.
fn submitting(app: &mut App, repo: &Path, home: &Path, field: ScopeField) -> Pushing {
    let mut pushes = pushes(home);
    pushes.window = asking(ScopePrompt::Open(field));
    pushes.edit(app, &a_manifest(), repo, Edited::Submit, now());
    pushes.window
}

// The value the loop keeps, with no home and a Linear nothing here sends
// anything to: the two paths through `edit` below resolve no board and open no
// client, and a `Pushes` with no home is what proves the first of those.
fn typing(window: Pushing) -> Pushes<Boarding> {
    let mut pushes = Pushes::with_client(Boarding::unopened(), None);
    pushes.window = window;
    pushes
}

// The window a field typed into is, for `edit`.
fn asking(prompt: ScopePrompt) -> Pushing {
    Pushing {
        field: prompt,
        brief: Some(WRITTEN.to_owned()),
        ..Pushing::closed()
    }
}

fn field(text: &str) -> ScopeField {
    ScopeField::new(FILING_HEADING, text)
}

fn press(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn opened(edited: Edited) -> ScopeField {
    match edited {
        Edited::Open(field) => field,
        other => panic!("the field is not up: {other:?}"),
    }
}

// The sentence `push.rs` words for this refusal, asked of it rather than spelled
// here: these tests assert that the line on the thread is that sentence, and
// `tests/push.rs` asserts what the sentence says.
fn refusal(repo: &Path, home: &Path, scope: Option<&str>) -> String {
    let error = prepare(
        &a_manifest(),
        repo,
        home,
        &from_manifest_path(repo, WRITTEN),
        scope,
    )
    .expect_err("a refusal");
    one_line(&error.to_string())
}

#[test]
fn one_candidate_opens_the_dialog_over_the_project_the_team_and_the_key_name() {
    // One sigil, one record of that name: nothing to ask, so the question is
    // the dialog itself and no field comes up in front of it.
    let repo = a_repository();
    let home = a_home(repo.path(), &[SCOPE]);
    let mut app = App::default();

    let pushing = pressing(&mut app, repo.path(), home.path());

    let filing = pushing.confirm.filing().expect("the dialog is up");
    assert_eq!(
        filing.project(),
        TITLE,
        "the name is the brief's title line"
    );
    let destination = filing.destination();
    assert_eq!(
        destination.team(),
        TEAM,
        "the team is the `[[scope]]` record's"
    );
    assert_eq!(destination.key(), KEY_NAME, "the key is named, never shown");
    assert_eq!(
        destination.scope(),
        SCOPE,
        "the name the board was resolved from"
    );
    assert_eq!(
        pushing.field,
        ScopePrompt::Closed,
        "a field came up as well"
    );
    assert!(notes(&app).is_empty(), "a dialog said something as well");
}

#[test]
fn a_key_value_is_in_nothing_the_window_holds_or_renders() {
    // The one thing on this path that must never be printed. The window keeps
    // the prepared push a Yes files, which carries the value; its `Debug`
    // rendering, which a failing assertion elsewhere in the suite would print,
    // must not show it.
    let repo = a_repository();
    let home = a_home(repo.path(), &[SCOPE]);
    let mut app = App::default();

    let pushing = pressing(&mut app, repo.path(), home.path());

    assert!(pushing.confirm.is_open(), "{:?}", notes(&app));
    assert!(
        !format!("{pushing:?}").contains(NOT_A_KEY),
        "the key value is in the window"
    );
}

#[test]
fn several_candidates_open_the_field_with_the_candidates_under_it_and_no_dialog() {
    // The wildcard holds every recorded scope, so this repository has two
    // boards and nothing to break the tie with. The engine's own sentence
    // names `--scope`, which is a flag on the subcommand and nothing a panel
    // has, so the field that is about to open is the instruction instead.
    let repo = a_repository();
    let home = a_home(repo.path(), &["*"]);
    let mut app = App::default();

    let pushing = pressing(&mut app, repo.path(), home.path());

    let field = pushing.field.field().expect("the field is up");
    assert_eq!(field.text(), "", "the field opened over somebody's typing");
    let rule = field.rule().expect("the candidates are under the field");
    for candidate in [SCOPE, OTHER_SCOPE] {
        assert!(rule.contains(candidate), "{rule} does not name {candidate}");
    }
    assert!(!rule.contains("--scope"), "{rule} names a flag warlock has");
    assert!(!pushing.confirm.is_open(), "the dialog came up unanswered");
    assert!(notes(&app).is_empty(), "the field said something as well");
}

#[test]
fn a_name_that_is_not_a_candidate_reopens_the_field_over_the_typing() {
    // One character from being right, and the reader is left looking at what
    // they typed with the candidates underneath it. The cursor is moved first,
    // because a refusal that put it back to the end would be a refusal that
    // retypes the field.
    let repo = a_repository();
    let home = a_home(repo.path(), &["*"]);
    let mut app = App::default();
    let typed = opened(edit_for(press(KeyCode::Left), &field("billing")));

    let pushing = submitting(&mut app, repo.path(), home.path(), typed.clone());

    let field = pushing.field.field().expect("the field is up again");
    assert_eq!(field.text(), typed.text(), "the typing was thrown away");
    assert_eq!(field.cursor(), typed.cursor(), "the cursor moved");
    assert_eq!(
        field.rule(),
        Some(refusal(repo.path(), home.path(), Some("billing")).as_str()),
        "the rule line is not the engine's own sentence",
    );
    assert!(!pushing.confirm.is_open(), "a refused name opened a dialog");
    assert!(notes(&app).is_empty(), "a refused name said it twice");
}

#[test]
fn a_candidate_submitted_takes_the_field_down_and_puts_the_dialog_up() {
    let repo = a_repository();
    let home = a_home(repo.path(), &["*"]);
    let mut app = App::default();

    let pushing = submitting(&mut app, repo.path(), home.path(), field(OTHER_SCOPE));

    let filing = pushing.confirm.filing().expect("the dialog is up");
    assert_eq!(
        filing.destination().scope(),
        OTHER_SCOPE,
        "the answer picked no board"
    );
    assert_eq!(
        filing.destination().team(),
        OTHER_TEAM,
        "the team is the other record's"
    );
    assert_eq!(filing.project(), TITLE);
    assert_eq!(pushing.field, ScopePrompt::Closed, "the field is still up");
    assert!(notes(&app).is_empty());
}

#[test]
fn a_submitted_name_is_read_with_the_spaces_around_it_taken_off() {
    // What the field hands over is what somebody typed into it, and a name
    // with a space in front of it is the name.
    let repo = a_repository();
    let home = a_home(repo.path(), &["*"]);
    let mut app = App::default();

    let pushing = submitting(&mut app, repo.path(), home.path(), field("  web  "));

    let filing = pushing.confirm.filing().expect("the dialog is up");
    assert_eq!(filing.destination().scope(), OTHER_SCOPE);
}

#[test]
fn every_refusal_is_push_rs_own_line_on_the_thread_with_no_window_up() {
    // What the panel adds to `prepare`'s refusals: one line on the thread, in
    // `push.rs`'s words, and no window. One of each kind — a machine holding
    // no sigil, a checkout with no key bound, a bound name the key store no
    // longer holds, a brief that is gone, and a brief already filed.
    let repo = a_repository();

    let unsigiled = a_dir();
    save_key_binding(unsigiled.path(), repo.path(), KEY_NAME).expect("a binding that writes");
    save_key(unsigiled.path(), KEY_NAME, NOT_A_KEY).expect("a key store that writes");

    let unbound = a_dir();
    save_sigils(unbound.path(), repo.path(), &[SCOPE.to_owned()]).expect("a config that writes");

    let dangling = a_dir();
    save_sigils(dangling.path(), repo.path(), &[SCOPE.to_owned()]).expect("a config that writes");
    save_key_binding(dangling.path(), repo.path(), KEY_NAME).expect("a binding that writes");

    let missing = a_repository();
    let missing_home = a_home(missing.path(), &[SCOPE]);
    fs::remove_file(missing.path().join(WRITTEN)).expect("the brief this test wrote");

    let filed = a_repository();
    let filed_home = a_home(filed.path(), &[SCOPE]);
    already_filed(filed.path());

    for (repo, home) in [
        (&repo, &unsigiled),
        (&repo, &unbound),
        (&repo, &dangling),
        (&missing, &missing_home),
        (&filed, &filed_home),
    ] {
        let mut app = App::default();

        let pushing = pressing(&mut app, repo.path(), home.path());

        assert_eq!(
            notes(&app),
            vec![refusal(repo.path(), home.path(), None)],
            "the line on the thread is not push.rs's own sentence",
        );
        assert_eq!(pushing, Pushing::closed(), "a refusal put a window up");
    }
}

#[test]
fn an_empty_field_is_refused_before_a_home_is_so_much_as_looked_for() {
    // The one refusal this module words itself, and the reason it does: the
    // engine would answer about a scope named nothing, and what is true is
    // that the reader has not typed yet. It is also the one path through
    // `Pushes::edit`'s submit that reads no home at all, which is what lets
    // this test drive that method over a value that has none.
    let repo = a_repository();
    let mut app = App::default();
    let mut pushes = typing(asking(ScopePrompt::Open(field("   "))));

    pushes.edit(&mut app, &a_manifest(), repo.path(), Edited::Submit, now());
    let pushing = pushes.window;

    let field = pushing.field.field().expect("the field is up");
    assert_eq!(field.rule(), Some(NO_SCOPE));
    assert!(!pushing.confirm.is_open());
    assert!(notes(&app).is_empty());
}

#[test]
fn typing_and_abandoning_move_nothing_but_the_field() {
    // Nothing has been resolved and nothing has been sent, so an Esc has
    // nothing to put back.
    let repo = a_repository();
    let app = App::default();
    let edit = |edited| {
        let mut pushes = typing(asking(ScopePrompt::Open(field("da"))));
        pushes.edit(
            &mut App::default(),
            &a_manifest(),
            repo.path(),
            edited,
            now(),
        );
        pushes.window
    };

    let typed = edit(edit_for(press(KeyCode::Char('t')), &field("da")));
    let closed = edit(Edited::Close);

    assert_eq!(typed.field, ScopePrompt::Open(field("dat")));
    assert!(!typed.confirm.is_open());
    assert_eq!(closed, Pushing::closed());
    assert!(notes(&app).is_empty());
}

// The manifest-relative spelling `/write` hands back is resolved against the
// repository root and nowhere else, so a `/push` run from a session started in
// a subdirectory reads the same file.
#[test]
fn the_brief_is_read_from_the_repository_root_rather_than_the_working_directory() {
    let repo = a_repository();
    let home = a_home(repo.path(), &[SCOPE]);
    let mut app = App::default();
    let elsewhere: PathBuf = repo.path().join("docs");

    let pushing = pressing(&mut app, repo.path(), home.path());
    let still_there = elsewhere.join("brief.md");

    assert!(still_there.exists(), "this test moved the brief");
    assert!(pushing.confirm.is_open(), "{:?}", notes(&app));
}

#[test]
fn a_yes_files_what_the_dialog_was_drawn_from_without_resolving_it_again() {
    // The sigils are taken away while the dialog is up. The push the reader
    // said yes to was resolved when the dialog opened, and that is what is
    // filed: the board the dialog named, with the key it was resolved with.
    let repo = a_repository();
    let home = a_home(repo.path(), &[SCOPE]);
    let mut app = App::default();
    let linear = Boarding::filing(URL);
    let mut pushes = Pushes::with_client(linear.clone(), Some(home.path().to_path_buf()));
    pushes.press(&mut app, &a_manifest(), repo.path(), WRITTEN, now());
    assert!(pushes.window.confirm.is_open(), "{:?}", notes(&app));
    save_sigils(home.path(), repo.path(), &[]).expect("a config that writes");

    pushes.answer(&mut app, PushAnswered::Send, now());
    let waited = Instant::now();
    while pushes.sending() && waited.elapsed() < Duration::from_secs(10) {
        pushes.keep_up(&mut app, now());
    }

    assert!(!pushes.sending(), "the push never reported");
    assert_eq!(pushes.window, Pushing::closed(), "the dialog is still up");
    assert_eq!(linear.opened_with(), [NOT_A_KEY]);
    assert!(
        notes(&app).iter().any(|note| note.contains(URL)),
        "{:?}",
        notes(&app)
    );
    let filed = Filed::load(repo.path()).expect("the record the push saved");
    assert_eq!(
        filed
            .record(WRITTEN)
            .expect("a record for the brief")
            .team(),
        TEAM
    );
}
