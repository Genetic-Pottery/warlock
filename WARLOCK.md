<!-- warlock -->
> Written by a model pass over this directory alone, to be read before its source and to say which source to read. A map, not a specification: check anything you are about to rely on against the files themselves, and where this document and the code disagree, the code is right.

# warlock

Root of the warlock workspace: the Cargo workspace manifest and shared lint/dependency policy for the two crates that make up warlock, plus licensing and formatting config for the repo.

## Files

- `Cargo.lock` (56.8 KB) — Auto-generated, pinned dependency graph for the workspace, including warlock-engine and warlock-tui and every third-party crate they resolve to (ratatui, clap, notify, blake3, toml, serde, etc.).
- `Cargo.toml` (6.9 KB) — Workspace manifest: members crates/warlock-engine and warlock-tui, workspace.package metadata, shared workspace.lints, and workspace.dependencies with justifications.
- `LICENSE` (11.0 KB) — Full text of the Apache License, Version 2.0 under which the project is licensed.
- `rustfmt.toml` (1.3 KB) — rustfmt configuration: pins edition/style_edition to 2024, newline_style Unix, use_field_init_shorthand true, kept close to rustfmt defaults.

## Directories

- `crates/` — The two workspace crates, warlock-engine (terminal-free core domain logic) and warlock-tui (terminal front end and warlock binary); go there for pacting, hashing, freshness, scopes/sigils, document schema, or TUI panels/rendering code.

## Structure

- Cargo.toml's [workspace] members list points to crates/warlock-engine and crates/warlock-tui
- workspace.lints and workspace.dependencies defined here are inherited by both crates via `workspace = true`
- Cargo.lock is generated from Cargo.toml and records the resolved versions, including the warlock-engine and warlock-tui workspace packages themselves

## Rules

- unsafe_code is denied workspace-wide; any future use requires an explicit #[allow] visible in the diff
- CI runs cargo clippy --workspace --all-targets -D warnings, so every workspace lint here is a build-breaking check
- clippy::pedantic is warned on as a group; clippy::nursery and clippy::cargo are deliberately left off
- any dependency added to workspace.dependencies must carry a one-line comment above it explaining why
- any lint set to allow must carry a comment directly above it saying why
- rustfmt targets Rust/style edition 2024, always emits Unix (LF) line endings, and assumes nixpkgs stable rustc with no rustup shims

## Where to look

- which crates exist and how they depend on each other → `crates` `warlock-engine`
- why a particular third-party dependency was chosen → `Cargo.toml` `workspace.dependencies`
- what lints or clippy groups are enforced in CI → `Cargo.toml` `workspace.lints`
- the exact resolved version of a dependency → `Cargo.lock` `warlock-tui`
- code formatting rules or edition settings → `rustfmt.toml` `style_edition`
- licensing terms → `LICENSE` `Apache License`
