<!-- warlock -->
> Written by a model pass over this directory alone, to be read before its source and to say which source to read. A map, not a specification: check anything you are about to rely on against the files themselves, and where this document and the code disagree, the code is right.

# warlock-engine

Engine crate of the workspace: the library that builds a module tree from a repo walk and pact manifest, decides per-directory freshness, and drives documentation generation through an Agent boundary. Owns the JSON document contract.

## Files

- `Cargo.toml` (1.1 KB) — Cargo.toml: engine crate manifest — deps blake3, ignore, serde, serde_json, toml; owns the JSON document contract; dev-deps serde_test, tempfile; lints from workspace

## Directories

- `src/` — All engine library source: tree loading, freshness decisions, hashing, pact/refresh drivers, document fill and rendering, scope/sigil/key config, routing, and CLAUDE.md writing. Start here for any engine question.

## Structure

- Cargo.toml declares the crate's dependencies (blake3, ignore, serde, serde_json, toml) and inherits lints from the workspace.
- Dev-dependencies serde_test and tempfile support tests; the manifest states that this crate owns the JSON document contract.
