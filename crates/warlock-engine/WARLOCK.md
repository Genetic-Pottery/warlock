<!-- warlock -->
> Written by a model pass over this directory alone, to be read before its source and to say which source to read. A map, not a specification: check anything you are about to rely on against the files themselves, and where this document and the code disagree, the code is right.

# warlock-engine

The warlock-engine crate: models a repo as a Tree of pacted/unpacted modules, hashes and views files, drives agents to fill and validate WARLOCK.md content, and enforces scope/sigil boundaries for pulls, drafting and splitting.

## Files

- `Cargo.toml` (1.4 KB) — Cargo manifest for the warlock-engine crate: workspace-inherited package metadata and lints; depends on blake3, ignore, serde (derive), serde_json and toml; dev-depends on serde_test and tempfile.

## Directories

- `src/` — Crate core: Tree model, hashing, agent-driven document fill/validation, and scope/sigil enforcement — go here for how any engine behavior works.

## Structure

- Cargo.toml is the workspace-inherited manifest for the warlock-engine crate, depending on blake3, ignore, serde (derive), serde_json and toml, with dev-dependencies serde_test and tempfile.
- src/ holds the crate's core: it models the repo as a Tree of pacted/unpacted modules, hashes and views files, drives agents to fill and validate WARLOCK.md content, and enforces scope/sigil boundaries for pulls, drafting and splitting.
