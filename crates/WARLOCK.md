<!-- warlock -->
> Written by a model pass over this directory alone, to be read before its source and to say which source to read. A map, not a specification: check anything you are about to rely on against the files themselves, and where this document and the code disagree, the code is right.

# crates

The crates directory holds warlock's two-crate workspace: warlock-engine, the domain core with no terminal or network dependency, and warlock-tui, the terminal front end and binary that depends on it.

## Directories

- `warlock-engine/` — The domain vocabulary for pacting a repository — manifest, freshness ledger, module document tree, agent seam, request fitting — go here for any question about engine behaviour with no TUI dependency.
- `warlock-tui/` — The terminal front end and warlock binary — tree/panel UI, headless subcommands, config — go here for any question about the running program or its interface.

## Structure

- warlock-tui depends on warlock-engine; the dependency edge runs TUI -> engine and never the reverse
- warlock-engine defines the domain types and logic that warlock-tui's binary consumes

## Where to look

- what is the core domain logic with no terminal or network dependency → `warlock-engine` `pact_subtree`
- what produces the warlock binary and its terminal UI → `warlock-tui` `App`
