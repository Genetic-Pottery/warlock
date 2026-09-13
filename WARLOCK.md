<!-- warlock -->
> Written by a model pass over this directory alone, to be read before its source and to say which source to read. A map, not a specification: check anything you are about to rely on against the files themselves, and where this document and the code disagree, the code is right.

# warlock

The workspace root of warlock, a two-crate Rust project: warlock-engine (domain logic) and warlock-tui (terminal front end shipping the warlock binary), plus workspace-wide manifest, lockfile, license and formatting configuration.

## Files

- `Cargo.lock` (56.8 KB) — Generated, exact dependency graph for the workspace; lists warlock-engine and warlock-tui and every pinned crate version they pull in.
- `Cargo.toml` (7.4 KB) — Workspace manifest: members, workspace.package, workspace.lints (unsafe_code deny, pedantic warn, missing_docs allow), workspace.dependencies with rationale.
- `LICENSE` (11.0 KB) — Full text of the Apache License, Version 2.0 under which the project is licensed.
- `rustfmt.toml` (1.3 KB) — Formatting config: edition = "2024", style_edition = "2024", newline_style = "Unix", use_field_init_shorthand = true.

## Directories

- `crates/` — The two-crate Rust source tree, warlock-engine (domain logic) and warlock-tui (terminal front end and binary); open it for how pact/refresh/freshness/validation logic and the terminal UI divide and connect.

## Structure

- The workspace resolver and members list in Cargo.toml point at crates/warlock-engine and crates/warlock-tui, the only two crates built here.
- Cargo.lock is generated from Cargo.toml and records the fully resolved dependency versions for both crates.
- rustfmt.toml's edition and style_edition settings track the edition declared in Cargo.toml's workspace.package.

## Rules

- unsafe_code is denied workspace-wide: there is no unsafe in warlock.
- missing_docs is allowed because brief 15 deletes source prose and writes comments back only where needed, not as doc-comment boilerplate.
- clippy::pedantic is warned as a group with priority -1 so individual allows below can override it; clippy::nursery and clippy::cargo are deliberately left off.
- CI runs cargo clippy --workspace --all-targets -- -D warnings, so anything warned here fails the build there.
- Every workspace.dependencies entry must carry a one-line comment above it justifying the dependency.
- rustfmt.toml stays close to rustfmt defaults; only options with a stated reason belong in it, and nothing nightly-only, since the local toolchain is nixpkgs stable rustc with no rustup shims.
- newline_style is forced to Unix so cargo fmt --check is platform-independent.

## Where to look

- what crates make up the project and how they divide responsibility → `crates` `warlock-engine`
- why a particular dependency like ratatui, ignore, blake3, notify, clap or ctrlc was chosen → `Cargo.toml` `workspace.dependencies`
- what lint settings apply to all crates and why → `Cargo.toml` `workspace.lints`
- which Rust edition and formatting conventions apply → `rustfmt.toml` `edition`
- licensing terms for the project → `LICENSE` `Apache License`
- the exact resolved version of a transitive dependency → `Cargo.lock` `warlock-tui`
