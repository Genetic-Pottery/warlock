<!-- warlock -->
> Written by a model pass over this directory alone, to be read before its source and to say which source to read. A map, not a specification: check anything you are about to rely on against the files themselves, and where this document and the code disagree, the code is right.

# warlock-engine

The warlock-engine crate root: the manifest holding the crate's package metadata and the strict dependency boundary that the domain logic in src depends on nothing TUI, terminal, HTTP or Anthropic related.

## Files

- `Cargo.toml` (1.1 KB) — Package manifest for warlock-engine: pins blake3, ignore, serde, serde_json, toml as dependencies, serde_test and tempfile as dev-dependencies, and inherits `[lints]` from the workspace.

## Directories

- `src/` — The crate's core domain code — pacting, the freshness decision, the document schema, fitting and elision, the Agent port — open it for how any of those actually work.

## Structure

- Cargo.toml declares the dependencies that src's modules (manifest.rs, hash.rs, document.rs) draw on: serde/serde_json/toml for (de)serialisation and blake3 for hashing.
- dev-dependencies serde_test and tempfile back src's manifest and document round-trip tests without appearing in the crate's own API.

## Rules

- No TUI, terminal, HTTP or Anthropic dependency belongs here: the dependency edge runs TUI -> engine and never back.
- The engine owns the JSON document contract a pass fills in, so it owns the parser for it via serde_json.
- Lint configuration lives in the root manifest so every crate is held to the same bar; this crate's [lints] is workspace = true.

## Where to look

- which dependencies the domain code is allowed to use → `Cargo.toml` `dependencies`
- why no reqwest or anthropic crate appears here → `Cargo.toml`
- where the actual pacting, document, and fitting logic lives → `src` `pact_subtree`
