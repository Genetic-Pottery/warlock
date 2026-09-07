<!-- warlock -->
> Written by a model pass over this directory alone, to be read before its source and to say which source to read. A map, not a specification: check anything you are about to rely on against the files themselves, and where this document and the code disagree, the code is right.

# warlock-engine

Core engine crate for warlock: the domain vocabulary for pacting a repository, holding the manifest, freshness ledger, module document tree, agent seam, and request-fitting logic, with no terminal, subprocess or network dependency.

## Files

- `Cargo.toml` (1.1 KB) — Crate manifest for warlock-engine, pinning workspace deps (blake3, ignore, serde, serde_json, toml) and dev-deps (serde_test, tempfile), enforcing TUI/terminal/HTTP/Anthropic isolation.

## Directories

- `src/` — The engine's Rust source: agent seam, manifest, freshness ledger, document schema, request fitting, and pacting operations.

## Rules

- No TUI, terminal, HTTP or Anthropic dependency belongs here: the dependency edge runs TUI -> engine and never back
- The engine owns the parser for the pass-filled JSON document contract, built via serde_json
- Lint configuration lives in the root manifest's [workspace.lints]; this crate defers to it

## Where to look

- what modules make up the engine and how they fit together → `src` `pact_subtree`
- what dependencies this crate pulls in and why → `Cargo.toml` `serde_json`
