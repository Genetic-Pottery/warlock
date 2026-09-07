<!-- warlock -->
> Written by a model pass over this directory alone, to be read before its source and to say which source to read. A map, not a specification: check anything you are about to rely on against the files themselves, and where this document and the code disagree, the code is right.

# src

The front end's own crate root plus binary: warlock_tui is data and pure functions over a flattened tree, panel, account and thread, with no terminal of its own; the warlock binary is the thin impure shell that owns the terminal's lifecycle, the event loop, headless subcommands and the boundary rules that gate writes.

## Files

- `account.rs` (86.6 KB) — Account and Section: the ordered per-directory record of what a pact run did, one clocked Line per activity, with the freezing-clock rule and no clock of its own. · declares `THINKING`, `WAITING`, `WRITING`, `Log`, `opened_at`, `started`, `closed_at`, `is_closed` (+43)
- `app.rs` (434.2 KB) — App, Row, Focus, Chrome, RunHeader, PactToggle: the flattened tree state, selection, scrolling, pact toggling and reseat_on for carrying a view across a reload. · declares `Row`, `new`, `file`, `with_child_count`, `with_ignored`, `with_document_row`, `with_scope`, `has_children` (+56)
- `boundary.rs` (14.4 KB) — verdict, Verdict, Reach: the one function deciding whether an operator may act on a directory, shared by the TUI's keys and the shell's headless writes. · declares `Reach`, `Verdict`, `message`, `verdict`, `closed_scope_message`, `blocking_scopes_message`
- `chatting.rs` (163.2 KB) — Chat: the conversation register, composer field and turn machinery (spawn_turn/start_turn/run_turn/apply_turn) driving /brief, /write and ordinary messages. · declares `Chat`, `new`, `with_agent`, `composer`, `set_composer_width`, `write_prompt`, `directory`, `answering` (+31)
- `check.rs` (30.1 KB) — warlock check <path> subcommand: prints which scope covers a path, what this machine holds, and whether they meet, in prose or --json. · declares `check`, `CHECK`, `PATH`, `SCOPE`, `SIGILS`, `OPENS`, `Checked`, `checked_onto` (+6)
- `claude.rs` (230.4 KB) — ClaudeAgent and ChatAgent: the transport running the claude CLI as a child process for both a pact pass (Agent) and a read-only chat turn (Converses). · declares `INVOCATION_TIMEOUT`, `BRIEF_EFFORT`, `BRIEF_MODEL`, `brief_instruction`, `CHAT_INSTRUCTION`, `WRITE_INSTRUCTION`, `Cancel`, `cancel` (+56)
- `colour.rs` (6.2 KB) — colour_for and the pinned indexed colours (unpacted/stale/fresh, focus, guide) as one total match. · declares `colour_for`, `FOCUS_COLOUR`, `GUIDE_COLOUR`
- `composer.rs` (118.0 KB) — Composer and compose_for/paste_for: the multi-line draft field at the foot of the panel, its cursor, wrapping and paste handling. · declares `COMPOSER_MAX_ROWS`, `Composer`, `new`, `at`, `cursor`, `set_width`, `width`, `set_muted` (+16)
- `config.rs` (31.3 KB) — warlock config subcommand: the interactive prompt that reads and writes this machine's sigils to <home>/.warlock/<project>/config.toml. · declares `configure`, `PROMPT`, `NOTHING`, `RULES`, `prompted`, `Held`, `held_for`, `preamble` (+5)
- `confirm.rs` (16.2 KB) — QuitConfirm, Answer, answer_for: the two-answer quit confirmation gate and what a keystroke does to it. · declares `Answer`, `QuitConfirm`, `is_open`, `highlighted`, `Answered`, `answer_for`
- `descent.rs` (14.4 KB) — Descent enum and descend/carry_on: the one shared entry point both doors (panel keys, shell subcommands) use to run pact/refresh/unpact and save the manifest exactly once. · declares `Descent`, `wanted`, `descend`, `carry_on`
- `editing.rs` (44.9 KB) — The edit key (e): opening a file in $EDITOR, suspending and restoring the terminal, and re-reading the file afterwards. · declares `edit_press`, `EDITOR_VAR`, `NO_EDITOR`, `Editor`, `came_back`, `edit_target`, `editor_command`, `run_editor`
- `edits.rs` (74.9 KB) — Headless writes: warlock unpact/scope add/scope remove, gated through Opened which cannot be built without the boundary already being asked. · declares `Opened`, `new`, `repo_root`, `manifest`, `target`, `opened`, `unpact`, `scope_add` (+8)
- `error.rs` (41.0 KB) — Error: the binary's whole one-line error vocabulary printed by main, covering every subcommand's failure modes and their exit-status mapping. · declares `Error`, `from_problems`, `one_line`, `fmt`, `source`, `from`
- `fixture.rs` (14.7 KB) — Test-only hand-written Tree fixtures (tree, tree_after_a_run) exercising every NodeState and nesting shape for the crate's own tests. · declares `tree`, `tree_after_a_run`, `files`
- `input.rs` (194.1 KB) — action_for, mouse_action, press_for: pure translations of key and mouse events into Action/MouseAction intents, with no terminal touched. · declares `Action`, `action_for`, `Pressed`, `press_for`, `MouseAction`, `mouse_action`, `is_ctrl_c`, `is_tab` (+3)
- `lib.rs` (24.3 KB) — The crate root: module declarations and the full pub use re-export surface for warlock_tui's data types and pure functions.
- `main.rs` (135.7 KB) — The warlock binary's event loop: terminal setup, dispatch to subcommands, and the frame-per-tick loop driving pacts, chats, watching and rendering. · declares `POLL_INTERVAL`, `CREATED`, `UPDATED`, `Cli`, `Command`, `ScopeCommand`, `main`, `status_for` (+11)
- `pacting.rs` (296.1 KB) — Pact: running a pact/refresh on a worker thread from a keystroke, Pact::press/keep_up/stop, reporting into the account and saving the manifest once. · declares `Pact`, `Reloaded`, `new`, `with_run`, `with_agent`, `running`, `stop`, `press` (+42)
- `panel.rs` (55.0 KB) — Panel, Card<T>, Showing: the three-card (account/thread/document) panel state, which one is on screen, and its scrolling window. · declares `Panel`, `Showing`, `Mode`, `panel_offset_for`, `showing`, `window_of`, `document_lines`, `show` (+39)
- `prompt.rs` (26.0 KB) — ScopePrompt, ScopeField, edit_for: the single-line scope-entry field opened by the s key, judging nothing itself. · declares `ScopeField`, `new`, `refused`, `directory`, `text`, `rule`, `ScopePrompt`, `is_open` (+4)
- `query.rs` (26.0 KB) — warlock stale/fresh subcommands: listing pacted directories in a given NodeState, spelled against the repository root, prose or --json. · declares `Listing`, `list`, `spelled`, `envelope`, `write_object`, `COMMAND`, `DIRECTORIES`, `wanted` (+6)
- `running.rs` (69.6 KB) — The headless warlock pact/refresh subcommands: Progress reporting to stdout, cancellation via Ctrl-C, and the Report of failures/status. · declares `pact`, `refresh`, `Progress`, `new`, `total`, `say`, `starting`, `documented` (+12)
- `scoping.rs` (47.8 KB) — The scope key (s), from keystroke to saved manifest: scope_press/scope_edit/scope_submit, folding and validating via the engine's validate_scope. · declares `scope_press`, `scope_edit`, `scope_submit`, `with_scope_on`, `no_pact_message`
- `session.rs` (34.2 KB) — Scope, Watched, reload_tree, closed_scope, sigils_held/sigils_under, load_app: where the tree came from, how it's kept true to disk, and the boundary check for p/r/s. · declares `NOT_REFRESHED`, `NOT_WATCHING`, `reload_tree`, `note`, `Scope`, `Watched`, `start`, `off_note` (+8)
- `standing.rs` (13.5 KB) — Standing: the shared prologue for every headless subcommand — working dir, repository root, manifest, home directory — and the FOR_* sentence tails. · declares `FOR_CLAUDE_MD`, `FOR_SIGILS`, `FOR_CHECK`, `FOR_LISTING`, `FOR_UNPACT`, `FOR_SCOPE_ADD`, `FOR_SCOPE_REMOVE`, `FOR_PACT` (+11)
- `stubs.rs` (4.3 KB) — Passing and Saying: in-memory stand-ins for the Agent and Converses model seams, for tests that are not about the model transport itself. · declares `Passing`, `filling`, `Saying`, `answering`, `run`, `wired`, `turn`, `raised`
- `submission.rs` (12.8 KB) — Submitted and submitted_for: classifying a composer draft as /brief, /write, /chat, a message, or a refusal, per the module's stated matching rule. · declares `Submitted`, `refusal`, `submitted_for`, `REFUSAL`
- `template.rs` (24.3 KB) — brief_template, DEFAULT_TEMPLATE, missing_sections: the shape a brief must take, read fresh from .warlock/brief-template.md or the built-in skeleton. · declares `DEFAULT_TEMPLATE`, `brief_template`, `Error`, `missing_sections`, `TEMPLATE_FILE`, `template_path`, `fmt`, `source` (+2)
- `terminal.rs` (14.3 KB) — TerminalGuard, Screen trait, install_panic_hook: the terminal's take/restore lifecycle including Screen::suspended for running $EDITOR. · declares `Screen`, `TerminalGuard`, `enter`, `install_panic_hook`, `size`, `draw`, `suspended`, `report_mouse` (+3)
- `thread.rs` (59.9 KB) — Thread, Turn, Ending: the conversation card's data — every typed message, the model's work and answer, and warlock's own unclocked notes. · declares `Ending`, `line`, `ending_for`, `Turn`, `message`, `answer`, `ending`, `started` (+26)
- `ui.rs` (405.9 KB) — draw and all layout/rendering: the frame, tree pane, panel, footer, header, composer, confirm and scope windows, plus hit_test. · declares `PANEL_INDENT`, `SAID_MARKER`, `NOTE_MARKER`, `draw`, `composer_on_screen`, `tree_height`, `panel_height`, `run_header_height` (+56)
- `viewing.rs` (24.6 KB) — The view key (v): view_press reads a file into the panel via the engine's view_file, with no run, no write, no reload. · declares `view_press`
- `watch.rs` (41.4 KB) — Watch, WatchPolicy, NodeSet, Watching: the filesystem watcher, its debounce/ceiling timing rules, and the filter over the last successful walk. · declares `QUIET_PERIOD`, `RELOAD_CEILING`, `COALESCED_RELOADS`, `NodeSet`, `from_tree`, `accepts`, `len`, `is_empty` (+17)
- `wrap.rs` (33.7 KB) — rows, wrapped, folded, Shape: breaking one card line into panel-width rows (word-wrapped) or field-width rows (character-filled). · declares `Shape`, `shape`, `rows`, `wrapped`, `folded`, `continued`, `filled`, `break_at` (+1)
- `writing.rs` (90.6 KB) — Where a brief goes and the writing of it: proposed_path guesses the doc's path from the reply (unfenced, title slugged, next number in the directory) and write_submit writes whatever is in the editable field, never the proposal itself, on Enter. · declares `WRITE_HEADING`, `proposed_path`, `write_opened`, `write_edit`, `write_submit`, `unfenced`, `slug_of`, `BRIEF_PREFIX` (+26)

