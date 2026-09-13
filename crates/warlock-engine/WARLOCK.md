<!-- warlock -->
> Written by a model pass over this directory alone, to be read before its source and to say which source to read. A map, not a specification: check anything you are about to rely on against the files themselves, and where this document and the code disagree, the code is right.

# warlock-engine

The warlock-engine crate: the domain logic for pacting a codebase's directories with WARLOCK.md documents, tracking freshness through hashes, and routing a model pass through fitting, validation and rendering, with no TUI, terminal, HTTP or network dependency of its own.

## Files

- `Cargo.toml` (1.1 KB) — The crate manifest; pins no TUI/terminal/HTTP/Anthropic dependency since the dependency edge runs TUI -> engine, declares serde/serde_json/toml as dependencies and serde_test/tempfile as dev-dependencies, and inherits `[lints]` from the workspace.

## Directories

- `src/` — The crate's core module: pact/refresh orchestration, freshness state, document schema and validation, request fitting, hashing, manifests, scopes and sigils; open it for how any of that logic works.

## Structure

- The crate's public API is entirely what `src` declares and re-exports.

## Rules

- No TUI, terminal, HTTP or Anthropic dependency belongs here: the dependency edge runs TUI -> engine and never back.
- The engine owns the JSON contract a pass fills in (`document`), including parsing it, since it builds, hands over and reads back that shape.
- Lint configuration lives in the root manifest so every crate is held to the same bar; this crate just inherits `[workspace.lints]`.
- `serde_test` and `tempfile` are dev-dependencies only, used to round-trip serde derives and to give manifest tests a throwaway directory.

## Where to look

- what dependencies this crate declares and why → `Cargo.toml` `serde_json`
- the actual pact/refresh/validate/render logic → `src` `pact_subtree`
- why no rustup/HTTP dependency shows up here → `Cargo.toml`
