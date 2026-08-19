# Red brief: Warlock navigation and scope

Make the tree show the whole repository and give it the navigation that implies.
Every non-ignored directory renders, gray until pacted; the view is scrollable,
collapsible, filterable, and can show files coloured by the module holding them.
No AI, no `claude`, no Linear, and nothing about how freshness is computed
changes.

Read `docs/warlock-design-doc.md` before drafting, sections 5 and 8 in
particular. **Both were rewritten after briefs 01 and 02 were cut, and this
project is the course correction that follows.** Where those briefs and the
current design doc disagree, the doc wins and this brief says so explicitly.
The disagreement is narrow but load-bearing: brief 02 §2 implements section 5's
old rule, "pacted nodes plus their unpacted surroundings", as a *load-time* rule
that prunes undocumented directories out of the tree entirely. Section 5 now
renders everything and demotes that rule to a view filter.

This brief also goes ahead of the AI pass that brief 02 named as coming next.
That ordering is deliberate. The refresh pass scopes context out of the tree it
is given, and building it against a tree that is about to change shape means
building it twice.

## Outcome

`cargo run` in any git repository — pacted, partially pacted, or never touched
by Warlock — draws that repository. Every directory git would not ignore is
there, gray unless pacted. The tree scrolls, so a repo with four hundred
directories is navigable rather than truncated at the bottom of the viewport.
Space collapses and expands a directory. One key hides everything unpacted when
you want to see only the part you are working in, and another shows files under
their directories, in that directory's colour.

A first-time user in an unpacted repo sees their whole codebase in gray, which
is the cold open section 5 now promises and which the current binary cannot
produce at all.

## Success criteria

- `cargo run` in a git repository with no `.warlock/` directory anywhere renders
  the full tree in gray. Today this is an error and the binary exits; that is
  the single most important fix in this project.
- Pressing the pact key on a documented directory in that repository creates
  `.warlock/pacts.toml` and turns the node yellow, with no prior setup step.
- Running in warlock's own repo shows `assets/`, every `src/`, and every other
  non-ignored directory, not just the three that hold READMEs. `target/`,
  `.git/` and `.warlock/` stay out.
- The pact key behaves exactly as it does today on documented directories, and
  on an undocumented one says why it did nothing instead of failing silently.
- A tree taller than the terminal scrolls: moving the selection past the bottom
  edge scrolls the view, the selected row is always drawn, and the header and
  footer never move.
- Space on a directory hides its descendants and draws it as collapsed; space
  again restores them. Collapsing a directory that contains the selection moves
  the selection to that directory rather than stranding it on a hidden row.
- The pacted-only filter shows pacted nodes and the ancestors needed to reach
  them, and nothing else. Toggling it off restores the full tree with the
  selection still on the same node.
- With the file toggle on, files appear under the directory holding them, in
  that directory's colour, and cannot be selected for pacting. With it off, the
  tree is exactly what it was before.
- Launch stays imperceptible on a repository of a few thousand directories, and
  hashing still happens only for pacted subtrees.
- The gate — `cargo fmt --all --check`, `cargo clippy --workspace --all-targets
  -- -D warnings`, `cargo test --workspace` — stays clean.

## Constraints

- Everything from briefs 01 and 02 still holds: stable Rust 2024, system
  toolchain and no `rust-toolchain.toml`, Apache-2.0, no `unsafe`, no HTTP
  client, no Anthropic SDK, no mocking framework, engine headless and testable
  with no terminal. The dependency edge still runs TUI to engine and never back.
- **No new dependencies are sanctioned.** Everything here is a walk that already
  happens, a `Vec` of rows that already exists, and arithmetic on a viewport
  height. A crate that only saves a few lines is still not worth the supply
  chain.
- **`NodeState` keeps exactly three variants.** Nothing in this project adds a
  state, and files in particular do not get one — see slice 5.
- **Expansion, filtering and scroll position are view state and live in the
  TUI.** `Node` and `Tree` derive serde and are the engine's persisted
  vocabulary; whether a row is collapsed changes on a keystroke and has no
  business round-tripping through a file. The engine hands over a tree, the
  front end decides what is on screen.
- The hashing walk stays separate from the rendering walk. Hashing covers pacted
  subtrees only, includes the README, and is what colours a node; the file list
  slice 5 adds is for drawing rows and must not become an input to a hash or a
  reason to hash more than is needed.
