use std::path::Path;

use warlock_engine::{held_sigils, save_sigils};

use super::{Change, change_sigils, holding};
use crate::error::Error;

fn names(names: &[&str]) -> Vec<String> {
    names.iter().map(|name| (*name).to_owned()).collect()
}

fn held(home: &Path, root: &Path) -> Vec<String> {
    held_sigils(home, root).expect("the config reads")
}

#[test]
fn add_keeps_what_is_held_folds_case_and_skips_repeats() {
    let (home, root) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    save_sigils(home.path(), root.path(), &names(&["docs"])).unwrap();

    let added = change_sigils(
        home.path(),
        root.path(),
        Change::Add(&names(&["Warlock-Team", "docs", "*"])),
    )
    .expect("every name is a sigil");

    assert_eq!(added, names(&["docs", "warlock-team", "*"]));
    assert_eq!(held(home.path(), root.path()), added);
}

#[test]
fn one_bad_name_writes_nothing() {
    let (home, root) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    save_sigils(home.path(), root.path(), &names(&["docs"])).unwrap();

    let refused = change_sigils(
        home.path(),
        root.path(),
        Change::Add(&names(&["fine", "not ok"])),
    );

    assert!(matches!(refused, Err(Error::Sigil { .. })), "{refused:?}");
    assert_eq!(held(home.path(), root.path()), names(&["docs"]));
}

#[test]
fn remove_takes_out_what_is_held_and_refuses_what_is_not() {
    let (home, root) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    save_sigils(home.path(), root.path(), &names(&["docs", "warlock-team"])).unwrap();

    let refused = change_sigils(
        home.path(),
        root.path(),
        Change::Remove(&names(&["docs", "elsewhere"])),
    );
    assert!(
        matches!(&refused, Err(Error::UnheldSigil { sigil }) if sigil == "elsewhere"),
        "{refused:?}"
    );
    assert_eq!(
        held(home.path(), root.path()),
        names(&["docs", "warlock-team"])
    );

    let removed =
        change_sigils(home.path(), root.path(), Change::Remove(&names(&["DOCS"]))).unwrap();
    assert_eq!(removed, names(&["warlock-team"]));
}

#[test]
fn clear_holds_nothing() {
    let (home, root) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    save_sigils(home.path(), root.path(), &names(&["docs"])).unwrap();

    assert_eq!(
        change_sigils(home.path(), root.path(), Change::Clear).unwrap(),
        Vec::<String>::new()
    );
    assert_eq!(held(home.path(), root.path()), Vec::<String>::new());
}

#[test]
fn holding_names_each_sigil_or_nothing() {
    assert_eq!(holding(&[]), "nothing");
    assert_eq!(holding(&names(&["a", "b"])), "`a`, `b`");
}
