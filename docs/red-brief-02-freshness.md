# Red brief: Warlock freshness engine

Replace the stub tree with the real one. Warlock walks an actual repository,
learns which modules are under pact from a state file it owns, hashes their
subtrees, and colours the tree from that comparison. This is the load-bearing
mechanical half of the product: the trigger, not the judge. No AI, no
`claude`, no Linear.

Read `warlock-design-doc.md` in this repo before drafting, sections 5, 6, 8 and
9 in particular. Foundations (red brief 01) is already merged: the workspace,
the `NodeState` vocabulary, `Node`/`Tree`/`DepthFirst`/`StateCounts`, the stub
tree and the TUI shell that renders it all exist and pass CI.

## Outcome

`cargo run` in a real repository draws that repository. Every module starts
gray. Pressing the pact key on a node writes it into `.warlock/` and it turns
yellow immediately. Edit a file underneath it, relaunch, and it is still
yellow, because the subtree hash it was granted against no longer matches — and
nothing in this project can turn it green, because green is earned by an AI
pass and that is the next brief.

The `.warlock/` manifest schema is decided here and written down. The design doc
calls it out as the open question whose migration hurts most, so it stops being
open at the end of this project.

## Success criteria

- `cargo run` from this repo's root renders warlock's own module tree — the
  directories that actually have READMEs — with correct nesting and no
  `target/` or `.git/` noise, and launch is imperceptible.
- A gray node pressed with the pact key turns yellow, and stays yellow across a
  relaunch. Pressing it again returns it to gray. Both write `.warlock/`.
- Touching any file at or below a pacted node changes its computed subtree hash
  and every ancestor pacted node's hash too. Touching a file in an ignored
  directory changes none of them.
- Hashing the same tree twice gives the same answer; hashing a fresh clone of
  the same commit at a different absolute path gives that same answer again.
- Launching from a subdirectory renders that subdirectory as the root while
  still finding the repository's one manifest above it, and a first launch in a
  repository with no `.warlock/` at all works and renders everything gray.
- `warlock-engine`'s tests build every tree they assert on from fixtures in a
  temporary directory, and still need no terminal.
- The existing gate — `cargo fmt --all --check`, `cargo clippy --workspace
  --all-targets -- -D warnings`, `cargo test --workspace` — stays clean, and
  every new dependency carries its one-line justification in the workspace
  table.

## Constraints

- Everything from brief 01 still holds: stable Rust 2024, system toolchain and
  no `rust-toolchain.toml`, Apache-2.0, no `unsafe`, no HTTP client, no
  Anthropic SDK, no mocking framework. The dependency edge still runs TUI to
  engine and never back.
- Three new dependencies are sanctioned and no more: `blake3` for hashing,
  `ignore` for gitignore semantics, and one serialisation format crate for the
  manifest. Anything beyond those three needs a reason in the diff.
- The engine gains filesystem access and nothing else. It still opens no
  sockets, spawns no processes and knows nothing about terminals.
- The manifest is committed to git. `.gitignore` currently holds `.red`,
  `.forman` and `/target`; `.warlock` does not join them.
- Warlock does not parse READMEs. It cares that one exists and what its bytes
  hash to. The section skeleton is still an open question and stays open.
- Do not follow symlinks, and do not let a directory cycle hang the walk.

## Out of scope

The AI half, entirely: invoking `claude`, the refresh pass, prompting, context
scoping, anything that could grant freshness. Also out: blessing and the gate,
pact expansion proposals and footprint prediction, Linear and Red/Forman
integration, the journal at `~/.warlock/`, the pre-push guard, the CI
validator, the paid crate, file watching or any daemon, editing files from
inside the TUI, and publishing anything.

One consequence worth stating plainly so it does not read as a bug: at the end
of this project **no node can become green through the product**. The
comparison that yields green is implemented and unit-tested, and you can see it
by hand-writing a granted hash into the manifest, but the only thing authorised
to write that hash for real is the AI pass in the next brief. A pacted node
that has never been judged is stale, which is the design doc's rule, not a
shortcut.

## Scope

