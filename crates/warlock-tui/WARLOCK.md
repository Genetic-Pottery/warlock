# warlock-tui

The terminal front end of warlock. This crate ships the `warlock` executable — a `[[bin]]` named `warlock`, distinct from the crate name `warlock-tui`, so `cargo run` builds and runs `warlock`. It also exposes a library (`warlock_tui`) so the pure parts of the front end are ordinary reachable API shared by the binary and its tests, with `src/main.rs` reduced to the thin shell around them.

## What it does

Draws the current state of the work tree the engine loads from wherever the binary was launched, and turns keystrokes into requests back to the engine. Nothing here computes anything about the tree — colour, freshness, module boundaries — all of that is decided by `warlock-engine` before a frame is drawn; this crate only asks for the tree (`load_tree`) and renders what comes back.

The whole surface is documented in the README, which a reader should treat as the spec, not just an overview:

- The tree view: indented, guided (`│ ├ └`), coloured by state, windowed to the terminal height, with a header naming which part of the repo is on screen and a three-line footer (tally, keys, one status line that is blank rather than absent when idle).
- The full key set (`action_for`): navigation, `Space` to collapse, `f` to show files, `o` to filter to pacted nodes, `p` to pact, `q`/Ctrl-C to quit, `Esc` with dual meaning — cancel a running pact, or quit if nothing is running.
- **`p` is the one key that writes anything.** It runs an engine pact over the selected subtree on a worker thread while the UI stays responsive, and is the only path by which a directory goes from undocumented/gray to yellow (pacted, ungranted) to green (documented, hashed, granted). There is no second green: refreshing an already-granted pact is explicitly out of scope today.
- File rows (`f`) are decorative only — inherited colour, no state of their own, not counted, not pactable, never opened. There is deliberately no file-viewer pane.
- The manifest this crate's pact key writes to (`.warlock/pacts.toml`) lives one level above the repo root actually being viewed and is committed to git; its schema is owned by `warlock-engine`, not here.
- All other view state (collapse set, filters, selection, scroll) is held only in memory by `App` and never persisted — the engine's serializable `Node`/`Tree` types have no field for any of it, so it structurally cannot leak into a document or the manifest.

## Structure

Two pieces, cleanly split:

- **The library** (`src/`, described by its own WARLOCK.md) — the pure core: `App`'s view-over-a-tree state, `colour_for`, `draw`/`ui`, input translation, watch-debounce policy, and `ClaudeAgent`, the one place that spawns the `claude` CLI as a child process on the engine's behalf. Everything pure takes the current instant as an argument rather than reading a clock, which is what makes minutes-long pacts and multi-second debounce windows testable in microseconds.
- **The binary** (`src/main.rs`) — the impure remainder: raw mode, alternate screen, the event loop, the worker thread a pact runs on, and terminal restoration on every exit path (normal quit, error, or panic) via an RAII guard and a chained panic hook.

## Before changing anything here

- **Dependency edge runs one way: TUI → engine, never back.** This crate knows terminal and keystroke vocabulary; the engine knows nothing about either. Domain logic (what a pact means, how freshness is judged, prompt composition) belongs in `warlock-engine`, not here — even `claude.rs`, which spawns the process, only transports what the engine composed and never adds to a prompt.
- **Crossterm is not a direct dependency.** Use `ratatui::crossterm`, Ratatui's re-export, so frontend and backend can't skew.
- **Don't reach for `Instant::now()` or a real terminal call inside the pure modules** — thread the value in instead; this is the whole reason the UI is testable without a terminal attached.
- **View state is deliberately non-persistent.** Don't add a way to save collapse/filter/selection state without first checking whether that belongs in the engine's serializable types (it currently structurally cannot, on purpose).
- **This is not an IDE.** No prompt box, no chat transcript, no file contents on screen — the panel is a freshness ledger. Keep additions in that register.