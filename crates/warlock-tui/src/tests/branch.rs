use warlock_engine::{held_base_branch, load_key_binding, save_key_binding};

use super::set;
use crate::error::Error;

fn a_dir() -> tempfile::TempDir {
    tempfile::tempdir().expect("a temporary directory")
}

fn text(out: Vec<u8>) -> String {
    String::from_utf8(out).expect("warlock writes its own text")
}

#[test]
fn use_sets_the_base_and_says_where_it_was_written() {
    let (home, root) = (a_dir(), a_dir());
    let mut out = Vec::new();

    set(home.path(), root.path(), Some("develop"), &mut out).expect("sets the base");

    assert_eq!(
        held_base_branch(home.path(), root.path())
            .expect("reads")
            .as_deref(),
        Some("develop")
    );
    let said = text(out);
    assert!(said.contains("`develop`"), "{said}");
    assert!(said.contains("config.toml"), "{said}");
}

#[test]
fn clear_goes_back_to_the_remotes_default() {
    let (home, root) = (a_dir(), a_dir());
    set(home.path(), root.path(), Some("develop"), &mut Vec::new()).expect("sets the base");
    let mut out = Vec::new();

    set(home.path(), root.path(), None, &mut out).expect("clears the base");

    assert_eq!(
        held_base_branch(home.path(), root.path()).expect("reads"),
        None
    );
    assert!(text(out).contains("the remote's default branch"));
}

#[test]
fn setting_the_base_keeps_the_key_binding() {
    let (home, root) = (a_dir(), a_dir());
    save_key_binding(home.path(), root.path(), "work").expect("binds a key");

    set(home.path(), root.path(), Some("develop"), &mut Vec::new()).expect("sets the base");

    assert_eq!(
        load_key_binding(home.path(), root.path())
            .expect("reads")
            .as_deref(),
        Some("work")
    );
}

#[test]
fn a_name_with_a_space_is_refused_and_nothing_is_written() {
    let (home, root) = (a_dir(), a_dir());
    let mut out = Vec::new();

    let refused = set(home.path(), root.path(), Some("my branch"), &mut out);

    assert!(matches!(refused, Err(Error::Sigils { .. })), "{refused:?}");
    assert!(out.is_empty());
    assert_eq!(
        held_base_branch(home.path(), root.path()).expect("reads"),
        None
    );
}
