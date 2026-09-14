<!-- warlock -->
> Written by a model pass over this directory alone, to be read before its source and to say which source to read. A map, not a specification: check anything you are about to rely on against the files themselves, and where this document and the code disagree, the code is right.

# warlock-engine

The engine crate: walks a repo into a freshness ledger, decides pact state per directory, runs model passes to fill and render WARLOCK.md documents, and persists manifests, scopes and sigils to disk.

## Files

- `Cargo.toml` (1.1 KB) — Manifest for warlock-engine: blake3, ignore, serde/serde_json and toml deps, kept free of TUI/terminal/HTTP/Anthropic crates by design.

## Directories

- `src/` — The crate's source: tree walking, freshness decisions, document filling and rendering, and manifest/scope/sigil persistence.

## Structure

- Manifest for warlock-engine: blake3, ignore, serde/serde_json and toml deps, kept free of TUI/terminal/HTTP/Anthropic crates by design.

## Where to look

- how is a directory's freshness state determined → `src` `decide_state`
- how does the repo tree get walked and built → `src` `load_tree`
- what does a WARLOCK.md pact request look like before it hits the model → `src`
- what shape does a model implementation have to satisfy → `src` `Agent`
- how is CLAUDE.md's warlock section written or updated → `src` `write_claude_md`
- where are per-directory pact grants stored → `src` `Manifest`
- how is a subtree's content hashed for staleness checks → `src` `subtree_hash`
- how does .warlockignore affect a directory's own visibility → `src` `is_ignored`
- how are scopes and sigils validated against each other → `src` `scope_opens_to`
- where do sigils get loaded or saved per checkout → `src` `load_sigils`
- how does pacting a directory run per-file then synthesis passes → `src` `pact_subtree`
- what dependencies does the engine crate declare → `Cargo.toml`
