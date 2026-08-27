# warlock-engine

This is the top-level crate directory of `warlock-engine`, the core domain library of Warlock. It holds the crate manifest (`Cargo.toml`), the long-form design account (`README.md`), and one subdirectory, `src/`, which is where every line of implementation actually lives — this crate has no other children.

## What this directory establishes

`Cargo.toml` is the enforceable half of the architecture: it declares the dependency set (`blake3`, `ignore`, `serde`, `toml`, plus `serde_test`/`tempfile` for tests) and, in its comments, states outright that no TUI, terminal, HTTP, or Anthropic dependency may ever appear here — the dependency edge runs TUI → engine and never back. Lints are inherited from the workspace root rather than configured locally, so this crate is held to the same bar as every other crate in the workspace, not a bar of its own choosing.

`README.md` is the authoritative design document for the crate: the three-state vocabulary, the tree shape, the manifest format and its atomicity/portability guarantees, the subtree hash's exact input and exclusions, the pure decision rule, the loader's rules for what becomes a node versus a module, the `Agent` seam, and the pact operation that grants freshness. Anyone changing behavior in `src/` should treat mismatches between the code and this file as a problem to fix, not an artifact to ignore — this README is current design, not a stale ticket.

## How it fits together

There is no code directly in this directory — `src/` is the entire implementation, already documented by its own `WARLOCK.md`: nine files (`lib.rs`, `state.rs`, `tree.rs`, `manifest.rs`, `hash.rs`, `decide.rs`, `load.rs`, `agent.rs`, `pact.rs`, `clock.rs`) implementing, respectively, the module index, the state vocabulary, pure tree shape, the on-disk manifest, the content hash, the pure staleness decision, the directory-to-tree loader, the model-pass seam (unimplemented here on purpose), the pact/unpact machinery, and a dependency-free timestamp helper.

## What a reader has to know before changing anything

- **The dependency direction is load-bearing, not incidental.** Adding any terminal, TUI, or subprocess-spawning dependency here — even transitively, even for a test — breaks the architecture that lets the engine run headless. `Cargo.toml`'s comment on this is not decorative.
- **This crate commits to exactly one on-disk format, the manifest** (`.warlock/pacts.toml`, TOML, atomic write-and-rename, root-relative forward-slash paths). Everything else (`Tree`, `Node`) derives serde but deliberately commits to no format of its own — don't read the manifest's concreteness as license to add formats elsewhere.
- **The three-state model and the never-skip-a-hash-failure rule are both closed design decisions**, detailed at length in the README and restated in `src/WARLOCK.md`. Treat proposals to add a fourth state or to silently tolerate an unreadable file as design regressions, not simplifications.
- **There is currently no refresh pass.** Freshness is only ever granted by a full subtree pact; a stale node stays stale until repacted. If you're picking this up, check whether it's already been started elsewhere before assuming it's still open.
- **Read `README.md` before `src/lib.rs`'s doc comments**, not instead of them — the README explains *why*, the source doc comments are the precise, current *what*.