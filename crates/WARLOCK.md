<!-- warlock -->
> Written by a model pass over this directory alone, to be read before its source and to say which source to read. A map, not a specification: check anything you are about to rely on against the files themselves, and where this document and the code disagree, the code is right.

# crates

Workspace root holding the two crates that make up the tool: warlock-engine for repo-walking and document/pact logic, and warlock-tui for the terminal front end and headless commands.

## Directories

- `warlock-engine/` — The engine crate: tree walking, pact/manifest state, WARLOCK.md fill/mend/render, scope and sigil logic, no TUI/terminal/HTTP/Anthropic deps — go here for core logic questions.
- `warlock-tui/` — The front-end crate: builds the warlock binary and warlock_tui library, panel rendering and headless subcommands atop warlock-engine — go here for CLI or panel questions.
