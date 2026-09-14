<!-- warlock -->
> Written by a model pass over this directory alone, to be read before its source and to say which source to read. A map, not a specification: check anything you are about to rely on against the files themselves, and where this document and the code disagree, the code is right.

# warlock-engine

warlock-engine is the core crate implementing the pact/refresh/document lifecycle for a repository's WARLOCK.md files: the Agent port, manifest and sigil/scope schemas, tree loading, hashing and freshness decisions, and request fitting, kept free of any TUI, terminal, HTTP or Anthropic dependency.

## Files

- `Cargo.toml` (1.1 KB) — Crate manifest declaring blake3, ignore, serde, serde_json and toml dependencies; forbids TUI/terminal/HTTP/Anthropic deps and shares workspace lints.

## Directories

- `src/` — The port, lifecycle engine, manifest/scope/sigil schemas, and tree/hash machinery that decide freshness and drive document generation.

## Structure

- The crate root is src, whose own WARLOCK.md describes lib.rs and every module it declares.

## Rules

- No TUI, terminal, HTTP or Anthropic dependency belongs in this crate: the dependency edge runs TUI -> engine and never back.
- The engine owns the JSON document contract it builds, hands over, reads back and checks, since a pass fills in that fixed shape.
- Lint configuration lives in the root workspace manifest so every crate is held to the same bar.
- serde_test round-trips values through serde's own tokens to prove derives independently of the on-disk format.
- tempfile gives manifest tests a throwaway directory to save into and load back from.

## Where to look

- what dependencies this crate is allowed to pull in → `Cargo.toml` `dependencies`
- the pact/refresh/document lifecycle implementation → `src` `pact_subtree`
- how freshness and hashing decide document staleness → `src` `decide_state`
