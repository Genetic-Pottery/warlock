use std::path::Path;
use std::time::Instant;
use std::{fs, io};

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use tempfile::TempDir;
use warlock_engine::{
    LinearScope, Manifest, Node, ScopeRecord, ScopedDirectory, Tree, config, config_path,
    load_key_binding, manifest, route_facts, save_key, save_key_binding, validate_scope,
};

use super::{Windows, key_edit, key_submit, record_edit, record_submit, scope_edit, scope_submit};
use crate::app::{App, OpenScopes};
use crate::prompt::{
    Edited, KeyChoice, KeyEdited, KeyPrompt, RecordEdited, RecordField, RecordForm, RecordPrompt,
    RecordTracker, ScopeField, ScopePrompt, edit_for, key_edit_for, record_edit_for,
};

// `super::scope_press` with no boundary in the way, so these tests are about
// the prompt rather than about being refused. The wildcard rather than
// `OpenScopes::Nothing` because the fixture manifest *does* scope
// `crates/engine`, and AI can work in no scope opens nothing that is scoped. Tests
// that are about the boundary call `super::scope_press` directly.
fn scope_press(app: &mut App, manifest: &Manifest, repo_root: &Path) -> ScopePrompt {
    super::scope_press(
        app,
        manifest,
        repo_root,
        &OpenScopes::named(["*"]),
        None,
        Instant::now(),
    )
}

// What no press of `s` and no Enter in its window is allowed to spend.
const LAST_KEY: &str = "something the last key said";

// A Jira site as somebody would type it, trailing slash and all: nothing on
// this path parses, normalises or judges one, so it is written back the way it
// was typed.
const JIRA_SITE: &str = "https://acme.atlassian.net/";

// The team key is read through the view, which is the only way there is to
// read one off a record.
fn team_of(record: &ScopeRecord) -> &str {
    LinearScope::of(record)
        .expect("the record window writes a Linear scope")
        .team_key()
}

// The tree below is written out by hand rather than loaded, so nothing is
// inside this until a scope is saved.
fn a_repo() -> TempDir {
    tempfile::tempdir().expect("a temporary directory")
}

/// The tree the app is built over, rooted at `root`.
///
/// `crates/engine` carries a scope in the manifest below, and nothing covers
/// the other three.
///
/// ```text
/// <root>
/// └── crates
///     ├── crates/engine       data-plane
///     └── crates/tui
/// ```
fn tree(root: &Path) -> Tree {
    Tree::new(
        Node::new(root).with_children([Node::new(root.join("crates")).with_children([
            Node::new(root.join("crates/engine")).with_scope(Some("data-plane".to_owned())),
            Node::new(root.join("crates/tui")),
        ])]),
    )
}

/// Which row of [`tree`] each directory is, in walk order.
const ROOT_ROW: usize = 0;
const ENGINE_ROW: usize = 2;
const TUI_ROW: usize = 3;

/// The app the event loop would hold for `root`, with `row` selected and a
/// line on the footer from the keystroke before this one.
fn app_on(root: &Path, row: usize) -> App {
    let mut app = App::from_tree(&tree(root));
    app.select_row(row);
    // After the selection, because moving the selection is what takes a
    // message down: every assertion below about the message line being left
    // alone needs there to be one.
    app.set_message(LAST_KEY);
    app
}

fn row(module: &str, scope: &str) -> ScopedDirectory {
    ScopedDirectory::new(".", module, scope).expect("a relative module path is inside the root")
}

/// The manifest the loop holds: `crates/engine` scoped, and no row for
/// anything else.
///
/// Named for what it holds rather than for its type, so that a test can
/// have a `manifest` of its own to edit and still ask what the fixture said.
fn scoped() -> Manifest {
    Manifest::with_directories([row("crates/engine", "data-plane")])
}

/// The scope stored on `module`, or `None` for a directory with no row.
fn scope_on<'manifest>(manifest: &'manifest Manifest, module: &str) -> Option<&'manifest str> {
    manifest.directory(module).map(ScopedDirectory::scope)
}

/// The manifest as it sits on disk under `root`, or `None` when none was
/// ever written there.
fn saved(root: &Path) -> Option<Manifest> {
    match Manifest::load(root) {
        Ok(manifest) => Some(manifest),
        Err(manifest::Error::NotFound { .. }) => None,
        Err(error) => panic!("the saved manifest could not be read: {error}"),
    }
}

/// A field over `module` AI can work in `text`, as the window would be by the time
/// Enter is pressed.
fn field(module: &str, text: &str) -> ScopeField {
    ScopeField::new(module, text)
}

/// The engine's own sentence about why `text` is not a scope: the one that
/// belongs under the field, asked of the one judge rather than spelled out
/// here.
fn refusal(text: &str) -> String {
    validate_scope(text)
        .expect_err("this text is not a scope")
        .to_string()
}

