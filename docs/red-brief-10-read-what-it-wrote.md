# Red brief: Read what it wrote

Warlock writes documents and will not show you one. Every colour in the tree is
a claim about a `WARLOCK.md` — granted after a pass read the directory, or owed
a look because the hash moved — and in nine briefs the document those claims are
about has never been on screen. Press `f` and it does appear, in the listing,
spelled like the code beside it and indistinguishable from it: the one artifact
warlock exists to produce, drawn as though it were another `mod.rs`.

Section 6 makes that a hole rather than a missing convenience. The hash is the
trigger and the human is the judge, and the design doc is emphatic that warlock
never decides a document is *wrong* — "whether a particular change warrants a
documentation update was always a judgement call, and warlock makes the change
visible rather than pretending to make that call for you." But the judgement it
reserves is not one anybody can make from a colour. It needs the document and
the code that drifted from it, together, and warlock currently offers neither.
It reports that something happened down there and then sends you to another
program to find out what.

This project puts the document in the tree and the file in the panel. A
documented directory draws its `WARLOCK.md` beneath it whether or not files are
shown, because that document is not one of the files the module is made of — it
is what the module's colour is *about*, and it has been hidden behind a toggle
that exists for something else. `v` on a file row opens it in the panel, one at
a time, view only. Shift-Tab swaps between the document and the account of the
run, so a reader can look at what an in-flight pact is doing and go straight
back to the line they were on.

Editing is deliberately somebody else's job. `e` hands the file to `$EDITOR` and
gets out of the way, which is section 9's escape hatch spelled as a keystroke:
the hatch exists so the tool never traps you, and using it says plainly that you
have stepped outside the model warlock is built for. Warlock does not become a
text editor to avoid admitting that.

Read `docs/warlock-design-doc.md` before drafting — sections 5, 6 and 9. Section
5's rule that a file "has no state of its own" is not amended here and must not
be: a document row takes the colour of the directory holding it exactly as every
other file row does, and gains nothing for being warlock's. Section 9 is the
whole argument for slice 4 and should be read as the constraint it is, not as
permission.

This brief depends on brief 05 for the panel it borrows and on brief 08 for the
row that lands mid-run. It has no engine work of consequence: one capped read,
and everything else is view state.

## Outcome

Open warlock on a pacted repository. The tree is directories, as it has always
been, and under each documented one a single `WARLOCK.md` — the only file on
screen. Nobody has to be told what that means. A gray directory holding a
document left behind by an un-pact is now visible too, and so is a green one
whose document has not landed yet.

Land on that row and press `v`. The panel stops being the account of the last
pact and becomes the document, from its first line. Tab moves the keys to it and
`j`/`k` read down it, exactly as they read down the account today.

Press `p` on a directory while you are reading. The tree flashes and repaints as
the run works, and the panel does not move — you asked for a document and you
still have it. Press Shift-Tab to look at what the pass is doing, and Shift-Tab
again to land back on the line you were on. The panel changes because you
changed it and for no other reason.

Press `e` on a file you want to fix. Warlock gives the terminal back, `$EDITOR`
opens on that file, and warlock is there again when you leave it. The directory
you edited is yellow, because it is: you changed content a document was granted
against, and the road home is `r` and a pass, not a saved buffer.

Press Shift-Tab in a session where nothing has been viewed and the footer says
so, naming `v`. It does not swap to an empty card and it does not swap to the
mark.

## Success criteria

- A directory with a `WARLOCK.md` draws a row for it with files hidden, one
  level deeper, in that directory's own colour. A directory without one draws
  nothing extra.
- The document row appears under the pacted-only filter wherever its directory
  does, and disappears with the rest of a collapsed directory's rows.
- With files shown the tree is exactly what it is today — every file including
  the document, once, in path order, with no row drawn twice.
- A gray directory holding a `WARLOCK.md` an un-pact left on disk draws that row
  like any other. No new colour, shade or marker distinguishes it.
- `v` on a file row reads that file and puts it in the panel. `v` on a directory
  row refuses and names the document row beneath it, or names `p` where there is
  no document to name.
- A file that is not valid UTF-8, or cannot be read, puts a line in the footer
  and leaves the panel as it was. Neither is fatal and neither leaves the event
  loop.
- A file longer than the cap is shown up to the cap with a last line saying it
  was cut, and the cap is `PER_FILE_BYTE_CAP` — warlock shows a reader exactly
  as much of a file as it would show a model.
- Tab with a document in the panel scrolls the document; the tree's selection
  does not move, and `Focus::Panel` means the panel's window over whichever card
  is showing.
- Shift-Tab swaps the panel between the document and the account, in both
  directions, and changes nothing else — not the focus, not the selection, not
  the scroll offset of the card it left.
