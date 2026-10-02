<!-- warlock -->
> Written by a model pass over this directory alone, to be read before its source and to say which source to read. A map, not a specification: check anything you are about to rely on against the files themselves, and where this document and the code disagree, the code is right.

# crates

The crates directory holds the workspace's two Rust crates: warlock-engine, the freshness ledger engine, and warlock-tui, the terminal UI and CLI built on it.

## Directories

- `warlock-engine/` — The engine crate: hashing/scoping, agent-driven document and ticket generation, and pact/pull/filed-ticket tracking; open for how the engine works.
- `warlock-tui/` — The TUI/CLI crate: App state, rendering, and pact/pull/push/cut/draft/refresh subcommand flows; open for CLI or TUI behavior.
