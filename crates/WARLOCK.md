<!-- warlock -->
> Written by a model pass over this directory alone, to be read before its source and to say which source to read. A map, not a specification: check anything you are about to rely on against the files themselves, and where this document and the code disagree, the code is right.

# crates

Holds the two crates that make up the workspace: warlock-engine, the freshness-ledger core, and warlock-tui, the terminal front end that builds the warlock binary over it.

## Directories

- `warlock-engine/` — The engine crate: hashing, tree walking, manifest state, pact engine and document filling; go here for freshness-ledger or WARLOCK.md-generation questions.
- `warlock-tui/` — The front-end crate building the warlock binary: App state, Panel/Composer/Thread cards, boundary check and subcommand dispatch; go here for terminal UI or CLI questions.
