# warlock-tui

The terminal front end of warlock. It is the crate that ships the `warlock`
executable: the binary target is named `warlock`, so `cargo run` from the repo
root builds and runs `warlock`, not `warlock-tui`.

Its job is presentation and input — drawing the current state of the work tree
and turning keystrokes into requests. What runs today is the shell around the
tree the engine loads from the **real working directory** the binary was
launched in:

- A header naming which tree is on screen: the walk's root, spelled relative to
  the repository root it sits in, or `(repository root)` when they are the same
  directory.
- The tree, drawn as an indented list on the alternate screen: one node per
  line, indented by the depth the engine's depth-first walk reports, coloured
  by state (gray unpacted, yellow stale, green fresh) via `colour_for`, with
  exactly one row highlighted as the selection.
- A footer with the counts of nodes by state and the active keybindings.
- Up/Down and `k`/`j` to move the selection one row at a time; it clamps at
  both ends rather than wrapping. `p` toggles the pact on the selected node.
  `q`, `Esc` and Ctrl-C exit.

Nothing here computes anything about the tree: the front end asks the engine
for it (`load_tree`, on the working directory the binary was launched in) and
renders what it gets back. Which directories are modules and what colour each
one is are the engine's answers, arrived at before a frame is drawn.

**A directory is a module when it directly contains a `README.md`.** That is
the whole test — no README is parsed, not its headings and not a word of it. A
directory with no README of its own is an ordinary directory that has no
documentation yet: it is drawn like any other node, and it cannot be pacted,
because a pact is a promise about a document and there is no document to
promise anything about yet. Pressing `p` on one does nothing at all — no colour
change, no write. The directory you launched in is drawn the same way, README
or not, and without one it cannot be pacted either.

## The pact key

`p` is the one keystroke that writes to disk, and it is a toggle with no
confirmation prompt: the action is cheap and its own undo.

- On a gray (unpacted) module it writes an entry for that module and the row is
  yellow on the very next frame, with no reload and no relaunch.
- On a module that is already pacted it removes the entry again and the row
  goes back to gray.

Exactly one node — the selected one — changes per press. There is no bulk or
recursive pacting, no undo stack and no dialog.

The row's colour and the tally in the footer move together with the file: `App`
flips the selected row's state and shifts one node between the count fields in
the same step, and the manifest is saved *before* the next frame is drawn, so
the screen never claims something that was not written. Saving goes through the
engine's `Manifest::save`, which writes a temporary file in `.warlock/` and
renames it over `pacts.toml`, so a reader sees the whole old manifest or the
whole new one and never half of either. A save that fails — a read-only
`.warlock/`, a full disk — is an ordinary failure and not a panic: the terminal
is restored first, one `warlock: …` line goes to stderr, and the exit status
says it did not work.

## Where the pact goes: `.warlock/pacts.toml`

There is **one manifest per repository**, at `.warlock/pacts.toml` under the
repository root — the nearest ancestor of the working directory holding a
`.warlock/` directory. It is **committed to git**: a pact is a fact about the
repository, not about one developer's checkout, so `.warlock` does not belong
in `.gitignore`. Launching from a subdirectory therefore scopes the *tree* to
that subdirectory while the pacts still come from, and go to, the one manifest
above it.

It holds one entry per pacted module. An entry names the pacted **directory**,
the **README** that documents it (held separately, because the file name is not
Warlock's to assume), and — only once freshness has been granted — the
**granted hash** it was granted against, and when. Pressing `p` writes the
first two and never the rest, so every entry this key creates carries no
granted hash. **Every path in it is relative to the repository
root**, with forward slashes, which is what lets two clones of the same commit
at different absolute paths hold byte-identical manifests. The file is TOML and
is meant to be readable, diffable and hand-editable; see
[`warlock-engine`](../warlock-engine/README.md#the-manifest-warlockpactstoml)
for the schema.

## Green is not reachable through the product

A node you pact here goes yellow and stays yellow, however long you look at it.
That is not a bug and not a missing refresh button: freshness is granted by an
AI pass that reads the diff and confirms or rewrites the document, and **that
pass does not exist yet**. Nothing in this workspace runs `claude`, prompts a
model, or writes a `granted_hash`.

So a pacted module with no granted hash is stale by definition — unjudged *is*
stale, and there is no fourth "unknown" colour — which is also why toggling a
pact hashes nothing. The only way to see green today is for a human to
hand-write a matching `granted_hash` into the manifest, which is exactly how
the tests that cover the fresh case reach it.

## Pure core, thin shell

Everything that can be tested without a terminal lives in the library
(`src/lib.rs`): `App`, which owns the flattened rows, the selection index and
the tally, and whose `toggle_pact` flips the selected row and hands back what a
manifest entry needs (its path, its README, and whether it is now pacted)
without touching a file; `colour_for`, a function from state to colour and
nothing else; and `draw`, which turns an app and a frame into a picture. Those
are covered by ordinary unit tests, the draw path against Ratatui's in-memory
`TestBackend`, so `cargo test --workspace` needs no terminal attached.

The binary (`src/main.rs`) is the impure remainder: raw mode, the alternate
screen, the blocking event loop, the mapping from key event to action, and the
manifest that a toggle edits and saves. Its
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
