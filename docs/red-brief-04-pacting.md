# Red brief: Warlock pacting

Make pacting real. `WARLOCK.md` replaces `README.md` as the file Warlock owns,
the pact key stops writing a manifest entry and starts invoking `claude`, and
pacting a directory pacts everything beneath it — a document per directory, a
manifest entry per directory, a granted hash per directory. Green becomes
reachable through the product for the first time.

Read `docs/warlock-design-doc.md` before drafting, sections 5, 8 and 11 in
particular. **Section 5 was rewritten after brief 03 was cut**, and the two
paragraphs added to it — "A module is a directory" and "Warlock's file is
`WARLOCK.md`" — are what this project implements. Section 17 lost its
pact-granularity bullet in the same edit: the subtree pact is rejected, not
parked, and nothing here should reintroduce it.

This brief stops at the pact. **The refresh pass is brief 05.** That split is
deliberate rather than a size dodge: pacting writes a document where there was
none, refresh reads a diff and decides whether an existing document still
holds, and the second needs the first's machinery plus a judgment this project
does not have to make. Building both at once means designing the reconciliation
rules while the invocation seam is still moving. The known cost is that at the
end of this project a directory can go green once and has no way back to green
after it is edited. Accepted knowingly, and stated in the README rather than
left to look like a bug.

## Outcome

`cargo run` in a repository nobody has pacted, cursor on the root, press the
pact key. Warlock walks down, invokes `claude` once per directory, writes a
`WARLOCK.md` into each, records what it granted, and the tree turns green from
the leaves up. Press it on `crates/warlock-engine` instead and exactly that
subtree goes green while the rest of the repo stays gray.

The tree stays drawn and responsive the whole time, because a root pact over a
real repository is minutes of model calls, not a keystroke that returns before
the next frame.

## Success criteria

- Pressing the pact key on a directory with no `WARLOCK.md` creates one in that
  directory and in every non-ignored directory beneath it, each with a manifest
  entry, and the subtree renders green.
- Pressing it on a directory that is already pacted un-pacts that subtree: the
  entries go, the nodes return to gray. The `WARLOCK.md` files stay on disk —
  they are documentation, and deleting a user's committed files on a keystroke
  is not something this tool does.
- A repository with no `claude` on `PATH` still launches, still draws its tree,
  and says so when the pact key is pressed rather than failing silently or
  crashing.
- Every `WARLOCK.md` Warlock writes is the product of a model pass over that
  directory's actual contents. There is no template, no stub, and no
  placeholder document that a later refresh would have to recognise and unpick.
- A pact interrupted halfway leaves a coherent repository: directories that got
  a document and a hash are green, directories whose subtree did not complete
  are yellow, and nothing is left claiming freshness it did not earn.
- Cancelling an in-flight pact stops it, kills the running `claude`, and leaves
  the same coherent state as an interruption.
- The tree redraws while a pact is running. Movement keys still work, the
  footer says what is happening and how far along it is, and the terminal is
  never left in a broken state on any path out.
- `warlock-engine` still opens no sockets and spawns no subprocesses, and its
  tests still run with no terminal, no network and no `claude` binary present.
- The gate — `cargo fmt --all --check`, `cargo clippy --workspace --all-targets
  -- -D warnings`, `cargo test --workspace` — stays clean.

## Constraints

- Everything from briefs 01 through 03 still holds: stable Rust 2024, system
  toolchain and no `rust-toolchain.toml`, Apache-2.0, no `unsafe`, **no HTTP
  client, no Anthropic SDK**, no mocking framework. The dependency edge still
  runs TUI to engine and never back.
- **The engine still spawns no processes.** `lib.rs`'s module documentation
  promises this in as many words and the promise stays. See slice 2: the engine
  owns what to ask and what to do with the answer, the binary owns the
  subprocess, and they meet at a trait.
- **No async runtime.** A background thread and a channel from `std` cover what
  slice 5 needs. `tokio` for one long-running child process is a large
  dependency bought for a small problem.
- **No new dependencies are sanctioned.** Process spawning is `std::process`,
  concurrency is `std::thread` and `std::sync::mpsc`. If something genuinely
  cannot be done without a crate, that needs a reason in the diff and a line in
  the workspace table.
- **`NodeState` keeps exactly three variants.** "Currently being pacted" is not
  a fourth colour. It is a fact about a run in progress and lives in the TUI
  alongside the other view state, for the same reason expansion and scroll
  position do.
- Warlock still does not parse `WARLOCK.md`. It cares that one exists and what
  its bytes hash to. The section skeleton is section 17's open question and
  stays open — this project writes whatever the model returns.
- The manifest format does not change. One entry per directory, holding the
  directory, the document, and an optional granted hash. That optionality is
  what slice 4 leans on for partial completion, so nothing about it moves.
