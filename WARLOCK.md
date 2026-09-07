<!-- warlock -->
> Written by a model pass over this directory alone, to be read before its source and to say which source to read. A map, not a specification: check anything you are about to rely on against the files themselves, and where this document and the code disagree, the code is right.

# warlock

Workspace root of warlock, a repository-pacting tool: manifest, license and toolchain config that bind the two crates together as one Cargo workspace.

## Files

- `Cargo.lock` (56.8 KB) — Generated, pinned dependency graph for the whole workspace; not hand-edited.
- `Cargo.toml` (6.9 KB) — Workspace manifest: members, workspace.package fields, [workspace.lints] for rustc/clippy, and [workspace.dependencies] with rationale per crate (serde, ratatui, ignore, blake3, notify, clap, ctrlc).
- `LICENSE` (11.0 KB) — Apache License, Version 2.0 full text.
- `rustfmt.toml` (1.3 KB) — Formatting config: edition/style_edition 2024, newline_style Unix, use_field_init_shorthand true, deliberately close to rustfmt defaults.

## Directories

- `crates/` — The two workspace member crates, warlock-engine (domain core) and warlock-tui (binary and terminal panel); open for any question about pacting logic or the TUI itself.

## Structure

- Cargo.toml declares crates/warlock-engine and crates/warlock-tui as workspace members
- Cargo.toml's [workspace.lints] is inherited by both member crates via [lints] workspace = true
- Cargo.lock is generated from Cargo.toml and the members' own manifests

## Rules

- unsafe_code = "deny" workspace-wide: no unsafe in warlock, any future use must carry an explicit #[allow] a reviewer will see
- CI runs cargo clippy --workspace --all-targets -- -D warnings, so every warning here fails the build
- Any lint set to allow must carry a comment directly above it saying why
- clippy::nursery and clippy::cargo are deliberately not enabled
- rustfmt parses as Rust 2024 and pins style_edition 2024 for reproducibility
- rustfmt always emits Unix (LF) line endings
- Rust comes from nixpkgs stable rustc, not rustup; nothing nightly-only belongs in rustfmt.toml

## Where to look

- why a dependency is in the workspace, e.g. why ratatui or notify or clap → `Cargo.toml` `workspace.dependencies`
- which crates make up the project and how they depend on each other → `crates` `warlock-engine`
- the lint policy CI enforces → `Cargo.toml` `workspace.lints`
- the project's license terms → `LICENSE` `Apache License`
- code formatting rules and why they deviate from rustfmt defaults → `rustfmt.toml` `style_edition`
