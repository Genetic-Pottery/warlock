<!-- warlock -->
> Written by a model pass over this directory alone, to be read before its source and to say which source to read. A map, not a specification: check anything you are about to rely on against the files themselves, and where this document and the code disagree, the code is right.

# crates

The workspace's two crates: warlock-engine, which walks a repository into a freshness ledger and fills/renders WARLOCK.md documents, and warlock-tui, the terminal front end and headless subcommands built on it.

## Directories

- `warlock-engine/` — The engine crate: ledger, decide.rs state rule, manifest/pact machinery, document fill/render, scope/sigil/ignore boundaries; go there for how state, pacts or documents are computed.
- `warlock-tui/` — The warlock binary and warlock_tui library: panel, keypress handling, filesystem watching, headless subcommands; go there for how a key or run becomes screen state or output.

## Structure

- Manifest for warlock-engine: blake3, ignore, serde/serde_json and toml deps, kept free of TUI/terminal/HTTP/Anthropic crates by design.
- Manifest for the warlock-tui crate: builds the `warlock` binary from src/main.rs and the warlock_tui library from src/lib.rs, wiring clap, ratatui, notify, ctrlc and warlock-engine.

## Where to look

- what decides whether a file is stale or fresh → `warlock-engine` `decide_state`
- what crate builds the warlock binary → `warlock-tui` `warlock`
- where is WARLOCK.md actually written and merged into CLAUDE.md → `warlock-engine` `write_claude_md`
- what dependencies does the tui pull in → `warlock-tui`
- how are two clones of the same repo verified to agree → `warlock-engine` `subtree_hash`
- where is the terminal-free library surface defined → `warlock-tui` `warlock_tui`
- where does the model-pass request/response shape live → `warlock-engine` `Request`
- how does the ledger tree get built from disk → `warlock-engine` `load_tree`
- where are pacts.toml entries read and saved → `warlock-engine` `Manifest`
- what performs a pact or refresh over a subtree → `warlock-engine` `pact_subtree`
- where do sigils get resolved to a config directory → `warlock-engine` `sigils_path`
- how is a directory excluded via .warlockignore → `warlock-engine` `is_ignored`
