# warlock-engine

The core domain library crate of Warlock: the crate root for the state vocabulary, the tree of work, the on-disk manifest, the subtree hash and the staleness decision it feeds, the loader that colours a real directory, the seam a model pass runs through, and the pact operation that writes documents and grants freshness. Everything at this level is packaging and argument, not implementation — `Cargo.toml` and `README.md` — with all code living in the single child directory `src/`.

## What's here

- **`Cargo.toml`** — the dependency list (`blake3`, `ignore`, `serde`, `toml`; `serde_test`/`tempfile` as dev-dependencies only) and, in its own comments, the architectural rule the whole crate is built around: no TUI, terminal, HTTP, or Anthropic dependency belongs here, because the dependency edge runs TUI → engine and never back. Lints come from the workspace root.
- **`README.md`** — a long, argued design document that is the source of truth the code implements, not background reading. It covers the three-state vocabulary and why there is deliberately no fourth "unknown" state, the tree's files-vs-children distinction, the manifest's on-disk format and atomic-write discipline, the exact inputs and deliberate exclusions of the subtree hash, why an unreadable file is a hard error rather than a silent skip, the `decide_state` truth table, the loader's rules for what counts as a node versus a documented module, the `Agent` seam (defined here, implemented nowhere in this crate), and the pact operation as the sole place a `granted_hash` is ever written.

## How it fits together

Everything else is under `src/`, which is flat and documents itself in `lib.rs`'s module list and re-exports. In outline: `state.rs` is the closed three-state enum; `tree.rs` is the pure tree shape; `manifest.rs` is `.warlock/pacts.toml` and its atomic write idiom; `hash.rs` is the subtree hash; `ignores.rs` handles `.warlockignore`; `decide.rs` is the pure staleness decision; `load.rs` builds a real coloured tree, rooted wherever it's invoked; `agent.rs` defines the model-pass seam without implementing it, so tests never spawn a real subprocess; `pact.rs` (by far the largest file) does the actual pacting, refreshing and un-pacting, including a content-keyed summary cache; `scope.rs` and `sigils.rs` supply scope/sigil vocabulary — judged but never enforced anywhere in this crate; `claude_md.rs` maintains a delimited splice-in-place block inside `CLAUDE.md`; `clock.rs` is a dependency-free RFC 3339 timestamp helper.

## What a reader has to know before changing anything

- **The TUI-never-depends-back rule is architecture, not convention.** `Cargo.toml` enforces it in comment form; no terminal, TUI, or subprocess-spawning dependency may be added here, even transitively or for a test — the crate must stay usable headless.
- **This crate commits to exactly one on-disk format: the manifest** (`.warlock/pacts.toml`, TOML, root-relative forward-slash paths). `Tree`/`Node` derive serde for a caller's convenience but commit to no format of their own.
- **The three-state model and the never-skip-a-hash-failure rule are closed design decisions**, argued at length in `README.md`. Adding a fourth state, or tolerating a silently-skipped unreadable file, is a design regression, not a simplification — a false green is the one outcome the design refuses to allow.
- **Freshness is granted in exactly one place** — inside the pact/refresh path in `src/pact.rs`, after every document in a subtree has been written, never per-directory. There is currently no other way back to green; a stale subtree can only be re-pacted from scratch (a refresh pass that re-grants only what deserves it is future work, not present in this crate).
- **Nothing in this crate enforces a scope boundary.** `scope.rs` and `sigils.rs` only describe what a valid scope/sigil looks like and what a person holds; nothing here checks a sigil against a scope and blocks or refuses anything — `claude_md.rs`'s generated text says so explicitly, and a test pins that promise.
- **Read `README.md` before `src/lib.rs`'s doc comments, not instead of them.** The README explains why the crate is shaped this way; the source doc comments are the precise, current what.

This directory is pacted; per project convention, treat the code as authoritative wherever it and this document disagree.