<!-- warlock -->
> Written by a model pass over this directory alone, to be read before its source and to say which source to read. A map, not a specification: check anything you are about to rely on against the files themselves, and where this document and the code disagree, the code is right.

# crates

The workspace's two crates: warlock-engine, the freshness ledger engine, and warlock-tui, the interactive TUI and CLI built on it.

## Directories

- `warlock-engine/` — The freshness ledger engine crate: hashing/scoping, agent-driven document and ticket generation, and pact/pull/filed-ticket tracking; go there for how the engine itself works.
- `warlock-tui/` — The warlock binary and warlock_tui library: TUI and CLI for pact, refresh, scope, push, cut, pull, chat/brief, plus Linear and git/gh clients; go there for any specific command or UI piece.
