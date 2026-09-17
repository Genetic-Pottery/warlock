use std::path::{Path, PathBuf};

use warlock_engine::{Manifest, Node, NodeState, PactEntry, Tree};

use super::{Listed, Listing, listed, listed_onto, object, state_word};
use crate::error::Error;
use crate::standing::Standing;
use crate::status_for;

fn listing_of(repo: &Path, listing: Listing, path: Option<&str>, json: bool) -> Vec<String> {
    let standing = Standing::at(repo.to_path_buf(), repo.to_path_buf());
    let mut out = Vec::new();
    listed_onto(&standing, listing, path.map(PathBuf::from), json, &mut out)
        .expect("a listing inside a repository answers");
    String::from_utf8(out)
        .expect("warlock writes its own text")
        .lines()
        .map(str::to_owned)
        .collect()
}

fn a_checkout() -> tempfile::TempDir {
    let repo = tempfile::tempdir().expect("a temporary directory");
    std::fs::create_dir_all(repo.path().join(".git")).expect("a repository marker");
    std::fs::create_dir_all(repo.path().join("src")).expect("a source directory");
    std::fs::write(repo.path().join("src/lib.rs"), "//! a module\n").expect("a source file");
    repo
}

#[test]
fn the_composition_lists_nothing_fresh_in_a_repository_that_never_pacted() {
    let repo = a_checkout();

    assert!(
        listing_of(repo.path(), Listing::Fresh, None, false).is_empty(),
        "nothing was ever granted, so nothing is fresh"
    );
}

#[test]
fn an_omitted_path_means_the_root_and_a_given_one_means_that_directory() {
    let repo = a_checkout();
    // Pacted but never granted, which is what stale means: unpacted is a
    // state of its own and is in neither listing.
    Manifest::with_entries([
        PactEntry::new(".", ".", "WARLOCK.md").expect("the root is inside itself"),
        PactEntry::new(".", "src", "src/WARLOCK.md").expect("`src` is inside the root"),
    ])
    .save(repo.path())
    .expect("a manifest that saves");

    let whole = listing_of(repo.path(), Listing::Stale, None, false);
    let inner = listing_of(repo.path(), Listing::Stale, Some("src"), false);

    // The root's listing reaches `src`, and `src`'s does not reach back up.
    assert!(whole.contains(&".".to_owned()), "{whole:?}");
    assert!(whole.contains(&"src".to_owned()), "{whole:?}");
    assert!(!inner.contains(&".".to_owned()), "{inner:?}");
    assert!(inner.contains(&"src".to_owned()), "{inner:?}");
}

#[test]
fn the_json_listing_is_one_line_whatever_it_found() {
    let repo = a_checkout();

    let lines = listing_of(repo.path(), Listing::Fresh, None, true);

    assert_eq!(lines.len(), 1, "not one object: {lines:?}");
    assert!(
        lines[0].contains("\"directories\":[]"),
        "an empty answer is an empty array and never an absent one: {}",
        lines[0]
    );
}

const REPO: &str = "/repo";

// Built as values rather than loaded from disk: what is under test is the
// filtering and the spelling, and the colouring is the engine's own.
fn a_repository() -> Tree {
    Tree::new(
        Node::new(REPO, "/repo/WARLOCK.md", NodeState::PactedStale).with_children([
            Node::new(
                "/repo/crates",
                "/repo/crates/WARLOCK.md",
                NodeState::PactedFresh,
            )
            .with_children([
                Node::new(
                    "/repo/crates/engine",
                    "/repo/crates/engine/WARLOCK.md",
                    NodeState::PactedStale,
                ),
                Node::new("/repo/crates/scratch", None, NodeState::Unpacted),
            ]),
            Node::new(
                "/repo/docs",
                "/repo/docs/WARLOCK.md",
                NodeState::PactedFresh,
            ),
        ]),
    )
}

fn paths(rows: &[Listed]) -> Vec<&str> {
    rows.iter().map(|row| row.path.as_str()).collect()
}

fn rows(tree: &Tree, listing: Listing) -> Vec<Listed> {
    listed(tree, Path::new(REPO), listing.wanted()).expect("a tree inside its own repository")
}

#[test]
fn the_repository_root_prints_as_a_dot_and_never_as_a_blank_line() {
    // The root can be a pacted module like any other, and `.` is what the
    // manifest calls it. An empty line would be a directory a script could
    // not act on and a reader could not see.
    let tree = a_repository();

    assert_eq!(paths(&rows(&tree, Listing::Stale)), [".", "crates/engine"]);
    assert!(
        paths(&rows(&tree, Listing::Stale))
            .iter()
            .all(|path| !path.is_empty())
    );
}

#[test]
fn a_listing_is_the_state_that_was_asked_for_and_nothing_else() {
    let tree = a_repository();

    assert_eq!(paths(&rows(&tree, Listing::Stale)), [".", "crates/engine"]);
    assert_eq!(paths(&rows(&tree, Listing::Fresh)), ["crates", "docs"]);
    // The unpacted directory is in neither: these two subcommands list
    // pacted directories, and a directory nobody pacted is in no state to
    // report.
    for listing in [Listing::Stale, Listing::Fresh] {
        assert!(!paths(&rows(&tree, listing)).contains(&"crates/scratch"));
    }
}

