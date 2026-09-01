# warlock-tui

The terminal front end of warlock: the crate that ships the `warlock` executable — a `[[bin]]` named `warlock`, distinct from the crate name `warlock-tui`, so `cargo run` from the repo root builds and runs `warlock` — and also exposes a library, `warlock_tui`, carrying the pure parts of the front end as ordinary reachable API shared by the binary and its own tests.

## What lives here

Three things sit directly in this directory: `Cargo.toml`, stating the dependency contract; `README.md`, a long spec of the panel; and `src/`, where all the actual code lives, with its own `WARLOCK.md` inventorying it module by module. Read `src/`'s document before touching anything under `src/`.

**The README is stale relative to the code, and materially so.** It describes a footer with a plain tally/keys/message line and a `p`/`r` key set only, and says a viewer pane is "deliberately unbuilt." `src/`'s document describes a three-card panel (an account of a running pact, a conversation thread, or a read document), a composer, `input.rs`/`composer.rs`/`confirm.rs`/`prompt.rs` as distinct modal machinery, a `ChatAgent` alongside `ClaudeAgent`, and disk-writing (`writing.rs`, in fact one of several impure files named in `src/`'s document as living in the *binary*, alongside `pacting.rs`, `chatting.rs`, `scoping.rs`, `viewing.rs`, `editing.rs`, `session.rs`, `terminal.rs`, `input.rs`, `config.rs`, `error.rs`) — none of which the README mentions. Trust `src/`'s document and the source over the README wherever they disagree. This directory's own document is itself owed a pass to reconcile the two; what follows is written from `src/`'s document, not from the README.

## What this crate is for

It draws the current state of the work tree the engine loads from wherever the binary was launched, and turns keystrokes into requests back to the engine. It computes nothing about the tree itself — colour, freshness, and module boundaries are all decided by `warlock-engine` before a frame is drawn; this crate only asks and renders what comes back.

## How the parts fit together

`Cargo.toml` states the one hard rule the crate is built around: the dependency edge runs **TUI → `warlock-engine`, and never back**. This crate knows terminal and keystroke vocabulary; the engine knows neither, and never depends on this crate. `warlock-engine` is a `path` dependency. `notify`, `ratatui`, and `serde_json` are the crate's other direct dependencies; crossterm is deliberately not among them — code uses `ratatui::crossterm`, Ratatui's own re-export of the backend it was built against, so frontend and backend can never skew apart. `tempfile` is a dev-dependency only, letting tests that write config write into a throwaway home directory instead of the developer's real one — the promise the engine's explicit home parameter exists to make keepable. Lint configuration lives in the root manifest's `[workspace.lints]`, applied here via `[lints] workspace = true`.

Within `src/` (summarized there, not restated here), the split is between a pure core — flattened/filtered view state, rendering, input translation, colour rules, the panel's three cards, the modal windows, brief-mode template support, and pact/scope orchestration as data transformations — and a thin impure shell, of which only `claude.rs` sits in `src/` itself (the sole process-spawning point, home to both `ClaudeAgent` and `ChatAgent`); the rest of the binary's impure surface (terminal lifecycle, the filesystem watcher, keyboard/mouse event loop, and disk writes) lives in the binary's own files, per `src/`'s document. That split is what lets minutes-long pacts and a whole UI be driven through tests in microseconds against in-memory buffers and hand-fed instants.

## What a reader has to know before changing anything

- **Domain logic belongs in the engine, never here.** What a pact means, how freshness is judged, prompt composition, and what makes a scope or sigil valid are engine questions.
- **View state is never persisted.** `src/`'s document describes the modal windows (quit confirmation, scope/write prompt, composer) as never being fields on `App` — load-bearing, since it's what makes "answering No/Esc leaves the app untouched" true by construction rather than careful restoration.
- **The manifest is `.warlock/pacts.toml`, one per repository, committed to git**, saved once per run via the engine's atomic save, never once per directory.
- **The purity boundary is the whole design of `src/`.** Only `claude.rs` spawns a process here; nearly everything else takes the current instant or terminal size as a parameter rather than reading a clock or a tty internally.
- **Two agents share the one process-spawning seam.** `ClaudeAgent` implements the engine's `Agent` port for pacts; `ChatAgent` is a sibling used for read-only, session-persistent conversation turns.
- **A pact running behind a conversation must never leak into it, and vice versa** — the account card and the thread card are kept strictly separate, per `src/`'s document.
- **This is not an IDE.** The panel is a freshness ledger with a narrow composer and a conversation bolted on. Check any addition against that register before building it.

## `src/`

Summarized in its own document, not restated here: a pure core (`app.rs`'s `App`, the pure renderer `ui.rs`, shared line-wrapping `wrap.rs`, colour rules `colour.rs`, the panel's `Account`/`Log` and `Thread`/`Turn` cards, input-translation types consumed by the binary's `input.rs`, and the three modal-state modules `composer.rs`/`confirm.rs`/`prompt.rs`) around the one impure file in this directory, `claude.rs` — the sole process-spawning point, hosting both `ClaudeAgent` and `ChatAgent`, with a private `stream` submodule turning `stream-json` output into activity values. Supporting modules classify submitted drafts (`submission.rs`) and own the brief template and its completeness check (`template.rs`). A private, test-only fixture (`fixture.rs`) supplies a hand-written tree so this crate's own tests need no real filesystem. `app.rs` and `ui.rs` were too large to send in full and were summarized rather than sent, so non-trivial changes to `App` or the drawing code call for reading those two files directly first. No `WARLOCK.md` for a subdirectory of `src` was supplied with this request, so nothing below `src/` is described here.