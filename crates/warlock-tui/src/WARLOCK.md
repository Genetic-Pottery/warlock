<!-- warlock -->
> Written by a model pass over this directory alone, to be read before its source and to say which source to read. A map, not a specification: check anything you are about to rely on against the files themselves, and where this document and the code disagree, the code is right.

# src

The warlock-tui library crate: the terminal front end's pure core, holding App, Panel, Account, Thread and the terminal binary's front-end modules for boundary checks, subcommands and the event loop's keystroke handling — everything but the terminal itself, which lives in main.rs and terminal.rs.

## Files

- `account.rs` (69.2 KB) — The Account and Section types: an ordered log of clocked Line rows per directory a pact ran over, plus Outcome, Voice, the clock rule Log::shown_at, and helpers size, money. · declares `THINKING`, `WAITING`, `WRITING`, `Log`, `opened_at`, `started`, `closed_at`, `is_closed`, `row_count`, `freeze`, `push`, `extend_or_open`, `rewrite_or_open`, `rows`, `Outcome`, `Section` (+40)
- `app.rs` (289.1 KB) — App and Row: the flattened tree state, walk_of/reflow deriving drawn rows from all_rows, Chrome, Run, RunHeader, PactIntent, PactToggle, Sigils, and reseat_on. · declares `Row`, `new`
- `boundary.rs` (10.3 KB) — Pure boundary judgement shared by the panel and shell doors: Reach, Verdict, verdict(), closed_scope_message, blocking_scopes_message. · declares `Reach`, `Verdict`, `message`, `verdict`, `closed_scope_message`, `blocking_scopes_message`
- `chatting.rs` (108.4 KB) — Chat and ChatAgent-driving logic: register (brief/chat/write) transitions, BRIEF_NOTE/CHAT_NOTE, and the worker that sends turns without blocking the loop. · declares `TURN_LOST`, `BRIEF_COMMAND`, `CHAT_COMMAND`, `WRITE_COMMAND`, `BRIEF_NOTE`, `CHAT_NOTE`, `ALREADY_CHATTING`, `NOT_BRIEFING`
- `check.rs` (23.0 KB) — `warlock check <path>`: prints scope coverage and sigil-opening verdict via checked()/checked_onto(), prose() and JSON object(). · declares `check`, `CHECK`, `PATH`, `SCOPE`, `SIGILS`, `OPENS`, `Checked`, `checked_onto`, `checked`, `prose`, `covering_line`, `holding_line`, `verdict_line`, `sigils_value`
- `claude.rs` (165.5 KB) — The Claude CLI child-process adapter: ClaudeAgent, Wired, Cancel, stream reading, ARGS, MODEL, CONTEXT_TOKENS, INVOCATION_TIMEOUT. · declares `INVOCATION_TIMEOUT`, `BRIEF_EFFORT`, `BRIEF_MODEL`, `PROGRAM`, `ARGS`, `MODEL`, `CONTEXT_TOKENS`, `EFFORT`
- `colour.rs` (6.7 KB) — Fixed indexed terminal colours: colour_for(NodeState), FOCUS_COLOUR, SYSTEM_COLOUR, GUIDE_COLOUR, CONVERSATION_COLOUR. · declares `colour_for`, `FOCUS_COLOUR`, `SYSTEM_COLOUR`, `GUIDE_COLOUR`, `CONVERSATION_COLOUR`
- `composer.rs` (83.0 KB) — Composer: the multi-line draft buffer, compose_for/paste_for pure keystroke functions, COMPOSER_MAX_ROWS, ComposerWindow. · declares `COMPOSER_MAX_ROWS`, `Composer`, `MARGIN`, `CHORD`
- `config.rs` (23.8 KB) — `warlock config`: the interactive sigil prompt, preamble(), sigils_in(), hold(), read_line(), RULES. · declares `configure`, `PROMPT`, `NOTHING`, `RULES`, `prompted`, `Held`, `held_for`, `preamble`, `fmt`, `holding`, `sigils_in`, `hold`, `read_line`
- `confirm.rs` (11.6 KB) — QuitConfirm dialog: Answer, Answered, answer_for() key handling for the quit confirmation. · declares `Answer`, `QuitConfirm`, `is_open`, `highlighted`, `Answered`, `answer_for`
- `descent.rs` (11.1 KB) — Descent enum (Pact/Refresh/Unpact) and descend(): the one place the manifest is saved after a walk; carry_on() for Ctrl-C. · declares `Descent`, `wanted`, `descend`, `carry_on`
- `editing.rs` (32.5 KB) — The `e` key: edit_press() spawns $EDITOR on the selected file, came_back() reloads the tree and refills the document card. · declares `edit_press`, `EDITOR_VAR`, `NO_EDITOR`, `Editor`, `came_back`, `edit_target`, `editor_command`, `run_editor`
- `edits.rs` (55.1 KB) — Headless writes: Opened (the boundary-gated manifest handle), unpacted()/scoped()/unscoped(), and unpact/scope_add/scope_remove subcommands. · declares `Opened`, `new`, `repo_root`, `manifest`, `target`, `opened`, `unpact`, `scope_add`, `scope_remove`, `unpacted`, `scoped`, `unscoped`, `scope_on`, `unpacted_line`, `scoped_line`, `unscoped_line`
- `error.rs` (24.3 KB) — The shared Error enum for panel and subcommands, its one-line Display, and one_line() flattening helper. · declares `Error`, `from_problems`, `one_line`, `fmt`, `source`, `from`
- `fixture.rs` (12.4 KB) — Test-only hand-built Tree fixtures (tree(), tree_after_a_run()) used by app.rs and ui.rs tests; no real directory is read. · declares `tree`, `tree_after_a_run`, `files`
- `input.rs` (146.8 KB) — Pure key/mouse-to-intent mapping: Action, action_for(), press_for(), mouse_action() consulted before App is touched. · declares `Action`, `action_for`
- `lib.rs` (2.6 KB) — The crate root: module declarations and the full re-export surface of warlock_tui's public API.
- `main.rs` (91.6 KB) — The terminal binary's entry point: Cli/Subcommand parsing, TerminalGuard lifecycle, panic hook install, and the event loop driving App.
- `pacting.rs` (250.7 KB) — The `p`/`r` keys as a worker thread: Pact<P>, PactEvent channel, Work tag distinguishing pact from refresh, Reloaded. · declares `Pact`, `Reloaded`, `PACT_LOST`, `PACT_CANCELLED`
- `panel.rs` (31.0 KB) — Panel: the three-card (Account/Thread/document) right-hand pane, Showing, Mode, Card<T>, Shown trait, panel_offset_for(). · declares `Panel`, `Showing`, `Mode`, `panel_offset_for`, `showing`, `window_of`, `document_lines`, `show`, `open_account`, `show_document`, `has_content`, `next_card`, `write_run`, `refill_document`, `start_turn`, `note` (+31)
- `prompt.rs` (19.5 KB) — ScopePrompt and ScopeField: the single-line scope-editing prompt, edit_for() keystroke handling, Edited. · declares `ScopeField`, `new`, `refused`, `directory`, `text`, `rule`, `ScopePrompt`, `is_open`, `field`, `Edited`, `edit_for`, `CHORD`
- `query.rs` (18.7 KB) — `warlock stale`/`warlock fresh`: Listing, listed(), state_word(), envelope()/write_object() shared JSON helpers. · declares `Listing`, `list`, `spelled`, `envelope`, `write_object`, `COMMAND`, `DIRECTORIES`, `wanted`, `word`, `Listed`, `state_word`, `listed_onto`, `listed`, `listing`
- `running.rs` (52.9 KB) — `warlock pact`/`warlock refresh` subcommands: Progress observer, Report, listening() Ctrl-C handler, descended() composition. · declares `pact`, `refresh`, `Progress`, `new`, `total`, `say`, `starting`, `repaired`, `documented`, `unchanged`, `skipped`, `named`, `Report`, `onto`, `status`, `report` (+6)
- `scoping.rs` (40.5 KB) — The `s` key: scope_press(), scope_edit(), scope_submit(), with_scope_on() manifest rebuild shared with headless scope writes. · declares `scope_press`, `scope_edit`, `scope_submit`, `with_scope_on`, `no_pact_message`
- `session.rs` (36.7 KB) — Scope, Watched, reload_tree(), load_app(), closed_scope(): where the tree on screen comes from and stays true to disk. · declares `NOT_REFRESHED`, `NOT_WATCHING`, `NOT_CLEANED`, `reload_tree`, `note`, `Scope`, `Watched`, `start`, `off_note`, `round`, `caught_up`, `start_watching`, `closed_scope`, `sigils_under`, `load_manifest`, `load_app` (+4)
- `standing.rs` (8.7 KB) — Standing: resolves working dir/repo root/home into parameters every subcommand takes; the FOR_* refusal-tail constants. · declares `FOR_CLAUDE_MD`, `FOR_SIGILS`, `FOR_CHECK`, `FOR_LISTING`, `FOR_UNPACT`, `FOR_SCOPE_ADD`, `FOR_SCOPE_REMOVE`, `FOR_PACT`, `FOR_REFRESH`, `Standing`, `here`, `at`, `repo_root`, `target`, `manifest`, `home` (+3)
- `stubs.rs` (1.4 KB) — Test-only stand-in Agent/Converses implementations (Passing, Saying) for tests not about child-process behaviour. · declares `Passing`, `filling`, `Saying`, `answering`, `run`, `wired`, `turn`, `raised`
- `submission.rs` (8.3 KB) — submitted_for(): parses a composer draft into Submitted (Brief/Write/Chat/Message/Refused), case-sensitive command matching. · declares `Submitted`, `refusal`, `submitted_for`, `REFUSAL`
- `template.rs` (19.0 KB) — brief_template()/DEFAULT_TEMPLATE/missing_sections(): the brief document shape, read fresh per call, no cache. · declares `DEFAULT_TEMPLATE`, `brief_template`, `Error`, `missing_sections`, `TEMPLATE_FILE`, `template_path`, `fmt`, `source`, `sections_of`, `carries`
- `terminal.rs` (7.5 KB) — TerminalGuard and the Screen trait: raw mode/alt screen/mouse/paste setup and restore, install_panic_hook(). · declares `Screen`, `TerminalGuard`, `enter`, `install_panic_hook`, `size`, `draw`, `suspended`, `report_mouse`, `take_terminal`, `drop`, `restore_terminal`
- `thread.rs` (44.2 KB) — Thread and Turn: the conversation card built on Log, Ending and ending_for() mapping agent::Error to panel wording. · declares `Ending`, `line`, `ending_for`, `Turn`, `message`, `answer`, `ending`, `started`, `is_closed`, `Thread`, `new`, `ask`, `note`, `record`, `end`, `turns` (+18)
- `ui.rs` (282.3 KB) — draw(): the one frame renderer and its measuring entry points (tree_height, panel_height, hit_test, etc.) all routed through areas(). · declares `INDENT`, `GUIDE`, `GUIDE_BRANCH`, `GUIDE_LAST`, `SELECTION_MARKER`, `COLLAPSED_MARKER`, `EXPANDED_MARKER`, `NO_MARKER`, `MARK`
- `viewing.rs` (18.1 KB) — The `v` key: view_press() reads the selected file via view_file and calls App::show_document. · declares `view_press`
- `watch.rs` (30.5 KB) — Filesystem watching: NodeSet (the walk-derived filter), WatchPolicy (debounce/ceiling timing), Watch/Watching (the impure notify wrapper). · declares `QUIET_PERIOD`, `RELOAD_CEILING`, `COALESCED_RELOADS`, `NodeSet`, `from_tree`, `accepts`, `len`, `is_empty`, `WatchPolicy`, `new`, `follow`, `watched`, `saw`, `accepted`, `due`, `reload_started` (+9)
- `wrap.rs` (24.3 KB) — Line wrapping: Shape/shape(), rows()/wrapped() (word-break, for the panel), folded() (byte-preserving, for the composer field). · declares `Shape`, `shape`, `rows`, `wrapped`, `folded`, `continued`, `filled`, `break_at`, `first_character`
- `writing.rs` (57.9 KB) — `/write`'s path proposal and `write_submit`: `unfenced` strips a wrapping fence, `slug_of` and `next_number` build `warlock-brief-NN-slug.md`, and the shape check via `missing_sections` refuses before any byte is written. · declares `WRITE_HEADING`, `proposed_path`, `write_opened`, `write_edit`, `write_submit`, `unfenced`, `slug_of`, `BRIEF_PREFIX`, `UNTITLED`, `SLUG_MAX`, `NO_PATH`, `NOTHING_TO_WRITE`, `refused`, `document`, `document_on`, `put` (+13)

