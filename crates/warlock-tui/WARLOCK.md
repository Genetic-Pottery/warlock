<!-- warlock -->
> Written by a model pass over this directory alone, to be read before its source and to say which source to read. A map, not a specification: check anything you are about to rely on against the files themselves, and where this document and the code disagree, the code is right.

# warlock-tui

The warlock-tui crate: builds the warlock binary and warlock_tui library, the terminal-free front end for the panel and its headless subcommands, wiring clap, ratatui, notify and ctrlc onto warlock-engine.

## Files

- `Cargo.toml` (1.6 KB) — Manifest for the warlock-tui crate: builds the `warlock` binary from src/main.rs and the warlock_tui library from src/lib.rs, wiring clap, ratatui, notify, ctrlc and warlock-engine.

## Directories

- `src/` — App state, panel rendering, boundary-gated edits, and headless pact/refresh/check/scope/config/stale/fresh commands, tied together by main.rs and lib.rs.

## Structure

- Manifest for the warlock-tui crate: builds the `warlock` binary from src/main.rs and the warlock_tui library from src/lib.rs, wiring clap, ratatui, notify, ctrlc and warlock-engine.
