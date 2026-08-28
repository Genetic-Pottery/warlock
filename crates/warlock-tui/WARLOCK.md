# warlock-tui

The crate root of `warlock-tui`, the terminal front end of warlock. It ships the `warlock` executable — a `[[bin]]` target named `warlock`, distinct from the crate name, so `cargo run` builds and runs `warlock` — and also exposes a library (`warlock_tui`) carrying the pure parts of the front end as ordinary reachable API, shared by the binary and its tests.

## What lives here

Only two files sit directly in this directory: `Cargo.toml` and `README.md`. All the code — `App`, rendering, input translation, the pact worker, the process-spawning agent, the event loop, the two modal gates — lives one level down in `src/`, whose own WARLOCK.md describes it in full. This directory's job is to state the crate's identity and dependency contract, and to hold the README that is treated as the spec for the whole front end's behavior: every key binding, every screen element, the manifest format, and the state-colour rules.

## What this crate is for

It draws the current state of the work tree the engine loads from wherever the binary was launched, and turns keystrokes and mouse events into requests back to the engine. It computes nothing about the tree itself — colour, freshness, module boundaries are all decided by `warlock-engine` before a frame is drawn. This crate only asks (`load_tree`) and renders what comes back. The most consequential key (`p`) launches a pact run — the only way a directory's state changes on disk — and `s` writes a scope directly into the manifest without a worker or a reload.

## What a reader has to know before changing anything

- **The dependency edge is one-directional and declared in `Cargo.toml`: TUI → `warlock-engine`, never the reverse.** This crate knows terminal and keystroke vocabulary; the engine knows nothing about either. Domain logic — what a pact means, how freshness is judged, prompt composition, what makes a scope or sigil valid — belongs in the engine, never here.
- **Crossterm is not a direct dependency.** Use `ratatui::crossterm`, Ratatui's re-export of the backend it was built against, so frontend and backend versions cannot skew apart.
- **`p` is the only key that writes a pact** — it runs an engine pact over the selected subtree on a worker thread while the UI stays responsive, and it is the only path from undocumented/gray to yellow (pacted, ungranted) to green (documented, hashed, granted). `s` is the other key that writes anything: it edits a scope into `.warlock/pacts.toml` synchronously, with no worker and no reload, because it is one string on one manifest entry rather than a model pass.
- **All other view state — collapse set, filters, selection, scroll, and the two modal gates (quit confirmation, scope prompt) — is held only in memory and never persisted.** The engine's serializable types have no field for any of it, so this is structural, not a policy choice that could quietly be walked back by adding a field somewhere convenient.
- **The purity boundary is the whole design of `src/`.** Only `claude.rs` spawns a process; only `main.rs`, `terminal.rs`, `watch.rs`'s watcher handle, and `config.rs`'s stdin touch a real terminal, clock, or OS handle. Everything else is a pure function of its arguments, with the current instant always threaded in rather than read internally — what makes minutes-long pacts and multi-second debounce windows testable without a real terminal or a real clock. Read `src/`'s WARLOCK.md before touching anything there.
- **This is not an IDE.** No prompt box, no chat transcript, no file contents ever shown on screen, no free-text input beyond the two narrow, purpose-built fields for scope and sigil text. The panel is a freshness ledger — facts about what a pass did — and any addition should be checked against that register before it's built.