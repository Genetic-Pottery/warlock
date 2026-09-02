//! `warlock stale` and `warlock fresh`: which pacted directories are in which
//! state, on the ordinary screen, with nothing written anywhere.
//!
//! The third and fourth subcommands, in the shape `warlock init` gave the first
//! and `warlock config` the second: dispatched before anything touches the
//! terminal, no alternate screen, no raw mode, no panic hook, and a failure is
//! an [`Error`] returned to `main`, which prints it in the same place and the
//! same shape as a tree that would not load. Nothing here writes a file, spawns
//! a process or runs a model pass — these are questions, and the whole of what
//! they do is read the repository and print the answer.
//!
//! # Nothing here decides what stale means
//!
//! The state on every line is the state [`load_tree`] already put on the node,
//! which is [`decide_state`](warlock_engine::decide_state)'s answer: a recorded
//! `granted_hash` against the hash of what is on disk now. So a listing is a
//! filter over a load and nothing else. That is the point of the subcommands
//! existing at all — the alternative for a script is parsing
//! `.warlock/pacts.toml` and hashing the tree itself, which is the staleness
//! rule written a second time somewhere it will drift from the first.
//!
//! # Where the answer is rooted, and what it is spelled against
//!
//! Two different roots, and keeping them apart is most of this module. The
//! *tree* is rooted at the path the reader named, because a listing is "at or
//! below here"; the *spelling* is against the repository root above the working
//! directory, because a manifest path is repository-root-relative and a reader
//! who runs `warlock stale crates` in one shell and `warlock stale` in another
//! must not get two different names for one directory. With no path at all the
//! two coincide: the omitted path is the repository root and never the working
//! directory, so `warlock stale` answers about the repository from wherever it
//! is typed.
//!
//! [`to_manifest_path`] is what does that spelling, so `.` is what the
//! repository root itself prints as — a real answer, since the root can be a
//! pacted module like any other, and never the empty line a naive
//! `strip_prefix` would produce.
//!
//! # The words on the lines are warlock's, and they are a contract
//!
//! [`state_word`] is a total match written here rather than
//! [`NodeState`]'s serde derive or the footer's own wording. Both of those are
//! free to change — the derive spells the internal form the engine may rename,
//! and the footer spells what looks right on a screen — while these three words
//! are what a script greps for, and a shell contract that moves when a renderer
//! is retitled is not a contract.
//!
//! # What is refused, and what is merely empty
//!
//! An empty answer is a success: nothing on stdout and exit 0, because "nothing
//! is stale" is the answer rather than the absence of one. A question warlock
//! could not answer is one line on stderr and exit 1 — a manifest that will not
//! parse, no repository above the working directory, a path that cannot be
//! walked, and a path that has no repository-relative spelling. A load's
//! non-fatal [`problems`](warlock_engine::Loaded::problems) are refusals too,
//! for the reason [`load_app`](crate::session::load_app) refuses them at
//! startup: a directory whose subtree could not be hashed is coloured stale on
//! no evidence, and printing it under `warlock stale` would be inventing a
//! verdict out of an unreadable file. The exit contract says an unreadable path
//! is a 1, so it is one here rather than a line a script would act on.

use std::env;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value, json};
use warlock_engine::{Loaded, NodeState, Tree, load_tree, repository_root, to_manifest_path};

use crate::error::Error;

/// What a listing wants a repository root for, as the tail of
/// [`Error::NoRepository`]'s sentence. `init`'s own tail is spelled beside
/// `init` in `main.rs`, and `config`'s beside `config`.
const FOR_LISTING: &str = "list the directories under";

/// The one field every `--json` object carries, naming the question it answers.
///
/// A consumer that has piped several of these together can tell them apart by
/// it, and a consumer that asked for one gets to assert it got that one.
const COMMAND: &str = "command";

/// The field a listing's answer is under, and the shape later slices follow:
/// `directories` for a listing, `scope`/`sigils`/`opens` for a check.
const DIRECTORIES: &str = "directories";

/// Which of the two listings was asked for.
///
/// A type rather than a `NodeState` passed straight down, because the state to
/// filter by and the word the object names the command with are two facts that
/// have to agree, and a caller holding them separately is a caller that can
/// disagree with itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Listing {
    /// `warlock stale`.
    Stale,
    /// `warlock fresh`.
    Fresh,
}