- Warlock still does not parse READMEs, and the README section skeleton stays an
  open question.
- Pact granularity stays as section 5 specifies: one README per directory, node
  and pact and directory being the same thing. The subtree-pact alternative is
  parked in section 17 and nothing here should quietly implement it.

## Out of scope

The AI half, still and entirely: invoking `claude`, the refresh pass, prompting,
context scoping, anything that grants freshness or writes a `granted_hash`.
Green remains unreachable through the product for the same reason it was in
brief 02, and that is not a bug this project fixes.

Also out: blessing and the gate, pact expansion proposals and footprint
prediction, Linear and Red/Forman integration, the journal at `~/.warlock/`, the
pre-push guard, the CI validator, the paid crate, file watching or any daemon,
and publishing anything.

**Reading a file's contents is out of scope, deliberately.** This project puts
files on screen; it does not open them. A viewer pane is a real feature with its
own questions — syntax highlighting, large files, binary detection, how it
shares the terminal with the tree — and folding it in here would double the
project and blur what it is for. Section 5's promise that nothing is hidden from
someone who wants to read code is served next, not now. Say so in the README
rather than leaving it looking half-built.

**Pacting, in its real form, is out of scope and belongs to the AI brief.**
Section 8's pact is one action: the AI scans the module, writes its README,
grants the hash, and the node lands green, with the README as the artifact left
behind. Nothing in that sequence can happen without `claude`, so none of it
happens here.

What ships today — a key that writes a manifest entry and colours a node yellow
— is a placeholder for that operation, not a smaller version of it, and this
project should treat it the way brief 02 §4 treated `stub_tree`: keep it working
because the rendering work needs something to toggle, and do not build on it.
Two things follow. Do not extend it to undocumented directories (slice 3). And
do not let it accrete behaviour that the real operation will have to unpick,
because the AI brief replaces it rather than extending it.

Worth flagging forward: real pacting is a long-running action with a model call
inside it, where today's is a keystroke that returns before the next frame. That
is a UI problem — progress, cancellation, what the tree does while a scan is in
flight — and it is the AI brief's to solve, but nothing here should make it
harder by assuming pacting is instant.

## Scope

### 1. Root resolution, and repositories nobody has pacted

`repository_root` currently walks up looking for a `.warlock/` directory, and
`load_tree` returns `NoRepositoryRoot` when it finds none. The consequence is
that Warlock cannot open a repository until Warlock has already been used on it,
which is exactly backwards for a tool whose cold open is a gray tree.

Brief 02 §2 specified anchoring on `.git/` and gave the reason: "in a repository
nobody has pacted anything in yet there is no `.warlock/` to find, and that is
precisely the state the tool has to start from." That is the intent to restore.
The root is the nearest ancestor holding `.git/`; the manifest still lives at
`<root>/.warlock/pacts.toml` and nothing about the file's location or format
changes. Only discovery moves.

`repository_root` is public API whose doc comment and `lib.rs` re-export both
describe the old anchor, so both need rewriting rather than just the body. The
one genuine failure is now a working directory with no `.git/` above it, and it
should say that in those terms.

Two consequences to handle rather than discover later. Pacting in a repository
with no `.warlock/` must create the directory on the way to writing the
manifest, which previously could not happen because root resolution guaranteed
it already existed. And `a_directory_outside_any_repository_is_an_error_that_says_so`
now needs a temporary directory with no git repository above it, which on a
developer's machine is not the same thing as a bare `tempdir()`.

### 2. A scrolling viewport

This comes before the tree gets bigger, not after, so that no merge ever leaves
`main` silently truncating rows. It needs nothing from slice 1, which is why the
dependency chain starts here rather than there.

`ui.rs` currently draws rows starting at the top of the tree area and stops when
it runs out of area, which was correct when a tree was five nodes and is a
truncation bug the moment slice 3 lands. Give the tree area a scroll offset: the
visible window is a slice of the flattened rows, the selected row is always
inside it, and moving the selection past either edge moves the window by the
minimum needed rather than recentring.

