<!-- warlock -->
> Written by a model pass over this directory alone, to be read before its source and to say which source to read. A map, not a specification: check anything you are about to rely on against the files themselves, and where this document and the code disagree, the code is right.

# warlock-engine

The warlock-engine crate: the core library that walks a repo tree, tracks per-directory freshness against a pacts.toml manifest, drives LLM agent calls to fill and validate WARLOCK.md documents, and manages scopes, sigils, pulls and filed tickets.

## Files

- `Cargo.toml` (1.4 KB) — Cargo manifest for the warlock-engine crate: workspace-inherited package metadata and lints; depends on blake3, ignore, serde (derive), serde_json and toml; dev-depends on serde_test and tempfile.

## Directories

- `src/` — The engine core's source: walking, hashing, pacting, document fills, scopes/sigils, pulls and filed tickets; open it for how any of those work.

## Structure

- Cargo.toml defines the warlock-engine crate with workspace-inherited package metadata and lints.
- warlock-engine depends on blake3, ignore, serde (derive), serde_json and toml.
- warlock-engine dev-depends on serde_test and tempfile.
- src/ holds the crate's implementation, built out from Cargo.toml's declared dependencies.
