<!-- warlock -->
> Written by a model pass over this directory alone, to be read before its source and to say which source to read. A map, not a specification: check anything you are about to rely on against the files themselves, and where this document and the code disagree, the code is right.

# warlock-engine

The warlock-engine crate: it walks a repo, hashes and scopes directories, orchestrates LLM agents to fill and validate WARLOCK.md documents, and tracks pact/pull/draft state across runs.

## Files

- `Cargo.toml` (1.4 KB) — Cargo manifest for the warlock-engine crate: workspace-inherited package metadata and lints; depends on blake3, ignore, serde (derive), serde_json and toml; dev-depends on serde_test and tempfile.

## Directories

- `src/` — All crate source lives here: agent boundary, document fill/validation, pact/pull orchestration, scope/route/sigil resolution, and shared walk/hash/state helpers.

## Structure

- Cargo manifest for the warlock-engine crate: workspace-inherited package metadata and lints; depends on blake3, ignore, serde (derive), serde_json and toml; dev-depends on serde_test and tempfile.
