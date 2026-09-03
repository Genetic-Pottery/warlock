# src

This directory is `warlock`'s terminal front-end binary and its rendering/state library, built together at the crate root. It is one crate with two blended halves — a `warlock-tui` library (pure state, drawing, and key/mouse translation, no terminal I/O) and the `warlock` binary (the event loop, subcommand dispatch, and the four impure seams: the terminal, the `claude` child process, the filesystem watcher, and Ctrl-C). The dependency runs one way: the binary's modules (`main.rs`, `pacting.rs`, `chatting.rs`, `scoping.rs`, `viewing.rs`, `editing.rs`, `writing.rs`, `session.rs`, `terminal.rs`, `input.rs`, `error.rs`, `config.rs`, `check.rs`, `query.rs`, `running.rs`, `edits.rs`) depend on the library modules (`app.rs`, `account.rs`, `thread.rs`, `colour.rs`, `composer.rs`, `confirm.rs`, `prompt.rs`, `submission.rs`, `template.rs`, `claude.rs`, `ui.rs`, `wrap.rs`, `watch.rs`, `lib.rs`), never the reverse — and the library knows nothing about terminals; the binary owns raw mode, the alternate screen, and process spawning.

## The governing idea

Almost everything here is a value and a pure function over it: given a key event and a situation, `input.rs`'s `action_for`/`press_for` say what happens; given an app and an event, the account/thread/composer/confirm/prompt types say what changes; `ui.rs`'s `draw` is a pure function of app state, chrome, and an instant. The only impure things are named and kept small: `claude.rs` spawns `claude` as a child process (the sole place anything is spawned), `terminal.rs` owns raw-mode/alternate-screen/mouse-capture lifecycle, `watch.rs`'s `Watch` owns a `notify` handle, and `main.rs`'s `listening()` installs a Ctrl-C handler. Everything downstream of those is data, so almost the entire test suite drives a whole pact, a whole chat turn, or a whole keystroke without a terminal, network, or real `claude`.

## The tree/pact/scope/document machinery (library side)

