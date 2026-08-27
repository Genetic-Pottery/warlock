//! `.warlockignore`: what the repository says Warlock is not about.
//!
//! A repository holds content that is real, tracked and no part of what a
//! document should be written from — a folder of images, a design notebook,
//! the author's scratch notes. Keeping it out of a pact used to mean keeping
//! it out of git; worse, it counted towards every subtree hash above it, so
//! editing a note turned a directory yellow and bought a model pass over
//! content Warlock never read.
//!
//! `.warlockignore` is how a repository says so, in the syntax its authors
//! already know. It is gitignore syntax because it *is* gitignore matching:
//! the [`ignore`] crate reads the file at every level through
//! [`WalkBuilder::add_custom_ignore_filename`], so nesting, `!` negation,
//! anchoring and directory-only patterns all behave exactly as git behaves,
//! and this crate keeps no skip list of its own.
//!
//! The filename is spelled once, here, and every walk that honours it takes it
//! from [`FILENAME`]. It is not configurable: one name, known to everyone
//! reading the repository, is the point.
//!
//! # Why a root check exists at all
//!
//! A walker does not apply ignore rules to the root path it was handed — the
//! rules say what to descend into, and the caller already chose the root. That
//! is right for a walk and wrong for us: a directory can be selected directly,
//! by a manifest entry or by a keypress, and hashing or pacting it because it
//! happened to be the root would hand back exactly the content the reader
//! asked to exclude. [`is_ignored`] is that missing check, made of the same
//! machinery: a one-deep walk of the parent, which is where the ancestors'
//! rules apply to this directory's name.

use std::path::Path;

use ignore::WalkBuilder;

/// The one filename that says what Warlock is not about.
///
/// Spelled here and nowhere else, so registering it on a walk and asking
/// whether a directory survives it can never drift apart.
pub(crate) const FILENAME: &str = ".warlockignore";

/// Whether `path` is itself excluded by a `.warlockignore` at or above it.
///
/// Only `.warlockignore` rules are asked about — `.gitignore`, `.ignore`,
/// git's global excludes and the hidden-file rule are all off. Those belong to
/// the walk, which applies them while descending; this is the narrow question
/// of whether the repository named *this* directory as content Warlock does
/// not cover, and nothing else may answer it. (A temporary directory is a
/// hidden directory on most platforms, so a root check that honoured the
/// hidden rule would call every test fixture excluded.)
///
/// A path with no parent, and a path that is not on disk, are both "not
/// ignored": nothing above the first can name it, and the second is the
/// caller's own walk's error to report rather than a verdict to make here.
///
/// # Errors
///
/// Whatever the walker says — a parent directory that cannot be listed, or a
/// `.warlockignore` in it that cannot be parsed. Never degraded to "no rules":
/// a caller that cannot tell whether content is excluded must fail rather than
/// process it.
pub(crate) fn is_ignored(path: &Path) -> Result<bool, ignore::Error> {
    let Some(name) = path.file_name() else {
        // `/`, or a path ending in `..`: nothing above names it.
        return Ok(false);
    };
    let Some(parent) = path.parent() else {
        return Ok(false);
    };
    // A relative single-component path has an empty parent, which is this
    // directory rather than no directory.
    let parent = if parent.as_os_str().is_empty() {
        Path::new(".")
    } else {
        parent
    };
    // Absent is not excluded. Without this, a directory that was deleted would
    // be indistinguishable from one the rules removed — and a vanished
    // directory has to reach the caller as the error it is.
    if path.symlink_metadata().is_err() {
        return Ok(false);
    }

    for entry in walk_one_deep(parent) {
        let entry = entry?;
        // A rule file the walker could not use is reported beside its
        // directory rather than in place of it. Unusable rules are not no
        // rules, so it becomes the error it is instead of a verdict of "not
        // ignored" that would let excluded content through.
        if let Some(err) = entry.error() {
            return Err(err.clone());
        }
        if entry.depth() == 1 && entry.file_name() == name {
            // It survived its parent's listing, so no rule removed it.
            return Ok(false);
        }
    }
    Ok(true)
}

/// The parent's own entries, and only its own entries, filtered by
/// `.warlockignore` alone.
///
/// `parents(true)` is what carries the rules in *this* directory's ancestors
/// down onto it; the walker builds a custom-ignore matcher per ancestor
/// whether or not git's own sources are switched off.
fn walk_one_deep(parent: &Path) -> ignore::Walk {
    WalkBuilder::new(parent)
        .follow_links(false)
        .require_git(false)
        .max_depth(Some(1))
        .hidden(false)
        .git_ignore(false)
        .git_global(false)
        .git_exclude(false)
        .ignore(false)
        .parents(true)
        .add_custom_ignore_filename(FILENAME)
        .build()
}