#[test]
fn a_path_lists_what_is_at_or_below_it_spelled_against_the_repository() {
    // The tree is rooted at the path the reader named; the spelling is
    // still repository-root-relative, so `warlock stale crates` and
    // `warlock stale` call one directory by one name.
    let subtree = Tree::new(
        Node::new(
            "/repo/crates",
            "/repo/crates/WARLOCK.md",
            NodeState::PactedFresh,
        )
        .with_children([Node::new(
            "/repo/crates/engine",
            "/repo/crates/engine/WARLOCK.md",
            NodeState::PactedStale,
        )]),
    );

    assert_eq!(paths(&rows(&subtree, Listing::Stale)), ["crates/engine"]);
    assert_eq!(paths(&rows(&subtree, Listing::Fresh)), ["crates"]);
}

#[test]
fn nothing_in_the_state_asked_about_is_an_empty_answer_rather_than_a_failure() {
    // Every directory is fresh, so `warlock stale` has nothing to say —
    // which is the answer, and the caller prints no lines at all for it.
    let tree = Tree::new(
        Node::new(REPO, "/repo/WARLOCK.md", NodeState::PactedFresh).with_children([Node::new(
            "/repo/docs",
            "/repo/docs/WARLOCK.md",
            NodeState::PactedFresh,
        )]),
    );

    assert_eq!(rows(&tree, Listing::Stale), Vec::new());
    // And the exit contract's own half of that fact: an answer with nothing
    // in it is still an answer warlock reached, so what `main` makes of it
    // is a 0 and not the status it spends on a refusal.
    let answer = listed(&tree, Path::new(REPO), NodeState::PactedStale)
        .map(|listed| assert!(listed.is_empty()));
    assert_eq!(status_for(&answer), 0);

    assert_eq!(paths(&rows(&tree, Listing::Fresh)), [".", "docs"]);
}

#[test]
fn a_directory_with_no_repository_relative_spelling_is_a_refusal() {
    // A tree rooted outside the repository the paths are measured against:
    // listing nothing and exiting 0 would tell a script that nothing over
    // there is stale, which is not something warlock knows.
    let elsewhere = Tree::new(Node::new(
        "/elsewhere",
        "/elsewhere/WARLOCK.md",
        NodeState::PactedStale,
    ));

    let refused = listed(&elsewhere, Path::new(REPO), NodeState::PactedStale).map(|_| ());
    // A question warlock could not answer, so the other status: 1, and
    // never the 0 an empty listing gets.
    assert_eq!(status_for(&refused), 1);

    let error = refused.expect_err("a path outside the repository has no manifest form");
    assert!(
        matches!(error, Error::Unspellable { .. }),
        "the engine's own case was rewrapped: {error:?}"
    );
    assert!(!error.to_string().contains('\n'), "`main` prints one line");
}

#[test]
fn the_object_is_the_shape_a_consumer_was_promised() {
    let tree = Tree::new(Node::new(
        "/repo/crates/engine",
        "/repo/crates/engine/WARLOCK.md",
        NodeState::PactedStale,
    ));

    assert_eq!(
        object(Listing::Stale, &rows(&tree, Listing::Stale)).to_string(),
        r#"{"command":"stale","directories":[{"path":"crates/engine","state":"stale"}]}"#
    );
}

#[test]
fn the_fresh_object_names_itself_fresh_all_the_way_down() {
    let tree = Tree::new(Node::new(
        "/repo/crates/engine",
        "/repo/crates/engine/WARLOCK.md",
        NodeState::PactedFresh,
    ));

    assert_eq!(
        object(Listing::Fresh, &rows(&tree, Listing::Fresh)).to_string(),
        r#"{"command":"fresh","directories":[{"path":"crates/engine","state":"fresh"}]}"#
    );
}

#[test]
fn an_empty_answer_is_an_empty_array_rather_than_a_missing_field() {
    // A consumer indexing into `directories` should not have to spell the
    // empty answer a second way.
    assert_eq!(
        object(Listing::Stale, &[]).to_string(),
        r#"{"command":"stale","directories":[]}"#
    );
}

#[test]
fn no_object_carries_a_root() {
    // Deliberate: an absolute machine path is the one non-reproducible
    // thing in a document meant to be diffable across machines, and it
    // tells a consumer a constant.
    let tree = a_repository();
    for listing in [Listing::Stale, Listing::Fresh] {
        let printed = object(listing, &rows(&tree, listing)).to_string();
        assert!(!printed.contains("root"), "{printed}");
        assert!(!printed.contains(REPO), "{printed}");
    }
}

#[test]
fn the_state_words_are_warlocks_own_and_not_the_engines() {
    assert_eq!(state_word(NodeState::Unpacted), "unpacted");
    assert_eq!(state_word(NodeState::PactedStale), "stale");
    assert_eq!(state_word(NodeState::PactedFresh), "fresh");

    // Not the serde form, which is the engine's internal spelling and is
    // free to be renamed without a shell contract moving under anybody.
    for state in NodeState::ALL {
        assert_ne!(state_word(state), format!("{state:?}"));
    }
}
