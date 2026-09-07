//! The front end minus the terminal. Nothing in this crate opens a terminal,
//! reads a key or owns an event loop: `src/main.rs` does all of that and hands
//! values in, which is what keeps the draw path assertable against an in-memory
//! buffer and everything else against plain values. Two modules reach past that
//! rule on purpose — `claude` runs the CLI as a child process because the
//! engine's port spawns nothing, and `watch` holds a filesystem watcher — and
//! both are built so the decisions made about what they hear are values a test
//! can drive without either.

mod account;
mod app;
mod claude;
mod colour;
mod composer;
mod confirm;
#[cfg(test)]
mod fixture;
pub mod panel;
mod prompt;
mod submission;
mod template;
mod thread;
mod ui;
mod watch;
mod wrap;

pub use account::Account;
pub use account::Line;
pub use account::Outcome;
pub use account::Section;
pub use account::size;
pub use app::App;
pub use app::Chrome;
pub use app::Focus;
pub use app::PactIntent;
pub use app::PactToggle;
pub use app::Row;
pub use app::Run;
pub use app::RunHeader;
pub use app::Sigils;
pub use app::reseat_on;
pub use claude::Activities;
pub use claude::Activity;
pub use claude::BRIEF_EFFORT;
pub use claude::BRIEF_MODEL;
pub use claude::CHAT_INSTRUCTION;
pub use claude::Cancel;
pub use claude::ChatAgent;
pub use claude::ClaudeAgent;
pub use claude::Converses;
pub use claude::INVOCATION_TIMEOUT;
pub use claude::WRITE_INSTRUCTION;
pub use claude::Wired;
pub use claude::brief_instruction;
pub use colour::colour_for;
pub use composer::COMPOSER_MAX_ROWS;
pub use composer::Composed;
pub use composer::Composer;
pub use composer::ComposerWindow;
pub use composer::Pasted;
pub use composer::compose_for;
pub use composer::paste_for;
pub use confirm::Answer;
pub use confirm::Answered;
pub use confirm::QuitConfirm;
pub use confirm::answer_for;
pub use panel::{Mode, Panel};
pub use prompt::Edited;
pub use prompt::ScopeField;
pub use prompt::ScopePrompt;
pub use prompt::edit_for;
pub use submission::Submitted;
pub use submission::submitted_for;
pub use template::DEFAULT_TEMPLATE;
pub use template::Error as TemplateError;
pub use template::brief_template;
pub use template::missing_sections;
pub use thread::Ending;
pub use thread::Thread;
pub use thread::Turn;
pub use thread::ending_for;
pub use ui::Hit;
pub use ui::composer_height;
pub use ui::composer_on_screen;
pub use ui::draw;
pub use ui::hit_test;
pub use ui::panel_height;
pub use ui::panel_width;
pub use ui::run_header_height;
pub use ui::tree_height;
pub use watch::COALESCED_RELOADS;
pub use watch::NodeSet;
pub use watch::QUIET_PERIOD;
pub use watch::RELOAD_CEILING;
pub use watch::Watch;
pub use watch::WatchPolicy;
pub use watch::Watching;
