# Red brief: Back to green

A yellow module with no way home. Since brief 04 the only route from yellow
back to green has been the pact key pressed twice: an un-pact that drops the
grant on every directory in the subtree, then a pact that buys a model pass for
every directory in it — the ones nothing touched included. Edit one file in a
forty-directory repository and the road back costs forty passes to re-derive
thirty-nine documents that were already true, and it begins by throwing away
the thirty-nine grants that said so. Section 6 of the design doc has carried
the consequence as a known tradeoff from the start: "on a repo nobody refreshes
for a week, the whole tree goes yellow and the signal degrades." It degrades
because the only way back is priced like a first pact.

This project gives yellow a one-key road home and prices it by what actually
changed. A refresh descends the subtree, skips every directory that is already
green, and runs a pass only where the hash says something happened — deepest
first, so a parent is described with its children's new documents already on
disk. Nothing is un-pacted on the way, so a grant is only ever replaced by a
newer one.

The other half of the same problem is that warlock covers directories nobody
asked it to cover. A repository has content that is real, tracked, and no part
of what an agent needs to understand the code: a folder of images, a design
notebook, the author's own notes. Today the only way to keep those out of a
pact is to keep them out of git, which is not a trade anybody should have to
make. `.warlockignore` lets a repository say what warlock is not about, in the
syntax every developer already knows.

Read `docs/warlock-design-doc.md` before drafting, sections 5 and 6 in
particular. **This is the first brief that moves `subtree_hash`'s input rules**,
which 02 through 07 each froze — and it moves them deliberately, so read
section 6's split of trigger from judge before deciding anything about the hash.
Section 5's paragraph on the walk ("deliberately not a skip list Warlock
maintains") needs an amendment rather than a deletion: that argument is about
*warlock* keeping a list, and `.warlockignore` is the repository's own. The
`node_modules` line stands — that is still gitignore's job.

This project depends on brief 07 for its economics rather than its code. A
refresh re-reads directories, and a directory holding a big file re-reads that
file — except that brief 07 made an unchanged file's summary free, so the pass
a refresh pays for is the directory pass and nothing else. Brief 07 said this
one would be the cheaper problem for it; this is where that is spent.

## Outcome

Work in a repository with warlock open. Change a file, watch its directory and
the ones above it go yellow, and press one key on any of them. Only the yellow
directories under it run a pass — the green siblings are not touched and not
paid for — and the tree goes green. In a forty-directory repository after a
one-file edit, that is a handful of passes instead of forty, and no grant was
discarded to get there.

Put `/docs` and `/assets` in a `.warlockignore` and pact the repository root.
Those directories stay in the tree, gray, outside warlock's management, and no
pass is ever spent on them. Edit a file in either one and nothing turns yellow,
because nothing warlock wrote ever described them. Green now means "nothing
warlock covers has changed", which is what it was always reaching for.

Watch a pact run with files shown. Each `WARLOCK.md` appears in the tree as the
pass that wrote it finishes, under the directory it belongs to, instead of the
whole set arriving at the end — so a row that has gone green is a row whose
document you can see and open.

Reach for the key that stops a run and miss. Esc with nothing running no longer
drops you back to a shell: a small window asks whether to leave, No is where the
highlight starts, and any answer but yes puts you back exactly where you were.

Run `warlock init` in a repository and an `AGENTS.md` is there afterwards,
saying what warlock is, what the colours mean, and what to do with the
`WARLOCK.md` files — so a session started in that repository knows the fast way
in without anybody explaining it again.

## Success criteria

- The refresh key on a stale directory runs a pass on each stale directory at
  or below it and on none of the fresh ones, pinned in engine tests by a fake
  agent that records which directories it was asked about.
- A refresh whose passes all succeed leaves every directory it covered green,
  and leaves the grant on every directory it skipped exactly as it found it —
  same hash, same timestamp. No directory loses a grant to a refresh.
- Refreshing a subtree with one changed file in it costs one pass per directory
  on the path from the change to the refreshed root, and no others.
- The refresh key on a fresh directory runs nothing and says so; on an unpacted
  or ignored directory it refuses and names the key that would help; on a file
  row it refuses as the pact key already does.
- A refresh in flight cancels on Esc, records what it completed, and reports
  like a pact. A second refresh or pact key pressed during one starts nothing
  and says so, exactly as a second pact key does today.
- The footer counts a refresh by what a refresh is doing — the stale
  directories it will visit, not every directory in the subtree — so the
  progress line is never a count of work that is not going to happen.
- A directory matched by `.warlockignore` is never pacted, never given a
  document and never given a manifest entry. It stays in the tree, drawn gray.
- A file matched by `.warlockignore` is never gathered into any request, and
  never contributes a byte to any `subtree_hash`.
