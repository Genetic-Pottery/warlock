<!-- warlock -->
> Written by a model pass over this directory alone, to be read before its source and to say which source to read. A map, not a specification: check anything you are about to rely on against the files themselves, and where this document and the code disagree, the code is right.

# crates

The crates directory holds the workspace's two Cargo packages: the warlock-engine library and the warlock-tui binary and library, splitting core repo/pact/pull logic from the terminal UI and CLI.

## Directories

- `warlock-engine/` — Core library for walking/hashing repo trees, pacted freshness, LLM document fills, scopes, sigils, keys, pulls, drafting and ticket-splitting/filing; go here for engine logic questions.
- `warlock-tui/` — Terminal UI and CLI binary (warlock) for filing briefs to Linear, pulling tickets through agent-run work, and maintaining a repo's freshness ledger; go here for interface, CLI or client-integration questions.

## Structure

- warlock-engine is a Cargo library providing core logic for walking/hashing repo trees, tracking pacted freshness, driving LLM document fills, and managing scopes, sigils, keys, pulls, drafting and ticket-splitting/filing.
- warlock-tui is a terminal UI and CLI binary (warlock) for filing briefs to Linear, pulling tickets through agent-run work, and maintaining a repo's freshness ledger across scopes and sigils; depends on warlock-engine.
