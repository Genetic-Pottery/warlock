# src

This is `warlock_tui`'s library crate: the terminal front end's pure core, plus the one binary (`main.rs`) that turns it into a running program. Everything except `claude.rs` and `terminal.rs`/`watch.rs`'s watcher handle is data and functions over data — no terminal, no clock read internally, no process spawned — which is what lets the whole UI be tested against in-memory buffers and hand-driven instants.

## What is here

- **`lib.rs`** — the crate's manifest of what it exports and why; read it first for the shape of the whole thing. States the one real architectural rule: TUI depends on `warlock_engine`, never the reverse.
- **`app.rs`** — `App`, the front end's state: a flattened, filtered, scrollable view over the engine's `Tree`, plus `reseat_on` for carrying that view across a freshly reloaded tree by path rather than by row index.
- **`account.rs`** — `Account`, the running record of one pact: sections per directory, clocked lines per thing a pass did. Holds no clock of its own; every clock is computed against an `Instant` handed in by the caller.
- **`claude.rs`** — the *only* place in the codebase that spawns a child process. Runs the `claude` CLI as the engine's `Agent` implementation, with a documented multi-thread arrangement (writer/reader/waiter threads) to avoid pipe deadlocks and make a running child killable from another thread via `Cancel`.
- **`colour.rs`** — the fixed, hand-picked mapping from `NodeState` (and focus/guide lines) to terminal colours. Deliberately indexed (not theme-respecting) colours, because on this screen colour *is* the message.
- **`ui.rs`** — pure rendering: `draw` builds one frame from an `App`, an `Account`, and an `Instant`; `hit_test` maps a screen point back to a named region for mouse input.
- **`watch.rs`** — filesystem-watching split into an impure handle (`Watch`, wraps `notify`) and two pure decision types: `NodeSet` (which paths are Warlock's business — exactly the last walk's directories) and `WatchPolicy` (when a reload is owed — debounce + ceiling + one coalesced reload).
- **`error.rs`** — `Error`, the binary's whole error vocabulary; every variant renders as one line via `one_line`, because `main` prints exactly one line to a restored terminal.
- **`fixture.rs`** — test-only, hand-written `Tree` values (`tree`, `tree_after_a_run`) that `app.rs` and `ui.rs` tests assert exact output against. Not a loader or a fallback; never reachable outside `#[cfg(test)]`.
- **`main.rs`** — the binary. Owns the terminal's lifecycle, the event loop, and nothing else. Long module doc explains the concurrency shape: a pact runs on a worker thread, progress arrives over a channel drained once per poll round, and the loop never blocks on it.
- **`input.rs`** — pure translation from raw crossterm key/mouse events to `Action`/`MouseAction` enums; no terminal I/O.
- **`pacting.rs`** — orchestrates a pact run: spawns the worker thread, drives the engine's `pact_subtree`/`unpact_subtree`, translates worker events into `Account` updates.
- **`session.rs`** — `Scope` (root + repo root, resolved once at startup), `reload_tree` (the *only* place the tree is re-read, and only from the event loop's thread), `Watched` (glues `Watch`/`WatchPolicy` to the loop), and manifest loading.
- **`terminal.rs`** — `TerminalGuard` and `install_panic_hook`: the one guaranteed spelling of "restore the terminal," run on every exit path including a panic.

## What a reader has to know before changing anything

**The purity boundary is the whole design.** Only `claude.rs` spawns a process; only `main.rs`/`terminal.rs`/`watch.rs`'s `Watch` touch a real terminal, clock, or OS watcher handle. Everything else — `App`, `Account`, `ui::draw`, `colour_for`, `WatchPolicy`, `input`'s translation functions — is a pure function of its arguments, including the current instant, which is *always* passed in rather than read via `Instant::now()` inside these modules. This is what lets minutes-long pacts and multi-second debounce windows be tested in microseconds. Don't reach for a clock or a terminal call inside these files; thread the value through instead.

**Two clocks, one rule.** Both `Account`'s per-line clocks and `WatchPolicy`'s debounce/ceiling timers follow the same discipline: freeze on the next event, tick against `now` only while still "live." Understand `account.rs`'s doc comment on this before touching either.

**Rows vs. tree.** `App` keeps `all_rows` (everything) and `rows` (filtered/collapsed view), rebuilt by `reflow` whenever filters or collapse state change; collapsed state is tracked by *path*, not row index, so it survives a rebuild. `reseat_on` is the one path for putting a view back onto a newly-loaded tree — selection, collapse, filters, and scroll all carry across by path with ancestor fallback.

**Footer/panel messaging has one precedence rule**, enforced by `session::note`: a housekeeping line (reload failure, watcher-off notice) never overwrites whatever a pact run already said, because the pact's outcome is the news and everything else is bookkeeping.

**Cancel vs. quit are different operations.** Esc cancels a pact (worker finishes writing/hashing/saving what's already done); `q`/Ctrl-C quits outright (drops the `Cancel` handle, kills the child, the manifest rewrite is simply skipped). Both route through one `Cancel`, dropped via `CancelGuard`, so every exit path — including a panic — takes the child process down with it.

**Colours are fixed and few on purpose**: exactly three state colours plus a focus colour plus a guide colour, all indexed rather than theme-respecting, all asserted pairwise-distinct in `colour.rs`'s tests. Adding a `NodeState` variant will fail `colour_for`'s exhaustive match at compile time — that's intentional; don't add a fallback arm.

**This is not an IDE.** No prompt box, no free-text input; the panel is a freshness ledger (facts about what a pass did), not a chat transcript or a dashboard — no tokens, no turn counts shown, deliberately.