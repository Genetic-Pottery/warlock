# src

This is `warlock_tui`'s library crate: the pure core of the terminal front end, plus `main.rs`'s binary shell around it. Three modal gates (confirm-on-quit, scope prompt, write-path prompt) and the session/pact/scoping plumbing wire it together. Almost everything here is a pure function over data — no terminal, no clock read internally, no process spawned — which is what lets minutes-long pacts and a whole UI be driven through tests in microseconds against in-memory buffers and hand-fed instants.

## What is here

- **`lib.rs`** — the crate's manifest of exports; states the one hard rule: TUI depends on `warlock_engine`, never the reverse.
- **`app.rs`** — `App`: the flattened, filtered, scrollable view over the engine's `Tree`, plus `reseat_on` for carrying that view across a freshly reloaded tree by path. Also `Chrome`, `Row`, `Focus`, `Sigils`, `PactToggle`, `Run`.
- **`account.rs`** — `Account`, the running record of one pact's activity, clocked against a caller-supplied `Instant`.
- **`chatting.rs`** — `Chat`: the whole conversation as one value — the agent, the turn in flight, the draft at the foot of the panel, where a brief would go, and the window a `/write` answer opens. Underneath it the same not-blocking shape as `pacting.rs` (`spawn_turn`, `start_turn`, `run_turn`, `apply_turn`) over a smaller job that writes no documents and reloads no tree.
- **`claude.rs`** — the *only* place that spawns a child process: runs the `claude` CLI as both the engine's `Agent` (`ClaudeAgent`, for pacts) and a read-only chat agent (`ChatAgent`, for conversation turns).
- **`template.rs`** — `brief_template`: the shape a brief-mode conversation is aimed at, read fresh at the keystroke — warlock's built-in `DEFAULT_TEMPLATE`, or `<root>/.warlock/brief-template.md` where the repository has written one instead.
- **`colour.rs`** — the fixed, exhaustively-matched mapping from `NodeState`/focus/guide to indexed terminal colours.
- **`composer.rs`** — the composer: the several-line draft field at the foot of the panel, and `compose_for`, which lets every single-letter command go back to being a letter while it holds the keyboard.
- **`submission.rs`** — `submitted_for`/`Submitted`: what a submitted draft turns out to be — one of the three slash commands, an ordinary message for the model, or a refusal that words its own line.
- **`confirm.rs`** — the quit-confirmation gate (`QuitConfirm`/`Answer`/`Answered`, `answer_for`). Not a field on `App`, so answering No leaves the app untouched. Ctrl-C bypasses it.
- **`prompt.rs`** — the scope-entry field's sibling gate (`ScopePrompt`/`ScopeField`/`Edited`, `edit_for`). Judges nothing about scope validity.
- **`scoping.rs`** — binary-side glue from keystroke to manifest write for the scope key: `scope_press`, `scope_edit`, `scope_submit`. Runs synchronously on the event loop's own thread.
- **`ui.rs`** — pure rendering: `draw` builds one frame from `App`, `Account`, an `Instant`, `QuitConfirm`, `ScopePrompt`, and the composer; `hit_test` maps a screen point back to a named region.
- **`watch.rs`** — filesystem watching split into an impure handle (`Watch`, wraps `notify`) and two pure decision types: `NodeSet` and `WatchPolicy`.
- **`error.rs`** — `Error`, the binary's whole error vocabulary; every variant renders as one line via `one_line`.
- **`fixture.rs`** — test-only, hand-written `Tree` values; unreachable outside `#[cfg(test)]`.
- **`main.rs`** — the binary: terminal lifecycle, argument dispatch (`init`/`config`/`-h`/`--help`/else refused), the event loop, and its concurrency shape (pact/turn on worker threads, progress drained once per poll round).
- **`input.rs`** — pure translation from crossterm events to `Action`/`MouseAction`/`Pressed`; `press_for` layers both modal gates and the composer in front of `action_for`.
- **`pacting.rs`** — `Pact`: the agent runs are made with and the run in flight, with `press`/`keep_up`/`stop`/`running` — `Chat`'s opposite number. Spawns the worker thread, drives `pact_subtree`/`refresh_subtree`/`unpact_subtree`, translates worker events into `Account` updates, reloads the tree once at the end.
- **`viewing.rs`** — the `v` key: reads a file into the panel's document card on the event loop's own thread. Never refused for an in-flight run, unlike pact/refresh/scope.
- **`editing.rs`** — the `e` key: hands the terminal to `$EDITOR` via `TerminalGuard::suspended` and re-reads the file afterward if it's the one on the document card; also re-reads the tree, since an edited `WARLOCK.md` restales its own directory.
- **`writing.rs`** — the `/write` artifact: `proposed_path` derives where a brief would go, while `write_opened`/`write_edit`/`write_submit` are the window's keystrokes and the one place warlock puts a document on disk.
- **`session.rs`** — `Scope` (root + repo root + `Chrome`, resolved once), `reload_tree`, `Watched`, `sigils_held`/`sigils_under`, manifest loading.
- **`config.rs`** — `warlock config`: prints what sigils this machine holds, reads one line from stdin in cooked mode, writes the machine-local sigil config under the home directory.
- **`terminal.rs`** — `TerminalGuard` and `install_panic_hook`: the one guaranteed spelling of "restore the terminal" on every exit path, plus `suspended`, which gives the terminal to a foreign process and takes it back.
- **`thread.rs`** — `Thread`/`Turn`/`Ending`: the panel's third card, the conversation.
- **`wrap.rs`** — `wrapped`/`shape`/`rows`: breaks one card's lines into as many panel rows as they need at a given width.

