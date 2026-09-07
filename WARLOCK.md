<!-- warlock -->
> Written by a model pass over this directory alone, to be read before its source and to say which source to read. A map, not a specification: check anything you are about to rely on against the files themselves, and where this document and the code disagree, the code is right.

# warlock

The workspace root of warlock, a Rust project split into an engine crate and a terminal-UI crate, defining shared lints, dependency versions and formatting rules for both.

## Files

- `Cargo.lock` (56.8 KB) — Generated lockfile pinning exact versions and checksums of every dependency, including warlock-engine and warlock-tui themselves.
- `Cargo.toml` (6.9 KB) — Workspace manifest declaring members warlock-engine and warlock-tui, shared package metadata, workspace lints (unsafe_code deny, missing_docs, pedantic clippy) and workspace.dependencies with rationale comments.
- `LICENSE` (11.0 KB) — Apache License, Version 2.0 full text.
- `rustfmt.toml` (1.3 KB) — Formatting config pinning edition 2024, style_edition 2024, Unix newlines and use_field_init_shorthand, kept close to rustfmt defaults.

## Directories

- `crates/` — Holds the two member crates warlock-engine and warlock-tui; go here for engine domain logic or TUI/CLI behaviour.

## Structure

- Cargo.toml declares workspace members under crates/ and defines workspace.lints and workspace.dependencies inherited by both crates
- rustfmt.toml and Cargo.toml's edition/rust-version settings apply uniformly across all workspace members

## Rules

- unsafe_code is denied workspace-wide; no unsafe in warlock
- CI runs cargo clippy --workspace --all-targets -- -D warnings, so any warning here fails the build
- any lint set to allow must carry a comment directly above it explaining why
- clippy::nursery and clippy::cargo groups are deliberately not enabled
- Rust from nixpkgs, not rustup: rust-toolchain.toml is inert, rust-version in Cargo.toml is authoritative
- rustfmt.toml deviates from defaults only where a comment states a reason, and nothing nightly-only

## Where to look

- what dependencies does the project use and why → `Cargo.toml` `workspace.dependencies`
- what lints are enforced and which are banned → `Cargo.toml` `unsafe_code`
- engine domain logic like pacting or freshness → `crates` `warlock-engine`
- terminal UI, panel, or CLI subcommand behaviour → `crates` `warlock-tui`
- exact pinned dependency versions → `Cargo.lock` `warlock-engine`
- code formatting conventions → `rustfmt.toml` `style_edition`
- license terms → `LICENSE` `Apache License`
