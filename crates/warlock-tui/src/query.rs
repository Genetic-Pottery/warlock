//! `warlock stale` and `warlock fresh`: which pacted directories are in which
//! state, on the ordinary screen, with nothing written anywhere.
//!
//! Two roots, and keeping them apart is most of this module. The *tree* is
//! rooted at the path the reader named, because a listing is "at or below
//! here"; the *spelling* is against the repository root, because a manifest
//! path is repository-root-relative and `warlock stale crates` run from one
//! shell must not name a directory differently from `warlock stale` run from
//! another. Nothing here decides what stale means, and the words for the states
//! are [`state_word`]'s rather than [`NodeState`]'s serde derive, which spells
//! an internal form the engine stays free to rename.
//!
//! An empty answer is a success, so anything warlock could not answer has to be
//! a refusal instead: a directory whose subtree could not be hashed is coloured
//! stale on no evidence, and printing it here would be inventing a verdict out
//! of an unreadable file.

use std::io::{self, Write};
use std::path::{Path, PathBuf};

use serde_json::{Map, Value, json};
use warlock_engine::{Loaded, NodeState, Tree, load_tree};

use crate::error::Error;
use crate::standing::{FOR_LISTING, Standing};

const COMMAND: &str = "command";

const DIRECTORIES: &str = "directories";

// A type rather than a `NodeState` passed straight down: the state to filter by
// and the word the object names the command with are two facts that have to
// agree, and a caller holding them separately can disagree with itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Listing {
    Stale,
    Fresh,
}

impl Listing {
    const fn wanted(self) -> NodeState {
        match self {
            Self::Stale => NodeState::PactedStale,
            Self::Fresh => NodeState::PactedFresh,
        }
    }

    const fn word(self) -> &'static str {
        match self {
            Self::Stale => "stale",
            Self::Fresh => "fresh",
        }
    }
}

// The state is carried per row even though every row of one listing is in the
// same state: that is the shape the JSON fixes, and it is read off the node
// rather than off the request, so a row says what warlock found rather than what
// was asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Listed {
    path: String,
    state: &'static str,
}

// Total on purpose, so a fourth state in the engine breaks this at compile time
// rather than printing as whatever a fallback arm picked. Neither `NodeState`'s
// serde derive nor the footer's wording: the first is the engine's internal
// spelling and the second is a renderer's choice about what reads well, and both
// stay free to move while these three words are a shell contract.
const fn state_word(state: NodeState) -> &'static str {
    match state {
        NodeState::Unpacted => "unpacted",
        NodeState::PactedStale => "stale",
        NodeState::PactedFresh => "fresh",
    }
}

// `path` is taken relative to the working directory, as a person typing one at a
// shell means it, and an absolute one is used as it stands — `Path::join` does
// both. It is not normalised here: `load_tree` does that and comes back rooted
// at the path it walked, which is the form worth measuring against the
// repository root afterwards.
pub(crate) fn list(listing: Listing, path: Option<PathBuf>, json: bool) -> Result<(), Error> {
    // Stood up *first*, before the load below, because it is what "path omitted"
    // means: `load_tree` finds the repository root above whatever it is handed,
    // which for `warlock stale crates` would be the same root by a longer road
    // and for `warlock stale` would be no question at all — there would be
    // nothing to root the tree at yet.
    listed_onto(
        &Standing::here(FOR_LISTING)?,
        listing,
        path,
        json,
        &mut io::stdout(),
    )
}

// Split from `list` so the order is something a test can run: an omitted path
// means the repository root and not the working directory, the load's problems
// are refused *before* a single row is printed, and every row is spelled against
// the root rather than against wherever the reader was standing.
fn listed_onto<W: Write>(
    standing: &Standing,
    listing: Listing,
    path: Option<PathBuf>,
    json: bool,
    out: &mut W,
) -> Result<(), Error> {
    let repo_root = standing.repo_root();
    let root = path.map_or_else(|| repo_root.to_path_buf(), |path| standing.target(path));

    let Loaded { tree, problems } = load_tree(&root).map_err(|source| Error::Load { source })?;
    // Refused rather than reported around, as the startup load refuses them:
    // the nodes above an unreadable file are coloured stale on no evidence, and
    // a script reading this listing cannot tell such a line from a verdict. See
    // the module docs.
    if let Some(error) = Error::from_problems(&problems) {
        return Err(error);
    }

    let directories = listed(&tree, repo_root, listing.wanted())?;
    if json {
        write_object(out, &object(listing, &directories));
    } else {
        for directory in &directories {
            drop(writeln!(out, "{}", directory.path));
        }
    }
    Ok(())
}

// The tree's own root is spelled first and the result thrown away. It is the one
// path the reader named, so a listing aimed somewhere with no
// repository-relative form has to refuse; without this it would be a walk whose
// matching nodes all happened to be spellable or, worse, a walk with no matching
// nodes at all, which prints nothing and exits 0. "Nothing is stale over there"
// is not something warlock knows.
fn listed(tree: &Tree, repo_root: &Path, wanted: NodeState) -> Result<Vec<Listed>, Error> {
    spelled(repo_root, tree.root_path())?;

    tree.walk()
        .filter(|(node, _)| node.state == wanted)
        .map(|(node, _)| {
            Ok(Listed {
                path: spelled(repo_root, &node.path)?,
                state: state_word(node.state),
            })
        })
        .collect()
}

// One line in one place, so the root check, every row of the answer and
// `warlock check` cannot disagree about what a path is called or about what it
// means for one to have no name.
pub(crate) fn spelled(repo_root: &Path, path: &Path) -> Result<String, Error> {
    Standing::at(repo_root.to_path_buf(), repo_root.to_path_buf()).spelled(path)
}

// `directories` is an array and empty when nothing matched, never absent and
// never null: a consumer indexing into it should not have to spell the empty
// answer twice. There is no `root` field, deliberately — an absolute machine
// path is the one non-reproducible thing in output meant to be diffable across
// machines, and it tells a consumer a constant.
fn object(listing: Listing, directories: &[Listed]) -> Value {
    let rows = directories
        .iter()
        .map(|directory| json!({ "path": directory.path, "state": directory.state }))
        .collect::<Vec<_>>();
    envelope(listing.word(), [(DIRECTORIES, Value::Array(rows))])
}

// Shared with `warlock check`, whose answer is this envelope with `scope`,
// `sigils` and `opens` in it: "exactly one parseable object" is a promise easier
// to keep in one function than in each subcommand that makes it.
//
// Fields are printed in insertion order, which is what makes `command` first a
// fact rather than an accident of the alphabet — see the `preserve_order`
// feature in the workspace manifest, taken for exactly this.
pub(crate) fn envelope(
    command: &str,
    body: impl IntoIterator<Item = (&'static str, Value)>,
) -> Value {
    let mut object = Map::new();
    object.insert(COMMAND.to_owned(), Value::String(command.to_owned()));
    for (field, value) in body {
        object.insert(field.to_owned(), value);
    }
    Value::Object(object)
}

// Compact and on one line, which is what a pipe into `jq` wants and what makes
// "exactly one object" visible from the shape of the output rather than only
// from parsing it. Takes the writer rather than reaching for stdout so the
// composition above it is one a test can read back. A write that fails is
// ignored: a closed pipe is the ordinary end of `warlock stale | head`, and
// there is nothing useful to say about it on the same broken stream.
pub(crate) fn write_object<W: Write>(out: &mut W, object: &Value) {
    drop(writeln!(out, "{object}"));
}

#[cfg(test)]
mod tests {
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
}
