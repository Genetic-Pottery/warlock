<!-- warlock -->
> Written by a model pass over this directory alone, to be read before its source and to say which source to read. A map, not a specification: check anything you are about to rely on against the files themselves, and where this document and the code disagree, the code is right.

# warlock

Root of the Cargo workspace containing the warlock-engine and warlock-tui crates. Holds the workspace manifest with shared lint config and dependency choices, the lockfile, rustfmt settings, and the Apache 2.0 license.

## Files

- `Cargo.lock` (64.1 KB) — Cargo.lock — auto-generated dependency lockfile for workspace crates warlock-engine and warlock-tui; not hand-edited.
- `Cargo.toml` (9.0 KB) — Workspace manifest for warlock-engine and warlock-tui: shared lint config, and dependency choices (serde, serde_json, toml, ratatui, ignore, blake3, notify, clap, ctrlc, arboard) each justified inline.
- `LICENSE` (11.0 KB) — Apache License 2.0, the standard boilerplate legal text; no project-specific content.
- `rustfmt.toml` (1.3 KB) — rustfmt.toml: workspace formatting config — pins edition 2024, style_edition 2024, LF newlines, and field init shorthand, deviating from defaults only where noted.

## Directories

- `crates/` — Holds the two workspace crates: warlock-engine (library: tree loading, freshness, hashing, document fill and rendering, JSON contract) and warlock-tui (the `warlock` binary, CLI, terminal UI, agent). Go here for any source code.

## Structure

- Cargo.toml is the workspace manifest for warlock-engine and warlock-tui, and Cargo.lock is the auto-generated lockfile covering those same two crates.
- Formatting for the whole workspace is set in rustfmt.toml, which pins edition 2024 and style_edition 2024.
- warlock-tui depends on warlock-engine; the engine does not depend on the front-end.
