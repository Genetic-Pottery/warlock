<!-- warlock -->
> Written by a model pass over this directory alone, to be read before its source and to say which source to read. A map, not a specification: check anything you are about to rely on against the files themselves, and where this document and the code disagree, the code is right.

# warlock-tui

The warlock-tui crate: the terminal front end of warlock, shipping the `warlock` binary and the warlock_tui library of pure state (App, Account, Thread, Panel, Composer, colours, wrapping) that the binary's main.rs assembles into the running panel.

## Files

- `Cargo.toml` (1.6 KB) — Package manifest for warlock-tui: names the `warlock` binary (src/main.rs) and the warlock_tui library (src/lib.rs), depends on warlock-engine plus clap, ctrlc, notify, ratatui, serde_json, and pulls tempfile only for warlock-config's headless tests.

## Directories

- `src/` — The front end minus the terminal: pure tree/panel/composer/scope-prompt state, colour and wrap logic, the claude.rs child-process adapter and watch.rs filesystem watcher, all assembled by main.rs.

## Structure

- Cargo.toml's [lib] path points at src/lib.rs, which re-exports the pure state both the binary and tests use
- Cargo.toml's [[bin]] path points at src/main.rs, the thin shell wrapping the library
- dependency edge runs warlock-tui -> warlock-engine: the front end knows domain vocabulary, the engine knows nothing of terminals
- ctrlc is pressed into service only by src/running.rs for headless `warlock pact` runs
- tempfile is a dev-dependency only, used by warlock-config tests writing to a throwaway home directory

## Rules

- the executable ships as `warlock` even though the crate is named warlock-tui
- the library exposes the pure state-to-colour and app-state logic as ordinary reachable API, not crate-private code
- the TUI-to-engine dependency direction is fixed: the engine must know nothing about terminals
- ctrlc is only ever pulled in from src/running.rs, nowhere else in the crate
- lint configuration is not set here; it is inherited from the workspace's [workspace.lints]

## Where to look

- what binary does this crate produce and where does it start → `src` `main.rs`
- why Ctrl-C exists as a dependency here → `Cargo.toml` `ctrlc`
- which crate the TUI depends on for domain logic → `Cargo.toml` `warlock-engine`
- how config tests avoid touching a real home directory → `Cargo.toml` `tempfile`
- the pure state, colours, wrapping and panel logic behind the binary → `src` `lib.rs`
