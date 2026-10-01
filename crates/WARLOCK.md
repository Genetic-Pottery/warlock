<!-- warlock -->
> Written by a model pass over this directory alone, to be read before its source and to say which source to read. A map, not a specification: check anything you are about to rely on against the files themselves, and where this document and the code disagree, the code is right.

# crates

The crates directory holds the two Rust crates that make up the project: warlock-engine, the core library, and warlock-tui, the terminal UI and CLI.

## Directories

- `warlock-engine/` — The core library crate: walks a repo tree, tracks freshness against pacts.toml, drives LLM fills of WARLOCK.md documents, and manages scopes, sigils, pulls and filed tickets.
- `warlock-tui/` — The terminal UI and CLI crate: a state machine, event loop, and rendering plus subcommands (pact, push, pull, cut, check, key, list, resume) driving manifest, boundary, brief, and Linear-filing logic.

## Structure

- warlock-engine holds the warlock-engine crate.
- warlock-tui holds the warlock-tui crate.
