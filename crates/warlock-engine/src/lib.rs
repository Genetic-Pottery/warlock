//! The engine owns the domain vocabulary and never depends on the TUI: the
//! dependency edge runs TUI -> engine and never back. It touches the
//! filesystem, but it opens no sockets and spawns no subprocesses — the binary
//! reaches a model by running a CLI, which is what lets every test here run
//! with no `claude`, no network and no terminal.

pub mod agent;
pub mod briefs;
pub mod clock;
pub mod drafting;
pub mod filing;
pub mod fill;
pub mod keys;
pub mod load;
pub mod manifest;
pub mod pulls;
pub mod route;
pub mod scope;
pub mod sigils;
pub mod splitting;
pub mod tree;
pub mod view;
mod walk;
pub mod working;

pub use briefs::DEFAULT_BRIEF_DIRECTORY;
pub use briefs::briefs_path;
pub use briefs::load_briefs;
pub use clock::now_rfc3339;
// The crate's only unqualified `Error`, and it stays the filing one rather than
// growing a `FilingError` alias: every other module's error is reached as
// `route::Error`, `keys::Error` and so on, and a second spelling of one type is
// how two call sites end up looking like they refuse different things.
pub use filing::Destination;
pub use filing::Error;
pub use filing::Target;
pub use filing::resolve_filing;
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
pub use manifest::SCHEMA_VERSION;
pub use manifest::ScopeRecord;
pub use manifest::ScopedDirectory;
pub use manifest::from_manifest_path;
pub use manifest::manifest_path;
pub use manifest::to_manifest_path;
pub use pulls::PullRun;
pub use pulls::PullSubtask;
pub use pulls::ReasonMissing;
pub use pulls::Reset;
pub use pulls::ResetMode;
pub use pulls::RunStatus;
pub use pulls::ScopeRuns;
pub use pulls::SubtaskStatus;
pub use pulls::brief_path;
pub use pulls::held_runs;
pub use pulls::pulls_dir;
pub use pulls::run_dir;
pub use pulls::run_manifest_path;
pub use pulls::state_path;
pub use route::Route;
pub use route::RouteFacts;
pub use route::resolve_route;
pub use route::route_facts;
pub use scope::scope_covering;
pub use scope::scope_opens_to;
pub use scope::validate_scope;
pub use scope::validate_sigil;
pub use sigils::held_base_branch;
pub use sigils::held_sigils;
pub use sigils::load_key_binding;
pub use sigils::load_sigils;
pub use sigils::project_directory;
pub use sigils::save_base_branch;
pub use sigils::save_key_binding;
pub use sigils::save_sigils;
pub use sigils::scopes_bound_to;
pub use sigils::sigils_path;
pub use tree::DepthFirst;
pub use tree::Node;
pub use tree::Tree;
pub use view::PER_FILE_BYTE_CAP;
pub use view::Unviewable;
pub use view::Viewed;
pub use view::view_file;