The header and footer stay pinned. Keep the offset in the app state next to the
selection, since the two only ever change together, and keep the rule pure
enough to test with no terminal: given a row count, a viewport height and a
selection, the offset is determined. That is the test, and it wants the
off-by-one cases — selection on the first row, on the last, exactly at each
edge, and a viewport taller than the tree.

Page-up and page-down are worth the few lines here while the arithmetic is
already open, and `g`/`G` for top and bottom if it stays small.

### 3. Every directory in the tree

depends_on: [2]

Delete the pruning filter in `load.rs` — the one keeping a child only when it
has a README or a descendant that does — so every directory the walk reaches
becomes a node. The walk itself already does the right thing: `ignore` gives
gitignore semantics at every level, hidden directories stay out, `.warlock/` is
pruned by name, and symlinks are not followed. None of that changes.

What changes is what `readme: None` means. It stops being "a connector, present
only for structure" and becomes the ordinary case: a directory with no
documentation yet. The connector concept retires. Sibling ordering, depth-first
walk order and `StateCounts` all work as they are.

This slice is a few lines of deletion and a lot of prose. `load.rs`'s module
documentation, `Node::readme`'s doc comment, `Tree`'s doc comment and
`crates/warlock-engine/README.md` all currently explain the pruning rule as
following from section 5, and all of them become wrong the moment the filter
goes. `this_repository_loads_with_its_crates_and_nothing_ignored` needs to
expect `assets/` and the `src/` directories too. Leave the ignore assertions in
it exactly as they are; they are the part still worth guarding.

One behaviour change comes with it: `App::toggle_pact` reads
`row.readme.clone()?` and returns `None` on a node with no README, which was
invisible while such nodes were pruned and becomes a silently dead key the
moment they are all on screen. Make the refusal say why — this node has no
README yet — without lifting the restriction, for the reasons under **Out of
scope**.

### 4. Collapse and expand

depends_on: [3]

Space toggles whether the selected directory's descendants are drawn. Collapsed
directories draw a marker so a collapsed node is distinguishable from an empty
one; an empty directory should not look like it is hiding something.

The app currently flattens the tree into rows once, in `from_tree`. Collapsing
means that flattening is re-run whenever the collapsed set changes, filtering
out any node with a collapsed ancestor. Hold the collapsed set as paths rather
than indices, so it survives a reload of the tree — pacting already rebuilds
state, and a user whose tree re-expands every time they press the pact key will
notice immediately.

Selection is the part that needs care. Collapsing a directory containing the
selection must move the selection to that directory, not leave an index pointing
at a row that is no longer drawn. Test that directly, along with collapsing the
root, expanding a directory whose parent is collapsed, and the selection's
position after a collapse that shortens the list above it.

Default expansion is everything expanded. It is predictable, it matches what
every file tree does, and it is what makes section 5's cold open true: someone
who launches in an unpacted repository should see their codebase, not a single
root row they have to go digging under. Users collapse what they do not want,
and slice 5 gives them a faster way to do it.

### 5. The pacted-only filter and the file toggle

depends_on: [4]

Two view toggles, both keys in the footer alongside the others.

The **pacted-only filter** draws pacted nodes and the ancestors needed to reach
them, and nothing else. That is brief 02's load-time pruning rule brought back
as a view: the same shape, now optional and reversible, which is the whole point
of the course correction. Toggling it must not lose the selection — if the
selected node is filtered away, land somewhere predictable and say so in the
tests. It composes with collapsing rather than replacing it.

The **file toggle** shows files under the directory that holds them. Files come
from the walk the loader already performs — it currently sees file entries and
skips them, so this is collecting what is being thrown away, not a second pass
— which also means files inherit the ignore rules for free.

Files are not nodes. Add a `files: Vec<PathBuf>` to `Node`, sorted, alongside
`children`. A file therefore has no `state` of its own to disagree with its
directory, no `readme`, and no children, and `is_leaf` keeps meaning "no child
directories" rather than blurring into "is a file". Section 5's rule that a file
takes its module's colour stops being a rule anyone has to remember and becomes
the only thing the types can express. The renderer emits file rows in the
parent's colour; the pact key does nothing on one.

Off by default. The default view is modules, because that is the altitude the
work happens at.

Finish by updating the root README's "What works today" section and both crate
READMEs: that the tree is now the whole repository rather than the documented
part of it, what the new keys do, that files are visible but not readable yet,
and that green is still not reachable.