- **`app.rs`** — `App`: the flattened, filterable, collapsible view of the loaded `Tree` (rows, selection, scroll, focus, pacted-only/show-files toggles), plus the three-card `Panel` (account/thread/document) and `reseat_on` for carrying view state across a reload.
- **`account.rs`** — `Account`/`Section`/`Log`: the ledger of one pact run — one section per directory, one clocked line per activity, freezing/ticking rules shared with `thread.rs`'s turns.
- **`thread.rs`** — `Thread`/`Turn`/`Ending`: the chat conversation card — questions, work lines, answers, and warlock's own unclocked `Note` entries, interleaved in one sequence.
- **`claude.rs`** — the whole model-pass transport: `ClaudeAgent` (implements the engine's `Agent` port by running `claude`) and `ChatAgent` (a read-only, session-persistent chat agent used for turns); also owns `Cancel`/`Activities`, the stream-JSON parser, and the byte/size formatting shared with `account.rs`.
- **`colour.rs`** — the fixed three-state (+ focus/guide) colour palette, pinned as indexed-256 rather than themed.
- **`composer.rs`**, **`confirm.rs`**, **`prompt.rs`** — the three keyboard-owning overlays: the multi-line draft field, the quit confirmation, and the scope/write text prompt — each a value plus a pure `*_for(key, state) -> outcome` function.
- **`submission.rs`** — classifies a submitted composer draft into `/brief`, `/write`, `/chat`, an ordinary message, or a refusal.
- **`template.rs`** — reads (never caches) a repository's `.warlock/brief-template.md`, falling back to a compiled-in `DEFAULT_TEMPLATE`, and checks a written document against it (`missing_sections`).
- **`ui.rs`** — the whole rendering layer: layout, hit-testing, every pane's draw function, and (per its summary) an extensive test suite driving an in-memory backend.
- **`wrap.rs`** — breaks one drawn `Line` into panel-width rows, shared by every card.
- **`watch.rs`** — `Watch` (impure, hears filesystem events) and `WatchPolicy`/`NodeSet` (pure: which paths matter, when a reload is owed).
- **`lib.rs`** — the crate root: extensive module docs plus the re-export surface consumed by the binary.

## The binary: dispatch, keys, and the two runs

- **`main.rs`** — argument parsing (`clap`), subcommand dispatch, the event loop itself (poll, key/mouse handling via `input.rs`, `keep_up` draining), and `status_for`'s exit-code contract (0/1/2/3/4/130).
- **`input.rs`** — `action_for` (key → `Action`), `press_for` (the gate: Ctrl-C, quit confirmation, the three overlay windows, the composer, then the tree), and `mouse_action`/`hit_test` glue.
- **`pacting.rs`** — the pact/refresh worker: spawns a thread running the engine's `pact_subtree`/`refresh_subtree`, reports over a channel into the `Account`, and is the sole writer of the panel's run card; shared verbatim in shape with **`running.rs`**, which is the headless `warlock pact`/`warlock refresh` subcommand built on the same `Progress` observer, signal handling, and one-save-at-the-end rule.
- **`chatting.rs`** — the chat turn worker (mirrors `pacting.rs`'s shape at smaller scale): spawns `ChatAgent` turns, drains into `Thread`, and drives `/brief`/`/write`/`/chat` mode transitions.
- **`scoping.rs`** — the `s` key: opens the scope prompt, judges and writes a scope via `validate_scope`, on the loop's own thread (no worker, no reload).
- **`viewing.rs`** — the `v` key: a capped, synchronous file read into the document card.
- **`editing.rs`** — the `e` key: hands a file to `$EDITOR`, suspending/resuming the terminal via `terminal.rs`, then reloads the tree and (conditionally) the document card.
- **`writing.rs`** — the `/write` artifact: proposes a path (`proposed_path`, slug/number/fence logic) and writes it (`write_submit`) after checking template-section coverage, refusing to overwrite.
- **`session.rs`** — `Scope` (the two resolved roots + `Chrome`), `reload_tree`, `Watched` (binds `watch.rs`'s policy to the loop), `closed_scope`/`sigils_under` (the boundary-check machinery shared by every scoped key and subcommand).
- **`terminal.rs`** — `TerminalGuard` (raw mode/alt screen/mouse capture lifecycle, including suspend/resume for `$EDITOR`) and the panic hook.
- **`error.rs`** — `Error`, the binary's single one-line error vocabulary, and `one_line` (flattening multi-line diagnostics), used everywhere a failure reaches stderr or the footer.
- **`config.rs`**, **`check.rs`**, **`query.rs`** — the read-only subcommands: `warlock config` (sigil prompt), `warlock check` (scope/sigil verdict), `warlock stale`/`fresh` (listings), all sharing `envelope`/`print_object` JSON shaping.
- **`edits.rs`** — the headless writes (`warlock unpact`, `warlock scope add/remove`): `Opened`, the boundary-gated entry point shared with `running.rs`, enforcing that the scope check happens before any manifest read.

## Boundary and scope

Per the repo's own `CLAUDE.md`, the boundary/scope/sigil design (who may pact, refresh, un-pact, or write a scope over a given directory) is argued at length in `docs/`, and this directory is where it is *enforced*: `session.rs::closed_scope` is the single check point, reused by `pacting.rs`'s `p`/`r` keys, `scoping.rs`'s `s` key, and `edits.rs`/`running.rs`'s headless subcommands, so the TUI and the shell refuse one rule in one wording.

## What a reader needs before changing anything here

- Almost every module's own doc comment argues its design at length (why a value here, why not a field there, why a cost is heard-and-discarded, etc.); read the module you're editing before assuming a shortcut is safe — these files were written to make the "why not the obvious thing" explicit.
- The pure/impure split is load-bearing for testability: new functionality should default to being a value + pure function, with impurity pushed to one of the four existing seams (`claude.rs`, `terminal.rs`, `watch.rs`, the signal handler in `main.rs`/`running.rs`) rather than a fifth one.
- `pacting.rs` and `chatting.rs` are deliberately parallel in shape (spawn → channel → drain → one save/no save); a change to one's worker lifecycle likely has a matching change on the other side to make.
- The exit-status contract (`main.rs::status_for`) and the boundary-refusal wording (`session.rs`) are each asserted from both the TUI and the headless subcommands in cross-checking tests; changing either sentence or status number means updating both call sites' tests, not just one.