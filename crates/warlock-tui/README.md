# warlock-tui

The terminal front end of warlock. It is the crate that ships the `warlock`
executable: the binary target is named `warlock`, so `cargo run` from the repo
root builds and runs `warlock`, not `warlock-tui`.

Its job is presentation and input — drawing the current state of the work tree
and turning keystrokes into requests. What runs today is the shell around the
engine's stub tree:

- The tree, drawn as an indented list on the alternate screen: one node per
  line, indented by the depth the engine's depth-first walk reports, coloured
  by state (gray unpacted, yellow stale, green fresh) via `colour_for`, with
  exactly one row highlighted as the selection.
- A footer with the counts of nodes by state and the active keybindings.
- Up/Down and `k`/`j` to move the selection one row at a time; it clamps at
  both ends rather than wrapping. `q`, `Esc` and Ctrl-C exit.

Nothing here computes anything about the tree: the front end asks the engine
for it (today, `stub_tree`) and renders what it gets back.

## Pure core, thin shell

Everything that can be tested without a terminal lives in the library
(`src/lib.rs`): `App`, which owns the flattened rows and the selection index;
`colour_for`, a function from state to colour and nothing else; and `draw`,
which turns an app and a frame into a picture. Those are covered by ordinary
unit tests, the draw path against Ratatui's in-memory `TestBackend`, so
`cargo test --workspace` needs no terminal attached.

The binary (`src/main.rs`) is the impure remainder: raw mode, the alternate
screen, the blocking event loop, and the mapping from key event to action. Its
one job beyond wiring is that the terminal is restored on *every* way out — a
normal quit, an error returned to `main`, or a panic — because raw mode left
switched on hands the user back a shell that no longer echoes what they type.
An RAII guard covers the first two and a chained panic hook, installed before
raw mode is entered, covers the third.

## The dependency edge runs one way

`warlock-tui` is the side of the boundary that is allowed to depend on
[`warlock-engine`](../warlock-engine/README.md), and does, as a `path`
dependency. The domain logic lives in the engine; this crate consumes it and
never the other way around. The edge runs **TUI → engine, and never back**.

Crossterm is not named as a dependency here either: Ratatui re-exports the
backend it was built against as `ratatui::crossterm`, and using that re-export
makes a version skew between frontend and backend impossible.
