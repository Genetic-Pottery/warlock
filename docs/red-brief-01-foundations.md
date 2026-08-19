# Red brief: Warlock foundations

Stand up the Rust workspace Warlock will be built in, and prove the engine/TUI
seam works end to end with a stub. This is scaffolding: no freshness logic, no
AI, no Linear. When it is done, every later ticket has a place to land and a
compiler, formatter, linter and CI that already say yes.

Read `warlock-design-doc.md` in this repo before drafting. It is the whole
product spec; this project only builds sections 12's skeleton, nothing else.

## Outcome

Anyone who clones this repo can run `cargo test` and `cargo run` and get a
working TUI that draws a three-colour module tree out of typed data the engine
handed it. The engine/TUI boundary from the design doc exists as two real crates
with a one-way dependency, so the free/paid split later is a matter of adding
crates rather than untangling one. Nothing about staleness is decided yet; what
is decided is where that decision will live.

## Success criteria

- `cargo build --workspace` and `cargo test --workspace` pass from a clean
  clone on stable, with no warnings.
- `cargo run` opens a TUI, draws a tree whose nodes are gray, yellow and green,
  moves a selection with the arrow keys and `j`/`k`, and quits on `q` leaving
  the terminal in a usable state (also after a panic).
- `warlock-engine`'s manifest lists no TUI or terminal dependency, and its tests
  run with no terminal attached.
- CI runs format check, clippy with warnings denied, and the test suite on every
  push and pull request, and fails if any of the three do.
- Every dependency in the workspace dependency table carries a one-line comment
  saying why it is there.

## Constraints

- Rust, stable toolchain, 2024 edition. The toolchain is provided by the system
  (currently rustc 1.97.1, cargo 1.97.0), not by rustup — there are no rustup
  shims on this machine, so do not add a `rust-toolchain.toml` and do not try to
  install or switch a toolchain. The version floor goes in `rust-version` under
  `[workspace.package]`, which cargo actually enforces.
- TUI is Ratatui over Crossterm. Not negotiable, it is in the design doc.
- Work in this repo, `warlock`, which currently holds only `README.md`, an
  Apache-2.0 `LICENSE` and the design doc. Keep the licence as it is and set
  `license = "Apache-2.0"` in the crate metadata.
- No HTTP client and no Anthropic SDK, ever. Warlock talks to AI by spawning the
  `claude` CLI as a subprocess, and that comes later.
- The engine stays headless and testable without a terminal, a repo or a model.
  The dependency edge runs TUI to engine and never back.
- No `unsafe`.
- Keep the dependency count small. Prefer std. A crate that only saves a few
  lines is not worth the supply chain.
- No mocking framework; hand-written fakes and fixtures, matching how Red and
  Forman are tested.

## Out of scope

Everything that is the actual product. Specifically: subtree hashing, any
staleness or freshness computation, invoking `claude` for anything, reading real
module READMEs off disk, pacts, pact expansion, blessing and the gate, the
journal at `~/.warlock/`, the pre-push guard, the CI validator, Linear or
Red/Forman integration, the paid crate and any licensing infrastructure,
publishing to crates.io, release binaries, and anything cross-platform beyond
"do not gratuitously break Windows".

Also out of scope on purpose: the `.warlock/` manifest schema. The design doc
lists it as an open question and getting it wrong costs a migration, so nothing
here should persist a state file or serialise anything to disk. Types may derive
serde so the option is open, but no format is committed to.

## Scope

### 1. Cargo workspace with engine and TUI crates

Create the workspace at the repo root: a virtual manifest with
`crates/warlock-engine` (library) and `crates/warlock-tui` (binary, produces an
executable called `warlock`). Shared metadata — version, edition, rust-version,
licence, repository — lives in `[workspace.package]` and both crates inherit it.
Shared dependencies live in `[workspace.dependencies]` so versions are declared
once.

Set `rust-version` in `[workspace.package]` to the toolchain in use, and add a
`/target` entry to the existing `.gitignore` (which currently only ignores
`.red` and `.forman`; leave those alone). Each crate gets a short README stating what it is and, for the engine,
that it must never depend on the TUI. Both crates compile and `cargo test`
passes against a placeholder test.

Nothing in this slice does anything useful at runtime. It is the shape.

### 2. The engine's state vocabulary and a stub tree

depends_on: [1]

Give `warlock-engine` the public types the rest of the product hangs off, and
nothing else. The state enum is the important one: a node is unpacted, pacted
and stale, or pacted and fresh, with no fourth variant and no "unknown" — the
design doc is explicit that unjudged *is* stale, and the type should make a
fourth state impossible to represent rather than merely discouraged.

Alongside it, a node type carrying its path, the path of its README, its state
and its children, and a tree type that owns the root and can be walked. Add the
obvious cheap operations a renderer will want: iterate nodes depth first, count
by state, find a node by path. Derive serde on what will eventually be
persisted, but persist nothing.

Then one constructor that builds a small hard-coded tree with a mix of all three
states, clearly named as a stub — this is what the TUI renders in slice 3 and
what proves the seam before any real filesystem walking exists. It must be
obvious to a later reader that this is a placeholder, not the loader.

Unit tests cover the walk order, the counts and the round trip of the state
enum. No filesystem access, no hashing, no I/O of any kind.

### 3. TUI shell rendering the stub tree

depends_on: [1, 2]

`cargo run` opens an alternate screen in raw mode, renders the engine's stub
tree as an indented list with each node coloured by its state — gray unpacted,
yellow stale, green fresh — moves a highlighted selection with the arrow keys
and `j`/`k`, and quits on `q`, `Esc` or Ctrl-C.

Terminal restoration is the part that has to be right. Leaving a terminal in raw
mode on exit is the difference between a tool people try twice and a tool people
try once, so restore on the normal path, on an error path and on a panic, via a
panic hook installed before the terminal is put into raw mode.

Keep the structure honest even though it is small: an app state that owns the
tree and the selection index, a pure function mapping state to a colour, and an
event loop that mutates the app and redraws. The tree the TUI renders arrives
from the engine as data; the TUI decides how it looks and knows nothing about
where it came from. Anything that could be tested without a terminal should be —
the colour mapping and the selection movement at minimum.

A footer showing the counts by state and the keybindings is worth the ten lines.

### 4. Format, lint and CI

depends_on: [1]

Configure the tooling so "it builds" means something. A `rustfmt.toml` (leave it
close to default; only set what there is a reason to set), and clippy configured
through `[workspace.lints]` inherited by both crates, with the pedantic-adjacent
lints that catch real problems turned on and anything noisy turned off
deliberately rather than by accident.

Add a GitHub Actions workflow running on push and pull request that checks
formatting, runs clippy with warnings denied, and runs the test suite, on stable
with a cached cargo registry and target directory. Three separate steps so a
failure names itself. The repo already has a `.github` directory from Red and
Forman's tooling — add to it rather than replacing anything.

Confirm in the crate READMEs, or in the root one, which three commands a
contributor is expected to run before pushing.
