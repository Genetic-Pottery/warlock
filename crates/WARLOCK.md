<!-- warlock -->
> Written by a model pass over this directory alone, to be read before its source and to say which source to read. A map, not a specification: check anything you are about to rely on against the files themselves, and where this document and the code disagree, the code is right.

# crates

The crates directory holds the workspace's Rust crates: warlock-engine, the module tree, freshness and pact engine library, and warlock-tui, the front end and CLI that builds the warlock binary on top of it.

## Directories

- `warlock-engine/` — Engine crate root; module tree, per-file freshness against pacts.toml, and the describe-then-grant pact engine behind an Agent boundary — go here for freshness or pact questions.
- `warlock-tui/` — Front end and CLI crate; panel/app state, boundary and scope enforcement, agent chat driving /pact /refresh /brief /write, terminal rendering — go here for key press, command or frame questions.

## Structure

- The warlock-engine crate: builds the module tree from a repo walk, computes per-file freshness against a pacts.toml manifest, and drives the describe-then-grant pact engine reading and writing WARLOCK.md documents through an Agent boundary.
- The warlock-tui crate: builds the `warlock` binary and warlock_tui library, depending on warlock-engine.
