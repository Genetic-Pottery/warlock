# Red brief: Warlock keeps up

A tree that notices. Warlock reads the filesystem once, at startup, and never
again — so a file created, edited or deleted while it is open is invisible until
it is relaunched, and a directory that has gone stale carries on claiming it is
fresh. This project makes the tree follow the disk, makes a running pact obvious
on the row it is working on, makes the three colours readable, answers the
mouse, and puts warlock's own mark in the empty panel.

Read `docs/warlock-design-doc.md` before drafting, section 5 in particular: the
three colours, the rule that files take their directory's colour and have no
state of their own, and the promise that warlock runs from any subdirectory.
Nothing here changes any of it.

This project depends on brief 05. Its re-seat slice builds the ability to put a
view back — selection, collapse, filter and scroll — on a freshly loaded tree,
and was deliberately specified as "re-seat this view on a new tree" rather than
"reload after a pact" so that this project's watcher would have something to
call. Slice 1 below is that caller. It cannot land before that slice does.

**The refresh pass is not in this project.** A directory edited after its pact
still has no route back to green except being pacted again from scratch. That
gap has been open since brief 04 and is the obvious candidate for brief 07;
folding it in here would mean designing the reconciliation rules — what the
model is shown, what "still holds" means, whether a confirming pass rewrites the
document or only re-grants it — inside a project about the view keeping up with
the disk. Two different problems, and the second one is larger.

## Outcome

Leave warlock open and work in another window. Save a file and the directory
holding it goes yellow, by itself, within a moment. Create a directory or drop a
file into one and the row appears. Pull a branch and the tree redraws to match
it. Nothing needs a keystroke and nothing needs a relaunch.

Press the pact key and the directory being worked on pulses between yellow and
green while the model runs on it, so the row itself says what the footer and the
panel say. Press the key again mid-run and warlock says a pact is already
running rather than doing nothing at all.

And the yellow that carries all of this is a colour you can actually read.

Reach for the mouse and it answers. The wheel scrolls whichever pane the pointer
is over. A click selects the row under it and hands that pane the keys, and
clicking the directory already selected opens or closes it. Nothing that was a
key stops being one.

Launch warlock in a repository nothing has been pacted in yet and the panel is
not a blank rectangle: warlock's mark sits in it, quiet, and goes the moment
there is an account to put there.

## Success criteria

- With warlock open, saving a file in a pacted directory turns that directory
  and its pacted ancestors yellow without any keystroke, within about a second
  of the save.
- Creating a file or directory adds the row; deleting one removes it. A `git
  checkout` or `git pull` that changes many files redraws the tree once, not
  once per file.
- Running `cargo build` — or anything else that writes thousands of files into
  an ignored directory — does not redraw the tree, does not re-hash, and does
  not make warlock stutter.
- The selection, the collapsed directories, the pacted-only filter and the
  scroll position all survive every one of those redraws.
- On a machine where the watcher cannot start — an exhausted inotify limit, a
  platform without one — warlock launches, draws, pacts and behaves exactly as
  it does today, and says once on the footer that live updates are off.
- While a pact runs, the directory currently being pacted alternates between
  yellow and green on the row, and stops when the run does.
- A second pact key press during a run puts a line on the footer saying a pact
  is already running, and starts nothing.
- The three colours are distinguishable from each other and readable on both
  dark and light terminal backgrounds, and yellow in particular is legible
  rather than washed out.
- The wheel scrolls the pane the pointer is over, whichever pane has focus. A
  click selects the row under it and focuses the pane it landed in; clicking the
  already-selected directory opens or closes it.
- Every action the mouse reaches is still reachable by key, and warlock is still
  fully usable on a terminal that reports no mouse at all.
- Mouse reporting is off in the shell warlock came back to, however it left —
  quit, `Ctrl-C`, error or panic — and a key turns capture off mid-session so
  the terminal's own text selection can be used without leaving.
- Before the first pact the panel shows the mark, centred and quiet; a panel too
  narrow for it shows the empty border instead of a fragment; the first line of
  account replaces it and it does not come back.
