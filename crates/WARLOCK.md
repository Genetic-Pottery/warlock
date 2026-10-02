<!-- warlock -->
> Written by a model pass over this directory alone, to be read before its source and to say which source to read. A map, not a specification: check anything you are about to rely on against the files themselves, and where this document and the code disagree, the code is right.

# crates

The crates directory holds the two Rust crates that make up the workspace: warlock-engine, the freshness ledger engine, and warlock-tui, the CLI and terminal interface built on it.

## Directories

- `warlock-engine/` — The freshness ledger engine: hashing/scoping, agent-driven document and ticket generation, pact/pull/filed-ticket tracking; go here for how the engine works.
- `warlock-tui/` — The warlock binary and library: CLI subcommands and the TUI event loop; go here for how any command or on-screen behavior is implemented.
