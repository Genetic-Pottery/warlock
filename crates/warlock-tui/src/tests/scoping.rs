use std::path::{Path, PathBuf};
use std::{fs, io};

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use tempfile::TempDir;
use warlock_engine::{
    Manifest, Node, NodeState, PactEntry, ScopeRecord, Tree, manifest, route_facts, validate_scope,
};
use warlock_tui::{
    App, Edited, RecordAsk, RecordFields, RecordPrompt, Recorded, ScopeField, ScopePrompt, Sigils,
    edit_for, record_edit_for,
};

use super::{Asking, record_edit, record_submit};

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

// The scope window as it stands after a submit, for the tests that are about
// the write. A name the manifest has no record for moves the question to the
// other window instead, and that is a failure here rather than something to
// assert around further down: a test that meant to ask it says so by calling
// `super::scope_submit` and matching on `Asking::Record`.
fn scope_submit(
    app: &mut App,
    manifest: &mut Manifest,
    repo_root: &Path,
    field: &ScopeField,
) -> ScopePrompt {
    asked(super::scope_submit(app, manifest, repo_root, field))
}

fn scope_edit(
    app: &mut App,
    manifest: &mut Manifest,
    repo_root: &Path,
    prompt: &ScopePrompt,
    edited: Edited,
) -> ScopePrompt {
    asked(super::scope_edit(app, manifest, repo_root, prompt, edited))
}

