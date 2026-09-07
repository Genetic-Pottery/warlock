//! The engine owns the domain vocabulary and never depends on the TUI: the
//! dependency edge runs TUI -> engine and never back. It touches the
//! filesystem, but it opens no sockets and spawns no subprocesses — reaching a
//! model is a port ([`Agent`]) the binary implements by running a CLI, which is
//! what lets every test here run with no `claude`, no network and no terminal.

pub mod agent;
pub mod briefs;
pub mod claude_md;
pub mod clock;
pub mod decide;
pub mod document;
pub mod fitting;
pub mod hash;
mod ignores;
mod languages;
pub mod load;
pub mod manifest;
pub mod pact;
pub mod scope;
pub mod sigils;
pub mod state;
pub mod tree;

pub use agent::Agent;
pub use briefs::DEFAULT_BRIEF_DIRECTORY;
pub use briefs::briefs_path;
pub use briefs::load_briefs;
pub use claude_md::Written;
pub use claude_md::write_claude_md;
pub use clock::now_rfc3339;
pub use decide::decide_state;
pub use document::Fill;
pub use document::stub_answer;
pub use fitting::Omission;
pub use fitting::PER_FILE_BYTE_CAP;
pub use hash::subtree_hash;
pub use load::Loaded;
pub use load::load_tree;
pub use load::repository_root;
pub use manifest::Manifest;
pub use manifest::PactEntry;
pub use manifest::SCHEMA_VERSION;
pub use manifest::from_manifest_path;
pub use manifest::manifest_path;
pub use manifest::to_manifest_path;
pub use pact::Pacted;
pub use pact::PactedSubtree;
pub use pact::Pacting;
pub use pact::Refusal;
pub use pact::Unviewable;
pub use pact::Unwatched;
pub use pact::Viewed;
pub use pact::closed_scopes_at_or_below;
pub use pact::pact_directory;
pub use pact::pact_subtree;
pub use pact::refresh_subtree;
pub use pact::unpact_subtree;
pub use pact::view_file;
pub use scope::scope_covering;
pub use scope::scope_opens_to;
pub use scope::validate_scope;
pub use scope::validate_sigil;
pub use sigils::load_sigils;
pub use sigils::project_directory;
pub use sigils::save_sigils;
pub use sigils::sigils_path;
pub use state::NodeState;
pub use tree::DepthFirst;
pub use tree::IntoDocument;
pub use tree::Node;
pub use tree::StateCounts;
pub use tree::Tree;
