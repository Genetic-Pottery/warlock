# warlock-engine

The core library crate of warlock. It owns the domain logic — the state
vocabulary, the tree of work, the record of which modules are pacted, and the
rules that move it forward. What exists today is the vocabulary, the shape of
the tree, the on-disk manifest, and a loader that builds a tree out of a real
directory:

- `NodeState`, the three-state model from section 5 of the design doc —
  unpacted, pacted-and-stale, pacted-and-fresh, with no "unknown" fourth state
  because unjudged *is* stale.
- `Node`, one node of the project tree: its path, the path of its README when
  it has one (`readme: Option<PathBuf>`), its state, and its children.
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
- `load_tree`, which turns a working directory into a `Tree` coloured by the
  manifest above it, with `repository_root` for the upward walk that finds that
  manifest and `LoadError` for everything that can stop either. See [the
  loader](#the-loader-load_tree) below.
- `stub_tree`, one small tree written out by hand — three levels deep, with at
  least one node in each state — so the engine/TUI seam can be exercised
  without a repository behind it.

`Node` and `Tree` are pure shape. Their fields are public so a renderer can
walk them with each node's depth and state in hand, and a caller builds them
directly with `Node::new` / `Tree::new`. A node's state is a plain stored
field: nothing in the tree types computes staleness, and nothing anywhere in
this crate computes it yet — the loader stores what the manifest says and no
more.

`Node` and `Tree` derive serde's `Serialize`/`Deserialize` so a caller can
choose a format, and they still commit to none: the derives are tested by
round-tripping through serde's own token stream (`serde_test`, a
dev-dependency), which never names a format.

The manifest is the one place the crate commits to a format: it is TOML, it
lives at `.warlock/pacts.toml`, and `Manifest::save` / `Manifest::load` read
and write exactly the path they are given.

The crate now reaches the filesystem in two ways and no others. It reads and
writes that manifest, and it *walks* directories — via the `ignore` crate, so
`.gitignore` at every level is respected and `.git/` and `target/` are skipped
without a hand-maintained list, never following a symlink. That is the whole
capability boundary: it still depends on no terminal crate, opens no sockets,
spawns no subprocesses and contains no `unsafe`.

## The manifest: `.warlock/pacts.toml`

A pact pressed in the TUI has to survive a relaunch, so it is written down.
There is **exactly one manifest per repository**, at `.warlock/pacts.toml`
under the repository root, and it is **committed to git** — a pact is a fact
about the repository, not about one developer's checkout, so `.warlock` must
never be added to `.gitignore`. One file rather than one file per module means
a single read and a single atomic write, and a file that can be read as a
document. The accepted cost is merge conflicts when two branches pact or
refresh different modules; a conflict between two opaque hashes is resolved by
re-running a refresh.

It is TOML because of section 9 of the design doc: the escape hatch. Whatever
Warlock records has to be something a human can open, read, diff and hand-edit
their way out of.

```toml
version = 1

[[pact]]
module = "crates/warlock-engine"
readme = "crates/warlock-engine/README.md"
granted_hash = "9f2b1c…"
granted_at = "2026-08-19T14:03:11Z"

[[pact]]
module = "crates/warlock-tui"
readme = "crates/warlock-tui/README.md"
```

### The keys

| Key | Where | Meaning |
| --- | --- | --- |
| `version` | top level, integer, required | The schema version. This build reads and writes `1` (`SCHEMA_VERSION`). |
| `[[pact]]` | top level, array of tables | One table per pacted module, in file order. Omitted entirely when nothing is pacted. |
| `module` | in a `[[pact]]`, string, required | The pacted directory, relative to the manifest's directory. |
| `readme` | in a `[[pact]]`, string, required | The README documenting that module — held separately because the file name is not Warlock's to assume. |
| `granted_hash` | in a `[[pact]]`, string, optional | The subtree hash captured when freshness was last granted. Opaque to this crate: nothing here computes or verifies it. |
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
`PactEntry::module_path` / `readme_path` give a real path back under a root.

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

