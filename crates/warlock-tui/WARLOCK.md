<!-- warlock -->
> Written by a model pass over this directory alone, to be read before its source and to say which source to read. A map, not a specification: check anything you are about to rely on against the files themselves, and where this document and the code disagree, the code is right.

# warlock-tui

The warlock-tui crate: builds the `warlock` binary and the warlock_tui library, providing the TUI application, CLI commands, and the engine driving pact/refresh/scope/pull/push/cut/draft flows against a repo's manifest and a Linear board.

## Files

- `Cargo.toml` (2.2 KB) — Cargo manifest for the warlock-tui crate: builds the `warlock` binary (src/main.rs) and the `warlock_tui` library (src/lib.rs); depends on warlock-engine, ratatui, clap, notify, arboard, ureq, ctrlc, serde_json.

## Directories

- `src/` — The crate's source: TUI application, CLI commands, and the engine behind pact/refresh/scope/pull/push/cut/draft; consult it for any question about how a specific command or view is implemented.

## Structure

- Cargo manifest for the warlock-tui crate: builds the `warlock` binary (src/main.rs) and the `warlock_tui` library (src/lib.rs).
- Depends on warlock-engine, ratatui, clap, notify, arboard, ureq, ctrlc, serde_json.
