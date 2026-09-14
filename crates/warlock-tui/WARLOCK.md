<!-- warlock -->
> Written by a model pass over this directory alone, to be read before its source and to say which source to read. A map, not a specification: check anything you are about to rely on against the files themselves, and where this document and the code disagree, the code is right.

# warlock-tui

The warlock-tui crate: the terminal front end shipping the `warlock` binary, built as a pure library (state, panel cards, composer, conversation, run account) plus the impure claude subprocess and filesystem watch, wired so the screen, keys and pact runs can be tested without a terminal; depends on warlock-engine for the domain.

## Files

- `Cargo.toml` (1.6 KB) — Manifest for warlock-tui: bin `warlock` (src/main.rs), lib `warlock_tui` (src/lib.rs), deps clap/ctrlc/notify/ratatui/serde_json/warlock-engine, dev-dep tempfile, workspace lints.

## Directories

- `src/` — The warlock_tui library and binary source: tree/panel/composer/conversation state, the claude and watch impurities, and every headless subcommand — go here for keys, colours, boundary checks, or subcommand wording.

## Structure

- The dependency edge runs TUI -> engine: warlock-tui knows terminal and domain vocabulary, warlock-engine knows neither terminals nor this crate.
- The bin target `warlock` at src/main.rs is a thin shell around the lib target `warlock_tui` at src/lib.rs, which both the binary and tests use.
- Only headless runs from src/running.rs press ctrlc into service, since a `warlock pact` run has no panel to press Esc in.
- tempfile is a dev-dependency used by the `warlock config` tests to write a real config file under a throwaway home directory.

## Rules

- Lint configuration lives in the root manifest's [workspace.lints] so every crate is held to the same bar.

## Where to look

- which key does what in the terminal UI → `src` `action_for`
- how claude is invoked as a subprocess → `src` `ClaudeAgent`
- why an operator is refused at a directory (the boundary) → `src` `verdict`
- building or running the warlock binary → `Cargo.toml` `warlock`
- why Ctrl-C exists in a headless run → `Cargo.toml` `ctrlc`
