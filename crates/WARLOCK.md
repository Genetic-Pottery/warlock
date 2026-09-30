<!-- warlock -->
> Written by a model pass over this directory alone, to be read before its source and to say which source to read. A map, not a specification: check anything you are about to rely on against the files themselves, and where this document and the code disagree, the code is right.

# crates

The workspace's crates directory: holds the warlock-engine and warlock-tui Rust crates that together implement the repo-modeling engine and the warlock TUI/CLI binary.

## Directories

- `warlock-engine/` — Engine crate: models a repo as a Tree, hashes/views files, fills and validates WARLOCK.md, enforces scope/sigil boundaries — go here for engine behavior.
- `warlock-tui/` — TUI/CLI crate: builds the warlock binary and warlock_tui library driving pact/refresh/scope/pull/push/cut/draft — go here for command or view implementation.
