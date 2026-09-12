<!-- warlock -->
> Written by a model pass over this directory alone, to be read before its source and to say which source to read. A map, not a specification: check anything you are about to rely on against the files themselves, and where this document and the code disagree, the code is right.

# warlock-tui

The warlock-tui crate: the terminal front end and binary of warlock, holding the panel's pure state (accounts, threads, app tree, composer, wrapping, drawing) plus the impure seams (claude child process, filesystem watch, editor, subcommands) that the warlock binary drives.

## Files

- `Cargo.toml` (1.6 KB) — Crate manifest: warlock-tui library warlock_tui plus the warlock binary; declares the TUI-to-engine dependency edge and dev-only tempfile use for config tests.

## Directories

- `src/` — The library and binary source: pure front-end state, panel cards, boundary checks, and the claude/watch impure seams — open for any question about a specific type, key or subcommand.

## Structure

- Cargo.toml's [[bin]] warlock at src/main.rs depends on the [lib] warlock_tui at src/lib.rs
- warlock-tui depends on warlock-engine; the dependency runs TUI -> engine, never the reverse
- ctrlc is pressed into service only by src/running.rs for headless `warlock pact` runs
- tempfile is a dev-dependency used only by the `warlock config` tests

## Rules

- The crate is named warlock-tui but the executable it ships is named warlock
- Lint configuration is not set here; it comes from [workspace.lints] in the root manifest

## Where to look

- what binary does this crate produce → `Cargo.toml` `warlock`
- why is ctrlc a dependency → `Cargo.toml` `ctrlc`
- any specific TUI type, key handler or subcommand → `src` `App`
