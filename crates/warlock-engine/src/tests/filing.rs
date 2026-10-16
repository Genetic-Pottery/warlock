use std::mem::discriminant;
use std::path::Path;

use super::{Destination, Error, resolve_filing};
use crate::config::{config_path, save_key_binding, save_open_scopes};
use crate::keys::save_key;
use crate::manifest::{Manifest, ScopeRecord, ScopedDirectory, manifest_path};
use crate::route::{self, resolve_route};

// A plausible Linear key rather than the word "secret", so a test that looks for
// it proves the redaction and not an unlucky choice of needle.
const KEY: &str = "lin_api_9f3c0d11aa47";

// The second scope's key, so a target that resolved the wrong binding is told
// apart by its value and not only by its name.
const OTHER_KEY: &str = "lin_api_7b21e4c0fd93";

fn a_dir() -> tempfile::TempDir {
    tempfile::tempdir().expect("a temporary directory")
}

fn scoped(path: &str, scope: &str) -> ScopedDirectory {
    ScopedDirectory::new(".", path, scope).expect("a relative path is inside the root")
}

// One directory per scope, so the rows agree with the records: `Unrecorded` is
// decided from the rows and the board refusals from the records, and a fixture
// that recorded a scope no directory carries could pass one while the other
// never runs.
fn a_manifest(scopes: &[&str]) -> Manifest {
    Manifest::with_directories(
        scopes
            .iter()
            .map(|scope| scoped(scope, scope))
            .collect::<Vec<_>>(),
    )
    .with_scopes(
        scopes
            .iter()
            .map(|scope| ScopeRecord::new(*scope, format!("Team {scope}"), "In Review", "warlock")),
    )
}

fn owned(open: &[&str]) -> Vec<String> {
    open.iter().map(|scope| (*scope).to_owned()).collect()
}

fn opens(home: &Path, root: &Path, open: &[&str]) {
    save_open_scopes(home, root, &owned(open)).expect("records what this machine opens");
}

// The two scopes these fixtures can file to, each bound to a key this machine
// really holds, so a test about the board half fails for its own reason rather
// than at `Unbound`. Two different names rather than one: the binding is per
// scope, and a fixture that bound both to one key could not tell a target that
// resolved the right scope's key from one that resolved whatever was bound.
fn bound(home: &Path, root: &Path) {
    save_key_binding(home, root, "data-plane", "work").expect("binds the first scope");
    save_key_binding(home, root, "web", "personal").expect("binds the second");
    save_key(home, "work", KEY).expect("stores that key on this machine");
    save_key(home, "personal", OTHER_KEY).expect("stores the other");
}

#[test]
fn one_candidate_is_the_board_and_carries_the_key() {
    let (home, root) = (a_dir(), a_dir());
    bound(home.path(), root.path());
    opens(home.path(), root.path(), &["data-plane"]);

    let manifest = a_manifest(&["data-plane", "web"]);
    let target = resolve_filing(&manifest, root.path(), home.path(), None)
        .expect("one open scope with one record of that name is one board");

    assert_eq!(target.scope(), "data-plane");
    assert_eq!(target.record().name(), "data-plane");
    assert_eq!(target.record().team_key(), "Team data-plane");
    assert_eq!(target.record().label(), "warlock");
    assert_eq!(target.key(), "work");
    assert_eq!(
        target.value(),
        KEY,
        "the caller with a target is the caller about to make the request"
    );
    assert!(
        !format!("{target:?}").contains(KEY),
        "the one type in this crate that holds a key must not print it: {target:?}"
    );
}

#[test]
fn a_destination_is_the_target_by_name_with_no_key_value_in_it() {
    let (home, root) = (a_dir(), a_dir());
    bound(home.path(), root.path());
    opens(home.path(), root.path(), &["data-plane"]);

    let manifest = a_manifest(&["data-plane", "web"]);
    let destination = resolve_filing(&manifest, root.path(), home.path(), None)
        .expect("one board")
        .destination();

    assert_eq!(
        destination,
        Destination::new("data-plane", "Team data-plane", "warlock", "work")
    );
    assert!(
        !format!("{destination:?}").contains(KEY),
        "a destination printed the key value: {destination:?}"
    );
}

