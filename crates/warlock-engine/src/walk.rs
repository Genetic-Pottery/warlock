use std::ffi::OsStr;
use std::path::Path;

use ignore::WalkBuilder;

pub(crate) const MANIFEST_DIR: &str = ".warlock";

// A symlinked directory walked as a symlink is never descended into, so a cycle
// of them terminates. A fixture with a `.gitignore` and no `.git` still has to
// be ignored.
//
// Hidden entries are skipped by a filter rather than by the walker's own
// hidden-file rule only so the root itself is never filtered: a tree opened
// from inside a hidden directory still has a root. `.warlock/` is hidden, so
// the manifest stays out of the tree by the same test.
pub(crate) fn listing(root: &Path) -> WalkBuilder {
    let mut builder = WalkBuilder::new(root);
    builder
        .follow_links(false)
        .require_git(false)
        .hidden(false)
        .filter_entry(|entry| entry.depth() == 0 || !is_hidden(entry.file_name()));
    builder
}

fn is_hidden(name: &OsStr) -> bool {
    name.as_encoded_bytes().starts_with(b".")
}
