<!-- warlock -->
> Written by a model pass over this directory alone, to be read before its source and to say which source to read. A map, not a specification: check anything you are about to rely on against the files themselves, and where this document and the code disagree, the code is right.

# warlock-engine

The warlock-engine crate: builds the module tree from a repo walk, computes per-file freshness against a pacts.toml manifest, and drives the describe-then-grant pact engine reading and writing WARLOCK.md documents through an Agent boundary.

## Files

- `Cargo.toml` (1.1 KB) — Cargo.toml: engine crate manifest — deps blake3, ignore, serde, serde_json, toml; owns the JSON document contract; dev-deps serde_test, tempfile; lints from workspace

## Directories

- `src/` — Crate root; agent boundary, briefs/claude_md/clock/decide/document/fitting/hash/ignores/languages/load/manifest/pact/scope/sigils/state/tree modules — open for module tree, freshness, or pact engine questions.

## Structure

- Cargo.toml: engine crate manifest — deps blake3, ignore, serde, serde_json, toml; owns the JSON document contract; dev-deps serde_test, tempfile; lints from workspace
