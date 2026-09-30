<!-- warlock -->
> Written by a model pass over this directory alone, to be read before its source and to say which source to read. A map, not a specification: check anything you are about to rely on against the files themselves, and where this document and the code disagree, the code is right.

# crates

The crates directory holding the two Rust crates that make up the project: warlock-engine, the repo-modeling and document-driving core, and warlock-tui, the terminal UI and CLI built on it.

## Directories

- `warlock-engine/` — Crate modeling a repo as a Tree of pacted/unpacted modules, hashing/viewing files, driving agent-filled WARLOCK.md documents, and enforcing scope/sigil boundaries — go here for how the engine works.
- `warlock-tui/` — Crate building the `warlock` binary and `warlock_tui` library, the terminal UI and CLI for pact/refresh/pull/cut/push/chat/brief flows — go here for how a flow, screen or command works.