impl Listing {
    /// The state a directory has to be in to appear in this listing.
    const fn wanted(self) -> NodeState {
        match self {
            Self::Stale => NodeState::PactedStale,
            Self::Fresh => NodeState::PactedFresh,
        }
    }

    /// The word this listing is typed as, which is also the word its object
    /// names itself with.
    const fn word(self) -> &'static str {
        match self {
            Self::Stale => "stale",
            Self::Fresh => "fresh",
        }
    }
}

/// One directory of an answer: how the manifest spells it, and what warlock
/// knows about it.
///
/// The state is carried per row even though every row of one listing is in the
/// same state, because that is the shape the JSON fixes and because it is read
/// off the node rather than off the request — a row says what warlock found,
/// not what was asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Listed {
    /// The directory, repository-root-relative with forward slashes, and `.`
    /// for the repository root itself.
    path: String,
    /// The state, in warlock's own vocabulary. See [`state_word`].
    state: &'static str,
}

/// What `state` is called on a line of output and in a JSON object.
///
/// A total match, so a fourth state in the engine breaks this at compile time
/// rather than being printed as whatever a fallback arm picked. Deliberately
/// not [`NodeState`]'s serde derive: that spells the engine's internal form,
/// which the engine has to stay free to rename, and pinning a shell contract to
/// it would leak this edge backwards into the crate that knows nothing about
/// shells. Deliberately not the footer's wording either — that is a renderer's
/// choice about what reads well in a band of text — even though the two happen
/// to agree today.
const fn state_word(state: NodeState) -> &'static str {
    match state {
        NodeState::Unpacted => "unpacted",
        NodeState::PactedStale => "stale",
        NodeState::PactedFresh => "fresh",
    }
}

/// `warlock stale [path]` and `warlock fresh [path]`: print the directories at
/// or below `path` in the state `listing` asks for.
///
/// The steps are the module doc's, in this order: the working directory says
/// where the repository is, the repository root is resolved from it — so an
/// omitted `path` is that root and not the directory the reader happens to
/// stand in — the tree is loaded rooted at whatever was named, and the walk is
/// filtered and printed.
///
/// `path` is taken relative to the working directory, as a person typing one at
/// a shell means it, and an absolute one is used as it stands ([`Path::join`]
/// does both). It is not made absolute or normalised here: [`load_tree`] does
/// that itself and comes back rooted at the path it actually walked, which is
/// the form worth measuring against the repository root afterwards.
///
/// # Errors
///
/// [`Error::WorkingDirectory`] and [`Error::NoRepository`] before anything is
/// read; [`Error::Load`] for a path that cannot be walked or a manifest that
/// will not parse; [`Error::Problems`] for a load that could not colour every
/// node it was asked to; and [`Error::Unspellable`] for a path with no
/// repository-relative form. Every one of them is one line on stderr and an
/// exit status of 1, and none of them prints a partial answer first.
pub(crate) fn list(listing: Listing, path: Option<PathBuf>, json: bool) -> Result<(), Error> {
    let working_dir = env::current_dir().map_err(|source| Error::WorkingDirectory { source })?;
    // Asked directly rather than taken from the load below, and asked *first*,
    // because it is what "path omitted" means: `load_tree` finds the repository
    // root above whatever it is handed, which for `warlock stale crates` would
    // be the same root by a longer road and for `warlock stale` would be no
    // question at all — there would be nothing to root the tree at yet.
    let repo_root = repository_root(&working_dir).ok_or(Error::NoRepository {
        start: working_dir.clone(),
        wanted: FOR_LISTING,
    })?;
    let root = path.map_or_else(|| repo_root.clone(), |path| working_dir.join(path));

    let Loaded { tree, problems } = load_tree(&root).map_err(|source| Error::Load { source })?;
    // Refused rather than reported around, as the startup load refuses them:
    // the nodes above an unreadable file are coloured stale on no evidence, and
    // a script reading this listing cannot tell such a line from a verdict. See
    // the module docs.
    if let Some(error) = Error::from_problems(&problems) {
        return Err(error);
    }

    let directories = listed(&tree, &repo_root, listing.wanted())?;
    if json {
        print_object(&object(listing, &directories));
    } else {
        for directory in &directories {
            println!("{}", directory.path);
        }
    }
    Ok(())
}

