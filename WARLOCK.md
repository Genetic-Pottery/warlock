<!-- warlock -->
> Written by a model pass over this directory alone, to be read before its source and to say which source to read. A map, not a specification: check anything you are about to rely on against the files themselves, and where this document and the code disagree, the code is right.

# warlock

Root of the warlock workspace: a two-crate Rust project (warlock-engine, warlock-tui) that builds the warlock repository-documentation tool, with its manifest, lockfile, license and formatting settings.

## Files

- `Cargo.lock` (56.8 KB) — Generated lockfile pinning exact versions and checksums for every dependency of warlock-engine and warlock-tui.
- `Cargo.toml` (6.9 KB) — Workspace manifest: members list, shared package metadata, workspace-wide clippy/rustc lints (unsafe_code deny, pedantic warn), and the annotated workspace.dependencies (serde, serde_json, toml, ratatui, ignore, blake3, notify, clap, ctrlc, serde_test, tempfile).
- `LICENSE` (11.0 KB) — Full text of the Apache License, Version 2.0 covering the repository.
- `rustfmt.toml` (1.3 KB) — rustfmt config pinning edition 2024, style_edition 2024, Unix newlines, and use_field_init_shorthand, each with a stated reason.

## Directories

- `crates/` — The two-crate workspace itself: warlock-engine (domain core) and warlock-tui (terminal front end and binary); go here for any question about engine or TUI behaviour.

## Structure

- Cargo.toml lists crates/warlock-engine and crates/warlock-tui as workspace members
- warlock-tui depends on warlock-engine per Cargo.lock; dependency edge runs TUI -> engine only
- workspace.lints and workspace.dependencies in Cargo.toml are inherited by both member crates via `workspace = true`
- rustfmt.toml and Cargo.toml's edition/rust-version together fix how source in crates/ is parsed and formatted

## Rules

- unsafe_code is denied workspace-wide: no unsafe in warlock
- CI runs cargo clippy --workspace --all-targets -- -D warnings, so any warning fails the build
- any lint set to allow must carry a comment above it saying why
- any workspace.dependencies entry must carry a one-line comment justifying the dependency
- rustfmt always emits LF (newline_style = Unix), pinned edition and style_edition 2024
- rust-version is 1.97.1, and rustup shims are absent so rust-toolchain.toml would be inert here

## Where to look

- why a given dependency is in the tree → `Cargo.toml` `workspace.dependencies`
- what lints CI enforces and why → `Cargo.toml` `workspace.lints`
- exact pinned version of a transitive dependency → `Cargo.lock` `warlock-engine`
- engine or TUI source code and behavior → `crates` `warlock-engine`
- formatting style rules for Rust source → `rustfmt.toml` `style_edition`
- licensing terms → `LICENSE` `Apache License`