- Editing, adding or deleting ignored content changes no directory's colour.
- `.warlockignore` is honoured with gitignore's own semantics — nesting,
  negation with `!`, anchoring, directory-only patterns — because it is
  gitignore's own matcher, not a second implementation.
- Adding, editing or removing a `.warlockignore` restales the directories whose
  covered content it changes, and that is documented as the intended
  consequence rather than discovered as a surprise.
- With files shown, each `WARLOCK.md` a running pact writes appears in the tree
  as its directory is reported documented, in that directory's current colour,
  and the rows the run has already painted keep the colour the run gave them.
- Esc and `q` open a confirmation, drawn over everything else, with No
  highlighted first. Yes leaves; No, Esc and any unbound key return to exactly
  the view that was on screen. Ctrl-C still leaves immediately.
- The confirmation never appears while a pact or refresh is running, because Esc
  means cancel there and `q` is the deliberate way out of a run.
- `warlock init` writes `AGENTS.md` at the repository root, creates it if it is
  absent, and updates its own section in place if it is present — running it
  twice leaves the file byte-identical to running it once, and never disturbs a
  line it did not write.
- `warlock` with no arguments still opens the TUI, unchanged.
- `warlock-engine` still opens no sockets and spawns no subprocesses, and its
  tests still run with no terminal, no network and no `claude` binary present.
- The gate — `cargo fmt --all --check`, `cargo clippy --workspace
  --all-targets -- -D warnings`, `cargo test --workspace` — stays clean.

## Constraints

- Everything from briefs 01 through 07 still holds: stable Rust 2024, system
  toolchain and no `rust-toolchain.toml`, Apache-2.0, no `unsafe`, no HTTP
  client, no Anthropic SDK, no mocking framework, no async runtime. The
  dependency edge still runs TUI to engine and never back.
- **No new dependency.** `.warlockignore` is the `ignore` crate's
  `add_custom_ignore_filename`, which the workspace already has and which is
  the whole reason `.gitignore` is honoured properly today. `warlock init` is
  one subcommand read off `std::env::args`, not an argument parser.
- **Still three colours, and no fourth.** An ignored directory is gray, because
  gray already means exactly "outside warlock's management" — and gray under a
  green parent is a state the tree can already reach, since `unpact_subtree`
  drops only entries at or below the directory it is given and leaves every
  ancestor's grant standing. Nothing here adds a state, a shade or a marker.
- **A refresh never un-pacts anything.** It replaces grants and writes
  documents; it removes no manifest entry and drops no grant, including on the
  directories it skips. This is the difference between it and the two key
  presses it replaces, and it is the property most worth a test.
- **Fresh is still only ever earned.** A refresh grants a directory because a
  pass ran on it and a hash was taken afterwards, exactly as a pact does. There
  is no path here that re-grants a directory without describing it, and no
  "still holds" shortcut that returns green without a pass.
- **The AGENTS.md text is code.** It lives beside the prompts in the crate,
  reviewed in a diff like everything else. No template file, no configuration,
  no override.
- **`warlock init` writes one file and nothing else.** No `.warlock/`, no
  `.warlockignore`, no manifest, no pact, no model pass. It is scaffolding, and
  it prints what it did on the ordinary screen without ever entering the
  alternate one.
- **Nothing new is fatal.** A `.warlockignore` that cannot be read, an
  `AGENTS.md` that cannot be written, a refresh whose passes fail: each is a
  line the reader can act on, and none of them is a way out of the event loop.
  Section 3 of the design doc still governs.
- **Refresh is still manual and nothing runs by itself.** A refresh happens
  because a key was pressed. No refresh on startup, no refresh on save, no
  background pass.
- **The keys line has a width problem already** — its own documentation says it
  is wider than eighty columns — and this brief adds a key to it. Whichever
  slice puts `r` on that line is the slice that has to say how the line still
  fits, rather than pushing the way out off the right-hand edge.

## Out of scope

**Telling direct staleness from inherited staleness.** The obvious companion to
the refresh key: record what a directory's *own* files hash to, so the tree can
show the directory you edited differently from the eight ancestors that are
yellow because of it. Deferred deliberately and nothing is blocked behind it —
the refresh's skipping is computed from the per-directory grants the manifest
already has, so this would buy legibility rather than cost. It wants a second
field on `PactEntry` and a decision about how the tree draws the difference,
which is a display question and its own project.

**Reconciliation, diffs, and "still holds".** A refresh here re-reads a stale
directory as it is now and writes a new document, exactly as a first pact does.
It is not shown its own diff, it is not asked whether the old document survives,
and there is no confirming pass that re-grants without rewriting. Those are the
questions briefs 06 and 07 both named, and answering them needs the refresh to
exist first.