- Shift-Tab before any `v` in the session refuses with a footer line naming `v`,
  and the panel stays on the account.
- **A run never changes which card is showing.** Starting, finishing, cancelling
  or failing a pact or refresh leaves the panel exactly where the reader put it,
  with the account filling up behind Shift-Tab.
- The document card holds what was read when `v` was pressed. Nothing rewrites
  it under the reader; a second `v` on the same row reads it again.
- `e` on a file row restores the terminal, runs `$EDITOR` on that file, and
  re-enters raw mode and the alternate screen afterwards — on the editor exiting
  normally, on it failing, and on `$EDITOR` being unset, which refuses with a
  line naming the variable.
- `e` refuses while a pact or refresh is running, and says so.
- Warlock writes no byte of any file a reader viewed. The only writers in the
  workspace are still the pact, the refresh, the manifest, the scope key and
  `warlock init`.
- The keys line still fits its budget with `v` on it, and the way out is still
  the last thing on it and still on screen at eighty columns.
- `warlock-engine` still opens no sockets and spawns no subprocesses, and its
  tests still run with no terminal, no network and no `claude` binary present.
- The gate — `cargo fmt --all --check`, `cargo clippy --workspace
  --all-targets -- -D warnings`, `cargo test --workspace` — stays clean.

## Constraints

- Everything from briefs 01 through 09 still holds: stable Rust 2024, system
  toolchain and no `rust-toolchain.toml`, Apache-2.0, no `unsafe`, no HTTP
  client, no Anthropic SDK, no mocking framework, no async runtime. The
  dependency edge still runs TUI to engine and never back.
- **No new dependency.** No markdown renderer, no syntax highlighter, no editor
  crate, no pager. A document is drawn as the lines it has, in the panel's own
  style, by the code that already draws lines in that panel.
- **View only, and the tool says so.** Warlock never takes a keystroke into a
  file. There is no insert mode, no buffer, no save, and no confirmation dialog
  about unsaved anything, because there is nothing to save. Editing leaves the
  program.
- **Still three colours, and no fourth.** A document row is a file row and takes
  its directory's state, as file rows have since brief 05. Nothing here gives a
  file a state, and the fact that this particular file is the one the state is
  *about* changes how it is drawn in no way at all.
- **`App` touches no filesystem.** The read happens where the pact and the scope
  writes already happen — in the caller that resolves a keystroke — and what
  reaches `App` is bytes or an error, never a path to open later.
- **The panel is one slot and two cards, never three and never a list.** No
  history, no stack, no cycling. Shift-Tab is a toggle between exactly two
  things, one of which may not exist yet.
- **Nothing swaps the card except the reader.** This is the constraint that
  makes the feature worth having, and the one most likely to be eroded by a
  well-meaning "but the run just started". The tree already says a run is
  happening — brief 06 gave it the flash and the repaint — and a reader who
  wants the account has one key. Section 3 governs: visible and easy, never
  taken out of their hands.
- **The document card is a snapshot.** It does not follow the selection, it does
  not follow the watcher, and it does not update when a pass rewrites the file
  under it. A card that changed while somebody was reading it would be the same
  class of surprise as a tree that repainted itself out from under a keystroke.
- **`$EDITOR` gets the whole terminal or none of it.** Raw mode off, alternate
  screen left, mouse reporting off, child run to completion, all three restored
  — through `TerminalGuard` and `restore_terminal`, so there is still exactly
  one spelling of what "put it back" means, and the panic hook still covers the
  path where the child kills us.
- **Nothing new is fatal.** An unreadable file, a file of bytes, an unset
  `$EDITOR`, an editor that exits non-zero: each is a line in the footer the
  reader can act on, and none of them is a way out of the event loop.
- **Editing restales, and the brief says so out loud.** A `WARLOCK.md` is an
  ordinary file in the walk `subtree_hash` runs, so hand-editing a document
  turns its own directory yellow the moment it is saved, and the road back to
  green is a refresh that rewrites what was typed. That is section 9 working —
  manual edits are not fought, they are reconciled — and it belongs in the
  module's narrative and in `CLAUDE.md`, so a reader who does it has been told
  rather than surprised.

## Out of scope

**An editor in the panel.** Not deferred, refused. A text editor is a project
several times this one, and building a bad one inside a freshness ledger would
make warlock worse at the thing it is for while remaining worse than vim at the
thing it is not. `$EDITOR` is the answer and it is a better answer than a good
in-panel editor would be, because it is the one that says where the boundary is.

**More than one document at a time.** No tabs, no splits, no most-recently-used
list, no `v` on a second file leaving the first one somewhere. Opening a
document replaces the document. This is a constraint chosen for what it
communicates as much as for what it costs: warlock is not where you read your
codebase, and a reader who wants three files open is being told, gently, to use
the tool that is for that.

