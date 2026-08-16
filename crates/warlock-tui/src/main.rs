//! Terminal front end for warlock.
//!
//! Shell only for now: the alternate screen, the raw-mode terminal guard and
//! the event loop land in a later slice. What exists today is the state those
//! will drive — the engine's stub tree, flattened and selectable — so this
//! binary builds it and prints it as plain lines.

use warlock_engine::stub_tree;
use warlock_tui::App;

fn main() {
    let app = App::from_tree(&stub_tree());
    for row in app.rows() {
        println!("{}{}", "  ".repeat(row.depth), row.path.display());
    }
}

#[cfg(test)]
mod tests {
    /// Placeholder: keeps the test target wired up until real behaviour lands.
    #[test]
    fn binary_is_named_warlock() {
        assert_eq!(env!("CARGO_BIN_NAME"), "warlock");
    }
}
