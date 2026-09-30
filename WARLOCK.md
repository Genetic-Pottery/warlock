<!-- warlock -->
> Written by a model pass over this directory alone, to be read before its source and to say which source to read. A map, not a specification: check anything you are about to rely on against the files themselves, and where this document and the code disagree, the code is right.

# warlock

The workspace root of the warlock Rust project: a Cargo workspace tying together the warlock-engine and warlock-tui crates under shared package metadata, lint rules, and pinned dependency versions.

## Files

- `Cargo.lock` (78.6 KB) — Cargo-generated lockfile (version 4) pinning exact crate versions and checksums for the workspace, including warlock-engine and warlock-tui plus ratatui, clap, ureq, notify, blake3, ignore and serde. Not for manual editing.
- `Cargo.toml` (10.4 KB) — Cargo workspace manifest: members crates/warlock-engine and crates/warlock-tui; shared package metadata (edition 2024), lint rules, and pinned dependency versions (serde, ratatui, clap, blake3, notify, ignore, ureq).
- `LICENSE` (11.0 KB) — Apache License 2.0, the standard boilerplate legal text; no project-specific content.
- `rustfmt.toml` (1.3 KB) — rustfmt.toml: workspace formatting config — pins edition 2024, style_edition 2024, LF newlines, and field init shorthand, deviating from defaults only where noted.

## Directories

- `crates/` — Holds the warlock-engine and warlock-tui crates — go here for engine behavior or command/view implementation.

## Structure

- Cargo.lock — Cargo-generated lockfile (version 4) pinning exact crate versions and checksums for the workspace, including warlock-engine and warlock-tui plus ratatui, clap, ureq, notify, blake3, ignore and serde. Not for manual editing.
- Cargo.toml — Cargo workspace manifest: members crates/warlock-engine and crates/warlock-tui; shared package metadata (edition 2024), lint rules, and pinned dependency versions (serde, ratatui, clap, blake3, notify, ignore, ureq).
- rustfmt.toml — workspace formatting config — pins edition 2024, style_edition 2024, LF newlines, and field init shorthand, deviating from defaults only where noted.
- LICENSE — Apache License 2.0, the standard boilerplate legal text; no project-specific content.