- `warlock-engine` still opens no sockets and spawns no subprocesses, and its
  tests still run with no terminal, no network and no `claude` binary present.
- The gate — `cargo fmt --all --check`, `cargo clippy --workspace --all-targets
  -- -D warnings`, `cargo test --workspace` — stays clean.

## Constraints

- Everything from briefs 01 through 05 still holds: stable Rust 2024, system
  toolchain and no `rust-toolchain.toml`, Apache-2.0, no `unsafe`, no HTTP
  client, no Anthropic SDK, no mocking framework, no async runtime. The
  dependency edge still runs TUI to engine and never back.
- **The engine does not change.** Watching is a fact about a front end that
  stays open; the engine's job is still to answer what a tree is when it is
  asked. A slice editing `warlock-engine` has put something on the wrong side of
  the seam.
- **One new dependency is sanctioned, narrowly**: a filesystem watcher for
  `warlock-tui`, with a line in the workspace table saying what it is for. The
  engine's dependency list does not move. No debouncer crate — see slice 1.
- **A watcher is never fatal.** Not at startup, not while running. This extends
  the rule brief 05 set for a failed view refresh: once warlock is up, nothing
  about the view failing to keep up may take the program down.
- **`NodeState` still has exactly three variants.** A row being pacted right now
  is a fact about a run in flight, held in the TUI beside the other view state,
  and the flash renders it. It is not a fourth state and it is not stored.
- **Files keep taking the colour of the directory holding them**, exactly as
  today. Slice 3 changes what the three colours *are*, not what carries them.
- The walk is the definition of the tree. Anything deciding which paths matter
  goes through what a load produced, not through a second implementation of
  gitignore semantics.
- Movement, collapse and filter keys keep working during a pact, and the
  selection never follows the run. True since WAR-21.05, not revisited here.
- **The mouse is an addition and never a requirement.** Everything it reaches
  stays reachable by key. A terminal that reports no mouse, or a user who turns
  capture off, loses nothing but the pointer.
- **Mouse capture is undone by the path raw mode is undone by** — the terminal
  guard and its restore function, so the panic hook covers it as well. Reporting
  left on hands back a shell that prints escape sequences every time the pointer
  crosses it, which outlives whatever went wrong.
- **The mark is not a message.** No version, no tagline, no key hints, no
  welcome. It is what an empty pane looks like, and it yields to the account.

## Out of scope

**The refresh pass**, as above. Brief 07 at the earliest.

**Hover highlighting**, though the rest of the mouse is in. crossterm's capture
turns motion reporting on whether it is wanted or not, so a highlight that
follows the pointer costs a redraw on every mouse move in order to say what the
selection already says. Named here so it is a decision rather than an omission.

**Reading a file's contents anywhere in the UI**, `WARLOCK.md` included. Fifth
brief running. The panel brief 05 built shows the model's work and stays that
way. The mark in slice 5 is not an exception to this: it is what an empty pane
looks like, not something read off the disk.

**Any watcher-driven action other than reloading the tree.** No auto-pact on
change, no auto-refresh, no background model calls. The watcher's entire job is
to say "something moved" so the view can catch up. A tool that starts spending
money because a file was saved is a different and much worse product.

**A daemon, a background service, or any state that outlives the process.**

Also out, as before: blessing and the gate, pact expansion proposals and
footprint prediction, Linear and Red/Forman integration, the journal at
`~/.warlock/`, the pre-push guard, the CI validator, non-interactive entry
points, the paid crate, and publishing anything.

## Scope

### 1. Watch the filesystem, and let the walk decide what matters

`load_tree` runs once, in `load_app`. Brief 05 added a second call, when a pact
ends. This slice adds the third and last reason to reload: something on disk
moved.

Take `notify` and no debouncer crate. The debouncer crates exist to preserve
event kinds and pair up renames, and none of that is wanted here — warlock does
not care *what* happened, only *that* something did, because the answer is always
the same re-walk. The event loop already wakes every 100 ms and brief 05 already
put the reload on that thread, so the debounce is a timestamp and a comparison
in a loop that exists, which is a smaller thing to own than another dependency.

