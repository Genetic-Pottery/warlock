<!-- warlock -->
> Written by a model pass over this directory alone, to be read before its source and to say which source to read. A map, not a specification: check anything you are about to rely on against the files themselves, and where this document and the code disagree, the code is right.

# crates

The crates directory holds the two crates that make up warlock: warlock-engine, the terminal-free core domain logic, and warlock-tui, the terminal front end and `warlock` binary built on it.

## Directories

- `warlock-engine/` — Core engine crate: pacting, hashing, freshness decisions, scope/sigils, the Fill/WARLOCK.md document schema; no TUI, terminal, HTTP or Anthropic dependency.
- `warlock-tui/` — Terminal front end crate, packaged as the `warlock` binary plus the warlock_tui library: App state, panels, boundary/write logic, rendering, headless subcommands.

## Structure

- warlock-tui depends on warlock-engine for domain vocabulary
- the dependency edge runs TUI -> engine and never back

## Where to look

- core domain logic, pacting, hashing, freshness, document schema → `warlock-engine` `Agent`
- the warlock terminal binary or TUI panels and rendering → `warlock-tui` `warlock_tui`
