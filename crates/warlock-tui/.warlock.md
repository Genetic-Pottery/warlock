<!-- warlock -->
> Written by a model pass over this directory alone, to be read before its source and to say which source to read. A map, not a specification: check anything you are about to rely on against the files themselves, and where this document and the code disagree, the code is right.

# warlock-tui

The warlock-tui crate: packages the warlock binary and library, providing every CLI subcommand (init, config, pact, refresh, scope, key, brief, push, draft, pull, resume, check) plus the interactive terminal event loop.

## Files

- `Cargo.toml` (2.2 KB) — Cargo.toml: warlock-tui package manifest — bin "warlock" (src/main.rs) and lib warlock_tui (src/lib.rs), deps on warlock-engine, ratatui, clap, notify.

## Directories

- `src/` — The crate's source: subcommand handlers, the TUI event loop and rendering, and the Claude/Linear/git integrations — look here for how any command or on-screen behavior is implemented.

## Structure

- Cargo.toml defines the warlock-tui package manifest with bin "warlock" built from src/main.rs and lib warlock_tui built from src/lib.rs.
- Cargo.toml declares dependencies on warlock-engine, ratatui, clap, and notify.
