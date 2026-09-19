use std::fs;
use std::path::{Path, PathBuf};

use warlock_engine::{
    Manifest, Node, NodeState, PactEntry, ScopeRecord, Tree, manifest_path, route_facts,
    save_sigils, validate_scope,
};
use warlock_tui::App;

use super::{Opened, scoped_line, unpacted_line, unscoped_line};
use crate::error::Error;
// The other door onto the un-pact rule, pressed here so that the two are
// held to one answer in one place. See `pressed_p`.
use crate::pacting::pressed_p;
// The sentence itself, asked of the one function that writes it rather than
// retyped: the footer and the shell refuse the same boundary in the same
// words, and a test holding a copy of those words is a test that would go on
// passing while the two doors drifted apart.
use crate::boundary::closed_scope_message;
use crate::session::{load_manifest, sigils_under};
use crate::status_for;

// A grant on every entry, so that "the scope write left the run's own fields
// alone" is an assertion about two values that are really there.
const HASH: &str = "d0f5a1";

const AT: &str = "2026-08-19T07:32:00Z";

// Every test here builds both its repository and its home out of one of
// these, so nothing goes near the developer's real home.
fn a_dir() -> tempfile::TempDir {
    tempfile::tempdir().expect("a temporary directory")
}

// Granted rather than bare, because a scope write promises to leave the run's
// own fields where it found them and a promise about a hash needs a hash to be
// about. An un-pact drops whole entries, so it neither knows nor cares.
fn entry(module: &str) -> PactEntry {
    PactEntry::new(".", module, format!("{module}/WARLOCK.md"))
        .expect("a relative module path is inside the root")
        .with_grant(HASH, AT)
}

fn a_manifest() -> Manifest {
    Manifest::with_entries([
        entry("crates").with_scope("platform"),
        entry("crates/engine").with_scope("data-plane"),
        entry("crates/engine/src"),
        entry("docs"),
    ])
}

// The documents are on disk rather than assumed, because "every `WARLOCK.md`
// stays where it was" is one of the things an un-pact promises and a promise
// about files needs files to be about.
fn a_repository() -> tempfile::TempDir {
    let repo = a_dir();
    a_manifest()
        .save(repo.path())
        .expect("a manifest that saves");
    for entry in a_manifest().entries() {
        let document = entry.document_path(repo.path());
        fs::create_dir_all(document.parent().expect("a document has a directory"))
            .expect("a module directory");
        fs::write(&document, "a document\n").expect("a document");
    }

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
// handed in instead. The three writes below reach their edit through this
// and only through this, so there is no way to the second half without the
// first.
fn open(repo_root: &Path, home: &Path, path: &str) -> Result<Opened, Error> {
    let manifest = load_manifest(repo_root).expect("a manifest that reads");
    Opened::new(
        repo_root.to_path_buf(),
        Some(home),
        manifest,
        repo_root.join(path),
    )
}

fn unpact(repo_root: &Path, home: &Path, path: &str) -> Result<String, Error> {
    open(repo_root, home, path)?.unpacted()
}

// The three values a name nothing records wants, spelled once: most of the
// writes below are about a boundary, a path or a fold, and only need the record
// refusal out of their way.
const TEAM: &str = "Billing";

const REVIEW_STATE: &str = "In Review";

const LABEL: &str = "area/billing";

// The road every test that does not care about the record values takes, so
// adding a fourth value later is one signature and not thirty call sites.
fn scope_add(repo_root: &Path, home: &Path, path: &str, scope: &str) -> Result<String, Error> {
    scope_add_with(
        repo_root,
        home,
        path,
        scope,
        Some(TEAM),
        Some(REVIEW_STATE),
        Some(LABEL),
    )
}

// What `warlock scope add <path> <scope>` with no flags is: the shape the
// existing-record road is written with, and the shape every missing-flag
// refusal starts from.
fn scope_add_bare(repo_root: &Path, home: &Path, path: &str, scope: &str) -> Result<String, Error> {
    scope_add_with(repo_root, home, path, scope, None, None, None)
}

fn scope_add_with(
    repo_root: &Path,
    home: &Path,
    path: &str,
    scope: &str,
    team: Option<&str>,
    review_state: Option<&str>,
    label: Option<&str>,
) -> Result<String, Error> {
    open(repo_root, home, path)?.scoped(scope, team, review_state, label)
}

fn scope_remove(repo_root: &Path, home: &Path, path: &str) -> Result<String, Error> {
    open(repo_root, home, path)?.unscoped()
}

// The three values as a command line carried them, and the flags the refusal
// they earn has to name: both record refusals below are a table of these, and
// what each case is really asserting is the second half against the first.
type Case = (
    Option<&'static str>,
    Option<&'static str>,
    Option<&'static str>,
    &'static [&'static str],
);

fn stored(repo_root: &Path, module: &str) -> PactEntry {
    load_manifest(repo_root)
        .expect("a manifest that reads")
        .entry(module)
        .expect("the manifest holds this module")
        .clone()
}

// Asked of the one judge rather than retyped here, so a test cannot go on
// agreeing with a wording warlock no longer uses.
fn refusal(text: &str) -> String {
    validate_scope(text)
        .expect_err("this text is not a scope")
        .to_string()
}

