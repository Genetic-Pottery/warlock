# warlock-tui

The terminal front end of warlock. It is the crate that ships the `warlock`
executable: the binary target is named `warlock`, so `cargo run` from the repo
root builds and runs `warlock`, not `warlock-tui`.

Its job is presentation and input — drawing the current state of the work tree
and turning keystrokes into requests. What runs today is the shell around the
tree the engine loads from the **real working directory** the binary was
launched in — every directory that walk reached, documented or not, and nothing
that git ignores:

- A header naming which *part* of the repository is on screen: the walk's root,
  spelled relative to the repository root it sits in. Blank when they are the
  same directory, since the whole of a repository is not a part of it and the
  root row below already names the directory.
- The tree, drawn as an indented list on the alternate screen: one node per
  line, indented by the depth the engine's depth-first walk reports, coloured
  by state (gray unpacted, yellow stale, green fresh) via `colour_for`, with
  exactly one row highlighted as the selection, and led in by guides: a `│`
  down every level whose branch carries on below, and `├` or `└` on the row
  itself according to whether its directory holds anything after it. A
  directory carries `+` when what it holds is hidden and `-` when it is on
  screen; a row with nothing under it *in the view as it stands* carries
  neither, in the same two columns, so sibling names line up whatever marker
  they do or do not have.
- A window, not the whole list: the tree area draws the slice of rows starting
  at the app's scroll offset and running for as many lines as the area is
  tall, so a tree taller than the terminal scrolls under a header and a footer
  that stay put.
- A footer of three lines: the tally of nodes by state, the keys, and one line
  for whatever the app has to say about the last keystroke — why a pact was
  refused, say. That line is drawn blank rather than dropped when there is
  nothing to say, so the tree above it never shifts by a row. While a pact is
  running it says what the run is doing instead, and the keys line names the
  keys that mean something while it runs; the three lines stay three lines
  either way.

Nothing here computes anything about the tree: the front end asks the engine
for it (`load_tree`, on the working directory the binary was launched in) and
renders what it gets back. Which directories are modules and what colour each
one is are the engine's answers, arrived at before a frame is drawn.

**A directory is a module when it directly contains a `WARLOCK.md`.** That is
the whole test — no document is parsed, not its headings and not a word of it.
A directory with no document of its own is an ordinary directory that has no
documentation yet: it is drawn like any other node, and pacting it is exactly
how it stops being one, because writing that document is what a pact *does*.
The directory you launched in is drawn the same way, document or not, and is
pacted the same way too. A `README.md` is nobody's document: it is drawn as an
ordinary file like any other, it makes no module, and nothing here ever writes
to one.

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
- **`p`** pacts the selected directory and everything below it, or takes the
  lot back out again. It is the one key that writes anything; see below.
- **`q`** and **Ctrl-C** exit, whatever else is happening. Ctrl-C is handled
  here as a key event, not as a signal: raw mode is exactly the mode in which
  the terminal stops turning it into `SIGINT`.
- **`Esc`** reads two ways, and it is the only key that does. With nothing
  running it exits, as it always did. With a pact running it cancels *that* and
  stays: the run is what is in front of the reader, stopping it is the only
  thing a key meaning "not this" can want, and quitting outright on the key
  nearest to hand would be the one keystroke that throws away minutes of
  somebody else's model time by mistake.

Lower case only for `f`, `o` and `p` — the upper-case letters are different
keystrokes and mean nothing here — while `g` and `G` are told apart by case
alone, so a terminal that reports the shift modifier alongside the letter lands
on the same action as one that does not.

## The pact key

`p` is the one keystroke that writes anything, and what it writes is a subtree:
the selected directory and every directory below it, together, in one run.
There is no dialog and no confirmation prompt.

- On a gray (unpacted) directory it starts a pact over the whole subtree, and
  every row in it is yellow on the very next frame. The engine works the
  directories **children before parents** — a parent is passed the documents
  its children have just written — and writes a `WARLOCK.md` for each one.
  Every document is written first and only then is anything hashed and
  granted, so no directory is granted a hash that the next write invalidates.
