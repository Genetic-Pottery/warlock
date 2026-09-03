<!-- warlock -->
> Written by a model pass over this directory alone, to be read before its source and to say which source to read. A map, not a specification: check anything you are about to rely on against the files themselves, and where this document and the code disagree, the code is right.

# src

This is the `warlock-tui` crate's source directory: the terminal front end, minus the terminal itself. It compiles to a library (`lib.rs`) consumed by the binary crate (`main.rs`, `stubs.rs`, and the modules `main.rs` declares as its own — `input.rs`, `pacting.rs`, `chatting.rs`, `scoping.rs`, `viewing.rs`, `editing.rs`, `writing.rs`, `session.rs`, `terminal.rs`, `error.rs`, `boundary.rs`, `standing.rs`, `edits.rs`, `running.rs`, `query.rs`, `check.rs`, `config.rs`, `descent.rs`). The library half (declared by `lib.rs`) is data and pure functions — the tree flattened to rows, colours, wrapping, key-to-intent translation — testable with nothing attached to stdout. The binary half is the impure shell: terminal lifecycle, the event loop, process spawning, and the CLI subcommands.

Per this repository's own convention, everything below is a description of what these files do as of the pass that wrote it, not a claim about what they *should* do — see `CLAUDE.md`.

## The library crate (declared and re-exported by `lib.rs`)

