<!-- warlock -->
> Written by a model pass over this directory alone, to be read before its source and to say which source to read. A map, not a specification: check anything you are about to rely on against the files themselves, and where this document and the code disagree, the code is right.

# crates

Workspace holding the two Rust crates that make up the project: warlock-engine, the TUI/HTTP/Anthropic-free freshness-ledger engine, and warlock-tui, the terminal front end and CLI that depend on it.

## Directories

- `warlock-engine/` — The engine crate: hashing, tree walking, manifest state, pacted/unpacted tracking, and filling WARLOCK.md documents; kept free of TUI/terminal/HTTP/Anthropic deps.
- `warlock-tui/` — The terminal-free front end and CLI, builds the warlock binary and warlock_tui library for pact/refresh/scope/view/write/chat, depends on warlock-engine.
