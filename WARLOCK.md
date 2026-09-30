<!-- warlock -->
> Written by a model pass over this directory alone, to be read before its source and to say which source to read. A map, not a specification: check anything you are about to rely on against the files themselves, and where this document and the code disagree, the code is right.

# warlock

The workspace root: a Cargo workspace of two packages, warlock-engine and warlock-tui, with shared package metadata, lint rules and pinned dependency versions (serde, ratatui, clap, blake3, notify, ignore, ureq).

## Files

- `Cargo.lock` (78.6 KB) — Cargo-generated lockfile (version 4) pinning exact crate versions and checksums for the workspace, including warlock-engine and warlock-tui plus ratatui, clap, ureq, notify, blake3, ignore and serde. Not for manual editing.
- `Cargo.toml` (10.4 KB) — Cargo workspace manifest: members crates/warlock-engine and crates/warlock-tui; shared package metadata (edition 2024), lint rules, and pinned dependency versions (serde, ratatui, clap, blake3, notify, ignore, ureq).
- `LICENSE` (11.0 KB) — Apache License 2.0, the standard boilerplate legal text; no project-specific content.
- `rustfmt.toml` (1.3 KB) — rustfmt.toml: workspace formatting config — pins edition 2024, style_edition 2024, LF newlines, and field init shorthand, deviating from defaults only where noted.

## Directories

- `crates/` — Holds the workspace's two Cargo packages: warlock-engine (repo-modeling/document engine) and warlock-tui (terminal UI/CLI); go there for module structure or feature flows.

## Structure

- Cargo.toml lists workspace members crates/warlock-engine and crates/warlock-tui, and pins shared metadata, lints and dependency versions.
- Cargo.lock is generated from Cargo.toml, pinning exact versions and checksums for every crate the workspace depends on.
- rustfmt.toml sets formatting rules (edition 2024, style_edition 2024, LF newlines, field init shorthand) applied across the workspace's crates.
- LICENSE applies the Apache License 2.0 to the workspace as a whole; it has no dependency on other files.
