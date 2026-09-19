use std::path::{Path, PathBuf};
use std::{fs, io};

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use tempfile::TempDir;
use warlock_engine::{
    Manifest, Node, NodeState, PactEntry, ScopeRecord, Tree, manifest, route_facts, validate_scope,
};
use warlock_tui::{App, Edited, ScopeField, ScopePrompt, Sigils, edit_for};

use super::{records_scope, scope_edit, scope_submit, with_scope_recorded};

// `super::scope_press` with no boundary in the way, so these tests are about
// the prompt rather than about being refused. The wildcard rather than
// `Sigils::Nothing` because the fixture manifest *does* scope
// `crates/engine`, and holding nothing opens nothing that is scoped. Tests
// that are about the boundary call `super::scope_press` directly.
fn scope_press(
    app: &mut App,
    manifest: &Manifest,
    repo_root: &Path,
    in_flight: bool,
) -> ScopePrompt {
    super::scope_press(app, manifest, repo_root, &Sigils::held(["*"]), in_flight)
}

// A grant on every entry, so "the write left the grant alone" is an
// assertion about two values that are really there.
const HASH: &str = "d0f5a1";

const AT: &str = "2026-08-19T07:32:00Z";

// What no press of `s` and no Enter in its window is allowed to spend.
const LAST_KEY: &str = "something the last key said";

// The tree below is written out by hand rather than loaded, so nothing is
// inside this until a scope is saved.
fn a_repo() -> TempDir {
    tempfile::tempdir().expect("a temporary directory")
}

/// The tree the app is built over, rooted at `root`.
///
/// Two pacted directories and one unpacted one, which is everything the
/// press has to tell apart: `crates/engine` is fresh and carries a scope in
/// the manifest below, `crates/tui` is stale and carries none, and `crates`
/// itself is pacted by nothing.
///
/// ```text
/// <root>                      pacted, stale
/// └── crates                  unpacted
///     ├── crates/engine       pacted, fresh
///     └── crates/tui          pacted, stale
/// ```
fn tree(root: &Path) -> Tree {
    Tree::new(
        Node::new(root, root.join("WARLOCK.md"), NodeState::PactedStale).with_children([
            Node::new(root.join("crates"), None::<PathBuf>, NodeState::Unpacted).with_children([
                Node::new(
                    root.join("crates/engine"),
                    root.join("crates/engine/WARLOCK.md"),
                    NodeState::PactedFresh,
                ),
                Node::new(
                    root.join("crates/tui"),
                    root.join("crates/tui/WARLOCK.md"),
                    NodeState::PactedStale,
                ),
            ]),
        ]),
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

/// A granted entry for `module`, documented the way a pact documents it.
fn entry(module: &str) -> PactEntry {
    PactEntry::new(".", module, format!("{module}/WARLOCK.md"))
        .expect("a relative module path is inside the root")
        .with_grant(HASH, AT)
}

/// The manifest the loop holds: `crates/engine` scoped, `crates/tui` not.
///
/// Named for what it holds rather than for its type, so that a test can
/// have a `manifest` of its own to edit and still ask what the fixture said.
fn pacts() -> Manifest {
    Manifest::with_entries([
        entry("crates/engine").with_scope("data-plane"),
        entry("crates/tui"),
    ])
}

/// The scope stored on `module`, or `None` for an entry with none.
fn scope_on<'manifest>(manifest: &'manifest Manifest, module: &str) -> Option<&'manifest str> {
    manifest
        .entry(module)
        .expect("the fixture manifest holds this module")
        .scope()
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
const CLOSED: &str =
    "crates/engine is scoped `data-plane` — hold that sigil to work here, with `warlock config`";

#[test]
fn s_is_refused_on_a_directory_whose_scope_this_machine_does_not_hold() {
    let repo = a_repo();
    let mut app = app_on(repo.path(), ENGINE_ROW);
    let before = app.clone();

    let prompt = super::scope_press(
        &mut app,
        &pacts(),
        repo.path(),
        &Sigils::held(["web"]),
        false,
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
    // nothing else moved — no colour, no selection, no account started.
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

        let prompt = super::scope_press(&mut app, &pacts(), repo.path(), &sigils, false);

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
        // Nobody has run `warlock config` here. A sigil is what opens a
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

        let prompt = super::scope_press(&mut app, &pacts(), repo.path(), &sigils, false);

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

    // The permissive default lives on the directory and only there, which is
    // what keeps a repository that has never scoped anything unaffected by
    // boundaries existing at all.
    let prompt = super::scope_press(&mut app, &pacts(), repo.path(), &Sigils::Nothing, false);

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
        &pacts(),
        repo.path(),
        &Sigils::held(["data-plane"]),
        false,
    );

    assert_eq!(prompt, ScopePrompt::open("crates/tui", ""));
}

#[test]
fn an_inner_scope_replaces_the_outer_one_rather_than_adding_to_it() {
    let repo = a_repo();
    let manifest = Manifest::with_entries([
        entry("crates/engine").with_scope("data-plane"),
        entry("crates/tui"),
        entry(".").with_scope("platform"),
    ]);

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
            false
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
            false
        ),
        ScopePrompt::open("crates/tui", "")
    );
}

