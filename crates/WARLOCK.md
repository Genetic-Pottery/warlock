<!-- warlock -->
> Written by a model pass over this directory alone, to be read before its source and to say which source to read. A map, not a specification: check anything you are about to rely on against the files themselves, and where this document and the code disagree, the code is right.

# crates

Cargo workspace members for the project: the warlock-engine library (module tree, freshness decisions, documentation generation, JSON document contract) and the warlock-tui crate (terminal UI and the `warlock` command-line binary).

## Directories

- `warlock-engine/` — Engine library crate: repo-walk module tree, pact manifest, per-directory freshness, hashing, document fill and rendering, scope/sigil/key config. Open for logic and JSON document contract questions.
- `warlock-tui/` — Terminal UI and CLI crate: builds the `warlock` binary with clap subcommands (init, config, stale, fresh, check, pact, refresh, scope, key), TUI state, input, drawing, agent chat, watching.

## Structure

- warlock-tui depends on warlock-engine, so the dependency runs from the UI/CLI crate to the engine library.
- warlock-engine owns the JSON document contract, and warlock-tui also depends on serde_json.
