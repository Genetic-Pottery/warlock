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

    let Loaded { tree, problems, .. } =
        load_tree(&root).map_err(|source| Error::Load { source })?;
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
#[path = "tests/query.rs"]
mod tests;