**A preview pane.** The panel does not follow the tree's selection. Making it
follow would delete the difference between looking at a row and asking for it,
and would mean the document you chose could not survive a keypress in the tree —
which is exactly what Shift-Tab exists to protect.

**Rendering the markdown.** No headings, no bold, no wrapping at a measured
column, no tables. Lines as they are in the file, one per row, cut at the
panel's width the way the account's lines already are. A `WARLOCK.md` is written
to be read as text by an agent, and this is the same text.

**Syntax highlighting, and searching within a document.** Both are real and
neither is the thing missing. They would also both want a dependency, and this
brief takes none.

**Viewing anything the walk does not list.** Gitignored and hidden files never
arrive in a node's listing at all — `.env` among them, doubly — and this brief
does not reach around the walk to fetch them. What *is* reachable is everything
under a `.warlockignore`d directory, which keeps its rows and its files on
purpose, so the folder of images warlock excluded is exactly the folder a reader
can look into and confirm was worth excluding.

**Any route from an edit back to green that is not a pass.** No "I fixed it
myself, mark it fresh", no manual grant, no key that records a hash without
describing the directory. Fresh is granted where a pass ran and nowhere else,
and an edit affordance is not a licence to open a second door into that rule.

**Telling a document that is behind its code from one that is not.** The panel
shows a document; it does not diff it against anything, annotate it, or say
which parts the model would now write differently. That is reconciliation, still
out since brief 08, and it needs the reading to exist before it can be built on
top of it.

Also out, unchanged from brief 09: telling direct staleness from inherited
staleness; configurable staleness; cache eviction; making the caps configurable;
enforcement of scopes against sigils in any form, including a `warlock check`;
crossings as records and the architecture diagnostic; blessing and the gate;
pact expansion proposals and footprint prediction; Linear and Red/Forman
integration; the journal at `~/.warlock/<project>/<date>`; the pre-push guard;
the CI validator; `warlock status` and every other non-interactive entry point;
the paid crate; and publishing anything.

## Scope

### 1. The document in the tree

depends_on: []

With files hidden, a documented directory draws one row beneath it: its own
`WARLOCK.md`, one level deeper, in the directory's colour, exactly the row `f`
already produces for it. The change is which filter keeps it, not what it is.

`reflow` applies three filters in a fixed order and its own documentation says
why files go first: the pactedness pass and the collapsed pass both reason by
depth, and a file row sits deeper than the directory holding it, so a hidden file
left in the list would be a row those passes had to reason around for nothing.
**That comment stops being true here and has to be rewritten rather than left
standing.** A surviving document row is precisely such a row, and both later
passes now have to be right about it: it keeps its directory's state so the
pacted-only pass keeps it wherever it keeps the directory, and it sits under the
directory so the collapsed pass takes it away with the rest. Both fall out of
what a file row already is, which is the argument for leaving it a file row
rather than inventing a fourth kind.

The reason it is not a toggle is the reason the brief exists. A reader meeting
warlock for the first time should see directories and one strange file and ask
the question themselves; behind `f` the effect never happens, because `f` shows
everything at once and the document goes back to looking like a `mod.rs`. `f`
keeps its current meaning — the files a module is made of — and gains nothing to
say about the one file that is not one of them.

Two states become visible that were not, and neither gets a marker. An unpacted
directory holding a `WARLOCK.md` left behind by `unpact_subtree`, which drops
manifest entries and no documents, now shows the document under a gray row: the
honest picture of what is on disk. And a pacted directory whose document has not
landed shows nothing under it, which is the same fact from the other side. The
tree says what is there. It does not editorialise about it.

The cost is paid knowingly. A fully-pacted repository draws roughly twice the
rows it draws today — this one goes from eight to fourteen — and the collapse
key is the existing answer to that. It is the right price: a tree of directories
and the one document each of them has is the smallest thing that can show a
reader what warlock produced, and the alternative that costs no rows is a marker
on the directory row in the slot brief 09 gave the scope label, which is
rejected because nobody can land on a marker and press `v`, and because it does
not produce the question. This is settled rather than provisional; reopen it on
evidence that the taller tree hurts, not on the appeal of the shorter one.

### 2. `v` opens a file

depends_on: []

`v` on a file row reads that file and puts it in the panel. On a directory row
it refuses, and it refuses on purpose: a directory is not a thing you open, and
the refusal names the document row beneath it — or names `p` where the directory
has no document — so the refusal teaches the tree rather than just saying no.
Refusals go where the pact key's refusals already go.

The read is the engine's, capped at `PER_FILE_BYTE_CAP`, and the cap is not a
new number. Brief 07 already decided how much of a file warlock is willing to
hold at once, and a viewer that would slurp a two-hundred-megabyte artifact into
a `Vec<String>` is the same defect the caps were introduced for. A file over the
cap is shown up to it and says on its last line that it was cut. That warlock
shows a reader exactly what it would show a model is a property worth having and
worth a test.

