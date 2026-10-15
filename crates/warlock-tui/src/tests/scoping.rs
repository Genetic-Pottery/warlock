use std::path::Path;
use std::time::Instant;
use std::{fs, io};

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use tempfile::TempDir;
use warlock_engine::{
    Manifest, Node, ScopeRecord, ScopedDirectory, Tree, manifest, route_facts, validate_scope,
};

use super::{Windows, record_edit, record_submit, scope_edit, scope_submit};
use crate::app::{App, Sigils};
use crate::prompt::{
    Edited, RecordEdited, RecordField, RecordForm, RecordPrompt, ScopeField, ScopePrompt, edit_for,
    record_edit_for,
};

// `super::scope_press` with no boundary in the way, so these tests are about
// the prompt rather than about being refused. The wildcard rather than
// `Sigils::Nothing` because the fixture manifest *does* scope
// `crates/engine`, and holding nothing opens nothing that is scoped. Tests
// that are about the boundary call `super::scope_press` directly.
fn scope_press(app: &mut App, manifest: &Manifest, repo_root: &Path) -> ScopePrompt {
    super::scope_press(
        app,
        manifest,
        repo_root,
        &Sigils::held(["*"]),
        None,
        Instant::now(),
    )
}

// What no press of `s` and no Enter in its window is allowed to spend.
const LAST_KEY: &str = "something the last key said";

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

/// A field over `module` holding `text`, as the window would be by the time
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
const CLOSED: &str = "crates/engine is scoped `data-plane` — hold that sigil to work here, with `warlock sigil add data-plane`";