**Filtering is the decision that makes or breaks this slice.** `notify` reports
everything under the watched root, `target/` and `.git/` included, so an
unfiltered watcher turns a `cargo build` into thousands of events and a
continuous re-walk. Do not tell the watcher what to skip, and do not
re-implement gitignore matching. Instead: **an event counts only if its immediate
parent is a directory the last load produced.** `target/debug/` is not in that
set, so nothing beneath it counts, and `.git/` is not either. A brand-new
directory's parent *is* in the set, so it triggers a re-walk, and the walk
itself — which already knows every rule git knows — decides whether the newcomer
belongs. The last walk is the filter, and there stays exactly one implementation
of what a tree contains.

Three timings, because there are three different problems:

- **A quiet period of around 250 ms** before reloading, so the write, rename and
  chmod burst of one editor save produces one redraw.
- **A ceiling of a couple of seconds** between reloads while events keep
  arriving, so a `git checkout` or a build — which emits events continuously for
  many seconds — neither starves the tree until it finishes nor triggers a
  reload every quarter second.
- **Coalescing**: an event arriving while a reload is in progress sets a flag and
  causes exactly one more reload afterwards. Never a queue.

Pick the numbers and say in the diff what each one is protecting against. They
matter more than they look, because a reload re-hashes, and hashing reads the
bytes of every file in every pacted subtree.

Watch the tree's root — the working directory `load_tree` was given — not the
repository root. That is what is on screen, and watching higher only produces
events the filter discards. Also watch `.warlock/pacts.toml`: one extra path on
the same trigger, and it means a `git pull` bringing new grants, or a second
warlock running in the same repository, repaints the tree instead of leaving a
stale one.

**Suppressed while a pact is in flight, but remembered.** A pact writes
documents and documents are events. Ignore the trigger while a run is going, set
the flag, and let the run's own end-of-run reload clear it rather than reloading
twice.

**A watcher that cannot start is not an error.** An exhausted inotify limit is an
ordinary thing on a large repository, not an exceptional one. Warlock runs
without live updates, says so once on the footer, and everything else behaves as
it does today.

The reload itself is brief 05's re-seat, called from a new place. If this slice
finds itself writing view-restoration code, that code already exists.

**If a full re-walk ever proves too slow on a large repository**, the answer is
to narrow the *hash* — re-hash only the subtrees containing a changed path — and
not to narrow the walk. Recorded here so it is a known option rather than a
rediscovery, but not taken now: correctness first, and the walk is what
correctness is defined by.

### 2. Flash the row being pacted, and answer a second pact key

depends_on: []

Pressing the pact key paints the whole subtree yellow immediately and it stays
that way for the entire run, so the tree says nothing about which directory is
being worked on right now. The footer knows and brief 05's panel knows; the tree
does not.

The row for the directory currently being pacted alternates between yellow and
green while the pass on it runs. What it means, in the words that produced it:
*"I am trying to become green, please hold on."* It uses the two colours already
understood rather than inventing a third thing to learn, and it puts the walk on
the tree — the row pulses, the pass finishes, the next row starts.

Everything needed is already there. `App` holds the in-flight path, the event
loop wakes every 100 ms, and ratatui redraws only what changed, so a phase of
around half a second costs one row's worth of cells. Only the row being pacted
pulses: the rest of the subtree stays the yellow the keypress painted, or the
flash becomes a strobe.

`NodeState` gains nothing. This is a rendering decision made from view state the
front end already holds.

In the same slice, close the last silent path: **a second pact key press during a
run says so.** `pact_press` already returns `None` while a run is in flight, so
double-pacting has been impossible since WAR-21.05 — but it refuses without a
word, which is the same "nothing happened, is it broken?" failure this sequence
of projects has been about throughout. One line on the footer.

### 3. Make the three colours readable

depends_on: []

Yellow is the state the product asks people to act on, and it is the hardest of
the three to see. Slice 2 raises the bar further by alternating it with green on
the same row: the two have to be distinguishable at a glance, not merely
different in name.

`colour_for` in `crates/warlock-tui/src/colour.rs` is the only place any of the
three colours is named — one total `match`, deliberately — so this is one
function and the three tests that pin it. Pick better values there. Yellow is
the one that needs it; check the other two against the new yellow rather than
assuming they still separate.