**`lib.rs`** — the crate root. An extensive module doc comment explains the overall design (the flattened-row `App`, the ticking-clock `Account`/`Thread` model, `WatchPolicy`'s timing rules, the two `claude`-running agents, the gate types for the quit confirmation/scope prompt/composer), declares every module (`account`, `app`, `claude`, `colour`, `composer`, `confirm`, `fixture` (test-only), `panel`, `prompt`, `submission`, `template`, `thread`, `ui`, `watch`, `wrap`), and re-exports the public surface each of those modules offers, each with its own doc line.

**`account.rs`** — `Account`, the record of one pact run: a `Section` per directory, a `Log` of timestamped lines per section, with the "ticking clock" rule (only the newest line in an open section counts up against a caller-supplied `now`). Defines `Outcome` (how a directory's pass ended), the crate-wide `Line` enum (every drawable row type across the whole panel), and formatting helpers (`clock`, `money`, `size`, `tool_line`, `writing_line`).

**`app.rs`** — `App`, the front end's central state: the flattened `Row`s, which are collapsed, the selection, scroll offset, filters (`pacted_only`, `show_files`), `Focus` (tree/panel/composer), an in-flight run tracker (`InFlight`/`Run`/`RunHeader`/`Summarising`), `Sigils`/`Chrome` (what this machine holds, resolved once), and the `Panel` (imported from `panel.rs`). Also defines `PactToggle` and the free function `reseat_on`, which carries a view's state across a freshly reloaded `Tree`.

**`claude.rs`** — the transport layer: the only place in the crate that spawns a `claude` CLI child process. Defines `ClaudeAgent` (implements the engine's `Agent` port for a pact/refresh pass) and `ChatAgent` (implements `Converses` for a conversational turn, read-only tools). Handles pipe deadlock avoidance, cancellation (`Cancel`/`CancelGuard`), streaming activity reporting (`Activity`/`Activities`), and session continuity for chat.

**`colour.rs`** — `colour_for`, the one total match from `NodeState` to a pinned 256-colour index (gray/orange-yellow/green for unpacted/stale/fresh), plus `FOCUS_COLOUR` and `GUIDE_COLOUR`, both deliberately outside that palette.

**`composer.rs`** — `Composer`, the multi-line draft field at the foot of the panel, and `compose_for`, the pure function mapping a key event to a `Composed` outcome (`Typing`/`Leave`/`Submit`). Owns its own height/windowing arithmetic (`COMPOSER_MAX_ROWS`).

**`confirm.rs`** — `QuitConfirm`/`Answer`, the quit-confirmation dialog state (kept deliberately outside `App`), and `answer_for`, the pure key-to-`Answered` function.

**`fixture.rs`** (test-only, `#[cfg(test)]`, not compiled into the released library) — two hand-written `Tree` values (`tree()` and `tree_after_a_run()`) used by this crate's own tests so assertions don't depend on a real repository on disk.

**`panel.rs`** — `Panel`, holding the three cards (`Account`, `Thread`, `Document`) of which one is shown (`Showing`) at a time, plus `Mode` (Chat/Brief) and the generic `Card<T>` windowing type shared by all three cards.

**`prompt.rs`** — `ScopePrompt`/`ScopeField`, the `s`-key scope-editing prompt state (again kept outside `App`), and `edit_for`, its pure key-handling function. Judges nothing about scope validity itself.

**`submission.rs`** — `submitted_for`, the pure function classifying a composer draft as one of warlock's three slash commands (`/brief`, `/write`, `/chat`), an ordinary message, or a `Refused` draft with one canned refusal line.

**`template.rs`** — `brief_template`, reading `.warlock/brief-template.md` from disk on every call (or falling back to the compiled-in `DEFAULT_TEMPLATE`), and `missing_sections`, checking a written document against a template's `## ` headings.

**`thread.rs`** — `Thread`/`Turn`/`Ending`, the chat conversation model: an ordered sequence of turns (question, observed work, answer) and notes (warlock's own unclocked lines). `ending_for` maps engine `agent::Error`s to `Ending` variants.

**`ui.rs`** — the rendering layer: `draw` (the top-level frame entry point), all layout/sizing functions (`areas`, `tree_height`, `panel_height`, `composer_height`, etc.), `hit_test`/`Hit` for pointer-to-region mapping, and every draw function for the tree pane, panel, header, footer, quit-confirmation window, and scope/write prompt window. By byte count this is the largest file in the crate.

**`watch.rs`** — `Watch` (the impure filesystem-watcher handle), `NodeSet` (which directories are "warlock's business" — the last walk itself), and `WatchPolicy` (the pure timing logic: `QUIET_PERIOD`, `RELOAD_CEILING`, `COALESCED_RELOADS`), plus `Watching` (a watcher or the reason there is none).

**`wrap.rs`** — `wrapped`/`rows`, breaking one card line into the rows a panel of a given width can draw, preserving the drawing "shape" (prefix/marker) of continuation rows.

## The binary crate

**`main.rs`** — the `warlock` executable's entry point: CLI parsing (`Cli`/`Command`/`ScopeCommand` via `clap`), `main`, exit-status mapping (`status_for`), the `init` subcommand, the interactive `run()` loop, and `Session<S, P, C>` — the generic struct (parameterized over `Screen`/`Agent`/`Converses` for testability) that replaces the eleven loose locals the event loop used to thread through separately, with methods `draw`, `press`, `keep_up`.

**`terminal.rs`** — the `Screen` trait (the seam between the real terminal and test doubles) and `TerminalGuard`, the RAII type guaranteeing raw mode/alternate screen/mouse capture are restored on every exit path (normal, error, panic via `install_panic_hook`, or suspension via `Screen::suspended` for `$EDITOR`).

**`input.rs`** — the largest keyboard/mouse-translation module: `Action`/`action_for` (first layer: what a keystroke means as an app command), `Pressed`/`press_for` (second layer: the gate — Ctrl-C, quit confirmation, scope prompt, write prompt, composer — before falling through to `action_for`), and `MouseAction`/`mouse_action` (pointer translation via `hit_test`).

**`pacting.rs`** — the machinery behind the `p`/`r` keys: `Pact<P>`, `Work` (Pact vs Refresh), `Running`/`PactEvent`/`Reporting` (the worker thread and its channel), `pact_press`/`refresh_press` (boundary-checked keystroke handling), and `drain` (draining worker events into the panel each frame). By byte count, the largest source file (non-test) in the binary.

**`chatting.rs`** — `Chat<C>`, the conversational side: the composer, the register (`Mode`), and turns, unifying five previously-separate call sites (draft, submit, `/brief`, `/write`, write-confirmation). Spawns worker threads via `spawn_turn`/`run_turn`, drained via `apply_turn`.

**`scoping.rs`** — the `s`-key path: `scope_press`/`scope_edit`/`scope_submit`, ending in a single synchronous `.warlock/pacts.toml` write (no worker thread, no reload). Shares `with_scope_on` with the headless `warlock scope add`/`remove` commands in `edits.rs`.

**`viewing.rs`** — the `v`-key path: `view_press`, the smallest of the three key modules — a single capped, uncached file read into the panel's document card, refused nowhere by an in-flight run.

**`editing.rs`** — the `e`-key path: `edit_press`, suspending the terminal to run `$EDITOR` as a foreground child (via `Screen::suspended`), and `came_back`, reloading the tree afterward since an edited `WARLOCK.md` restales its own directory.

**`writing.rs`** — the `/write` command's path-proposal and file-writing logic: `proposed_path` (slug + sequence number from a reply's `# ` heading), `write_submit` (the actual write: root-escape/existing-path refusal, section-completeness check via `template.rs`'s `missing_sections`, staleness reporting). By byte count the second-largest file in the binary.

**`session.rs`** — `Scope` (the tree's root and repository root, resolved once), `reload_tree` (the one place the tree is re-read from disk), `Watched` (ties `Watch`/`WatchPolicy` to the event loop's thread), `closed_scope` (the one place `p`/`r`/`s` ask the upward boundary question), and `load_app` (startup: load tree, resolve repo root, read sigils).

**`boundary.rs`** — `verdict`/`Verdict`/`Reach`, the single boundary-decision function shared by the TUI's keys (`session.rs`) and the headless writes (`edits.rs`), plus the two shared refusal-wording functions `closed_scope_message` and `blocking_scopes_message`.

**`standing.rs`** — `Standing`, the "where warlock is" prologue every headless subcommand opens with (working directory, repository root, manifest, home directory), and the `FOR_*` constants naming what each subcommand wanted a root for.

**`error.rs`** — `Error`, the binary's single error enum, every variant printing as one line (`one_line`), covering every failure any subcommand or the interactive loop can produce, plus `status_for`'s exit-code vocabulary is documented per-variant here.

**`descent.rs`** — `Descent`/`descend`, the one shared middle between the panel's `p`/`r` keys and the shell's `pact`/`refresh`/`unpact`: dispatches to the engine's `pact_subtree`/`refresh_subtree`/`unpact_subtree` and saves the manifest exactly once. Also `carry_on`, the shared cancellation-continuation check.

**`edits.rs`** — the three headless write subcommands: `warlock unpact`, `warlock scope add`, `warlock scope remove`. `Opened` is the gate type — it cannot be constructed without the boundary check (`boundary::verdict`) having passed.

**`running.rs`** — the headless `warlock pact`/`warlock refresh` subcommands: `Progress` (a `pact::Observer` printing plain stdout lines), `Report` (collecting per-directory failures), Ctrl-C handling (`listening`), and `descended`, the core composition tying it together.

**`query.rs`** — `warlock stale`/`warlock fresh`: `Listing`, `listed`/`listed_onto`, `state_word` (warlock's own three-word vocabulary, deliberately independent of the engine's serde spelling), and the shared JSON helpers `envelope`/`write_object` also used by `check.rs`.

**`check.rs`** — `warlock check <path>`: `checked`/`Checked`, answering which scope covers a path, what this machine holds (`sigils_under`, borrowed from `session.rs`), and whether the two meet — read-only, refuses nothing.

**`config.rs`** — `warlock config`: the one-line-in, one-file-out sigil-recording prompt. `sigils_in` (parses/validates/dedupes a typed line), `hold` (writes via the engine's `save_sigils`), `preamble` (everything printed above the cursor).

**`stubs.rs`** (test-only support, not `#[cfg(test)]` itself but only referenced from tests) — `Passing` and `Saying`, in-memory stand-ins for `Agent` and `Converses` respectively, used by tests elsewhere in the binary that are not about the model transport itself.