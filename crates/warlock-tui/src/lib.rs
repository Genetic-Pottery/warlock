//! Everything warlock does except own the terminal. `src/main.rs` parses argv,
//! takes and restores the terminal, and runs the session's event loop; it hands
//! each subcommand to its function here and each key, click and paste to
//! [`Interactive`]. Nothing in this crate enters the alternate screen or runs
//! that loop, and a session reaches its screen only through [`Screen`]: that is
//! what lets a whole session be driven round by round in a test, and the draw
//! path be asserted against an in-memory buffer.
//!
//! The rest of the outside world is reached on purpose — `claude` runs the CLI
//! as a child process because the engine's port spawns nothing, `git` runs `git`
//! and `gh` through `claude`'s process plumbing because a pull request is made of
//! subprocesses, `linear` opens a socket because the board is somewhere else,
//! `watch` holds a filesystem watcher, and `asking` and `key` read stdin at a
//! shell, raw only for the length of one read. The stand-ins tests drive those
//! seams with are in `stubs.rs`, one set for every test in the crate, so a
//! second copy of one beside the test that wants it is a duplicate.

mod account;
mod app;
mod asking;
mod boundary;
mod brief;
mod briefing;
mod chatting;
pub mod check;
mod claude;
mod clipboard;
mod colour;
mod composer;
mod config;
mod confirm;
mod crossings;
mod cut;
mod cutting;
mod descent;
mod editing;
mod edits;
mod error;
#[cfg(test)]
mod fixture;
mod freshness;
mod git;
mod inflight;
mod input;
mod interactive;
mod key;
mod linear;
mod modal;
mod pacting;
mod panel;
pub mod planned;
mod prompt;
mod pull;
mod puller;
mod pulling;
mod push;
mod pushing;
mod query;
mod queue;
mod rescope;
mod resume;
mod running;
mod scoping;
mod screen;
mod selection;
mod session;
mod standing;
#[cfg(test)]
mod stubs;
mod submission;
mod template;
mod thread;
mod ui;
mod viewing;
mod watch;
mod wrap;
mod writing;

pub use account::Account;
pub use account::Line;
pub use account::Voice;
pub use brief::Error as BriefError;
pub use brief::ScopeBlockError;
pub use brief::brief_at;
pub use brief::scope_block_in;
pub use briefing::brief;
pub use claude::Activities;
pub use claude::Activity;
pub use claude::Cancel;
pub use claude::ChatAgent;
pub use claude::ClaudeAgent;
pub use claude::DRAFTING_CONTRACT;
pub use claude::Drafting;
pub use claude::INVOCATION_TIMEOUT;
pub use claude::NOTHING_SETTLES_IT;
pub use claude::Splitting;
pub use claude::WORKING_TIMEOUT;
pub use claude::Working;
pub use claude::brief_instruction;
pub use claude::drafting_opening;
pub use claude::propose_answer;
pub use claude::proposing_instruction;
pub use claude::working_opening;
pub use claude::working_retry;
pub use claude::working_system_prompt;
pub use colour::colour_for;
pub use config::configure;
pub use crossings::Crossing;
pub use crossings::crossings_after;
pub use crossings::crossings_in;
pub use edits::scope_add;
pub use edits::scope_remove;
pub use edits::unpact;
pub use error::Error;
pub use error::status_for;
pub use git::COMMAND_TIMEOUT;
pub use git::Commit;
pub use git::Dirty;
pub use git::Error as GitError;
pub use git::Forge;
pub use git::Freshness;
pub use git::Gh;
pub use git::Git;
pub use git::HUMAN_GATE;
pub use git::Head;
pub use git::Opened;
pub use git::PullRequest;
pub use git::Ran;
pub use git::Repository;
pub use git::Runs;
pub use git::Spawner;
pub use git::branch_name;
pub use git::commit_message;
pub use git::pull_request_body;
pub use git::pull_request_title;
pub use interactive::Interactive;
pub use interactive::POLL_INTERVAL;
pub use key::key_add;
pub use key::key_forget;
pub use key::key_list;
pub use key::key_use;
pub use linear::Client as LinearClient;
pub use linear::Error as LinearError;
pub use linear::Posts;
pub use linear::Priority;
pub use linear::Queue;
pub use linear::QueuedIssue;
pub use linear::REQUEST_TIMEOUT;
pub use linear::StateType;
pub use pull::pull;
pub use push::push;
pub use query::Listing;
pub use query::list;
pub use queue::choose;
pub use rescope::RecordFields;
pub use resume::resume;
pub use running::pact;
pub use running::refresh;
pub use screen::Screen;
pub use standing::FOR_CLAUDE_MD;
pub use standing::Standing;
pub use template::DEFAULT_TEMPLATE;
pub use template::brief_template;
pub use template::missing_sections;
pub use thread::Ending;
pub use thread::Thread;
pub use watch::NodeSet;
pub use watch::QUIET_PERIOD;
pub use watch::WatchPolicy;
