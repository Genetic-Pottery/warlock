<!-- warlock -->
> Written by a model pass over this directory alone, to be read before its source and to say which source to read. A map, not a specification: check anything you are about to rely on against the files themselves, and where this document and the code disagree, the code is right.

# warlock-tui

warlock-tui is the crate that ships the `warlock` binary: the terminal front end (panel, tree, composer, conversation cards) plus every CLI subcommand (check, config, pact, refresh, stale, fresh, unpact, scope add/remove), built on top of warlock-engine's domain types.

## Files

- `Cargo.toml` (1.6 KB) — Crate manifest: binary `warlock` (src/main.rs), library `warlock_tui` (src/lib.rs), dependencies clap/ctrlc/notify/ratatui/serde_json/warlock-engine, dev-dependency tempfile, workspace lints.

## Directories

- `src/` — The front end and CLI's source: pure state/event modules, the impure claude and watch seams, all behind lib.rs's re-exports — go here for any panel, key-handling, subcommand or process-spawning question.

## Structure

- The dependency edge runs TUI -> engine: warlock-tui knows terminal and domain vocabulary, warlock-engine knows neither.
- The library exposes the pure app-state and colour logic as ordinary reachable API, while src/main.rs is the thin binary shell around it.
- Only headless runs in src/running.rs use ctrlc, since a warlock pact has no panel to press Esc in.
- Tests write a real config file into a throwaway home directory using tempfile, matching the engine's explicit home parameter.

## Rules

- The crate is named warlock-tui but the binary it ships is named `warlock`.
- Lint configuration is inherited from the workspace root rather than set locally.

## Where to look

- which files make up the terminal UI versus the CLI subcommands → `src` `main.rs`
- why ctrlc is a dependency here at all → `Cargo.toml` `ctrlc`
- how tests avoid touching a developer's real config → `Cargo.toml` `tempfile`
