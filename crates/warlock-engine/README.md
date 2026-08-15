# warlock-engine

The core library crate of warlock. It owns the domain logic — the state
vocabulary, the tree of work, and the rules that move it forward. So far that
is:

- `NodeState`, the three-state model from section 5 of the design doc —
  unpacted, pacted-and-stale, pacted-and-fresh, with no "unknown" fourth state
  because unjudged *is* stale.
- `Node`, one node of the project tree: its path, the path of its README, its
  state, and its children.
- `Tree`, which owns the root node.

`Node` and `Tree` are pure shape. Their fields are public so a renderer can
walk them with each node's depth and state in hand, and a caller builds them
directly with `Node::new` / `Tree::new`. A node's state is a plain stored
field: nothing here computes staleness. Building a tree from a real directory
is the job of a filesystem loader that does not exist yet, and the rules
arrive in a later slice.

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
