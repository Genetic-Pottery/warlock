<!-- warlock -->
> Written by a model pass over this directory alone, to be read before its source and to say which source to read. A map, not a specification: check anything you are about to rely on against the files themselves, and where this document and the code disagree, the code is right.

# warlock-tui

The warlock-tui crate: a Cargo package building the `warlock` binary and `warlock_tui` library that provide the terminal UI and CLI over pact/refresh/pull/cut/push/chat/brief flows.

## Files

- `Cargo.toml` (2.2 KB) — Cargo manifest for the warlock-tui crate: builds the `warlock` binary (src/main.rs) and the `warlock_tui` library (src/lib.rs); depends on warlock-engine, ratatui, clap, notify, arboard, ureq, ctrlc, serde_json.

## Directories

- `src/` — The crate's Rust source: TUI/CLI driving pact/refresh/pull/cut/push/chat/brief flows over the freshness ledger, scope manifest, and Linear/git/gh/claude integrations.

## Structure

- Cargo.toml builds the `warlock` binary from src/main.rs and the `warlock_tui` library from src/lib.rs.
- Cargo.toml declares dependencies warlock-engine, ratatui, clap, notify, arboard, ureq, ctrlc, serde_json.
