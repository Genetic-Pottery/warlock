<!-- warlock -->
> Written by a model pass over this directory alone, to be read before its source and to say which source to read. A map, not a specification: check anything you are about to rely on against the files themselves, and where this document and the code disagree, the code is right.

# warlock

Root of the Cargo workspace holding the warlock-engine and warlock-tui crates. Carries the workspace manifest, lockfile, license and rustfmt config; the source lives under crates/.

## Files

- `Cargo.lock` (64.1 KB) — Cargo.lock — auto-generated dependency lockfile for workspace crates warlock-engine and warlock-tui; not hand-edited.
- `Cargo.toml` (9.0 KB) — Workspace manifest for warlock-engine and warlock-tui: shared lint config, and dependency choices (serde, serde_json, toml, ratatui, ignore, blake3, notify, clap, ctrlc, arboard) each justified inline.
- `LICENSE` (11.0 KB) — Apache License 2.0, the standard boilerplate legal text; no project-specific content.
- `rustfmt.toml` (1.3 KB) — rustfmt.toml: workspace formatting config — pins edition 2024, style_edition 2024, LF newlines, and field init shorthand, deviating from defaults only where noted.

## Directories

- `crates/` — Workspace member crates: warlock-engine (library: walk, freshness, hashing, document fill/render, JSON contract) and warlock-tui (TUI and the `warlock` CLI binary). Open for any code question.

## Structure

- Cargo.toml defines the workspace for warlock-engine and warlock-tui, with shared lint config and inline justification for each dependency choice.
- Cargo.lock is the auto-generated lockfile for the workspace crates and is not hand-edited.
- rustfmt.toml sets workspace formatting: edition 2024, style_edition 2024, LF newlines, and field init shorthand.
- LICENSE is the standard Apache License 2.0 text with no project-specific content.