#[test]
fn several_candidates_name_every_one_of_them_and_the_way_to_pick() {
    let (home, root) = (a_dir(), a_dir());
    bound(home.path(), root.path());
    opens(home.path(), root.path(), &["data-plane", "web"]);

    let error = resolve_filing(
        &a_manifest(&["data-plane", "web"]),
        root.path(),
        home.path(),
        None,
    )
    .expect_err("a brief is not about a directory, so nothing breaks this tie");

    assert!(matches!(&error, Error::Several { .. }), "{error:?}");
    let text = error.to_string();
    assert!(text.contains("`data-plane`"), "{text}");
    assert!(text.contains("`web`"), "{text}");
    assert!(
        text.contains("--scope"),
        "the refusal names the fix: {text}"
    );
}

#[test]
fn the_wildcard_is_every_recorded_scope() {
    // One record and the wildcard is one candidate, and a second record turns the
    // same open set into the several-candidates refusal: the boundary rule is
    // `scope_opens_to`'s, so `*` is never a name to compare against.
    let (home, root) = (a_dir(), a_dir());
    bound(home.path(), root.path());
    opens(home.path(), root.path(), &["*"]);

    let one = a_manifest(&["data-plane"]);
    let target = resolve_filing(&one, root.path(), home.path(), None)
        .expect("the wildcard over one recorded scope is one candidate");
    assert_eq!(target.scope(), "data-plane");

    let error = resolve_filing(
        &a_manifest(&["data-plane", "web"]),
        root.path(),
        home.path(),
        None,
    )
    .expect_err("the wildcard over two recorded scopes is two candidates");
    assert!(matches!(&error, Error::Several { .. }), "{error:?}");
}

#[test]
fn a_name_among_the_candidates_picks_that_board() {
    let (home, root) = (a_dir(), a_dir());
    bound(home.path(), root.path());
    opens(home.path(), root.path(), &["*"]);

    let manifest = a_manifest(&["data-plane", "web"]);
    let target = resolve_filing(&manifest, root.path(), home.path(), Some("web"))
        .expect("a name answers the question the refusal asks");

    assert_eq!(target.scope(), "web");
    assert_eq!(target.record().team_key(), "Team web");
    assert_eq!(
        target.key(),
        "personal",
        "the key resolved is the one bound to the record that was picked"
    );
    assert_eq!(target.value(), OTHER_KEY);
}

#[test]
fn a_name_that_is_not_a_candidate_is_refused_with_every_candidate_named() {
    let (home, root) = (a_dir(), a_dir());
    bound(home.path(), root.path());
    opens(home.path(), root.path(), &["data-plane"]);

    // `web` is recorded here and open to nobody: a name this repository knows is
    // still not a board this machine can file to, which is the case a check
    // against the records alone would let through.
    let error = resolve_filing(
        &a_manifest(&["data-plane", "web"]),
        root.path(),
        home.path(),
        Some("web"),
    )
    .expect_err("a recorded scope that is not open on this machine is not a candidate");

    assert!(
        matches!(&error, Error::Unknown { name, .. } if name == "web"),
        "{error:?}"
    );
    let text = error.to_string();
    assert!(text.contains("`web`"), "{text}");
    assert!(
        text.contains("`data-plane`"),
        "the refusal names what can be filed to instead: {text}"
    );
}