- Hashing rules do not change. Deterministic over sorted relative paths and file
  bytes, the document hashed along with everything else, pacted subtrees only.

## Out of scope

**The refresh pass, entirely.** Reading a diff, deciding whether an existing
document still holds, updating it, regranting. That is brief 05 and it is the
larger half of section 6.

Also out: blessing and the gate, pact expansion proposals and footprint
prediction, Linear and Red/Forman integration, the journal at `~/.warlock/`,
the pre-push guard, the CI validator, the paid crate, file watching or any
daemon, and publishing anything.

**Reading a file's contents in the TUI is still out**, as it was in brief 03.
Files are visible, not readable. The viewer pane remains the next real feature
and is still not this one.

**The project-and-ticket pipeline is out.** Section 11 names two invocation
lifetimes and this project implements only the short one. The ten-minute
human-in-the-loop session that Red and Forman prototype is a different shape
with different problems, and nothing here should be built to accommodate it.

**Do not build a prompt-tuning surface.** No configuration file for the prompt,
no per-project overrides, no template directory. The prompt is code in this
project. Making it configurable before there is a single prompt that works is
building the knob before the thing the knob turns.

## Scope

### 1. `WARLOCK.md`, and the vocabulary that goes with it

The file Warlock writes is `WARLOCK.md`. `load.rs`'s `README_FILE` constant is
the only place the name is detected, and `PactEntry`'s stored value was always
a path rather than an assumed filename — its own doc comment says "the file
name is not Warlock's to assume" — so the mechanical change is small and the
naming change is large.

Rename what the name touches. `Node::readme`, `Row::readme`, `PactEntry::readme`
and `PactEntry::readme_path`, `IntoReadme`, `has_readme`, and the doc comments
and README passages that explain any of them. Section 17's first open question
is "manifest schema fields — decide early; migrations hurt", and this is that
bullet coming due: there are no manifests in the wild yet, so the field is free
to rename today and a migration to rename later.

Pick one word and use it everywhere rather than letting `doc`, `document` and
`warlock_md` all appear. The brief has no opinion on which, only that a reader
should never have to wonder whether two names mean the same field.

This slice changes no behaviour. A repository pacted before it and a repository
pacted after it differ in one filename and one field name, and the tree looks
identical. That is what makes it safe to land first and separately, and it
should land first and separately: it touches nearly every file in the
workspace, and a mechanical diff of that size tangled up with an architectural
one is two reviews nobody can do.

Existing `README.md` files stop being special. They are ordinary files, listed
under their directory when the file toggle is on, hashed as part of the
subtree, and never written to.

### 2. The agent seam

depends_on: [1]

Section 11 fixes the mechanism: Warlock invokes the `claude` CLI as a
subprocess, feeds it a prompt and scoped context, and reads stdout. It holds no
credentials and is inert without a logged-in `claude` on `PATH`.

That collides with a promise `lib.rs` makes today — the engine "opens no
sockets and spawns no subprocesses" — and the collision is the interesting part
of this slice. Resolve it with a port rather than by moving domain logic into
the front end:

- The **engine** defines a trait for one model pass: given a request, produce a
  response or an error. It builds the requests, interprets the responses, and
  owns every decision about what a pact means. It still spawns nothing.
- The **binary** implements that trait with `std::process::Command`, owning the
  child, its stdin, its stdout and its exit status.

The alternative — a third crate that spawns `claude` and sits between the two —
buys isolation this workspace does not need yet and costs a crate boundary to
maintain. The alternative that is actually wrong is putting prompt construction
in `warlock-tui`, which would put domain logic on the far side of the seam
brief 02 was careful about.

The payoff is testability. Engine tests supply a hand-written fake that returns
canned markdown, so the whole pact operation is exercised with no `claude`
binary, no network and no terminal — which is the standing rule for that crate's
tests, and the reason a mocking framework was never needed and still is not.

Failures are ordinary and each needs its own answer: no `claude` on `PATH`, a
`claude` that exits non-zero, one that writes nothing to stdout, one that hangs.
The first is the common case on a fresh machine and deserves a message naming
the binary rather than a generic I/O error. A timeout is worth having; pick a
duration generous enough that a slow model pass is not mistaken for a hang.

### 3. What one directory's pact sends and gets back

depends_on: [2]

Section 11 calls context scoping "the actual differentiator: maximal relevant
context, minimal waste." This slice is that sentence turned into a decision, for
one directory in isolation.

Decide and write down what a single pact request carries. The directory's own
files are obviously in. Whether its children's `WARLOCK.md` files are in is the
question worth thinking about, and slice 4's ordering exists to make the answer
available: pact depth-first, children before parents, and a parent's request can
carry its children's finished documents instead of their full source. A module
summarising its submodules from their documentation is both cheaper and closer
to how the tree is meant to be read.

