use std::path::PathBuf;

use warlock_engine::{Manifest, ScopedDirectory};

use super::{Verdict, permits, unscoped_message};
use crate::error::Error;

// Never read: nothing in this module touches a disk.
const ROOT: &str = "/repo";

fn scoped(directories: &[(&str, &str)]) -> Manifest {
    Manifest::with_directories(directories.iter().map(|(module, scope)| {
        ScopedDirectory::new(ROOT, module, *scope)
            .expect("a relative module path is inside the root")
    }))
}

fn at(module: &str) -> PathBuf {
    PathBuf::from(ROOT).join(module)
}

fn asked(manifest: &Manifest, module: &str, sigils: &[&str]) -> Verdict {
    let held: Vec<String> = sigils.iter().map(|sigil| (*sigil).to_owned()).collect();
    permits(&at(module), &PathBuf::from(ROOT), manifest, &held)
}

#[test]
fn a_directory_nothing_covers_is_open_to_anyone() {
    let manifest = scoped(&[("assets", "assets")]);

    assert_eq!(
        asked(&manifest, "crates", &[]),
        Verdict::Open,
        "anyone may draw the first scope over a directory nothing covers"
    );
}

#[test]
fn a_scope_no_held_sigil_opens_closes_the_directory() {
    let manifest = scoped(&[("crates", "platform")]);

    assert_eq!(
        asked(&manifest, "crates", &["web"]),
        Verdict::Closed {
            scope: "platform".to_owned()
        },
        "holding the wrong sigil is refused"
    );
    assert_eq!(
        asked(&manifest, "crates", &[]),
        Verdict::Closed {
            scope: "platform".to_owned()
        },
        "and so is holding none at all"
    );
    assert_eq!(
        asked(&manifest, "crates", &["platform"]),
        Verdict::Open,
        "the sigil that names the scope opens it"
    );
}

#[test]
fn a_scope_covers_everything_beneath_it() {
    let manifest = scoped(&[("crates", "platform")]);

    assert_eq!(
        asked(&manifest, "crates/engine", &[]),
        Verdict::Closed {
            scope: "platform".to_owned()
        },
        "a directory that says nothing takes the nearest scope above it"
    );
}

#[test]
fn asking_about_here_says_nothing_about_what_is_underneath() {
    let manifest = scoped(&[("crates/engine", "platform")]);

    assert_eq!(
        asked(&manifest, "crates", &[]),
        Verdict::Open,
        "a scope beneath a directory does not close the directory"
    );
}

#[test]
fn the_nearest_scope_answers_over_the_one_above_it() {
    let manifest = scoped(&[("crates", "platform"), ("crates/engine", "data-plane")]);

    assert_eq!(
        asked(&manifest, "crates/engine", &["platform"]),
        Verdict::Closed {
            scope: "data-plane".to_owned()
        },
        "holding the outer scope does not open a directory scoped inside it"
    );
    assert_eq!(
        asked(&manifest, "crates/engine", &["data-plane"]),
        Verdict::Open,
    );
}

#[test]
fn the_unscoped_sentence_names_the_way_to_draw_a_scope() {
    let shell = Error::Unscoped {
        path: "assets".to_owned(),
    }
    .to_string();

    assert_eq!(
        shell,
        unscoped_message("assets"),
        "the shell words an unscoped path the way the boundary does"
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
