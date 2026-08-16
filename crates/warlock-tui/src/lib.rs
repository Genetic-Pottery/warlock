//! The pure parts of warlock's terminal front end.
//!
//! Everything here is data and functions over data: the flattened tree the
//! screen shows, where the selection sits, and which colour a state is drawn
//! in. None of it opens a terminal, so all of it is testable with nothing
//! attached to stdout. Raw mode, the alternate screen and the event loop belong
//! to the binary in `src/main.rs`.
//!
//! The dependency edge runs TUI -> engine: this crate knows the engine's
//! vocabulary, and the engine knows nothing about terminals.

mod app;
mod colour;

/// The front end's state: the flattened tree and the selected row.
pub use app::App;
/// One line of the flattened tree.
pub use app::Row;
/// The colour a node state is drawn in.
pub use colour::colour_for;
