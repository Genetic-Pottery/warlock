<!-- warlock -->
> Written by a model pass over this directory alone, to be read before its source and to say which source to read. A map, not a specification: check anything you are about to rely on against the files themselves, and where this document and the code disagree, the code is right.

# warlock

Root of the warlock workspace: the Rust project that pacts directories with WARLOCK.md documents and enforces scope/sigil boundaries, split into an engine crate and a TUI front end.

## Files

- `Cargo.lock` (56.8 KB) — Generated lockfile pinning exact versions of every dependency for warlock-engine and warlock-tui, including ratatui, clap, notify, blake3, ignore, toml, serde_json.
- `Cargo.toml` (6.9 KB) — Workspace manifest: two member crates, shared package metadata (edition 2024, rust-version 1.97.1, Apache-2.0), workspace lints, and workspace.dependencies with per-crate justifying comments.
- `LICENSE` (11.0 KB) — Full text of the Apache License, Version 2.0 under which warlock is licensed.
- `rustfmt.toml` (1.3 KB) — Formatting config: edition/style_edition 2024, newline_style Unix, use_field_init_shorthand true, kept close to rustfmt defaults.

## Directories

- `crates/` — Holds warlock-engine (domain logic, no UI/network deps) and warlock-tui (terminal front end and binary); open for how the two crates split and depend on each other.

## Structure

- workspace.dependencies in Cargo.toml are inherited by both member crates listed in [workspace] members
- workspace.lints in Cargo.toml apply wherever a crate opts in with [lints] workspace = true
- rustfmt.toml edition/style_edition mirrors workspace.package.edition in Cargo.toml

## Rules

- unsafe_code is denied workspace-wide; any future use requires an explicit #[allow] visible in the diff
- any workspace.dependencies or lint entry set to allow must carry a comment above it explaining why
- CI runs cargo clippy --workspace --all-targets -- -D warnings, so every warn-level lint here fails the build
- rustfmt always emits LF (newline_style = Unix), never CRLF
- clippy::nursery and clippy::cargo are deliberately left off, per the comment explaining the cost/benefit

## Where to look

- why a given crate dependency was chosen → `Cargo.toml` `workspace.dependencies`
- what lints are enforced and why → `Cargo.toml` `workspace.lints`
- which crates make up the project and how they depend on each other → `crates` `warlock-engine`
- code formatting rules → `rustfmt.toml` `style_edition`
- license terms → `LICENSE` `Apache License`