#[test]
fn a_run_in_flight_is_answered_before_the_boundary_is() {
    let repo = a_repo();
    let mut app = app_on(repo.path(), ENGINE_ROW);
    app.set_pact_in_flight("/repo/crates/engine", 1, 2);

    let prompt = super::scope_press(
        &mut app,
        &pacts(),
        repo.path(),
        &Sigils::held(["web"]),
        true,
    );

    // Both refusals apply; the in-flight one is the one on screen. It goes
    // on the progress line the reader is already watching, and the message
    // line the run has taken is left alone — a boundary sentence written
    // there would be the one sentence they could not see.
    assert_eq!(prompt, ScopePrompt::Closed);
    assert_eq!(
        app.message(),
        Some(LAST_KEY),
        "the boundary spoke over a run's own line"
    );
}

#[test]
fn s_on_a_pacted_row_opens_the_prompt_on_the_scope_that_directory_carries() {
    let repo = a_repo();
    let mut app = app_on(repo.path(), ENGINE_ROW);
    let before = app.clone();

    let prompt = scope_press(&mut app, &pacts(), repo.path(), false);

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

    let prompt = scope_press(&mut app, &pacts(), repo.path(), false);
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

    let prompt = scope_press(&mut app, &pacts(), repo.path(), false);

    assert_eq!(prompt, ScopePrompt::open("crates/tui", ""));
}

#[test]
fn a_pacted_directory_the_manifest_has_no_entry_for_opens_on_an_empty_field() {
    // The root of this fixture is pacted on screen and absent from the
    // manifest, which is what a hand-edited file in another window leaves.
    // The window still opens — the reader asked about the row in front of
    // them — and holds nothing, because nothing is what is stored.
    let repo = a_repo();
    let mut app = app_on(repo.path(), ROOT_ROW);

    let prompt = scope_press(&mut app, &pacts(), repo.path(), false);

    assert_eq!(prompt, ScopePrompt::open(".", ""));
}

#[test]
fn the_field_holds_what_the_manifest_says_and_never_anything_off_the_row() {
    // The same app and the same row against two manifests: what is in the
    // field moves with the manifest, which it could not do if it were being
    // read off the tree.
    let repo = a_repo();
    let mut app = app_on(repo.path(), ENGINE_ROW);
    let elsewhere = Manifest::with_entries([entry("crates/engine").with_scope("web")]);

    assert_eq!(
        scope_press(&mut app, &pacts(), repo.path(), false),
        ScopePrompt::open("crates/engine", "data-plane")
    );
    assert_eq!(
        scope_press(&mut app, &elsewhere, repo.path(), false),
        ScopePrompt::open("crates/engine", "web")
    );
}

