<!-- warlock -->
> Written by a model pass over this directory alone, to be read before its source and to say which source to read. A map, not a specification: check anything you are about to rely on against the files themselves, and where this document and the code disagree, the code is right.

# warlock-tui

The warlock-tui crate: builds the warlock binary and the warlock_tui library, the terminal front end for warlock's pact state, plus the headless subcommands sharing its boundary and session logic.

## Files

- `Cargo.toml` (1.6 KB) — Manifest for the warlock-tui crate: builds the `warlock` binary from src/main.rs and the warlock_tui library from src/lib.rs, wiring clap, ratatui, notify, ctrlc and warlock-engine.

## Directories

- `src/` — The panel, keypress handling, filesystem watching, and headless subcommands (check, config, stale, fresh, pact, refresh, unpact, scope); go there for how a key or run becomes screen state or a subcommand's output.

## Structure

- Manifest for the warlock-tui crate: builds the `warlock` binary from src/main.rs and the warlock_tui library from src/lib.rs, wiring clap, ratatui, notify, ctrlc and warlock-engine.

## Where to look

- what crate builds the warlock binary → `Cargo.toml` `warlock`
- what dependencies does the tui pull in → `Cargo.toml`
- where is the terminal-free library surface defined → `Cargo.toml` `warlock_tui`
