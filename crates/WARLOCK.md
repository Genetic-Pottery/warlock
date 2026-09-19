<!-- warlock -->
> Written by a model pass over this directory alone, to be read before its source and to say which source to read. A map, not a specification: check anything you are about to rely on against the files themselves, and where this document and the code disagree, the code is right.

# crates

Cargo workspace crates directory holding the engine library (module tree, freshness, JSON document contract, documentation generation) and the front-end crate (the `warlock` binary, CLI subcommands, terminal UI, claude agent).

## Directories

- `warlock-engine/` — Engine library crate: tree loading, freshness decisions, hashing, pact/refresh drivers, document fill and rendering, config, routing. Go here for core logic and the JSON document contract.
- `warlock-tui/` — Front-end crate: the `warlock` binary, clap CLI subcommands, ratatui terminal UI, and the claude child-process agent. Go here for command handling, UI behaviour and agent invocation.

## Structure

- warlock-tui depends on warlock-engine for its underlying logic; the engine does not depend on the front-end.
