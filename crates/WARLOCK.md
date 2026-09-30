<!-- warlock -->
> Written by a model pass over this directory alone, to be read before its source and to say which source to read. A map, not a specification: check anything you are about to rely on against the files themselves, and where this document and the code disagree, the code is right.

# crates

The crates directory holds the workspace's two Cargo packages: the warlock-engine library and the warlock-tui binary/library built on it.

## Directories

- `warlock-engine/` — Repo-modeling and document engine — Tree of pacted/unpacted modules, hashing, agent-driven fill/validation, scope/sigil enforcement.
- `warlock-tui/` — Terminal UI and CLI crate — pact/refresh/pull/cut/push/chat/brief flows, Linear/git/gh/claude integrations.