### 1. The `.warlock/` manifest

Decide the schema, then implement loading and saving it. One file per
repository, `.warlock/pacts.toml`, human-readable and diff-friendly because it
lands in git and section 9 promises people can hand-edit their way out of
trouble.

One file rather than one per module, deliberately: it is a single read and a
single atomic write, it can be opened and understood as a document, and
file-per-module needs a path-to-filename encoding that is its own small source
of bugs. The known cost is merge conflicts — two branches pacting different
modules, or refreshing different modules once granted hashes start moving, edit
the same region of the same file and git will ask, and a conflict between two
opaque hashes can only be resolved by re-running a refresh. Accepted knowingly.

It carries a schema version, and one entry per pacted module holding the
module's path, the path of its README, and — optionally — the subtree hash that
was granted and when. That optionality is the whole design: an entry with no
granted hash is a module that is pacted but has never been judged, and the
design doc is explicit that unjudged *is* stale. Make the absence
representable rather than encoding it as a sentinel hash or a boolean beside
one.

Paths are stored relative to the manifest's own directory, so a clone at a
different absolute path reads identically. Round-trip tests, an unknown-version
file rejected with a clear error rather than silently misparsed, and a
malformed file that reports which entry broke. Saving is atomic — write beside
and rename — because a truncated manifest loses every pact in the repo.

No hashing here, and no walking. This slice is a file format and the two
functions that move it in and out of memory.

### 2. Walking a repository into a tree

depends_on: [1]

Give the engine a loader that turns a directory on disk into the `Tree` the
renderer already knows how to draw, replacing `stub_tree` as the source of
truth.

A directory is a module node when it directly contains a `README.md`. A
directory without one is also a node when it lies between the root and a module
— when it has a module somewhere beneath it — so that `crates/` appears and the
nesting reflects the repository rather than collapsing two crates into siblings
of the root. That is section 5's "pacted nodes plus their unpacted
surroundings", and it keeps empty directories that lead nowhere out of the
picture. The directory the walk was rooted at is always a node whether or not
it has a README.

README-less nodes are therefore structural, not a single special case at the
root: connectors have none, and modular invocation means launching from
`crates/` makes one the root. So `Node.readme` becomes an `Option<PathBuf>`
rather than a path pointing at a file that was never there. That is the same
call slice 1 makes about the granted hash, for the same reason — absence is
represented, not encoded as a sentinel — and it is why brief 01 gave
`NodeState` exactly three variants. It costs little: nothing in
`crates/warlock-tui` reads `readme` today, so the change is engine-internal.
Note that `Some(path)` still does not promise the file exists; a README can be
deleted between load and hash, and that is a different situation from a node
that declares none.

Use `ignore` so `.gitignore`, `.git/` and `target/` are respected without a
hand-maintained skip list, and exclude `.warlock/` itself from everything — a
directory that changes every time you pact something cannot be part of what
pacting is measured against.

Root resolution implements the design doc's modular invocation: walk up from
the working directory to the nearest ancestor containing `.git/`, take that as
the repository root, load the one manifest at `.warlock/pacts.toml` beside it,
but root the rendered tree at the working directory. Running from
`crates/warlock-engine` shows that module and its children; running from the
repo root shows everything.

Anchor on `.git/` rather than on `.warlock/`, which would be circular: in a
repository nobody has pacted anything in yet there is no `.warlock/` to find,
and that is precisely the state the tool has to start from. Git is already a
hard assumption here — the manifest is committed, ignore semantics come from
`.gitignore`, and the pre-push guard is coming — so depending on it for root
resolution costs nothing that was not already spent. A working directory with
no `.git/` anywhere above it is the one genuine failure, and slice 4 reports it
as such.

State comes from the manifest by presence alone at this point — an entry means
stale, no entry means unpacted. Slice 3 is what lets an entry mean fresh.
Fixture-based tests in a temporary directory: nesting, ignore behaviour, a
repository with no manifest at all coming back entirely gray, and a
subdirectory invocation finding the manifest above it.

### 3. Subtree hashing and the staleness decision

depends_on: [1, 2]

