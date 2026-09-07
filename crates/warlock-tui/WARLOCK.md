<!-- warlock -->
> Written by a model pass over this directory alone, to be read before its source and to say which source to read. A map, not a specification: check anything you are about to rely on against the files themselves, and where this document and the code disagree, the code is right.

# warlock-tui

warlock-tui is the front-end crate of warlock's product: the warlock binary shipping the terminal panel and its headless subcommands (pact, refresh, check, config, scope), built over the pure warlock_tui library of tree/panel/account/thread state.

## Files

- `Cargo.toml` (1.6 KB) — Manifest: binary warlock at src/main.rs, library warlock_tui at src/lib.rs, dependency edge runs TUI -> warlock-engine, ctrlc only for headless runs, tempfile as dev-dependency for config tests.

## Directories

- `src/` — The crate's own root and every module: App/Panel/Thread state, the event loop, boundary and descent gating, headless subcommands, rendering — open for any question about front-end behavior.

## Rules

- The executable name is warlock; the library name is warlock_tui.
- The dependency direction is TUI -> engine: the front end knows domain vocabulary, the engine knows nothing about terminals.
- ctrlc is pressed into service only by headless runs, only from src/running.rs.
- tempfile is a dev-dependency only, used to keep warlock config tests off the developer's real home directory.
- Lint configuration is inherited from the workspace root manifest, not set here.

## Where to look

- what binary and library names does this crate produce → `Cargo.toml` `warlock_tui`
- why does the crate depend on ctrlc → `Cargo.toml` `ctrlc`
- any question about panel state, key handling, or rendering → `src` `App`
- how headless config tests avoid touching a real home directory → `Cargo.toml` `tempfile`
