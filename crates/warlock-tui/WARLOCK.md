# warlock-tui

This directory is the crate root of `warlock-tui`, the terminal front end of warlock. It ships the `warlock` executable — a `[[bin]]` target named `warlock`, distinct from the crate name, so `cargo run` builds and runs `warlock` — and also exposes a library (`warlock_tui`) that carries the pure parts of the front end as ordinary reachable API, shared by the binary and its tests.

## What lives here

Only two files sit directly in this directory: `Cargo.toml` and `README.md`. The actual code — `App`, rendering, input translation, the pact worker, the process-spawning agent, the event loop — lives one level down in `src/`, whose own WARLOCK.md describes it in full. This directory's job is to state the crate's identity and dependency contract, and to hold the README that is treated as the spec for the whole front end's behavior (every key binding, every screen element, the manifest format, the state-colour rules).

## What this crate is for

It draws the current state of the work tree the engine loads from wherever the binary was launched, and turns keystrokes into requests back to the engine. It computes nothing about the tree itself — colour, freshness, module boundaries are all decided by `warlock-engine` before a frame is drawn. This crate only asks (`load_tree`) and renders what comes back, then turns key and mouse events into actions, the most consequential of which (`p`) launches a pact run that is the only way a directory's state changes on disk.

## What a reader has to know before changing anything

- **The dependency edge is one-directional and declared in `Cargo.toml`: TUI → `warlock-engine`, never the reverse.** This crate knows terminal and keystroke vocabulary; the engine knows nothing about either. Domain logic — what a pact means, how freshness is judged, prompt composition — belongs in the engine, never here.
- **Crossterm is not a direct dependency.** Use `ratatui::crossterm`, Ratatui's re-export of the backend it was built against, so frontend and backend versions cannot skew apart.
- **`p` is the only key that writes anything** — it runs an engine pact over the selected subtree on a worker thread while the UI stays responsive, and it is the only path from undocumented/gray to yellow (pacted, ungranted) to green (documented, hashed, granted). There is no second green today: refreshing an already-granted pact is out of scope.
- **All other view state — collapse set, filters, selection, scroll — is held only in memory and never persisted.** The engine's serializable types have no field for any of it, so this is structural, not a policy choice that could quietly be walked back by adding a field somewhere convenient.
- **This is not an IDE.** No prompt box, no chat transcript, no file contents ever shown on screen. The panel is a freshness ledger — facts about what a pass did — and any addition should be checked against that register before it's built.
- The library's own purity discipline (no clock or terminal access inside pure modules; instants and terminal state always threaded in as arguments) is what makes minutes-long pacts and multi-second debounce windows testable without a real terminal or a real clock — see `src/`'s WARLOCK.md before touching anything there.