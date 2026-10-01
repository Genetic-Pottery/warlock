<!-- warlock -->
> Written by a model pass over this directory alone, to be read before its source and to say which source to read. A map, not a specification: check anything you are about to rely on against the files themselves, and where this document and the code disagree, the code is right.

# crates

The crates directory holds warlock's two Cargo packages: the warlock-engine library with the core repo/pact/draft/pull logic, and the warlock-tui package building the warlock binary and warlock_tui library that drive the CLI and TUI.

## Directories

- `warlock-engine/` — Core library crate for walking/hashing repo trees, pacted freshness, LLM document fills, scopes, sigils, keys, pulls, drafting and ticket-splitting/filing — open for how any of that logic works.
- `warlock-tui/` — Package building the warlock binary and warlock_tui library: the TUI app, CLI, brief/draft/push/pull flows, and git/Claude/Linear integrations — open for how a command or screen works.
