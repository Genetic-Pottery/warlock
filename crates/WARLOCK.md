<!-- warlock -->
> Written by a model pass over this directory alone, to be read before its source and to say which source to read. A map, not a specification: check anything you are about to rely on against the files themselves, and where this document and the code disagree, the code is right.

# crates

Holds the two crates making up the project: warlock-engine, which walks and analyzes a repo and orchestrates document fill/validation, and warlock-tui, which provides the CLI and terminal UI built on it.

## Directories

- `warlock-engine/` — The engine crate: repo walking, hashing, scoping, agent orchestration, and pact/pull/draft state; go here for questions about how documents get filled or validated.
- `warlock-tui/` — The TUI/CLI crate: App state, rendering, and the pact/pull/push/cut/draft/refresh subcommands; go here for questions about CLI commands or terminal UI behavior.

## Structure

- The warlock-engine crate: it walks a repo, hashes and scopes directories, orchestrates LLM agents to fill and validate WARLOCK.md documents, and tracks pact/pull/draft state across runs.
- The warlock-tui crate: a binary (warlock) and library (warlock_tui) providing the terminal UI and CLI, with App state driving tree/panel rendering and the pact, pull, push, cut, draft and refresh subcommand flows against a repo's manifest and boundary.
