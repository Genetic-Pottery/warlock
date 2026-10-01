<!-- warlock -->
> Written by a model pass over this directory alone, to be read before its source and to say which source to read. A map, not a specification: check anything you are about to rely on against the files themselves, and where this document and the code disagree, the code is right.

# warlock-tui

The warlock-tui crate: a terminal UI state machine, event loop, and rendering plus the CLI subcommands (pact, push, pull, cut, check, key, list, resume) that drive the manifest, boundary, brief, and Linear-filing logic.

## Files

- `Cargo.toml` (2.2 KB) — Cargo.toml: warlock-tui package manifest — bin "warlock" (src/main.rs) and lib warlock_tui (src/lib.rs), deps on warlock-engine, ratatui, clap, notify.

## Directories

- `src/` — The crate's source, including the state machine, event loop, rendering, and CLI subcommands; go there for how any TUI or CLI behavior is implemented.

## Structure

- Cargo.toml defines the "warlock" binary at src/main.rs and the warlock_tui library at src/lib.rs.
- The package depends on warlock-engine, ratatui, clap, and notify.
