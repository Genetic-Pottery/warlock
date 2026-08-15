# warlock-engine

The core library crate of warlock. It owns the domain logic — the state
vocabulary, the tree of work, and the rules that move it forward. What exists
today is the vocabulary and the shape of the tree, plus a hard-coded tree to
render until real loading lands:

- `NodeState`, the three-state model from section 5 of the design doc —
  unpacted, pacted-and-stale, pacted-and-fresh, with no "unknown" fourth state
  because unjudged *is* stale.
- `Node`, one node of the project tree: its path, the path of its README, its
  state, and its children.
- `Tree`, which owns the root node and can be walked, tallied and searched:
  - `Tree::walk` — a depth-first iterator (`DepthFirst`) yielding every node
    with its depth, parents before children, siblings in stored order. The
    depth comes out of the walk because the renderer indents by it.
  - `Tree::counts` — a `StateCounts` with one field per state, so a state can
    neither be missed nor invented and an absent state counts zero.
  - `Tree::find` — the node at a given path, or `None`. Paths are compared as
    stored: no normalisation, no filesystem.
- `stub_tree`, a **placeholder** returning one small tree written out by hand —
  three levels deep, with at least one node in each state — so the engine/TUI
  seam can be exercised before any of it is real.

`Node` and `Tree` are pure shape. Their fields are public so a renderer can
walk them with each node's depth and state in hand, and a caller builds them
directly with `Node::new` / `Tree::new`. A node's state is a plain stored
field: nothing here computes staleness. Building a tree from a real directory
is the job of a filesystem loader that does not exist yet, and the rules
arrive in a later slice.

The types derive serde's `Serialize`/`Deserialize` so a caller can choose a
format, but the crate commits to none: it reads and writes no files. The
derives are tested by round-tripping through serde's own token stream
(`serde_test`, a dev-dependency), which never names a format.

## `stub_tree` is not the loader

`stub_tree` walks no directory, opens no file and computes no staleness. Every
path and every state it returns is a literal typed into `src/stub.rs`, chosen
only to give a renderer something with more than one level of nesting and one
node of each colour. It exists because section 12 builds the engine before the
TUI, which would otherwise leave the TUI with nothing to draw. When the
filesystem loader arrives, `stub_tree` and its module go — it is a stopgap and
should never be mistaken for the real thing.

## The dependency edge runs one way

`warlock-engine` never depends on the TUI, and never depends on any terminal
crate — no `warlock-tui`, no Ratatui, no Crossterm, no terminal or rendering
library of any kind. The dependency edge runs **TUI → engine, and never back**.

This is an architectural boundary, not a convention. The engine must stay
usable without a terminal attached, so that another front end (or a test
harness) can drive it directly. Any change that would make the engine reach
back toward the TUI is a change to the architecture and should be treated as
one.