**Configurable staleness.** No strict mode, no relaxed mode, no setting that
decides whether staleness climbs the tree. It climbs, because a parent's
document is written from its children's documents and is genuinely falsified
when they change; a mode that painted such a parent green would make green mean
nothing in exactly the repositories where it matters most. `.warlockignore` is
the sanctioned way to make the tree quieter, and it works by removing content
from what warlock covers rather than by loosening what a colour promises.

**An un-pacted child's document is still gathered by its parent's pass.**
`pact_directory` reads the `WARLOCK.md` files it finds beside a directory's
children and consults no manifest, so a document left on disk by an un-pact is
still material for the parent above it. That is today's behaviour, it is
unchanged here, and it is named so it is a known wart rather than a discovery.

**Cache eviction**, as in brief 07, and for the same reasons.

**Making the caps configurable.** `PER_FILE_BYTE_CAP`, `REQUEST_BYTE_CAP` and
`WALK_DEPTH` hold their values and their non-configurability. `.warlockignore`
changes what warlock is pointed at, not how much of it fits in a request.

Also out, as before: blessing and the gate, pact expansion proposals and
footprint prediction, Linear and Red/Forman integration, the journal at
`~/.warlock/`, the pre-push guard, the CI validator, `warlock status` and any
other non-interactive entry point that reports freshness, the paid crate, and
publishing anything. `warlock init` is a subcommand that writes a file, and is
deliberately not the thin end of a CLI that pacts or judges.

## Scope

### 1. Refresh the stale, skip the fresh

depends_on: []

The engine learns to descend a subtree and pass over what is already green.
`pact_subtree` walks its directories and describes all of them; a refresh walks
the same directories and describes only the ones `decide_state` calls stale,
using the manifest entry each already carries and the hash of its content now.
That comparison exists and is pure — this slice is about which directories reach
phase one, not about a new judgement.

Pruning at green is allowed, and is worth pinning as the property it is. A
directory is granted only when every directory beneath it was documented in the
same run — `pact_subtree` withholds the grant from any directory with an
undocumented descendant — so green implies the whole subtree beneath it is
green, and a green row can be skipped along with everything under it in one
test. That invariant is load-bearing for the cost model, so it gets a test of
its own rather than a comment.

**The hazard is phase two, and it is the one thing in this brief that can
silently destroy state.** `rewrite` drops any existing entry whose module the
pact covered but which earned no entry this run — that is how an un-pact and a
partial pact stay honest. A refresh covers directories it deliberately does not
describe, so left alone it would strip the grant from every green directory it
skipped and turn the whole subtree yellow: the exact opposite of the point.
Skipped directories must carry their existing entries through untouched, and a
test should hold a refreshed subtree's skipped entries up against the ones it
started with, hash and timestamp included.

Order is deepest-first, as a pact already is, and for a sharper reason here: a
parent is re-described from its children's documents, so a refresh that ran a
parent before its stale child would write the parent from the document it was
about to replace.

Everything else about a run is unchanged, because it is the same run: the same
observer, the same cancellation between directories, the same partial-completion
rule, the same one timestamp for the whole pass.

### 2. The refresh key

depends_on: [1]

`r`, and only on a directory that is pacted and stale. On a fresh one it runs
nothing and says the directory is already fresh; on an unpacted one it refuses
and names `p`; on a file row it refuses the way `p` already does. Refusals go
where the pact key's refusals go, so nothing here invents a second way of
saying no.

It spawns a worker and reports the way a pact does — the same channel, the same
account in the panel, the same cancel handle, the same "already running"
refusal if a run is in flight — because it *is* a run, and a reader watching one
should not have to learn a second set of rules for it.

What has to be its own is the counting. A pact says "directory 3 of 40"; a
refresh over the same subtree is visiting seven directories, and saying 40 would
be a progress line for work that will not happen. The footer counts the stale
directories the refresh will visit, and the wording says refreshing rather than
pacting, so the reader can tell which of the two they started.

Then the keys line, which is where this slice has to be careful: it is already
documented as wider than eighty columns, and the way out lives at the end of it.
Adding `r` without shortening something pushes "how do I leave" off the edge.
Decide the wording here.

### 3. `.warlockignore`

depends_on: []

One filename, given to the `ignore` crate at three of the four walks, and
withheld from the fourth on purpose:

- `pactable_directories` (`pact.rs`) — an ignored directory is never offered a
  pass, so it gets no document and no manifest entry, and draws gray.
- the request walk (`pact.rs`) — an ignored file inside a pacted directory is
  never gathered, so it reaches no request in any of its three states.
- `subtree_hash` (`hash.rs`) — ignored bytes never break a hash. This is the
  one that makes the feature worth having: without it, ignoring `/docs` would
  stop the passes but still turn the tree yellow every time a note was edited,
  and the refresh those edits provoked would pay for a pass to re-grant against
  content it never read.
- the tree walk (`load.rs`) — **unchanged**, so ignored directories keep their
  rows. The reader can see what they excluded, and `p` on such a row refuses
  and names the file, the way it already refuses a file row.

