<!-- warlock -->
> Written by a model pass over this directory alone, to be read before its source and to say which source to read. A map, not a specification: check anything you are about to rely on against the files themselves, and where this document and the code disagree, the code is right.

# warlock-tui

The warlock-tui crate: packages the `warlock` CLI binary and warlock_tui library, pulling in warlock-engine, ratatui, clap and notify to run the interactive TUI and its pacting, briefing, drafting, pulling and pushing engine.

## Files

- `Cargo.toml` (2.2 KB) — Cargo.toml: warlock-tui package manifest — bin "warlock" (src/main.rs) and lib warlock_tui (src/lib.rs), deps on warlock-engine, ratatui, clap, notify.

## Directories

- `src/` — The crate source: CLI entrypoint, TUI event loop, rendering, and the pact/brief/draft/pull/push engine against a repo's boundary scopes; go here for any command or state-machine question.

## Structure

- Cargo.toml: warlock-tui package manifest — bin "warlock" (src/main.rs) and lib warlock_tui (src/lib.rs), deps on warlock-engine, ratatui, clap, notify.
