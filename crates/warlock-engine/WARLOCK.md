<!-- warlock -->
> Written by a model pass over this directory alone, to be read before its source and to say which source to read. A map, not a specification: check anything you are about to rely on against the files themselves, and where this document and the code disagree, the code is right.

# warlock-engine

The core engine crate for warlock: pacting directories, hashing subtrees, deciding freshness, and laying out the WARLOCK.md documents a model pass fills in, with no TUI or terminal dependency.

## Files

- `Cargo.toml` (1.1 KB) — Crate manifest: no TUI/terminal/HTTP/Anthropic deps allowed, serde_json owns the Fill contract, dev-deps serde_test and tempfile, lints from workspace.

## Directories

- `src/` — All engine source: Agent trait, pacting, hashing, manifest, scope/sigils, document schema, tree types.

## Structure

- Cargo.toml declares dependencies consumed by src's modules (blake3, ignore, serde, serde_json, toml)
- dependency edge runs TUI -> engine and never back, per this manifest's comment
- dev-dependencies (serde_test, tempfile) are used only by tests in src

## Rules

- No TUI, terminal, HTTP or Anthropic dependency belongs here: the dependency edge runs TUI -> engine and never back
- The engine owns the JSON document contract's parser, not just its use
- Lint configuration lives in the root manifest; this crate uses [lints] workspace = true

## Where to look

- what dependencies the engine crate uses → `Cargo.toml` `blake3`
- why no TUI or Anthropic crate is depended on here → `Cargo.toml` `workspace`
- the actual engine source code and types → `src` `Agent`
- how the JSON document (Fill) is parsed and validated → `src` `Fill`
