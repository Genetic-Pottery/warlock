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
//! What a run is *doing* while it runs is data of the same kind. [`Account`] is
//! one pact's own record of itself — a section per directory, a line per thing a
//! pass was seen doing, an elapsed clock on every line — and it holds no clock of
//! its own: the instant a line is measured against is handed in by whoever is
//! drawing the frame, so a whole minutes-long run can be driven through it in a
//! test with nothing attached to stdout and no time passing at all.
//!
//! *When* the tree is read again is decided the same way. The disk keeps moving
//! after a load — a file saved in another window, a branch checked out, a build
//! writing thousands of files nobody wants to hear about — and [`WatchPolicy`]
//! is what that movement is weighed by: a filter that is the last walk itself,
//! so an event counts only when its immediate parent is a directory the load
//! produced, and three timing rules that turn one editor save into one reload
//! and a long burst into a handful. It watches nothing and reloads nothing, and
//! it holds no clock either — the instant it compares against is handed in, so a
//! ten-second burst is driven through it in a test in microseconds. Hearing that
//! the disk moved at all is the one impure thing in that module and is kept
//! apart from every decision made about it: [`Watch`] owns the watcher handle,
//! hands on the paths it is given unfiltered, and comes back as [`Watching`] —
//! a value — when no watcher could be started, because warlock without live
//! updates is still warlock.
//!
//! The other exception is [`ClaudeAgent`], which owns a child process: it
//! implements the engine's [`Agent`](warlock_engine::Agent) port by running the
//! `claude` CLI, because the engine spawns nothing and something has to. Its
//! module is the only module in this crate that runs anything, it decides
//! nothing about what a prompt says, and it needs no terminal either — its tests
//! point it at stand-ins and pass on a machine with no `claude` installed.
//!
//! The dependency edge runs TUI -> engine: this crate knows the engine's
//! vocabulary, and the engine knows nothing about terminals.

mod account;
mod app;
mod claude;
mod colour;
/// The hand-written tree the tests in this crate draw and walk, so none of
/// them needs a repository on disk. Test-only, and private on purpose: the
/// real tree comes from the engine's loader.
#[cfg(test)]
mod fixture;
mod ui;
mod watch;

/// Everything one pact did, in the order it did it: a section per directory, a
/// line per thing a pass was seen doing, and a clock on every line.
pub use account::Account;
/// One drawable row of an [`Account`]: a section heading, a clocked line, or the
/// run's closing summary.
pub use account::Line;
/// How a directory's pass ended, in the words it ends its section with.
pub use account::Outcome;
/// One directory's pass inside an [`Account`].
pub use account::Section;
/// The front end's state: the flattened tree, which of it is collapsed, the
/// selected row and the slice of rows on screen — moved by the keys, which
/// drive whichever pane has the focus, or by a pointer, which names the row and
/// the pane it landed on and so consults no focus at all. It also carries what
/// the binary has told it about the terminal's mouse capture, which the footer
/// names the key that changes it by.
pub use app::App;
/// Which of the screen's two panes the keys are driving: toggled by the focus
/// key, or set outright by a click naming a pane.
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
/// What is drawn where a pointer landed: the footer, a border, the tree's
/// header, a row of the tree's window, a line of the panel's.
pub use ui::Hit;
/// Draw one frame of the app.
pub use ui::draw;
/// Which part of the frame a screen point falls on, measured off the layout the
/// frame is cut by.
pub use ui::hit_test;
/// How many lines of the pact's account a terminal of a given size has room for
/// in the panel.
pub use ui::panel_height;
/// How many rows of tree a terminal of a given size has room for.
pub use ui::tree_height;
/// How many further reloads the events arriving during a reload are worth: one,
/// however many of them there were.
pub use watch::COALESCED_RELOADS;
/// The directories the last successful load produced: the filter every
/// filesystem event is held against.
pub use watch::NodeSet;
/// How long the disk has to be quiet before a reload — the debounce.
pub use watch::QUIET_PERIOD;
/// The longest a reload is put off while events keep arriving — the ceiling.
pub use watch::RELOAD_CEILING;
/// A running filesystem watcher over the tree's root and the manifest: it
/// hands on the paths it hears about and decides nothing.
pub use watch::Watch;
/// Told what moved on disk and when, it answers whether the tree is owed a
/// reload right now — and reloads nothing itself.
pub use watch::WatchPolicy;
/// What came of asking for a watcher: one that is running, or one line saying
/// why there is none.
pub use watch::Watching;
