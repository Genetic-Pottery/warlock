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
pub mod drafting;
pub mod filed;
pub mod filing;
pub mod fitting;
pub mod hash;
mod ignores;
pub mod keys;
mod languages;
pub mod load;
pub mod manifest;
pub mod pact;
pub mod route;
pub mod scope;
pub mod sigils;
pub mod state;
pub mod tree;
mod walk;

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
pub use filed::CutRecord;
pub use filed::Filed;
pub use filed::FiledRecord;
pub use filed::filed_path;
pub use filed::fold_title;
// The crate's only unqualified `Error`, and it stays the filing one rather than
// growing a `FilingError` alias: every other module's error is reached as
// `route::Error`, `keys::Error` and so on, and a second spelling of one type is
// how two call sites end up looking like they refuse different things.
pub use filing::Error;
pub use filing::Target;
pub use filing::resolve_filing;
pub use fitting::Omission;
pub use fitting::PER_FILE_BYTE_CAP;
pub use hash::file_hash;
pub use hash::subtree_hash;
pub use keys::Forgotten;
pub use keys::forget_key;
pub use keys::keys_path;
pub use keys::load_key;
pub use keys::load_key_names;
pub use keys::save_key;
pub use load::Loaded;
pub use load::load_tree;
pub use load::repository_root;
pub use manifest::Manifest;
pub use manifest::PactEntry;
pub use manifest::SCHEMA_VERSION;
pub use manifest::ScopeRecord;
pub use manifest::from_manifest_path;
pub use manifest::manifest_path;
pub use manifest::to_manifest_path;
pub use pact::Pacted;
pub use pact::PactedSubtree;
pub use pact::Pacting;
pub use pact::Refusal;
pub use pact::Repaired;
pub use pact::Unviewable;
pub use pact::Unwatched;
pub use pact::Viewed;
pub use pact::closed_scopes_at_or_below;
pub use pact::pact_directory;
pub use pact::pact_subtree;
pub use pact::refresh_subtree;
pub use pact::unpact_ignored;
pub use pact::unpact_subtree;
pub use pact::view_file;
pub use route::Route;
pub use route::RouteFacts;
pub use route::resolve_route;
pub use route::route_facts;
pub use scope::scope_covering;
pub use scope::scope_opens_to;
pub use scope::validate_scope;
pub use scope::validate_sigil;
pub use sigils::load_key_binding;
pub use sigils::load_sigils;
pub use sigils::project_directory;
pub use sigils::save_key_binding;
pub use sigils::save_sigils;
pub use sigils::sigils_path;
pub use state::NodeState;
pub use tree::DepthFirst;
pub use tree::IntoDocument;
pub use tree::Node;
pub use tree::StateCounts;
pub use tree::Tree;
pub use walk::DOCUMENT_FILE;
