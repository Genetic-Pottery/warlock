<!-- warlock -->
> Written by a model pass over this directory alone, to be read before its source and to say which source to read. A map, not a specification: check anything you are about to rely on against the files themselves, and where this document and the code disagree, the code is right.

# warlock-engine

The warlock-engine crate: walks repositories into a Tree of Nodes, hashes and decides freshness state per directory, calls the Agent trait to fill and render WARLOCK.md documents, and tracks pacts, scopes, sigils, pulls, and ticket drafting/splitting/filing.

## Files

- `Cargo.toml` (1.4 KB) — Cargo manifest for the warlock-engine crate: workspace-inherited package metadata and lints; depends on blake3, ignore, serde (derive), serde_json and toml; dev-depends on serde_test and tempfile.

## Directories

- `src/` — Crate root source: Tree/Node walking, hashing, Agent trait, pact/scope/sigil/pull orchestration, and WARLOCK.md document fill and render logic.

## Structure

- Cargo manifest for the warlock-engine crate: workspace-inherited package metadata and lints; depends on blake3, ignore, serde (derive), serde_json and toml; dev-depends on serde_test and tempfile.
