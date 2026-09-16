<!-- warlock -->
> Written by a model pass over this directory alone, to be read before its source and to say which source to read. A map, not a specification: check anything you are about to rely on against the files themselves, and where this document and the code disagree, the code is right.

# warlock

Workspace root for the warlock-engine and warlock-tui crates, holding the Cargo manifest and lockfile, license, and shared rustfmt config that govern the whole build.

## Files

- `Cargo.lock` (64.1 KB) — Cargo-generated lockfile pinning exact dependency versions for the warlock-engine and warlock-tui crates; not hand-edited.
- `Cargo.toml` (9.0 KB) — Workspace manifest for the warlock-engine and warlock-tui crates: shared lints, edition 2024, and the dependency list with rationale for each crate.
- `LICENSE` (11.0 KB) — Full text of the Apache License, Version 2.0, governing use and redistribution of the repository's contents.
- `rustfmt.toml` (1.3 KB) — rustfmt.toml — workspace formatting config: pins edition 2024, style_edition 2024, LF newlines, and struct field-init shorthand.

## Directories

- `crates/` — Holds the warlock-engine (freshness-ledger core) and warlock-tui (terminal UI, warlock binary) crates; go here for either crate's source.

## Structure

- Workspace manifest for the warlock-engine and warlock-tui crates: shared lints, edition 2024, and the dependency list with rationale for each crate.
- Cargo-generated lockfile pinning exact dependency versions for the warlock-engine and warlock-tui crates; not hand-edited.
- rustfmt.toml — workspace formatting config: pins edition 2024, style_edition 2024, LF newlines, and struct field-init shorthand.
- Full text of the Apache License, Version 2.0, governing use and redistribution of the repository's contents.