The trade-off to decide and write down: a named ANSI colour respects whatever
palette the user's terminal is themed with and gives warlock no control over
what it actually looks like, while a specific value gives control and ignores
the theme. Somewhere in between — a specific colour from the 256-colour palette
rather than one of the eight names — is likely the right answer for yellow, but
the slice owns that call. Check it against a dark background and a light one
before settling; a colour that reads well on one and vanishes on the other is
not fixed.

Also in this slice: **indent guides**. A real repository rendered whole is
hundreds of rows, and nesting is currently carried by leading spaces alone.
Box-drawing characters down the tree make depth readable at a glance.

Keep them quiet. Guides as bright as the rows they sit beside compete with the
state colours for attention; drawn dim, they disappear until looked at, which is
what a guide is for.

### 4. Answer the mouse

depends_on: []

Nothing on this screen answers a pointer. The event loop matches `Event::Key`
and drops everything else, so on a terminal already reporting them, mouse events
are read and thrown away.

This is here rather than in brief 05 because hit-testing wants to be written
once, against a layout that has stopped moving, and now it has. `areas` in
`crates/warlock-tui/src/ui.rs` already cuts the frame into the panel, the tree
column and the footer, and `pane_inner`, `tree_rows_area`, `tree_height` and
`panel_height` already say how many rows each of them holds. Turning a screen
coordinate into "row twelve of the tree" belongs beside them, in the one module
that knows where anything is. If this slice finds itself working out a layout,
that layout already exists.

Crossterm comes through ratatui's re-export, as everything else in this crate
does, so `EnableMouseCapture` costs no new dependency.

Three behaviours and no fourth:

- **The wheel scrolls the pane under the pointer**, not the focused pane.
  Pointing at something and turning the wheel is already a complete instruction;
  making it also require a Tab first is a correctness nobody asked for.
- **A click selects the row under it** and gives that pane the keys — Tab's job,
  done by pointing.
- **Clicking the directory that is already selected opens or closes it**, which
  is what space does. Select and expand on one button, and no double-click,
  whose timing warlock would then have to own.

Hover highlighting is out, for the reason given above.

**Mouse capture takes the terminal's own text selection away.** That is the real
cost of this slice, and it is why the slice owes a way out: a key — `m`, on the
footer with the others — turns reporting off and back on for the session. Many
terminals also bypass capture while Shift is held, but that is the terminal's
behaviour rather than warlock's and it is not true everywhere, so it is not the
answer here.

And it has to be turned off on the way out. `EnableMouseCapture` goes in
`TerminalGuard::enter` beside `EnterAlternateScreen`; `DisableMouseCapture` goes
in `restore_terminal` beside `LeaveAlternateScreen`. Then quitting, failing and
panicking all undo it by the path that already undoes raw mode, which is the
whole reason that path is shaped the way it is.

### 5. Put warlock's mark in the empty panel

depends_on: []

Before the first pact the panel draws its border and nothing inside it, and
`draw_panel` says why in its own words: a screen that said something before
anything had happened would be saying it about nothing. That is right about a
*message*. It says nothing about the program's name.

Draw the warlock mark in that space, in characters. `assets/warlock-logo.png` is
what it comes from — it is what the README opens with, and the two should be
recognisably the same thing. Centred in the panel, drawn quiet, and gone the
instant there is a first line of account to put there.

It has to survive a small terminal. The panel takes the majority of the width,
but the floor is real — at 40 columns it is 20 wide — so the art needs a size
below which it is simply not drawn. Half a logo is worse than an empty
rectangle, and an empty rectangle is exactly what this replaces, so falling back
to it costs nothing.

Keep it to plain ASCII, or as near as the shape allows. Slice 3's box-drawing
guides are the ceiling on what warlock assumes a terminal can render, and this
is the pane with the least reason to test that ceiling.

The earlier call was to live with the empty space before committing to art. It
has now been lived with. If what gets drawn reads as clutter rather than as
identity, the answer is to cut it rather than to shrink it.