/// A plain press of `code`, as crossterm reports one.
fn press(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

/// What the footer says when the boundary turns `s` down over
/// `crates/engine`, which the fixture scopes `data-plane`.
const CLOSED: &str = "crates/engine is scoped `data-plane`, which is closed on this machine — `warlock open data-plane` lets AI work here";

#[test]
fn s_is_refused_on_a_directory_whose_scope_is_closed_on_this_machine() {
    let repo = a_repo();
    let mut app = app_on(repo.path(), ENGINE_ROW);
    let before = app.clone();

    let prompt = super::scope_press(
        &mut app,
        &scoped(),
        repo.path(),
        &OpenScopes::named(["web"]),
        None,
        Instant::now(),
    );

    // A boundary must be open to redraw it. Without this the one key whose
    // whole purpose is to move a scope would be the one key a scope did not
    // cover.
    assert_eq!(prompt, ScopePrompt::Closed, "the window must not open");
    assert_eq!(
        app.message(),
        Some(CLOSED),
        "the refusal names the scope wanted and how to open it"
    );
    // Saying so is the whole of it: the same app with the new line on it and
    // nothing else moved — no colour and no selection.
    let said = {
        let mut said = before.clone();
        said.set_message(CLOSED);
        said
    };
    assert_eq!(app, said, "the refusal did more to the app than say so");
}

#[test]
fn a_matching_open_scope_opens_the_prompt() {
    let repo = a_repo();

    for open in [
        OpenScopes::named(["data-plane"]),
        OpenScopes::named(["web", "data-plane"]),
        OpenScopes::named(["*"]),
    ] {
        let mut app = app_on(repo.path(), ENGINE_ROW);

        let prompt = super::scope_press(
            &mut app,
            &scoped(),
            repo.path(),
            &open,
            None,
            Instant::now(),
        );

        assert_eq!(
            prompt,
            ScopePrompt::open("crates/engine", "data-plane"),
            "{open:?} opens `data-plane`"
        );
        assert_eq!(app.message(), Some(LAST_KEY), "{open:?} said something");
    }
}

#[test]
fn a_machine_with_no_scope_open_is_refused_by_a_scoped_directory() {
    let repo = a_repo();

    for open in [
        // Nobody has run `warlock open` here, so no scope is open — the empty
        // set is not a universal bypass, or the boundary would hold for exactly
        // those people who had already opted into being bound by it.
        OpenScopes::Nothing,
        // And a config that will not parse cannot establish that anything is
        // open, which is the same answer. The header says `where AI can work is
        // unknown` for as long as it lasts, so the two are told apart on screen.
        OpenScopes::Unknown,
    ] {
        let mut app = app_on(repo.path(), ENGINE_ROW);

        let prompt = super::scope_press(
            &mut app,
            &scoped(),
            repo.path(),
            &open,
            None,
            Instant::now(),
        );

        assert_eq!(prompt, ScopePrompt::Closed, "{open:?} opened `data-plane`");
        assert_eq!(app.message(), Some(CLOSED), "{open:?} said the wrong thing");
    }
}

#[test]
fn an_unscoped_directory_stays_open_to_a_machine_with_no_scope_open() {
    let repo = a_repo();
    let mut app = app_on(repo.path(), TUI_ROW);

    // Anyone may draw the first scope over a directory nothing covers: a scope
    // is the only thing that opens a directory to work, so if drawing one also
    // needed a scope, no repository could ever be set up.
    let prompt = super::scope_press(
        &mut app,
        &scoped(),
        repo.path(),
        &OpenScopes::Nothing,
        None,
        Instant::now(),
    );

    assert_eq!(prompt, ScopePrompt::open("crates/tui", ""));
}

#[test]
fn a_directory_no_scope_covers_is_open_to_a_machine_with_another_scope_open() {
    let repo = a_repo();
    let mut app = app_on(repo.path(), TUI_ROW);

    // `crates/tui` carries no scope and nothing above it does either, so
    // there is no boundary here to be outside of.
    let prompt = super::scope_press(
        &mut app,
        &scoped(),
        repo.path(),
        &OpenScopes::named(["data-plane"]),
        None,
        Instant::now(),
    );

    assert_eq!(prompt, ScopePrompt::open("crates/tui", ""));
}

#[test]
fn an_inner_scope_replaces_the_outer_one_rather_than_adding_to_it() {
    let repo = a_repo();
    let manifest =
        Manifest::with_directories([row("crates/engine", "data-plane"), row(".", "platform")]);

    // The outer scope open alone does not reach the inner one: an outer
    // scope is a default for what said nothing below it, never a second gate
    // that `platform` could satisfy on `data-plane`'s behalf.
    let mut app = app_on(repo.path(), ENGINE_ROW);
    assert_eq!(
        super::scope_press(
            &mut app,
            &manifest,
            repo.path(),
            &OpenScopes::named(["platform"]),
            None,
            Instant::now()
        ),
        ScopePrompt::Closed
    );

    // And the outer scope does cover the directory that said nothing.
    let mut app = app_on(repo.path(), TUI_ROW);
    assert_eq!(
        super::scope_press(
            &mut app,
            &manifest,
            repo.path(),
            &OpenScopes::named(["platform"]),
            None,
            Instant::now()
        ),
        ScopePrompt::open("crates/tui", "")
    );
}

#[test]
fn s_on_a_scoped_row_opens_the_prompt_on_the_scope_that_directory_carries() {
    let repo = a_repo();
    let mut app = app_on(repo.path(), ENGINE_ROW);
    let before = app.clone();

    let prompt = scope_press(&mut app, &scoped(), repo.path());

    // Over the directory as the manifest spells it, holding what is true
    // right now — so an Enter pressed straight afterwards writes back what
    // was already there rather than clearing it.
    assert_eq!(prompt, ScopePrompt::open("crates/engine", "data-plane"));
    assert_eq!(
        app, before,
        "opening the window changed something about the view"
    );
}

#[test]
fn esc_closes_the_prompt_and_leaves_the_app_exactly_as_it_was() {
    let repo = a_repo();
    let mut app = app_on(repo.path(), ENGINE_ROW);
    let before = app.clone();

    let prompt = scope_press(&mut app, &scoped(), repo.path());
    let field = prompt.field().expect("the prompt is up").clone();

    // The loop's own Esc arm, through the pure function that decides it:
    // the prompt comes down and nothing else is done, because there is
    // nothing to put back.
    assert_eq!(edit_for(press(KeyCode::Esc), &field), Edited::Close);
    assert_eq!(app, before, "Esc left something behind");
    assert_eq!(app.message(), Some(LAST_KEY), "and it kept the last line");
}

#[test]
fn a_directory_carrying_no_scope_opens_on_an_empty_field() {
    let repo = a_repo();
    let mut app = app_on(repo.path(), TUI_ROW);

    let prompt = scope_press(&mut app, &scoped(), repo.path());

    assert_eq!(prompt, ScopePrompt::open("crates/tui", ""));
}

#[test]
fn the_root_opens_on_an_empty_field_spelled_as_the_manifest_spells_it() {
    let repo = a_repo();
    let mut app = app_on(repo.path(), ROOT_ROW);

    let prompt = scope_press(&mut app, &scoped(), repo.path());

    assert_eq!(prompt, ScopePrompt::open(".", ""));
}

#[test]
fn the_field_holds_what_the_manifest_says_and_never_anything_off_the_row() {
    // The same app and the same row against two manifests: what is in the
    // field moves with the manifest, which it could not do if it were being
    // read off the tree.
    let repo = a_repo();
    let mut app = app_on(repo.path(), ENGINE_ROW);
    let elsewhere = Manifest::with_directories([row("crates/engine", "web")]);

    assert_eq!(
        scope_press(&mut app, &scoped(), repo.path()),
        ScopePrompt::open("crates/engine", "data-plane")
    );
    assert_eq!(
        scope_press(&mut app, &elsewhere, repo.path()),
        ScopePrompt::open("crates/engine", "web")
    );
}

#[test]
fn a_directory_with_no_manifest_spelling_opens_nothing_and_says_why() {
    // A tree rooted outside its own repository: nothing on disk, because
    // whether a path can be spelled relative to a root is arithmetic.
    let mut app = App::from_tree(&Tree::new(Node::new("/outside/thing")));

    let prompt = scope_press(&mut app, &Manifest::new(), Path::new("/repo"));

    assert_eq!(prompt, ScopePrompt::Closed);
    assert!(
        app.message().is_some_and(|line| !line.is_empty()),
        "a window with nowhere to write said nothing"
    );
}

#[test]
fn enter_sets_the_scope_and_leaves_every_other_row_alone() {
    let repo = a_repo();
    let mut app = app_on(repo.path(), ENGINE_ROW);
    let mut manifest = recorded();

    let windows = scope_submit(
        &mut app,
        &mut manifest,
        repo.path(),
        &field("crates/engine", "platform"),
    );

    assert_eq!(windows, Windows::default(), "the window is answered");
    let written = saved(repo.path()).expect("the submit wrote the manifest");
    assert_eq!(
        written, manifest,
        "what is on disk is what this thread believes"
    );
    assert_eq!(scope_on(&written, "crates/engine"), Some("platform"));
    assert_eq!(written.directory("docs"), recorded().directory("docs"));
    // And the order the file is written in, so the diff is the scope line.
    assert_eq!(
        written
            .directories()
            .iter()
            .map(ScopedDirectory::path)
            .collect::<Vec<_>>(),
        ["crates/engine", "docs"]
    );
}

// `third-party` is named by no row in the fixture and `data-plane` loses its
// only row to the clear below: a write that pruned the records to what the rows
// spell would drop both, and a record is the one thing here that is allowed to
// outlive the rows that named it.
fn records() -> Vec<ScopeRecord> {
    vec![
        ScopeRecord::new("data-plane", "Data Plane", "In Review", "area/data-plane"),
        ScopeRecord::new("third-party", "Vendor", "Triage", "area/vendor"),
    ]
}

/// [`scoped`] holding a record for every name the submits below write.
///
/// A scope name no record claims puts the record window up instead of writing,
/// so a test about the write itself has to name one that is already recorded —
/// which the fixture above does not, and deliberately: the tests that are about
/// the second window need a name nothing claims.
fn known() -> Manifest {
    scoped().with_scopes(records().into_iter().chain([
        ScopeRecord::new("billing", "Billing", "In Review", "area/billing"),
        ScopeRecord::new("web", "Web", "Triage", "area/web"),
    ]))
}

#[test]
fn a_set_and_a_clear_both_leave_the_records_in_the_file() {
    let repo = a_repo();
    let mut app = app_on(repo.path(), ENGINE_ROW);
    let mut manifest = known();

    for text in ["billing", ""] {
        let windows = scope_submit(
            &mut app,
            &mut manifest,
            repo.path(),
            &field("crates/engine", text),
        );

        assert_eq!(windows, Windows::default());
        let written = saved(repo.path()).expect("the submit wrote the manifest");
        assert_eq!(
            written.scopes(),
            known().scopes(),
            "writing `{text}` over the scope moved a record",
        );
        assert_eq!(
            written, manifest,
            "and what is on disk is what this thread believes"
        );
    }

    assert_eq!(
        scope_on(&manifest, "crates/engine"),
        None,
        "the clear really did take the last row naming `data-plane`",
    );
}

#[test]
fn enter_on_an_empty_field_clears_the_scope_and_drops_the_row() {
    let repo = a_repo();
    let mut app = app_on(repo.path(), ENGINE_ROW);
    let mut manifest = scoped();

    let windows = scope_submit(
        &mut app,
        &mut manifest,
        repo.path(),
        &field("crates/engine", ""),
    );

    // Neither window: clearing is an answer, and there is no name to record.
    assert_eq!(windows, Windows::default());
    let written = saved(repo.path()).expect("the submit wrote the manifest");
    assert_eq!(written, manifest);
    assert_eq!(
        scope_on(&written, "crates/engine"),
        None,
        "an empty field clears the scope rather than storing one"
    );
    assert!(written.directories().is_empty(), "{written:?}");
}

#[test]
fn a_set_and_a_clear_leave_the_view_and_the_message_line_untouched() {
    // A successful write says nothing, and does not even spend the last
    // keystroke's sentence. The app is compared whole.
    let repo = a_repo();
    let mut app = app_on(repo.path(), ENGINE_ROW);
    let mut manifest = known();
    let before = app.clone();

    for text in ["billing", ""] {
        let windows = scope_submit(
            &mut app,
            &mut manifest,
            repo.path(),
            &field("crates/engine", text),
        );

        assert_eq!(windows, Windows::default());
        assert_eq!(
            app.message(),
            Some(LAST_KEY),
            "writing {text:?} took the message line"
        );
        assert_eq!(app, before, "writing {text:?} moved the view");
    }
}

#[test]
fn input_the_engine_refuses_keeps_the_prompt_up_and_writes_nothing() {
    let repo = a_repo();
    let mut app = app_on(repo.path(), ENGINE_ROW);
    let mut manifest = scoped();
    let before = app.clone();
    let typed = field("crates/engine", "Control Plane");

    let windows = scope_submit(&mut app, &mut manifest, repo.path(), &typed);

    // The text and the cursor exactly where they were, one character away
    // from being fixed, with the broken rule under the field.
    assert_eq!(
        windows,
        Windows::asking(typed.refused(refusal("control plane"))),
    );
    assert_eq!(saved(repo.path()), None, "a refusal wrote to disk");
    assert_eq!(manifest, scoped(), "a refusal edited the manifest");
    assert_eq!(app, before, "a refusal moved the view");
}

#[test]
fn a_list_typed_into_the_field_is_refused_and_never_stored_as_two_scopes() {
    let repo = a_repo();
    let mut app = app_on(repo.path(), ENGINE_ROW);
    let mut manifest = scoped();
    let typed = field("crates/engine", "control-plane, data-plane");

    let windows = scope_submit(&mut app, &mut manifest, repo.path(), &typed);

    // The character rule, and it is the engine's own sentence: a directory
    // has one scope, so the comma is what is refused rather than a
    // separator warlock would have split on.
    assert_eq!(
        windows,
        Windows::asking(typed.refused(refusal("control-plane, data-plane"))),
    );
    assert_eq!(saved(repo.path()), None);
    assert_eq!(
        manifest.directories().len(),
        1,
        "the list became a second row"
    );
    assert_eq!(scope_on(&manifest, "crates/engine"), Some("data-plane"));
}

#[test]
fn what_was_typed_is_folded_before_it_is_judged_and_stored() {
    // `Data-Plane` and `data-plane` are one boundary, not two, and folding
    // belongs here rather than in the validator — which refuses a capital
    // outright, as the assertion below its sibling shows.
    let repo = a_repo();
    let mut app = app_on(repo.path(), TUI_ROW);
    let mut manifest = known();

    let windows = scope_submit(
        &mut app,
        &mut manifest,
        repo.path(),
        &field("crates/tui", "Data-Plane"),
    );

    // Closed and not the record window, which is the fold happening before
    // the lookup as well as before the judge: `data-plane` is recorded and
    // `Data-Plane` is not.
    assert_eq!(
        windows,
        Windows::default(),
        "the fold happened before the judge"
    );
    assert!(validate_scope("Data-Plane").is_err());
    assert_eq!(scope_on(&manifest, "crates/tui"), Some("data-plane"));
    assert_eq!(
        scope_on(&saved(repo.path()).expect("the submit wrote"), "crates/tui"),
        Some("data-plane"),
    );
}

#[test]
fn a_manifest_that_will_not_save_puts_its_reason_on_the_message_line() {
    // A file where `.warlock/` has to be: the save cannot make the
    // directory, which is the cheapest real version of a disk that will not
    // take the write.
    let repo = a_repo();
    fs::write(repo.path().join(".warlock"), "not a directory\n")
        .expect("writes a file in the way of the manifest directory");
    let mut app = app_on(repo.path(), ENGINE_ROW);
    let mut manifest = known();

    let windows = scope_submit(
        &mut app,
        &mut manifest,
        repo.path(),
        &field("crates/engine", "billing"),
    );

    // The reason goes on the footer and the window comes down off it; the
    // manifest this thread holds still says what is on disk, which is that
    // the scope was not changed.
    assert_eq!(windows, Windows::default());
    assert!(
        app.message().is_some_and(|line| !line.is_empty()),
        "a save that failed said nothing"
    );
    assert_ne!(app.message(), Some(LAST_KEY), "and said it here");
    assert_eq!(manifest, known(), "the failed write was believed");
    assert_eq!(
        fs::read_to_string(repo.path().join(".warlock"))
            .expect("the file in the way is still readable"),
        "not a directory\n"
    );
}

#[test]
fn a_directory_the_manifest_has_no_row_for_gets_one() {
    let repo = a_repo();
    let mut app = app_on(repo.path(), ROOT_ROW);
    let mut manifest = known();

    let windows = scope_submit(&mut app, &mut manifest, repo.path(), &field(".", "billing"));

    assert_eq!(windows, Windows::default());
    assert_eq!(scope_on(&manifest, "."), Some("billing"));
    assert_eq!(
        scope_on(&saved(repo.path()).expect("the submit wrote"), "."),
        Some("billing"),
    );
    assert_eq!(scope_on(&manifest, "crates/engine"), Some("data-plane"));
}

#[test]
fn the_write_goes_to_the_manifest_under_the_repository_root() {
    // Where the file lands, said once: `.warlock/scopes.toml` under the root
    // handed in, and nowhere else. The engine owns the path, and this is
    // the assertion that the root travelling through here is the one the
    // loop resolved.
    let repo = a_repo();
    let mut app = app_on(repo.path(), ENGINE_ROW);
    let mut manifest = known();

    scope_submit(
        &mut app,
        &mut manifest,
        repo.path(),
        &field("crates/engine", "billing"),
    );

    let written = repo.path().join(".warlock").join("scopes.toml");
    assert!(written.is_file(), "{} is not a file", written.display());
    assert!(
        fs::read_to_string(&written)
            .expect("the manifest reads back")
            .contains("billing")
    );
    // And nothing was left beside it: the save renames its temporary file
    // over the target rather than leaving one behind.
    let beside: Vec<String> = fs::read_dir(repo.path().join(".warlock"))
        .and_then(|entries| {
            entries
                .map(|entry| Ok(entry?.file_name().to_string_lossy().into_owned()))
                .collect::<io::Result<Vec<String>>>()
        })
        .expect("the manifest directory reads back");
    assert_eq!(beside, ["scopes.toml"]);
}

#[test]
fn typing_and_abandoning_move_the_prompt_and_nothing_else() {
    let repo = a_repo();
    let mut app = app_on(repo.path(), ENGINE_ROW);
    let mut manifest = scoped();
    let before = app.clone();
    let typed = field("crates/engine", "web");

    assert_eq!(
        scope_edit(
            &mut app,
            &mut manifest,
            repo.path(),
            &ScopePrompt::open("crates/engine", "we"),
            Edited::Open(typed.clone()),
        ),
        Windows::asking(typed),
        "a character typed did something other than land in the field",
    );
    assert_eq!(
        scope_edit(
            &mut app,
            &mut manifest,
            repo.path(),
            &ScopePrompt::open("crates/engine", "web"),
            Edited::Close,
        ),
        Windows::default(),
    );
    assert_eq!(saved(repo.path()), None, "typing wrote to disk");
    assert_eq!(manifest, scoped(), "typing edited the manifest");
    assert_eq!(app, before, "typing moved the view");
}

#[test]
fn a_submit_with_no_window_up_writes_nothing() {
    // The road `press_for` cannot take — it only consults `edit_for` while a
    // window is up — answered rather than left to be discovered.
    let repo = a_repo();
    let mut app = app_on(repo.path(), ENGINE_ROW);
    let mut manifest = scoped();

    let windows = scope_edit(
        &mut app,
        &mut manifest,
        repo.path(),
        &ScopePrompt::Closed,
        Edited::Submit,
    );

    assert_eq!(windows, Windows::default());
    assert_eq!(saved(repo.path()), None);
    assert_eq!(manifest, scoped());
}

#[test]
fn the_whole_path_from_the_press_to_the_saved_manifest_is_one_key_at_a_time() {
    // The event loop's own arms, in the order a reader presses them: `s`,
    // four characters, a Backspace over the last of them, Enter. Nothing
    // here is a terminal, a network or a `claude` — the whole path is these
    // three functions over a directory of this test's own.
    let repo = a_repo();
    let mut app = app_on(repo.path(), TUI_ROW);
    let mut manifest = known();

    let mut prompt = scope_press(&mut app, &manifest, repo.path());
    assert_eq!(prompt, ScopePrompt::open("crates/tui", ""));

    for code in [
        KeyCode::Char('w'),
        KeyCode::Char('e'),
        KeyCode::Char('b'),
        KeyCode::Char('x'),
        KeyCode::Backspace,
        KeyCode::Enter,
    ] {
        let edited = {
            let field = prompt.field().expect("the window is still up");
            edit_for(press(code), field)
        };
        let windows = scope_edit(&mut app, &mut manifest, repo.path(), &prompt, edited);
        assert_eq!(
            windows.record,
            RecordPrompt::Closed,
            "`web` is recorded in this fixture, so nothing here asks for one",
        );
        prompt = windows.scope;
    }

    assert_eq!(prompt, ScopePrompt::Closed, "Enter left the window up");
    assert_eq!(scope_on(&manifest, "crates/tui"), Some("web"));
    assert_eq!(
        scope_on(&saved(repo.path()).expect("the keys wrote"), "crates/tui"),
        Some("web"),
    );
    // And the message line came through the whole of it untouched.
    assert_eq!(app.message(), Some(LAST_KEY));
}

#[test]
fn a_refusal_typed_out_can_be_fixed_and_written_without_reopening_the_window() {
    // Enter on something the engine turns down, then a Backspace and a
    // character over the very text that was refused: the window stayed up
    // holding it, which is the whole point of a refusal not closing.
    let repo = a_repo();
    let mut app = app_on(repo.path(), TUI_ROW);
    let mut manifest = known();
    let mut prompt = ScopePrompt::open("crates/tui", "web!");

    for code in [KeyCode::Enter, KeyCode::Backspace, KeyCode::Enter] {
        let edited = {
            let field = prompt.field().expect("the window is still up");
            edit_for(press(code), field)
        };
        prompt = scope_edit(&mut app, &mut manifest, repo.path(), &prompt, edited).scope;
        if code == KeyCode::Backspace {
            assert_eq!(saved(repo.path()), None, "the refused text was written");
        }
    }

    assert_eq!(prompt, ScopePrompt::Closed);
    assert_eq!(scope_on(&manifest, "crates/tui"), Some("web"));
}

/// A repository with more in it than the write touches: two `[[directory]]`
/// rows and three `[[scope]]` records, none of which the record writes below
/// are allowed to change.
///
/// Bigger than [`scoped`] on purpose. A rebuild that dropped everything it
/// did not write would pass over a file holding one untouched block, and the
/// defect this fixture exists for dropped a whole section at once.
fn recorded() -> Manifest {
    Manifest::with_directories([
        row("crates/engine", "data-plane"),
        row("docs", "third-party"),
    ])
    .with_scopes(records().into_iter().chain([ScopeRecord::new(
        "platform",
        "Platform",
        "Backlog",
        "area/platform",
    )]))
}

/// Whether `warlock check` would find a record for the scope covering
/// `module`, asked of the router itself rather than restated here.
///
/// No home, which is the key half of the facts and none of this test's
/// business; the record half is read the same either way.
fn routes_to_a_record(manifest: &Manifest, module: &str) -> bool {
    route_facts(module, ".", manifest, None)
        .expect("the module path is inside the root")
        .record()
        .is_some()
}

/// `form` with `values` typed into its fields, through the same keys a reader
/// presses: Tab between them, so the text lands where the focus is rather than
/// being put into the form behind the prompt's back.
///
/// As many values as the form has fields — three for a Linear form and four for
/// a Jira one.
fn filled<const N: usize>(form: &RecordForm, values: [&str; N]) -> RecordForm {
    let mut form = form.clone();
    for value in values {
        for character in value.chars() {
            form = opened(press(KeyCode::Char(character)), &form);
        }
        form = opened(press(KeyCode::Tab), &form);
    }
    form
}

fn opened(key: KeyEvent, form: &RecordForm) -> RecordForm {
    match record_edit_for(key, form) {
        RecordEdited::Open(next) => next,
        other => panic!("{key:?} should have left the record window up, and gave {other:?}"),
    }
}

// The home handed to a record submit by the tests below that are about the
// manifest rather than about the question that follows a save: a machine with
// no home has no config a binding could land in, so it writes the scope and its
// record and asks for no key. The home the asking itself is driven over is a
// directory of the test's own — see [`a_home`].
const NO_HOME: Option<&Path> = None;

/// The record window [`scope_submit`] puts up over `text`, panicking when the
/// name took the write road instead.
fn asked_for(
    app: &mut App,
    manifest: &mut Manifest,
    root: &Path,
    module: &str,
    text: &str,
) -> RecordForm {
    let windows = scope_submit(app, manifest, root, &field(module, text));
    assert_eq!(
        windows.scope,
        ScopePrompt::Closed,
        "the first window stayed up over `{text}`"
    );
    windows
        .record
        .form()
        .unwrap_or_else(|| panic!("`{text}` was written rather than asked about"))
        .clone()
}

#[test]
fn a_name_no_record_claims_puts_the_second_window_up_and_writes_nothing() {
    let repo = a_repo();
    let mut app = app_on(repo.path(), TUI_ROW);
    let mut manifest = scoped();
    let before = app.clone();

    let windows = scope_submit(
        &mut app,
        &mut manifest,
        repo.path(),
        &field("crates/tui", "Billing"),
    );

    // The first window down and the second one up over the folded name, which
    // is the name the record will be filed under and the one the row will
    // carry — a second fold or a second read of the field could disagree.
    assert_eq!(
        windows,
        Windows {
            scope: ScopePrompt::Closed,
            record: RecordPrompt::open(RecordTracker::Linear, "crates/tui", "billing"),
            key: KeyPrompt::Closed,
        },
    );
    assert_eq!(saved(repo.path()), None, "the question wrote to disk");
    assert_eq!(manifest, scoped(), "the question edited the manifest");
    assert_eq!(app, before, "the question moved the view");
}

#[test]
fn a_name_that_already_has_a_record_is_written_with_no_second_window() {
    let repo = a_repo();
    let mut app = app_on(repo.path(), TUI_ROW);
    let mut manifest = scoped().with_scopes(records());

    let windows = scope_submit(
        &mut app,
        &mut manifest,
        repo.path(),
        &field("crates/tui", "third-party"),
    );

    assert_eq!(
        windows,
        Windows::default(),
        "a recorded name was asked about"
    );
    assert_eq!(
        scope_on(&saved(repo.path()).expect("the submit wrote"), "crates/tui"),
        Some("third-party"),
    );
    assert_eq!(
        manifest.scopes(),
        records(),
        "the write added a second record for a name that had one",
    );
}

#[test]
fn an_empty_field_clears_the_scope_and_asks_for_no_record() {
    // The clear is not a name, so there is nothing to record: the road to the
    // second window is the one thing an empty field must not take, whatever
    // the manifest records.
    let repo = a_repo();
    let mut app = app_on(repo.path(), ENGINE_ROW);
    let mut manifest = scoped();

    let windows = scope_submit(
        &mut app,
        &mut manifest,
        repo.path(),
        &field("crates/engine", ""),
    );

    assert_eq!(windows, Windows::default());
    assert_eq!(scope_on(&manifest, "crates/engine"), None);
    assert_eq!(
        scope_on(
            &saved(repo.path()).expect("the clear wrote the manifest"),
            "crates/engine"
        ),
        None,
    );
}

#[test]
fn a_blank_value_is_refused_under_its_own_field_and_writes_nothing() {
    let repo = a_repo();

    for (which, values) in [
        (RecordField::TeamKey, ["", "In Review", "area/billing"]),
        (RecordField::ReviewState, ["Billing", "   ", "area/billing"]),
        (RecordField::Label, ["Billing", "In Review", ""]),
    ] {
        let mut app = app_on(repo.path(), TUI_ROW);
        let mut manifest = scoped();
        let before = app.clone();
        let typed = filled(
            &RecordForm::new(RecordTracker::Linear, "crates/tui", "billing"),
            values,
        );

        let windows = record_submit(&mut app, &mut manifest, repo.path(), NO_HOME, &typed);

        let form = windows
            .record
            .form()
            .unwrap_or_else(|| panic!("a blank {} closed the window", which.name()));
        let rule = form
            .field(which)
            .and_then(ScopeField::rule)
            .unwrap_or_else(|| panic!("nothing was said under the {}", which.name()))
            .to_owned();
        assert!(
            rule.contains(which.name()),
            "the complaint does not name the field it is about: {rule}",
        );
        // Everything else exactly as it was typed, cursors included — one
        // keystroke away from being fixed, in the field that was refused.
        assert_eq!(
            windows.record,
            RecordPrompt::Open(typed.refused(which, rule))
        );
        assert_eq!(saved(repo.path()), None, "a refusal wrote to disk");
        assert_eq!(manifest, scoped(), "a refusal edited the manifest");
        assert_eq!(app, before, "a refusal moved the view");
    }
}

#[test]
fn a_blank_team_key_is_refused_in_the_words_the_field_is_labelled_in() {
    let repo = a_repo();
    let mut app = app_on(repo.path(), TUI_ROW);
    let mut manifest = scoped();
    let typed = filled(
        &RecordForm::new(RecordTracker::Linear, "crates/tui", "billing"),
        ["", "In Review", "area/billing"],
    );

    let windows = record_submit(&mut app, &mut manifest, repo.path(), NO_HOME, &typed);

    // Spelled out rather than built from `name()`: the sentence a reader is
    // stopped by is the one place the value is called a key, so it is read
    // here as a reader reads it.
    let form = windows
        .record
        .form()
        .expect("a blank team key closed the window");
    assert_eq!(
        form.field(RecordField::TeamKey).and_then(ScopeField::rule),
        Some("a team key cannot be blank"),
    );
}

#[test]
fn a_blank_jira_value_is_refused_under_its_own_field_and_writes_nothing() {
    let repo = a_repo();

    for (which, values) in [
        (
            RecordField::BaseUrl,
            ["", "PAY", "In Review", "area/payments"],
        ),
        (
            RecordField::ProjectKey,
            [JIRA_SITE, "  ", "In Review", "area/payments"],
        ),
        (
            RecordField::ReviewState,
            [JIRA_SITE, "PAY", "", "area/payments"],
        ),
        (RecordField::Label, [JIRA_SITE, "PAY", "In Review", ""]),
    ] {
        let mut app = app_on(repo.path(), TUI_ROW);
        let mut manifest = scoped();
        let typed = filled(
            &RecordForm::new(RecordTracker::Jira, "crates/tui", "payments"),
            values,
        );

        let windows = record_submit(&mut app, &mut manifest, repo.path(), NO_HOME, &typed);

        let form = windows
            .record
            .form()
            .unwrap_or_else(|| panic!("a blank {} closed the window", which.name()));
        let rule = form
            .field(which)
            .and_then(ScopeField::rule)
            .unwrap_or_else(|| panic!("nothing was said under the {}", which.name()));
        assert_eq!(rule, format!("a {} cannot be blank", which.name()));
        assert_eq!(saved(repo.path()), None, "a refusal wrote to disk");
        assert_eq!(manifest, scoped(), "a refusal edited the manifest");
    }
}

#[test]
fn two_blank_fields_are_refused_under_the_upper_one_in_either_form() {
    // Where the reader is already looking, which is the field the form's own
    // order puts first: Jira's base URL above its review state, and Linear's
    // team key above its label.
    let repo = a_repo();

    for (form, upper, lower) in [
        (
            filled(
                &RecordForm::new(RecordTracker::Jira, "crates/tui", "payments"),
                ["", "PAY", "   ", "area/payments"],
            ),
            RecordField::BaseUrl,
            RecordField::ReviewState,
        ),
        (
            filled(
                &RecordForm::new(RecordTracker::Linear, "crates/tui", "billing"),
                ["", "In Review", "  "],
            ),
            RecordField::TeamKey,
            RecordField::Label,
        ),
    ] {
        let mut app = app_on(repo.path(), TUI_ROW);
        let mut manifest = scoped();

        let windows = record_submit(&mut app, &mut manifest, repo.path(), NO_HOME, &form);

        let refused = windows
            .record
            .form()
            .expect("two blank fields closed the window")
            .clone();
        assert_eq!(
            refused.field(upper).and_then(ScopeField::rule),
            Some(format!("a {} cannot be blank", upper.name()).as_str()),
        );
        assert_eq!(
            refused.field(lower).and_then(ScopeField::rule),
            None,
            "the lower blank field was complained about too",
        );
        assert_eq!(
            refused.focus(),
            upper,
            "the cursor is not in the field that was refused",
        );
    }
}

#[test]
fn a_jira_form_writes_the_record_its_own_four_fields_describe() {
    // The Jira road through the same submit: the four values the form holds go
    // in as typed, and what lands is the record `ScopeRecord::jira` builds out
    // of them rather than a Linear one missing a team.
    let repo = a_repo();
    let mut app = app_on(repo.path(), TUI_ROW);
    let mut manifest = scoped();
    let form = filled(
        &RecordForm::new(RecordTracker::Jira, "crates/tui", "payments"),
        [JIRA_SITE, "PAY", "In Review", "area/payments"],
    );

    let windows = record_submit(&mut app, &mut manifest, repo.path(), NO_HOME, &form);

    assert_eq!(
        windows.record,
        RecordPrompt::Closed,
        "the window is answered"
    );
    let written = saved(repo.path()).expect("the submit wrote the manifest");
    assert_eq!(
        written, manifest,
        "what is on disk is what this thread believes"
    );
    assert_eq!(scope_on(&written, "crates/tui"), Some("payments"));
    let record = written.scopes().last().expect("the record was written");
    assert_eq!(
        record,
        &ScopeRecord::jira("payments", JIRA_SITE, "PAY", "In Review", "area/payments"),
    );
    assert!(
        LinearScope::of(record).is_err(),
        "a Jira form wrote a Linear record",
    );
}

#[test]
fn esc_on_the_record_window_leaves_the_file_byte_identical() {
    let repo = a_repo();
    let fixture = recorded();
    fixture.save(repo.path()).expect("the fixture was written");
    let path = repo.path().join(".warlock").join("scopes.toml");
    let before = fs::read(&path).expect("the fixture reads back");
    let mut app = app_on(repo.path(), TUI_ROW);
    let mut manifest = fixture.clone();

    let form = asked_for(
        &mut app,
        &mut manifest,
        repo.path(),
        "crates/tui",
        "billing",
    );
    // The loop's own Esc arm, through the pure function that decides it, over
    // a window with all three values in it: there is nothing to put back,
    // because nothing was written when the question was asked.
    assert_eq!(
        record_edit_for(
            press(KeyCode::Esc),
            &filled(&form, ["Billing", "In Review", "area/billing"]),
        ),
        RecordEdited::Close,
    );

    assert_eq!(
        fs::read(&path).expect("the manifest reads back"),
        before,
        "Esc moved a byte of the file",
    );
    assert_eq!(
        manifest, fixture,
        "Esc edited the manifest this thread holds"
    );
    assert_eq!(
        scope_on(&manifest, "crates/tui"),
        None,
        "Esc scoped the directory"
    );
}

#[test]
fn the_record_submit_writes_the_scope_and_the_record_in_one_save() {
    let repo = a_repo();
    let mut app = app_on(repo.path(), TUI_ROW);
    let mut manifest = recorded();
    let before = app.clone();

    // Both windows in turn, as a reader answers them: the folded name comes
    // out of the first and the three values go into the second, and nothing is
    // written until the second is answered.
    let form = asked_for(
        &mut app,
        &mut manifest,
        repo.path(),
        "crates/tui",
        "Billing",
    );
    assert_eq!(saved(repo.path()), None, "the question wrote to disk");
    let windows = record_submit(
        &mut app,
        &mut manifest,
        repo.path(),
        NO_HOME,
        &filled(&form, [" Billing Squad ", "In Review", "Area/Billing"]),
    );

    assert_eq!(
        windows.record,
        RecordPrompt::Closed,
        "the window is answered"
    );
    let written = saved(repo.path()).expect("the submit wrote the manifest");
    assert_eq!(
        written, manifest,
        "what is on disk is what this thread believes"
    );
    assert_eq!(scope_on(&written, "crates/tui"), Some("billing"));
    // As typed and nowhere near a trim or a fold: these three belong to
    // somebody's tracker.
    let record = written.scopes().last().expect("the record was written");
    assert_eq!(record.name(), "billing");
    assert_eq!(team_of(record), " Billing Squad ");
    assert_eq!(record.review_state(), "In Review");
    assert_eq!(record.label(), "Area/Billing");
    assert_eq!(app, before, "the write moved the view");
}

#[test]
fn the_router_finds_the_team_review_state_and_label_the_window_was_given() {
    // What `warlock check` prints for the directory just scoped, asked of the
    // router itself: a record filed under a name the row does not carry would
    // route nowhere, which is the whole reason the second window exists.
    let repo = a_repo();
    let mut app = app_on(repo.path(), TUI_ROW);
    let mut manifest = recorded();

    let form = asked_for(
        &mut app,
        &mut manifest,
        repo.path(),
        "crates/tui",
        "Billing",
    );
    record_submit(
        &mut app,
        &mut manifest,
        repo.path(),
        NO_HOME,
        &filled(&form, ["Billing", "In Review", "area/billing"]),
    );

    let written = saved(repo.path()).expect("the submit wrote the manifest");
    assert!(routes_to_a_record(&written, "crates/tui"));
    let facts =
        route_facts("crates/tui", ".", &written, None).expect("the module path is inside the root");
    let record = facts.record().expect("the router found no record");
    assert_eq!(team_of(record), "Billing");
    assert_eq!(record.review_state(), "In Review");
    assert_eq!(record.label(), "area/billing");
}

#[test]
fn a_name_recorded_since_the_window_opened_is_not_rewritten() {
    // The manifest gaining the record between the two windows, which is the
    // one road to a `Recorded` refusal from here. Said out loud: a window that
    // came down silently would be the outcome a reader cannot tell from a
    // write.
    let repo = a_repo();
    let mut app = app_on(repo.path(), TUI_ROW);
    let mut manifest = recorded();
    let form = filled(
        &RecordForm::new(RecordTracker::Linear, "crates/tui", "data-plane"),
        ["Someone Else", "Done", "area/other"],
    );

    let windows = record_submit(&mut app, &mut manifest, repo.path(), NO_HOME, &form);

    assert_eq!(windows.record, RecordPrompt::Closed);
    assert_eq!(
        saved(repo.path()),
        None,
        "a record already there was rewritten"
    );
    assert_eq!(manifest, recorded(), "and the manifest was edited");
    let message = app.message().expect("the refusal said nothing");
    assert!(message.contains("data-plane"), "{message}");
}

#[test]
fn a_record_that_will_not_save_puts_its_reason_on_the_message_line() {
    // The same disk that will not take a scope, for the write that carries two
    // halves: the manifest this thread holds still says what is on disk, which
    // is neither half.
    let repo = a_repo();
    fs::write(repo.path().join(".warlock"), "not a directory\n")
        .expect("writes a file in the way of the manifest directory");
    let mut app = app_on(repo.path(), TUI_ROW);
    let mut manifest = scoped();
    let form = filled(
        &RecordForm::new(RecordTracker::Linear, "crates/tui", "billing"),
        ["Billing", "In Review", "area/billing"],
    );

    let windows = record_submit(&mut app, &mut manifest, repo.path(), NO_HOME, &form);

    assert_eq!(windows.record, RecordPrompt::Closed);
    assert_ne!(
        app.message(),
        Some(LAST_KEY),
        "a save that failed said nothing"
    );
    assert_eq!(manifest, scoped(), "the failed write was believed");
}

#[test]
fn the_whole_path_through_both_windows_is_one_key_at_a_time() {
    // The event loop's own arms over both windows, in the order a reader
    // presses them: `s`, a name nothing records, Enter, the three values with a
    // Tab between them, Enter. Nothing here is a terminal, a worker or a
    // `claude` — the whole path is these four functions over a directory of
    // this test's own, and the two windows are never up together.
    let repo = a_repo();
    let mut app = app_on(repo.path(), TUI_ROW);
    let mut manifest = recorded();

    let mut prompt = scope_press(&mut app, &manifest, repo.path());
    assert_eq!(prompt, ScopePrompt::open("crates/tui", ""));

    let mut record = RecordPrompt::Closed;
    for code in "billing".chars().map(KeyCode::Char).chain([KeyCode::Enter]) {
        let edited = {
            let field = prompt.field().expect("the first window is still up");
            edit_for(press(code), field)
        };
        let windows = scope_edit(&mut app, &mut manifest, repo.path(), &prompt, edited);
        prompt = windows.scope;
        record = windows.record;
    }

    // The first window down and the second up, out of the one Enter: a name
    // nothing records is not written by that key at all.
    assert_eq!(prompt, ScopePrompt::Closed);
    assert_eq!(
        record,
        RecordPrompt::open(RecordTracker::Linear, "crates/tui", "billing")
    );
    assert_eq!(saved(repo.path()), None, "the first Enter wrote to disk");
    assert_eq!(manifest, recorded(), "the first Enter edited the manifest");

    for code in ["Billing Squad", "In Review", "area/billing"]
        .into_iter()
        .flat_map(|value| value.chars().map(KeyCode::Char).chain([KeyCode::Tab]))
        .chain([KeyCode::Enter])
    {
        assert_eq!(saved(repo.path()), None, "{code:?} wrote before the submit");
        let edited = {
            let form = record.form().expect("the second window is still up");
            record_edit_for(press(code), form)
        };
        record = record_edit(
            &mut app,
            &mut manifest,
            repo.path(),
            NO_HOME,
            &record,
            edited,
        )
        .record;
    }

    assert_eq!(record, RecordPrompt::Closed, "Enter left the window up");
    // One file, holding both halves: the directory row carries the folded name and
    // the record filed under it carries the three values, so `warlock check`
    // on this directory has a route to print.
    let written = saved(repo.path()).expect("the keys wrote");
    assert_eq!(
        written, manifest,
        "what is on disk is what the loop believes"
    );
    assert_eq!(scope_on(&written, "crates/tui"), Some("billing"));
    let record = written.scopes().last().expect("the record was written");
    assert_eq!(record.name(), "billing");
    assert_eq!(team_of(record), "Billing Squad");
    assert_eq!(record.review_state(), "In Review");
    assert_eq!(record.label(), "area/billing");
    // And the message line came through the whole of it untouched.
    assert_eq!(app.message(), Some(LAST_KEY));
}

#[test]
fn a_pull_in_flight_turns_s_down_on_the_thread_and_opens_no_window() {
    let repo = a_repo();
    let mut app = app_on(repo.path(), ENGINE_ROW);

    let prompt = super::scope_press(
        &mut app,
        &scoped(),
        repo.path(),
        &OpenScopes::named(["*"]),
        Some("`WAR-140` is being pulled"),
        Instant::now(),
    );

    assert_eq!(prompt, ScopePrompt::Closed);
    let said: Vec<String> = app
        .panel()
        .thread()
        .map(|thread| thread.lines(Instant::now()))
        .unwrap_or_default()
        .into_iter()
        .filter_map(|line| match line {
            crate::account::Line::Note { text } => Some(text),
            _ => None,
        })
        .collect();
    assert_eq!(said, ["`WAR-140` is being pulled; no scope was written"]);
}

#[test]
fn a_pull_in_flight_is_answered_before_the_boundary_is() {
    let repo = a_repo();
    let mut app = app_on(repo.path(), ENGINE_ROW);

    let prompt = super::scope_press(
        &mut app,
        &scoped(),
        repo.path(),
        &OpenScopes::named(["web"]),
        Some("`WAR-140` is being pulled"),
        Instant::now(),
    );

    // Both refusals apply, and the pull's goes on the thread: the boundary's
    // sentence is not put on the message line over it.
    assert_eq!(prompt, ScopePrompt::Closed);
    assert_eq!(
        app.message(),
        Some(LAST_KEY),
        "the boundary spoke as well as the pull"
    );
}

/// The one value the key window must never touch, stored under all three names
/// so that any sighting of it below is this path having read a key.
const SECRET: &str = "lin_api_this_value_must_never_be_held";

/// A machine holding three keys, as `warlock key add` would have left it: the
/// window's rows are the store's names, so a binding needs a store to pick from.
fn a_home() -> TempDir {
    let home = tempfile::tempdir().expect("a temporary directory");
    for name in ["work", "acme", "personal"] {
        save_key(home.path(), name, SECRET).expect("a key stores");
    }
    home
}

/// The names the store lists, which is the order the rows are in and so which
/// row each press of Down lands on: the store answers sorted.
const NAMES: [&str; 3] = ["acme", "personal", "work"];

/// The scope the window is opened over, which the fixture manifest records.
const KEY_SCOPE: &str = "data-plane";

/// A second scope of the same checkout, bound before the window opens: what a
/// save here must carry through rather than replace.
const OTHER_SCOPE: &str = "third-party";

/// The window as [`KeyPrompt::open`] builds it over `scope`, with the first row
/// lit.
fn a_choice(home: &Path, scope: &str) -> KeyChoice {
    KeyPrompt::open(home, scope)
        .expect("a store that reads opens a window")
        .choice()
        .expect("an open prompt holds its choice")
        .clone()
}

/// The name `scope` is bound to in this checkout's config, where a checkout
/// with no config at all and a scope the config does not name are both the one
/// answer: unbound.
fn bound(home: &Path, root: &Path, scope: &str) -> Option<String> {
    match load_key_binding(home, root, scope) {
        Ok(name) => name,
        Err(config::Error::NotFound { .. }) => None,
        Err(error) => panic!("the config could not be read: {error}"),
    }
}

/// The config file byte for byte, or `None` when none was ever written for this
/// checkout — which is what an Esc over an unconfigured one has to leave.
fn config(home: &Path, root: &Path) -> Option<Vec<u8>> {
    let path = config_path(home, root);
    match fs::read(&path) {
        Ok(bytes) => Some(bytes),
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => panic!("{} could not be read: {error}", path.display()),
    }
}

#[test]
fn enter_binds_the_lit_row_to_the_windows_scope_and_says_nothing() {
    let repo = a_repo();
    let home = a_home();
    let mut app = app_on(repo.path(), ENGINE_ROW);
    let before = app.clone();
    let choice = a_choice(home.path(), KEY_SCOPE);

    let prompt = key_submit(&mut app, home.path(), repo.path(), &choice);

    assert_eq!(prompt, KeyPrompt::Closed, "the window is answered");
    // The row that was lit, asked of the window rather than spelled out twice:
    // what reached the config has to be the name the reader was looking at.
    assert_eq!(
        bound(home.path(), repo.path(), KEY_SCOPE).as_deref(),
        choice.highlighted(),
    );
    assert_eq!(choice.highlighted(), Some(NAMES[0]));
    // A successful write says nothing at all, not even spending the last
    // keystroke's sentence, and moves nothing about the view.
    assert_eq!(
        app.message(),
        Some(LAST_KEY),
        "the write took the message line"
    );
    assert_eq!(app, before, "the write moved the view");
    // Nothing else in the checkout moved: the binding is a fact about this
    // machine, so the manifest was never opened.
    assert_eq!(saved(repo.path()), None, "a binding wrote the manifest");
    // And a name is all that reached the file. There is no field a value could
    // have gone in, and this is the assertion that none went anywhere else.
    let written =
        String::from_utf8(config(home.path(), repo.path()).expect("the submit wrote the config"))
            .expect("the config is text");
    assert!(written.contains(NAMES[0]), "{written}");
    assert!(!written.contains(SECRET), "a key value reached the config");
}

#[test]
fn the_whole_path_from_the_key_window_to_the_written_binding_is_one_key_at_a_time() {
    // The event loop's own arms, in the order a reader presses them: one Down
    // off the row that opened lit, then Enter. Nothing here is a terminal — the
    // whole path is these two functions over a home and a repository of this
    // test's own.
    let repo = a_repo();
    let home = a_home();
    let mut app = app_on(repo.path(), ENGINE_ROW);
    // A binding already in the file for another scope of the same checkout,
    // which this write must carry through rather than replace.
    save_key_binding(home.path(), repo.path(), OTHER_SCOPE, NAMES[2])
        .expect("the other scope binds");
    let mut prompt = KeyPrompt::Open(a_choice(home.path(), KEY_SCOPE));

    for code in [KeyCode::Down, KeyCode::Enter] {
        assert_eq!(
            bound(home.path(), repo.path(), KEY_SCOPE),
            None,
            "{code:?} wrote before the submit"
        );
        let edited = {
            let choice = prompt.choice().expect("the window is still up");
            key_edit_for(press(code), choice)
        };
        prompt = key_edit(&mut app, home.path(), repo.path(), &prompt, edited);
    }

    assert_eq!(prompt, KeyPrompt::Closed, "Enter left the window up");
    assert_eq!(
        bound(home.path(), repo.path(), KEY_SCOPE).as_deref(),
        Some(NAMES[1]),
        "the Down was not the row that was bound",
    );
    assert_eq!(
        bound(home.path(), repo.path(), OTHER_SCOPE).as_deref(),
        Some(NAMES[2]),
        "binding one scope took another scope's key",
    );
    assert_eq!(app.message(), Some(LAST_KEY));
}

#[test]
fn esc_on_the_key_window_writes_nothing_and_leaves_the_scope_unbound() {
    let repo = a_repo();
    let home = a_home();
    let mut app = app_on(repo.path(), ENGINE_ROW);
    let before = app.clone();
    // Both checkouts an Esc can happen over: one already configured, where
    // "unchanged" is the file's own bytes, and one with no config at all, where
    // it is the file still not being there.
    for configured in [false, true] {
        if configured {
            save_key_binding(home.path(), repo.path(), OTHER_SCOPE, NAMES[2])
                .expect("the other scope binds");
        }
        let was = config(home.path(), repo.path());
        assert_eq!(was.is_some(), configured);
        let choice = a_choice(home.path(), KEY_SCOPE);

        // The loop's own Esc arm, through the pure function that decides it:
        // the window comes down and the config is not opened, because nothing
        // was written when the question was asked.
        assert_eq!(key_edit_for(press(KeyCode::Esc), &choice), KeyEdited::Close);
        let prompt = key_edit(
            &mut app,
            home.path(),
            repo.path(),
            &KeyPrompt::Open(choice),
            KeyEdited::Close,
        );

        assert_eq!(prompt, KeyPrompt::Closed);
        assert_eq!(
            config(home.path(), repo.path()),
            was,
            "Esc moved a byte of the config"
        );
        assert_eq!(
            bound(home.path(), repo.path(), KEY_SCOPE),
            None,
            "Esc bound the scope"
        );
        assert_eq!(app, before, "Esc moved the view");
        assert_eq!(app.message(), Some(LAST_KEY), "and it kept the last line");
    }
}

#[test]
fn a_binding_the_engine_refuses_says_so_on_the_message_line_and_takes_the_window_down() {
    // A file where this checkout's config directory has to be: the save cannot
    // make the directory, which is the cheapest real version of a config that
    // will not take the write.
    let repo = a_repo();
    let home = a_home();
    let directory = config_path(home.path(), repo.path())
        .parent()
        .expect("the config sits in a project directory")
        .to_path_buf();
    fs::write(&directory, "not a directory\n")
        .expect("writes a file in the way of the config directory");
    let mut app = app_on(repo.path(), ENGINE_ROW);
    let choice = a_choice(home.path(), KEY_SCOPE);

    let prompt = key_submit(&mut app, home.path(), repo.path(), &choice);

    // The reason goes on the footer and the window comes down off it, as
    // `record_submit` has it: a window that closed on a write that never
    // happened is the one outcome a reader cannot tell from success.
    assert_eq!(prompt, KeyPrompt::Closed);
    let message = app.message().expect("a save that failed said nothing");
    assert!(!message.is_empty(), "a save that failed said nothing");
    assert_ne!(app.message(), Some(LAST_KEY), "and said it here");
    assert!(!message.contains(SECRET), "a key value reached the footer");
    assert_eq!(
        fs::read_to_string(&directory).expect("the file in the way is still readable"),
        "not a directory\n"
    );
}

#[test]
fn a_key_submit_with_no_window_up_writes_nothing() {
    // The road `press_for` cannot take — it only consults `key_edit_for` while
    // the window is up — answered rather than left to be discovered.
    let repo = a_repo();
    let home = a_home();
    let mut app = app_on(repo.path(), ENGINE_ROW);

    let prompt = key_edit(
        &mut app,
        home.path(),
        repo.path(),
        &KeyPrompt::Closed,
        KeyEdited::Submit,
    );

    assert_eq!(prompt, KeyPrompt::Closed);
    assert_eq!(config(home.path(), repo.path()), None);
    assert_eq!(app.message(), Some(LAST_KEY));
}

/// The scope the chain below brings into being: a name [`recorded`] claims no
/// record for, so the press takes the road through all three windows.
const NEW_SCOPE: &str = "billing";

#[test]
fn the_whole_path_from_the_press_to_the_key_window_is_one_key_at_a_time() {
    // The event loop's own arms over all three windows, in the order a reader
    // presses them: `s`, a name nothing records, Enter, the three values with a
    // Tab between them, Enter. Nothing here is a terminal, a worker or a
    // `claude` — the whole path is these five functions over a repository and a
    // home of this test's own, and the three windows are never up together.
    let repo = a_repo();
    let home = a_home();
    let mut app = app_on(repo.path(), TUI_ROW);
    let mut manifest = recorded();

    let mut prompt = scope_press(&mut app, &manifest, repo.path());
    assert_eq!(prompt, ScopePrompt::open("crates/tui", ""));

    let mut record = RecordPrompt::Closed;
    for code in NEW_SCOPE.chars().map(KeyCode::Char).chain([KeyCode::Enter]) {
        let edited = {
            let field = prompt.field().expect("the first window is still up");
            edit_for(press(code), field)
        };
        let windows = scope_edit(&mut app, &mut manifest, repo.path(), &prompt, edited);
        // The first window never asks for a key, whatever it hands on to: there
        // is nothing recorded yet to bind a key to.
        assert_eq!(
            windows.key,
            KeyPrompt::Closed,
            "{code:?} on the first window asked for a key",
        );
        prompt = windows.scope;
        record = windows.record;
    }

    assert_eq!(prompt, ScopePrompt::Closed);
    assert_eq!(
        record,
        RecordPrompt::open(RecordTracker::Linear, "crates/tui", NEW_SCOPE)
    );

    let mut key = KeyPrompt::Closed;
    for code in ["Billing Squad", "In Review", "area/billing"]
        .into_iter()
        .flat_map(|value| value.chars().map(KeyCode::Char).chain([KeyCode::Tab]))
        .chain([KeyCode::Enter])
    {
        assert_eq!(
            config(home.path(), repo.path()),
            None,
            "{code:?} wrote the config before anything was recorded",
        );
        let edited = {
            let form = record.form().expect("the second window is still up");
            record_edit_for(press(code), form)
        };
        let windows = record_edit(
            &mut app,
            &mut manifest,
            repo.path(),
            Some(home.path()),
            &record,
            edited,
        );
        record = windows.record;
        key = windows.key;
    }

    // The second window down and the third up out of the one Enter, over the
    // scope that was just recorded and over the names this machine stores.
    assert_eq!(record, RecordPrompt::Closed, "Enter left the window up");
    assert_eq!(
        key,
        KeyPrompt::Open(a_choice(home.path(), NEW_SCOPE)),
        "the save did not end with the key window over the scope just recorded",
    );
    // The record is on disk before the question is asked, which is the order
    // that lets the question be abandoned.
    let written = saved(repo.path()).expect("the keys wrote the manifest");
    assert_eq!(
        written, manifest,
        "what is on disk is what the loop believes"
    );
    assert_eq!(scope_on(&written, "crates/tui"), Some(NEW_SCOPE));
    let record = written.scopes().last().expect("the record was written");
    assert_eq!(record.name(), NEW_SCOPE);
    assert_eq!(team_of(record), "Billing Squad");
    // And asking is not binding: nothing has reached this machine's config, and
    // the message line came through the whole of it untouched.
    assert_eq!(
        config(home.path(), repo.path()),
        None,
        "the question wrote the config",
    );
    assert_eq!(app.message(), Some(LAST_KEY));
}

#[test]
fn an_abandoned_key_window_leaves_the_scope_recorded_and_unbound() {
    // Esc on the third window, over a machine that holds keys and a scope whose
    // record is already on disk: the one outcome the fixed order exists for.
    let repo = a_repo();
    let home = a_home();
    let mut app = app_on(repo.path(), TUI_ROW);
    let mut manifest = recorded();

    let form = asked_for(
        &mut app,
        &mut manifest,
        repo.path(),
        "crates/tui",
        "Billing",
    );
    let windows = record_submit(
        &mut app,
        &mut manifest,
        repo.path(),
        Some(home.path()),
        &filled(&form, ["Billing", "In Review", "area/billing"]),
    );
    let choice = windows
        .key
        .choice()
        .expect("a record that saved ends with the key window up")
        .clone();
    assert_eq!(choice.scope(), NEW_SCOPE);
    let path = repo.path().join(".warlock").join("scopes.toml");
    let saved_bytes = fs::read(&path).expect("the submit wrote the manifest");

    // The loop's own Esc arm, through the pure function that decides it.
    assert_eq!(key_edit_for(press(KeyCode::Esc), &choice), KeyEdited::Close);
    let prompt = key_edit(
        &mut app,
        home.path(),
        repo.path(),
        &windows.key,
        KeyEdited::Close,
    );

    // Nothing written to this machine, and nothing taken back from the
    // repository: the record was saved first and the key asked second, so the
    // scope stands recorded and unbound rather than half written.
    assert_eq!(prompt, KeyPrompt::Closed);
    assert_eq!(
        config(home.path(), repo.path()),
        None,
        "Esc wrote the config",
    );
    assert_eq!(
        bound(home.path(), repo.path(), NEW_SCOPE),
        None,
        "Esc bound the scope",
    );
    assert_eq!(
        fs::read(&path).expect("the manifest reads back"),
        saved_bytes,
        "Esc moved a byte of the manifest",
    );
    let written = saved(repo.path()).expect("the manifest is still there");
    assert_eq!(
        written, manifest,
        "Esc edited the manifest this thread holds"
    );
    assert_eq!(
        scope_on(&written, "crates/tui"),
        Some(NEW_SCOPE),
        "Esc took the scope off the row",
    );
    assert!(
        written
            .scopes()
            .iter()
            .any(|record| record.name() == NEW_SCOPE),
        "Esc dropped the `[[scope]]` record: {written:?}",
    );
    assert_eq!(app.message(), Some(LAST_KEY), "and it kept the last line");
}

#[test]
fn a_record_that_will_not_save_asks_for_no_key_on_a_machine_holding_three() {
    // The same disk that will not take a scope, with a store that would have
    // answered: the save is what decides whether the question is asked, so the
    // chain ends on the message line and the third window stays down.
    let repo = a_repo();
    let home = a_home();
    fs::write(repo.path().join(".warlock"), "not a directory\n")
        .expect("writes a file in the way of the manifest directory");
    let mut app = app_on(repo.path(), TUI_ROW);
    let mut manifest = scoped();
    let form = filled(
        &RecordForm::new(RecordTracker::Linear, "crates/tui", NEW_SCOPE),
        ["Billing", "In Review", "area/billing"],
    );

    let windows = record_submit(
        &mut app,
        &mut manifest,
        repo.path(),
        Some(home.path()),
        &form,
    );

    // Neither window up, and the reason on the footer.
    assert_eq!(windows, Windows::default());
    assert!(
        app.message().is_some_and(|line| !line.is_empty()),
        "a save that failed said nothing",
    );
    assert_ne!(app.message(), Some(LAST_KEY), "and said it here");
    // The store is not why: asked straight out, this home opens a window. What
    // is missing is a record on disk to bind a key to.
    assert!(
        KeyPrompt::open(home.path(), NEW_SCOPE)
            .expect("the store reads")
            .is_open(),
        "the store would not have opened a window either",
    );
    assert_eq!(manifest, scoped(), "the failed write was believed");
    assert_eq!(
        config(home.path(), repo.path()),
        None,
        "a save that failed wrote the config",
    );
    assert_eq!(
        fs::read_to_string(repo.path().join(".warlock"))
            .expect("the file in the way is still readable"),
        "not a directory\n",
    );
}
