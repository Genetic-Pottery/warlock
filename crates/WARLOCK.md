<!-- warlock -->
> Written by a model pass over this directory alone, to be read before its source and to say which source to read. A map, not a specification: check anything you are about to rely on against the files themselves, and where this document and the code disagree, the code is right.

# crates

crates is the workspace root holding the two crates that make up the project: warlock-engine, the domain/lifecycle core, and warlock-tui, the terminal front end and CLI that ships the `warlock` binary.

## Directories

- `warlock-engine/` — The pact/refresh/document lifecycle core: Agent port, manifest/scope/sigil schemas, tree/hash/freshness machinery — go here for domain logic free of any TUI dependency.
- `warlock-tui/` — The terminal front end and CLI that ships the `warlock` binary — go here for panel, key-handling, subcommand or process-spawning questions.

## Structure

- The dependency edge runs one way: warlock-tui depends on warlock-engine, never the reverse.
- warlock-engine defines the domain and lifecycle types that warlock-tui's CLI subcommands and panel operate on.

## Rules

- warlock-engine forbids TUI, terminal, HTTP or Anthropic dependencies, keeping the dependency edge one-directional.
- Lint configuration is shared from the workspace root manifest across both crates.

## Where to look

- where the core lifecycle logic for pact/refresh/document lives → `warlock-engine` `pact_subtree`
- where the terminal UI and CLI subcommands live → `warlock-tui` `main.rs`
- why a dependency is or isn't allowed in the core crate → `warlock-engine` `Cargo.toml`
- how the `warlock` binary is built and what it depends on → `warlock-tui` `Cargo.toml`
