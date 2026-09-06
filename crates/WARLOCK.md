<!-- warlock -->
> Written by a model pass over this directory alone, to be read before its source and to say which source to read. A map, not a specification: check anything you are about to rely on against the files themselves, and where this document and the code disagree, the code is right.

# crates

Workspace directory holding warlock's two crates: warlock-engine, the domain logic for pacting directories with WARLOCK.md documents and enforcing scope/sigil boundaries, and warlock-tui, the terminal front end that depends on it.

## Directories

- `warlock-engine/` — Core domain logic — document/pact/manifest/scope/hash modules — with no TUI, terminal, HTTP or subprocess dependency; open for staleness decisions or boundary rules.
- `warlock-tui/` — The terminal front end and warlock binary — tree/panel UI, model seams, headless subcommands; open for any question about the app's behaviour or CLI.

## Structure

- warlock-tui depends on warlock-engine; the dependency edge runs TUI -> engine and never back

## Where to look

- what dependencies the domain crate is allowed to have → `warlock-engine` `dependencies`
- where the actual TUI and subcommand logic lives → `warlock-tui` `App`
