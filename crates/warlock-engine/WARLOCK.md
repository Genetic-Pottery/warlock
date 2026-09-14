<!-- warlock -->
> Written by a model pass over this directory alone, to be read before its source and to say which source to read. A map, not a specification: check anything you are about to rely on against the files themselves, and where this document and the code disagree, the code is right.

# warlock-engine

The engine crate: walks a repository into a freshness ledger, tracks pacted/unpacted/stale state against a manifest, fills and renders WARLOCK.md documents through a model-pass Agent port, and manages scopes, sigils and ignores that bound where pacts apply, kept free of TUI, terminal, HTTP or Anthropic dependencies.

## Files

- `Cargo.toml` (1.1 KB) — Manifest for warlock-engine: blake3, ignore, serde/serde_json and toml deps, kept free of TUI/terminal/HTTP/Anthropic crates by design.

## Directories

- `src/` — the crate's source: Tree/Node ledger, decide.rs state rule, manifest and pact machinery, document fill/render, scope/sigil/ignore boundaries

## Structure

- Manifest for warlock-engine: blake3, ignore, serde/serde_json and toml deps, kept free of TUI/terminal/HTTP/Anthropic crates by design.

## Where to look

- what decides whether a file is stale or fresh → `src` `decide_state`
- where is WARLOCK.md actually written and merged into CLAUDE.md → `src` `write_claude_md`
- how are two clones of the same repo verified to agree → `src` `subtree_hash`
- where does the model-pass request/response shape live → `src` `Request`
- how does the ledger tree get built from disk → `src` `load_tree`
- where are pacts.toml entries read and saved → `src` `Manifest`
- what performs a pact or refresh over a subtree → `src` `pact_subtree`
- where do sigils get resolved to a config directory → `src` `sigils_path`
- how is a directory excluded via .warlockignore → `src` `is_ignored`
- where are the freshness states enumerated → `src` `NodeState`
- how is a fill turned into rendered WARLOCK.md content → `src` `render`
- where do per-language test/declaration rules live → `src` `TABLE`
