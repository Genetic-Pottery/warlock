<!-- warlock -->
> Written by a model pass over this directory alone, to be read before its source and to say which source to read. A map, not a specification: check anything you are about to rely on against the files themselves, and where this document and the code disagree, the code is right.

# crates

The crates directory holds the workspace's two Rust crates, warlock-engine and warlock-tui, that together implement the repo-modeling engine and the TUI/CLI built on it.

## Directories

- `warlock-engine/` — Crate modeling a repo as a Tree of pacted/unpacted modules, hashing/viewing files, agent-driven WARLOCK.md fill/validation, and scope/sigil enforcement.
- `warlock-tui/` — Crate providing the warlock binary and warlock_tui library: TUI/CLI for pacting, drafting, filing/pulling tickets, and rendering the freshness ledger.

## Structure

- warlock-engine holds the crate core: Tree model, hashing, agent-driven document fill/validation, and scope/sigil enforcement.
- warlock-tui builds the warlock binary from src/main.rs and the warlock_tui library from src/lib.rs, depending on warlock-engine.
