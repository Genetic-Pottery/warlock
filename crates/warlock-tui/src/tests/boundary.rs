use std::path::PathBuf;

use warlock_engine::{Manifest, PactEntry};

use super::{Reach, Verdict, verdict};
use crate::error::Error;

// Never read: nothing in this module touches a disk.
const ROOT: &str = "/repo";

fn pact(module: &str, scope: Option<&str>) -> PactEntry {
    let entry = PactEntry::new(ROOT, module, format!("{module}/WARLOCK.md"))
        .expect("a relative module path is inside the root");
    match scope {
        Some(scope) => entry.with_scope(scope),
        None => entry,
    }
}

fn pacts(entries: &[(&str, Option<&str>)]) -> Manifest {
    Manifest::with_entries(entries.iter().map(|(module, scope)| pact(module, *scope)))
}

fn at(module: &str) -> PathBuf {
    PathBuf::from(ROOT).join(module)
}

fn asked(manifest: &Manifest, module: &str, sigils: &[&str], reach: Reach) -> Verdict {
    let held: Vec<String> = sigils.iter().map(|sigil| (*sigil).to_owned()).collect();
    verdict(&at(module), &PathBuf::from(ROOT), manifest, &held, reach)
}

#[test]
fn a_directory_nothing_covers_is_open_to_anyone() {
    let manifest = pacts(&[("crates", None)]);

    assert_eq!(
        asked(&manifest, "crates", &[], Reach::Here),
        Verdict::Open,
        "the permissive default sits on the directory, not on the operator"
    );
}

#[test]
fn a_scope_no_held_sigil_opens_closes_the_directory() {
    let manifest = pacts(&[("crates", Some("platform"))]);

    assert_eq!(
        asked(&manifest, "crates", &["web"], Reach::Here),
        Verdict::Closed {
            scope: "platform".to_owned()
        },
        "holding the wrong sigil is refused"
    );
    assert_eq!(
        asked(&manifest, "crates", &[], Reach::Here),
        Verdict::Closed {
            scope: "platform".to_owned()
        },
        "and so is holding none at all"
    );
    assert_eq!(
        asked(&manifest, "crates", &["platform"], Reach::Here),
        Verdict::Open,
        "the sigil that names the scope opens it"
    );
}

#[test]
fn a_scope_covers_everything_beneath_it() {
    let manifest = pacts(&[("crates", Some("platform")), ("crates/engine", None)]);

    assert_eq!(
        asked(&manifest, "crates/engine", &[], Reach::Here),
        Verdict::Closed {
            scope: "platform".to_owned()
        },
        "a directory that says nothing takes the nearest scope above it"
    );
}

#[test]
fn asking_about_here_says_nothing_about_what_is_underneath() {
    // The directory itself is open; the pact under it is not.
    let manifest = pacts(&[("crates", None), ("crates/engine", Some("platform"))]);

    assert_eq!(
        asked(&manifest, "crates", &[], Reach::Here),
        Verdict::Open,
        "`p`, `r` and `s` touch one directory and ask about one directory"
    );
    assert_eq!(
        asked(&manifest, "crates", &[], Reach::HereAndBelow),
        Verdict::ClosedBelow {
            scopes: vec!["platform".to_owned()]
        },
        "an un-pact drops the pacts underneath, so it has to ask about them"
    );
}

#[test]
fn a_boundary_underneath_that_is_held_is_no_obstacle() {
    let manifest = pacts(&[("crates", None), ("crates/engine", Some("platform"))]);

    assert_eq!(
        asked(&manifest, "crates", &["platform"], Reach::HereAndBelow),
        Verdict::Open,
        "the rule refuses over scopes not held, not over having any"
    );
}

#[test]
fn the_scope_above_is_answered_before_the_scopes_below() {
    // Closed both ways at once: a scope on the directory itself, and a
    // different one on a pact beneath it.
    let manifest = pacts(&[
        ("crates", Some("platform")),
        ("crates/engine", Some("data-plane")),
    ]);

    assert_eq!(
        asked(&manifest, "crates", &[], Reach::HereAndBelow),
        Verdict::Closed {
            scope: "platform".to_owned()
        },
        "an operator who may not act here at all does not need a list of \
             what is underneath — and this ordering used to live in two callers"
    );
}

#[test]
fn both_doors_say_the_same_sentence_about_one_verdict() {
    let closed = Verdict::Closed {
        scope: "platform".to_owned(),
    };
    let panel = closed
        .message("crates")
        .expect("a refusing verdict has a line");
    let shell = Error::ClosedScope {
        path: "crates".to_owned(),
        scope: "platform".to_owned(),
    }
    .to_string();

    assert_eq!(
        panel, shell,
        "the footer and the shell word one refusal once"
    );
}

#[test]
fn an_open_verdict_has_nothing_to_say() {
    assert_eq!(
        Verdict::Open.message("crates"),
        None,
        "a verdict that permits puts nothing on the footer"
    );
}
