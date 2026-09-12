<!-- warlock -->
> Written by a model pass over this directory alone, to be read before its source and to say which source to read. A map, not a specification: check anything you are about to rely on against the files themselves, and where this document and the code disagree, the code is right.

# crates

crates is the workspace root holding warlock's two crates: warlock-engine, which pacts a directory into a WARLOCK.md and judges freshness, and warlock-tui, the terminal front end shipping the warlock binary.

## Directories

- `warlock-engine/` — The pacting, hashing, scoping and manifest logic; open for how a directory's WARLOCK.md gets generated or judged fresh/stale.
- `warlock-tui/` — The terminal front end and warlock binary: pure app/panel/composer state plus the claude.rs adapter and watch.rs; open for how the panel runs or what the binary does.

## Structure

- dependency edge runs warlock-tui -> warlock-engine, never the reverse
- warlock-engine owns the JSON pact schema; warlock-tui consumes it via serde_json but adds no domain logic of its own
- lint configuration for both crates is inherited from the workspace's [workspace.lints], not set per-crate

## Rules

- warlock-engine carries no TUI, terminal, HTTP or Anthropic dependency
- the engine owns the JSON contract a pass fills in; the TUI must not reimplement it
- ctrlc is only ever pulled into warlock-tui from src/running.rs

## Where to look

- how a directory gets pacted into a WARLOCK.md → `warlock-engine` `pact_subtree`
- what binary this workspace produces → `warlock-tui` `main.rs`
- where the JSON schema for pass output lives → `warlock-engine` `serde_json`
- which crate depends on which → `warlock-tui` `warlock-engine`
