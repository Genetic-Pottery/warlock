<!-- warlock -->
> Written by a model pass over this directory alone, to be read before its source and to say which source to read. A map, not a specification: check anything you are about to rely on against the files themselves, and where this document and the code disagree, the code is right.

# warlock-tui

The warlock-tui package: builds the `warlock` binary and warlock_tui library, the TUI application and CLI driving scope/pact/refresh, briefs, drafting, pushing to Linear and pulling tickets, plus the git/Claude agent integrations those flows need.

## Files

- `Cargo.toml` (2.2 KB) — Cargo.toml: warlock-tui package manifest — bin "warlock" (src/main.rs) and lib warlock_tui (src/lib.rs), deps on warlock-engine, ratatui, clap, notify.

## Directories

- `src/` — The crate's source: TUI app, CLI, brief/draft/push/pull flows, and git/Claude/Linear integrations — open it for how any command or screen works.

## Structure

- Cargo.toml: warlock-tui package manifest — bin "warlock" (src/main.rs) and lib warlock_tui (src/lib.rs), deps on warlock-engine, ratatui, clap, notify.
