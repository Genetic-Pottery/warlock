# warlock-engine

The core domain library of Warlock, and a crate root in the thinnest sense: this directory itself holds only `Cargo.toml` and `README.md`, plus its one child, `src/`, where all implementation lives.

## What's here

- **`Cargo.toml`** is the enforceable half of the architecture. Its dependency list is deliberately small — `blake3`, `ignore`, `serde`, `toml`, with `serde_test`/`tempfile` as dev-dependencies — and its own comments state the rule outright: no TUI, terminal, HTTP, or Anthropic dependency belongs here, because the dependency edge runs TUI → engine and never back. Lints come from `[workspace.lints]` in the root manifest, not from local configuration, so this crate is held to the same bar as every other crate in the workspace.
- **`README.md`** is the authoritative design account, not background reading. It lays out the three-state vocabulary (`Unpacted` / `PactedStale` / `PactedFresh`, deliberately with no fourth "unknown" state — unjudged *is* stale), the pure tree shape, the manifest format at `.warlock/pacts.toml` and its atomic write-then-rename discipline, the exact input and exclusions of the subtree hash (an unreadable file is a hard error, never a silent skip, because a skip would hash like a deletion and could manufacture a false green), the pure `decide_state` truth table, the loader's rules for what becomes a node versus a documented module, the `Agent` seam (defined here, implemented nowhere in this crate), and the pact operation — the only code path that ever grants a `granted_hash`.

## How it fits together

Everything else lives in `src/`, already documented by its own `WARLOCK.md`. In brief: `lib.rs` is the module index; `state.rs` holds the closed three-state enum; `tree.rs` is pure tree shape plus loader-set `ignored`/`scope` fields; `manifest.rs` owns the on-disk manifest and the write-temp-then-rename idiom used everywhere; `hash.rs` computes the subtree hash; `ignores.rs` implements `.warlockignore`; `decide.rs` is the pure staleness decision; `load.rs` builds a real `Tree` from a real directory; `agent.rs` defines the model-pass seam without implementing it; `pact.rs` (large, described only from a prior summary) does the actual pacting work; `scope.rs` and `sigils.rs` implement the scope/sigil vocabulary (judged but not yet enforced anywhere); `claude_md.rs` maintains a delimited block in `CLAUDE.md`; and `clock.rs` is a dependency-free RFC 3339 timestamp helper.

## What a reader has to know before changing anything

- **The dependency direction is architecture, not convention.** Adding any terminal, TUI, or subprocess-spawning dependency here — even transitively, even for a test — breaks headless operation. `Cargo.toml`'s comment on this is a constraint, not decoration.
- **This crate commits to exactly one on-disk format**: the manifest. `Tree`/`Node` derive serde but commit to no format of their own.
- **The three-state model and never-skip-a-hash-failure rule are closed design decisions**, argued at length in `README.md` and restated in `src/WARLOCK.md`. Treat a proposal to add a fourth state, or to tolerate a silently-skipped unreadable file, as a design regression.
- **Freshness is granted in exactly one place** — inside the pact/refresh path in `src/pact.rs`, after every document in a subtree is written. There is currently no other way back to green.
- **Nothing in this crate enforces a scope boundary.** `scope.rs` and `sigils.rs` only say what a string is and what a person holds; no code here checks a sigil against a scope and blocks anything. `claude_md.rs` says so explicitly, and a test pins that promise.
- **Read `README.md` before `src/lib.rs`'s doc comments, not instead of them.** The README explains *why*; the source doc comments are the precise, current *what*.
- **`src/pact.rs` was too large to read in full here**; what's known about it comes from a prior summary in `src/WARLOCK.md`, not from its own text — open it directly before relying on specifics beyond what's already documented.