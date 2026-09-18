use std::fs;
use std::mem::discriminant;
use std::path::Path;

use super::{Error, resolve_route};
use crate::keys::{keys_path, save_key};
use crate::manifest::{Manifest, PactEntry, ScopeRecord, manifest_path};
use crate::scope::scope_opens_to;
use crate::sigils::{save_key_binding, save_sigils, sigils_path};

// The string every test that must not see a key looks for. It is a plausible
// Linear key rather than the word "secret", so a test that passes proves the
// redaction and not an unlucky choice of needle.
const KEY: &str = "lin_api_9f3c0d11aa47";

fn a_dir() -> tempfile::TempDir {
    tempfile::tempdir().expect("a temporary directory")
}

fn entry(module: &str) -> PactEntry {
    PactEntry::new(".", module, format!("{module}/WARLOCK.md"))
        .expect("a relative module path is inside the root")
}

fn a_manifest() -> Manifest {
    Manifest::with_entries([entry("crates").with_scope("data-plane"), entry("docs")]).with_scopes([
        ScopeRecord::new("data-plane", "Data Plane", "In Review", "area/data-plane"),
    ])
}

// A checkout that is bound to a key this machine really holds: everything the
// key half of a route needs, so a test about the scope half fails for its own
// reason rather than at `Unbound`.
fn bound(home: &Path, root: &Path) {
    save_key_binding(home, root, "work").expect("binds this checkout to a key");
    save_key(home, "work", KEY).expect("stores that key on this machine");
}

fn owned(sigils: &[&str]) -> Vec<String> {
    sigils.iter().map(|sigil| (*sigil).to_owned()).collect()
}

fn rendered(error: &Error) -> String {
    use std::error::Error as _;
    use std::fmt::Write as _;

    let mut text = format!("{error} | {error:?}");
    let mut cause = error.source();
    while let Some(source) = cause {
        write!(text, " | {source} | {source:?}").expect("a string never fails to grow");
        cause = source.source();
    }
    text
}

#[test]
fn a_scope_this_machine_holds_no_sigil_for_still_routes() {
    let (home, root) = (a_dir(), a_dir());
    bound(home.path(), root.path());
    save_sigils(home.path(), root.path(), &[]).expect("records an empty sigil set");
    let manifest = a_manifest();

    let route = resolve_route("crates/engine/src", root.path(), &manifest, home.path())
        .expect("a scoped path on a bound checkout resolves, held or not");

    assert_eq!(route.scope(), "data-plane");
    assert_eq!(route.record().name(), "data-plane");
    assert_eq!(route.record().team(), "Data Plane");
    assert_eq!(route.record().review_state(), "In Review");
    assert_eq!(route.record().label(), "area/data-plane");
    assert_eq!(route.key(), "work");
    assert!(
        !route.opens(),
        "holding nothing opens nothing that is scoped, and that is a fact \
         beside the route rather than a refusal"
    );
    assert_eq!(route.opens(), scope_opens_to(Some("data-plane"), &[]));
    assert!(
        !format!("{route:?}").contains(KEY),
        "a route carries the key's name and never its value"
    );
}

#[test]
fn the_sigil_fact_is_scope_opens_to_and_never_changes_the_route() {
    let manifest = a_manifest();

    for held in [
        owned(&[]),
        owned(&["web"]),
        owned(&["data-plane"]),
        owned(&["*"]),
        owned(&["web", "data-plane"]),
        owned(&["web", "*"]),
    ] {
        let (home, root) = (a_dir(), a_dir());
        bound(home.path(), root.path());
        save_sigils(home.path(), root.path(), &held).expect("records what this machine holds");

        let route = resolve_route("crates/engine", root.path(), &manifest, home.path())
            .expect("a scoped path routes whatever this machine holds");

        assert_eq!(
            route.opens(),
            scope_opens_to(Some("data-plane"), &held),
            "the fact is the rule's answer and not a second copy of it: {held:?}"
        );
        assert_eq!(route.scope(), "data-plane", "{held:?}");
        assert_eq!(route.record().team(), "Data Plane", "{held:?}");
        assert_eq!(route.record().review_state(), "In Review", "{held:?}");
        assert_eq!(route.record().label(), "area/data-plane", "{held:?}");
        assert_eq!(
            route.key(),
            "work",
            "a closed scope files against the same team, state and label as an \
             open one: {held:?}"
        );
    }
}

#[test]
fn an_unscoped_path_is_refused_by_name_whatever_this_machine_holds() {
    // `scope_opens_to` calls an unscoped directory open to anyone, and it still
    // does here. What is refused is not a boundary being crossed but work with
    // nowhere to be filed: a machine holding the wildcard is turned down in the
    // same words as one holding nothing, which is what "reported, not gated"
    // costs on this side.
    for held in [owned(&[]), owned(&["*"]), owned(&["data-plane"])] {
        assert!(scope_opens_to(None, &held), "{held:?}");

        let (home, root) = (a_dir(), a_dir());
        bound(home.path(), root.path());
        save_sigils(home.path(), root.path(), &held).expect("records what this machine holds");

        let error = resolve_route("docs/adr", root.path(), &a_manifest(), home.path())
            .expect_err("nothing at or above `docs/adr` carries a scope");

        assert!(
            matches!(&error, Error::Unscoped { path } if path == Path::new("docs/adr")),
            "{error:?} for {held:?}"
        );
        assert!(error.to_string().contains("docs/adr"), "{error}");
    }
}

