use std::fs;
use std::path::Path;

use warlock_engine::{Agent, Manifest, agent, stub_answer};
use warlock_tui::Cancel;

use super::{Descent, carry_on, descend};
use crate::error::Error;

struct Answering;

impl Agent for Answering {
    fn run(&self, request: &agent::Request) -> Result<agent::Response, agent::Error> {
        // Every slot the engine checks for, filled: what a pass has to
        // answer before the engine will write a document down.
        Ok(agent::Response::new(stub_answer(request)))
    }
}

struct Never;

impl Agent for Never {
    fn run(&self, _request: &agent::Request) -> Result<agent::Response, agent::Error> {
        panic!("a fresh subtree must cost no pass")
    }
}

fn a_checkout() -> tempfile::TempDir {
    let repo = tempfile::tempdir().expect("a temporary directory");
    fs::create_dir_all(repo.path().join(".git")).expect("a repository marker");
    fs::create_dir_all(repo.path().join("src")).expect("a source directory");
    fs::write(repo.path().join("src/lib.rs"), "//! a module\n").expect("a source file");
    repo
}

fn stored(repo: &Path) -> Vec<String> {
    Manifest::load(repo)
        .expect("a manifest that reads")
        .entries()
        .iter()
        .map(|entry| entry.module().to_owned())
        .collect()
}

#[test]
fn a_pact_saves_once_and_the_manifest_on_disk_is_what_came_back() {
    let repo = a_checkout();

    let subtree = descend(
        Descent::Pact,
        repo.path(),
        repo.path(),
        &Manifest::new(),
        &Answering,
        &Cancel::new(),
        &mut |_| {},
    )
    .expect("a pact of a readable subtree");

    assert!(subtree.failures.is_empty(), "{:?}", subtree.failures);
    assert_eq!(
        stored(repo.path()),
        subtree
            .manifest
            .entries()
            .iter()
            .map(|entry| entry.module().to_owned())
            .collect::<Vec<_>>(),
        "the manifest on disk is not the one the descent handed back"
    );
}

#[test]
fn an_un_pact_drops_the_entries_and_leaves_every_document_on_disk() {
    let repo = a_checkout();
    descend(
        Descent::Pact,
        repo.path(),
        repo.path(),
        &Manifest::new(),
        &Answering,
        &Cancel::new(),
        &mut |_| {},
    )
    .expect("a pact of a readable subtree");
    let documents: Vec<_> = ["WARLOCK.md", "src/WARLOCK.md"]
        .into_iter()
        .map(|name| fs::read(repo.path().join(name)).expect("a document the pact wrote"))
        .collect();

    let manifest = Manifest::load(repo.path()).expect("a manifest that reads");
    descend(
        Descent::Unpact,
        repo.path(),
        repo.path(),
        &manifest,
        &Answering,
        &Cancel::new(),
        &mut |_| {},
    )
    .expect("an un-pact of a pacted subtree");

    assert!(
        stored(repo.path()).is_empty(),
        "an entry survived the un-pact"
    );
    for (name, before) in ["WARLOCK.md", "src/WARLOCK.md"].into_iter().zip(documents) {
        assert_eq!(
            fs::read(repo.path().join(name)).expect("the document is still there"),
            before,
            "un-pacting touched `{name}` on disk"
        );
    }
}

#[test]
fn a_refresh_of_a_fresh_subtree_spends_no_pass_and_still_saves() {
    let repo = a_checkout();
    descend(
        Descent::Pact,
        repo.path(),
        repo.path(),
        &Manifest::new(),
        &Answering,
        &Cancel::new(),
        &mut |_| {},
    )
    .expect("a pact of a readable subtree");
    let before = stored(repo.path());

    let manifest = Manifest::load(repo.path()).expect("a manifest that reads");
    let subtree = descend(
        Descent::Refresh,
        repo.path(),
        repo.path(),
        &manifest,
        &Never,
        &Cancel::new(),
        &mut |_| {},
    )
    .expect("a refresh with nothing stale");

    assert!(subtree.failures.is_empty());
    assert_eq!(
        stored(repo.path()),
        before,
        "a no-op refresh moved an entry"
    );
}

#[test]
fn a_subtree_that_cannot_be_walked_is_a_pact_error_and_saves_nothing() {
    let repo = a_checkout();

    let error = descend(
        Descent::Pact,
        &repo.path().join("nowhere"),
        repo.path(),
        &Manifest::new(),
        &Answering,
        &Cancel::new(),
        &mut |_| {},
    )
    .expect_err("a directory that is not there cannot be pacted");

    assert!(matches!(error, Error::Pact { .. }), "{error:?}");
    assert!(
        !repo.path().join(".warlock/pacts.toml").exists(),
        "a run that never started wrote a manifest"
    );
}

#[test]
fn an_un_pact_of_a_path_the_manifest_cannot_spell_is_a_manifest_error() {
    let repo = a_checkout();

    let error = descend(
        Descent::Unpact,
        Path::new("/elsewhere"),
        repo.path(),
        &Manifest::new(),
        &Answering,
        &Cancel::new(),
        &mut |_| {},
    )
    .expect_err("nothing outside the repository has a manifest form");

    assert!(matches!(error, Error::Manifest { .. }), "{error:?}");
}

#[test]
fn a_pulled_say_when_stops_a_run_and_an_unpulled_one_does_not() {
    let cancel = Cancel::new();

    assert_eq!(
        carry_on(&cancel),
        warlock_engine::Pacting::Continue,
        "nobody has said stop"
    );

    cancel.cancel();

    assert_eq!(
        carry_on(&cancel),
        warlock_engine::Pacting::Stop,
        "the latch both doors read was pulled and one of them carried on"
    );
}

#[test]
fn every_descent_wants_a_root_for_something_it_can_name() {
    for descent in [Descent::Pact, Descent::Refresh, Descent::Unpact] {
        assert!(
            !descent.wanted().is_empty(),
            "{descent:?} cannot say what it wanted a repository for"
        );
    }
}
