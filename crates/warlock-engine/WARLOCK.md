<!-- warlock -->
> Written by a model pass over this directory alone, to be read before its source and to say which source to read. A map, not a specification: check anything you are about to rely on against the files themselves, and where this document and the code disagree, the code is right.

# warlock-engine

Core engine crate for warlock: the domain logic for pacting directories into WARLOCK.md documents, tracking a three-colour freshness ledger over a pact manifest, scoping who may change what, and fitting a directory's files into a model request.

## Files

- `Cargo.toml` (1.1 KB) — Crate manifest: no TUI/terminal/HTTP/Anthropic deps allowed here (dependency edge runs TUI -> engine); blake3, ignore, serde, serde_json, toml deps; serde_test/tempfile dev-deps; lints from workspace.

## Directories

- `src/` — The engine's source: agent port, document schema/validation, fitting ladder, hash/tree/manifest, scope/sigil boundary logic, and pact operations.

## Rules

- No TUI, terminal, HTTP or Anthropic dependency belongs in this crate: the dependency edge runs TUI -> engine and never back
- The engine owns the parser for the pass-fill JSON contract (document), since it owns that shape
- Lint configuration is inherited from the root manifest's [workspace.lints] so every crate is held to the same bar

## Where to look

- what dependencies the engine crate uses → `Cargo.toml` `serde_json`
- the actual engine source code and logic → `src` `Agent`
