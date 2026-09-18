<!-- warlock -->
> Written by a model pass over this directory alone, to be read before its source and to say which source to read. A map, not a specification: check anything you are about to rely on against the files themselves, and where this document and the code disagree, the code is right.

# crates

The workspace's two crates: warlock-engine, the core library for tree loading, hashing, the pact engine and document generation, and warlock-tui, the front end and CLI that builds the `warlock` binary atop it.

## Directories

- `warlock-engine/` — Core crate — Agent boundary, tree loading and hashing, the two-phase pact engine, WARLOCK.md/pacts.toml generation and repair; go here for freshness logic.
- `warlock-tui/` — Builds the `warlock` binary and warlock_tui library — panel state, boundary/scope enforcement, agent chat, terminal rendering; go here for CLI and UI behavior.
