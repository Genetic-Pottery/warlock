# warlock-engine

The core domain library crate of Warlock. This directory itself is a thin crate root — `Cargo.toml` and `README.md` — with all implementation living in its one child, `src/`.

## What's here

- **`Cargo.toml`** enforces the crate's architectural boundary in dependency form: only `blake3`, `ignore`, `serde`, `toml`, plus `serde_test`/`tempfile` as dev-dependencies. Its comments state the rule directly — no TUI, terminal, HTTP, or Anthropic dependency belongs here, because the dependency edge runs TUI → engine and never back. Lints are inherited from the workspace root, not configured locally.
- **`README.md`** is the authoritative design account, not incidental background: the three-state vocabulary (`Unpacted` / `PactedStale` / `PactedFresh`, with no fourth "unknown" state — unjudged *is* stale), the pure tree shape, the `.warlock/pacts.toml` manifest format and its atomic write-then-rename discipline, the exact inputs and exclusions of the subtree hash (an unreadable file is a hard error, never a silent skip — a skip would hash like a deletion and could manufacture a false green), the pure `decide_state` truth table, the loader's rules for what becomes a node versus a documented module, the `Agent` seam (defined here, implemented nowhere in this crate), and the pact operation, the only code path that ever grants a `granted_hash`.

## How it fits together

All implementation is in `src/`, already documented by its own `WARLOCK.md`: `lib.rs` is the module index and re-export surface; `state.rs` the closed three-state enum; `tree.rs` pure tree shape plus loader-set `ignored`/`scope` fields; `manifest.rs` the on-disk manifest and its write-temp-then-rename idiom; `hash.rs` the subtree hash; `ignores.rs` `.warlockignore` handling; `decide.rs` the pure staleness decision; `load.rs` the real-directory loader; `agent.rs` the model-pass seam, defined but not implemented; `pact.rs` (large, described only from a prior summary) the actual pacting work; `scope.rs` and `sigils.rs` the scope/sigil vocabulary (judged but not enforced anywhere); `claude_md.rs` a delimited, splice-in-place block in `CLAUDE.md`; and `clock.rs` a dependency-free RFC 3339 timestamp helper.

## What a reader has to know before changing anything

- **The dependency direction is architecture, not convention.** No terminal, TUI, or subprocess-spawning dependency belongs here, even transitively, even for a test — headless operation must survive. `Cargo.toml`'s comment on this is a constraint.
- **This crate commits to exactly one on-disk format**: the manifest. `Tree`/`Node` derive serde but commit to no format of their own.
- **The three-state model and never-skip-a-hash-failure rule are closed design decisions**, argued in `README.md` and restated in `src/WARLOCK.md`. Treat a proposal to add a fourth state, or to tolerate a silently-skipped unreadable file, as a design regression.
- **Freshness is granted in exactly one place** — inside the pact/refresh path in `src/pact.rs`, after every document in a subtree is written. There is currently no other way back to green.
- **Nothing in this crate enforces a scope boundary.** `scope.rs` and `sigils.rs` only say what a string is and what a person holds; nothing here checks a sigil against a scope and blocks anything. `claude_md.rs` says so explicitly, and a test pins that promise.
- **Read `README.md` before `src/lib.rs`'s doc comments, not instead of them.** The README explains *why*; the source doc comments are the precise, current *what*.
- **`src/pact.rs` was too large to read in full here**; what's known about it comes from a prior summary in `src/WARLOCK.md`, not from its own text — open it directly before relying on specifics beyond what's already documented.

This directory is pacted and its `WARLOCK.md` reflects the state above; per project convention, treat the code as authoritative wherever it and this document disagree, and note the gap rather than silently trusting either side.