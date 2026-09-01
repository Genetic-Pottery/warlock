# crates

The Cargo workspace root for Warlock. This directory holds the workspace-level `Cargo.toml` (and its lockfile) plus two member crates, `warlock-engine` and `warlock-tui`, each fully documented by its own `WARLOCK.md`. There is no implementation code directly here — only the shared build configuration that binds the two crates together.

## What this directory establishes

The workspace `Cargo.toml` is where the two crates' lints and shared settings are declared once and inherited downward — both `warlock-engine`'s and `warlock-tui`'s own documents note that their lints come from `[lints] workspace = true`, pointing back to this root. Anyone tightening or loosening a lint, bumping a shared dependency version, or adding a third crate does it here, and the change propagates to both members.

## How the two crates relate

- **`warlock-engine`** is the domain library: the three-state freshness vocabulary, the tree shape, the manifest format, the subtree hash, the pure staleness decision, the directory loader, the model-pass seam, and the pact operation that writes documents and grants freshness. Its own document is explicit that no TUI, terminal, HTTP, or Anthropic dependency belongs in it, even transitively. It defines what a valid scope or sigil looks like but, per its own document, enforces nothing against either — nothing in the crate matches a sigil against a scope and blocks work.
- **`warlock-tui`** is the terminal front end: the `warlock` binary plus a `warlock_tui` library. Per its own document, it draws whatever `warlock-engine` computes and translates keystrokes and other input into engine calls, and owns the one place a `claude` subprocess is spawned (`claude.rs`, hosting both a pact-driving `ClaudeAgent` and a read-only, session-persistent `ChatAgent`) — but only as a transport for prompts the engine composed. Its own README has drifted behind its `src/`'s document (a three-card panel, a composer, and disk-writing that the README doesn't mention), so `src/`'s document and the source are the trustworthy account there, not the README.

The dependency edge between them runs one direction only: **TUI → engine, never back**. Both crates' own documents assert this independently and both `Cargo.toml`s state it as the rule the dependency lists are built around — `warlock-engine` is a `path` dependency of `warlock-tui`, never the reverse. It is the load-bearing architectural fact of this workspace: a change that makes `warlock-engine` depend on `warlock-tui`, or that lets terminal or keystroke concepts leak into the engine, breaks the design regardless of how it's expressed in `Cargo.toml`.

## What a reader has to know before changing anything

- **This is a two-crate, one-direction workspace.** Before adding a dependency to either crate, check which side of the TUI→engine edge it belongs on. A dependency needed only for rendering, input, or subprocess-spawning has no business in `warlock-engine`'s `Cargo.toml`, even if it would be convenient there.
- **Lints are workspace-owned.** If a lint feels wrong in one crate, the fix is likely here, not a local `#[allow]` in that crate — check the workspace manifest first.
- **This directory is build plumbing, not design documentation.** The actual design (the three-state model, manifest format, key bindings, view architecture, purity boundaries) lives in the two crates' own READMEs and `WARLOCK.md`s, and each crate's document says its own source is authoritative over its own README where they disagree. Read those before making a change that isn't purely about workspace wiring.
- **Neither crate's document claims scope/sigil enforcement exists.** `warlock-engine` defines the vocabulary and `warlock-tui` can write a scope into the manifest, but neither document describes anything that matches a sigil against a scope and blocks work.
- **Freshness is granted per-subtree, not per-directory**, per `warlock-engine`'s document — `pact_subtree`/`refresh_subtree`, with `r` covering the narrower re-grant of already-stale directories within an already-pacted subtree and `p` covering pacting or un-pacting the whole subtree.