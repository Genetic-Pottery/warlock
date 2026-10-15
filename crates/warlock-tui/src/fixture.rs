//! Written out by hand rather than loaded: no directory is read and no file is
//! opened, so the app and draw tests can assert exact lines without depending on
//! whichever repository the test ran in. This is a fixture and nothing else —
//! not a loader, not a fallback for a failed load. The shapes below are
//! load-bearing: change them and the line-by-line assertions in `ui.rs` and the
//! counts in `app.rs` change with them.

use std::path::{Path, PathBuf};

use warlock_engine::{Node, Tree};

/// ```text
/// warlock                          unscoped
/// │   README.md
/// ├── warlock/crates               scoped `platform`, no files of its own
/// │   ├── warlock/crates/engine    scoped `data-plane`, nearer than `platform`
/// │   │       Cargo.toml
/// │   └── warlock/crates/tui       covered by `platform` from above
/// │           Cargo.toml
/// └── warlock/assets               unscoped
///             logo.svg
/// ```
///
/// The listings follow the loader's own rules, so a view tested against this is
/// tested against something a real load could produce: path order, and no
/// subdirectory — those are children. The paths are literals, related to this
/// repository's layout only so a failing assertion reads like something
/// recognisable.
pub(crate) fn tree() -> Tree {
    Tree::new(
        Node::new("warlock")
            .with_files(files("warlock", ["README.md"]))
            .with_children([
                Node::new("warlock/crates")
                    .with_scope(Some("platform".to_owned()))
                    .with_children([
                        Node::new("warlock/crates/engine")
                            .with_scope(Some("data-plane".to_owned()))
                            .with_files(files("warlock/crates/engine", ["Cargo.toml"])),
                        Node::new("warlock/crates/tui")
                            .with_covering(Some("platform".to_owned()))
                            .with_files(files("warlock/crates/tui", ["Cargo.toml"])),
                    ]),
                Node::new("warlock/assets").with_files(files("warlock/assets", ["logo.svg"])),
            ]),
    )
}

/// ```text
/// warlock                          unscoped
/// │   README.md
/// ├── warlock/crates               scoped `platform`, no files of its own
/// │   ├── warlock/crates/engine    scoped `data-plane`
/// │   │       Cargo.toml
/// │   └── warlock/crates/tui       covered by `platform` from above
/// │           Cargo.toml
/// └── warlock/assets               scoped `assets` now   <- what was written
///             logo.svg
/// ```
///
/// One directory differs, and only in the way a scope written in another window
/// makes one differ: a re-seat has to be shown carrying a view across a tree that
/// has changed *somewhere*, not across a tree that has been replaced. Written out
/// rather than derived from [`tree`] by mutation, because a fixture whose shape
/// has to be worked out by reading a patch is one the assertions against it
/// cannot be read beside.
pub(crate) fn tree_after_a_rescope() -> Tree {
    Tree::new(
        Node::new("warlock")
            .with_files(files("warlock", ["README.md"]))
            .with_children([
                Node::new("warlock/crates")
                    .with_scope(Some("platform".to_owned()))
                    .with_children([
                        Node::new("warlock/crates/engine")
                            .with_scope(Some("data-plane".to_owned()))
                            .with_files(files("warlock/crates/engine", ["Cargo.toml"])),
                        Node::new("warlock/crates/tui")
                            .with_covering(Some("platform".to_owned()))
                            .with_files(files("warlock/crates/tui", ["Cargo.toml"])),
                    ]),
                Node::new("warlock/assets")
                    .with_scope(Some("assets".to_owned()))
                    .with_files(files("warlock/assets", ["logo.svg"])),
            ]),
    )
}

// Whole paths, not bare names: that is how the loader spells a listing.
fn files<'a>(
    directory: &'a str,
    names: impl IntoIterator<Item = &'a str> + 'a,
) -> impl Iterator<Item = PathBuf> + 'a {
    names
        .into_iter()
        .map(|name| Path::new(directory).join(name))
}

#[cfg(test)]
#[path = "tests/fixture.rs"]
mod tests;
