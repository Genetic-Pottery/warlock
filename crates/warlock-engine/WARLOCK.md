<!-- warlock -->
> Written by a model pass over this directory alone, to be read before its source and to say which source to read. A map, not a specification: check anything you are about to rely on against the files themselves, and where this document and the code disagree, the code is right.

# warlock-engine

Cargo crate for the core engine library: the module tree, pact manifest, hashing and freshness state, documenting directories through an Agent boundary, and scopes, sigils, keys, filing, briefs and CLAUDE.md output. Source is in src/.

## Files

- `Cargo.toml` (1.4 KB) — Cargo manifest for the warlock-engine crate: workspace-inherited package metadata and lints; depends on blake3, ignore, serde (derive), serde_json and toml; dev-depends on serde_test and tempfile.

## Directories

- `src/` — All engine source: tree and manifest models, hashing, pact/refresh drivers, Agent boundary, document rendering, scope/sigil/key/filing resolution. Go here for any engine behaviour question.

## Structure

- Cargo.toml is the crate manifest, inheriting workspace package metadata and lints; it depends on blake3, ignore, serde, serde_json and toml, with serde_test and tempfile as dev-dependencies.
