<!-- warlock -->
> Written by a model pass over this directory alone, to be read before its source and to say which source to read. A map, not a specification: check anything you are about to rely on against the files themselves, and where this document and the code disagree, the code is right.

# warlock-tui

The warlock-tui crate's Cargo manifest: builds the `warlock` binary from src/main.rs and the `warlock_tui` library from src/lib.rs, depending on warlock-engine, ratatui, clap, notify, arboard, ureq, ctrlc and serde_json.

## Files

- `Cargo.toml` (2.2 KB) — Cargo manifest for the warlock-tui crate: builds the `warlock` binary (src/main.rs) and the `warlock_tui` library (src/lib.rs); depends on warlock-engine, ratatui, clap, notify, arboard, ureq, ctrlc, serde_json.

## Directories

- `src/` — The crate's source: TUI and CLI for pacting/refreshing scopes, chatting/drafting briefs, filing/pulling Linear tickets, and rendering the freshness ledger; go there for how any command or view works.

## Structure

- Cargo.toml defines the warlock binary built from src/main.rs and the warlock_tui library built from src/lib.rs.
- Cargo.toml declares the crate's dependencies: warlock-engine, ratatui, clap, notify, arboard, ureq, ctrlc, serde_json.