#[test]
fn the_four_ways_to_have_no_candidate_are_four_different_refusals() {
    let manifest = a_manifest(&["data-plane"]);

    // A repository that records no scope, asked before anything about this
    // machine: the candidates come from the records alone, so no scope opened
    // here would make a board and the fix is a line in the manifest. Opening
    // nothing, opening one name and opening the wildcard all land on this one
    // sentence, because all three are the same repository.
    let (empty_home, empty_root) = (a_dir(), a_dir());
    bound(empty_home.path(), empty_root.path());
    let recordless = Manifest::with_directories([scoped("crates", "data-plane")]);
    let mut boardless = Vec::new();
    for open in [&[][..], &["data-plane"][..], &["*"][..]] {
        opens(empty_home.path(), empty_root.path(), open);
        let error = resolve_filing(&recordless, empty_root.path(), empty_home.path(), None)
            .expect_err("no `[[scope]]` record is nowhere to file, whatever is open");
        assert!(
            matches!(error, Error::Boardless { .. }),
            "opening {open:?}: {error:?}"
        );
        let text = error.to_string();
        assert!(
            text.contains("[[scope]]")
                && text.contains(&manifest_path(empty_root.path()).display().to_string()),
            "the refusal names the file to go and fix: {text}"
        );
        assert!(
            !text.contains("warlock open"),
            "the fix is in the repository, not on this machine: {text}"
        );
        boardless.push(error);
    }

    // Nothing open at all, and a machine nobody has configured for this checkout:
    // `load_open_scopes` saying NotFound is a machine that opens nothing here.
    let (home, root) = (a_dir(), a_dir());
    bound(home.path(), root.path());
    opens(home.path(), root.path(), &[]);
    let nothing_open = resolve_filing(&manifest, root.path(), home.path(), None)
        .expect_err("a machine that opens nothing files to nothing");
    assert!(
        matches!(nothing_open, Error::NothingOpen),
        "{nothing_open:?}"
    );
    assert!(
        nothing_open.to_string().contains("warlock open <scope>"),
        "{nothing_open}"
    );

    let (fresh_home, fresh_root) = (a_dir(), a_dir());
    let never_configured = resolve_filing(&manifest, fresh_root.path(), fresh_home.path(), None)
        .expect_err("no config for this checkout at all");
    assert_eq!(
        discriminant(&never_configured),
        discriminant(&nothing_open),
        "{never_configured:?}"
    );

    // Open scopes this repository records no scope for: the machine is pointed at
    // the wrong repository, which is a different fix to the one below.
    let (open_home, open_root) = (a_dir(), a_dir());
    bound(open_home.path(), open_root.path());
    opens(open_home.path(), open_root.path(), &["billing", "web"]);
    let unmatched = resolve_filing(&manifest, open_root.path(), open_home.path(), None)
        .expect_err("nothing here is named `billing` or `web`");
    assert!(
        matches!(unmatched, Error::Unmatched { .. }),
        "{unmatched:?}"
    );
    let text = unmatched.to_string();
    assert!(
        text.contains("`billing`") && text.contains("`web`"),
        "{text}"
    );
    assert!(
        text.contains(&manifest_path(open_root.path()).display().to_string()),
        "{text}"
    );

    // And an open scope some directory carries with no `[[scope]]` record: the
    // manifest is one line short rather than the machine being wrong. It records
    // `web`, because a manifest recording nothing at all is the refusal above
    // rather than this one — this is a file one line short, not a file nobody
    // has started.
    let unrecorded_manifest = Manifest::with_directories([scoped("crates", "data-plane")])
        .with_scopes([ScopeRecord::new("web", "Team web", "In Review", "warlock")]);
    let (bare_home, bare_root) = (a_dir(), a_dir());
    bound(bare_home.path(), bare_root.path());
    opens(bare_home.path(), bare_root.path(), &["data-plane"]);
    let unrecorded = resolve_filing(
        &unrecorded_manifest,
        bare_root.path(),
        bare_home.path(),
        None,
    )
    .expect_err("no `[[scope]]` record carries that name");
    assert!(
        matches!(unrecorded, Error::Unrecorded { .. }),
        "{unrecorded:?}"
    );
    let text = unrecorded.to_string();
    assert!(text.contains("`data-plane`"), "{text}");
    assert!(
        text.contains("[[scope]]")
            && text.contains(&manifest_path(bare_root.path()).display().to_string()),
        "the refusal names the file to go and fix: {text}"
    );

    let refusals = [&boardless[0], &nothing_open, &unmatched, &unrecorded];
    for (index, one) in refusals.iter().enumerate() {
        for other in &refusals[index + 1..] {
            assert_ne!(
                discriminant(*one),
                discriminant(*other),
                "each of these is fixed somewhere else: {one:?} and {other:?}"
            );
        }
    }
}