Three ways it can go wrong, all of them lines and none of them fatal: the file
cannot be read, the bytes are not UTF-8, or the row's path has vanished since the
tree was loaded. A binary file gets a line saying so rather than a panel full of
replacement characters — this is a tool for reading documents and source, and
pretending a PNG is text helps nobody.

Where the filesystem is touched is the constraint that shapes this slice. `App`
touches none, and this must not be the exception: the read happens in the caller
that dispatches the keystroke, next to where a pact write already happens, and
`App` is handed the lines or the error.

### 3. Two cards, one slot

depends_on: [2]

The panel holds the account or the document, never both and never neither. It
starts on the account — which before the first run is the mark, as it is today —
and Shift-Tab swaps to the other card. Crossterm delivers Shift-Tab as
`BackTab`, which `input.rs` already knows about and deliberately leaves unbound;
this is the binding it was left free for.

Before any `v`, the document card does not exist and Shift-Tab does not swap. It
says so instead, naming `v`, in the footer where every other refusal lives. This
is the same shape as `s` refusing on a gray row and naming `p`: a key that
silently does nothing reads as a key that is broken, and one line is the
difference.

Focus is a separate axis and stays one. Tab still moves the keys between the
tree and the panel; what changes is that `Focus::Panel` now scrolls the panel's
window over **whichever card is showing**, and its documentation — which
currently says the account, in as many words — is updated to say so. Each card
keeps its own scroll offset, so Shift-Tab away and back lands on the line the
reader left, and the account's offset goes on doing what brief 05 built it to do
while nobody is looking at it.

**Nothing else in the program changes which card is showing.** Not a pact
starting, not one finishing, not one failing, not the watcher reloading the
tree. This is the slice's real content and the one thing a test should hold
directly: start a run with a document in the panel, drive it to completion, and
assert the panel is still the document. The tree carries the run — brief 06 gave
it the flash, and the account is one key away for a reader who wants the detail
— and a panel that grabbed itself back would take that choice away at exactly
the moment somebody was using it.

### 4. `$EDITOR`, and what it costs

depends_on: [2]

`e` on a file row hands that file to `$EDITOR`. Warlock restores the terminal
whole — raw mode off, alternate screen left, mouse reporting off — runs the
editor as a foreground child, waits, and takes the terminal back. All three go
through `TerminalGuard` and `restore_terminal` rather than a second spelling of
teardown, and the panic hook keeps covering the path where the child takes the
process down with it. On return the tree reloads, because the file may well have
changed and the colours are downstream of that.

It refuses while a pact or refresh is running. A run owns the screen and is
writing progress into it, and suspending the terminal underneath one is a way to
come back to a corrupted display and a half-drawn account. The refusal is a
line, like every other.

`$EDITOR` unset is a refusal naming the variable, not a fallback to `vi`.
Guessing at an editor is how a program ends up holding somebody in modal vi they
did not ask for and cannot leave.

The consequence goes in the diff rather than in a reader's lap. Editing a
`WARLOCK.md` restales its own directory the instant it is saved — the document
is an ordinary file in `subtree_hash`'s walk — and the only road back to green
is `r` and a pass that rewrites what was typed. That is section 9 working
exactly as written, and it is the sharpest available statement of what warlock
is: it will hand you the pen and it will not pretend afterwards that the
document was granted. Say it in `claude_md.rs`'s block and in the module that
owns the key.

### 5. A footer that lays itself out

depends_on: []

The keys line is at 148 columns of a 148-column budget, enforced by a
compile-time assert that exists to stop exactly this brief from widening it.
There is no slack. `v: view` costs eleven columns with its gap and `e: edit`
another eleven, and the shortenings that bought `r` and `s` have already been
spent — the names left on the line are `k/j: row`, `PgUp/PgDn`, `g/G: ends`.
There is nothing there to sell.

So this is the brief that pays the debt `MAX_KEYS_WIDTH`'s own documentation
records: "the fix is a footer that lays the keys out for the width it has, and
that is not this constant's job." It is this slice's job. The footer takes the
width it was given and fits what it can, dropping from the middle rather than
the end, so that the way out survives on an eighty-column terminal instead of
being the first thing cut. The budget constant and its assert go with the
problem they were holding still.

If that turns out to be more than this brief should carry, the fallback is to
put `v` on the line and leave `e` and Shift-Tab off it, on Tab's precedent —
Tab has never been on the keys line and the program has not suffered for it.
That is a real option and it should be taken deliberately rather than by
running out of room. Decide it here, in this slice, and not in the two slices
that add the keys.
