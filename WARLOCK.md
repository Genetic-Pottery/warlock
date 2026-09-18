<!-- warlock -->
> Written by a model pass over this directory alone, to be read before its source and to say which source to read. A map, not a specification: check anything you are about to rely on against the files themselves, and where this document and the code disagree, the code is right.

# warlock

Repository root of the workspace, holding the Cargo workspace manifest, its lockfile, licensing, and formatting configuration for the warlock-engine and warlock-tui crates.

## Files

- `Cargo.lock` (64.1 KB) — Cargo.lock — auto-generated dependency lockfile for workspace crates warlock-engine and warlock-tui; not hand-edited.
- `Cargo.toml` (9.0 KB) — Workspace manifest for warlock-engine and warlock-tui: shared lint config, and dependency choices (serde, serde_json, toml, ratatui, ignore, blake3, notify, clap, ctrlc, arboard) each justified inline.
- `LICENSE` (11.0 KB) — Apache License 2.0, the standard boilerplate legal text; no project-specific content.
- `rustfmt.toml` (1.3 KB) — rustfmt.toml: workspace formatting config — pins edition 2024, style_edition 2024, LF newlines, and field init shorthand, deviating from defaults only where noted.

## Directories

- `crates/` — Holds the two workspace crates, warlock-engine (core freshness logic) and warlock-tui (CLI/UI, builds the `warlock` binary).

## Structure

- Workspace manifest for warlock-engine and warlock-tui: shared lint config, and dependency choices (serde, serde_json, toml, ratatui, ignore, blake3, notify, clap, ctrlc, arboard) each justified inline.
- Cargo.lock — auto-generated dependency lockfile for workspace crates warlock-engine and warlock-tui; not hand-edited.
- rustfmt.toml: workspace formatting config — pins edition 2024, style_edition 2024, LF newlines, and field init shorthand, deviating from defaults only where noted.
- Apache License 2.0, the standard boilerplate legal text; no project-specific content.
