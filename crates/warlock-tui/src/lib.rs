//! The pure parts of warlock's terminal front end.
//!
//! Everything here is data and functions over data: the flattened tree the
//! screen shows, where the selection sits, which colour a state is drawn in,
//! and what one frame looks like. None of it opens a terminal, so all of it is
//! testable with nothing attached to stdout — the draw path against an
//! in-memory buffer, the rest against plain values. Raw mode, the alternate
//! screen and the event loop belong to the binary in `src/main.rs`.
//!
//! The dependency edge runs TUI -> engine: this crate knows the engine's
//! vocabulary, and the engine knows nothing about terminals.

mod app;
mod colour;
/// The hand-written tree the tests in this crate draw and walk, so none of
/// them needs a repository on disk. Test-only, and private on purpose: the
/// real tree comes from the engine's loader.
#[cfg(test)]
mod fixture;
mod ui;

/// The front end's state: the flattened tree, the selected row and the slice
/// of rows on screen.
pub use app::App;
/// What a pact toggle changed, for whoever has to write it to the manifest.
pub use app::PactToggle;
/// One line of the flattened tree.
pub use app::Row;
/// The colour a node state is drawn in.
pub use colour::colour_for;
/// Draw one frame of the app.
pub use ui::draw;
