<!-- warlock -->
> Written by a model pass over this directory alone, to be read before its source and to say which source to read. A map, not a specification: check anything you are about to rely on against the files themselves, and where this document and the code disagree, the code is right.

# warlock-engine

warlock-engine is the core crate: the Agent model-call boundary, tree loading and hashing, the two-phase pact engine that describes then grants freshness, and WARLOCK.md/pacts.toml generation and repair.

## Files

- `Cargo.toml` (1.1 KB) — Cargo.toml: engine crate manifest — deps blake3, ignore, serde, serde_json, toml; owns the JSON document contract; dev-deps serde_test, tempfile; lints from workspace

## Directories

- `src/` — The crate's modules: agent, briefs, claude_md, clock, decide, document, fitting, hash, ignores, languages, lib, load, manifest, pact, scope, sigils, state, tree, walk.

## Structure

- Cargo.toml: engine crate manifest — deps blake3, ignore, serde, serde_json, toml; owns the JSON document contract; dev-deps serde_test, tempfile; lints from workspace
