use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use tempfile::TempDir;
use warlock_engine::{
    Manifest, PactEntry, ScopeRecord, resolve_filing, save_key, save_key_binding, save_sigils,
};
use warlock_tui::{App, Edited, Line, ScopeField, ScopePrompt, edit_for};

use super::{FILING_HEADING, NO_SCOPE, Pushes, Pushing, filing_to};
use crate::error::one_line;
use crate::stubs::Boarding;

// Not a key, and named so that nothing reading this file mistakes it for one:
// `tests/push.rs` keeps the same rule. Nothing below reads it back — a target
// is built and dropped inside the module under test, and what comes out of it
// is a window holding names.
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

// The half of the module that takes the home as a parameter, which is the half
// every test drives: a `Pushes` built by `new` reads the machine's own home, and
// a test that went through there would resolve the board of the machine it runs
// on.
fn filing(app: &mut App, repo: &Path, home: &Path, asked: Option<&ScopeField>) -> Pushing {
    filing_to(app, &a_manifest(), repo, home, WRITTEN, asked, now())
}

// The value the loop keeps, with no home and a Linear nothing here sends
// anything to: the two paths through `edit` below resolve no board and open no
// client, and a `Pushes` with no home is what proves the first of those.
fn typing() -> Pushes<Boarding> {
    Pushes::with_client(Boarding::filing(""), None)
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

// The sentence the engine words for this refusal, asked of the engine rather
// than spelled here: these tests assert that the line on the thread is that
// sentence, not what the sentence says. The key refusals stay worded in
// `route.rs`, where `warlock check` meets them first.
fn refusal(repo: &Path, home: &Path, scope: Option<&str>) -> String {
    let error = resolve_filing(&a_manifest(), repo, home, scope).expect_err("a refusal");
    one_line(&error.to_string())
}

#[test]
fn one_candidate_opens_the_dialog_over_the_project_the_team_and_the_key_name() {
    // One sigil, one record of that name: nothing to ask, so the question is
    // the dialog itself and no field comes up in front of it.
    let repo = a_repository();
    let home = a_home(repo.path(), &[SCOPE]);
    let mut app = App::default();

    let pushing = filing(&mut app, repo.path(), home.path(), None);

    let filing = pushing.confirm.filing().expect("the dialog is up");
    assert_eq!(
        filing.project(),
        TITLE,
        "the name is the brief's title line"
    );
    assert_eq!(filing.team(), TEAM, "the team is the `[[scope]]` record's");
    assert_eq!(filing.key(), KEY_NAME, "the key is named, never shown");
    assert_eq!(
        filing.scope(),
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
    // The one thing on this path that must never be printed. The target the
    // engine handed back carries it; what the window keeps is four names, so
    // there is nowhere in it for the value to be — including in the `Debug`
    // rendering a failing assertion elsewhere in the suite would print.
    let repo = a_repository();
    let home = a_home(repo.path(), &[SCOPE]);
    let mut app = App::default();

    let pushing = filing(&mut app, repo.path(), home.path(), None);

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

    let pushing = filing(&mut app, repo.path(), home.path(), None);

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

    let pushing = filing(&mut app, repo.path(), home.path(), Some(&typed));

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

    let pushing = filing(
        &mut app,
        repo.path(),
        home.path(),
        Some(&field(OTHER_SCOPE)),
    );

    let filing = pushing.confirm.filing().expect("the dialog is up");
    assert_eq!(filing.scope(), OTHER_SCOPE, "the answer picked no board");
    assert_eq!(filing.team(), OTHER_TEAM, "the team is the other record's");
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

    let pushing = filing(&mut app, repo.path(), home.path(), Some(&field("  web  ")));

    let filing = pushing.confirm.filing().expect("the dialog is up");
    assert_eq!(filing.scope(), OTHER_SCOPE);
}

#[test]
fn every_board_refusal_is_the_engines_own_line_on_the_thread_with_no_window_up() {
    // The three the engine answers before anything could be sent: a machine
    // holding no sigil, a checkout with no key bound, and a bound name the key
    // store no longer holds. Each is asserted against the sentence the engine
    // words for it — the last two are `route.rs`'s, which is where `warlock
    // check` meets them first — rather than against a wording of this
    // module's, which is the whole point: there is none.
    let repo = a_repository();

    let unsigiled = a_dir();
    save_key_binding(unsigiled.path(), repo.path(), KEY_NAME).expect("a binding that writes");
    save_key(unsigiled.path(), KEY_NAME, NOT_A_KEY).expect("a key store that writes");

    let unbound = a_dir();
    save_sigils(unbound.path(), repo.path(), &[SCOPE.to_owned()]).expect("a config that writes");

    let dangling = a_dir();
    save_sigils(dangling.path(), repo.path(), &[SCOPE.to_owned()]).expect("a config that writes");
    save_key_binding(dangling.path(), repo.path(), KEY_NAME).expect("a binding that writes");

    for home in [&unsigiled, &unbound, &dangling] {
        let mut app = App::default();

        let pushing = filing(&mut app, repo.path(), home.path(), None);

        assert_eq!(
            notes(&app),
            vec![refusal(repo.path(), home.path(), None)],
            "the line on the thread is not the engine's own sentence",
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
    let prompt = ScopePrompt::Open(field("   "));

    // No home at all, which is the point: this refusal is answered before one is
    // so much as looked at.
    let pushing = typing().edit(
        &mut app,
        &a_manifest(),
        repo.path(),
        Some(WRITTEN),
        &prompt,
        Edited::Submit,
        now(),
    );

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
    let prompt = ScopePrompt::Open(field("da"));
    let edit = |edited| {
        typing().edit(
            &mut App::default(),
            &a_manifest(),
            repo.path(),
            Some(WRITTEN),
            &prompt,
            edited,
            now(),
        )
    };

    let typed = edit(edit_for(press(KeyCode::Char('t')), &field("da")));
    let closed = edit(Edited::Close);

    assert_eq!(typed.field, ScopePrompt::Open(field("dat")));
    assert!(!typed.confirm.is_open());
    assert_eq!(closed, Pushing::closed());
    assert!(notes(&app).is_empty());
}

#[test]
fn a_brief_that_is_no_longer_a_brief_is_a_line_and_no_dialog() {
    // The document is read at the moment of the push rather than remembered
    // from the write, so a file edited with `e` in between — or deleted — is
    // answered for here. The board was resolved and nothing was sent.
    let repo = a_repository();
    let home = a_home(repo.path(), &[SCOPE]);
    let mut app = App::default();
    fs::remove_file(repo.path().join(WRITTEN)).expect("the brief this test wrote");

    let pushing = filing(&mut app, repo.path(), home.path(), None);

    assert_eq!(
        pushing,
        Pushing::closed(),
        "a missing brief put a window up"
    );
    assert_eq!(notes(&app).len(), 1, "{:?}", notes(&app));
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

    let pushing = filing_to(
        &mut app,
        &a_manifest(),
        repo.path(),
        home.path(),
        WRITTEN,
        None,
        now(),
    );
    let still_there = elsewhere.join("brief.md");

    assert!(still_there.exists(), "this test moved the brief");
    assert!(pushing.confirm.is_open(), "{:?}", notes(&app));
}
