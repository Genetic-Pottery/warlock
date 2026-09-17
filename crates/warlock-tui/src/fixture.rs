//! Written out by hand rather than loaded: no directory is read, no file is
//! opened and no staleness is computed, so the app and draw tests can assert
//! exact lines without depending on whichever repository the test ran in. This
//! is a fixture and nothing else — not a loader, not a fallback for a failed
//! load. The shapes below are load-bearing: change them and the line-by-line
//! assertions in `ui.rs` and the counts in `app.rs` change with them.

use std::path::{Path, PathBuf};

use warlock_engine::{Node, NodeState, Tree};

/// ```text
/// warlock                          document, pacted, stale
/// │   README.md, WARLOCK.md
/// ├── warlock/crates               no document yet, unpacted, no files of its own
/// │   ├── warlock/crates/engine    document, pacted, fresh
/// │   │       Cargo.toml, WARLOCK.md
/// │   └── warlock/crates/tui       document, pacted, stale
/// │           WARLOCK.md
/// └── warlock/assets               document, unpacted
///             WARLOCK.md, logo.svg
/// ```
///
/// The listings follow the loader's own rules, so a view tested against this is
/// tested against something a real load could produce: path order, no
/// subdirectory — those are children — and a directory's own `WARLOCK.md` among
/// them, since the loader lists what the walk saw rather than what the walk saw
/// minus one special name. The paths are literals, related to this repository's
/// layout only so a failing assertion reads like something recognisable.
pub(crate) fn tree() -> Tree {
    Tree::new(
        Node::new("warlock", "warlock/WARLOCK.md", NodeState::PactedStale)
            .with_files(files("warlock", ["README.md", "WARLOCK.md"]))
            .with_children([
                // No document of its own yet, so `None` and unpacted, which is
                // what the loader makes of such a directory.
                Node::new("warlock/crates", None, NodeState::Unpacted).with_children([
                    Node::new(
                        "warlock/crates/engine",
                        "warlock/crates/engine/WARLOCK.md",
                        NodeState::PactedFresh,
                    )
                    .with_files(files("warlock/crates/engine", ["Cargo.toml", "WARLOCK.md"])),
                    Node::new(
                        "warlock/crates/tui",
                        "warlock/crates/tui/WARLOCK.md",
                        NodeState::PactedStale,
                    )
                    .with_files(files("warlock/crates/tui", ["WARLOCK.md"])),
                ]),
                Node::new(
                    "warlock/assets",
                    "warlock/assets/WARLOCK.md",
                    NodeState::Unpacted,
                )
                .with_files(files("warlock/assets", ["WARLOCK.md", "logo.svg"])),
            ]),
    )
}

/// ```text
/// warlock                          document, pacted, stale
/// │   README.md, WARLOCK.md
/// ├── warlock/crates               document now, pacted, fresh
/// │   │   WARLOCK.md               <- what the run wrote
/// │   ├── warlock/crates/engine    document, pacted, fresh
/// │   │       Cargo.toml, WARLOCK.md
/// │   └── warlock/crates/tui       document, pacted, stale
/// │           WARLOCK.md
/// └── warlock/assets               document, unpacted
///             WARLOCK.md, logo.svg
/// ```
///
/// One directory differs, and only in the way a finished run makes one differ:
/// a re-seat has to be shown carrying a view across a tree that has changed
/// *somewhere*, not across a tree that has been replaced. Written out rather
/// than derived from [`tree`] by mutation, because a fixture whose shape has to
/// be worked out by reading a patch is one the assertions against it cannot be
/// read beside.
pub(crate) fn tree_after_a_run() -> Tree {
    Tree::new(
        Node::new("warlock", "warlock/WARLOCK.md", NodeState::PactedStale)
            .with_files(files("warlock", ["README.md", "WARLOCK.md"]))
            .with_children([
                // The run wrote this document and granted its hash, so the
                // directory that was undocumented and unpacted is neither now.
                Node::new(
                    "warlock/crates",
                    "warlock/crates/WARLOCK.md",
                    NodeState::PactedFresh,
                )
                .with_files(files("warlock/crates", ["WARLOCK.md"]))
                .with_children([
                    Node::new(
                        "warlock/crates/engine",
                        "warlock/crates/engine/WARLOCK.md",
                        NodeState::PactedFresh,
                    )
                    .with_files(files("warlock/crates/engine", ["Cargo.toml", "WARLOCK.md"])),
                    Node::new(
                        "warlock/crates/tui",
                        "warlock/crates/tui/WARLOCK.md",
                        NodeState::PactedStale,
                    )
                    .with_files(files("warlock/crates/tui", ["WARLOCK.md"])),
                ]),
                Node::new(
                    "warlock/assets",
                    "warlock/assets/WARLOCK.md",
                    NodeState::Unpacted,
                )
                .with_files(files("warlock/assets", ["WARLOCK.md", "logo.svg"])),
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
