<!-- warlock -->
> Written by a model pass over this directory alone, to be read before its source and to say which source to read. A map, not a specification: check anything you are about to rely on against the files themselves, and where this document and the code disagree, the code is right.

# warlock-tui

The warlock-tui crate root: builds the `warlock` binary and `warlock_tui` library, providing the TUI and CLI over warlock-engine.

## Files

- `Cargo.toml` (2.2 KB) — Cargo manifest for the warlock-tui crate: builds the `warlock` binary (src/main.rs) and the `warlock_tui` library (src/lib.rs); depends on warlock-engine, ratatui, clap, notify, arboard, ureq, ctrlc, serde_json.

## Directories

- `src/` — The TUI binary and CLI source: pact/refresh/scope walks, chat/brief/draft/cut/pull/push flows, Linear and git/gh integration, and ratatui rendering/input — go here for how any warlock command or screen works.

## Structure

- Cargo.toml builds the `warlock` binary from src/main.rs and the `warlock_tui` library from src/lib.rs
- Cargo.toml declares warlock-tui's dependency on warlock-engine, ratatui, clap, notify, arboard, ureq, ctrlc, and serde_json
