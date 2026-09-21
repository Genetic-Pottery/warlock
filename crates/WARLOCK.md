<!-- warlock -->
> Written by a model pass over this directory alone, to be read before its source and to say which source to read. A map, not a specification: check anything you are about to rely on against the files themselves, and where this document and the code disagree, the code is right.

# crates

Cargo workspace members for warlock: the core engine library (warlock-engine) and the command-line tool with its terminal panel (warlock-tui). Go here to choose between engine logic and the CLI/panel front end.

## Directories

- `warlock-engine/` — Core engine crate: tree and manifest models, hashing and freshness state, pact/refresh drivers, Agent boundary, document rendering, scopes, sigils, keys, filing. Go here for engine behaviour questions.
- `warlock-tui/` — CLI and ratatui panel crate that builds the `warlock` binary and `warlock_tui` library: subcommands, panel state and drawing, chat, push to Linear, scope prompts. Go here for front-end questions.

## Structure

- warlock-tui depends on warlock-engine for the core logic, so the dependency runs from the front end to the engine and not the other way.
