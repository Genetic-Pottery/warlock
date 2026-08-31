# warlock-engine

The core domain library crate of Warlock. It owns the state vocabulary, the tree of work, the on-disk manifest, the subtree hash and the staleness decision it feeds, the loader that colours a real directory, the seam a model pass runs through, and the pact operation that writes documents and grants freshness. Everything here is a thin crate root — `Cargo.toml` and `README.md` — with all implementation living in the single child directory `src/`.

## What's here

- **`Cargo.toml`** declares the dependencies (`blake3`, `ignore`, `serde`, `toml`; `serde_test`/`tempfile` as dev-dependencies only) and states, in comment form, the architectural rule this whole crate is built around: no TUI, terminal, HTTP, or Anthropic dependency belongs here, because the dependency edge runs TUI → engine and never back. Lints come from the workspace root.
- **`README.md`** is the authoritative, argued design account — not background reading but the source of truth the code implements. It's long, and worth reading before touching anything: the three-state vocabulary and why there is no fourth "unknown" state, the tree shape's files-vs-children distinction, the manifest's on-disk format and atomic write discipline, the exact inputs and deliberate exclusions of the subtree hash, why an unreadable file is a hard error rather than a silent skip, the `decide_state` truth table, the loader's rules for what counts as a node versus a documented module, the `Agent` seam (defined here, implemented nowhere in this crate), and the pact operation as the sole place a `granted_hash` is ever written.

## How it fits together

All implementation is in `src/`, which is flat and documents itself. In brief: `state.rs` is the closed three-state enum; `tree.rs` is the pure tree shape; `manifest.rs` is `.warlock/pacts.toml` and its atomic write idiom; `hash.rs` is the subtree hash; `ignores.rs` handles `.warlockignore`; `decide.rs` is the pure staleness decision; `load.rs` builds a real coloured tree; `agent.rs` defines the model-pass seam without implementing it; `pact.rs` (large) does the actual pacting, refreshing and un-pacting work, including a content-keyed summary cache; `scope.rs` and `sigils.rs` supply the scope/sigil vocabulary — judged but never enforced anywhere in this crate; `claude_md.rs` maintains a delimited splice-in-place block inside `CLAUDE.md`; `clock.rs` is a dependency-free RFC 3339 timestamp helper.

## What a reader has to know before changing anything

- **The TUI-never-depends-back rule is architecture, not convention.** No terminal, TUI, or subprocess-spawning dependency belongs here, even transitively, even for a test — the crate must stay usable headless.
- **This crate commits to exactly one on-disk format: the manifest.** `Tree`/`Node` derive serde for a caller's convenience but commit to no format of their own.
- **The three-state model and the never-skip-a-hash-failure rule are closed design decisions**, argued at length in `README.md`. A proposal to add a fourth state, or to tolerate a silently-skipped unreadable file, is a design regression, not a simplification.
- **Freshness is granted in exactly one place** — inside the pact/refresh path in `src/pact.rs`, after every document in a subtree has been written, never per-directory. There is currently no other way back to green; a stale subtree can only be re-pacted from scratch.
- **Nothing in this crate enforces a scope boundary.** `scope.rs` and `sigils.rs` only describe what a valid scope/sigil looks like and what a person holds; nothing here checks a sigil against a scope and blocks or refuses anything — `claude_md.rs`'s generated text says so explicitly, and a test pins that promise.
- **Read `README.md` before `src/lib.rs`'s doc comments, not instead of them.** The README explains why the crate is shaped this way; the source doc comments are the precise, current what.

This directory is pacted; per project convention, treat the code as authoritative wherever it and this document disagree.