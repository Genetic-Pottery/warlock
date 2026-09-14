<!-- warlock -->
> Written by a model pass over this directory alone, to be read before its source and to say which source to read. A map, not a specification: check anything you are about to rely on against the files themselves, and where this document and the code disagree, the code is right.

# crates

Holds the two crates that make up the workspace: warlock-engine for repo walking, pact-state decisions and document filling/rendering, and warlock-tui for the terminal panel and headless subcommands built on it.

## Directories

- `warlock-engine/` — The engine crate: tree walking, freshness decisions, WARLOCK.md filling and rendering, and manifest/scope/sigil persistence, kept free of TUI/HTTP deps.
- `warlock-tui/` — The warlock binary and warlock_tui library: the panel, keypress handling, filesystem watching, and headless subcommands sharing its session logic.

## Structure

- Manifest for warlock-engine: blake3, ignore, serde/serde_json and toml deps, kept free of TUI/terminal/HTTP/Anthropic crates by design.
- Manifest for the warlock-tui crate: builds the warlock binary from src/main.rs and the warlock_tui library from src/lib.rs, wiring clap, ratatui, notify, ctrlc and warlock-engine.

## Where to look

- how is a directory's freshness state decided → `warlock-engine` `decide_state`
- how does the repo tree get walked and built → `warlock-engine` `load_tree`
- what shape does a model implementation have to satisfy → `warlock-engine` `Agent`
- where are per-directory pact grants stored → `warlock-engine` `Manifest`
- how are scopes and sigils validated against each other → `warlock-engine` `scope_opens_to`
- what crate builds the warlock binary → `warlock-tui` `warlock`
- where is the terminal-free library surface defined → `warlock-tui` `warlock_tui`
- what dependencies does the engine crate declare → `warlock-engine`
- what dependencies does the tui pull in → `warlock-tui`