/// The directories of `tree` in `wanted`, spelled relative to `repo_root`, in
/// the order the walk meets them: parents before children, siblings by name.
///
/// The state is compared rather than recomputed. Every node was coloured by the
/// load that produced it and nothing here hashes anything, which is the whole
/// of why this file can be sure it agrees with the tree on screen.
///
/// The tree's own root is spelled first and for its own sake, and the result
/// thrown away. It is the one path the reader named, so a `warlock stale` aimed
/// somewhere with no repository-relative form has to be a refusal — without
/// this it would be a walk whose every matching node happened to be spellable
/// or, far worse, a walk with no matching nodes at all, which prints nothing
/// and exits 0. "Nothing is stale over there" is not something warlock knows.
///
/// # Errors
///
/// [`Error::Unspellable`] for the root, or for any directory in the answer,
/// that is outside `repo_root` or not valid UTF-8.
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

/// `path` as the manifest spells one: relative to `repo_root`, forward slashes,
/// and `.` for the root itself.
///
/// One line, in one place, so that the root check and every row of the answer
/// cannot disagree about what a path is called or about what it means for one
/// to have no name.
fn spelled(repo_root: &Path, path: &Path) -> Result<String, Error> {
    to_manifest_path(repo_root, path).map_err(|source| Error::Unspellable { source })
}

/// The object `--json` prints for a listing: the command it answers, and the
/// directories it found.
///
/// `directories` is an array and empty when nothing matched, never absent and
/// never null: a consumer indexing into it should not have to spell the empty
/// answer twice. There is no `root` field, deliberately — an absolute machine
/// path is the one non-reproducible thing in a document meant to be diffable
/// across machines, and it tells a consumer a constant.
fn object(listing: Listing, directories: &[Listed]) -> Value {
    let rows = directories
        .iter()
        .map(|directory| json!({ "path": directory.path, "state": directory.state }))
        .collect::<Vec<_>>();
    envelope(listing.word(), [(DIRECTORIES, Value::Array(rows))])
}

/// The one object a `--json` query prints: `command`, then whatever that
/// command has to say.
///
/// The envelope in one small place because it is shared rather than this
/// listing's own — a check's answer is the same envelope with `scope`, `sigils`
/// and `opens` in it — and because "exactly one parseable object" is a promise
/// easier to keep in one function than in each subcommand that makes it.
fn envelope(command: &str, body: impl IntoIterator<Item = (&'static str, Value)>) -> Value {
    let mut object = Map::new();
    object.insert(COMMAND.to_owned(), Value::String(command.to_owned()));
    for (field, value) in body {
        object.insert(field.to_owned(), value);
    }
    Value::Object(object)
}

/// Print `object` as the one line of JSON it is.
///
/// Compact and on a single line, which is what a pipe into `jq` wants and what
/// makes "exactly one object" visible from the shape of the output rather than
/// only from parsing it. [`Value`]'s own [`Display`](std::fmt::Display) is that
/// form, so there is no serialisation here that could fail.
fn print_object(object: &Value) {
    println!("{object}");
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use warlock_engine::{Node, NodeState, Tree};

    use super::{Listed, Listing, listed, object, state_word};
    use crate::error::Error;

    /// The repository root every tree here is spelled against.
    const REPO: &str = "/repo";

    /// A tree of a repository with a pacted root, a stale module under it, a
    /// fresh one beside it, and an unpacted directory below the fresh one.
    ///
    /// Built as values rather than loaded from disk: what is under test is the
    /// filtering and the spelling, and the colouring is the engine's own and
    /// tested there.
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

    /// The paths a listing would print, in the order it would print them.
    fn paths(rows: &[Listed]) -> Vec<&str> {
        rows.iter().map(|row| row.path.as_str()).collect()
    }

    /// The rows of `listing` over `tree`, spelled against [`REPO`].
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

        let error = listed(&elsewhere, Path::new(REPO), NodeState::PactedStale)
            .expect_err("a path outside the repository has no manifest form");

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
