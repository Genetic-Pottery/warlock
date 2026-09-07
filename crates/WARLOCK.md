<!-- warlock -->
> Written by a model pass over this directory alone, to be read before its source and to say which source to read. A map, not a specification: check anything you are about to rely on against the files themselves, and where this document and the code disagree, the code is right.

# crates

The crates workspace directory for warlock, holding the two member crates: warlock-engine, the domain logic core, and warlock-tui, the terminal front end and binary.

## Directories

- `warlock-engine/` — Core domain crate — pacting, freshness ledger, scope/sigil boundary logic, fitting; go here for engine questions, never TUI/HTTP.
- `warlock-tui/` — Terminal front end crate producing the warlock binary — panel UI, headless subcommands; go here for CLI or display behaviour.

## Structure

- warlock-tui depends on warlock-engine; the dependency edge runs TUI -> engine and never the reverse

## Where to look

- where the domain/document logic lives → `warlock-engine` `Agent`
- where the terminal UI and binary logic lives → `warlock-tui` `App`
