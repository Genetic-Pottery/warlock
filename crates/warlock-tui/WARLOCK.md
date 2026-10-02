<!-- warlock -->
> Written by a model pass over this directory alone, to be read before its source and to say which source to read. A map, not a specification: check anything you are about to rely on against the files themselves, and where this document and the code disagree, the code is right.

# warlock-tui

The warlock-tui crate: a binary (warlock) and library (warlock_tui) providing the terminal UI and CLI, with App state driving tree/panel rendering and the pact, pull, push, cut, draft and refresh subcommand flows against a repo's manifest and boundary.

## Files

- `Cargo.toml` (2.2 KB) — Cargo.toml: warlock-tui package manifest — bin "warlock" (src/main.rs) and lib warlock_tui (src/lib.rs), deps on warlock-engine, ratatui, clap, notify.

## Directories

- `src/` — Crate root holding the App state machine, rendering, and all subcommand flows; go here for any question about CLI commands, TUI screens, pact/pull/push/cut/draft/refresh behavior, or Linear/Claude integration.

## Structure

- Cargo.toml: warlock-tui package manifest — bin "warlock" (src/main.rs) and lib warlock_tui (src/lib.rs), deps on warlock-engine, ratatui, clap, notify.
