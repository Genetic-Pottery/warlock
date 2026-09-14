<!-- warlock -->
> Written by a model pass over this directory alone, to be read before its source and to say which source to read. A map, not a specification: check anything you are about to rely on against the files themselves, and where this document and the code disagree, the code is right.

# warlock

Repository root of the warlock workspace, holding the two-crate build (warlock-engine, warlock-tui), its dependency lockfile, license, and shared rustfmt config.

## Files

- `Cargo.lock` (56.8 KB) — Auto-generated lockfile pinning exact versions and checksums for every crate dependency of warlock-engine and warlock-tui; not hand-edited.
- `Cargo.toml` (7.4 KB) — Workspace manifest for warlock-engine and warlock-tui: shared lints, package metadata, and pinned dependencies (serde, ratatui, ignore, blake3, notify, clap, ctrlc).
- `LICENSE` (11.0 KB) — Full text of the Apache License, Version 2.0, governing use and redistribution of the repository's contents.
- `rustfmt.toml` (1.3 KB) — rustfmt.toml — workspace formatting config: pins edition 2024, style_edition 2024, LF newlines, and struct field-init shorthand.

## Directories

- `crates/` — Holds the warlock-engine and warlock-tui crates; go there for engine decisions, tree walking, or the TUI panel and binary.

## Structure

- Auto-generated lockfile pinning exact versions and checksums for every crate dependency of warlock-engine and warlock-tui; not hand-edited.
- Workspace manifest for warlock-engine and warlock-tui: shared lints, package metadata, and pinned dependencies (serde, ratatui, ignore, blake3, notify, clap, ctrlc).
- rustfmt.toml — workspace formatting config: pins edition 2024, style_edition 2024, LF newlines, and struct field-init shorthand.

## Where to look

- what license governs this repository → `LICENSE`
- what versions of dependencies are actually locked in → `Cargo.lock`
- what crates make up the workspace and what they depend on → `Cargo.toml`
- how is code formatted across the workspace → `rustfmt.toml`
- where does the engine crate for freshness decisions live → `Cargo.toml` `warlock-engine`
- where does the terminal panel crate live → `Cargo.toml` `warlock-tui`