Decide what happens with a directory holding a hundred files, or one file of
four megabytes. There is a budget whether or not it is written down, and an
unwritten one is discovered in production.

Decide the invocation mode section 11 leaves open — headless print mode per
directory against a longer session — and say why in the diff. Per-directory is
the obvious default given that each directory is independent and the pass is
specified as short-lived, but the cost of re-establishing context every time is
real and worth naming.

The response is markdown, written to `<directory>/WARLOCK.md` verbatim.
Warlock does not parse it, reformat it, or validate its sections; section 17's
skeleton question is open and this project does not close it. What Warlock does
check is that the response is non-empty and is not an error message the model
produced instead of a document — a zero-byte or apologetic `WARLOCK.md` that
then gets hashed and granted is a false green, and section 6 is explicit that
green is earned.

Nothing here writes a manifest entry or computes a hash. This slice is one
request, one response, one file.

### 4. Pacting a subtree

depends_on: [3]

The pact operation proper: one keystroke, N directories.

Walk the selected node's subtree, depth-first, children before parents, and run
slice 3's operation on each. The ordering is not cosmetic. A parent's request
wants its children's finished documents (slice 3), and a parent's subtree hash
covers its children's `WARLOCK.md` files, so a parent granted before its
children are written is stale the instant they are.

That second consequence forces the shape of this slice: **write every document
first, then compute and grant every hash.** One phase writing files, a second
phase hashing and recording. A per-directory "write, hash, grant" loop
invalidates each ancestor as it descends and produces a subtree that is yellow
the moment it finishes, which is exactly the bug that would take a day to find
and one sentence here to prevent.

Partial completion is the interesting case and the manifest already has the
answer. A directory whose entire subtree got a document gets an entry with a
granted hash and renders green. A directory with any failed descendant gets an
entry with **no** granted hash, which by section 6's rule is pacted and stale,
and renders yellow. That is what the optional granted hash was designed for in
brief 02 §1 — "an entry with no granted hash is a module that is pacted but has
never been judged" — and reusing it here means partial failure needs no new
representation and no fourth state.

Un-pacting is the inverse and stays simple: remove the entries for the subtree,
leave the files. Say so where a user will read it, because "pact then un-pact"
looks like it should be a round trip and it deliberately is not.

The manifest is written once, at the end, through the existing atomic save. A
root pact over a large repository writes hundreds of entries in one commit;
that is the known cost of one file per repository, accepted in brief 02 §1 and
not reopened here.

### 5. The tree while a pact is running

depends_on: [4]

Brief 03 flagged this and left it: "real pacting is a long-running action with a
model call inside it, where today's is a keystroke that returns before the next
frame. That is a UI problem — progress, cancellation, what the tree does while a
scan is in flight — and it is the AI brief's to solve."

Solving it means the pact runs off the event loop. A background thread does the
work and sends progress over a channel; the loop keeps drawing, keeps handling
keys, and drains the channel each frame. No async runtime, for the reason under
**Constraints**.

Progress needs to say which directory is being worked and how many are left,
because a root pact on a real repository is minutes and a frozen-looking tree is
indistinguishable from a hung one. The footer's message line, added in brief 03 §3, is
the place for it; a second progress widget is more chrome than this needs.

Cancellation has two halves and both matter. The orchestration stops descending
between directories, and the `claude` currently running is killed rather than
left orphaned holding the user's subscription. Documents already written stay,
and the manifest records what actually completed, which is slice 4's partial
rule doing its job for a second reason.

Decide what the tree shows for a directory being worked right now. It is not a
fourth state and it is not a colour — see **Constraints** — so it is a marker,
a spinner, or nothing at all beyond the progress line. Nothing is a defensible
answer if the progress line is good enough.

What must not happen: a keystroke that starts a second pact while one is
running, a pact that keeps running after the user quits, and a terminal left
unrestored because the work was in flight when something failed. Brief 01's
restoration guarantees apply on every path out, including the ones that now go
through a thread.

Finish with the documentation, which now splits by durability rather than
going in one place. The root README no longer tracks what currently works — it
carries "(Currently under development)" at the top instead, deliberately, so
that a section describing the state of the build does not have to be rewritten
every week. So:

**Durable product facts go in the root README**, in the sections it already has.
That Warlock's artifact is a `WARLOCK.md` per directory and that the project's
own `README.md` files are never written to. That pacting is recursive, and that
un-pacting removes the pact but leaves the documents. That `claude` must be on
`PATH` and that Warlock holds no credentials of its own — which is a promise
about the product, not a note about this release, and belongs alongside the
others. None of these change when refresh lands.

**State-of-the-build facts go in the two crate READMEs**, which do track what is
implemented: that green is now reachable, and that a node which goes stale
cannot yet be made green again because refresh is the next project.
