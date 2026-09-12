<!-- warlock -->
> Written by a model pass over this directory alone, to be read before its source and to say which source to read. A map, not a specification: check anything you are about to rely on against the files themselves, and where this document and the code disagree, the code is right.

# warlock-engine

warlock-engine is the crate that pacts a directory: running a model pass over its files and children's documents to fill in a WARLOCK.md, judging freshness by content hash, and persisting the result in a pact manifest.

## Files

- `Cargo.toml` (1.1 KB) — Crate manifest: no TUI/terminal/HTTP/Anthropic dependency belongs here, dependency edge runs TUI -> engine only; declares serde_json as the engine's JSON contract, plus serde_test/tempfile dev-deps.

## Directories

- `src/` — The crate's core modules: pacting, hashing, scoping, manifest and document schema; open for how a directory gets pacted or judged fresh/stale.

## Rules

- no TUI, terminal, HTTP or Anthropic dependency belongs here: the dependency edge runs TUI -> engine and never back
- the shape a pass fills in is JSON, a fixed object the engine builds, hands over, reads back and checks (document); the engine owns that contract
- lint configuration lives in the root workspace manifest, not here
- dev-dependencies (serde_test, tempfile) are tests only

## Where to look

- what modules make up the engine and how they fit together → `src` `pact_subtree`
- why the JSON schema parser lives in this crate rather than the CLI or TUI → `Cargo.toml` `serde_json`
