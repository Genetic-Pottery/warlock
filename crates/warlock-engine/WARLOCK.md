<!-- warlock -->
> Written by a model pass over this directory alone, to be read before its source and to say which source to read. A map, not a specification: check anything you are about to rely on against the files themselves, and where this document and the code disagree, the code is right.

# warlock-engine

The warlock-engine crate: a Cargo library providing the core logic for walking/hashing repo trees, tracking pacted freshness, driving LLM document fills, and managing scopes, sigils, keys, pulls, drafting and ticket-splitting/filing.

## Files

- `Cargo.toml` (1.4 KB) — Cargo manifest for the warlock-engine crate: workspace-inherited package metadata and lints; depends on blake3, ignore, serde (derive), serde_json and toml; dev-depends on serde_test and tempfile.

## Directories

- `src/` — The crate's core library source, from the Agent trait through pact orchestration to pulls, drafting and splitting.

## Structure

- Cargo manifest for the warlock-engine crate: workspace-inherited package metadata and lints; depends on blake3, ignore, serde (derive), serde_json and toml; dev-depends on serde_test and tempfile.
