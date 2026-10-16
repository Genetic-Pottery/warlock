use std::fs;
use std::mem::discriminant;
use std::path::Path;

use super::{Error, resolve_route, route_facts};
use crate::config::{config_path, save_key_binding, save_open_scopes};
use crate::keys::{keys_path, save_key};
use crate::manifest::{Manifest, ScopeRecord, ScopedDirectory, manifest_path};
use crate::scope::scope_opens_to;

// The string every test that must not see a key looks for. It is a plausible
// Linear key rather than the word "secret", so a test that passes proves the
// redaction and not an unlucky choice of needle.
const KEY: &str = "lin_api_9f3c0d11aa47";

fn a_dir() -> tempfile::TempDir {
    tempfile::tempdir().expect("a temporary directory")
}

fn scoped(path: &str, scope: &str) -> ScopedDirectory {
    ScopedDirectory::new(".", path, scope).expect("a relative path is inside the root")
}

fn a_manifest() -> Manifest {
    Manifest::with_directories([scoped("crates", "data-plane")]).with_scopes([ScopeRecord::new(
        "data-plane",
        "Data Plane",
        "In Review",
        "area/data-plane",
    )])
}

// A checkout that is bound to a key this machine really holds: everything the
// key half of a route needs, so a test about the scope half fails for its own
// reason rather than at `Unbound`.
fn bound(home: &Path, root: &Path) {
    save_key_binding(home, root, "data-plane", "work").expect("binds that scope to a key");
    save_key(home, "work", KEY).expect("stores that key on this machine");
}

