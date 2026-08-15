//! Terminal front end for warlock.
//!
//! Shape only: this binary is intentionally inert for now. There is no
//! rendering and no event loop yet — both land in a later slice, along with the
//! terminal dependencies they need.

fn main() {}

#[cfg(test)]
mod tests {
    /// Placeholder: keeps the test target wired up until real behaviour lands.
    #[test]
    fn binary_is_named_warlock() {
        assert_eq!(env!("CARGO_BIN_NAME"), "warlock");
    }
}
