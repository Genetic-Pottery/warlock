# warlock-tui

The terminal front end of warlock. It is the crate that ships the `warlock`
executable: the binary target is named `warlock`, so `cargo run` from the repo
root builds and runs `warlock`, not `warlock-tui`.

Its job is presentation and input — drawing the current state of the work tree
and turning keystrokes into requests. What runs today is the shell around the
tree the engine loads from the **real working directory** the binary was
launched in — every directory that walk reached, documented or not, and nothing
that git ignores:

- A header naming which tree is on screen: the walk's root, spelled relative to
  the repository root it sits in, or `(repository root)` when they are the same
  directory.
- The tree, drawn as an indented list on the alternate screen: one node per
  line, indented by the depth the engine's depth-first walk reports, coloured
  by state (gray unpacted, yellow stale, green fresh) via `colour_for`, with
  exactly one row highlighted as the selection. A directory carries `+` when
  its children are hidden and `-` when they are on screen; a row with nothing
  under it carries neither, in the same two columns, so sibling names line up
  whatever marker they do or do not have.
- A window, not the whole list: the tree area draws the slice of rows starting
  at the app's scroll offset and running for as many lines as the area is
  tall, so a tree taller than the terminal scrolls under a header and a footer
  that stay put.
- A footer of three lines: the tally of nodes by state, the keys, and one line
  for whatever the app has to say about the last keystroke — why a pact was
  refused, say. That line is drawn blank rather than dropped when there is
  nothing to say, so the tree above it never shifts by a row.

Nothing here computes anything about the tree: the front end asks the engine
for it (`load_tree`, on the working directory the binary was launched in) and
renders what it gets back. Which directories are modules and what colour each
one is are the engine's answers, arrived at before a frame is drawn.

**A directory is a module when it directly contains a `README.md`.** That is
the whole test — no README is parsed, not its headings and not a word of it. A
directory with no README of its own is an ordinary directory that has no
documentation yet: it is drawn like any other node, and it cannot be pacted,
because a pact is a promise about a document and there is no document to
promise anything about yet. Pressing `p` on one changes no colour and writes
nothing; it puts one line in the footer saying so, rather than refusing
silently. The directory you launched in is drawn the same way, README or not,
and without one it cannot be pacted either.

## The keys

Every binding is in `action_for`, and it is the whole list — there is no mouse,
and only presses count, so a key release or an auto-repeat does nothing:

- **Up/Down**, and **`k`/`j`**, move the selection one row; it clamps at both
  ends rather than wrapping.
- **PgUp/PgDn** move it by a screenful — whatever the frame just drawn had room
  for, which is the same height the window scrolls by.
- **`g`** selects the first row of the tree and **`G`** the last.
- **Space** collapses the selected directory, hiding its descendants, and
  expands it again. On a row with no children it is a complete no-op: nothing
  is hidden, nothing is recorded, and the last keystroke's message is left
  where it is rather than swept away by a key that did nothing.
- **`f`** shows the files inside each directory as well as the directories, and
  hides them again.
- **`o`** narrows the view to the pacted nodes and the ancestors that reach
  them, and widens it back to the whole walk.
- **`p`** toggles the pact on the selected node. It is the one key that writes
  to disk; see below.
- **`q`**, **`Esc`** and **Ctrl-C** exit. Ctrl-C is handled here as a key event,
  not as a signal: raw mode is exactly the mode in which the terminal stops
  turning it into `SIGINT`.

Lower case only for `f`, `o` and `p` — the upper-case letters are different
keystrokes and mean nothing here — while `g` and `G` are told apart by case
alone, so a terminal that reports the shift modifier alongside the letter lands
on the same action as one that does not.

## The pact key

`p` is the one keystroke that writes to disk, and it is a toggle with no
confirmation prompt: the action is cheap and its own undo.

- On a gray (unpacted) module it writes an entry for that module and the row is
  yellow on the very next frame, with no reload and no relaunch.
- On a module that is already pacted it removes the entry again and the row
  goes back to gray.
- On an undocumented directory, or on a file row, it refuses: no state moves,
  no count moves, nothing is written, and the footer's message line says which
  refusal it was — a directory with no README yet, or a file, which is part of
  a module rather than being one.

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

## Files are shown, not opened

