<!-- warlock -->
> Written by a model pass over this directory alone, to be read before its source and to say which source to read. A map, not a specification: check anything you are about to rely on against the files themselves, and where this document and the code disagree, the code is right.

# crates

Workspace directory holding warlock's two crates: warlock-engine, the pure domain core for pacting repositories, and warlock-tui, the front-end crate shipping the warlock binary and its terminal panel.

## Directories

- `warlock-engine/` — The domain core: manifest, freshness ledger, module document tree, agent seam, request fitting; open for questions about pacting logic with no terminal or network dependency.
- `warlock-tui/` — The front-end crate: warlock binary, terminal panel, headless subcommands, and the warlock_tui library of tree/panel/account/thread state.

## Structure

- warlock-tui depends on warlock-engine; the dependency edge runs TUI -> engine and never back
- warlock-engine has no TUI, terminal, HTTP or Anthropic dependency

## Rules

- Lint configuration is set once in the workspace root manifest's [workspace.lints] and inherited by both member crates

## Where to look

- domain vocabulary for pacting a repository, manifest, freshness ledger → `warlock-engine` `pact_subtree`
- the warlock binary, terminal panel, headless subcommands → `warlock-tui` `warlock_tui`
