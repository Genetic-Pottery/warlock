<!-- warlock -->
> Written by a model pass over this directory alone, to be read before its source and to say which source to read. A map, not a specification: check anything you are about to rely on against the files themselves, and where this document and the code disagree, the code is right.

# warlock-engine

The warlock-engine crate: the core freshness-ledger engine for warlock, owning pacts, scopes, sigils, hashing, fitting a directory into a model's context, and rendering the resulting document, with the TUI depending on it and never the reverse.

## Files

- `Cargo.toml` (1.1 KB) — Crate manifest: pins the TUI -> engine dependency edge, serde/serde_json/toml/blake3/ignore deps, serde_test and tempfile dev-deps, and workspace lints.

## Directories

- `src/` — The engine's modules: agent, briefs, claude_md, clock, decide, document, fitting, hash, ignores, languages, lib, load, manifest, pact, scope, sigils, state, tree.

## Structure

- Cargo.toml declares dependencies consumed throughout src (serde_json for the Fill schema, blake3 for hashing, ignore for file walking, toml for manifests)
- dev-dependencies serde_test and tempfile are used only by src's test code, not by runtime modules

## Rules

- No TUI, terminal, HTTP or Anthropic dependency belongs in this crate: the dependency edge runs TUI -> engine and never back
- serde_json is the engine's own parser for the fixed JSON shape a pass fills in (document), not a general utility shared incidentally with the TUI
- Lint configuration is inherited from the workspace root manifest via [lints] workspace = true, not set locally

## Where to look

- what this crate depends on and why no HTTP or TUI crate appears → `Cargo.toml` `workspace`
- where the actual engine logic and types live → `src` `pact_directory`
- how tests round-trip manifests or serde structures → `Cargo.toml` `serde_test`