The manifest stores hashes; it does not compute or compare them, and nothing
here sets `NodeState::PactedFresh`. It is read from and written to a root the
caller supplies: finding that root is the loader's job (`repository_root`), not
the manifest's. There is no directory scan to discover modules, no file
watching, no locking protocol and no migration tooling beyond rejecting
versions it does not know.

## The loader: `load_tree`

`load_tree(working_dir)` returns the `Tree` for a directory on disk, or a
`LoadError` saying why it could not.

### What makes a node

**A directory is a module node when it directly contains a `README.md`**, and
that path becomes its `readme: Some(...)`. That is the whole test. Warlock
never parses a README — not its headings, not its length, not a word of it. It
cares only that one exists, because the design doc makes the tree of module
READMEs the interface, and a README is a module's claim to be one.

A directory with no README of its own is kept only as a **connector**, with
`readme: None`, and only when a module node sits somewhere below it — `crates/`
in this repository is one. A README-less directory with no module-node
descendant is **dropped entirely**: it is neither a module nor on the way to
one, so putting it in the tree would be noise.

The directory the walk is rooted at is always a node, README or not.

### Where the root comes from

Section 12's modular invocation rule says the scope of a run is wherever it was
invoked, with no privileged root. So the two roots are deliberately different
things:

- The **repository root** is the nearest ancestor of the working directory that
  holds a `.warlock/` directory, found by walking up (`repository_root`). The
  single manifest is read from there.
- The **tree root** is the working directory itself. Launch from
  `crates/warlock-engine` in this repository and `Tree::root_path()` is
  `crates/warlock-engine`, with node states taken from the manifest two levels
  above it.

A working directory with no `.warlock/` anywhere above it is not a repository:
that is `LoadError::NoRepositoryRoot`, whose `Display` names the condition and
the directory the search started from. It is an error value, never a panic.

### What the walk skips

Traversal is the `ignore` crate, so `.gitignore` at every level, hidden
directories (`.git/` among them) and global excludes are honoured as git
honours them — there is no hand-maintained skip list to drift out of date, and
`target/` disappears because the repository already ignores it. `.warlock/` is
pruned unconditionally on top of that, even if someone puts a `README.md` in
it. Symlinks are never followed, so a symlinked directory cycle terminates
instead of hanging. Siblings come out ordered by directory name, so loading an
unchanged tree twice gives two `Tree` values that compare equal.

### State is presence, and nothing can be fresh

A node's state is decided by one question: does the manifest hold an entry for
its path? If it does, the node is `NodeState::PactedStale`. If it does not, the
node is `NodeState::Unpacted`. **The loader cannot produce
`NodeState::PactedFresh`** — freshness needs a subtree hash compared against a
granted one, and this crate computes no hashes. A repository whose `.warlock/`
holds no manifest, or an empty one, loads with every node `Unpacted`; a
manifest that exists but cannot be understood is an error, not a silent empty
one.

## `stub_tree` survives, but is not the source of truth

`load_tree` is where a real tree comes from. `stub_tree` walks no directory,
opens no file and computes no staleness: every path and every state it returns
is a literal typed into `src/stub.rs`, chosen only to give a renderer something
with more than one level of nesting and one node of each colour — including a
fresh one the loader cannot yet produce. It stays because a front end or a test
harness benefits from a fixed tree with no filesystem behind it. Nothing about
a repository can be inferred from it.

## The dependency edge runs one way

`warlock-engine` never depends on the TUI, and never depends on any terminal
crate — no `warlock-tui`, no Ratatui, no Crossterm, no terminal or rendering
library of any kind. The dependency edge runs **TUI → engine, and never back**.

This is an architectural boundary, not a convention. The engine must stay
usable without a terminal attached, so that another front end (or a test
harness) can drive it directly. Any change that would make the engine reach
back toward the TUI is a change to the architecture and should be treated as
one.
