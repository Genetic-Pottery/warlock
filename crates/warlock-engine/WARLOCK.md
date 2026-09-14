<!-- warlock -->
> Written by a model pass over this directory alone, to be read before its source and to say which source to read. A map, not a specification: check anything you are about to rely on against the files themselves, and where this document and the code disagree, the code is right.

# warlock-engine

The engine crate: walks a repo into a freshness-tracked tree, runs model passes to fill and mend WARLOCK.md documents, and persists pact state, scopes and sigils, kept free of TUI/terminal/HTTP/Anthropic deps.

## Files

- `Cargo.toml` (1.1 KB) — Manifest for warlock-engine: blake3, ignore, serde/serde_json and toml deps, kept free of TUI/terminal/HTTP/Anthropic crates by design.

## Directories

- `src/` — The crate's source: tree walking, pact/manifest state, document fill/mend/render, scope and sigil logic — go here for any implementation question.

## Structure

- Manifest for warlock-engine: blake3, ignore, serde/serde_json and toml deps, kept free of TUI/terminal/HTTP/Anthropic crates by design.
