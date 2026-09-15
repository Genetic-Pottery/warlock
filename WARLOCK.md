<!-- warlock -->
> Written by a model pass over this directory alone, to be read before its source and to say which source to read. A map, not a specification: check anything you are about to rely on against the files themselves, and where this document and the code disagree, the code is right.

# warlock

Repository root for the workspace holding warlock-engine and warlock-tui, plus the top-level license, lockfile, manifest, and formatting config governing the whole project.

## Files

- `Cargo.lock` (56.8 KB) — Auto-generated lockfile pinning exact versions and checksums for every crate dependency of warlock-engine and warlock-tui; not hand-edited.
- `Cargo.toml` (7.4 KB) — Workspace manifest for warlock-engine and warlock-tui: shared lints, package metadata, and pinned dependencies (serde, ratatui, ignore, blake3, notify, clap, ctrlc).
- `LICENSE` (11.0 KB) — Full text of the Apache License, Version 2.0, governing use and redistribution of the repository's contents.
- `rustfmt.toml` (1.3 KB) — rustfmt.toml — workspace formatting config: pins edition 2024, style_edition 2024, LF newlines, and struct field-init shorthand.

## Directories

- `crates/` — Holds the two Rust crates, warlock-engine and warlock-tui, for questions about engine internals or the terminal front end/CLI.

## Structure

- Workspace manifest for warlock-engine and warlock-tui: shared lints, package metadata, and pinned dependencies (serde, ratatui, ignore, blake3, notify, clap, ctrlc).
- Auto-generated lockfile pinning exact versions and checksums for every crate dependency of warlock-engine and warlock-tui; not hand-edited.
- rustfmt.toml — workspace formatting config: pins edition 2024, style_edition 2024, LF newlines, and struct field-init shorthand.
- Full text of the Apache License, Version 2.0, governing use and redistribution of the repository's contents.
