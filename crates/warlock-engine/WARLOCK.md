<!-- warlock -->
> Written by a model pass over this directory alone, to be read before its source and to say which source to read. A map, not a specification: check anything you are about to rely on against the files themselves, and where this document and the code disagree, the code is right.

# warlock-engine

The engine crate: builds and maintains a repo's freshness ledger, hashing files, tracking pacted/unpacted state, walking directories and filling WARLOCK.md documents via an agent port, kept free of TUI, terminal, HTTP or Anthropic dependencies.

## Files

- `Cargo.toml` (1.1 KB) — Manifest for warlock-engine: blake3, ignore, serde/serde_json and toml deps, kept free of TUI/terminal/HTTP/Anthropic crates by design.

## Directories

- `src/` — The crate root: modules for hashing, tree walking, manifest state, pact engine and document filling; start at lib.rs to see how they connect.

## Structure

- Manifest for warlock-engine: blake3, ignore, serde/serde_json and toml deps, kept free of TUI/terminal/HTTP/Anthropic crates by design.
