<!-- warlock -->
> Written by a model pass over this directory alone, to be read before its source and to say which source to read. A map, not a specification: check anything you are about to rely on against the files themselves, and where this document and the code disagree, the code is right.

# warlock-tui

The warlock-tui crate: the terminal front end that shows and edits a warlock tree, packaged as the `warlock` binary with a library of pure state, panel cards, boundary/write logic and rendering behind it.

## Files

- `Cargo.toml` (1.6 KB) — Crate manifest: binary is `warlock` (src/main.rs), library `warlock_tui` (src/lib.rs), depends on warlock-engine and ratatui; the TUI->engine dependency edge and lint-inheritance are noted in comments.

## Directories

- `src/` — All source: App state, key/mouse input, panel cards, boundary/write logic, rendering, model-seam agents, and the binary's headless subcommands.

## Structure

- The binary (src/main.rs) depends on the library (src/lib.rs), which re-exports the crate's API
- The crate depends on warlock-engine for domain vocabulary; the engine has no terminal knowledge
- ctrlc is used only by headless runs (src/running.rs) since a pact run has no panel to press Esc in
- tempfile is a dev-dependency for `warlock config` tests needing a throwaway home directory, off the engine's explicit home parameter

## Rules

- The executable name is `warlock`, distinct from the crate/library name `warlock-tui`/`warlock_tui`
- Lint configuration is inherited from the workspace root manifest, not set here
- ctrlc, notify, serde_json, ratatui and warlock-engine are the only non-dev dependencies

## Where to look

- what does the binary actually do and where does its source live → `src` `main`
- why is ctrlc a dependency at all → `Cargo.toml` `ctrlc`
- how config tests avoid touching the real home directory → `Cargo.toml` `tempfile`