`f` puts the files a node lists on screen, each one directly under the
directory holding it and one level deeper, in the order the tree listed them
and before that directory's subdirectories. A file row is drawn and nothing
else:

- **It takes its directory's colour.** The state on a file row is a copy of the
  state of the directory holding it — the colour says which module the file
  belongs to, not something about the file — so pacting a directory recolours
  its file rows in the same keystroke.
- **It has no state of its own.** No README, no children, nothing the engine
  ever decided about it; the engine does not treat a file as a node, and neither
  does anything here.
- **It is counted nowhere.** The footer's tally is the engine's count of nodes,
  so it does not move by one when `f` is pressed either way: a module has the
  same files whether or not they are on screen.
- **It cannot be pacted.** `p` refuses it and says why, because a pact is made
  with the directory holding a file rather than with the file.

And that is the whole of what a file row does: **nothing opens a file.** No key
reads one, no pane shows its contents, and the tree is the only thing on screen.
A viewer pane is deliberately the next thing to build rather than a missing part
of this one — the tree had to be worth moving around in before there was any
point putting a file beside it.

## Where the pact goes: `.warlock/pacts.toml`

There is **one manifest per repository**, at `.warlock/pacts.toml` under the
repository root — the nearest ancestor of the working directory holding a
`.git/` directory. The manifest's place under that root has not moved; only
what marks the root has. A repository with no `.warlock/` yet opens fine, all
unpacted, and the first pact creates the directory. It is **committed to git**: a pact is a fact about the
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

## The view is never written down

`p` is the only key that touches the disk, and everything the other keys change
is **view state owned by `App` and held nowhere else**:

- which directories are collapsed, as a set of node *paths* — never row
  indices, which name a different node the moment the row list is rebuilt;
- whether the pacted-only filter is on;
- whether files are shown;
- where the selection sits, and the scroll offset that keeps it inside the
  window the renderer draws.

None of it reaches disk, and there is nowhere for it to reach. The engine's
`Node` and `Tree` derive `Serialize` and `Deserialize`, and those derives are
the whole vocabulary a node could be written down in — path, README, state,
children, files — with no field for a collapsed set, a filter flag, a selection
or an offset, so no view state can ride out through them. `.warlock/pacts.toml`
is narrower still: it holds pacts, one entry per pacted module, and the [schema
in `warlock-engine`](../warlock-engine/README.md#the-manifest-warlockpactstoml)
has nothing in it about what was on screen when a pact was made.

That is why none of these keys saves anything or re-reads anything. Collapsing,
filtering and showing files re-filter the walk the app already holds, put the
selection and the offset back in range, and let the next frame draw the shorter
or longer list. It also means the view does not survive the process: every run
starts fully expanded, unfiltered, with files hidden and the first row selected.
Carrying a collapsed set across a rebuilt `App` is possible in memory —
`App::with_collapsed` takes one, which is how a collapsed tree would survive a
reload — but nothing writes it out, and nothing in the binary reloads today.

## Green is not reachable through the product

A node you pact here goes yellow and stays yellow, however long you look at it.
That is not a bug and not a missing refresh button: freshness is granted by an
AI pass that reads the diff and confirms or rewrites the document, and **that
pass does not exist yet**. Nothing in this workspace runs `claude` or prompts a
model, and nothing outside the tests writes a `granted_hash`.

So a pacted module with no granted hash is stale by definition — unjudged *is*
stale, and there is no fourth "unknown" colour — which is also why toggling a
pact hashes nothing. The only way to see green today is for a human to
hand-write a matching `granted_hash` into the manifest, which is exactly how
the tests that cover the fresh case reach it.

## Pure core, thin shell

Everything that can be tested without a terminal lives in the library
(`src/lib.rs` and the modules behind it): `App`, which owns the flattened rows,
the view state above — the collapsed set, the two filter flags, the selection
and the scroll offset — the tally and the one line the footer has to say, and
whose `toggle_pact` flips the selected row and hands back what a manifest entry
needs (its path, its README, and whether it is now pacted) without touching a
file; `colour_for`, a function from state to colour and nothing else; `draw`,
which turns an app and a frame into a picture; and `tree_height`, which answers
from the same layout `draw` uses how many rows of tree a terminal has room for.
Those are covered by ordinary unit tests, the draw path against Ratatui's
in-memory `TestBackend`, so `cargo test --workspace` needs no terminal attached.

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