## Structure

- main.rs drives the event loop, calling into input.rs for key/mouse translation, pacting.rs/chatting.rs for worker threads, session.rs for tree state, and terminal.rs for setup/teardown.
- pacting.rs and chatting.rs both call descent.rs's descend/carry_on and boundary.rs's verdict to gate and run work, then save via warlock_engine.
- edits.rs, check.rs, query.rs, config.rs, running.rs all build on standing.rs's Standing prologue and error.rs's Error vocabulary.
- session.rs's closed_scope and edits.rs's Opened both call boundary.rs's verdict, keeping panel and shell refusals worded identically.
- ui.rs draws from app.rs (App/Row), panel.rs (Panel/Card), thread.rs (Thread) and wrap.rs (rows) but writes nothing back to them.
- app.rs re-exports panel behavior by forwarding to panel.rs methods rather than owning card state directly.
- claude.rs and stubs.rs both implement the engine's Agent port and the crate's own Converses port, used interchangeably by pacting.rs/chatting.rs.

## Rules

- The three NodeState colours (245/214/71) and the focus/guide colours (45/240) are pinned 256-indexed values, never named ANSI colours.
- A chat turn's tool vector grants only Read, Grep and Glob — nothing that writes, edits, shells out or reaches the network.
- The manifest is saved exactly once per descent, after the walk completes and never during it (descent.rs).
- Boundary verdicts are asked in one fixed order: the scope covering the directory first, then (for un-pact only) the scopes below it.
- Every Error variant renders as exactly one line, since main prints one line after the terminal is restored.
- A quit confirmation and a scope prompt are values outside App, never fields on it, so answering No/Esc leaves App untouched by construction.
- Only key-press events act in confirm.rs/prompt.rs/composer.rs; releases and repeats are treated as no-ops.
- A run's account and a chat turn never populate each other's card; a pact fills its own account card even while a conversation is on screen.

## Where to look

- why is a pact run on a worker thread with a channel → `pacting.rs` `Pact`
- what happens when Ctrl-C is pressed during a headless pact → `running.rs` `Progress`
- how are scope and sigil boundary refusals worded identically in TUI and shell → `boundary.rs` `closed_scope_message`
- how does /write decide the output file path → `writing.rs` `proposed_path`
- why doesn't warlock scroll long lines instead of wrapping them → `wrap.rs` `wrapped`
- what does warlock check --json print → `check.rs` `Checked`
- how is the terminal restored after $EDITOR runs → `terminal.rs` `suspended`
- why is a chat turn shaped like a run but not an Agent → `claude.rs` `ChatAgent`
- what decides when the tree is reloaded after a file changes on disk → `watch.rs` `WatchPolicy`
- how are /brief, /write and /chat distinguished from an ordinary message → `submission.rs` `submitted_for`
- what does un-pacting refuse that pacting does not → `edits.rs` `Opened`
- in-memory test doubles for the model instead of a real claude process → `stubs.rs` `Passing`
