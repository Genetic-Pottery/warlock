use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use tempfile::TempDir;
use warlock_engine::{
    Manifest, ScopeRecord, ScopedDirectory, from_manifest_path, load_key_binding, save_key,
    save_key_binding, save_open_scopes,
};

use super::{Pushes, Pushing};
use crate::account::Line;
use crate::app::App;
use crate::confirm::{Answer, PushAnswered};
use crate::error::{Error, one_line};
use crate::prompt::KeyPrompt;
use crate::push::prepare;
use crate::stubs::{Boarding, Op};

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

// The manifest-relative spelling the composer hands up for the brief typed
// after `/push <SCOPE>`.
const WRITTEN: &str = "docs/brief.md";

const TITLE: &str = "Push a brief to the board";

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
// machine's open scopes to say, which is the rule these tests are about.
fn a_manifest() -> Manifest {
    Manifest::with_directories([ScopedDirectory::new(".", "docs", SCOPE)
        .expect("a relative module path is inside the root")])
    .with_scopes([a_record(SCOPE, TEAM), a_record(OTHER_SCOPE, OTHER_TEAM)])
}

// A repository with the manifest saved and the brief on disk.
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

// A home of this test's own, every time: the open scopes, the bindings and the
// key store are all under it, and the module under test is
// only ever asked about the one handed in below. Nothing here reads `HOME`.
//
// Both of the manifest's scopes are bound, to the one name, because the config
// holds a key name per scope and a test that files into `OTHER_SCOPE` needs
// that scope's binding as much as `SCOPE`'s. Which of the two is a candidate
// stays the open scopes' question, which is the rule these tests are about.
fn a_home(root: &Path, open: &[&str]) -> TempDir {
    let home = a_dir();
    let open: Vec<String> = open.iter().map(|scope| (*scope).to_owned()).collect();
    save_open_scopes(home.path(), root, &open).expect("a config that writes");
    for scope in [SCOPE, OTHER_SCOPE] {
        save_key_binding(home.path(), root, scope, KEY_NAME).expect("a binding that writes");
    }
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

// A `Pushes` built with a home of the test's own: one built by `new` reads the
// machine's, and a test that went through there would resolve the board of the
// machine it runs on. The board panics if it is opened, because nothing before
// a Yes may open one.
fn pushes(home: &Path) -> Pushes<Boarding> {
    Pushes::with_client(Boarding::unopened(), Some(home.to_path_buf()))
}

// `/push <scope> docs/brief.md`, over a scope with a key bound: every test that
// goes through here is one of those, and the key window that comes back is the
// assertion that none of them was asked for a binding instead.
fn pressing(app: &mut App, repo: &Path, home: &Path, scope: &str) -> Pushing {
    let mut pushes = pushes(home);
    let prompt = pushes.press(app, &a_manifest(), repo, scope, WRITTEN, now());
    assert_eq!(
        prompt,
        KeyPrompt::Closed,
        "a bound scope was asked for a binding"
    );
    pushes.window
}

// The same command over a scope bound to nothing: both halves of what it comes
// to, which is the window the flow holds and the key window the session is to
// put up.
fn asking(app: &mut App, repo: &Path, home: &Path, scope: &str) -> (Pushes<Boarding>, KeyPrompt) {
    let mut pushes = pushes(home);
    let prompt = pushes.press(app, &a_manifest(), repo, scope, WRITTEN, now());
    (pushes, prompt)
}

// A home with the scope open and a key in the store, and no binding for any scope:
// the checkout somebody has just cloned, which is the road this question
// exists for.
fn an_unbound_home(root: &Path) -> TempDir {
    let home = a_dir();
    save_open_scopes(home.path(), root, &[SCOPE.to_owned()]).expect("a config that writes");
    save_key(home.path(), KEY_NAME, NOT_A_KEY).expect("a key store that writes");
    home
}

// The sentence `push.rs` words for this refusal, asked of it rather than spelled
// here: these tests assert that the line on the thread is that sentence, and
// `tests/push.rs` asserts what the sentence says.
fn refusal(repo: &Path, home: &Path, scope: &str) -> String {
    let error = prepare(
        &a_manifest(),
        repo,
        home,
        scope,
        &from_manifest_path(repo, WRITTEN),
    )
    .expect_err("a refusal");
    one_line(error)
}

#[test]
fn a_push_opens_the_dialog_over_the_project_the_team_and_the_key_name() {
    let repo = a_repository();
    let home = a_home(repo.path(), &[SCOPE]);
    let mut app = App::default();

    let pushing = pressing(&mut app, repo.path(), home.path(), SCOPE);

    let filing = pushing.confirm.filing().expect("the dialog is up");
    assert_eq!(
        filing.project(),
        TITLE,
        "the name is the brief's title line"
    );
    let destination = filing.destination();
    assert_eq!(
        destination.ticket_prefix(),
        TEAM,
        "the team is the `[[scope]]` record's"
    );
    assert_eq!(destination.key(), KEY_NAME, "the key is named, never shown");
    assert_eq!(destination.scope(), SCOPE);
    assert!(notes(&app).is_empty(), "a dialog said something as well");
}

#[test]
fn a_key_value_is_in_nothing_the_window_holds_or_renders() {
    // The window keeps the prepared push a Yes files, which carries the value;
    // its `Debug` rendering, which a failing assertion elsewhere in the suite
    // would print, must not show it.
    let repo = a_repository();
    let home = a_home(repo.path(), &[SCOPE]);
    let mut app = App::default();

    let pushing = pressing(&mut app, repo.path(), home.path(), SCOPE);

    assert!(pushing.confirm.is_open(), "{:?}", notes(&app));
    assert!(
        !format!("{pushing:?}").contains(NOT_A_KEY),
        "the key value is in the window"
    );
}

#[test]
fn a_machine_that_could_file_to_either_board_files_to_the_scope_typed() {
    let repo = a_repository();
    let home = a_home(repo.path(), &["*"]);
    let mut app = App::default();

    let pushing = pressing(&mut app, repo.path(), home.path(), OTHER_SCOPE);

    let filing = pushing.confirm.filing().expect("the dialog is up");
    assert_eq!(filing.destination().scope(), OTHER_SCOPE);
    assert_eq!(filing.destination().ticket_prefix(), OTHER_TEAM);
    assert!(notes(&app).is_empty());
}

#[test]
fn every_refusal_is_push_rs_own_line_on_the_thread_with_no_window_up() {
    // What the panel adds to `prepare`'s refusals: one line on the thread, in
    // `push.rs`'s words, and no window. One of each kind — a machine with no
    // scope open, a bound name the key store no longer holds, a scope that is no
    // candidate, and a brief that is gone. Every home here binds the scope the
    // command names, because an unbound one is the question asked ahead of
    // `prepare` rather than one of its refusals.
    let repo = a_repository();

    let nothing_open = a_dir();
    save_key_binding(nothing_open.path(), repo.path(), SCOPE, KEY_NAME)
        .expect("a binding that writes");
    save_key(nothing_open.path(), KEY_NAME, NOT_A_KEY).expect("a key store that writes");

    let dangling = a_dir();
    save_open_scopes(dangling.path(), repo.path(), &[SCOPE.to_owned()])
        .expect("a config that writes");
    save_key_binding(dangling.path(), repo.path(), SCOPE, KEY_NAME).expect("a binding that writes");

    let opened = a_home(repo.path(), &[SCOPE]);
    // `billing` is no candidate here, and that is the refusal this case is
    // about: the binding is written so the push reaches `prepare` to be told
    // so, rather than being asked which key a scope nothing records files
    // with.
    save_key_binding(opened.path(), repo.path(), "billing", KEY_NAME)
        .expect("a binding that writes");

    let missing = a_repository();
    let missing_home = a_home(missing.path(), &[SCOPE]);
    fs::remove_file(missing.path().join(WRITTEN)).expect("the brief this test wrote");

    for (repo, home, scope) in [
        (&repo, &nothing_open, SCOPE),
        (&repo, &dangling, SCOPE),
        (&repo, &opened, "billing"),
        (&missing, &missing_home, SCOPE),
    ] {
        let mut app = App::default();

        let pushing = pressing(&mut app, repo.path(), home.path(), scope);

        assert_eq!(
            notes(&app),
            vec![refusal(repo.path(), home.path(), scope)],
            "the line on the thread is not push.rs's own sentence",
        );
        assert_eq!(pushing, Pushing::closed(), "a refusal put a window up");
    }
}

// The manifest-relative spelling is resolved against the repository root and
// nowhere else, so a `/push` run from a session started in a subdirectory reads
// the same file.
#[test]
fn the_brief_is_read_from_the_repository_root_rather_than_the_working_directory() {
    let repo = a_repository();
    let home = a_home(repo.path(), &[SCOPE]);
    let mut app = App::default();
    let elsewhere: PathBuf = repo.path().join("docs");

    let pushing = pressing(&mut app, repo.path(), home.path(), SCOPE);
    let still_there = elsewhere.join("brief.md");

    assert!(still_there.exists(), "this test moved the brief");
    assert!(pushing.confirm.is_open(), "{:?}", notes(&app));
}

#[test]
fn a_yes_files_what_the_dialog_was_drawn_from_without_resolving_it_again() {
    // Every scope is closed while the dialog is up. The push the reader
    // said yes to was resolved when the dialog opened, and that is what is
    // filed: the board the dialog named, with the key it was resolved with.
    let repo = a_repository();
    let home = a_home(repo.path(), &[SCOPE]);
    let mut app = App::default();
    let linear = Boarding::filing(URL);
    let mut pushes = Pushes::with_client(linear.clone(), Some(home.path().to_path_buf())).inline();
    pushes.press(&mut app, &a_manifest(), repo.path(), SCOPE, WRITTEN, now());
    assert!(pushes.window.confirm.is_open(), "{:?}", notes(&app));
    save_open_scopes(home.path(), repo.path(), &[]).expect("a config that writes");

    pushes.answer(&mut app, PushAnswered::Send, now());
    pushes.keep_up(&mut app, now());

    assert!(!pushes.sending(), "the push never reported");
    assert_eq!(pushes.window, Pushing::closed(), "the dialog is still up");
    assert_eq!(linear.opened_with(), [NOT_A_KEY]);
    // Both lines, in the order the thread said them: the board named while the
    // worker runs is the board the address comes back under, and the address is
    // the one thing a push must not lose.
    assert_eq!(
        notes(&app),
        vec![
            format!("filing to {TEAM}"),
            format!("filed to {TEAM}: {URL}")
        ]
    );
}

#[test]
fn a_yes_to_a_brief_the_board_already_holds_says_so_on_the_thread_and_creates_nothing() {
    const EARLIER: &str = "https://linear.app/acme/project/push-a-brief-9f8e7d";
    let repo = a_repository();
    let home = a_home(repo.path(), &[SCOPE]);
    let mut app = App::default();
    let linear = Boarding::filing(URL).already_holding(EARLIER);
    let mut pushes = Pushes::with_client(linear.clone(), Some(home.path().to_path_buf())).inline();
    pushes.press(&mut app, &a_manifest(), repo.path(), SCOPE, WRITTEN, now());

    pushes.answer(&mut app, PushAnswered::Send, now());
    pushes.keep_up(&mut app, now());

    assert!(
        !linear.ops().contains(&Op::CreateBrief),
        "{:?}",
        linear.ops()
    );
    assert!(
        notes(&app).iter().any(|note| note.contains(EARLIER)),
        "{:?}",
        notes(&app)
    );
}

// The question asked ahead of `prepare`, over a temporary repository and a
// temporary home every time: the binding this checkout holds for the scope
// typed is read out of the config under that home and nowhere else.
#[test]
fn a_push_of_a_scope_bound_to_nothing_asks_for_the_key_and_opens_no_dialog() {
    let repo = a_repository();
    let home = an_unbound_home(repo.path());
    let mut app = App::default();

    let (pushes, prompt) = asking(&mut app, repo.path(), home.path(), SCOPE);

    let choice = prompt.choice().expect("the key window is up");
    assert_eq!(
        choice.scope(),
        SCOPE,
        "the window names a scope the command never did"
    );
    assert_eq!(
        choice.names(),
        [KEY_NAME],
        "the rows are not this home's key store"
    );
    assert_eq!(
        pushes.window,
        Pushing::closed(),
        "the question put a push dialog up as well"
    );
    assert!(notes(&app).is_empty(), "{:?}", notes(&app));
    // The board panics if it is opened, so nothing being sent is the stand-in's
    // own assertion; this is the other half of it — nothing was prepared, so
    // there is no key in anything the flow holds.
    assert!(
        !format!("{pushes:?}").contains(NOT_A_KEY),
        "the parked push carries a key value"
    );
}

#[test]
fn the_binding_written_leads_to_the_dialog_over_the_same_scope_and_brief() {
    // What the key window's Enter does, in the engine's own call, and then the
    // push picked up behind it: the dialog over the scope and the brief the
    // command named, with No lit, and no second question.
    let repo = a_repository();
    let home = an_unbound_home(repo.path());
    let mut app = App::default();
    let (mut pushes, prompt) = asking(&mut app, repo.path(), home.path(), SCOPE);
    assert!(prompt.is_open(), "{:?}", notes(&app));
    save_key_binding(home.path(), repo.path(), SCOPE, KEY_NAME).expect("a binding that writes");

    pushes.bound(&mut app, &a_manifest(), repo.path(), now());

    let filing = pushes
        .window
        .confirm
        .filing()
        .expect("the dialog the binding led to");
    assert_eq!(filing.project(), TITLE, "the brief is not the one typed");
    assert_eq!(filing.destination().scope(), SCOPE);
    assert_eq!(filing.destination().ticket_prefix(), TEAM);
    assert_eq!(filing.destination().key(), KEY_NAME);
    assert_eq!(filing.answer(), Answer::No, "the dialog opened on Yes");
    assert!(notes(&app).is_empty(), "{:?}", notes(&app));

    // And the push is answered once: the question is not waiting to be asked
    // again, so a second Enter at a window somebody else's road put up picks
    // nothing up.
    let dialog = pushes.window.clone();
    pushes.bound(&mut app, &a_manifest(), repo.path(), now());

    assert_eq!(pushes.window, dialog, "the push was prepared twice");
    assert!(notes(&app).is_empty(), "{:?}", notes(&app));
}

#[test]
fn an_abandoned_question_puts_the_whole_push_down_and_writes_no_binding() {
    // Esc on the key window: the window is the one that writes, and it never
    // opens the config, so what is asserted here is the other half — nothing is
    // prepared, no dialog opens and the scope is still bound to nothing.
    let repo = a_repository();
    let home = an_unbound_home(repo.path());
    let mut app = App::default();
    let (mut pushes, prompt) = asking(&mut app, repo.path(), home.path(), SCOPE);
    assert!(prompt.is_open(), "{:?}", notes(&app));

    pushes.put_down();
    // The press that answers the window is answered by the flow either way, so
    // an Esc followed by an Enter at a later window cannot pick this push up.
    pushes.bound(&mut app, &a_manifest(), repo.path(), now());

    assert_eq!(pushes.window, Pushing::closed(), "a dialog came up");
    assert!(!pushes.sending(), "something was sent");
    assert_eq!(
        load_key_binding(home.path(), repo.path(), SCOPE).expect("the config still reads"),
        None,
        "the scope was bound by an abandoned question"
    );
    assert!(notes(&app).is_empty(), "{:?}", notes(&app));
}

#[test]
fn a_checkout_with_no_config_at_all_is_one_line_on_the_thread_and_no_question() {
    // The other answer `load_key_binding` has, and the reason the question is
    // only asked on `Ok(None)`: there is no config to write a binding into, so
    // the engine's own sentence about that goes on the thread and nothing comes
    // up.
    let repo = a_repository();
    let home = a_dir();
    let mut app = App::default();

    let (pushes, prompt) = asking(&mut app, repo.path(), home.path(), SCOPE);

    let source = load_key_binding(home.path(), repo.path(), SCOPE)
        .expect_err("a checkout nobody configured");
    assert_eq!(notes(&app), vec![one_line(Error::Config { source })]);
    assert_eq!(prompt, KeyPrompt::Closed, "a question was asked anyway");
    assert_eq!(pushes.window, Pushing::closed(), "a dialog came up");
}
