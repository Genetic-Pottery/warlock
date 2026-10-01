<!-- warlock -->
> Written by a model pass over this directory alone, to be read before its source and to say which source to read. A map, not a specification: check anything you are about to rely on against the files themselves, and where this document and the code disagree, the code is right.

# warlock-tui

The warlock-tui crate: a terminal UI and CLI binary (warlock) for filing briefs to Linear, pulling tickets through agent-run work, and maintaining a repo's freshness ledger across scopes and sigils.

## Files

- `Cargo.toml` (2.2 KB) — Cargo.toml: warlock-tui package manifest — bin "warlock" (src/main.rs) and lib warlock_tui (src/lib.rs), deps on warlock-engine, ratatui, clap, notify.

## Directories

- `src/` — All crate source: entrypoint, TUI event loop and rendering, Linear/git/claude clients, pull/push/draft flows, and boundary/scope logic.

## Structure

- Cargo.toml: warlock-tui package manifest — bin "warlock" (src/main.rs) and lib warlock_tui (src/lib.rs), deps on warlock-engine, ratatui, clap, notify.
