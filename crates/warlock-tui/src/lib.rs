//! Warlock's terminal front end, minus the terminal.
//!
//! Almost everything here is data and functions over data: the flattened tree
//! the screen shows, where the selection sits, which colour a state is drawn
//! in, and what one frame looks like. None of it opens a terminal, so all of it
//! is testable with nothing attached to stdout — the draw path against an
//! in-memory buffer, the rest against plain values. Raw mode, the alternate
//! screen and the event loop belong to the binary in `src/main.rs`.
//!
//! A view here outlives the tree it was built on, because a run writes documents
//! and the tree has to be read again to show them. [`reseat_on`] is that move,
//! and it is a function over two values like everything else: rows, states and
//! the tally come from the new tree, while the selection, the collapsed
//! directories, the filters and the window come from the old view and are
//! carried by path rather than by row number, so a reload leaves the reader
//! where they were rather than at the top. It reads no disk and asks for no
//! reload of its own — *when* a tree is read again is the binary's business.
//!
//! The one exception is [`ClaudeAgent`], which owns a child process: it
//! implements the engine's [`Agent`](warlock_engine::Agent) port by running the
//! `claude` CLI, because the engine spawns nothing and something has to. Its
//! module is the only
//! module in this crate that runs anything, it decides nothing about what a
//! prompt says, and it needs no terminal either — its tests point it at
//! stand-ins and pass on a machine with no `claude` installed.
//!
//! The dependency edge runs TUI -> engine: this crate knows the engine's
//! vocabulary, and the engine knows nothing about terminals.

mod app;
mod claude;
mod colour;
/// The hand-written tree the tests in this crate draw and walk, so none of
/// them needs a repository on disk. Test-only, and private on purpose: the
/// real tree comes from the engine's loader.
#[cfg(test)]
mod fixture;
mod ui;

/// The front end's state: the flattened tree, which of it is collapsed, the
/// selected row and the slice of rows on screen.
pub use app::App;
/// Which of the screen's two panes the keys are driving.
pub use app::Focus;
/// What a pact toggle changed, for whoever has to write it to the manifest.
pub use app::PactToggle;
/// One line of the flattened tree.
pub use app::Row;
/// Put a view back on top of a tree that has just been read again: same
/// selection, same collapsed directories, same filters, same window, new rows.
pub use app::reseat_on;
/// A handle for hearing what a model pass is doing, from another thread, while
/// it is still running.
pub use claude::Activities;
/// One thing a model pass was seen doing: a tool call, a stretch of thinking, or
/// what the pass cost.
pub use claude::Activity;
/// A handle for stopping a model pass from another thread: it kills the child
/// running now and refuses to start another.
pub use claude::Cancel;
/// The engine's model-pass port, implemented by running the `claude` CLI as a
/// child process.
pub use claude::ClaudeAgent;
/// How long one model pass is given before it is killed: five minutes, per
/// invocation rather than per pact.
pub use claude::INVOCATION_TIMEOUT;
/// The colour a node state is drawn in.
pub use colour::colour_for;
/// Draw one frame of the app.
pub use ui::draw;
/// How many rows of tree a terminal of a given size has room for.
pub use ui::tree_height;
