use std::io::Write;
use std::path::Path;

use serde_json::{Map, Value};
use warlock_engine::to_manifest_path;

use crate::error::Error;

const COMMAND: &str = "command";

// One line in one place, so every subcommand that prints a path agrees about
// what it is called and about what it means for one to have no name.
pub(crate) fn spelled(repo_root: &Path, path: &Path) -> Result<String, Error> {
    to_manifest_path(repo_root, path).map_err(|source| Error::Unspellable { source })
}

// Shared by every subcommand that answers with `--json`: "exactly one parseable
// object" is a promise easier to keep in one function than in each subcommand
// that makes it.
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
// ignored: a closed pipe is the ordinary end of `warlock check --json | head`,
// and there is nothing useful to say about it on the same broken stream.
pub(crate) fn write_object<W: Write>(out: &mut W, object: &Value) {
    drop(writeln!(out, "{object}"));
}

#[cfg(test)]
#[path = "tests/query.rs"]
mod tests;