fn asked(asking: Asking) -> ScopePrompt {
    match asking {
        Asking::Scope(prompt) => prompt,
        Asking::Record(fields) => {
            panic!("`{}` asked for a record rather than writing", fields.name())
        }
    }
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
///
/// Every name the tests below type is recorded here, because a submit of a
/// name with no `[[scope]]` record asks for one instead of writing: these are
/// the tests about the write, and the ones about the second question type a
/// name this list does not hold.
fn pacts() -> Manifest {
    Manifest::with_entries([
        entry("crates/engine").with_scope("data-plane"),
        entry("crates/tui"),
    ])
    .with_scopes(RECORDED.map(|name| ScopeRecord::new(name, "Platform", "In Review", name)))
}

/// The scope names [`pacts`] holds a record for, and so the names a submit
/// writes rather than asking about. Anything else is new.
const RECORDED: [&str; 3] = ["data-plane", "billing", "web"];

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
// allowed to outlive the pacts that named it. `billing` is here because the
// set below writes it and a name with no record asks a second question instead
// — these three are the whole of the file's records for that test, and the
// assertion is that they come back unmoved.
fn records() -> Vec<ScopeRecord> {
    vec![
        ScopeRecord::new("data-plane", "Data Plane", "In Review", "area/data-plane"),
        ScopeRecord::new("third-party", "Vendor", "Triage", "area/vendor"),
        ScopeRecord::new("billing", "Billing", "In Review", "area/billing"),
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

/// A record window over `module` for `name`, filled one keystroke at a time
/// through [`record_edit_for`] — which is the only way to fill one, because
/// the three fields belong to the prompt and nothing else sets them. The
/// values are typed in [`RecordAsk::ORDER`], a Tab between each, so what is
/// asserted about afterwards went in the way a person's would.
fn fields(module: &str, name: &str, values: [&str; 3]) -> RecordFields {
    filled(RecordFields::new(module, name), values)
}

/// [`fields`] over a window the submit handed back, for the tests that reach
/// the record question by pressing `s` rather than by building one.
fn filled(mut fields: RecordFields, values: [&str; 3]) -> RecordFields {
    for value in values {
        for character in value.chars() {
            fields = opened(record_edit_for(press(KeyCode::Char(character)), &fields));
        }
        fields = opened(record_edit_for(press(KeyCode::Tab), &fields));
    }
    fields
}

fn opened(recorded: Recorded) -> RecordFields {
    match recorded {
        Recorded::Open(fields) => fields,
        other => panic!("typing into the record window answered {other:?}"),
    }
}

#[test]
fn a_scope_name_the_manifest_has_no_record_for_asks_for_one_and_writes_nothing() {
    // The whole of the second question: a name nothing routes, so the window
    // moves rather than the file. Nothing has been decided yet — an Esc from
    // here leaves a repository with no manifest in it at all.
    let repo = a_repo();
    let mut app = app_on(repo.path(), TUI_ROW);
    let mut manifest = pacts();
    let before = app.clone();

    let asking = super::scope_submit(
        &mut app,
        &mut manifest,
        repo.path(),
        &field("crates/tui", "Ledger"),
    );

    assert_eq!(
        asking,
        Asking::Record(RecordFields::new("crates/tui", "ledger")),
        "the record is asked for under the folded name, on three empty fields",
    );
    assert_eq!(saved(repo.path()), None, "asking wrote to disk");
    assert_eq!(manifest, pacts(), "asking edited the manifest");
    assert_eq!(app, before, "asking moved the view");
}

#[test]
fn a_scope_name_that_already_has_a_record_is_written_with_no_second_question() {
    // A record is shared by every directory carrying that scope, so a second
    // question here would be an offer to overwrite somebody else's route.
    let repo = a_repo();
    let mut app = app_on(repo.path(), TUI_ROW);
    let mut manifest = pacts();

    let prompt = scope_submit(
        &mut app,
        &mut manifest,
        repo.path(),
        &field("crates/tui", "billing"),
    );

    assert_eq!(prompt, ScopePrompt::Closed, "the window is answered");
    let written = saved(repo.path()).expect("the submit wrote the manifest");
    assert_eq!(scope_on(&written, "crates/tui"), Some("billing"));
    assert_eq!(
        written.scopes(),
        pacts().scopes(),
        "the records moved for a name that already had one",
    );
}

#[test]
fn a_blank_value_in_any_of_the_three_fields_is_refused_and_writes_nothing() {
    // Blank is the only judgement this window makes, and whitespace counts:
    // a field holding a space reads as empty on screen, and a record routing
    // work to ` ` is the one this question exists to prevent.
    for blank in RecordAsk::ORDER {
        for typed_blank in ["", "   "] {
            let repo = a_repo();
            let mut app = app_on(repo.path(), TUI_ROW);
            let mut manifest = pacts();
            let values =
                RecordAsk::ORDER.map(|ask| if ask == blank { typed_blank } else { "Billing" });
            let typed = fields("crates/tui", "ledger", values);

            let prompt = record_submit(&mut app, &mut manifest, repo.path(), &typed);

            let RecordPrompt::Open(refused) = prompt else {
                panic!("a blank {} closed the window", blank.heading())
            };
            assert_eq!(
                refused.asking(),
                blank,
                "the question is asked of the field that was refused",
            );
            let rule = refused
                .field(blank)
                .rule()
                .expect("the refusal said nothing under the field");
            assert!(rule.contains(blank.heading()), "{rule}");
            // Every field byte for byte as it was, cursor included: a refusal
            // leaves whoever typed it one character away from fixing it.
            for ask in RecordAsk::ORDER {
                assert_eq!(refused.field(ask).text(), typed.field(ask).text());
                assert_eq!(refused.field(ask).cursor(), typed.field(ask).cursor());
            }
            assert_eq!(saved(repo.path()), None, "a refusal wrote to disk");
            assert_eq!(manifest, pacts(), "a refusal edited the manifest");
        }
    }
}

#[test]
fn an_accepted_record_saves_the_scope_and_the_record_in_one_write() {
    // One save, both halves: there is no moment on disk where the pact names a
    // scope nothing routes. The team is typed with spaces around it and a
    // capital in it, and comes back exactly that way — this window folds,
    // trims and judges nothing beyond blankness.
    let repo = a_repo();
    let mut app = app_on(repo.path(), TUI_ROW);
    let mut manifest = pacts();
    let typed = fields(
        "crates/tui",
        "ledger",
        ["  Billing Platform ", "In Review", "area/ledger"],
    );

    let prompt = record_submit(&mut app, &mut manifest, repo.path(), &typed);

    assert_eq!(prompt, RecordPrompt::Closed, "the window is answered");
    let written = saved(repo.path()).expect("the submit wrote the manifest");
    assert_eq!(
        written, manifest,
        "what is on disk is what this thread believes"
    );
    assert_eq!(
        scope_on(&written, "crates/tui"),
        Some("ledger"),
        "the pact carries the name the record was written under",
    );
    assert_eq!(
        written.scopes().last(),
        Some(&ScopeRecord::new(
            "ledger",
            "  Billing Platform ",
            "In Review",
            "area/ledger"
        )),
        "the three values are stored as typed",
    );
    // And everything the write did not set out to change: the records that
    // were already there, in order, and the other entry whole.
    assert_eq!(&written.scopes()[..RECORDED.len()], pacts().scopes());
    assert_eq!(
        written.entry("crates/engine"),
        pacts().entry("crates/engine")
    );
    let tui = written
        .entry("crates/tui")
        .expect("the entry is still there");
    assert_eq!(tui.document(), "crates/tui/WARLOCK.md");
    assert_eq!(tui.granted_hash(), Some(HASH));
    assert_eq!(tui.granted_at(), Some(AT));
}

/// The bytes of `.warlock/pacts.toml` under `root`, or `None` where the file
/// is not there.
///
/// Read as text and never parsed, because "byte-identical to what was read" is
/// a claim about the file: two manifests that compare equal can be written two
/// ways, and a comparison of parsed values would pass through a rewrite that
/// reordered or reworded every line in the repository's history.
fn on_disk(root: &Path) -> Option<String> {
    match fs::read_to_string(root.join(".warlock").join("pacts.toml")) {
        Ok(text) => Some(text),
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => panic!("the saved manifest could not be read: {error}"),
    }
}

/// `text` typed into the scope window one character at a time and Enter
/// pressed, through the event loop's own two functions — so the record window
/// these tests go on to answer is the one the keys actually reach, rather than
/// a [`RecordFields`] built beside the road.
fn submitted(
    app: &mut App,
    manifest: &mut Manifest,
    repo_root: &Path,
    mut prompt: ScopePrompt,
    text: &str,
) -> Asking {
    for code in text.chars().map(KeyCode::Char).chain([KeyCode::Enter]) {
        let edited = {
            let field = prompt.field().expect("the scope window is still up");
            edit_for(press(code), field)
        };
        match super::scope_edit(app, manifest, repo_root, &prompt, edited) {
            Asking::Scope(open) => prompt = open,
            // Only the Enter can ask, and when it does that is the answer this
            // helper was called for.
            record @ Asking::Record(_) => return record,
        }
    }
    Asking::Scope(prompt)
}

fn asking_for(asking: Asking) -> RecordFields {
    match asking {
        Asking::Record(fields) => fields,
        Asking::Scope(prompt) => panic!("the name was written rather than asked about: {prompt:?}"),
    }
}

#[test]
fn esc_on_the_record_window_leaves_the_file_byte_for_byte_as_it_was_read() {
    // The second window is the one place in this key's path where a person has
    // already typed a name that nothing on disk routes. Esc there has to be
    // worth nothing at all: not the scope without the record, not a record
    // under a scope nobody carries, and not a rewritten file either.
    let repo = a_repo();
    let mut manifest = pacts().with_scopes(records());
    manifest
        .save(repo.path())
        .expect("the fixture manifest saves");
    let file = on_disk(repo.path()).expect("the fixture manifest is on disk");
    let held = manifest.clone();
    let mut app = app_on(repo.path(), TUI_ROW);
    let before = app.clone();

    let prompt = scope_press(&mut app, &manifest, repo.path(), false);
    let asking = submitted(&mut app, &mut manifest, repo.path(), prompt, "ledger");
    let typed = filled(asking_for(asking), ["Ledger", "In Review", "area/ledger"]);
    assert_eq!(
        record_edit_for(press(KeyCode::Esc), &typed),
        Recorded::Close,
        "Esc did something other than close the window",
    );
    let prompt = record_edit(
        &mut app,
        &mut manifest,
        repo.path(),
        &RecordPrompt::Open(typed),
        Recorded::Close,
    );

    assert_eq!(prompt, RecordPrompt::Closed, "Esc left the window up");
    assert_eq!(
        on_disk(repo.path()).as_deref(),
        Some(file.as_str()),
        "Esc rewrote the file",
    );
    assert_eq!(manifest, held, "Esc edited the manifest this thread holds");
    assert_eq!(
        scope_on(&manifest, "crates/tui"),
        None,
        "Esc left the name behind on the pact",
    );
    assert_eq!(app, before, "Esc moved the view");
}

#[test]
fn esc_on_the_record_window_leaves_a_repository_that_had_no_manifest_without_one() {
    // The same Esc where there is no file to be identical to: the fixture
    // manifest lives in this thread alone, as it does in every test above, so
    // anything at all under `.warlock/` afterwards was written by this press.
    let repo = a_repo();
    let mut app = app_on(repo.path(), TUI_ROW);
    let mut manifest = pacts();

    let prompt = scope_press(&mut app, &manifest, repo.path(), false);
    let asking = submitted(&mut app, &mut manifest, repo.path(), prompt, "ledger");
    let typed = filled(asking_for(asking), ["Ledger", "In Review", "area/ledger"]);
    let prompt = record_edit(
        &mut app,
        &mut manifest,
        repo.path(),
        &RecordPrompt::Open(typed),
        Recorded::Close,
    );

    assert_eq!(prompt, RecordPrompt::Closed);
    assert_eq!(on_disk(repo.path()), None, "Esc wrote a manifest");
    assert!(
        !repo.path().join(".warlock").exists(),
        "Esc left the manifest directory behind",
    );
    assert_eq!(
        manifest,
        pacts(),
        "Esc edited the manifest this thread holds"
    );
}

/// The lines of `after` that were not in `before`, or `None` when a line of
/// `before` is missing from `after` or has moved.
///
/// A line-wise subsequence is what "every other record and row is
/// byte-identical" comes to for a file this write only adds to: a line that
/// was reworded, reordered, merged or dropped cannot be matched in order, and
/// whatever is left over is exactly what appeared.
fn added<'text>(before: &str, after: &'text str) -> Option<Vec<&'text str>> {
    let mut before = before.lines().peekable();
    let mut added = Vec::new();
    for line in after.lines() {
        if before.peek() == Some(&line) {
            before.next();
        } else {
            added.push(line);
        }
    }
    before.next().is_none().then_some(added)
}

#[test]
fn a_record_write_leaves_every_other_record_and_row_byte_for_byte() {
    // Read off the disk on both sides rather than compared as manifests: the
    // promise is about the diff a person is going to commit, and the three
    // records and two pacts that this write has no business touching are in
    // the file with it.
    let repo = a_repo();
    let mut app = app_on(repo.path(), TUI_ROW);
    let mut manifest = pacts().with_scopes(records());
    manifest
        .save(repo.path())
        .expect("the fixture manifest saves");
    let before = on_disk(repo.path()).expect("the fixture manifest is on disk");

    let typed = fields(
        "crates/tui",
        "ledger",
        ["Ledger", "In Review", "area/ledger"],
    );
    let prompt = record_submit(&mut app, &mut manifest, repo.path(), &typed);

    assert_eq!(prompt, RecordPrompt::Closed, "the window is answered");
    let after = on_disk(repo.path()).expect("the submit wrote the manifest");
    let added = added(&before, &after).unwrap_or_else(|| {
        panic!("the write moved a line it did not write:\n{before}\n---\n{after}")
    });
    // And what did appear is the two halves of this one edit and nothing else:
    // the scope onto the pact that was asked about, and the record appended
    // after the three that were already there.
    assert_eq!(
        added,
        [
            "scope = \"ledger\"",
            "",
            "[[scope]]",
            "name = \"ledger\"",
            "team = \"Ledger\"",
            "review_state = \"In Review\"",
            "label = \"area/ledger\"",
        ],
    );
}

#[test]
fn the_saved_manifest_routes_the_scoped_directory_to_the_record_just_written() {
    // The fact behind `warlock check` printing a full route where before it
    // could only print a scope: the route is resolved out of the file rather
    // than out of the manifest this thread holds, because `check` is another
    // process and what it can say is what was saved.
    let repo = a_repo();
    let mut app = app_on(repo.path(), TUI_ROW);
    let mut manifest = pacts();
    let typed = fields(
        "crates/tui",
        "ledger",
        ["Ledger", "In Review", "area/ledger"],
    );

    record_submit(&mut app, &mut manifest, repo.path(), &typed);

    let written = saved(repo.path()).expect("the submit wrote the manifest");
    // `None` for the home: the key half of a route is a machine's business and
    // reading a real one from a test is how a developer's credentials get read.
    let facts = route_facts(repo.path().join("crates/tui"), repo.path(), &written, None)
        .expect("the directory is inside the repository");

    assert_eq!(facts.scope(), Some("ledger"));
    let record = facts
        .record()
        .expect("the scope written by the record window routes nowhere");
    assert_eq!(record.team(), "Ledger");
    assert_eq!(record.review_state(), "In Review");
    assert_eq!(record.label(), "area/ledger");
    // And the boundary covers what is under it, which is where `check` is
    // usually run from.
    let below = route_facts(
        repo.path().join("crates/tui/src/main.rs"),
        repo.path(),
        &written,
        None,
    )
    .expect("the file is inside the repository");
    assert_eq!(below.record(), facts.record());
}

#[test]
fn an_empty_name_clears_the_scope_and_opens_no_second_window() {
    // A clear has no name to route, so there is nothing to record: the second
    // question is asked about a scope being written and never about one going
    // away. `data-plane`'s record stays in the file for whatever else carries
    // it, which is the assertion under the clear.
    let repo = a_repo();
    let mut app = app_on(repo.path(), ENGINE_ROW);
    let mut manifest = pacts();

    let asking = super::scope_submit(
        &mut app,
        &mut manifest,
        repo.path(),
        &field("crates/engine", ""),
    );

    assert_eq!(asking, Asking::Scope(ScopePrompt::Closed));
    let written = saved(repo.path()).expect("the clear wrote the manifest");
    assert_eq!(scope_on(&written, "crates/engine"), None);
    assert_eq!(written.scopes(), pacts().scopes());
}

#[test]
fn a_name_that_folds_onto_a_recorded_one_is_written_with_no_second_question() {
    // The record is looked for under the string that goes onto the pact, which
    // is the folded one. A lookup of `BILLING` would find nothing and ask a
    // second question about a scope `billing` already routes, and answering it
    // would write a second record under a name the manifest refuses twice.
    let repo = a_repo();
    let mut app = app_on(repo.path(), TUI_ROW);
    let mut manifest = pacts();

    let asking = super::scope_submit(
        &mut app,
        &mut manifest,
        repo.path(),
        &field("crates/tui", "BILLING"),
    );

    assert_eq!(asking, Asking::Scope(ScopePrompt::Closed));
    let written = saved(repo.path()).expect("the submit wrote the manifest");
    assert_eq!(scope_on(&written, "crates/tui"), Some("billing"));
    assert_eq!(
        written.scopes(),
        pacts().scopes(),
        "a name that already had a record grew a second one",
    );
}