Hash the content at and below a node, deterministically, and compare it to what
the manifest recorded.

Deterministic means the inputs are the sorted relative paths and the file bytes
and nothing else — no mtimes, no permissions, no inode numbers, no absolute
paths, no iteration order from the filesystem. Feed the path into the hash
alongside the contents so that renaming a file is a change. A node's hash
covers its children's files too, so an edit in a leaf breaks that leaf and
every pacted ancestor, which is exactly what "files at or below this node's
README have changed" means.

The README is hashed with everything else. A hand-edited README breaks the hash
and goes yellow, and that is the correct reading of section 9: edits outside
the garden are reconciled on the way back in, not fought.

Only pacted subtrees need hashing — an unpacted node's colour does not depend on
one — so do not compute what nothing will read. This is what keeps launch
instant on a repo where Warlock manages three directories out of two hundred.

Then the decision itself, which is small and should be a pure function over a
manifest entry and a computed hash: no entry is unpacted, an entry with no
granted hash is stale, an entry whose granted hash differs is stale, an entry
whose granted hash matches is fresh. Test all four, including the fresh case,
against fixtures.

A file that cannot be read — permissions, or it vanishes mid-walk — makes its
node stale. Not an error that abandons the load, and not a colour of its own:
`NodeState` has three variants on purpose and section 6 says unjudged *is*
stale, so an "undetermined" node would be a fourth state entering through the
error path. The rule it follows from is that green is earned. A subtree Warlock
could not read in full is a subtree no AI has seen in full, and it does not get
to claim its documentation was confirmed.

Do not absorb the failure into the hash instead. A file that contributes
nothing when unreadable hashes identically to one that was deleted, so a
subtree granted while that file was absent could come back green on a run where
it merely could not be opened — a false green, the one outcome the design says
must be earned. Hashing the error text instead makes the hash
platform-dependent and breaks the identical-hash-from-a-fresh-clone criterion.

Keep unreadable and ignored strictly apart. Ignored is declared on purpose in
`.gitignore` and is not part of what documentation describes; unreadable is an
undeclared hole, and it must never be quietly promoted into the first category.
There is no per-file "skip this one" escape hatch here: if a file genuinely
must stay unreadable, the honest answers are to ignore it deliberately or to
leave that module unpacted, and both are human decisions rather than something
the loader makes on its own.

So hashing returns a `Result`, the colouring pass maps a failure to a stale node
and carries on, and one bad file never poisons the rest of the tree. It must not
be silent, though: a node stuck yellow with no stated reason is
indistinguishable from one the user edited. Return the tree alongside the
non-fatal problems encountered — path and cause — which leaves `Node`'s shape
alone and lets slice 4 decide how much of it to show.

The colouring pass belongs to this slice. Slice 2 already sets state from the
manifest by presence; this is where that same pass learns to consult hashes, and
slice 4 stays a repoint. The TUI receives a fully coloured tree and never
computes state itself, or domain logic has crossed the seam the wrong way.

### 4. The TUI on real data, and pacting from it

depends_on: [2, 3]

Point the front end at the loader instead of `stub_tree` and give it the one
write action this project has.

The pact key toggles the selected node: gray becomes a manifest entry with no
granted hash, which renders yellow on the next frame; pacted becomes gray again
and the entry is removed. The manifest is saved as it happens, and the
keybinding joins the footer alongside the movement keys. Show which directory
the tree is rooted at, since scope now depends on where the binary was invoked.

Failures here are ordinary and need to read as such rather than as crashes: a
working directory with no git repository above it, an unreadable
file mid-walk, a manifest that will not parse. The terminal restoration
guarantees from brief 01 apply unchanged — restore before reporting, on every
path out.

`stub_tree` stops being the product's data source. Keep it, demoted and
renamed if that reads better, as the fixture the TUI's `TestBackend` draw tests
build on, so those tests keep needing neither a terminal nor a filesystem.

Update the crate READMEs and the root README to describe what the tool now
does: that it reads a real tree, what makes a directory a module, where the
manifest lives and what is in it, and that green is not yet reachable.
