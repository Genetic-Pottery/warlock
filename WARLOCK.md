<!-- warlock -->
> Written by a model pass over this directory alone, to be read before its source and to say which source to read. A map, not a specification: check anything you are about to rely on against the files themselves, and where this document and the code disagree, the code is right.

# warlock

The project's workspace root: a two-crate Rust project (warlock-engine and warlock-tui) built under edition 2024, pinning its own dependency versions and formatting rules and carrying its Apache 2.0 license.

## Files

- `Cargo.lock` (78.6 KB) — Cargo-generated lockfile (version 4) pinning exact crate versions and checksums for the workspace, including warlock-engine and warlock-tui plus ratatui, clap, ureq, notify, blake3, ignore and serde. Not for manual editing.
- `Cargo.toml` (10.4 KB) — Cargo workspace manifest: members crates/warlock-engine and crates/warlock-tui; shared package metadata (edition 2024), lint rules, and pinned dependency versions (serde, ratatui, clap, blake3, notify, ignore, ureq).
- `LICENSE` (11.0 KB) — Apache License 2.0, the standard boilerplate legal text; no project-specific content.
- `rustfmt.toml` (1.3 KB) — rustfmt.toml: workspace formatting config — pins edition 2024, style_edition 2024, LF newlines, and field init shorthand, deviating from defaults only where noted.

## Directories

- `crates/` — Holds the two workspace crates, warlock-engine (the repo-modeling engine) and warlock-tui (the terminal UI and CLI) — go there for how either crate works.

## Structure

- Cargo.toml declares the workspace members crates/warlock-engine and crates/warlock-tui, plus shared edition, lint and dependency versions
- Cargo.lock pins exact resolved versions and checksums for the workspace's dependencies, including warlock-engine and warlock-tui
- rustfmt.toml sets the formatting rules (edition 2024, style_edition 2024, LF newlines, field init shorthand) applied across the workspace
- LICENSE provides the Apache License 2.0 text covering the project
