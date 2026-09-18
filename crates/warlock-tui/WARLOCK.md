<!-- warlock -->
> Written by a model pass over this directory alone, to be read before its source and to say which source to read. A map, not a specification: check anything you are about to rely on against the files themselves, and where this document and the code disagree, the code is right.

# warlock-tui

The warlock-tui crate: builds the `warlock` binary and warlock_tui library — panel/app state, boundary and scope enforcement, agent chat driving /pact /refresh /brief /write, and terminal rendering.

## Files

- `Cargo.toml` (1.8 KB) — Cargo.toml for warlock-tui: builds the `warlock` binary from src/main.rs and the warlock_tui library from src/lib.rs, depending on arboard, clap, ctrlc, notify, ratatui, serde_json and warlock-engine.

## Directories

- `src/` — Front end and CLI source — panel/app state, boundary checks, chat, rendering; go here for how a key press, command or frame is handled.

## Structure

- Cargo.toml for warlock-tui: builds the `warlock` binary from src/main.rs and the warlock_tui library from src/lib.rs.
- Depends on arboard, clap, ctrlc, notify, ratatui, serde_json and warlock-engine.
