<!-- warlock -->
> Written by a model pass over this directory alone, to be read before its source and to say which source to read. A map, not a specification: check anything you are about to rely on against the files themselves, and where this document and the code disagree, the code is right.

# warlock

The repository root of warlock: a Cargo workspace with two crates, warlock-engine and warlock-tui, plus licensing, formatting config, and a demo recording of a warlock TUI session.

## Files

- `Cargo.lock` (78.6 KB) — Cargo.lock: generated lockfile pinning exact dependency versions/checksums for the warlock-engine and warlock-tui crates; not edited by hand.
- `Cargo.toml` (10.4 KB) — Workspace manifest: members crates/warlock-engine and warlock-tui, shared package metadata, lints, and pinned dependency versions.
- `LICENSE` (11.0 KB) — Standard Apache License 2.0 full text; governs use and distribution of the repository's code.
- `brief.cast` (71.2 KB) — asciinema recording of a warlock TUI session drafting a brief about adding tests/a test oracle, Warp terminal cast format.
- `rustfmt.toml` (1.3 KB) — rustfmt config: edition 2024, style_edition 2024, Unix newlines, use_field_init_shorthand enabled.

## Directories

- `crates/` — Holds the warlock-engine core library crate and the warlock-tui terminal UI/CLI crate; go here for how the project's crates are organized or split.

## Structure

- Workspace manifest: members crates/warlock-engine and warlock-tui, shared package metadata, lints, and pinned dependency versions.
- Cargo.lock: generated lockfile pinning exact dependency versions/checksums for the warlock-engine and warlock-tui crates; not edited by hand.
- asciinema recording of a warlock TUI session drafting a brief about adding tests/a test oracle, Warp terminal cast format.
- rustfmt config: edition 2024, style_edition 2024, Unix newlines, use_field_init_shorthand enabled.
- Standard Apache License 2.0 full text; governs use and distribution of the repository's code.
