<!-- warlock -->
> Written by a model pass over this directory alone, to be read before its source and to say which source to read. A map, not a specification: check anything you are about to rely on against the files themselves, and where this document and the code disagree, the code is right.

# warlock-tui

Front-end crate that builds the `warlock` binary and the warlock_tui library: the clap CLI subcommands, the ratatui terminal UI, and the claude child-process agent. Depends on warlock-engine for the underlying logic; all code is under src/.

## Files

- `Cargo.toml` (1.8 KB) — Cargo.toml for warlock-tui: builds the `warlock` binary from src/main.rs and the warlock_tui library from src/lib.rs, depending on arboard, clap, ctrlc, notify, ratatui, serde_json and warlock-engine.

## Directories

- `src/` — All crate source: clap subcommands in main.rs (init, config, stale, fresh, check, pact, refresh, scope, key), the ratatui UI (app, ui, panel, composer, thread), pact/refresh runs, and the claude agent in claude.rs.

## Structure

- Cargo.toml builds the `warlock` binary from src/main.rs and the warlock_tui library from src/lib.rs.
- The crate depends on warlock-engine plus arboard, clap, ctrlc, notify, ratatui and serde_json.
