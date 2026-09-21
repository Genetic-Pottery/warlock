<!-- warlock -->
> Written by a model pass over this directory alone, to be read before its source and to say which source to read. A map, not a specification: check anything you are about to rely on against the files themselves, and where this document and the code disagree, the code is right.

# warlock-tui

Cargo crate for the warlock command-line tool and its ratatui terminal panel. Builds the `warlock` binary and the `warlock_tui` library; ask here about subcommands, the panel front end, chat, push to Linear, scopes and sigils.

## Files

- `Cargo.toml` (2.2 KB) — Cargo manifest for the warlock-tui crate: builds the `warlock` binary (src/main.rs) and the `warlock_tui` library (src/lib.rs); depends on warlock-engine, ratatui, clap, notify, arboard, ureq, ctrlc, serde_json.

## Directories

- `src/` — All crate source: main.rs entry and subcommands, panel state, input, drawing, chat, push, scope prompts, Claude and Linear clients, boundary logic. Go here for any code question.

## Structure

- Cargo.toml builds the `warlock` binary from src/main.rs and the `warlock_tui` library from src/lib.rs.
- The crate depends on warlock-engine for the core logic and on ratatui, clap, notify, arboard, ureq, ctrlc and serde_json for terminal UI, argument parsing, watching, clipboard, HTTP, signals and JSON.
