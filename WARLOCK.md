<!-- warlock -->
> Written by a model pass over this directory alone, to be read before its source and to say which source to read. A map, not a specification: check anything you are about to rely on against the files themselves, and where this document and the code disagree, the code is right.

# warlock

Repository root of the warlock workspace: a Cargo workspace manifest and lockfile tying together the warlock-engine and warlock-tui crates, plus licensing, formatting config, and a recorded terminal session demonstrating the warlock TUI.

## Files

- `Cargo.lock` (78.6 KB) — Cargo-generated lockfile pinning exact versions/checksums for warlock-engine and warlock-tui's dependency graph; not hand-edited.
- `Cargo.toml` (10.4 KB) — Workspace manifest: members warlock-engine and warlock-tui, shared package metadata, lint config, and pinned dependency versions (serde, ratatui, clap, blake3, notify, etc.).
- `LICENSE` (11.0 KB) — Full text of the Apache License, Version 2.0, under which this repository is licensed.
- `brief.cast` (71.2 KB) — Asciinema cast of a cargo run session: `warlock` TUI boot, brief mode, and a test-oracle conversation; terminal replay data, not readable code.
- `rustfmt.toml` (1.3 KB) — rustfmt.toml: formatting config — edition 2024, style_edition 2024, Unix newlines, field init shorthand enabled.

## Directories

- `crates/` — Workspace members warlock-engine and warlock-tui; go here for engine logic, terminal UI, CLI or client-integration questions.

## Structure

- Workspace manifest: members warlock-engine and warlock-tui, shared package metadata, lint config, and pinned dependency versions (serde, ratatui, clap, blake3, notify, etc.).
- Cargo-generated lockfile pinning exact versions/checksums for warlock-engine and warlock-tui's dependency graph; not hand-edited.
- Asciinema cast of a cargo run session: warlock TUI boot, brief mode, and a test-oracle conversation; terminal replay data, not readable code.
- rustfmt.toml: formatting config — edition 2024, style_edition 2024, Unix newlines, field init shorthand enabled.
- Full text of the Apache License, Version 2.0, under which this repository is licensed.
