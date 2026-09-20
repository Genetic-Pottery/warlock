use std::mem::discriminant;
use std::path::Path;

use super::{Error, resolve_filing};
use crate::keys::save_key;
use crate::manifest::{Manifest, PactEntry, ScopeRecord, manifest_path};
use crate::route::{self, resolve_route};
use crate::sigils::{save_key_binding, save_sigils, sigils_path};

// A plausible Linear key rather than the word "secret", so a test that looks for
// it proves the redaction and not an unlucky choice of needle.
const KEY: &str = "lin_api_9f3c0d11aa47";

fn a_dir() -> tempfile::TempDir {
    tempfile::tempdir().expect("a temporary directory")
}

fn entry(module: &str) -> PactEntry {
    PactEntry::new(".", module, format!("{module}/WARLOCK.md"))
        .expect("a relative module path is inside the root")
}

// One pact per scope, so the entries agree with the records: `Unrecorded` is
// decided from the entries and the board refusals from the records, and a
// fixture that recorded a scope no pact carries could pass one while the other
// never runs.
fn a_manifest(scopes: &[&str]) -> Manifest {
    Manifest::with_entries(
        scopes
            .iter()
            .map(|scope| entry(scope).with_scope(*scope))
            .collect::<Vec<_>>(),
    )
    .with_scopes(
        scopes
            .iter()
            .map(|scope| ScopeRecord::new(*scope, format!("Team {scope}"), "In Review", "warlock")),
    )
}

fn owned(sigils: &[&str]) -> Vec<String> {
    sigils.iter().map(|sigil| (*sigil).to_owned()).collect()
}

fn holds(home: &Path, root: &Path, sigils: &[&str]) {
    save_sigils(home, root, &owned(sigils)).expect("records what this machine holds");
}

// A checkout bound to a key this machine really holds, so a test about the board
// half fails for its own reason rather than at `Unbound`.
fn bound(home: &Path, root: &Path) {
    save_key_binding(home, root, "work").expect("binds this checkout to a key");
    save_key(home, "work", KEY).expect("stores that key on this machine");
}

#[test]
fn one_candidate_is_the_board_and_carries_the_key() {
    let (home, root) = (a_dir(), a_dir());
    bound(home.path(), root.path());
    holds(home.path(), root.path(), &["data-plane"]);

    let manifest = a_manifest(&["data-plane", "web"]);
    let target = resolve_filing(&manifest, root.path(), home.path(), None)
        .expect("one held sigil with one record of that name is one board");

    assert_eq!(target.scope(), "data-plane");
    assert_eq!(target.record().name(), "data-plane");
    assert_eq!(target.record().team(), "Team data-plane");
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
fn several_candidates_name_every_one_of_them_and_the_way_to_pick() {
    let (home, root) = (a_dir(), a_dir());
    bound(home.path(), root.path());
    holds(home.path(), root.path(), &["data-plane", "web"]);

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
    // same held set into the several-candidates refusal: the boundary rule is
    // `scope_opens_to`'s, so `*` is never a name to compare against.
    let (home, root) = (a_dir(), a_dir());
    bound(home.path(), root.path());
    holds(home.path(), root.path(), &["*"]);

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
    holds(home.path(), root.path(), &["*"]);

    let manifest = a_manifest(&["data-plane", "web"]);
    let target = resolve_filing(&manifest, root.path(), home.path(), Some("web"))
        .expect("a name answers the question the refusal asks");

    assert_eq!(target.scope(), "web");
    assert_eq!(target.record().team(), "Team web");
}

#[test]
fn a_name_that_is_not_a_candidate_is_refused_with_every_candidate_named() {
    let (home, root) = (a_dir(), a_dir());
    bound(home.path(), root.path());
    holds(home.path(), root.path(), &["data-plane"]);

    // `web` is recorded here and held by nobody: a name this repository knows is
    // still not a board this machine can file to, which is the case a check
    // against the records alone would let through.
    let error = resolve_filing(
        &a_manifest(&["data-plane", "web"]),
        root.path(),
        home.path(),
        Some("web"),
    )
    .expect_err("a recorded scope this machine holds no sigil for is not a candidate");

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
fn the_three_ways_to_have_no_candidate_are_three_different_refusals() {
    let manifest = a_manifest(&["data-plane"]);

    // Nothing held at all, and a machine nobody has configured for this checkout:
    // `load_sigils` saying NotFound is a machine that holds nothing here.
    let (home, root) = (a_dir(), a_dir());
    bound(home.path(), root.path());
    holds(home.path(), root.path(), &[]);
    let unsigiled = resolve_filing(&manifest, root.path(), home.path(), None)
        .expect_err("a machine holding nothing files to nothing");
    assert!(matches!(unsigiled, Error::Unsigiled), "{unsigiled:?}");
    assert!(
        unsigiled.to_string().contains("warlock config"),
        "{unsigiled}"
    );

    let (fresh_home, fresh_root) = (a_dir(), a_dir());
    let never_configured = resolve_filing(&manifest, fresh_root.path(), fresh_home.path(), None)
        .expect_err("no config for this checkout at all");
    assert_eq!(
        discriminant(&never_configured),
        discriminant(&unsigiled),
        "{never_configured:?}"
    );

    // Held sigils this repository records no scope for: the machine is pointed at
    // the wrong repository, which is a different fix to the one below.
    let (held_home, held_root) = (a_dir(), a_dir());
    bound(held_home.path(), held_root.path());
    holds(held_home.path(), held_root.path(), &["billing", "web"]);
    let unmatched = resolve_filing(&manifest, held_root.path(), held_home.path(), None)
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
        text.contains(&manifest_path(held_root.path()).display().to_string()),
        "{text}"
    );

    // And a held sigil some pact carries with no `[[scope]]` record: the manifest
    // is one line short rather than the machine being wrong.
    let unrecorded_manifest = Manifest::with_entries([entry("crates").with_scope("data-plane")]);
    let (bare_home, bare_root) = (a_dir(), a_dir());
    bound(bare_home.path(), bare_root.path());
    holds(bare_home.path(), bare_root.path(), &["data-plane"]);
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

    let refusals = [&unsigiled, &unmatched, &unrecorded];
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
fn a_checkout_that_is_both_ambiguous_and_unbound_is_told_about_the_board() {
    // Two things to fix, and `--scope` is the one that is about this push: the
    // board refusals are decided before the key store is touched.
    let (home, root) = (a_dir(), a_dir());
    holds(home.path(), root.path(), &["*"]);

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
    holds(unbound_home.path(), unbound_root.path(), &["data-plane"]);
    let unbound = resolve_filing(&manifest, unbound_root.path(), unbound_home.path(), None)
        .expect_err("this checkout is bound to no key");
    assert!(
        matches!(
            &unbound,
            Error::Key {
                source: route::Error::Unbound { .. }
            }
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
            &sigils_path(unbound_home.path(), unbound_root.path())
                .display()
                .to_string()
        ),
        "{unbound}"
    );

    let (dangling_home, dangling_root) = (a_dir(), a_dir());
    holds(dangling_home.path(), dangling_root.path(), &["data-plane"]);
    save_key_binding(dangling_home.path(), dangling_root.path(), "work").expect("binds");
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
