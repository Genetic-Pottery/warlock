use super::{and_listed, listed};

#[test]
fn a_single_item_is_named_alone() {
    assert_eq!(listed(&["WAR-1"]), "`WAR-1`");
    assert_eq!(and_listed(&["## Outcome".to_owned()]), "## Outcome");
}

#[test]
fn nothing_named_is_the_empty_string() {
    assert_eq!(listed::<&str>(&[]), "");
    assert_eq!(and_listed(&[]), "");
}

#[test]
fn a_backticked_list_is_separated_by_commas_throughout() {
    assert_eq!(
        listed(&["WAR-1", "WAR-2", "WAR-3"]),
        "`WAR-1`, `WAR-2`, `WAR-3`"
    );
}

#[test]
fn the_last_of_a_sentence_is_joined_with_and_and_no_comma() {
    let sections = ["## Outcome".to_owned(), "## Scope".to_owned()];
    assert_eq!(and_listed(&sections), "## Outcome and ## Scope");

    let three = ["`--a`".to_owned(), "`--b`".to_owned(), "`--c`".to_owned()];
    assert_eq!(and_listed(&three), "`--a`, `--b` and `--c`");
}
