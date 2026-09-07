//! `.warlockignore` is gitignore syntax because it *is* gitignore matching:
//! the [`ignore`] crate reads it at every level, so nesting, negation,
//! anchoring and directory-only patterns behave as git behaves and this crate
//! keeps no skip list of its own.

use std::path::Path;

use ignore::WalkBuilder;

/// Spelled here and nowhere else, so registering the name on a walk and asking
/// whether a directory survives it cannot drift apart.
pub(crate) const FILENAME: &str = ".warlockignore";

/// The check a walker cannot make for its own root: rules say what to descend
/// into, and the root was chosen by the caller. A directory can be selected
/// directly — by a manifest entry, by a keypress — and covering it because it
/// happened to be the root would hand back the content the repository asked to
/// exclude. Made of the same machinery: a one-deep walk of the parent, which is
/// where the ancestors' rules apply to this directory's name.
///
/// Only `.warlockignore` rules are asked about; the rest belong to the walk,
/// which applies them while descending. In particular the hidden-file rule is
/// off, because a temporary directory is hidden on most platforms and a root
/// check honouring it would call every test fixture excluded.
///
/// Never degraded to "no rules": a caller that cannot tell whether content is
/// excluded must fail rather than process it.
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
        // Unusable rules are not no rules: a verdict of "not ignored" here
        // would let excluded content through.
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

/// `parents(true)` is what carries the ancestors' rules down onto this
/// directory; the walker builds a custom-ignore matcher per ancestor whether or
/// not git's own sources are switched off.
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
