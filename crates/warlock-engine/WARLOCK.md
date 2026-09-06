<!-- warlock -->
> Written by a model pass over this directory alone, to be read before its source and to say which source to read. A map, not a specification: check anything you are about to rely on against the files themselves, and where this document and the code disagree, the code is right.

# warlock-engine

Crate root for warlock-engine: the core domain logic for pacting a codebase's directories with model-written WARLOCK.md documents, deciding staleness by content hash, and enforcing scope/sigil boundaries, with no dependency on any TUI, terminal, HTTP or subprocess.

## Files

- `Cargo.toml` (1.1 KB) — Manifest: declares dependencies (blake3, ignore, serde, serde_json, toml), dev-dependencies (serde_test, tempfile), and pins the dependency edge to run TUI -> engine and never back.

## Directories

- `src/` — All engine source: agent/document/pact/manifest/scope/hash modules; open for the domain vocabulary, staleness decisions, and boundary enforcement.

## Rules

- No TUI, terminal, HTTP or Anthropic dependency belongs in this crate: the dependency edge runs TUI -> engine and never back
- The JSON shape a pass fills in (`document`) is owned by this crate, including its parser, even though the TUI and CLI use the same serde_json dependency for other streams
- serde_test and tempfile are dev-dependencies only, used to round-trip serde derives and to give manifest tests a throwaway directory
- Lint configuration is not set here; it is inherited from `[workspace.lints]` in the root manifest

## Where to look

- what dependencies this crate is allowed and forbidden to have → `Cargo.toml` `dependencies`
- the core domain logic and module layout → `src` `pact_subtree`
- why serde_json is a dependency here → `Cargo.toml` `serde_json`
