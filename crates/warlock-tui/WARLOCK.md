<!-- warlock -->
> Written by a model pass over this directory alone, to be read before its source and to say which source to read. A map, not a specification: check anything you are about to rely on against the files themselves, and where this document and the code disagree, the code is right.

# warlock-tui

The warlock-tui crate: the workspace manifest for warlock's terminal front end and its binary, wiring the crate name warlock_tui and its bin warlock to a src that depends on warlock-engine and never the reverse.

## Files

- `Cargo.toml` (1.6 KB) — Crate manifest: declares the warlock_tui lib and warlock binary, its dependency on warlock-engine, ctrlc for headless Ctrl-C, tempfile as a dev-dependency for config tests, and workspace lints.

## Directories

- `src/` — The crate's whole implementation — tree/panel UI, model seams, boundary and confirmation prompts, and the headless subcommands — go here for any question about behaviour or code.

## Structure

- Cargo.toml declares src/main.rs as the warlock binary and src/lib.rs as the warlock_tui library
- The dependency edge runs TUI -> engine: this crate depends on warlock-engine, not the reverse
- tempfile is a dev-dependency only, used by config tests that need a throwaway home directory

## Rules

- The dependency edge runs TUI -> engine: the front end knows the domain vocabulary, the engine knows nothing about terminals
- ctrlc is pressed into service only by headless runs from src/running.rs, since a pact run has no panel to press Esc in
- tempfile keeps warlock config tests off the developer's real home directory
- Lint configuration lives in the root manifest's [workspace.lints], not here

## Where to look

- what binary does this crate produce and where is its entry point → `Cargo.toml` `warlock`
- why does a headless pact run need a Ctrl-C dependency → `Cargo.toml` `ctrlc`
- how do the warlock config tests avoid touching a real home directory → `Cargo.toml` `tempfile`
- where does the actual TUI and subcommand logic live → `src` `App`
