<!-- warlock -->
> Written by a model pass over this directory alone, to be read before its source and to say which source to read. A map, not a specification: check anything you are about to rely on against the files themselves, and where this document and the code disagree, the code is right.

# crates

The crates directory holds the two Rust crates that make up the project: warlock-engine, the core engine, and warlock-tui, the binary and CLI/TUI built on top of it.

## Directories

- `warlock-engine/` — The engine crate — models the repo as a Tree, hashes/views files, drives agent-driven document fill/validation, and enforces scope/sigil boundaries; go here for how engine behavior works.
- `warlock-tui/` — The TUI crate — builds the `warlock` binary and `warlock_tui` library over warlock-engine, with pact/refresh/scope walks, chat/brief/draft/cut/pull/push flows, and rendering; go here for how any warlock command or screen works.

## Structure

- warlock-engine is the crate that models a repo as a Tree of pacted/unpacted modules, hashes and views files, drives agent-driven document fill/validation, and enforces scope/sigil boundaries for pulls, drafting and splitting
- warlock-tui depends on warlock-engine