#[test]
fn an_open_boundary_drops_the_subtree_and_leaves_every_document_on_disk() {
    let repo = a_repository();
    let home = a_dir();
    holding(home.path(), repo.path(), &["platform", "data-plane"]);

    let said = unpact(repo.path(), home.path(), "crates").expect("an open boundary writes");

    // The subtree went, the sibling that merely shares a prefix of the name
    // did not, and the engine decided which was which.
    let modules: Vec<String> = load_manifest(repo.path())
        .expect("a manifest that reads")
        .entries()
        .iter()
        .map(|entry| entry.module().to_owned())
        .collect();
    assert_eq!(modules, ["docs"]);
    assert!(
        said.starts_with("unpacted crates — 3 entries dropped"),
        "{said}"
    );
    assert_eq!(status_for(&Ok(())), 0);

    // The promise the whole command is shaped around: warlock forgot the
    // pact, and the prose is still the repository's.
    for module in ["crates", "crates/engine", "crates/engine/src", "docs"] {
        let document = repo.path().join(module).join("WARLOCK.md");
        assert!(document.is_file(), "{} was removed", document.display());
    }
}

#[test]
fn a_closed_boundary_refuses_and_leaves_the_manifest_byte_identical() {
    let repo = a_repository();
    let home = a_dir();
    // The nearest scope wins, so a machine holding the outer boundary is
    // still outside the inner one.
    holding(home.path(), repo.path(), &["platform"]);
    let before = manifest_bytes(repo.path()).expect("a manifest on disk");

    let refused = unpact(repo.path(), home.path(), "crates/engine");

    let error = refused.expect_err("a scope this machine does not hold refuses");
    assert!(
        matches!(error, Error::ClosedScope { .. }),
        "the boundary was refused as something else: {error:?}"
    );
    // The footer's own sentence, named rather than copied, about this path
    // and this scope: the shell says what the keystroke says.
    assert_eq!(
        error.to_string(),
        closed_scope_message("crates/engine", "data-plane")
    );
    assert!(!error.to_string().contains('\n'), "`main` prints one line");
    // The refusal's own status: not the 1 warlock spends on something it
    // could not do, because nothing was spent and nothing here can be
    // retried into working.
    assert_eq!(status_for(&Err(error)), 3);
    assert_eq!(manifest_bytes(repo.path()).as_deref(), Some(&before[..]));
}

#[test]
fn a_machine_holding_nothing_is_refused_by_every_scope_it_meets() {
    // No config at all: the ordinary state of a machine nobody has run
    // `warlock config` on, and the one an agent in a fresh checkout is in.
    let repo = a_repository();
    let home = a_dir();
    let before = manifest_bytes(repo.path()).expect("a manifest on disk");

    for path in ["crates", "crates/engine", "crates/engine/src"] {
        let error = unpact(repo.path(), home.path(), path)
            .expect_err("holding nothing opens nothing that is scoped");
        assert!(
            matches!(error, Error::ClosedScope { .. }),
            "{path}: {error:?}"
        );
    }
    // And the unscoped directory beside them is open to that same machine:
    // the permissive default is on the directory and only there.
    unpact(repo.path(), home.path(), "docs").expect("nothing scopes `docs`");
    assert_ne!(manifest_bytes(repo.path()).as_deref(), Some(&before[..]));
}

#[test]
fn the_success_line_names_every_dropped_entry_that_carried_a_scope() {
    // `.` is the root and the root carries no scope, so the boundary over it
    // waves this through — and the boundaries *under* it wave it through
    // because this machine holds both of them. It then says out loud whose
    // they were.
    let repo = a_repository();
    let home = a_dir();
    holding(home.path(), repo.path(), &["platform", "data-plane"]);

    let said = unpact(repo.path(), home.path(), ".").expect("every scope below is held");

    assert_eq!(
        said,
        "unpacted . — 4 entries dropped, 2 scoped (crates: platform, \
             crates/engine: data-plane)"
    );
    assert!(
        load_manifest(repo.path())
            .expect("a manifest that reads")
            .entries()
            .is_empty()
    );
}

#[test]
fn a_boundary_below_the_path_refuses_the_unpact_and_names_every_scope_in_the_way() {
    // The blast radius, closed. `crates` opens to this machine and the root
    // is scoped by nobody, but both un-pacts reach a boundary this machine
    // is outside of — and an entry is the only home a scope has.
    let repo = a_repository();
    let home = a_dir();
    holding(home.path(), repo.path(), &["platform"]);
    let before = manifest_bytes(repo.path()).expect("a manifest on disk");

    let error = unpact(repo.path(), home.path(), "crates")
        .expect_err("`crates/engine` is scoped `data-plane` and this machine is not");
    assert!(
        matches!(error, Error::ClosedScopeBelow { .. }),
        "the descendant boundary was refused as something else: {error:?}"
    );
    assert_eq!(
        error.to_string(),
        "un-pacting crates would drop pacts scoped `data-plane` — hold that sigil with \
             `warlock config`, or un-pact the parts you hold"
    );
    assert!(!error.to_string().contains('\n'), "`main` prints one line");
    // A 1 and deliberately not the boundary's 3: this machine may work at
    // `crates`, and the second road out of the sentence — un-pact the parts
    // you hold — needs no sigil at all, so it is not the "you are outside,
    // go and ask" verdict 3 exists for. Argued on `status_for`.
    assert_eq!(status_for(&Err(error)), 1);

    // The root, whose own unscoped-ness bought the whole repository today:
    // every distinct scope in the way is named, deduplicated and in the
    // manifest's order, so obtaining one sigil does not reveal the next.
    let error = unpact(repo.path(), home.path(), ".")
        .expect_err("an unscoped root is not permission over the scopes below it");
    assert_eq!(
        error.to_string(),
        "un-pacting . would drop pacts scoped `data-plane` — hold that sigil with \
             `warlock config`, or un-pact the parts you hold"
    );

    // And nothing was written on the way to either refusal.
    assert_eq!(manifest_bytes(repo.path()).as_deref(), Some(&before[..]));

    // What is left is the road out the sentence offers: the parts this
    // machine does hold still un-pact, one subtree at a time.
    assert_eq!(
        unpact(repo.path(), home.path(), "docs").expect("nothing at or below `docs` is scoped"),
        "unpacted docs — 1 entry dropped"
    );
}

