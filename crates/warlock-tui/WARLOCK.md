<!-- warlock -->
> Written by a model pass over this directory alone, to be read before its source and to say which source to read. A map, not a specification: check anything you are about to rely on against the files themselves, and where this document and the code disagree, the code is right.

# warlock-tui

The warlock-tui crate: builds the warlock binary and the warlock_tui library, the terminal-free front end and CLI for pact/refresh/scope/view/write/chat, wired to clap, ratatui, notify, ctrlc and warlock-engine.

## Files

- `Cargo.toml` (1.6 KB) — Manifest for the warlock-tui crate: builds the `warlock` binary from src/main.rs and the warlock_tui library from src/lib.rs, wiring clap, ratatui, notify, ctrlc and warlock-engine.

## Directories

- `src/` — The terminal-free front end: flattened tree App, panel cards, key handling, the shared boundary check, and CLI subcommands — go here for how a keypress or subcommand is handled.

## Structure

- Manifest for the warlock-tui crate: builds the warlock binary from src/main.rs and the warlock_tui library from src/lib.rs, wiring clap, ratatui, notify, ctrlc and warlock-engine.
