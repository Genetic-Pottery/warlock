# warlock-engine

The core library crate of warlock. It owns the domain logic — the state
vocabulary, the tree of work, and the rules that move it forward. So far that
is `NodeState`, the three-state model from section 5 of the design doc —
unpacted, pacted-and-stale, pacted-and-fresh, with no "unknown" fourth state
because unjudged *is* stale. The tree types and the rules arrive in a later
slice.

The types derive serde's `Serialize`/`Deserialize` so a caller can choose a
format, but the crate commits to none: it reads and writes no files.

## The dependency edge runs one way

`warlock-engine` never depends on the TUI, and never depends on any terminal
crate — no `warlock-tui`, no Ratatui, no Crossterm, no terminal or rendering
library of any kind. The dependency edge runs **TUI → engine, and never back**.

This is an architectural boundary, not a convention. The engine must stay
usable without a terminal attached, so that another front end (or a test
harness) can drive it directly. Any change that would make the engine reach
back toward the TUI is a change to the architecture and should be treated as
one.
