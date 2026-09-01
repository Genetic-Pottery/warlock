# warlock-engine

The core domain library crate of Warlock. This is the crate root: packaging and argument only — `Cargo.toml` and `README.md` — with every line of implementation living in the single child directory `src/`. It owns the state vocabulary, the tree of work, the on-disk manifest, the subtree hash and the staleness decision it feeds, the loader that colours a real directory, the seam a model pass runs through, and the pact operation that writes documents and grants freshness.

## What's here

- **`Cargo.toml`** — the dependency list (`blake3`, `ignore`, `serde`, `toml`; `serde_test`/`tempfile` as dev-dependencies only), and in its own comments the rule the crate is built around: no TUI, terminal, HTTP, or Anthropic dependency belongs here. Lints come from the workspace root.
- **`README.md`** (~31KB) — a long, argued design document that is the source of truth the code implements, not background reading. It works through the three-state model and why there is deliberately no fourth "unknown" state, the tree's files-vs-children distinction, the manifest's on-disk format and atomic-write discipline, the subtree hash's exact inputs and deliberate exclusions, why an unreadable file is a hard error rather than a silent skip, the `decide_state` truth table, the loader's rules for nodes vs. documented modules, the `Agent` seam (defined here, implemented nowhere in this crate), and the pact operation as the sole place a `granted_hash` is ever written.

## How it fits together

All code is under `src/`, which is flat — no subdirectories — and documents itself starting from `lib.rs`'s module list and re-exports. Its own `WARLOCK.md` breaks the modules down individually (`state.rs`, `tree.rs`, `manifest.rs`, `briefs.rs`, `hash.rs`, `ignores.rs`, `decide.rs`, `load.rs`, `agent.rs`, `pact.rs`, `scope.rs`, `sigils.rs`, `claude_md.rs`, `clock.rs`); the shape worth carrying up here is: a hash is the mechanical trigger, a model pass is the judgement, a human can always read and hand-edit the result, and scopes/sigils are a second, independent vocabulary that this crate defines and judges but never enforces.

## What a reader has to know before changing anything

- **The dependency edge runs TUI → engine, never back.** No terminal, TUI, or subprocess-spawning dependency belongs here, even transitively or in a test — this crate must stay usable headless, and `Cargo.toml`'s comments say so explicitly.
- **The manifest (`.warlock/pacts.toml`) is the one on-disk format this crate commits to.** `Tree`/`Node` derive serde for a caller's convenience but commit to no format of their own.
- **The three-state model and the never-silently-skip-a-hash-failure rule are closed design decisions**, argued at length in `README.md`. Treat a proposal to add a fourth state, or to tolerate a silently-skipped unreadable file, as a design regression rather than a simplification.
- **Freshness is granted in exactly one place** — the pact/refresh path in `src/pact.rs`, after every document in a subtree has been written, never per-directory. There is currently no way back to green for a stale subtree short of re-pacting it; a refresh pass that re-grants only what deserves it is future work not yet present.
- **Nothing in this crate enforces a scope boundary.** `scope.rs` and `sigils.rs` describe what a valid scope/sigil looks like and what a person holds; nothing here matches a sigil against a scope and blocks or refuses anything.
- **Read `README.md` before `src/lib.rs`'s doc comments, not instead of them** — the README argues why the crate is shaped this way; the source doc comments are the precise, current what.

This directory is pacted; per project convention, where this document and the code disagree, the code is right.