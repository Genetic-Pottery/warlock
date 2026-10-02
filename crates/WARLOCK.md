<!-- warlock -->
> Written by a model pass over this directory alone, to be read before its source and to say which source to read. A map, not a specification: check anything you are about to rely on against the files themselves, and where this document and the code disagree, the code is right.

# crates

The crates directory holding the two crates that make up the workspace: warlock-engine, the repository-walking and document-filling engine, and warlock-tui, the CLI and TUI built on top of it.

## Directories

- `warlock-engine/` — Walks repos, hashes and decides freshness, fills and renders WARLOCK.md documents, and tracks pacts, scopes, sigils and pulls; go here for engine logic.
- `warlock-tui/` — Packages the warlock CLI and TUI, running the pact/brief/draft/pull/push engine against a repo's boundary scopes; go here for command or UI behavior.