## What a reader has to know before changing anything

**The purity boundary is the whole design.** Only `claude.rs` spawns a process; only `main.rs`/`terminal.rs`/`watch.rs`'s `Watch`/`config.rs`'s stdin touch a real terminal, clock, OS watcher handle, or interactive stdin. Everything else is a pure function of its arguments, including the current instant, always passed in rather than read via `Instant::now()`.

**Two clocks, one rule.** `Account`'s per-line clocks and `WatchPolicy`'s debounce/ceiling timers both freeze on the next event and tick against `now` only while still "live." `Thread`'s turns share the very same `Log` machinery as `Account`.

**Two agents, one seam, and one value apiece.** `ClaudeAgent` implements the engine's `Agent` port for pacts and lives inside `Pact`; `ChatAgent` is a sibling that is *not* an `Agent` and lives inside `Chat` — a turn is a typed message, not a request, one session for the process's life, granted only `Read`/`Grep`/`Glob`, never able to write.

**A run can happen inside a conversation.** A pact or refresh started while the thread is on screen appends as a turn nobody typed, holding the very `Account` the account card holds for the same run.

**Every keystroke that starts a run refuses to start a second one.** `p` and `r` go through `Pact::press`, which reads the one run `Pact` holds before it decides anything; `s` checks the same fact through `Pact::running`.

**Footer messaging has one precedence rule**, enforced by `session::note`: housekeeping gives way to whatever a pact's own message says.

**Modal gates and the composer layer in a fixed order** (documented on `input::press_for`): Ctrl-C first, then the quit question, then the scope prompt, then the write-path prompt, then the composer, then the ordinary keys.

**A view (`v`) and a swap are not pacts**, and unlike `p`/`r`/`s` are never refused for an in-flight run.

**Colours are fixed and few on purpose**: three state colours plus focus plus guide, all indexed and exhaustively matched. Adding a `NodeState` variant fails compilation deliberately.

**This is not an IDE.** No prompt box beyond the narrow composer, no chat vocabulary bleeding into the account's lines or vice versa; the panel is a freshness ledger with a conversation bolted on, not a dashboard.