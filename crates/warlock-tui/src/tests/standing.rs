use std::path::{Path, PathBuf};

use super::{FOR_CHECK, Standing};
use crate::error::Error;

fn standing(root: &Path) -> Standing {
    Standing::at(root.to_path_buf(), root.to_path_buf())
}

#[test]
fn a_relative_argument_is_joined_onto_the_working_directory_and_not_the_root() {
    let standing = Standing::at(PathBuf::from("/repo/crates"), PathBuf::from("/repo"));

    assert_eq!(
        standing.target("engine"),
        PathBuf::from("/repo/crates/engine"),
        "a relative path is the reader's, spelled from where they are standing"
    );
}

#[test]
fn an_absolute_argument_is_taken_as_it_was_typed() {
    let standing = Standing::at(PathBuf::from("/repo/crates"), PathBuf::from("/repo"));

    assert_eq!(standing.target("/elsewhere"), PathBuf::from("/elsewhere"));
}

#[test]
fn a_repository_that_never_pacted_anything_stands_on_an_empty_manifest() {
    let dir = tempfile::tempdir().expect("a temporary directory");

    let manifest = standing(dir.path())
        .manifest()
        .expect("a missing manifest reads as an empty one");

    assert!(
        manifest.entries().is_empty(),
        "nothing on disk and nothing pacted are the same thing to answer about"
    );
}

#[test]
fn a_manifest_that_will_not_parse_is_an_error_rather_than_an_empty_start() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let warlock = dir.path().join(".warlock");
    std::fs::create_dir_all(&warlock).expect("the bookkeeping directory");
    std::fs::write(warlock.join("pacts.toml"), "this is not toml {{{").expect("a broken file");

    standing(dir.path())
        .manifest()
        .expect_err("a manifest that exists and will not parse is a failure");
}

#[test]
fn the_root_itself_spells_as_a_dot() {
    let standing = Standing::at(PathBuf::from("/repo"), PathBuf::from("/repo"));

    assert_eq!(
        standing
            .spelled(Path::new("/repo"))
            .expect("the root spells"),
        ".",
        "the manifest's own spelling of the root"
    );
}

#[test]
fn a_path_outside_the_repository_has_no_spelling() {
    let standing = Standing::at(PathBuf::from("/repo"), PathBuf::from("/repo"));

    standing
        .spelled(Path::new("/elsewhere"))
        .expect_err("nothing outside the repository has a manifest form");
}

#[test]
fn a_missing_repository_names_what_the_subcommand_wanted_one_for() {
    // `here` reads the process's own directory, so what is pinned here is
    // the sentence rather than the walk: every tail is one of the nine
    // above, and `NoRepository` carries the one it was given.
    let error = Error::NoRepository {
        start: PathBuf::from("/tmp"),
        wanted: FOR_CHECK,
    };

    assert!(
        error.to_string().contains(FOR_CHECK),
        "the tail is what tells a reader which command could not be answered: {error}"
    );
}