#[test]
fn a_recorded_scope_no_directory_carries_is_still_a_board() {
    // The refusal above is on `scopes()` and not on the rows: where a brief files
    // is the `[[scope]]` record's question alone, and a repository that has
    // recorded a board before scoping any directory to it can still push. A test
    // written against the rows would have made this a refusal.
    let (home, root) = (a_dir(), a_dir());
    bound(home.path(), root.path());
    opens(home.path(), root.path(), &["data-plane"]);

    let manifest = Manifest::new().with_scopes([ScopeRecord::new(
        "data-plane",
        "WAR",
        "In Review",
        "warlock",
    )]);
    let target = resolve_filing(&manifest, root.path(), home.path(), None)
        .expect("one record of the open name is one board, carried by a directory or not");

    assert_eq!(target.scope(), "data-plane");
    assert_eq!(target.record().team_key(), "WAR");
}

#[test]
fn a_checkout_that_is_both_ambiguous_and_unbound_is_told_about_the_board() {
    // Two things to fix, and `--scope` is the one that is about this push: the
    // board refusals are decided before the key store is touched.
    let (home, root) = (a_dir(), a_dir());
    opens(home.path(), root.path(), &["*"]);

    let error = resolve_filing(
        &a_manifest(&["data-plane", "web"]),
        root.path(),
        home.path(),
        None,
    )
    .expect_err("two candidates and no key bound to this checkout");

    assert!(
        matches!(&error, Error::Several { .. }),
        "the board refusal comes first: {error:?}"
    );
}

#[test]
fn the_unbound_and_dangling_refusals_are_route_s_own_sentences() {
    let manifest = a_manifest(&["data-plane"]);

    // Both are compared against `resolve_route` on the same machine rather than
    // against a copy of the wording: a person meets these two from `warlock
    // check` first, and one refusal worded twice is one of them going stale.
    let (unbound_home, unbound_root) = (a_dir(), a_dir());
    opens(unbound_home.path(), unbound_root.path(), &["data-plane"]);
    let unbound = resolve_filing(&manifest, unbound_root.path(), unbound_home.path(), None)
        .expect_err("the scope this files to is bound to no key");
    assert!(
        matches!(
            &unbound,
            Error::Key {
                source: route::Error::Unbound { scope, .. }
            } if scope == "data-plane"
        ),
        "{unbound:?}"
    );
    assert_eq!(
        unbound.to_string(),
        resolve_route(
            "data-plane/src",
            unbound_root.path(),
            &manifest,
            unbound_home.path(),
        )
        .expect_err("the same machine refuses the same way")
        .to_string()
    );
    assert!(
        unbound.to_string().contains(
            &config_path(unbound_home.path(), unbound_root.path())
                .display()
                .to_string()
        ),
        "{unbound}"
    );

    let (dangling_home, dangling_root) = (a_dir(), a_dir());
    opens(dangling_home.path(), dangling_root.path(), &["data-plane"]);
    save_key_binding(
        dangling_home.path(),
        dangling_root.path(),
        "data-plane",
        "work",
    )
    .expect("binds");
    save_key(dangling_home.path(), "personal", KEY).expect("stores a different name's key");
    let dangling = resolve_filing(&manifest, dangling_root.path(), dangling_home.path(), None)
        .expect_err("the bound name is in no key store");
    assert!(
        matches!(
            &dangling,
            Error::Key {
                source: route::Error::Dangling { .. }
            }
        ),
        "{dangling:?}"
    );
    assert_eq!(
        dangling.to_string(),
        resolve_route(
            "data-plane/src",
            dangling_root.path(),
            &manifest,
            dangling_home.path(),
        )
        .expect_err("the same machine refuses the same way")
        .to_string()
    );

    let text = format!("{dangling} | {dangling:?}");
    assert!(text.contains("`work`"), "the name is the fix: {text}");
    assert!(!text.contains(KEY), "a key reached a refusal: {text}");
    assert!(!text.contains("lin_api"), "not even its shape: {text}");
}