- **The tree stays usable while the run goes on.** A pact is minutes of model
  passes on a worker thread, so the loop keeps drawing: moving, collapsing,
  filtering and showing files all work exactly as they did. The footer's
  message line says which directory is being worked and where it sits in the
  run — `pacting crates/engine (3/12)`, position and total, so a screen that
  has not changed in two minutes reads as work rather than as a hang.
- **No second pact starts while one is running.** Pressing `p` again changes
  nothing and says nothing: two runs writing the same documents and the same
  manifest would be a race, and there is nothing about it worth putting on a
  line the run in flight is already using.
- **Esc cancels the run**, and only while there is one to cancel; with nothing
  running Esc still quits, and `q` and Ctrl-C quit either way. A cancel stops
  the descent at the next directory *and* kills the `claude` in flight, so it
  lands in milliseconds rather than at the end of a pass. What was written
  stays written: the directories the run reached keep their documents and earn
  their entries, the ones it never reached are simply undocumented, and the
  subtree stays yellow.
- On a subtree that is already pacted it removes the entries for the whole of
  it and the rows go back to gray. **The documents are left on disk** —
  un-pacting is manifest editing and nothing else, no walk and no pass — and
  the footer says so, because a whole subtree turning the colour of a directory
  Warlock knows nothing about reads like the writing was thrown away.
- On a file row it refuses, and that is the only refusal left: no state moves,
  no count moves, nothing is written, and the footer's message line says that a
  pact is made with the directory holding a file rather than with the file. A
  directory with no `WARLOCK.md` yet is not refused — writing that document is
  the point of pressing the key.

Yellow is all the keystroke itself can claim: a pact with no grant behind it has
never been judged, and unjudged *is* stale — there is no fourth "unknown"
colour. The subtree turns green when the run comes back with every directory in
it documented, hashed and granted. A run with a failure anywhere in it leaves
the whole subtree yellow, branches that did earn their grants included, and
puts one line on the footer saying what went wrong and how much else did.

The tally in the footer moves with the rows: `App` paints the subtree and shifts
those nodes between the count fields in the same step, so the numbers keep
describing what is on screen without anything recounting it. The manifest is
saved **once**, at the end of the run, through the engine's `Manifest::save`,
which writes a temporary file in `.warlock/` and renames it over `pacts.toml`,
so a reader sees the whole old manifest or the whole new one and never half of
either; a save per directory would record a pact that was still running. The
documents are written the same way, to a hidden temporary and renamed over the
`WARLOCK.md`, so a cancel leaves no half-written file behind. A run that
recorded nothing — a subtree that could not be walked, a manifest that would not
save on a read-only `.warlock/` or a full disk — puts the rows back exactly as
they were before the key was pressed and puts the reason on the footer's message
line, rather than taking the screen down with it.

## Files are shown, not opened

`f` puts the files a node lists on screen, each one directly under the
directory holding it and one level deeper, in the order the tree listed them
and before that directory's subdirectories. A file row is drawn and nothing
else:

- **It takes its directory's colour.** The state on a file row is a copy of the
  state of the directory holding it — the colour says which module the file
  belongs to, not something about the file — so pacting a directory recolours
  its file rows in the same keystroke.
- **It has no state of its own.** No document, no children, nothing the engine
  ever decided about it; the engine does not treat a file as a node, and neither
  does anything here.
- **It is counted nowhere.** The footer's tally is the engine's count of nodes,
  so it does not move by one when `f` is pressed either way: a module has the
  same files whether or not they are on screen.
- **It cannot be pacted.** `p` refuses it and says why, because a pact is made
  with the directory holding a file rather than with the file.

And that is the whole of what a file row does: **nothing opens a file.** No key
reads one, no pane shows its contents, and the tree is the only thing on screen.
A viewer pane is deliberately unbuilt rather than a missing part of this one —
the tree had to be worth moving around in before there was any point putting a
file beside it.

## Where the pact goes: `.warlock/pacts.toml`

There is **one manifest per repository**, at `.warlock/pacts.toml` under the
repository root — the nearest ancestor of the working directory holding a
`.git/` directory. The manifest's place under that root has not moved; only
what marks the root has. A repository with no `.warlock/` yet opens fine, all
unpacted, and the first pact creates the directory. It is **committed to
git**: a pact is a fact about the repository, not about one developer's
checkout, so `.warlock` does not belong in `.gitignore`. Launching from a
subdirectory therefore scopes the *tree* to that subdirectory while the pacts
still come from, and go to, the one manifest above it.