#[test]
fn the_four_refusals_are_four_different_variants() {
    let manifest = a_manifest();

    let (home, root) = (a_dir(), a_dir());
    bound(home.path(), root.path());
    let unscoped = resolve_route("docs/adr", root.path(), &manifest, home.path())
        .expect_err("nothing scopes `docs`");
    assert!(matches!(unscoped, Error::Unscoped { .. }), "{unscoped:?}");
    assert!(unscoped.to_string().contains("docs/adr"), "{unscoped}");

    let unrecorded_manifest = Manifest::with_entries([entry("crates").with_scope("data-plane")]);
    let unrecorded = resolve_route(
        "crates/engine",
        root.path(),
        &unrecorded_manifest,
        home.path(),
    )
    .expect_err("no `[[scope]]` record carries that name");
    assert!(
        matches!(unrecorded, Error::Unrecorded { .. }),
        "{unrecorded:?}"
    );
    let text = unrecorded.to_string();
    assert!(text.contains("data-plane"), "{text}");
    assert!(
        text.contains(&manifest_path(root.path()).display().to_string()),
        "the refusal names the file to go and fix: {text}"
    );

    let (unbound_home, unbound_root) = (a_dir(), a_dir());
    save_sigils(
        unbound_home.path(),
        unbound_root.path(),
        &owned(&["data-plane"]),
    )
    .expect("a configured checkout that is bound to nothing");
    let unbound = resolve_route(
        "crates/engine",
        unbound_root.path(),
        &manifest,
        unbound_home.path(),
    )
    .expect_err("this checkout is bound to no key");
    assert!(matches!(unbound, Error::Unbound { .. }), "{unbound:?}");
    let text = unbound.to_string();
    assert!(
        text.contains(
            &sigils_path(unbound_home.path(), unbound_root.path())
                .display()
                .to_string()
        ),
        "{text}"
    );
    assert!(text.contains("warlock key use"), "{text}");

    // A checkout nobody has configured at all is the same refusal, against the
    // config file that would hold the binding: `load_sigils` saying NotFound is
    // a machine that holds nothing, and it never becomes a different answer.
    let (fresh_home, fresh_root) = (a_dir(), a_dir());
    let never_configured = resolve_route(
        "crates/engine",
        fresh_root.path(),
        &manifest,
        fresh_home.path(),
    )
    .expect_err("no config for this checkout at all");
    assert_eq!(
        discriminant(&never_configured),
        discriminant(&unbound),
        "{never_configured:?}"
    );

    let (dangling_home, dangling_root) = (a_dir(), a_dir());
    save_key_binding(dangling_home.path(), dangling_root.path(), "work").expect("binds");
    let dangling = resolve_route(
        "crates/engine",
        dangling_root.path(),
        &manifest,
        dangling_home.path(),
    )
    .expect_err("the bound name is in no key store");
    assert!(matches!(dangling, Error::Dangling { .. }), "{dangling:?}");
    let text = dangling.to_string();
    assert!(text.contains("`work`"), "{text}");
    assert!(
        text.contains(&keys_path(dangling_home.path()).display().to_string()),
        "{text}"
    );
    assert!(text.contains("warlock key add"), "{text}");

    let refusals = [&unscoped, &unrecorded, &unbound, &dangling];
    for (index, one) in refusals.iter().enumerate() {
        for other in &refusals[index + 1..] {
            assert_ne!(
                discriminant(*one),
                discriminant(*other),
                "each of these is fixed in a different file by a different \
                 command, so no two may arrive as one variant: {one:?} and {other:?}"
            );
        }
    }
}

#[test]
fn a_path_outside_the_root_is_not_a_path_nothing_scopes() {
    let (home, root) = (a_dir(), a_dir());
    bound(home.path(), root.path());

    let error = resolve_route("../elsewhere", root.path(), &a_manifest(), home.path())
        .expect_err("a path that escapes the root was never in this repository");

    assert!(
        matches!(error, Error::Path { .. }),
        "adding a scope would not fix this one: {error:?}"
    );
}

#[test]
fn a_dangling_binding_names_the_key_and_never_a_key_value() {
    let (home, root) = (a_dir(), a_dir());
    save_key_binding(home.path(), root.path(), "work").expect("binds this checkout");
    save_key(home.path(), "personal", KEY).expect("stores a different name's key");
    let manifest = a_manifest();

    let error = resolve_route("crates/engine", root.path(), &manifest, home.path())
        .expect_err("the bound name is in no key store");
    assert!(matches!(error, Error::Dangling { .. }), "{error:?}");

    let text = rendered(&error);
    assert!(
        text.contains("`work`") && text.contains("warlock key add work"),
        "the name is what a person types at the fix: {text}"
    );
    assert!(!text.contains(KEY), "a key reached an error: {text}");
    assert!(
        !text.contains("lin_api"),
        "not even the shape of a key: {text}"
    );

    // And a store that will not parse, with the key sitting on the line the
    // parser choked on: what `keys::Unparseable` holds back is not handed out
    // again by being wrapped in a refusal from here.
    fs::write(keys_path(home.path()), format!("work = \"{KEY}\" oops\n"))
        .expect("writes a broken store");
    let error = resolve_route("crates/engine", root.path(), &manifest, home.path())
        .expect_err("a store nobody can parse is broken, not empty");
    assert!(matches!(error, Error::Keys { .. }), "{error:?}");

    let text = rendered(&error);
    assert!(!text.contains(KEY), "a key reached an error chain: {text}");
    assert!(
        !text.contains("lin_api"),
        "not even the shape of a key: {text}"
    );
}
