<!-- warlock -->
> Written by a model pass over this directory alone, to be read before its source and to say which source to read. A map, not a specification: check anything you are about to rely on against the files themselves, and where this document and the code disagree, the code is right.

# warlock-tui

Crate for the terminal UI and the command-line tool: a clap entry point with subcommands, plus the TUI's state, input, rendering, agent chat and filesystem watching. Builds the `warlock` binary and the warlock_tui library.

## Files

- `Cargo.toml` (1.8 KB) — Cargo.toml for warlock-tui: builds the `warlock` binary from src/main.rs and the warlock_tui library from src/lib.rs, depending on arboard, clap, ctrlc, notify, ratatui, serde_json and warlock-engine.

## Directories

- `src/` — All the crate's Rust source: CLI subcommands (init, config, stale, fresh, check, pact, refresh, scope, key), TUI state, input, drawing, agent chat, watching. Open for any behaviour question.

## Structure

- Cargo.toml builds the `warlock` binary from src/main.rs and the warlock_tui library from src/lib.rs.
- The crate depends on arboard, clap, ctrlc, notify, ratatui, serde_json and warlock-engine.
