<!-- warlock -->
> Written by a model pass over this directory alone, to be read before its source and to say which source to read. A map, not a specification: check anything you are about to rely on against the files themselves, and where this document and the code disagree, the code is right.

# crates

The crates directory is the workspace root of warlock's Rust source: warlock-engine, the domain logic for pact/refresh/freshness/validation with no terminal dependency, and warlock-tui, the terminal front end shipping the warlock binary; open it for how the two crates divide domain logic from presentation.

## Directories

- `warlock-engine/` — The domain crate: pact/refresh orchestration, freshness hashing, document schema/validation, request fitting, manifests, scopes and sigils, with no TUI/HTTP dependency of its own.
- `warlock-tui/` — The terminal front end: the warlock binary plus the warlock_tui library of pure App/Account/Thread/Panel/Composer state, colours and wrapping, assembled by main.rs.

## Structure

- The dependency edge runs warlock-tui -> warlock-engine: the front end knows domain vocabulary, the engine knows nothing of terminals.

## Rules

- warlock-engine pins no TUI, terminal, HTTP or Anthropic dependency since that edge only ever runs from warlock-tui.
- Lint configuration for both crates is inherited from the workspace's [workspace.lints] rather than set per crate.

## Where to look

- how the pact/refresh/validate/render logic works → `warlock-engine` `pact_subtree`
- where the warlock binary starts and what state backs the panel → `warlock-tui` `main.rs`