#[test]
fn a_press_while_a_run_is_in_flight_opens_nothing_and_says_so_on_the_progress_line() {
    let repo = a_repo();
    let mut app = app_on(repo.path(), ENGINE_ROW);
    // The run the press is going to bounce off, as the event loop would
    // have told the app about it.
    app.set_pact_in_flight(repo.path().join("crates/tui"), 3, 12);
    let before = app.clone();

    let prompt = scope_press(&mut app, &pacts(), repo.path(), true);

    assert_eq!(prompt, ScopePrompt::Closed, "no window over a running pact");
    // Saying so is the whole of it: the same app with the flag set and
    // nothing else moved, exactly as a second `p` leaves it.
    let refused = {
        let mut refused = before.clone();
        refused.set_pact_refused();
        refused
    };
    assert_eq!(app, refused, "the press did more than say so");
    assert!(
        app.pact_line()
            .is_some_and(|line| line.ends_with("— already running")),
        "the refusal is worded onto the line the reader is watching: {:?}",
        app.pact_line()
    );
    assert_eq!(
        app.message(),
        Some(LAST_KEY),
        "the refusal did not go through the message"
    );
}

#[test]
fn leaning_on_the_scope_key_during_a_run_changes_nothing_further() {
    let repo = a_repo();
    let mut app = app_on(repo.path(), ENGINE_ROW);
    app.set_pact_in_flight(repo.path().join("crates/tui"), 3, 12);

    assert_eq!(
        scope_press(&mut app, &pacts(), repo.path(), true),
        ScopePrompt::Closed
    );
    let after_one = app.clone();

    assert_eq!(
        scope_press(&mut app, &pacts(), repo.path(), true),
        ScopePrompt::Closed
    );
    assert_eq!(app, after_one, "pressing again changed something");
}

#[test]
fn a_directory_with_no_manifest_spelling_opens_nothing_and_says_why() {
    // A tree rooted outside its own repository: nothing on disk, because
    // whether a path can be spelled relative to a root is arithmetic.
    let mut app = App::from_tree(&Tree::new(Node::new(
        "/outside/thing",
        "/outside/thing/WARLOCK.md",
        NodeState::PactedFresh,
    )));

    let prompt = scope_press(&mut app, &Manifest::new(), Path::new("/repo"), false);

    assert_eq!(prompt, ScopePrompt::Closed);
    assert!(
        app.message().is_some_and(|line| !line.is_empty()),
        "a window with nowhere to write said nothing"
    );
}

#[test]
fn enter_sets_the_scope_and_leaves_the_document_and_the_grant_alone() {
    let repo = a_repo();
    let mut app = app_on(repo.path(), ENGINE_ROW);
    let mut manifest = pacts();

    let prompt = scope_submit(
        &mut app,
        &mut manifest,
        repo.path(),
        &field("crates/engine", "billing"),
    );

    assert_eq!(prompt, ScopePrompt::Closed, "the window is answered");
    let written = saved(repo.path()).expect("the submit wrote the manifest");
    assert_eq!(
        written, manifest,
        "what is on disk is what this thread believes"
    );
    assert_eq!(scope_on(&written, "crates/engine"), Some("billing"));

    // The one field a person owns, and nothing else on the entry: the run's
    // fields are exactly as they were, and so is the entry beside it.
    let engine = written
        .entry("crates/engine")
        .expect("the entry is still there");
    assert_eq!(engine.document(), "crates/engine/WARLOCK.md");
    assert_eq!(engine.granted_hash(), Some(HASH));
    assert_eq!(engine.granted_at(), Some(AT));
    assert_eq!(written.entry("crates/tui"), pacts().entry("crates/tui"));
    // And the order the file is written in, so the diff is the scope line.
    assert_eq!(
        written
            .entries()
            .iter()
            .map(PactEntry::module)
            .collect::<Vec<_>>(),
        ["crates/engine", "crates/tui"]
    );
}

// `third-party` is named by no entry in the fixture and `data-plane` loses its
// only entry to the clear below: a write that pruned the records to what the
// entries spell would drop both, and a record is the one thing here that is
// allowed to outlive the pacts that named it.
fn records() -> Vec<ScopeRecord> {
    vec![
        ScopeRecord::new("data-plane", "Data Plane", "In Review", "area/data-plane"),
        ScopeRecord::new("third-party", "Vendor", "Triage", "area/vendor"),
    ]
}

