# src

This is `warlock_tui`'s library crate: the terminal front end's pure core, plus the `main.rs` binary that turns it into a running program, the two modal gates that sit in front of it (confirm-on-quit, scope prompt), and the session/pact/scoping plumbing that wires them together. Almost everything here is a pure function over data — no terminal, no clock read internally, no process spawned — which is what lets minutes-long pacts and a whole UI be driven through tests in microseconds against in-memory buffers and hand-fed instants.

## What is here

- **`lib.rs`** — the crate's manifest of what it exports; read it first. States the one hard rule: TUI depends on `warlock_engine`, never the reverse.
- **`app.rs`** — `App`: the flattened, filtered, scrollable view over the engine's `Tree` (`all_rows` vs. `rows`), plus `reseat_on` for carrying that view across a freshly reloaded tree by path rather than row index. Also `Chrome` (fixed per-session header), `Row`, `Focus`, `Sigils`, `PactToggle`, `Run`.
- **`account.rs`** — `Account`, the running record of one pact's activity, clocked against an `Instant` handed in by the caller rather than read internally.
- **`claude.rs`** — the *only* place in the codebase that spawns a child process: runs the `claude` CLI as the engine's `Agent`, with a documented multi-thread writer/reader/waiter arrangement and a `Cancel` handle to kill a running child from elsewhere.
- **`colour.rs`** — the fixed, hand-picked, exhaustively-matched mapping from `NodeState`/focus/guide lines to indexed terminal colours, deliberately theme-ignoring because colour *is* the message here.
- **`confirm.rs`** — the quit-confirmation gate (`QuitConfirm`/`Answer`/`Answered`, `answer_for`). Deliberately not a field on `App`, so answering No leaves the app untouched. Ctrl-C bypasses this gate entirely; the event loop must answer it first.
- **`prompt.rs`** — the scope-entry field's sibling gate (`ScopePrompt`/`ScopeField`/`Edited`, `edit_for`). Append/backspace/submit/close only, cursor always at the end; judges nothing about scope validity — that's the engine's job.
- **`scoping.rs`** — binary-side glue from keystroke to manifest write for the scope key: `scope_press`, `scope_edit`, `scope_submit`. Runs synchronously on the event loop's own thread — no worker, no reload — because a scope is one string on one manifest entry.
- **`ui.rs`** — pure rendering: `draw` builds one frame from `App`, `Account`, an `Instant`, `QuitConfirm`, and `ScopePrompt`; `hit_test` maps a screen point back to a named region.
- **`watch.rs`** — filesystem watching split into an impure handle (`Watch`, wraps `notify`) and two pure decision types: `NodeSet` (which paths matter) and `WatchPolicy` (when a reload is owed).
- **`error.rs`** — `Error`, the binary's whole error vocabulary; every variant renders as one line via `one_line`, shared by `main`, `pacting`, `session`, `config`, `scoping`.
- **`fixture.rs`** — test-only, hand-written `Tree` values (`tree`, `tree_after_a_run`) that other modules' tests assert exact output against; unreachable outside `#[cfg(test)]`.
- **`main.rs`** — the binary: terminal lifecycle, argument dispatch (`init`/`config`/`-h`/`--help`/else refused), the event loop, and the concurrency shape (pact on a worker thread, progress drained once per poll round, loop never blocks on it).
- **`input.rs`** — pure translation from crossterm events to `Action`/`MouseAction`, plus `press_for` (layers the quit gate in front of `action_for`) and `mouse_action` (goes inert while either modal gate is open).
- **`pacting.rs`** — orchestrates a pact/refresh run: spawns the worker thread, drives `pact_subtree`/`refresh_subtree`/`unpact_subtree`, translates worker events into `Account` updates, reloads the tree exactly once at the end.
- **`viewing.rs`** — the `v` key: reads a file into the panel's document card on the event loop's own thread. No worker, no channel, no reload — a read races nothing and finishes inside a frame. Never refused for an in-flight run, unlike pact/refresh/scope.
- **`editing.rs`** — the `e` key: hands the terminal to `$EDITOR` via `TerminalGuard::suspended` and re-reads the file afterward if it's the one on the document card.
- **`session.rs`** — `Scope` (root + repo root + `Chrome`, resolved once), `reload_tree` (the only place the tree is re-read, only from the event loop's thread), `Watched` (glues `Watch`/`WatchPolicy` to the loop), `sigils_held`/`sigils_under` (read once, tolerate a broken config as `Sigils::Unknown`), and manifest loading.
- **`config.rs`** — `warlock config`: prints what sigils this machine holds, reads one line from stdin in cooked mode, writes the machine-local sigil config under the home directory — never inside the repository. `home_directory()` is the crate's one point where `HOME`/`USERPROFILE` becomes a path.
- **`terminal.rs`** — `TerminalGuard` and `install_panic_hook`: the one guaranteed spelling of "restore the terminal" on every exit path, plus `suspended`, which gives the terminal to a foreign process (an editor) and takes it back.
- **`wrap.rs`** — `wrapped`, breaking one document line into as many panel rows as it needs (word-break, then mid-word fallback) — distinct from an account's lines, which are never wrapped, only ellipsis-cut.

## What a reader has to know before changing anything

**The purity boundary is the whole design.** Only `claude.rs` spawns a process; only `main.rs`/`terminal.rs`/`watch.rs`'s `Watch`/`config.rs`'s stdin touch a real terminal, clock, OS watcher handle, or interactive stdin. Everything else is a pure function of its arguments, including the current instant, which is always passed in rather than read via `Instant::now()` internally. Don't reach for a clock or a terminal call inside these files; thread the value through instead.

**Two clocks, one rule.** `Account`'s per-line clocks and `WatchPolicy`'s debounce/ceiling timers both freeze on the next event and tick against `now` only while still "live." Read `account.rs`'s doc comment before touching either.

**Rows vs. tree.** `App` keeps `all_rows` and a filtered/collapsed `rows`, rebuilt by `reflow`. Collapsed state is tracked by path, not row index. `reseat_on` is the one path for putting a view back onto a newly-loaded tree; the header (`Chrome`) is deliberately *not* carried by a reload — it's fixed at session start on `Scope` since neither the tree's identity nor held sigils can change while warlock runs.

**Footer messaging has one precedence rule**, enforced by `session::note`: a housekeeping line (reload failure, watcher-off notice) never overwrites a pact's own message, because the pact's outcome is the news.

**Two modal gates layer on top of Esc/`q`/`s`, plus cancel/quit.** Esc cancels a pact when one is running; with nothing running, Esc/`q` open the quit confirmation, and only Yes actually quits. `s` opens the scope prompt, refused (not ignored) when a run is in flight. Ctrl-C always leaves immediately, bypassing both gates — it must be caught before either gate would swallow it as an inert character-with-modifier. Neither gate is a field on `App`, deliberately: answering No/Esc must leave the app byte-for-byte untouched, and the cheapest guarantee of that is the app never having heard the question was asked.

**A scope is not a pact.** `scope_submit` runs synchronously between two frames — no worker, no channel, no account, no reload — since it's one string on one manifest entry, and it never touches the tree's freshness state.

**A view (`v`) is not a pact either**, and unlike `p`/`r`/`s` it is never refused for an in-flight run — a read races nothing. Failures (unreadable, vanished, not text) are one footer line each and never disturb whatever the panel was already showing.

**Colours are fixed and few on purpose**: three state colours plus a focus colour plus a guide colour, all indexed, all asserted pairwise-distinct. Adding a `NodeState` variant will fail `colour_for`'s exhaustive match at compile time — don't add a fallback arm.

**Mouse capture** is a fact the terminal keeps (toggled by `m`), copied to `App` each frame purely so the footer can name the key, but sourced from the terminal because `App` gets swapped/restored around a pact run.

**`editing.rs`'s `TerminalGuard::suspended`** is the one place warlock hands the screen to someone else and takes it back; it repaints by resizing rather than diffing, since the child left arbitrary content behind.

**Scopes vs. sigils are judged in one place each.** `scope.rs`/`config.rs` both defer to the engine's `validate_scope`/`validate_sigil`; no length constant or character predicate is duplicated here. Both fold typed text to lowercase before judging, in the same place a person supplies a string.

**This is not an IDE.** No prompt box, no free-text input beyond the two narrow fields (scope text, sigil line); the panel is a freshness ledger, not a chat transcript or dashboard.