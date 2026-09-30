<!-- warlock -->
> Written by a model pass over this directory alone, to be read before its source and to say which source to read. A map, not a specification: check anything you are about to rely on against the files themselves, and where this document and the code disagree, the code is right.

# warlock-tui

The warlock-tui crate: builds the `warlock` binary and `warlock_tui` library, a terminal UI and CLI driving pact/refresh/pull/cut/push/chat/brief flows over a repository's freshness ledger.

## Files

- `Cargo.toml` (2.2 KB) — Cargo manifest for the warlock-tui crate: builds the `warlock` binary (src/main.rs) and the `warlock_tui` library (src/lib.rs); depends on warlock-engine, ratatui, clap, notify, arboard, ureq, ctrlc, serde_json.

## Directories

- `src/` — The crate's source: terminal UI and CLI logic backed by git, Linear and a claude agent; go here for any question about how a flow, screen or command works.

## Structure

- Cargo.toml builds the `warlock` binary from src/main.rs and the `warlock_tui` library from src/lib.rs.
- Cargo.toml declares the crate's dependencies: warlock-engine, ratatui, clap, notify, arboard, ureq, ctrlc, serde_json.
