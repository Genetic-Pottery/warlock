<!-- warlock -->
> Written by a model pass over this directory alone, to be read before its source and to say which source to read. A map, not a specification: check anything you are about to rely on against the files themselves, and where this document and the code disagree, the code is right.

# warlock

The warlock workspace root: a Cargo workspace of two crates building the freshness-ledger tool that reads a repository and keeps its documentation honest, plus the license and formatting rules that govern it.

## Files

- `Cargo.lock` (56.8 KB) — Generated, exact dependency graph for the workspace, including warlock-engine and warlock-tui and every crate they pull in (ratatui, clap, notify, blake3, ignore, serde, toml, etc.).
- `Cargo.toml` (7.4 KB) — Workspace manifest: declares the two member crates, shared package metadata, the workspace-wide lint config (unsafe_code deny, pedantic warn, missing_docs allow), and workspace.dependencies with a comment justifying each dependency.
- `LICENSE` (11.0 KB) — Full text of the Apache License, Version 2.0 under which warlock is distributed.
- `rustfmt.toml` (1.3 KB) — rustfmt configuration: edition 2024, style_edition 2024, newline_style Unix, use_field_init_shorthand true, each pinned with a stated reason.

## Directories

- `crates/` — The two Rust crates, warlock-engine (core logic) and warlock-tui (terminal front end and binary); open for any question about engine types, pacts, scopes, sigils, or TUI panels, keys and subcommands.

## Structure

- crates/warlock-tui depends on crates/warlock-engine; the dependency runs tui -> engine, never the reverse
- Cargo.toml declares workspace.lints, which both crates opt into via `[lints] workspace = true` rather than repeating attributes

## Rules

- unsafe_code = "deny" workspace-wide: there is no unsafe in warlock
- missing_docs = "allow": brief 15 deletes source prose and restores comments only where their absence would let a correct-looking edit break something
- clippy::pedantic is warn-level as a group; clippy::nursery and clippy::cargo are deliberately not enabled
- any workspace.lints entry set to allow must carry a comment directly above it saying why
- any workspace.dependencies entry must carry a one-line comment above it saying why it's worth the supply chain
- CI runs cargo clippy --workspace --all-targets -- -D warnings, so anything warned about here fails the build
- rustfmt.toml stays close to rustfmt defaults; only options with a stated reason belong there, nothing nightly-only
- local toolchain is nixpkgs stable rustc with no rustup shims

## Where to look

- why a given dependency is in the tree → `Cargo.toml` `workspace.dependencies`
- what lints are enforced and why one is allowed → `Cargo.toml` `workspace.lints`
- exact pinned version of a transitive dependency → `Cargo.lock` `warlock-engine`
- engine types, pacts, scopes, sigils, or the Fill schema → `crates` `warlock-engine`
- the warlock binary, TUI panel state, or a subcommand → `crates` `warlock-tui`
- formatting conventions like brace style or newline handling → `rustfmt.toml` `newline_style`
- license terms for redistribution or modification → `LICENSE` `Apache License`
