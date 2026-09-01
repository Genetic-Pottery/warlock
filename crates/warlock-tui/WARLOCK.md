# warlock-tui

The terminal front end of warlock: the crate that ships the `warlock` executable — a `[[bin]]` named `warlock`, distinct from the crate name `warlock-tui`, so `cargo run` from the repo root builds and runs `warlock` — and also exposes a library, `warlock_tui`, carrying the pure parts of the front end as ordinary reachable API shared by the binary and its own tests.

## What lives here

Three things sit directly in this directory: `Cargo.toml`, stating the dependency contract; `README.md`, a long, detailed spec of the panel; and `src/`, where all the actual code lives, with its own `WARLOCK.md` inventorying it file by file. Read that document before touching anything under `src/`.

**The README is stale relative to the code, and materially so.** It describes a footer with a plain tally/keys/message line and a `p`/`r` key set only. It says nothing of a composer, a conversation thread, a document viewer, `v`/`e`/`s` keys, or a second `ChatAgent` — all of which are present and current in `src/`'s own document (composer, conversation thread, module scoping, file viewing, module editing, two agents sharing one process-spawning seam). Trust `src/`'s document and the source over the README wherever they disagree, and say so rather than treating the README as ground truth. This directory's own document is owed a pass to reconcile the two.

## What this crate is for

It draws the current state of the work tree the engine loads from wherever the binary was launched, and turns keystrokes into requests back to the engine. It computes nothing about the tree itself — colour, freshness, and module boundaries are all decided by `warlock-engine` before a frame is drawn; this crate only asks (`load_tree`) and renders what comes back. `p` launches a pact run over a subtree, children before parents, and the tree stays usable while it runs. `r` runs the same worker over just the stale directories of an already-pacted subtree, through the same account, cancel handle, and single save. `s` writes a scope directly into the manifest synchronously, since it's one string on one entry rather than a model pass.

## How the parts fit together

`Cargo.toml` states the one hard rule the crate is built around: the dependency edge runs **TUI → `warlock-engine`, and never back**. This crate knows terminal and keystroke vocabulary; the engine knows neither, and never depends on this crate. Crossterm is deliberately not a direct dependency — code uses `ratatui::crossterm`, Ratatui's own re-export of the backend it was built against, so frontend and backend can never skew apart. `tempfile` is a dev-dependency only, letting the `warlock config` tests write into a throwaway home directory instead of the developer's real one.

Within `src/` (summarized there, not restated here), the split is between a pure core — `App`'s flattened/filtered/scrollable view, rendering, input translation, colour rules, and the pact/chat/scoping orchestration as data transformations — and a thin impure shell: one file (`claude.rs`) that spawns a process, and a small named set of others that touch a real terminal, clock, filesystem watcher, or interactive stdin. That split is what lets minutes-long pacts and a whole UI be driven through tests in microseconds against in-memory buffers and hand-fed instants.

## What a reader has to know before changing anything

- **Domain logic belongs in the engine, never here.** What a pact means, how freshness is judged, prompt composition, and what makes a scope or sigil valid are engine questions. A change here that starts deciding one of those has crossed the boundary this crate exists to keep.
- **`p` pacts a whole subtree in one run, children before parents**, and the tree stays usable while it runs; `Esc` cancels a run in progress rather than quitting. On an already-pacted subtree, `p` un-pacts it instead — a manifest edit only, leaving documents on disk. No second `p`/`r`/`s` run starts while one is already going, while `v` (view) and the swap key race nothing and are never refused.
- **View state is never persisted.** Collapse set, filters, selection, scroll offset, and the modal gates live only in memory on `App` — structurally, since the engine's serializable types have no field any of it could ride out through. Every run starts fully expanded, unfiltered, first row selected.
- **The manifest is `.warlock/pacts.toml`, one per repository, committed to git**, saved once per run via the engine's atomic `Manifest::save`, never once per directory.
- **The purity boundary is the whole design of `src/`.** Only one file spawns a process; only a small named set of others touch a real terminal, clock, or OS handle. Everything else is a pure function of its arguments, with the current instant always threaded in rather than read internally.
- **This is not an IDE.** The panel is a freshness ledger with a narrow composer and a conversation bolted on — not a dashboard, not a general chat surface. Check any addition against that register before building it.

## `src/`

Summarized in its own document, not restated here: a pure core (`App`, `Account`, colour rules, rendering in `ui.rs`, input translation, pact/refresh/scope orchestration, a conversation thread, the composer, two modal gates) around a thin impure shell (`claude.rs` as the sole process-spawning point, `main.rs`/`terminal.rs`/`watch.rs`/`config.rs` as the only touches of a real terminal, clock, filesystem watcher, or interactive stdin). Two agents share that one seam — `ClaudeAgent` for pacts, `ChatAgent` for read-only conversation turns — and a run started mid-conversation feeds both the account card and the thread from a single call, never worded twice.