fn owned(open: &[&str]) -> Vec<String> {
    open.iter().map(|scope| (*scope).to_owned()).collect()
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
fn a_scope_that_is_not_open_on_this_machine_still_routes() {
    let (home, root) = (a_dir(), a_dir());
    bound(home.path(), root.path());
    save_open_scopes(home.path(), root.path(), &[]).expect("records an empty set of open scopes");
    let manifest = a_manifest();

    let route = resolve_route("crates/engine/src", root.path(), &manifest, home.path())
        .expect("a scoped path on a bound checkout resolves, open or not");

    assert_eq!(route.scope(), "data-plane");
    assert_eq!(route.record().name(), "data-plane");
    assert_eq!(route.record().team_key(), "Data Plane");
    assert_eq!(route.record().review_state(), "In Review");
    assert_eq!(route.record().label(), "area/data-plane");
    assert_eq!(route.key(), "work");
    assert!(
        !route.opens(),
        "opening nothing opens nothing that is scoped, and that is a fact \
         beside the route rather than a refusal"
    );
    assert_eq!(route.opens(), scope_opens_to(Some("data-plane"), &[]));
    assert!(
        !format!("{route:?}").contains(KEY),
        "a route carries the key's name and never its value"
    );
}

#[test]
fn the_open_fact_is_scope_opens_to_and_never_changes_the_route() {
    let manifest = a_manifest();

    for open in [
        owned(&[]),
        owned(&["web"]),
        owned(&["data-plane"]),
        owned(&["*"]),
        owned(&["web", "data-plane"]),
        owned(&["web", "*"]),
    ] {
        let (home, root) = (a_dir(), a_dir());
        bound(home.path(), root.path());
        save_open_scopes(home.path(), root.path(), &open).expect("records what this machine opens");

        let route = resolve_route("crates/engine", root.path(), &manifest, home.path())
            .expect("a scoped path routes whatever this machine opens");

        assert_eq!(
            route.opens(),
            scope_opens_to(Some("data-plane"), &open),
            "the fact is the rule's answer and not a second copy of it: {open:?}"
        );
        assert_eq!(route.scope(), "data-plane", "{open:?}");
        assert_eq!(route.record().team_key(), "Data Plane", "{open:?}");
        assert_eq!(route.record().review_state(), "In Review", "{open:?}");
        assert_eq!(route.record().label(), "area/data-plane", "{open:?}");
        assert_eq!(
            route.key(),
            "work",
            "a closed scope files against the same team, state and label as an \
             open one: {open:?}"
        );
    }
}

#[test]
fn an_unscoped_path_is_refused_by_name_whatever_this_machine_opens() {
    // Work with nowhere to be filed: a machine that opens the wildcard is turned
    // down in the same words as one that opens nothing, because an unscoped
    // directory is closed even to the wildcard and has no board either way.
    for open in [owned(&[]), owned(&["*"]), owned(&["data-plane"])] {
        assert!(!scope_opens_to(None, &open), "{open:?}");

        let (home, root) = (a_dir(), a_dir());
        bound(home.path(), root.path());
        save_open_scopes(home.path(), root.path(), &open).expect("records what this machine opens");

        let error = resolve_route("docs/adr", root.path(), &a_manifest(), home.path())
            .expect_err("nothing at or above `docs/adr` carries a scope");

        assert!(
            matches!(&error, Error::Unscoped { path } if path == Path::new("docs/adr")),
            "{error:?} for {open:?}"
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

    let unrecorded_manifest = Manifest::with_directories([scoped("crates", "data-plane")]);
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
    save_open_scopes(
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
    .expect_err("the covering scope is bound to no key");
    assert!(
        matches!(&unbound, Error::Unbound { scope, .. } if scope == "data-plane"),
        "{unbound:?}"
    );
    let text = unbound.to_string();
    let config = config_path(unbound_home.path(), unbound_root.path());
    assert!(
        text.contains(&config.display().to_string()) && text.contains("warlock key use data-plane"),
        "the fix is one line for one scope in that file: {text}"
    );

    // A checkout nobody has configured at all is the same refusal, against the
    // config file that would hold the binding: a missing config is a checkout
    // bound to nothing, and it never becomes a different answer.
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
    let (home, root) = (dangling_home.path(), dangling_root.path());
    save_key_binding(home, root, "data-plane", "work").expect("binds");
    let dangling = resolve_route("crates/engine", root, &manifest, home)
        .expect_err("the bound name is in no key store");
    assert!(
        matches!(&dangling, Error::Dangling { scope, .. } if scope == "data-plane"),
        "{dangling:?}"
    );
    let text = dangling.to_string();
    let store = keys_path(home);
    assert!(
        text.contains("`work`")
            && text.contains("`data-plane`")
            && text.contains(&store.display().to_string())
            && text.contains("warlock key add"),
        "{text}"
    );

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
fn a_recorded_scope_and_a_stored_key_report_every_fact() {
    let (home, root) = (a_dir(), a_dir());
    bound(home.path(), root.path());
    save_open_scopes(home.path(), root.path(), &owned(&["data-plane"])).expect("opens a scope");
    let manifest = a_manifest();

    let facts = route_facts(
        "crates/engine/src",
        root.path(),
        &manifest,
        Some(home.path()),
    )
    .expect("the reporting form refuses nothing an answer can be given for");

    assert_eq!(facts.scope(), Some("data-plane"));
    let record = facts
        .record()
        .expect("the scope carries a `[[scope]]` record");
    assert_eq!(record.team_key(), "Data Plane");
    assert_eq!(record.review_state(), "In Review");
    assert_eq!(record.label(), "area/data-plane");
    assert_eq!(facts.key(), Some("work"));
    assert!(facts.stored());
    assert!(
        !format!("{facts:?}").contains(KEY),
        "the facts carry the key's name and never its value"
    );
}

#[test]
fn a_covering_scope_with_no_record_still_reports_the_scope_and_the_key() {
    let (home, root) = (a_dir(), a_dir());
    bound(home.path(), root.path());
    // The same manifest the `Unrecorded` refusal is built on: a scope written on
    // a directory that no `[[scope]]` record names.
    let manifest = Manifest::with_directories([scoped("crates", "data-plane")]);

    let facts = route_facts("crates/engine", root.path(), &manifest, Some(home.path()))
        .expect("a scope nobody recorded is an absent record, not a refusal");

    assert_eq!(facts.scope(), Some("data-plane"));
    assert!(facts.record().is_none());
    assert_eq!(
        facts.key(),
        Some("work"),
        "the refusing form never reaches the binding on this input, which is \
         the reason this function exists"
    );
    assert!(facts.stored());

    // And the wrapper still turns exactly this into `Unrecorded`.
    let error = resolve_route("crates/engine", root.path(), &manifest, home.path())
        .expect_err("no `[[scope]]` record carries that name");
    assert!(matches!(error, Error::Unrecorded { .. }), "{error:?}");
}

#[test]
fn a_path_nothing_covers_reports_no_scope_and_no_key() {
    // A key is bound to a scope, so a path outside every scope has no binding
    // to report: naming whatever some other scope is bound to would put a key
    // beside a path it has nothing to do with.
    let (home, root) = (a_dir(), a_dir());
    bound(home.path(), root.path());
    let manifest = a_manifest();

    let facts = route_facts("docs/adr", root.path(), &manifest, Some(home.path()))
        .expect("nothing to route to is an answer here");

    assert_eq!(facts.scope(), None);
    assert!(facts.record().is_none());
    assert_eq!(facts.key(), None);
    assert!(!facts.stored());
}

#[test]
fn two_scopes_in_one_repository_route_to_two_different_keys() {
    // The repository this ticket exists for: two scopes filing to boards in two
    // Linear workspaces, each path resolving the key of the scope that covers
    // it and not the one that happens to be bound first.
    let (home, root) = (a_dir(), a_dir());
    let manifest =
        Manifest::with_directories([scoped("crates", "data-plane"), scoped("site", "web")])
            .with_scopes([
                ScopeRecord::new("data-plane", "Data Plane", "In Review", "area/data-plane"),
                ScopeRecord::new("web", "Web", "In Review", "area/web"),
            ]);

    save_key_binding(home.path(), root.path(), "data-plane", "work").expect("binds one scope");
    save_key_binding(home.path(), root.path(), "web", "personal").expect("binds the other");
    save_key(home.path(), "work", KEY).expect("stores one key");
    save_key(home.path(), "personal", "lin_api_7b21e4c0fd93").expect("stores the other");

    let here = resolve_route("crates/engine/src", root.path(), &manifest, home.path())
        .expect("a scoped path on a bound scope resolves");
    let there = resolve_route("site/pages", root.path(), &manifest, home.path())
        .expect("and so does one under the other scope");

    assert_eq!(here.scope(), "data-plane");
    assert_eq!(here.key(), "work");
    assert_eq!(there.scope(), "web");
    assert_eq!(there.key(), "personal");
    assert_ne!(
        here.key(),
        there.key(),
        "one checkout, two scopes, two keys: the whole reason the binding is \
         a map"
    );

    // And a third scope bound by nobody is refused by name while these two
    // still route: there is no fallback to either of them.
    let unbound = resolve_route("crates/engine", root.path(), &manifest, home.path());
    assert!(unbound.is_ok(), "{unbound:?}");
    let error = resolve_route(
        "billing/src",
        root.path(),
        &Manifest::with_directories([scoped("billing", "billing")]).with_scopes([
            ScopeRecord::new("billing", "Billing", "In Review", "area/billing"),
        ]),
        home.path(),
    )
    .expect_err("nothing binds `billing`");
    assert!(
        matches!(&error, Error::Unbound { scope, .. } if scope == "billing"),
        "{error:?}"
    );
}

#[test]
fn a_checkout_bound_to_nothing_reports_no_key() {
    let (home, root) = (a_dir(), a_dir());
    save_open_scopes(home.path(), root.path(), &owned(&["data-plane"]))
        .expect("a configured checkout that is bound to nothing");
    save_key(home.path(), "work", KEY).expect("a key this machine holds, bound to no checkout");
    let manifest = a_manifest();

    let facts = route_facts("crates/engine", root.path(), &manifest, Some(home.path()))
        .expect("an unbound checkout still routes as far as the scope");

    assert_eq!(facts.scope(), Some("data-plane"));
    assert_eq!(facts.key(), None);
    assert!(
        !facts.stored(),
        "a checkout bound to no name resolves no name, whatever is in the store"
    );

    // A checkout nobody configured at all reads the same, and so does a caller
    // that could resolve no home: each of the three is `warlock key use`, and a
    // person with no home has no key store to bind against either.
    let (fresh_home, fresh_root) = (a_dir(), a_dir());
    for facts in [
        route_facts(
            "crates/engine",
            fresh_root.path(),
            &manifest,
            Some(fresh_home.path()),
        ),
        route_facts("crates/engine", fresh_root.path(), &manifest, None),
    ] {
        let facts = facts.expect("no config and no home are both answers");
        assert_eq!(facts.scope(), Some("data-plane"));
        assert_eq!(facts.key(), None);
        assert!(!facts.stored());
    }
}

#[test]
fn a_bound_name_that_is_in_no_store_is_reported_by_name() {
    let (home, root) = (a_dir(), a_dir());
    save_key_binding(home.path(), root.path(), "data-plane", "work").expect("binds that scope");
    save_key(home.path(), "personal", KEY).expect("stores a different name's key");
    let manifest = a_manifest();

    let facts = route_facts("crates/engine", root.path(), &manifest, Some(home.path()))
        .expect("a name in no store is a fact about the name");

    assert_eq!(
        facts.key(),
        Some("work"),
        "the name is what a person types at `warlock key add`"
    );
    assert!(!facts.stored());
    assert!(
        !format!("{facts:?}").contains(KEY),
        "a stored key belonging to another name reached the facts: {facts:?}"
    );

    // An empty store and no store at all are the same answer as the wrong name.
    let (bare_home, bare_root) = (a_dir(), a_dir());
    save_key_binding(bare_home.path(), bare_root.path(), "data-plane", "work")
        .expect("binds that scope");
    let facts = route_facts(
        "crates/engine",
        bare_root.path(),
        &manifest,
        Some(bare_home.path()),
    )
    .expect("a machine with no key store has stored no names");
    assert_eq!(facts.key(), Some("work"));
    assert!(!facts.stored());
}

#[test]
fn a_dangling_binding_names_the_key_and_never_a_key_value() {
    let (home, root) = (a_dir(), a_dir());
    save_key_binding(home.path(), root.path(), "data-plane", "work").expect("binds that scope");
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