#[test]
fn a_machine_holding_nothing_is_told_about_every_boundary_at_once() {
    // Holding nothing — the ordinary state of a fresh checkout — the root
    // un-pact meets both scopes, and both are named in the manifest's own
    // order rather than one at a time.
    let repo = a_repository();
    let home = a_dir();

    let error = unpact(repo.path(), home.path(), ".")
        .expect_err("holding nothing opens nothing that is scoped");
    assert_eq!(
        error.to_string(),
        "un-pacting . would drop pacts scoped `platform`, `data-plane` — hold those sigils \
             with `warlock config`, or un-pact the parts you hold"
    );
    // The descendant refusal's 1 again, and holding nothing does not change
    // it: what decides the status is which question was refused, not how
    // much this machine holds. See `status_for`.
    assert_eq!(status_for(&Err(error)), 1);
}

#[test]
fn one_entry_is_counted_in_the_singular_and_an_unscoped_drop_says_no_more() {
    let repo = a_repository();
    let home = a_dir();

    assert_eq!(
        unpact(repo.path(), home.path(), "docs").expect("nothing scopes `docs`"),
        "unpacted docs — 1 entry dropped"
    );
}

#[test]
fn a_path_with_no_manifest_form_is_refused_with_nothing_written() {
    // Not a boundary question — coverage has nothing to say about a path
    // that is not in this repository — so it is the command's own refusal,
    // in the shape every other subcommand refuses one.
    let repo = a_repository();
    let home = a_dir();
    let before = manifest_bytes(repo.path()).expect("a manifest on disk");

    for outside in [PathBuf::from("/elsewhere"), repo.path().join("..")] {
        let manifest = load_manifest(repo.path()).expect("a manifest that reads");
        let refused = Opened::new(
            repo.path().to_path_buf(),
            Some(home.path()),
            manifest,
            outside.clone(),
        )
        .and_then(|opened| opened.unpacted());

        let error = refused.expect_err("a path outside the repository has no manifest form");
        assert!(
            matches!(error, Error::Unspellable { .. }),
            "{}: {error:?}",
            outside.display()
        );
        assert!(!error.to_string().contains('\n'), "`main` prints one line");
        assert_eq!(manifest_bytes(repo.path()).as_deref(), Some(&before[..]));
    }
}

#[test]
fn an_unpact_in_a_repository_that_never_pacted_anything_writes_the_empty_manifest() {
    // The decision recorded in the module docs, pinned here: it succeeds, it
    // drops nothing, and it saves — so the file that appears says exactly
    // what was already true.
    let repo = a_dir();
    let home = a_dir();
    assert_eq!(manifest_bytes(repo.path()), None);

    assert_eq!(
        unpact(repo.path(), home.path(), ".").expect("an empty manifest has no boundary"),
        "unpacted . — 0 entries dropped"
    );
    assert!(
        load_manifest(repo.path())
            .expect("a manifest that reads")
            .entries()
            .is_empty()
    );
    assert!(manifest_bytes(repo.path()).is_some(), "nothing was saved");
}

#[test]
fn a_scope_no_boundary_would_honour_is_still_named_when_it_is_dropped() {
    // `Data Plane!` is not a scope, so coverage ignores it and it closed
    // nothing — but somebody wrote it in the file, and a line that left it
    // out would be warlock deciding it did not count.
    let dropped = entry("crates/engine").with_scope("Data Plane!");

    assert_eq!(
        unpacted_line("crates", &[&dropped]),
        "unpacted crates — 1 entry dropped, 1 scoped (crates/engine: Data Plane!)"
    );
}

#[test]
fn an_open_boundary_writes_the_scope_and_moves_nothing_else_in_the_file() {
    let repo = a_repository();
    let home = a_dir();

    // `docs` carries no scope and nothing above it does, so it is open to a
    // machine that has never run `warlock config`.
    let said =
        scope_add(repo.path(), home.path(), "docs", "billing").expect("nothing scopes `docs`");

    assert_eq!(said, "docs is scoped `billing`");
    assert_eq!(status_for(&Ok(())), 0);
    let docs = stored(repo.path(), "docs");
    assert_eq!(docs.scope(), Some("billing"));
    // The one field a person owns, and nothing else on the entry.
    assert_eq!(docs.document(), "docs/WARLOCK.md");
    assert_eq!(docs.granted_hash(), Some(HASH));
    assert_eq!(docs.granted_at(), Some(AT));

    // Every other entry cloned untouched, in the order they were in, so the
    // diff against what was there is the one scope line.
    let after = load_manifest(repo.path()).expect("a manifest that reads");
    let before = a_manifest();
    assert_eq!(
        after
            .entries()
            .iter()
            .map(PactEntry::module)
            .collect::<Vec<_>>(),
        before
            .entries()
            .iter()
            .map(PactEntry::module)
            .collect::<Vec<_>>(),
    );
    for module in ["crates", "crates/engine", "crates/engine/src"] {
        assert_eq!(after.entry(module), before.entry(module), "{module}");
    }
}

