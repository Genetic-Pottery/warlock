<!-- warlock -->
> Written by a model pass over this directory alone, to be read before its source and to say which source to read. A map, not a specification: check anything you are about to rely on against the files themselves, and where this document and the code disagree, the code is right.

# warlock-tui

The warlock-tui crate: a Cargo package building the `warlock` binary and the warlock_tui library, which together provide the interactive TUI and CLI entry points for pact, refresh, scope, push, cut, pull and chat/brief flows, plus the Linear and git/gh clients they call through.

## Files

- `Cargo.toml` (2.2 KB) — Cargo.toml: warlock-tui package manifest — bin "warlock" (src/main.rs) and lib warlock_tui (src/lib.rs), deps on warlock-engine, ratatui, clap, notify.

## Directories

- `src/` — The interactive TUI, CLI entry points and engine driving pact, refresh, scope, push, cut, pull, chat/brief and the Linear/git/gh clients; open it for how any specific command or UI piece works.

## Structure

- Cargo.toml: warlock-tui package manifest — bin "warlock" (src/main.rs) and lib warlock_tui (src/lib.rs), deps on warlock-engine, ratatui, clap, notify.
