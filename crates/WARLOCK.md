<!-- warlock -->
> Written by a model pass over this directory alone, to be read before its source and to say which source to read. A map, not a specification: check anything you are about to rely on against the files themselves, and where this document and the code disagree, the code is right.

# crates

The crates directory is the workspace root for warlock's two Rust crates: warlock-engine, the core freshness-ledger engine, and warlock-tui, the terminal front end and warlock binary that depends on it.

## Directories

- `warlock-engine/` — The core engine crate: pacts, scopes, sigils, hashing, fitting and document rendering; open for questions about engine logic, types, or the Fill schema.
- `warlock-tui/` — The terminal front end and warlock binary: panel state, claude/watch seams, and subcommands; open for questions about a specific TUI type, key or subcommand.

## Structure

- warlock-tui depends on warlock-engine; the dependency runs TUI -> engine and never the reverse

## Rules

- The dependency edge between the two crates runs TUI -> engine and never back
- Lint configuration for both crates is inherited from the workspace root manifest, not set per-crate

## Where to look

- core engine logic, pacts, scopes, or sigils → `warlock-engine` `pact_directory`
- the warlock binary or a TUI type/key handler/subcommand → `warlock-tui` `App`
- why the TUI depends on the engine and not vice versa → `warlock-engine` `workspace`
