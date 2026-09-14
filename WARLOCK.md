<!-- warlock -->
> Written by a model pass over this directory alone, to be read before its source and to say which source to read. A map, not a specification: check anything you are about to rely on against the files themselves, and where this document and the code disagree, the code is right.

# warlock

warlock is the workspace root for the warlock project: a Rust Cargo workspace of two crates, warlock-engine and warlock-tui, that together build the `warlock` binary, a freshness-ledger tool for documentation.

## Files

- `Cargo.lock` (56.8 KB) — Auto-generated lockfile pinning exact versions and checksums for every dependency resolved across the workspace, including warlock-engine and warlock-tui themselves.
- `Cargo.toml` (7.4 KB) — Workspace manifest: declares the members (crates/warlock-engine, crates/warlock-tui), shared workspace.package metadata, workspace-wide rustc and clippy lint config, and the workspace.dependencies table with a reasoned comment on each crate taken.
- `LICENSE` (11.0 KB) — Full text of the Apache License, Version 2.0, under which the project is licensed.
- `rustfmt.toml` (1.3 KB) — rustfmt configuration for the workspace: pins edition = "2024", style_edition = "2024", newline_style = "Unix", and use_field_init_shorthand = true, each with a stated reason.

## Directories

- `crates/` — Workspace root holding warlock-engine (domain/lifecycle core) and warlock-tui (terminal front end and CLI) — go here for any source-level question.

## Structure

- The workspace member list in Cargo.toml wires crates/warlock-engine and crates/warlock-tui into one build.
- rustfmt.toml pins edition and style_edition to match [workspace.package] edition = "2024" in Cargo.toml.
- Cargo.lock records the resolved dependency graph, including the warlock-engine and warlock-tui package entries, for the manifest declared in Cargo.toml.

## Rules

- unsafe_code is denied workspace-wide: there is no unsafe in warlock except via an explicit, commented #[allow].
- CI runs cargo clippy --workspace --all-targets -- -D warnings, so every workspace lint warning fails the build.
- Any lint set to allow in the workspace manifest must carry a comment directly above it explaining why.
- clippy::nursery and clippy::cargo are deliberately left off the workspace lint groups.
- Any dependency added to workspace.dependencies needs a one-line comment above it justifying the supply-chain cost.
- rustfmt.toml stays close to rustfmt defaults; only options with a stated reason are set, and nothing nightly-only, since the local toolchain is nixpkgs stable rustc with no rustup shims.
- rustfmt forces newline_style = "Unix" so --check formatting is platform-independent given CI runs on Linux.

## Where to look

- what crates make up this project and how they depend on each other → `crates` `warlock-tui`
- why a particular third-party crate is in the dependency tree → `Cargo.toml` `workspace.dependencies`
- what license the project ships under → `LICENSE` `Apache License`
- what rustc edition or toolchain the workspace targets → `Cargo.toml` `rust-version`
- why unsafe code is disallowed in this codebase → `Cargo.toml` `unsafe_code`
- exact pinned versions of a dependency in use → `Cargo.lock` `warlock-engine`
- why formatting looks the way it does or how to reproduce it in CI → `rustfmt.toml` `newline_style`