#[test]
fn a_set_and_a_clear_both_leave_the_records_in_the_file() {
    let repo = a_repo();
    let mut app = app_on(repo.path(), ENGINE_ROW);
    let mut manifest = pacts().with_scopes(records());

    for text in ["billing", ""] {
        let prompt = scope_submit(
            &mut app,
            &mut manifest,
            repo.path(),
            &field("crates/engine", text),
        );

        assert_eq!(prompt, ScopePrompt::Closed);
        let written = saved(repo.path()).expect("the submit wrote the manifest");
        assert_eq!(
            written.scopes(),
            records(),
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
        "the clear really did take the last entry naming `data-plane`",
    );
}

#[test]
fn enter_on_an_empty_field_clears_the_scope_and_keeps_the_grant() {
    let repo = a_repo();
    let mut app = app_on(repo.path(), ENGINE_ROW);
    let mut manifest = pacts();

    let prompt = scope_submit(
        &mut app,
        &mut manifest,
        repo.path(),
        &field("crates/engine", ""),
    );

    assert_eq!(prompt, ScopePrompt::Closed);
    let written = saved(repo.path()).expect("the submit wrote the manifest");
    assert_eq!(written, manifest);
    assert_eq!(
        scope_on(&written, "crates/engine"),
        None,
        "an empty field clears the scope rather than storing one"
    );
    let engine = written
        .entry("crates/engine")
        .expect("the entry is still there");
    assert_eq!(engine.granted_hash(), Some(HASH));
    assert_eq!(engine.granted_at(), Some(AT));
}

#[test]
fn a_set_and_a_clear_leave_the_run_state_and_the_message_line_untouched() {
    // The whole of what a scope write is *not*: no worker thread to be in
    // flight, no progress line, no account, no success message, and not
    // even the last keystroke's sentence spent. The app is compared whole,
    // which covers every one of those at once.
    let repo = a_repo();
    let mut app = app_on(repo.path(), ENGINE_ROW);
    let mut manifest = pacts();
    let before = app.clone();

    for text in ["billing", ""] {
        let prompt = scope_submit(
            &mut app,
            &mut manifest,
            repo.path(),
            &field("crates/engine", text),
        );

        assert_eq!(prompt, ScopePrompt::Closed);
        assert!(!app.is_pacting(), "writing {text:?} started a run");
        assert_eq!(app.pact_line(), None, "and put a progress line up");
        assert!(app.panel().account().is_none(), "and opened an account");
        assert_eq!(app.message(), Some(LAST_KEY), "and took the message line");
        assert_eq!(app, before, "writing {text:?} moved the view");
    }
}

#[test]
fn input_the_engine_refuses_keeps_the_prompt_up_and_writes_nothing() {
    let repo = a_repo();
    let mut app = app_on(repo.path(), ENGINE_ROW);
    let mut manifest = pacts();
    let before = app.clone();
    let typed = field("crates/engine", "Control Plane");

    let prompt = scope_submit(&mut app, &mut manifest, repo.path(), &typed);

    // The text and the cursor exactly where they were, one character away
    // from being fixed, with the broken rule under the field.
    assert_eq!(
        prompt,
        ScopePrompt::Open(typed.refused(refusal("control plane"))),
    );
    assert_eq!(saved(repo.path()), None, "a refusal wrote to disk");
    assert_eq!(manifest, pacts(), "a refusal edited the manifest");
    assert_eq!(app, before, "a refusal moved the view");
}

#[test]
fn a_list_typed_into_the_field_is_refused_and_never_stored_as_two_scopes() {
    let repo = a_repo();
    let mut app = app_on(repo.path(), ENGINE_ROW);
    let mut manifest = pacts();
    let typed = field("crates/engine", "control-plane, data-plane");

    let prompt = scope_submit(&mut app, &mut manifest, repo.path(), &typed);

    // The character rule, and it is the engine's own sentence: a directory
    // has one scope, so the comma is what is refused rather than a
    // separator warlock would have split on.
    assert_eq!(
        prompt,
        ScopePrompt::Open(typed.refused(refusal("control-plane, data-plane"))),
    );
    assert_eq!(saved(repo.path()), None);
    assert_eq!(
        manifest.entries().len(),
        2,
        "the list became a second entry"
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
    let mut manifest = pacts();

    let prompt = scope_submit(
        &mut app,
        &mut manifest,
        repo.path(),
        &field("crates/tui", "Data-Plane"),
    );

    assert_eq!(
        prompt,
        ScopePrompt::Closed,
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
    let mut manifest = pacts();

    let prompt = scope_submit(
        &mut app,
        &mut manifest,
        repo.path(),
        &field("crates/engine", "billing"),
    );

    // The reason goes on the footer and the window comes down off it; the
    // manifest this thread holds still says what is on disk, which is that
    // the scope was not changed.
    assert_eq!(prompt, ScopePrompt::Closed);
    assert!(
        app.message().is_some_and(|line| !line.is_empty()),
        "a save that failed said nothing"
    );
    assert_ne!(app.message(), Some(LAST_KEY), "and said it here");
    assert_eq!(manifest, pacts(), "the failed write was believed");
    assert_eq!(
        fs::read_to_string(repo.path().join(".warlock"))
            .expect("the file in the way is still readable"),
        "not a directory\n"
    );
}

#[test]
fn a_directory_the_manifest_has_no_entry_for_writes_nothing_and_says_so() {
    // A pact hand-edited out of the file in another window between the
    // press and the Enter: there is no entry to write a scope on, and a
    // window that closed silently would be indistinguishable from success.
    let repo = a_repo();
    let mut app = app_on(repo.path(), ROOT_ROW);
    let mut manifest = pacts();

    let prompt = scope_submit(&mut app, &mut manifest, repo.path(), &field(".", "billing"));

    assert_eq!(prompt, ScopePrompt::Closed);
    assert_eq!(saved(repo.path()), None, "something was written anyway");
    assert_eq!(manifest, pacts());
    let message = app.message().expect("the refusal said nothing");
    assert!(message.contains('.'), "{message}");
    assert!(message.contains("`p`"), "{message}");
}

#[test]
fn the_write_goes_to_the_manifest_under_the_repository_root() {
    // Where the file lands, said once: `.warlock/pacts.toml` under the root
    // handed in, and nowhere else. The engine owns the path, and this is
    // the assertion that the root travelling through here is the one the
    // loop resolved.
    let repo = a_repo();
    let mut app = app_on(repo.path(), ENGINE_ROW);
    let mut manifest = pacts();

    scope_submit(
        &mut app,
        &mut manifest,
        repo.path(),
        &field("crates/engine", "billing"),
    );

    let written = repo.path().join(".warlock").join("pacts.toml");
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
    assert_eq!(beside, ["pacts.toml"]);
}

#[test]
fn typing_and_abandoning_move_the_prompt_and_nothing_else() {
    let repo = a_repo();
    let mut app = app_on(repo.path(), ENGINE_ROW);
    let mut manifest = pacts();
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
        ScopePrompt::Open(typed),
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
        ScopePrompt::Closed,
    );
    assert_eq!(saved(repo.path()), None, "typing wrote to disk");
    assert_eq!(manifest, pacts(), "typing edited the manifest");
    assert_eq!(app, before, "typing moved the view");
}

#[test]
fn a_submit_with_no_window_up_writes_nothing() {
    // The road `press_for` cannot take — it only consults `edit_for` while a
    // window is up — answered rather than left to be discovered.
    let repo = a_repo();
    let mut app = app_on(repo.path(), ENGINE_ROW);
    let mut manifest = pacts();

    let prompt = scope_edit(
        &mut app,
        &mut manifest,
        repo.path(),
        &ScopePrompt::Closed,
        Edited::Submit,
    );

    assert_eq!(prompt, ScopePrompt::Closed);
    assert_eq!(saved(repo.path()), None);
    assert_eq!(manifest, pacts());
}

#[test]
fn the_whole_path_from_the_press_to_the_saved_manifest_is_one_key_at_a_time() {
    // The event loop's own arms, in the order a reader presses them: `s`,
    // four characters, a Backspace over the last of them, Enter. Nothing
    // here is a terminal, a network or a `claude` — the whole path is these
    // three functions over a directory of this test's own.
    let repo = a_repo();
    let mut app = app_on(repo.path(), TUI_ROW);
    let mut manifest = pacts();

    let mut prompt = scope_press(&mut app, &manifest, repo.path(), false);
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
        prompt = scope_edit(&mut app, &mut manifest, repo.path(), &prompt, edited);
    }

    assert_eq!(prompt, ScopePrompt::Closed, "Enter left the window up");
    assert_eq!(scope_on(&manifest, "crates/tui"), Some("web"));
    assert_eq!(
        scope_on(&saved(repo.path()).expect("the keys wrote"), "crates/tui"),
        Some("web"),
    );
    // And the run state and the message line came through the whole of it
    // untouched: no key of those six was a run.
    assert!(!app.is_pacting());
    assert_eq!(app.pact_line(), None);
    assert_eq!(app.message(), Some(LAST_KEY));
}

#[test]
fn a_refusal_typed_out_can_be_fixed_and_written_without_reopening_the_window() {
    // Enter on something the engine turns down, then a Backspace and a
    // character over the very text that was refused: the window stayed up
    // holding it, which is the whole point of a refusal not closing.
    let repo = a_repo();
    let mut app = app_on(repo.path(), TUI_ROW);
    let mut manifest = pacts();
    let mut prompt = ScopePrompt::open("crates/tui", "web!");

    for code in [KeyCode::Enter, KeyCode::Backspace, KeyCode::Enter] {
        let edited = {
            let field = prompt.field().expect("the window is still up");
            edit_for(press(code), field)
        };
        prompt = scope_edit(&mut app, &mut manifest, repo.path(), &prompt, edited);
        if code == KeyCode::Backspace {
            assert_eq!(saved(repo.path()), None, "the refused text was written");
        }
    }

    assert_eq!(prompt, ScopePrompt::Closed);
    assert_eq!(scope_on(&manifest, "crates/tui"), Some("web"));
}

/// Every `\n\n`-separated block of the saved file that mentions none of
/// `written`, as the bytes on disk spell them.
///
/// The assertion below is over the file rather than over the `Manifest`,
/// because the defect this guards against — records dropped by a rebuild —
/// is invisible in a value compared against the value that dropped them.
///
/// The file's own final newline is taken off first: it belongs to whichever
/// block happens to be last, and a block gaining one because something was
/// appended after it is not a change to that block.
fn blocks_apart_from(root: &Path, written: &[&str]) -> Vec<String> {
    fs::read_to_string(root.join(".warlock").join("pacts.toml"))
        .expect("the manifest was saved")
        .trim_end_matches('\n')
        .split("\n\n")
        .filter(|block| !written.iter().any(|needle| block.contains(needle)))
        .map(str::to_owned)
        .collect()
}

/// What the write below creates and changes: the pact row it scopes, and the
/// record it adds.
const TOUCHED: [&str; 2] = ["crates/tui", "billing"];

/// A repository with more in it than the write touches: three `[[pact]]`
/// rows and three `[[scope]]` records, only one of each of which the write
/// below is allowed to reach.
///
/// Bigger than [`pacts`] on purpose. A rebuild that dropped everything it
/// did not write would pass over a file holding one untouched block, and the
/// defect this fixture exists for dropped a whole section at once.
fn recorded() -> Manifest {
    Manifest::with_entries([
        entry("crates/engine").with_scope("data-plane"),
        entry("crates/tui"),
        entry("docs").with_scope("third-party"),
    ])
    .with_scopes(records().into_iter().chain([ScopeRecord::new(
        "platform",
        "Platform",
        "Backlog",
        "area/platform",
    )]))
}

#[test]
fn recording_a_scope_leaves_every_row_and_record_it_did_not_touch_byte_identical() {
    let repo = a_repo();
    let manifest = recorded();
    manifest.save(repo.path()).expect("the fixture was written");
    let before = blocks_apart_from(repo.path(), &TOUCHED);

    let next = with_scope_recorded(
        &manifest,
        "crates/tui",
        "billing",
        "Billing",
        "In Review",
        "area/billing",
    )
    .expect("no record in the fixture is named `billing`");
    next.save(repo.path()).expect("the write was saved");

    let after = blocks_apart_from(repo.path(), &TOUCHED);
    assert_eq!(
        after, before,
        "the write reordered, reformatted or dropped something it did not set out to change",
    );
    // The version header, two pact rows and three records, so the comparison
    // above is over blocks that are really there rather than over two empty
    // lists agreeing.
    assert_eq!(after.len(), 6, "{after:#?}");
    // And the records themselves, read back rather than spelled: the three that
    // were there come first and unedited, with the new one after them.
    let written = saved(repo.path()).expect("the write was saved");
    assert_eq!(written.scopes()[..3], recorded().scopes()[..]);
    assert_eq!(written.scopes().len(), 4);
    assert_eq!(
        written
            .entries()
            .iter()
            .map(PactEntry::module)
            .collect::<Vec<_>>(),
        ["crates/engine", "crates/tui", "docs"],
        "the write reordered the pact rows",
    );
}

#[test]
fn the_new_record_is_stored_as_passed_and_the_pact_row_keeps_its_document_and_grant() {
    let repo = a_repo();
    let manifest = recorded();

    // Spaced and capitalised on purpose: a team, a review state and a label
    // belong to somebody's tracker, and anything trimmed or folded on the way
    // through here is a name that no longer matches the one over there.
    let next = with_scope_recorded(
        &manifest,
        "crates/tui",
        "billing",
        " Billing Squad ",
        "In Review",
        "Area/Billing",
    )
    .expect("no record in the fixture is named `billing`");
    next.save(repo.path()).expect("the write was saved");

    let written = saved(repo.path()).expect("the write was saved");
    let record = written.scopes().last().expect("the record was written");
    assert_eq!(record.name(), "billing");
    assert_eq!(record.team(), " Billing Squad ");
    assert_eq!(record.review_state(), "In Review");
    assert_eq!(record.label(), "Area/Billing");

    // The scope on the pact is the string the record is filed under, so
    // routing finds one from the other.
    assert_eq!(scope_on(&written, "crates/tui"), Some(record.name()));
    let tui = written
        .entry("crates/tui")
        .expect("the entry is still there");
    assert_eq!(tui.document(), "crates/tui/WARLOCK.md");
    assert_eq!(tui.granted_hash(), Some(HASH));
    assert_eq!(tui.granted_at(), Some(AT));
    assert_eq!(
        written.entry("crates/engine"),
        pacts().entry("crates/engine")
    );
}

#[test]
fn a_name_already_recorded_is_refused_rather_than_overwritten_or_merged() {
    let repo = a_repo();
    let manifest = recorded();

    assert!(records_scope(&manifest, "data-plane"));
    assert!(records_scope(&manifest, "third-party"));
    assert!(!records_scope(&manifest, "billing"));
    // Compared as `route_facts` compares it: a fold here would answer "already
    // recorded" for a name the router treats as another one.
    assert!(!records_scope(&manifest, "Data-Plane"));

    assert_eq!(
        with_scope_recorded(
            &manifest,
            "crates/tui",
            "data-plane",
            "Someone Else",
            "Done",
            "area/other",
        ),
        None,
        "a record already in the file was rewritten",
    );
    // Nothing anywhere: no file to hold a merged or deleted record, and the
    // manifest handed in is still every record and row it arrived with.
    assert_eq!(saved(repo.path()), None, "a refusal wrote to disk");
    assert_eq!(manifest, recorded(), "a refusal edited the manifest");
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

#[test]
fn the_lookup_and_the_router_agree_about_which_names_are_recorded() {
    // One manifest and three scopes over it: `data-plane` is recorded,
    // `billing` is recorded by nothing, and `third-party` is recorded only
    // under a capitalised spelling — a different name to the router, which
    // compares byte for byte and folds nothing.
    let manifest = Manifest::with_entries([
        entry("crates/engine").with_scope("data-plane"),
        entry("crates/tui").with_scope("billing"),
        entry("docs").with_scope("third-party"),
    ])
    .with_scopes([
        ScopeRecord::new("data-plane", "Data Plane", "In Review", "area/data-plane"),
        ScopeRecord::new("Third-Party", "Vendor", "Triage", "area/vendor"),
    ]);

    for (module, scope, has_a_record) in [
        ("crates/engine", "data-plane", true),
        ("crates/tui", "billing", false),
        ("docs", "third-party", false),
    ] {
        // Both answers, from the one fixture: a lookup that disagreed with the
        // router would have `warlock scope add` either refuse a name nothing
        // routes through, or write a second record beside a name it does.
        assert_eq!(
            records_scope(&manifest, scope),
            has_a_record,
            "the lookup disagrees about `{scope}`",
        );
        assert_eq!(
            routes_to_a_record(&manifest, module),
            has_a_record,
            "the router disagrees about `{scope}`, which covers {module}",
        );
    }
}