#[test]
fn s_is_refused_on_a_directory_whose_scope_this_machine_does_not_hold() {
    let repo = a_repo();
    let mut app = app_on(repo.path(), ENGINE_ROW);
    let before = app.clone();

    let prompt = super::scope_press(
        &mut app,
        &scoped(),
        repo.path(),
        &Sigils::held(["web"]),
        None,
        Instant::now(),
    );

    // You must hold a boundary to redraw it. Without this the one key whose
    // whole purpose is to move a scope would be the one key a scope did not
    // cover.
    assert_eq!(prompt, ScopePrompt::Closed, "the window must not open");
    assert_eq!(
        app.message(),
        Some(CLOSED),
        "the refusal names the scope wanted and where a sigil is recorded"
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
fn holding_a_matching_sigil_opens_the_prompt() {
    let repo = a_repo();

    for sigils in [
        Sigils::held(["data-plane"]),
        Sigils::held(["web", "data-plane"]),
        Sigils::held(["*"]),
    ] {
        let mut app = app_on(repo.path(), ENGINE_ROW);

        let prompt = super::scope_press(
            &mut app,
            &scoped(),
            repo.path(),
            &sigils,
            None,
            Instant::now(),
        );

        assert_eq!(
            prompt,
            ScopePrompt::open("crates/engine", "data-plane"),
            "{sigils:?} opens `data-plane`"
        );
        assert_eq!(app.message(), Some(LAST_KEY), "{sigils:?} said something");
    }
}

#[test]
fn a_machine_that_holds_no_sigil_is_refused_by_a_scoped_directory() {
    let repo = a_repo();

    for sigils in [
        // Nobody has run `warlock sigil add` here. A sigil is what opens a
        // scope, so holding none opens none — the empty set is not a
        // universal bypass, or the boundary would hold for exactly those
        // people who had already opted into being bound by it.
        Sigils::Nothing,
        // And a config that will not parse cannot establish that anything is
        // held, which is the same answer. The header says `holding unknown`
        // for as long as it lasts, so the two are told apart on screen.
        Sigils::Unknown,
    ] {
        let mut app = app_on(repo.path(), ENGINE_ROW);

        let prompt = super::scope_press(
            &mut app,
            &scoped(),
            repo.path(),
            &sigils,
            None,
            Instant::now(),
        );

        assert_eq!(
            prompt,
            ScopePrompt::Closed,
            "{sigils:?} opened `data-plane`"
        );
        assert_eq!(
            app.message(),
            Some(CLOSED),
            "{sigils:?} said the wrong thing"
        );
    }
}

#[test]
fn an_unscoped_directory_stays_open_to_a_machine_holding_nothing() {
    let repo = a_repo();
    let mut app = app_on(repo.path(), TUI_ROW);

    // Anyone may draw the first scope over a directory nothing covers: a scope
    // is the only thing that opens a directory to work, so if drawing one also
    // needed a scope, no repository could ever be set up.
    let prompt = super::scope_press(
        &mut app,
        &scoped(),
        repo.path(),
        &Sigils::Nothing,
        None,
        Instant::now(),
    );

    assert_eq!(prompt, ScopePrompt::open("crates/tui", ""));
}

#[test]
fn a_directory_no_scope_covers_is_open_to_a_machine_holding_something_else() {
    let repo = a_repo();
    let mut app = app_on(repo.path(), TUI_ROW);

    // `crates/tui` carries no scope and nothing above it does either, so
    // there is no boundary here to be outside of.
    let prompt = super::scope_press(
        &mut app,
        &scoped(),
        repo.path(),
        &Sigils::held(["data-plane"]),
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

    // Holding the outer scope alone does not reach the inner one: an outer
    // scope is a default for what said nothing below it, never a second gate
    // that `platform` could satisfy on `data-plane`'s behalf.
    let mut app = app_on(repo.path(), ENGINE_ROW);
    assert_eq!(
        super::scope_press(
            &mut app,
            &manifest,
            repo.path(),
            &Sigils::held(["platform"]),
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
            &Sigils::held(["platform"]),
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

/// `form` with `values` typed into its three fields, through the same keys a
/// reader presses: Tab between them, so the text lands where the focus is
/// rather than being put into the form behind the prompt's back.
fn filled(form: &RecordForm, values: [&str; 3]) -> RecordForm {
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
            record: RecordPrompt::open("crates/tui", "billing"),
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
        let typed = filled(&RecordForm::new("crates/tui", "billing"), values);

        let prompt = record_submit(&mut app, &mut manifest, repo.path(), &typed);

        let form = prompt
            .form()
            .unwrap_or_else(|| panic!("a blank {} closed the window", which.name()));
        let rule = form
            .field(which)
            .rule()
            .unwrap_or_else(|| panic!("nothing was said under the {}", which.name()))
            .to_owned();
        assert!(
            rule.contains(which.name()),
            "the complaint does not name the field it is about: {rule}",
        );
        // Everything else exactly as it was typed, cursors included — one
        // keystroke away from being fixed, in the field that was refused.
        assert_eq!(prompt, RecordPrompt::Open(typed.refused(which, rule)));
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
        &RecordForm::new("crates/tui", "billing"),
        ["", "In Review", "area/billing"],
    );

    let prompt = record_submit(&mut app, &mut manifest, repo.path(), &typed);

    // Spelled out rather than built from `name()`: the sentence a reader is
    // stopped by is the one place the value is called a key, so it is read
    // here as a reader reads it.
    let form = prompt.form().expect("a blank team key closed the window");
    assert_eq!(
        form.field(RecordField::TeamKey).rule(),
        Some("a team key cannot be blank"),
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
    let prompt = record_submit(
        &mut app,
        &mut manifest,
        repo.path(),
        &filled(&form, [" Billing Squad ", "In Review", "Area/Billing"]),
    );

    assert_eq!(prompt, RecordPrompt::Closed, "the window is answered");
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
    assert_eq!(record.team_key(), " Billing Squad ");
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
        &filled(&form, ["Billing", "In Review", "area/billing"]),
    );

    let written = saved(repo.path()).expect("the submit wrote the manifest");
    assert!(routes_to_a_record(&written, "crates/tui"));
    let facts =
        route_facts("crates/tui", ".", &written, None).expect("the module path is inside the root");
    let record = facts.record().expect("the router found no record");
    assert_eq!(record.team_key(), "Billing");
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
        &RecordForm::new("crates/tui", "data-plane"),
        ["Someone Else", "Done", "area/other"],
    );

    let prompt = record_submit(&mut app, &mut manifest, repo.path(), &form);

    assert_eq!(prompt, RecordPrompt::Closed);
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
        &RecordForm::new("crates/tui", "billing"),
        ["Billing", "In Review", "area/billing"],
    );

    let prompt = record_submit(&mut app, &mut manifest, repo.path(), &form);

    assert_eq!(prompt, RecordPrompt::Closed);
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
    assert_eq!(record, RecordPrompt::open("crates/tui", "billing"));
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
        record = record_edit(&mut app, &mut manifest, repo.path(), &record, edited);
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
    assert_eq!(record.team_key(), "Billing Squad");
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
        &Sigils::held(["*"]),
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
        &Sigils::held(["web"]),
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
