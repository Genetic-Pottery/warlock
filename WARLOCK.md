<!-- warlock -->
> Written by a model pass over this directory alone, to be read before its source and to say which source to read. A map, not a specification: check anything you are about to rely on against the files themselves, and where this document and the code disagree, the code is right.

# warlock

The workspace root for warlock, holding the crates/ workspace (warlock-engine and warlock-tui) plus shared build, lockfile, license and formatting configuration.

## Files

- `Cargo.lock` (56.8 KB) — Auto-generated lockfile pinning exact versions and checksums for every crate dependency of warlock-engine and warlock-tui; not hand-edited.
- `Cargo.toml` (7.4 KB) — Workspace manifest for warlock-engine and warlock-tui: shared lints, package metadata, and pinned dependencies (serde, ratatui, ignore, blake3, notify, clap, ctrlc).
- `LICENSE` (11.0 KB) — Full text of the Apache License, Version 2.0, governing use and redistribution of the repository's contents.
- `rustfmt.toml` (1.3 KB) — rustfmt.toml — workspace formatting config: pins edition 2024, style_edition 2024, LF newlines, and struct field-init shorthand.

## Directories

- `crates/` — The workspace's two crates, engine and tui; go there for how state, pacts, documents, or screen output are computed.

## Structure

- Auto-generated lockfile pinning exact versions and checksums for every crate dependency of warlock-engine and warlock-tui; not hand-edited.
- Workspace manifest for warlock-engine and warlock-tui: shared lints, package metadata, and pinned dependencies (serde, ratatui, ignore, blake3, notify, clap, ctrlc).
- Full text of the Apache License, Version 2.0, governing use and redistribution of the repository's contents.
- rustfmt.toml — workspace formatting config: pins edition 2024, style_edition 2024, LF newlines, and struct field-init shorthand.

## Where to look

- what decides whether a file is stale or fresh → `crates` `decide_state`
- what crate builds the warlock binary → `crates` `warlock`
- where is WARLOCK.md actually written and merged into CLAUDE.md → `crates` `write_claude_md`
- what dependencies does the tui pull in → `crates`
- how are two clones of the same repo verified to agree → `crates` `subtree_hash`
- where is the terminal-free library surface defined → `crates` `warlock_tui`
- where does the model-pass request/response shape live → `crates` `Request`
- how does the ledger tree get built from disk → `crates` `load_tree`
- where are pacts.toml entries read and saved → `crates` `Manifest`
- what performs a pact or refresh over a subtree → `crates` `pact_subtree`
- where do sigils get resolved to a config directory → `crates` `sigils_path`
- how is a directory excluded via .warlockignore → `crates` `is_ignored`