It holds one entry per pacted module. An entry names the pacted **directory**,
the **document** that describes it (held separately, because the file name is
not Warlock's to assume), and — only once freshness has been granted — the
**granted hash** it was granted against, and when. A pact that ran to the end of
its subtree writes all of it, one entry per directory, each granted against the
hash computed for it once every document in the run was on disk. An entry with
no granted hash is what partial completion looks like: a directory whose own
document failed gets no entry at all, and every ancestor of it inside the pact
gets an entry with no grant, which is pacted and stale — yellow — and needs no
state of its own to say so. **Every path in it is relative to the repository
root**, with forward slashes, which is what lets two clones of the same commit
at different absolute paths hold byte-identical manifests. The file is TOML and
is meant to be readable, diffable and hand-editable; see
[`warlock-engine`](../warlock-engine/README.md#the-manifest-warlockpactstoml)
for the schema.

## The view is never written down

`p` is the only key that writes anything — the `WARLOCK.md` files a pact
produces and the manifest that records them — and everything the other keys
change is **view state owned by `App` and held nowhere else**:

- which directories are collapsed, as a set of node *paths* — never row
  indices, which name a different node the moment the row list is rebuilt;
- whether the pacted-only filter is on;
- whether files are shown;
- where the selection sits, and the scroll offset that keeps it inside the
  window the renderer draws.

None of it reaches disk, and there is nowhere for it to reach. The engine's
`Node` and `Tree` derive `Serialize` and `Deserialize`, and those derives are
the whole vocabulary a node could be written down in — path, document, state,
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

## Green is reachable through the product

A subtree you pact here goes yellow while the run works and green when the run
comes back with every directory in it written, hashed and granted. That is the
whole of how green is reached, and it is reached by pressing one key: the pass
that writes each document is `claude`, spawned as a child process, so `claude`
has to be on `PATH` for a pact to come to anything. Nothing has to be
hand-written into the manifest any more, and the tests that cover the fresh case
are no longer the only things that reach it.

What is not reachable is green a *second* time. A node whose files have changed
since the grant goes yellow, and nothing here judges it again: refreshing a pact
— reading the diff and confirming or rewriting the document rather than writing
it from nothing — is the next project, and until it lands the only way back to
green is to un-pact the subtree and pact it over from scratch.

## Pure core, thin shell

Everything that can be tested without a terminal lives in the library
(`src/lib.rs` and the modules behind it): `App`, which owns the flattened rows,
the view state above — the collapsed set, the two filter flags, the selection
and the scroll offset — the tally, the one line the footer has to say and the
pact in flight it gives way to, and whose `toggle_pact` paints the selected
subtree and hands back what carrying the toggle out needs (which directory, and
which way it went) without touching a file; `colour_for`, a function from state
to colour and nothing else; `draw`, which turns an app and a frame into a
picture; and `tree_height`, which answers from the same layout `draw` uses how
many rows of tree a terminal has room for.
Those are covered by ordinary unit tests, the draw path against Ratatui's
in-memory `TestBackend`, so `cargo test --workspace` needs no terminal attached.

The library has one member that is not data and functions over data, and it is
`ClaudeAgent` (`src/claude.rs`): the transport half of the engine's agent seam,
which spawns `claude` as a child process, hands it on its stdin the prompt the
engine composed, reads what comes back on stdout and translates however that
went into the engine's words. It decides nothing about a prompt and never adds
to one — the moment this crate starts composing prompts, domain logic has
crossed to the wrong side of the seam — and it is what a cancel reaches through
to kill a pass in flight.

The binary (`src/main.rs`) is the impure remainder: raw mode, the alternate
screen, the event loop, the mapping from key event to action, the worker thread
a subtree pact runs on, and the manifest that a toggle edits and saves. The loop
waits on a keystroke rather than blocking for one, so a pact's progress reaches
the screen without anybody pressing anything. Its one job beyond wiring is that
the terminal is restored on *every* way out — a normal quit, an error returned
to `main`, or a panic — because raw mode left switched on hands the user back a
shell that no longer echoes what they type.
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
