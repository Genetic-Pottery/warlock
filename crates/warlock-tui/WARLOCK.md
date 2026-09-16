<!-- warlock -->
> Written by a model pass over this directory alone, to be read before its source and to say which source to read. A map, not a specification: check anything you are about to rely on against the files themselves, and where this document and the code disagree, the code is right.

# warlock-tui

warlock-tui builds the `warlock` binary: the terminal front end over warlock-engine, with App state, Panel/Composer/Thread cards, the boundary check shared by keys and headless doors, and subcommand dispatch for pact/refresh/unpact/check/config/scope/stale/fresh.

## Files

- `Cargo.toml` (1.8 KB) — Manifest for warlock-tui, building the `warlock` binary from src/main.rs over the warlock_tui library; depends on warlock-engine, arboard, ctrlc, ratatui.

## Directories

- `src/` — The crate root: App, Panel, boundary check, subcommand dispatch and terminal/watch/drawing code; go here for any front-end or CLI question.

## Structure

- Manifest for warlock-tui, building the `warlock` binary from src/main.rs over the warlock_tui library; depends on warlock-engine, arboard, ctrlc, ratatui.