#[test]
fn a_scope_that_replaces_another_says_whose_boundary_it_moved() {
    let repo = a_repository();
    let home = a_dir();
    holding(home.path(), repo.path(), &["data-plane"]);

    let said = scope_add(repo.path(), home.path(), "crates/engine", "billing")
        .expect("the machine holds the scope covering this directory");

    // The mitigation the un-pact line is: a boundary that moved is named,
    // because a script that quietly redrew somebody else's says whose.
    assert_eq!(said, "crates/engine is scoped `billing` — was `data-plane`");
    assert_eq!(
        stored(repo.path(), "crates/engine").scope(),
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

    let said = scope_add(repo.path(), home.path(), "docs", "Data-Plane")
        .expect("the fold happened before the judge");

    assert!(validate_scope("Data-Plane").is_err());
    assert_eq!(said, "docs is scoped `data-plane`");
    assert_eq!(stored(repo.path(), "docs").scope(), Some("data-plane"));
}

#[test]
fn removing_a_scope_clears_it_and_leaves_the_document_and_the_grant() {
    let repo = a_repository();
    let home = a_dir();
    holding(home.path(), repo.path(), &["data-plane"]);

    let said = scope_remove(repo.path(), home.path(), "crates/engine")
        .expect("the machine holds the scope covering this directory");

    assert_eq!(said, "crates/engine is no longer scoped — was `data-plane`");
    assert_eq!(status_for(&Ok(())), 0);
    let engine = stored(repo.path(), "crates/engine");
    assert_eq!(engine.scope(), None);
    assert_eq!(engine.document(), "crates/engine/WARLOCK.md");
    assert_eq!(engine.granted_hash(), Some(HASH));
    assert_eq!(engine.granted_at(), Some(AT));
    // The entry above it kept its own boundary: this is one entry's field.
    assert_eq!(stored(repo.path(), "crates").scope(), Some("platform"));
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

// `third-party` is named by no entry at all, and `data-plane` loses the only
// entry that named it to the un-pact below: a writer that pruned the records to
// what the entries spell would drop both, and where work under a scope is filed
// is not the un-pact's to forget.
fn records() -> Vec<ScopeRecord> {
    vec![
        ScopeRecord::new("data-plane", "Data Plane", "In Review", "area/data-plane"),
        ScopeRecord::new("third-party", "Vendor", "Triage", "area/vendor"),
    ]
}

// The same repository every other test here works over, with the records added
// before the first write rather than after: what these assert is that a command
// found them in the file and put them back.
fn a_repository_of_records() -> tempfile::TempDir {
    let repo = a_repository();
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

    // A name the fixture already records, because that is the add that writes
    // no record at all: the one that files a new one is the test below, and it
    // asserts the same records are still there under the one it appended.
    scope_add_bare(repo.path(), home.path(), "docs", "data-plane")
        .expect("`data-plane` is recorded, so the write wants no values");
    assert_eq!(
        record_bytes(repo.path()),
        before,
        "`scope add` moved a record"
    );
    assert_eq!(stored(repo.path(), "docs").scope(), Some("data-plane"));

    scope_remove(repo.path(), home.path(), "crates/engine").expect("the machine holds the scope");
    assert_eq!(
        record_bytes(repo.path()),
        before,
        "`scope remove` moved a record",
    );
    assert_eq!(stored(repo.path(), "crates/engine").scope(), None);
}

#[test]
fn an_unpact_keeps_the_records_including_the_one_it_orphaned() {
    let repo = a_repository_of_records();
    let home = a_dir();
    holding(home.path(), repo.path(), &["platform", "data-plane"]);
    let before = record_bytes(repo.path());

    unpact(repo.path(), home.path(), "crates").expect("an open boundary writes");

    let after = load_manifest(repo.path()).expect("a manifest that reads");
    assert_eq!(
        after
            .entries()
            .iter()
            .map(PactEntry::module)
            .collect::<Vec<_>>(),
        ["docs"],
        "the only entry naming `data-plane` really did go",
    );
    assert_eq!(after.scopes(), records());
    assert_eq!(record_bytes(repo.path()), before);
}

#[test]
fn a_name_nothing_records_wants_all_three_values_and_names_every_one_that_is_missing() {
    let repo = a_repository_of_records();
    let home = a_dir();
    let before = manifest_bytes(repo.path()).expect("a manifest on disk");

    // Every shape of "not all three", including none at all, which is what
    // `warlock scope add <path> <scope>` was until this slice.
    let missing: [Case; 5] = [
        (None, None, None, &["--team", "--review-state", "--label"]),
        (Some(TEAM), None, None, &["--review-state", "--label"]),
        (None, Some(REVIEW_STATE), Some(LABEL), &["--team"]),
        (Some(TEAM), Some(REVIEW_STATE), None, &["--label"]),
        (Some(TEAM), None, Some(LABEL), &["--review-state"]),
    ];

    for (team, review_state, label, wanted) in missing {
        let error = scope_add_with(
            repo.path(),
            home.path(),
            "docs",
            "billing",
            team,
            review_state,
            label,
        )
        .expect_err("`billing` is recorded nowhere");

        assert!(
            matches!(&error, Error::NoScopeRecord { scope, .. } if scope == "billing"),
            "{wanted:?}: {error:?}"
        );
        let said = error.to_string();
        // Every missing one named in the one line, and no flag that was
        // given named beside them: the whole point is a reader retyping the
        // command once rather than finding out about the next one after the
        // next run.
        for flag in wanted {
            assert!(said.contains(flag), "{wanted:?}: {said}");
        }
        for given in ["--team", "--review-state", "--label"] {
            assert_eq!(
                said.contains(given),
                wanted.contains(&given),
                "{wanted:?} named a flag that was given: {said}"
            );
        }
        assert!(!said.contains('\n'), "`main` prints one line");
        // A 1 and not the boundary's 3: nothing here is about what this
        // machine holds, and the same command typed again with the values in
        // it writes.
        assert_eq!(status_for(&Err(error)), 1, "{wanted:?}");
    }

    // And nothing was written on the way to any of them — not the pact's
    // scope either, which is what "one save" means from outside: there is no
    // road here that sets the scope and then fails to record it.
    assert_eq!(manifest_bytes(repo.path()).as_deref(), Some(&before[..]));
    assert_eq!(stored(repo.path(), "docs").scope(), None);
}

#[test]
fn a_blank_value_is_a_missing_one_and_anything_else_is_stored_exactly_as_typed() {
    let repo = a_repository();
    let home = a_dir();
    let before = manifest_bytes(repo.path()).expect("a manifest on disk");

    // Empty, spaces, and a tab: all three are a team, a state or a label
    // that routes nowhere while looking like it routes somewhere.
    for blank in ["", "   ", "\t"] {
        for (team, review_state, label) in [
            (blank, REVIEW_STATE, LABEL),
            (TEAM, blank, LABEL),
            (TEAM, REVIEW_STATE, blank),
        ] {
            let error = scope_add_with(
                repo.path(),
                home.path(),
                "docs",
                "billing",
                Some(team),
                Some(review_state),
                Some(label),
            )
            .expect_err("a blank value is not a value");

            assert!(
                matches!(error, Error::NoScopeRecord { .. }),
                "{blank:?}: {error:?}"
            );
            assert_eq!(status_for(&Err(error)), 1, "{blank:?}");
        }
    }
    assert_eq!(manifest_bytes(repo.path()).as_deref(), Some(&before[..]));

    // Past that one rule warlock judges none of them: what a team, a state or
    // a label may be is Linear's to say, so the padding and the capitals that
    // survive a blank check are written back exactly as they arrived.
    let padded = "  Billing Platform  ";
    scope_add_with(
        repo.path(),
        home.path(),
        "docs",
        "billing",
        Some(padded),
        Some(REVIEW_STATE),
        Some(LABEL),
    )
    .expect("a value with more than whitespace in it is a value");

    let after = load_manifest(repo.path()).expect("a manifest that reads");
    let record = after
        .scopes()
        .iter()
        .find(|record| record.name() == "billing")
        .expect("the write filed a record");
    assert_eq!(record.team(), padded);
    assert_eq!(record.review_state(), REVIEW_STATE);
    assert_eq!(record.label(), LABEL);
}

#[test]
fn a_new_name_writes_the_pact_and_its_record_together_and_moves_nothing_else() {
    let repo = a_repository_of_records();
    let home = a_dir();
    let before = record_bytes(repo.path());
    let entries_before = load_manifest(repo.path()).expect("a manifest that reads");

    let said = scope_add(repo.path(), home.path(), "docs", "billing")
        .expect("nothing scopes `docs` and nothing records `billing`");

    // The line is the one an add has always printed: the record is a second
    // thing written, not a second thing said.
    assert_eq!(said, "docs is scoped `billing`");
    assert_eq!(stored(repo.path(), "docs").scope(), Some("billing"));

    // The records that were there are still there, byte for byte and in
    // order, with the new one after them.
    let after = record_bytes(repo.path());
    assert!(after.starts_with(&before), "a record that was there moved");
    let written = load_manifest(repo.path()).expect("a manifest that reads");
    assert_eq!(&written.scopes()[..records().len()], &records()[..]);
    let filed = written.scopes().last().expect("the record that was filed");
    assert_eq!(filed.name(), "billing");
    assert_eq!(filed.team(), TEAM);
    assert_eq!(filed.review_state(), REVIEW_STATE);
    assert_eq!(filed.label(), LABEL);

    // And every pact row the write did not set out to change is the one it
    // read, grant and document and all.
    for module in ["crates", "crates/engine", "crates/engine/src"] {
        assert_eq!(
            written.entry(module),
            entries_before.entry(module),
            "{module}"
        );
    }

    // The point of filing the record in the same act: the route is there to
    // be answered for straight afterwards. This is the one engine call
    // `warlock check` renders its route line and its object from, so a name
    // this answers about is a name that command prints in full.
    let facts = route_facts(
        repo.path().join("docs"),
        repo.path(),
        &written,
        Some(home.path()),
    )
    .expect("a path inside the repository has a manifest form");
    let routed = facts.record().expect("the scope routes somewhere");
    assert_eq!(facts.scope(), Some("billing"));
    assert_eq!(routed.team(), TEAM);
    assert_eq!(routed.review_state(), REVIEW_STATE);
    assert_eq!(routed.label(), LABEL);
}

#[test]
fn the_name_that_is_recorded_is_the_folded_name_the_pact_carries() {
    // One boundary however it was typed, and therefore one record: a record
    // filed under `Billing` would be a record no pact ever names, which is
    // the unrouted scope this slice exists to stop making.
    let repo = a_repository();
    let home = a_dir();

    scope_add(repo.path(), home.path(), "docs", "Billing").expect("the fold happened first");

    let after = load_manifest(repo.path()).expect("a manifest that reads");
    assert_eq!(
        after.entry("docs").expect("the entry").scope(),
        Some("billing")
    );
    assert_eq!(
        after
            .scopes()
            .iter()
            .map(ScopeRecord::name)
            .collect::<Vec<_>>(),
        ["billing"]
    );
}

#[test]
fn a_recorded_name_takes_no_values_and_refuses_every_one_it_is_given() {
    let repo = a_repository_of_records();
    let home = a_dir();
    let before = manifest_bytes(repo.path()).expect("a manifest on disk");

    // One at a time and all three together, and the blank ones too: given is
    // given, and a `--team ''` that quietly did what no `--team` does is the
    // one outcome a reader could not tell from having their value written.
    let refused: [Case; 5] = [
        (Some(TEAM), None, None, &["--team"]),
        (None, Some(REVIEW_STATE), None, &["--review-state"]),
        (None, None, Some(LABEL), &["--label"]),
        (Some(""), None, None, &["--team"]),
        (
            Some(TEAM),
            Some(REVIEW_STATE),
            Some(LABEL),
            &["--team", "--review-state", "--label"],
        ),
    ];

    for (team, review_state, label, passed) in refused {
        let error = scope_add_with(
            repo.path(),
            home.path(),
            "docs",
            "data-plane",
            team,
            review_state,
            label,
        )
        .expect_err("`data-plane` already has a record");

        assert!(
            matches!(&error, Error::ScopeRecorded { scope, .. } if scope == "data-plane"),
            "{passed:?}: {error:?}"
        );
        let said = error.to_string();
        for flag in passed {
            assert!(said.contains(flag), "{passed:?}: {said}");
        }
        // Both roads out are in the line, because whoever typed the values
        // meant to say where the work is filed.
        assert!(said.contains(".warlock/pacts.toml"), "{said}");
        assert!(!said.contains('\n'), "`main` prints one line");
        assert_eq!(status_for(&Err(error)), 1, "{passed:?}");
        // Nothing written, and in particular not the scope: a refusal that
        // had already saved the pact would be the half-written state.
        assert_eq!(manifest_bytes(repo.path()).as_deref(), Some(&before[..]));
        assert_eq!(stored(repo.path(), "docs").scope(), None);
    }

    // And with none of them, the same command writes the scope exactly as it
    // did before this slice, leaving the record it found alone.
    let said = scope_add_bare(repo.path(), home.path(), "docs", "data-plane")
        .expect("a recorded name wants nothing else");
    assert_eq!(said, "docs is scoped `data-plane`");
    assert_eq!(stored(repo.path(), "docs").scope(), Some("data-plane"));
    assert_eq!(
        load_manifest(repo.path())
            .expect("a manifest that reads")
            .scopes(),
        records()
    );
}

#[test]
fn a_closed_boundary_is_the_whole_answer_whatever_the_values_were() {
    // The ordering that is the security property, held over the two new
    // refusals: whether `.warlock/pacts.toml` records a name, and therefore
    // which values the command wants, is a fact about the inside of a file
    // the reader has just been told they may not work in.
    let repo = a_repository_of_records();
    let home = a_dir();
    holding(home.path(), repo.path(), &["platform"]);
    let before = manifest_bytes(repo.path()).expect("a manifest on disk");

    for (scope, team, review_state, label) in [
        // A recorded name with values, which would otherwise be refused for
        // having them, and an unrecorded one with none, which would otherwise
        // be refused for wanting them.
        ("data-plane", Some(TEAM), Some(REVIEW_STATE), Some(LABEL)),
        ("billing", None, None, None),
    ] {
        let error = scope_add_with(
            repo.path(),
            home.path(),
            "crates/engine",
            scope,
            team,
            review_state,
            label,
        )
        .expect_err("a scope this machine does not hold refuses an add");

        assert!(
            matches!(error, Error::ClosedScope { .. }),
            "a record refusal outranked the boundary: {error:?}"
        );
        assert_eq!(
            error.to_string(),
            closed_scope_message("crates/engine", "data-plane"),
            "{scope}"
        );
        assert_eq!(status_for(&Err(error)), 3, "{scope}");
    }
    assert_eq!(manifest_bytes(repo.path()).as_deref(), Some(&before[..]));
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
        scope_add(repo.path(), home.path(), "crates/engine", "billing")
            .expect_err("a scope this machine does not hold refuses an add"),
        scope_remove(repo.path(), home.path(), "crates/engine").expect_err("and refuses a remove"),
    ];

    for error in refusals {
        assert!(
            matches!(error, Error::ClosedScope { .. }),
            "the boundary was refused as something else: {error:?}"
        );
        // The same sentence the un-pact is refused with and the same one
        // the footer puts up, asked of the one function that writes it.
        assert_eq!(
            error.to_string(),
            closed_scope_message("crates/engine", "data-plane")
        );
        assert!(!error.to_string().contains('\n'), "`main` prints one line");
        // The same boundary, so the same status the un-pact gets: one
        // refusal, one number, whichever write met it.
        assert_eq!(status_for(&Err(error)), 3);
    }
    assert_eq!(manifest_bytes(repo.path()).as_deref(), Some(&before[..]));
}

#[test]
fn the_boundary_is_asked_before_the_path_is_checked_for_an_entry() {
    // The ordering that is the security property: `crates/tui` has no entry
    // in the manifest and sits inside the boundary `crates` draws, so from
    // outside that boundary the answer is the scope refusal — never "is not
    // in the manifest", which is a fact about the inside of a file the
    // reader has just been told they may not work in.
    let repo = a_repository();
    let home = a_dir();
    let before = manifest_bytes(repo.path()).expect("a manifest on disk");

    let error = scope_add(repo.path(), home.path(), "crates/tui", "billing")
        .expect_err("holding nothing opens nothing that is scoped");

    assert!(
        matches!(error, Error::ClosedScope { .. }),
        "the manifest's shape leaked past a closed boundary: {error:?}"
    );
    assert!(
        !error.to_string().contains("not in the manifest"),
        "{error}"
    );
    assert_eq!(manifest_bytes(repo.path()).as_deref(), Some(&before[..]));

    // And past the same boundary held, the same path answers with what the
    // manifest holds — so the sentence exists and is only ever reached from
    // inside.
    holding(home.path(), repo.path(), &["platform"]);
    let error = scope_add(repo.path(), home.path(), "crates/tui", "billing")
        .expect_err("there is no entry to write a scope on");
    assert!(matches!(error, Error::NoPact { .. }), "{error:?}");
}

#[test]
fn a_closed_boundary_answers_a_clear_and_an_unpact_before_either_reads_the_manifest() {
    // The ordering the test above pins for `scope add`, held over the other
    // two writes, because it is one rule and the gate is one place: from
    // outside the boundary `crates` draws, neither may say what the manifest
    // holds about `crates/tui` — not "is not in the manifest" for the clear,
    // and not "0 entries dropped" for the un-pact, which is the same fact
    // about an empty subtree worded as a success.
    let repo = a_repository();
    let home = a_dir();
    let before = manifest_bytes(repo.path()).expect("a manifest on disk");

    for refused in [
        scope_remove(repo.path(), home.path(), "crates/tui"),
        unpact(repo.path(), home.path(), "crates/tui"),
    ] {
        let error = refused.expect_err("holding nothing opens nothing that is scoped");
        assert!(
            matches!(error, Error::ClosedScope { .. }),
            "the manifest's shape leaked past a closed boundary: {error:?}"
        );
        assert!(
            !error.to_string().contains("not in the manifest"),
            "{error}"
        );
        // And the status leaks nothing either: a closed boundary over a
        // path with no entry is the boundary's 3, the same number it would
        // be over a path with one.
        assert_eq!(status_for(&Err(error)), 3);
    }
    assert_eq!(manifest_bytes(repo.path()).as_deref(), Some(&before[..]));

    // And past the same boundary held, each answers about the manifest
    // after all: the clear with the refusal naming the pact that is not
    // there, the un-pact with a subtree that had nothing in it — so both
    // sentences exist and are only ever reached from inside.
    holding(home.path(), repo.path(), &["platform"]);
    let error = scope_remove(repo.path(), home.path(), "crates/tui")
        .expect_err("there is no entry to clear a scope on");
    assert!(matches!(error, Error::NoPact { .. }), "{error:?}");
    assert_eq!(
        unpact(repo.path(), home.path(), "crates/tui")
            .expect("nothing is pacted at or below `crates/tui`"),
        "unpacted crates/tui — 0 entries dropped"
    );
    assert_eq!(manifest_bytes(repo.path()).as_deref(), Some(&before[..]));
}

#[test]
fn a_scope_the_engine_refuses_prints_its_rule_and_writes_nothing() {
    let repo = a_repository();
    let home = a_dir();
    let before = manifest_bytes(repo.path()).expect("a manifest on disk");

    // The list, the capital-with-a-space, and the empty argument — which is
    // the `Empty` rule rather than a clear, because clearing is `scope
    // remove`. Each is judged after the fold, so the text held against the
    // judge here is the lower-cased one.
    for (given, folded) in [
        ("control-plane, data-plane", "control-plane, data-plane"),
        ("Control Plane", "control plane"),
        ("", ""),
        ("data-plane-", "data-plane-"),
    ] {
        let error =
            scope_add(repo.path(), home.path(), "docs", given).expect_err("this is not a scope");

        assert!(matches!(error, Error::Scope { .. }), "{given:?}: {error:?}");
        // The engine's own sentence about the one rule that was broken,
        // asked of the judge rather than retyped — and asked about the
        // folded text, because folding is the one thing done to what was
        // given.
        assert_eq!(error.to_string(), refusal(folded), "{given:?}");
        assert!(!error.to_string().contains('\n'), "{given:?}");
        assert_eq!(status_for(&Err(error)), 1, "{given:?}");
    }

    assert_eq!(manifest_bytes(repo.path()).as_deref(), Some(&before[..]));
}

#[test]
fn a_directory_with_no_entry_is_refused_past_an_open_boundary_and_writes_nothing() {
    // Nothing scopes `docs/adr` and nothing above it does, so the boundary
    // waves it through and the manifest gets the next word: there is no pact
    // here to carry a scope.
    let repo = a_repository();
    let home = a_dir();
    let before = manifest_bytes(repo.path()).expect("a manifest on disk");

    let refusals = [
        scope_add(repo.path(), home.path(), "docs/adr", "billing")
            .expect_err("`docs/adr` has no entry"),
        scope_remove(repo.path(), home.path(), "docs/adr")
            .expect_err("and has none to clear either"),
    ];

    for error in refusals {
        assert!(matches!(error, Error::NoPact { .. }), "{error:?}");
        let said = error.to_string();
        // `no_pact_message`'s shape: it names the directory and points at
        // pacting it.
        assert!(said.contains("docs/adr"), "{said}");
        assert!(said.contains("`p`"), "{said}");
        assert!(!said.contains('\n'), "`main` prints one line");
        assert_eq!(status_for(&Err(error)), 1);
    }
    assert_eq!(manifest_bytes(repo.path()).as_deref(), Some(&before[..]));
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
            opened().and_then(|opened| {
                opened.scoped("billing", Some(TEAM), Some(REVIEW_STATE), Some(LABEL))
            }),
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

// Every shape the two doors have to answer alike, in one manifest: a root
// entry carrying no scope, a boundary this machine holds on `crates`, one it
// does not on `crates/engine` below that, and a second subtree whose only
// boundary below is one it does hold. One manifest rather than one per case,
// because a parity test over several fixtures would be showing that the two
// doors agree about several different repositories.
fn a_manifest_of_boundaries_both_ways() -> Manifest {
    Manifest::with_entries([
        // Spelled out rather than through `entry`, which would document the
        // root as `./WARLOCK.md`.
        PactEntry::new(".", ".", "WARLOCK.md")
            .expect("the repository root is inside itself")
            .with_grant(HASH, AT),
        entry("crates").with_scope(HELD),
        entry("crates/engine").with_scope(CLOSED),
        entry("crates/engine/src"),
        entry("docs"),
        entry("docs/api").with_scope(HELD),
    ])
}

// No documents on disk: these tests are about which un-pacts are allowed, and
// neither door reads a `WARLOCK.md` to decide that. That an un-pact leaves
// every document where it was is pinned above, over a repository that has
// them.
fn a_repository_of_boundaries() -> (tempfile::TempDir, tempfile::TempDir) {
    let repo = a_dir();
    let home = a_dir();
    a_manifest_of_boundaries_both_ways()
        .save(repo.path())
        .expect("a manifest that saves");
    holding(home.path(), repo.path(), &[HELD]);

    (repo, home)
}

// Every directory the manifest names, each one pacted, so that `p` on any row
// is an un-pact.
fn a_panel_over(repo_root: &Path) -> App {
    let node = |name: &str, children: Vec<Node>| {
        Node::new(
            repo_root.join(name),
            None::<PathBuf>,
            NodeState::PactedFresh,
        )
        .with_children(children)
    };

    App::from_tree(&Tree::new(
        Node::new(repo_root, None::<PathBuf>, NodeState::PactedFresh).with_children([
            node(
                "crates",
                vec![node(
                    "crates/engine",
                    vec![node("crates/engine/src", Vec::new())],
                )],
            ),
            node("docs", vec![node("docs/api", Vec::new())]),
        ]),
    ))
}

// The answer, and deliberately not the mechanism: the panel refuses by
// painting nothing and putting a line on the footer, the shell by handing
// `main` an error to print. Those are two shapes of one rule, and this is
// what the two of them have to be equal in.
#[derive(Debug, PartialEq, Eq)]
enum Answer {
    WentAhead,
    Refused(String),
}

// The one difference between the doors that is not about the rule:
// `App::label_for` spells a row relative to the tree's root and falls back to
// the absolute path for the root row itself, where the shell spells that row
// `.`. Every other row is named by the manifest's own spelling on both sides,
// so this is a no-op for them.
fn as_the_shell_says_it(sentence: &str, repo_root: &Path) -> String {
    sentence.replace(&repo_root.display().to_string(), ".")
}

fn panel_answer(repo_root: &Path, home: &Path, path: &str) -> Answer {
    let manifest = load_manifest(repo_root).expect("a manifest that reads");
    // The header's own reading of the config `warlock config` wrote, which
    // is what the running app holds and what the shell reads for itself.
    let sigils = sigils_under(home, repo_root);
    let mut app = a_panel_over(repo_root);
    let target = if path == "." {
        repo_root.to_path_buf()
    } else {
        repo_root.join(path)
    };
    let row = app
        .rows()
        .iter()
        .position(|row| row.path == target)
        .expect("the panel draws a row for this directory");
    app.select_row(row);

    match pressed_p(&mut app, &manifest, repo_root, &sigils) {
        Some(toggle) => {
            assert!(!toggle.pacted, "{path}: the press was not an un-pact");
            Answer::WentAhead
        }
        None => Answer::Refused(as_the_shell_says_it(
            app.message().expect("a refused press says why"),
            repo_root,
        )),
    }
}

fn shell_answer(repo_root: &Path, home: &Path, path: &str) -> Answer {
    match unpact(repo_root, home, path) {
        Ok(_) => Answer::WentAhead,
        Err(error) => Answer::Refused(error.to_string()),
    }
}

#[test]
fn a_key_press_and_a_shell_prompt_answer_the_same_un_pact_alike() {
    // The rule's own last clause: there is no path by which one door refuses
    // and the other permits. Both are pressed over one manifest, by one
    // machine holding one sigil, and the answers are held against each other
    // *and* against what the answer is supposed to be — so a change to
    // either door alone fails here, and so does a change to both that moves
    // the rule.
    //
    // The rule is `docs/warlock-decision-un-pacting-across-a-descendant-scope.md`.
    let refused_here = format!(
        "crates/engine is scoped `{CLOSED}` — hold that sigil to work here, \
             with `warlock config`"
    );
    let refused_below = |label: &str| {
        format!(
            "un-pacting {label} would drop pacts scoped `{CLOSED}` — hold that sigil with \
                 `warlock config`, or un-pact the parts you hold"
        )
    };

    for (path, expected) in [
        // A scope on the target itself, which coverage has always seen.
        ("crates/engine", Answer::Refused(refused_here)),
        // A target this machine's own sigil opens, over an entry below it
        // that it does not: passing the first question is not permission for
        // the second.
        ("crates", Answer::Refused(refused_below("crates"))),
        // A boundary below that this machine holds is no obstacle, so the
        // subtree goes — the rule refuses over scopes, not over having any.
        ("docs", Answer::WentAhead),
        // The root, which carries no scope of its own. That is the absence
        // of a statement rather than permission over the statements below.
        (".", Answer::Refused(refused_below("."))),
    ] {
        // A repository each, because the un-pact that goes ahead saves.
        let (repo, home) = a_repository_of_boundaries();

        let panel = panel_answer(repo.path(), home.path(), path);
        let shell = shell_answer(repo.path(), home.path(), path);

        assert_eq!(
            panel, shell,
            "`p` and `warlock unpact` disagree over {path}"
        );
        assert_eq!(panel, expected, "the answer over {path} has changed");
    }
}
