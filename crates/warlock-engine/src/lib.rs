//! Core engine for warlock.
//!
//! This crate owns the domain vocabulary. It never depends on the TUI or on
//! any terminal crate, and it reads no files and opens no sockets: the
//! dependency edge runs TUI -> engine and never back.

mod state;

pub use state::NodeState;

#[cfg(test)]
mod tests {
    /// Placeholder: keeps the test target wired up until real behaviour lands.
    #[test]
    fn crate_is_named_warlock_engine() {
        assert_eq!(env!("CARGO_PKG_NAME"), "warlock-engine");
    }
}
