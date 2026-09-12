<!-- warlock -->
> Written by a model pass over this directory alone, to be read before its source and to say which source to read. A map, not a specification: check anything you are about to rely on against the files themselves, and where this document and the code disagree, the code is right.

# warlock

The workspace root of warlock, a two-crate Rust project: warlock-engine pacts a directory into a WARLOCK.md and judges freshness, warlock-tui is the terminal front end shipping the warlock binary.

## Files

- `Cargo.lock` (56.8 KB) — Generated lockfile pinning exact versions of every dependency for warlock-engine and warlock-tui, including ratatui, clap, notify, blake3, serde_json.
- `Cargo.toml` (7.4 KB) — Workspace manifest: members list, workspace.package metadata, shared [workspace.lints] for rust/clippy, and [workspace.dependencies] with rationale comments for serde, serde_json, toml, ratatui, ignore, blake3, notify, clap, ctrlc, serde_test, tempfile.
- `LICENSE` (11.0 KB) — Full text of the Apache License, Version 2.0.
- `rustfmt.toml` (1.3 KB) — rustfmt config for the workspace: edition = "2024", style_edition = "2024", newline_style = "Unix", use_field_init_shorthand = true, each with a one-line rationale.

## Directories

- `crates/` — The two crates warlock-engine and warlock-tui; open for how pacting/freshness logic works or how the terminal panel and binary run.

## Structure

- workspace.dependencies in Cargo.toml is the single version source both crates in crates/ draw from
- workspace.lints in Cargo.toml is inherited by both crates rather than set per-crate
- rustfmt.toml and Cargo.toml both pin edition 2024 so formatting and compilation agree

## Rules

- unsafe_code = "deny" workspace-wide; any exception needs an explicit #[allow] with a comment a reviewer will see
- missing_docs is allowed because brief 15 deletes source prose and writes comments back only where needed
- missing_errors_doc and missing_panics_doc are allowed for the same brief-15 reason
- clippy::pedantic is enabled as a group; clippy::nursery and clippy::cargo are deliberately not enabled
- any workspace.dependencies entry needs a one-line comment above it saying why it's worth the supply chain
- any workspace.lints allow needs a comment directly above it saying why
- rustfmt.toml stays close to rustfmt defaults; only options with a stated reason belong there
- no nightly-only rustfmt options, since the local toolchain is nixpkgs stable rustc with no rustup shims

## Where to look

- why is a dependency in the tree → `Cargo.toml` `workspace.dependencies`
- exact pinned version of a transitive dependency → `Cargo.lock` `warlock-engine`
- how pacting or freshness judging works → `crates` `warlock-engine`
- how the terminal panel or warlock binary runs → `crates` `warlock-tui`
- formatting rules for the codebase → `rustfmt.toml` `style_edition`
- licensing terms → `LICENSE` `Apache License`
- which lints are denied vs allowed and why → `Cargo.toml` `workspace.lints`
