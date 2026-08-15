//! Core engine for warlock.
//!
//! Shape only: this crate is intentionally empty for now. Behaviour lands in a
//! later slice. The engine never depends on the TUI or on any terminal crate.

#[cfg(test)]
mod tests {
    /// Placeholder: keeps the test target wired up until real behaviour lands.
    #[test]
    fn crate_is_named_warlock_engine() {
        assert_eq!(env!("CARGO_PKG_NAME"), "warlock-engine");
    }
}