## Structure

- main.rs owns the terminal lifecycle and event loop, dispatching subcommands before entering raw mode via TerminalGuard from terminal.rs.
- input.rs turns crossterm events into Action/MouseAction values that main.rs's loop applies to App.
- panel.rs, account.rs and thread.rs are all built on Log and Line from account.rs, which is what keeps their clock rule identical.
- app.rs derives its drawn rows (reflow) from walk_of over a Tree the engine hands over, never mutating all_rows in place.
- ui.rs draws Panel, App and Composer state via wrap.rs's rows()/shape(), and all its measuring functions route through the same areas() cut as draw().
- boundary.rs's verdict() is the single boundary judgement called by edits.rs's Opened::new, session.rs's closed_scope, and scoping.rs's scope_press.
- descent.rs's descend() is called by both pacting.rs (the panel's p/r keys) and running.rs (the headless pact/refresh subcommands) so both doors save the manifest identically.
- edits.rs's Opened gates running.rs's ran() and its own unpact/scope_add/scope_remove, so no headless write can skip the boundary check.
- chatting.rs's Chat drives claude.rs's ChatAgent/Converses and writes turns onto panel.rs's Panel through thread.rs's Thread.
- writing.rs's write_submit reads template.rs's brief_template/missing_sections before putting a document on disk, and shares ScopePrompt/ScopeField from prompt.rs with scoping.rs.
- session.rs's Watched wraps watch.rs's Watch/WatchPolicy and calls reload_tree on its own thread, never from a worker thread.
- error.rs's Error is the vocabulary returned by check.rs, config.rs, query.rs, edits.rs, running.rs and session.rs alike, printed as one line by main.rs.