`add_custom_ignore_filename` is the whole mechanism, and it brings gitignore's
real semantics with it — nested files, `!` negation, anchoring, directory-only
patterns — which is the same argument the workspace already made for taking the
`ignore` dependency rather than hand-rolling a skip list.

Two consequences to write down rather than leave to be found. First, the hash
change is the first deliberate move of `subtree_hash`'s input rules since brief
02, so `HASH_CONTEXT` and the input rules' documentation both need to say what
they now cover and why. Second, adopting a `.warlockignore` restales every
directory whose covered content it changes, all at once. That is honest — what
the pact was taken over really did change — but it should be stated in the
module's own narrative, because a reader who adds a file and watches their tree
go yellow deserves to have been told.

Section 5 of the design doc gets its amendment in the same diff: the walk is
still not a skip list warlock maintains, and this is the repository's own.

### 4. Documents appear as they land

depends_on: []

A running pact writes a `WARLOCK.md` beside each directory as it finishes it,
and with files shown none of them appears in the tree until the run ends. A row
goes green claiming it is documented while the document it names is not on
screen. The engine already reports it — `Observer::documented` fires per
directory and the loop already turns it into a repaint — so the fact is
arriving; nothing is drawing it.

The obvious fix is the wrong one and the slice should say so in its diff.
`reload_tree` re-derives every colour from the manifest **on disk**, and during
a run that manifest is still the pre-pact one — it is written once, at the end.
Calling it mid-run would repaint every row from stale state and wipe the green
the run has been painting, which is exactly why the reload sits at the bottom of
`apply_progress` and not in the event arm.

So the row is added rather than reloaded: on `Documented`, insert the document's
file row under the directory it belongs to, carrying that directory's current
state so it draws in the colour the run just gave it. It goes into the walk
behind the rows as well as the drawn rows, for the same reason a state change
does — a row written only into the drawn list lasts until the next collapse and
then quietly vanishes. With files hidden nothing appears, because nothing
should; the row is there when `f` asks for it.

### 5. A gate on the way out

depends_on: []

Esc and `q` stop leaving immediately. A small window is drawn over everything
else — centred, cleared behind so the tree does not show through — asking
whether to leave, with No highlighted first. Left and right move the highlight,
Enter takes it, `y` and `n` answer outright, Esc is No, and every other key does
nothing. No returns to exactly the view that was on screen, because the dialog
is a mode over the app and changes nothing in it.

Ctrl-C keeps leaving immediately, and that is deliberate. In raw mode it is not
a signal — if warlock does not answer it, nothing does — so it is the one way
out that must never depend on a dialog behaving. It is the reader's last resort
and it stays one.

The dialog never appears during a run, because during a run those keys already
mean something better: Esc cancels the pact, and `q` is the deliberate way out
that the footer already advertises alongside it. What this gate protects against
is the press *after* the cancel — the reflex second Esc that today lands on quit
— which is exactly the case where the dialog is standing between a reader and a
terminal they did not mean to be back in.

The footer says what the keys do, as it always has.

### 6. `warlock init`

depends_on: []

The first subcommand. `warlock init` writes an `AGENTS.md` at the repository
root — the nearest ancestor with a `.git/`, the same anchor the tree already
resolves from, so it can be run from any subdirectory and writes one file in the
right place. Outside a repository it refuses with a line, as the TUI does.
`warlock` with no arguments opens the TUI exactly as it does today; this is one
string compared against `args().nth(1)`, not an argument parser, and not a new
dependency.

Create or update. An absent file is written; a present one keeps every line it
has and gains warlock's section, delimited so a second run replaces that section
in place instead of appending a second copy. Running it twice leaves the file
identical to running it once, and nothing outside the delimiters is ever touched
— this is somebody's own file, and warlock owns a block in it rather than the
whole of it. The write-and-rename idiom the manifest and the documents already
use applies here too.

The text is a constant in the crate, reviewed in a diff like the prompts. What
it covers: what warlock is; that a `WARLOCK.md` is a per-directory document
written by a model and committed beside the code; that the colours mean
unpacted, stale and fresh, and that stale is mechanical while fresh is granted;
that a subtree hash is the trigger and not a judgement; and the load-bearing
instruction, which is to **read the `WARLOCK.md` files first** — they are the
fast route into the codebase, and a stale one may be behind the code it
describes.

What it deliberately does not contain is a template for writing a `WARLOCK.md`.
A pacting pass runs with no tools and a system prompt that replaces the CLI's,
with its whole request on stdin: it cannot read this file, and would not be
asked to. Guidance here would be a second prompt that no pass obeys, drifting
away from the real one in `pact.rs` with nothing to catch it. The prompts stay
code, and this file stays orientation for the reader's own agents.
