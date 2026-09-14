<!-- warlock -->
> Written by a model pass over this directory alone, to be read before its source and to say which source to read. A map, not a specification: check anything you are about to rely on against the files themselves, and where this document and the code disagree, the code is right.

# crates

The workspace's crates directory: the two-crate split between warlock-engine, the domain logic, and warlock-tui, the terminal front end shipping the warlock binary.

## Directories

- `warlock-engine/` — The domain crate — pacting, the freshness decision, the document schema, fitting and elision, the Agent port — no TUI, terminal, HTTP or Anthropic dependency; open for how the engine's contract works.
- `warlock-tui/` — The terminal front end crate shipping the warlock binary — state, panels, composer, conversation, claude subprocess, filesystem watch, headless subcommands; open for terminal UI, keys, or subcommand behaviour.

## Structure

- The dependency edge runs warlock-tui -> warlock-engine: the TUI crate depends on the engine crate for domain logic, never the reverse.

## Where to look

- how the freshness document schema and pacting logic work → `warlock-engine` `pact_subtree`
- how the terminal UI, keys, and panel behaviour work → `warlock-tui` `action_for`
- why the engine has no HTTP or Anthropic dependency → `warlock-engine`
- building or running the warlock binary → `warlock-tui` `warlock`
