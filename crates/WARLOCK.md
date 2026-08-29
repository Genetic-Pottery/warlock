# crates

The Cargo workspace root for Warlock. This directory holds the workspace-level `Cargo.toml` (and lockfile) plus two member crates, `warlock-engine` and `warlock-tui`, each fully documented by its own `WARLOCK.md`. There is no code directly here — only the shared build configuration that binds the two crates together.

## What this directory establishes

The workspace `Cargo.toml` is where the two crates' lints and shared settings are declared once and inherited downward, which is how `warlock-engine` can state "lints are inherited from the workspace root rather than configured locally" — that root is this directory. Anyone tightening or loosening a lint, bumping a shared dependency version, or adding a third crate does it here, and the change propagates to both members.

## How the two crates relate

- **`warlock-engine`** is the domain library: tree shape, manifest format, content hashing, the pure staleness decision, the directory loader, and the pact/unpact mechanics that grant freshness. It is deliberately headless — no TUI, terminal, HTTP, or Anthropic dependency is permitted in it, even transitively.
- **`warlock-tui`** is the terminal front end: the `warlock` binary plus a `warlock_tui` library. It renders whatever `warlock-engine` computes and translates keystrokes into engine calls (chiefly `p` to pact a subtree, `r` to refresh only the stale directories within a pacted subtree, `s` to write a scope directly). It owns the one place a `claude` subprocess gets spawned, but only as a transport for prompts the engine composed.

The dependency edge between them runs one direction only: **TUI → engine, never back**. This is asserted independently in both crates' own documentation and is the load-bearing architectural fact of this workspace — a change that makes `warlock-engine` depend on `warlock-tui`, or that lets terminal/keystroke concepts leak into the engine, breaks the design regardless of how it's expressed in `Cargo.toml`.

## What a reader has to know before changing anything

- **This is a two-crate, one-direction workspace.** Before adding a dependency to either crate, check which side of the TUI→engine edge it belongs on. A dependency needed only for rendering, input, or subprocess-spawning has no business in `warlock-engine`'s `Cargo.toml`, even if it would be convenient there.
- **Lints are workspace-owned.** If a lint feels wrong in one crate, the fix is likely here, not a local `#[allow]` in that crate — check the workspace manifest first.
- **Don't infer behavior from this directory.** It is build plumbing; the actual design (the three-state model, manifest format, key bindings, view architecture, purity boundaries) lives in the two crates' own READMEs and `WARLOCK.md`s. Read those before making a change that isn't purely about workspace wiring.
- **Neither crate enforces scope/sigil boundaries yet.** The engine defines the vocabulary and the TUI can write a scope into the manifest, but no code anywhere checks a sigil against a scope and blocks work. Don't assume that gap has closed just because it's been noted before.
- **A stale node stays stale until repacted or refreshed** — `r` now covers the narrower, per-directory case, so the engine-level fact to check before assuming a gap is which of `p` (whole subtree) or `r` (only the already-stale members of one) applies to the change at hand.