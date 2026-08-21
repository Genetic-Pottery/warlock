# warlock-engine

The core library crate of warlock. It owns the domain logic — the state
vocabulary, the tree of work, the record of which modules are pacted, and the
rules that move it forward. What exists today is the vocabulary, the shape of
the tree, the on-disk manifest, the subtree hash and the staleness decision it
feeds, a loader that builds a coloured tree out of a real directory, the seam a
model pass is reached through, and the pact that writes a subtree's documents
and grants the hashes that make it green:

- `NodeState`, the three-state model from section 5 of the design doc —
  unpacted, pacted-and-stale, pacted-and-fresh, with no "unknown" fourth state
  because unjudged *is* stale.
- `Node`, one node of the project tree: its `path`, the path of its document
  when it has one (`document: Option<PathBuf>`), its `state`, its `children`,
  and `files: Vec<PathBuf>` — the files sitting directly in that directory, its
  own `WARLOCK.md` among them, sorted by path. See [files are a listing, not
  children](#files-are-a-listing-not-children) below.
- `Tree`, which owns the root node and can be walked, tallied and searched:
  - `Tree::walk` — a depth-first iterator (`DepthFirst`) yielding every node
    with its depth, parents before children, siblings in stored order. The
    depth comes out of the walk because the renderer indents by it.
  - `Tree::counts` — a `StateCounts` with one field per state, so a state can
    neither be missed nor invented and an absent state counts zero.
  - `Tree::find` — the node at a given path, or `None`. Paths are compared as
    stored: no normalisation, no filesystem.
- `Manifest` and `PactEntry`, the in-memory form of `.warlock/pacts.toml` —
  the committed record of which modules are pacted — with `Manifest::save` and
  `Manifest::load` to move it on and off disk, and `ManifestError` for
  everything that can go wrong doing so. See [the manifest
  section](#the-manifest-warlockpactstoml) below.
- `subtree_hash`, the one digest over everything at and below a directory —
  the mechanical trigger a pact is compared against — with `HashError` for
  everything that can stop it. See [the subtree
  hash](#the-subtree-hash-subtree_hash) below.
- `decide_state`, the pure function from a manifest entry (or its absence) plus
  a computed hash to a `NodeState`. No filesystem, no clock, no I/O. See [the
  decision rule](#the-decision-rule-decide_state) below.
- `load_tree`, which turns a working directory into a `Tree` coloured by the
  manifest above it — returning `Loaded { tree, problems }`, with
  `repository_root` for the upward walk that finds that manifest, `LoadError`
  for everything that can stop the load and `LoadProblem` for everything that
  merely spoiled one node of it. See [the loader](#the-loader-load_tree) below.
- `Agent`, the port one model pass runs through — with `AgentRequest`,
  `AgentResponse` and `AgentError` — which this crate defines and never
  implements: running a model means running the `claude` CLI, and that
  subprocess belongs to the binary on the far side of the seam.
- `pact_subtree`, the operation a keystroke runs: write a `WARLOCK.md` for every
  directory at and below the selected one, children first, then hash each of
  them and grant it the hash just computed — with `pact_directory` for one
  directory, `gather_request` for the context one pass is scoped to,
  `unpact_subtree` for the reverse, and `PactObserver` / `Pacting` for saying
  where a pact has got to and stopping it. See [a completed pact grants
  freshness](#a-completed-pact-grants-freshness) below for what that means for
  green.

There is no hard-coded tree behind any of this. Every `Tree` is either built by
a caller node by node with `Node::new` / `Tree::new`, or loaded from a real
directory by `load_tree` — and what `load_tree` loads is decided by three rules:

- **Every directory the walk reaches is a node**, the working directory
  included. Nothing is pruned for being undocumented.
- **A directory that directly contains a `WARLOCK.md` is a module node**, and
  that path becomes its `document`. That is the whole test — no document is
  parsed. A directory with no document of its own is a node with
  `document: None`: an ordinary directory that has no documentation yet.
- **A node's state comes from the manifest entry plus the subtree hash.** No
  entry means `Unpacted` and no hashing at all; an entry means the node's
  directory is hashed and `decide_state` compares that hash against the granted
  one. Nothing is coloured by a literal.

`Node` and `Tree` are pure shape. Their fields are public so a renderer can
walk them with each node's depth and state in hand, and a caller builds them
directly with `Node::new` / `Tree::new`. A node's state is a plain stored
field: nothing in the tree types computes staleness. Computing it is the job of
`subtree_hash` and `decide_state`, and the loader is what puts the two
together.

`Node` and `Tree` derive serde's `Serialize`/`Deserialize` so a caller can
choose a format, and they still commit to none: the derives are tested by
round-tripping through serde's own token stream (`serde_test`, a
dev-dependency), which never names a format.

The manifest is the one place the crate commits to a format: it is TOML, it
lives at `.warlock/pacts.toml`, and `Manifest::save` / `Manifest::load` take
the repository root and read and write `<root>/.warlock/pacts.toml` under it —
the path `manifest_path` spells out.

The crate reaches the filesystem in four ways and no others. It reads and
writes that manifest; it *walks* directories — via the `ignore` crate, so
`.gitignore` at every level is respected, hidden directories such as `.git/` are
skipped and a `target/` the repository ignores never appears, all without a
hand-maintained list and never following a symlink; it *reads the bytes* of
the files under a pacted directory, in order to hash them or to put them in a
request; and it *writes a `WARLOCK.md`* for a directory a pact covered, verbatim
from what came back, through the same write-beside-and-rename its manifest goes
through. It reads those bytes and does not interpret them: no document is
parsed, and the only thing a hash ever gives back is a digest. That is the whole
capability boundary: it still depends on no terminal crate, opens no sockets,
spawns no subprocesses — the `claude` child belongs to whoever implements
`Agent`, which is never this crate — and contains no `unsafe`.

## Files are a listing, not children

`Node::files` exists so a renderer can show what is inside a directory. It is a
listing and nothing more, and every consequence of that is deliberate:

- **A file is not a node.** It has no state, no document and no children of its
  own, and nothing gives it any. `Node::new` starts a node with an empty list
  and `Node::with_files` attaches one, the same way `with_children` attaches
  children.
- **Files cannot unmake a leaf.** `Node::is_leaf` asks about child *nodes*
  only, so a directory holding files and no subdirectories is still a leaf.
- **Files are absent from `Tree::walk` and from `StateCounts`.** The walk
  yields nodes with their depth; a file is never one of them, and `Tree::counts`
  tallies states, which files do not have. Adding files to a tree changes
  neither the walked sequence nor the counts — a test asserts exactly that by
  holding the two trees side by side.
- **No hash reads the list.** `subtree_hash` walks the filesystem itself, so a
  node's digest is the same whether or not anything ever filled in its `files`.
  The listing is a view's input, never the trigger's.

A loaded node lists what the walk saw directly inside the directory, its own
`WARLOCK.md` included: a faithful listing rather than a listing minus one
special name, and a front end that would rather not draw the document twice
leaves it out on the way to the screen. Subdirectories are not in the list;
they are `children`. The order is the loader's doing, not the type's:
`with_files` stores what it is given, exactly as `with_children` does, and the
loader is what hands the paths over sorted.

## The manifest: `.warlock/pacts.toml`

A pact pressed in the TUI has to survive a relaunch, so it is written down.
There is **exactly one manifest per repository**, at `.warlock/pacts.toml`
under the repository root, and it is **committed to git** — a pact is a fact
about the repository, not about one developer's checkout, so `.warlock` must
never be added to `.gitignore`. One file rather than one file per module means
a single read and a single atomic write, and a file that can be read as a
document. The accepted cost is merge conflicts when two branches pact
different modules; a conflict between two opaque hashes is resolved by pacting
the module again.

It is TOML because of section 9 of the design doc: the escape hatch. Whatever
Warlock records has to be something a human can open, read, diff and hand-edit
their way out of.

```toml
version = 1

[[pact]]
module = "crates/warlock-engine"
document = "crates/warlock-engine/WARLOCK.md"
granted_hash = "9f2b1c…"
granted_at = "2026-08-19T14:03:11Z"

[[pact]]
module = "crates/warlock-tui"
document = "crates/warlock-tui/WARLOCK.md"
```

### The keys

| Key | Where | Meaning |
| --- | --- | --- |
| `version` | top level, integer, required | The schema version. This build reads and writes `1` (`SCHEMA_VERSION`). |
| `[[pact]]` | top level, array of tables | One table per pacted module, in file order. Omitted entirely when nothing is pacted. |
| `module` | in a `[[pact]]`, string, required | The pacted directory, relative to the manifest's directory. |
| `document` | in a `[[pact]]`, string, required | The document describing that module — held separately because the file name is not Warlock's to assume. |
| `granted_hash` | in a `[[pact]]`, string, optional | The subtree hash captured when freshness was last granted — what [`subtree_hash`](#the-subtree-hash-subtree_hash) returns. Opaque to the manifest itself, which stores and compares it as a string and never computes one. |
| `granted_at` | in a `[[pact]]`, string, optional | When that grant happened, as an RFC 3339 timestamp. A plain string, so no date/time crate is needed to read a field nothing here does arithmetic on. |

The version is checked **before** any entry is looked at, and a manifest whose
`version` is anything other than the supported one is rejected outright with
`ManifestError::UnsupportedVersion`, naming both the version found and the one
supported. It is never read as if it were the current schema: an old binary
guessing at a newer file is how a manifest gets silently rewritten and loses
data. Unknown keys are rejected too, at the top level and inside an entry.

### Paths are relative, with forward slashes

Every path in the manifest is stored relative to the manifest's own directory
— the parent of `.warlock/`, i.e. the repository root — with `/` as the
separator and `"."` for the root itself. That is what makes two clones of the
same commit, at different absolute paths and on different operating systems,
produce **byte-identical** manifests; a test asserts it by saving the same
logical manifest under two different temporary roots and comparing the file
bytes. `PactEntry::new` normalises a caller's path on the way in (rejecting
anything outside the root, or not valid UTF-8), `to_manifest_path` and
`from_manifest_path` do the conversion on their own, and
`PactEntry::module_path` / `document_path` give a real path back under a root.

### No granted hash means never judged

An entry that has never been judged is represented by `granted_hash` simply
being **absent** — not a sentinel hash, not an empty string, not a
`judged = false` boolean. It deserialises to `None`, `PactEntry::is_judged`
returns `false`, and a round-trip re-serialises with the key still omitted, so
hand-written files stay as their author wrote them. Absent means
pacted-but-never-judged, which is `NodeState::PactedStale` — the three-state
model has no fourth state, because unjudged *is* stale.

### Saving is atomic; missing is not corrupt

`Manifest::save` serialises first, creates `.warlock/` if it is not there,
writes the text to a temporary file **in that same directory** (so the rename
cannot cross a filesystem), syncs it, and renames it over `pacts.toml`. A
reader therefore sees either the whole old manifest or the whole new one,
never a half-written file, even if the process dies mid-save. No temporary
file survives a successful save — a test asserts the directory afterwards
holds exactly `pacts.toml` — and a failed one cleans up on a best-effort
basis.

`Manifest::load` distinguishes a manifest that is not there from one that
cannot be understood: a missing file is `ManifestError::NotFound` rather than
an empty manifest, because "this repository has never pacted anything" and
"this repository pacted nothing" are different facts and only the caller knows
which to act on. A caller happy to treat the first as the second matches on
`NotFound` and falls back to `Manifest::new()`. A malformed entry gives
`ManifestError::Entry`, naming the entry by index and by module path so the
error says which `[[pact]]` to go and fix.

`ManifestError` is a hand-rolled enum implementing `Display` and
`std::error::Error`: no error-handling dependency, because it would buy
nothing over the impls, and the variants are the vocabulary a front end shows
a user.

### What the manifest is not

The manifest stores hashes; it does not compute them or compare them to
anything, and it sets no `NodeState` — computing is `subtree_hash`'s job and
comparing is `decide_state`'s. It is read from and written to a root the caller
supplies: finding that root is the loader's job (`repository_root`), not the
manifest's. There is no directory scan to discover modules, no file watching,
no locking protocol and no migration tooling beyond rejecting versions it does
not know. Nor does it **decide** a `granted_hash`: `Manifest::save` writes
whatever grant an entry carries, and the one thing that ever puts one there is a
finished subtree pact — see [a completed pact grants
freshness](#a-completed-pact-grants-freshness).

## The subtree hash: `subtree_hash`

Section 6 of the design doc splits the job in two: the hash is the trigger, an
AI is the judge. `subtree_hash(dir)` is the whole of the trigger — one digest,
as lowercase hex, over everything at and below a directory, to be compared for
equality against the `granted_hash` a pact recorded.

### What goes in

For each file, in order, exactly two things:

1. **its path relative to the hashed directory**, with forward slashes — the
   same spelling the manifest stores paths in, produced by the same
   `to_manifest_path`;
2. **that file's bytes**, whole.

Both are length-prefixed, so no rearrangement of names and contents can be
mistaken for another: a directory `a/b` holding `c` and a file `a` holding `bc`
are different inputs and get different digests. Files are fed in **sorted by
that relative path**, not in the order the walk produced them. The whole digest
is domain-separated through blake3's key derivation with a versioned context
string, so it can never collide with a plain `blake3` of the same bytes taken
for some other purpose, and so a future change to what goes in can announce
itself instead of silently invalidating every recorded grant.

That is the entire input. Nothing else is in it, and the exclusions are
deliberate rather than an oversight:

| Excluded | Why |
| --- | --- |
| **mtimes** and every other timestamp | A fresh clone rewrites them all. A hash that moved when a file was touched would call a subtree stale for being checked out. |
| **Permission bits** and ownership | Umask and platform differences, not content. |
| **Inode and device numbers** | Per-filesystem accidents; identical on no two clones. |
| **Absolute paths** | The digest has to be the same in `/home/ada/warlock` and `/build/ci/42`. Only the path *relative to the hashed node* goes in — which is also what makes a node's hash independent of where it sits in the repository. |
| **Filesystem iteration order** | Whatever order the walker produced is thrown away by sorting, so two machines that enumerate a directory differently still agree. |
| **Directories themselves** | Only files contribute, so an empty directory is invisible to the hash — there is nothing in it to be out of date about. |
| **Symlinks** | Never followed and never hashed as their target: a link inside the subtree already has its target hashed, and a link out of it is not the subtree's content. |

The hash covers the node's own `WARLOCK.md` and every file in every descendant
directory. So editing any file at or below the node — the document included, by
hand, which section 9 says is correct behaviour to be reconciled rather than
fought — changes that node's hash and every ancestor's. Adding a file, deleting
one, or renaming one (same bytes, new relative path) changes it too.

### Ignore rules apply, and `.warlock/` never does

Traversal uses the same rules as [the loader's walk](#what-the-walk-skips) and
for the same reasons: the `ignore` crate, so `.gitignore` at every level,
hidden entries and global excludes are honoured as git honours them; symlinks
are not followed, so a symlinked directory cycle terminates instead of hanging.
`.warlock/` is pruned by name on top of that — Warlock's own bookkeeping is not
content of the module, and a manifest that changed the hash of the thing it
records would never settle.

So editing a file inside a gitignored directory (`target/`, a build artefact, a
local scratch file) changes no node's hash. That is the point: what a
repository declares ignored is not what its documents describe.

### An unreadable file is an error, never a skip

If a file the walk found cannot be read, `subtree_hash` returns
`HashError::Read` naming that file, and **no hash at all**. A partial digest is
never returned, and neither the error text nor the bytes that were readable go
into one.

This is a decision, not an implementation detail, and it should survive being
"simplified":

- **A skipped file hashes exactly like a deleted one.** If an unreadable file
  simply contributed nothing, a subtree that was granted at a time when that
  file did not exist would come back *fresh* on a run where the file is merely
  unopenable. That is a false green — the one outcome the design says has to be
  earned.
- **Hashing the error text instead would make the digest platform-dependent.**
  The operating system's wording for a permission failure is not stable across
  systems or locales, and the fresh-clone property — same commit, same digest,
  any machine — would break.

Ignored and unreadable are therefore kept strictly apart. Ignored is something
the repository declared on purpose in a `.gitignore`; unreadable is an
undeclared hole, and a hole gets reported. There is no per-file skip hatch and
no configuration to turn one on.

## The decision rule: `decide_state`

`decide_state(entry, computed_hash)` is the pure function from a manifest entry
— or its absence — plus a hash to a colour. No filesystem, no clock, no I/O,
total and infallible. Four cases, and they are the whole rule:

| Manifest entry | `granted_hash` | Verdict |
| --- | --- | --- |
| absent | — | `NodeState::Unpacted` |
| present | absent | `NodeState::PactedStale` |
| present | differs from the computed hash | `NodeState::PactedStale` |
| present | equals the computed hash | `NodeState::PactedFresh` |

The two stale rows collapse into one idea worth saying out loud: **never judged
and judged-against-something-else are the same answer.** Section 5 leaves no
room for a fourth "unknown" colour, because unjudged *is* stale. Staleness is
mechanical and needs nobody's opinion; freshness has to be granted. Hence the
asymmetry — every path through this function returns stale except the single
one where a recorded hash equals a computed one, compared as plain strings.

### A completed pact grants freshness

`decide_state` compares; it grants nothing. Granting happens in exactly one
place — `pact_subtree`, in its second phase, once every document the pact was
going to write is on disk: each directory it covered is hashed, and the entry
built for it is granted the hash just computed. That is the only code in this
workspace that writes a `granted_hash`, and it is what makes
`NodeState::PactedFresh` reachable through the product rather than only by hand.
A directory whose own document failed gets no entry at all, and its ancestors
inside the pact get an entry with no grant, so nothing goes green that a pass
did not actually describe.

What is still missing is the other direction: **nothing re-grants a node that
has gone stale.** Edit a file at or below a pacted directory and its hash moves,
so it and every ancestor go yellow — and the only way back to green today is to
pact the subtree again from scratch. A refresh pass that judges what changed and
re-grants only what deserves it is the next project, and no part of it is in
this crate yet.

The crate's own tests still reach the fresh case the other way, by writing a
granted hash into an entry by hand, because what they are exercising is the
comparison and not where either side of it came from.

## The loader: `load_tree`

`load_tree(working_dir)` returns the coloured `Tree` for a directory on disk,
or a `LoadError` saying why it could not.

### What makes a node

**Every directory the walk reaches is a node**, the directory the walk is
rooted at included. The loader drops nothing for being undocumented, so the
tree is the shape of the working directory rather than an opinion about which
parts of it are worth seeing.

**A directory is a module node when it directly contains a `WARLOCK.md`**, and
that path becomes its `document: Some(...)`. That is the whole test. Warlock
never parses a document — not its headings, not its length, not a word of it. It
cares only that one exists, because the design doc makes the tree of module
documents the interface, and a `WARLOCK.md` is a module's claim to be one. No
other name is special: a `README.md` beside it is the project's file, written
for people, and Warlock treats it as an ordinary file — it documents nothing
here, though its bytes go into the subtree hash like any other file's.

A directory with no document of its own is a node with `document: None` — an
ordinary directory that has no documentation yet, such as `crates/` or any
`src/` in this repository. It is not a lesser kind of node, and nothing about
it is missing except a document somebody has yet to write. A front end that
wants to show only the documented ones filters what it renders; that is a
view's decision, taken on the way to the screen, not the loader's.

**The files sitting directly in a directory come back on its node**, in path
order, as [`files`](#files-are-a-listing-not-children). They make no difference
to the node's children, its state or its document; the loader copies them
across and nothing else consults them.

### Where the root comes from

Section 12's modular invocation rule says the scope of a run is wherever it was
invoked, with no privileged root. So the two roots are deliberately different
things:

- The **repository root** is the nearest ancestor of the working directory that
  holds a `.git/` directory, found by walking up (`repository_root`). The
  single manifest is read from `<root>/.warlock/pacts.toml`, exactly where it
  has always lived — the anchor moved, the manifest did not. `.git/` is the
  anchor because a repository nobody has pacted yet is the normal way to meet
  Warlock: it opens as a tree of unpacted modules, and `.warlock/` appears
  under that same root the first time something is pacted. The check is a
  filesystem test for a directory named `.git`, so a checkout where `.git` is
  a *file* — a worktree, a submodule — is not a root here.
- The **tree root** is the working directory itself. Launch from
  `crates/warlock-engine` in this repository and `Tree::root_path()` is
  `crates/warlock-engine`, with node states taken from the manifest two levels
  above it.

A working directory with no `.git/` anywhere above it is not a repository, and
that is the only way resolving the root fails: `LoadError::NoRepositoryRoot`,
whose `Display` names `.git` and the directory the search started from. A
missing `.warlock/` is not a failure at all — it loads as an empty manifest.
It is an error value, never a panic.

### What the walk skips

Traversal is the `ignore` crate, so `.gitignore` at every level, hidden
directories (`.git/` among them) and global excludes are honoured as git
honours them — there is no hand-maintained skip list to drift out of date, and
`target/` disappears because the repository already ignores it. `.warlock/` is
pruned unconditionally on top of that, even if someone puts a `WARLOCK.md` in
it. Symlinks are never followed, so a symlinked directory cycle terminates
instead of hanging. Siblings come out ordered by directory name, so loading an
unchanged tree twice gives two `Tree` values that compare equal.

Directories and files come out of **the same single pass**: the walker yields
them interleaved, and a file is filed under its parent directory as it arrives,
so there is no second walk and no `read_dir`. A file therefore obeys exactly the
rules a directory does — a gitignored, hidden or `.warlock/` file is absent for
the same reason a gitignored, hidden or `.warlock/` directory is — and is
ordered the same way, by name, with each node's list sorted before the tree is
built. (The one asymmetry: a node's `document` is a direct filesystem check for
`WARLOCK.md`, so an ignore rule covering a document still leaves it documenting
its node while keeping it out of the listing.)

### Colouring goes through the hash

Presence in the manifest no longer decides a colour; it only decides whether
the question is worth asking. For each node the loader looks the path up in the
manifest, and:

- **no entry** — the node is `NodeState::Unpacted` and **is never hashed**.
  Directories nobody has pacted cost a load nothing but a directory entry.
- **an entry** — the node's own directory is hashed with `subtree_hash`, and
  the colour is whatever `decide_state` makes of (entry, hash), by [the
  four-case rule](#the-decision-rule-decide_state) above.

So a granted hash — one a pact recorded, or one somebody wrote by hand — that
still matches what is on disk loads as `NodeState::PactedFresh`, and a directory
edited since its pact loads yellow. A repository whose `.warlock/` holds no
manifest, or an empty one, loads with every node `Unpacted`; a manifest that
exists but cannot be understood is an error, not a silent empty one.

### The return shape: a tree and a list of problems

`load_tree` returns `Loaded { tree, problems }`, not a bare `Tree`. They are
answers to different questions with different lifetimes — the tree is the thing
to render, the problems are the thing to report once — so they are a plain pair
rather than problems hung off the tree. **`Node`'s fields are unchanged**: a
node that could not be hashed is stale like any other stale node, and a
renderer needs to know nothing more.

`problems` is a `Vec<LoadProblem>`, each carrying the `path` of the node it
happened at and the `cause` (a `HashError`, which names the offending *file* —
usually somewhere below that node). They come in the order the walk met them:
children before parents, siblings in name order. On a healthy repository the
list is empty.

Everything in that list is **non-fatal by definition** — the load that produced
it finished. The fatal cases stay in `LoadError` and are only these four: no
repository root, the working directory cannot be made absolute, a manifest that
is there but unreadable or unparseable, and a directory tree that cannot be
walked.

### An unreadable file makes one node stale, and says so

Hashing can fail, and [it is meant to](#an-unreadable-file-is-an-error-never-a-skip)
rather than skipping the file. One such file is one node's problem, not the
tree's, so the loader:

1. colours that node `NodeState::PactedStale` — its content is unknown, and
   unknown is stale;
2. pushes a `LoadProblem` naming the node and the cause;
3. carries on, colouring every other node correctly.

It is never silently ignored. A silent skip is the failure mode being designed
out: it would hash like a deletion and could hand back a green nobody earned.
No error text and no partial read ever reaches a digest — the failed hash
simply does not exist, and the node falls back to stale without one.

How much of `problems` a front end shows a user is the front end's call. A
caller that ignores the list entirely still gets a safe tree; it is just an
unexplained one.

## The dependency edge runs one way

`warlock-engine` never depends on the TUI, and never depends on any terminal
crate — no `warlock-tui`, no Ratatui, no Crossterm, no terminal or rendering
library of any kind. The dependency edge runs **TUI → engine, and never back**.

This is an architectural boundary, not a convention. The engine must stay
usable without a terminal attached, so that another front end (or a test
harness) can drive it directly. Any change that would make the engine reach
back toward the TUI is a change to the architecture and should be treated as
one.

Hashing added exactly one dependency, `blake3` — a fast, well-specified hash
whose digest of the same bytes is the same on any machine and any build, which
is what a grant recorded in a committed manifest needs. It is declared in
`[workspace.dependencies]` with that justification and used by `warlock-engine`
alone. The rest of the engine's dependencies are unchanged: `serde` and `toml`
for the manifest, `ignore` for the walk, `serde_test` and `tempfile` for tests.