## Rules

- Nothing in this crate but claude.rs and watch.rs opens a terminal, reads a key, spawns a child process or watches the filesystem; everything else is plain values.
- account.rs never calls Instant::now itself; every entry point and read takes the instant as a parameter.
- reflow is meant to be the only function turning all_rows into rows; App::insert_file_row is the sole exception and must apply the same three filters in the same order.
- Selection and collapsed state are remembered as paths, never row indices, because reflow and reseat_on rebuild the row list on every change.
- colour_for has no catch-all match arm, so a new NodeState breaks compilation rather than drawing under a fallback.
- Submitted command matching in submission.rs is case-sensitive by design: /BRIEF is refused rather than folded, because a refusal costs one line and a send costs a turn.
- $EDITOR is split on whitespace and never passed to sh -c, and there is no fallback editor if it is unset.
- boundary.rs functions read no disk and set no message; they are pure judgements over values so the panel and shell refuse identically.
- wrap.rs's rows() and folded() disagree on purpose: rows() breaks at words for reading, folded() fills every column and preserves every byte for the caret.
- INVOCATION_TIMEOUT in claude.rs is fixed at 5 minutes, and ARGS always includes --include-partial-messages so stream_event lines can be read.
- descent.rs saves the manifest exactly once, after the whole descent completes, never during it.

## Where to look

- how a directory's pact or refresh ending is worded on the panel → `account.rs` `Outcome`
- why a keystroke over a scoped directory is refused → `boundary.rs` `verdict`
- how the claude CLI child process is spawned and its output streamed → `claude.rs` `ClaudeAgent`
- how the account/thread/document cards share one scroll window → `panel.rs` `Card`
- how warlock pact/refresh report progress and failures on stdout/stderr → `running.rs` `Progress`
- how a directory's pass ended and what warlock check prints as JSON vs prose → `check.rs` `checked`
- which Action a keypress maps to → `input.rs` `action_for`
- how the two long keystrokes p and r run as a worker thread reporting over a channel → `pacting.rs` `Pact`
- how one frame is measured and drawn from App and Panel state → `ui.rs` `draw`
- how filesystem events are filtered and debounced into a reload → `watch.rs` `WatchPolicy`
- how /write proposes a path and validates a brief before saving it → `writing.rs` `write_submit`
- the crate's full public export list → `lib.rs` `pub use`
