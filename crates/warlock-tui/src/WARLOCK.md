<!-- warlock -->
> Written by a model pass over this directory alone, to be read before its source and to say which source to read. A map, not a specification: check anything you are about to rely on against the files themselves, and where this document and the code disagree, the code is right.

# src

The warlock-tui library crate's source: the terminal front end's pure parts — account and thread ledgers, the app's tree/row state, panel cards, composer, wrapping, drawing, and the claude/watch impure seams — all assembled and re-exported through lib.rs for the warlock binary to drive.

## Files

- `account.rs` (64.2 KB) — Account, Section, Log, Line, Outcome — the clocked record of one pact's run, one Section per directory, worded lines and the size/money helpers. · declares `THINKING`, `WAITING`, `WRITING`, `Log`, `opened_at`, `started`, `closed_at`, `is_closed` (+44)
- `app.rs` (287.8 KB) — App and Row — the flattened tree, selection, collapse and filter state, and reflow/walk_of that derive drawn rows from a Tree. · declares `Row`, `new`
- `boundary.rs` (10.3 KB) — Reach, Verdict, verdict() — the shared scope/sigil boundary check used by both the panel keys and headless subcommands. · declares `Reach`, `Verdict`, `message`, `verdict`, `closed_scope_message`, `blocking_scopes_message`
- `chatting.rs` (108.5 KB) — Chat, the conversation state machine — agent, in-flight turn, composer draft and brief window, plus /brief, /chat, /write command handling. · declares `TURN_LOST`, `BRIEF_COMMAND`, `CHAT_COMMAND`, `WRITE_COMMAND`, `BRIEF_NOTE`, `CHAT_NOTE`, `ALREADY_CHATTING`, `NOT_BRIEFING`
- `check.rs` (23.0 KB) — warlock check subcommand — Checked, prose/JSON rendering of which scope covers a path and whether held sigils open it. · declares `check`, `CHECK`, `PATH`, `SCOPE`, `SIGILS`, `OPENS`, `Checked`, `checked_onto` (+6)
- `claude.rs` (165.5 KB) — Runs `claude` as a child process — ClaudeAgent, Wired, stream reading, cancellation via Cancel and shared Mutex<Child>. · declares `INVOCATION_TIMEOUT`, `BRIEF_EFFORT`, `BRIEF_MODEL`, `PROGRAM`, `ARGS`, `MODEL`, `CONTEXT_TOKENS`, `EFFORT`
- `colour.rs` (3.7 KB) — colour_for(NodeState) and the FOCUS_COLOUR/GUIDE_COLOUR constants — the only place node states become terminal colours. · declares `colour_for`, `FOCUS_COLOUR`, `GUIDE_COLOUR`
- `composer.rs` (83.0 KB) — Composer, ComposerWindow, Composed, Pasted, compose_for/paste_for — the multi-line draft field and its keystroke/paste rules. · declares `COMPOSER_MAX_ROWS`, `Composer`, `new`, `at`, `cursor`, `set_width`, `width`, `set_muted` (+16)
- `config.rs` (23.8 KB) — warlock config subcommand — reads and rewrites sigils for this machine/repository via one prompt line, Held state, sigils_in/hold. · declares `configure`, `PROMPT`, `NOTHING`, `RULES`, `prompted`, `Held`, `held_for`, `preamble` (+5)
- `confirm.rs` (11.6 KB) — QuitConfirm, Answer, Answered, answer_for — the Esc/quit confirmation dialog's key handling. · declares `Answer`, `QuitConfirm`, `is_open`, `highlighted`, `Answered`, `answer_for`
- `descent.rs` (11.1 KB) — Descent enum (Pact/Refresh/Unpact) and descend() — the one shared engine-call-plus-save path used by both the panel and shell doors. · declares `Descent`, `wanted`, `descend`, `carry_on`
- `editing.rs` (32.5 KB) — edit_press/came_back — the `e` key, spawning $EDITOR as a foreground child and reloading the tree/document afterward. · declares `edit_press`, `EDITOR_VAR`, `NO_EDITOR`, `Editor`, `came_back`, `edit_target`, `editor_command`, `run_editor`
- `edits.rs` (55.1 KB) — Opened gate and the headless writes — warlock unpact / scope add / scope remove — boundary-checked manifest edits. · declares `Opened`, `new`, `repo_root`, `manifest`, `target`, `opened`, `unpact`, `scope_add` (+8)
- `error.rs` (24.3 KB) — Error enum and Display — every warlock failure flattened to one line via one_line(), shared by panel footer and shell exit. · declares `Error`, `from_problems`, `one_line`, `fmt`, `source`, `from`
- `fixture.rs` (12.4 KB) — tree()/tree_after_a_run() — hand-built Tree fixtures used by app and draw tests instead of a real repository walk. · declares `tree`, `tree_after_a_run`, `files`
- `input.rs` (146.8 KB) — Action, action_for/press_for/mouse_action — keys and mouse events turned into intentions, consulting confirm/prompt windows in order. · declares `Action`, `action_for`
- `lib.rs` (2.6 KB) — The crate root: module declarations and the full list of re-exports (Account, App, Panel, Thread, Watch, etc.) forming the public API.
- `main.rs` (91.6 KB) — warlock binary's entry point — clap Cli/Subcommand definitions, TerminalGuard lifecycle, panic hook, and the event loop driving everything.
- `pacting.rs` (229.3 KB) — Pact<P> worker and PactEvent channel — the `p`/`r` keystrokes as a background thread reporting progress, with Reloaded and CancelGuard. · declares `Pact`, `Reloaded`, `PACT_LOST`, `PACT_CANCELLED`
- `panel.rs` (31.0 KB) — Panel, Card<T>, Showing, Mode — the right-hand pane holding Account/Thread/document cards and the card-swap and scroll-window logic. · declares `Panel`, `Showing`, `Mode`, `panel_offset_for`, `showing`, `window_of`, `document_lines`, `show` (+39)
- `prompt.rs` (19.5 KB) — ScopeField, ScopePrompt, Edited, edit_for — the single-line scope/write prompt's text-field key handling. · declares `ScopeField`, `new`, `refused`, `directory`, `text`, `rule`, `ScopePrompt`, `is_open` (+4)
- `query.rs` (18.7 KB) — warlock stale/fresh subcommands — Listing, Listed, envelope()/write_object() JSON helpers shared with check.rs. · declares `Listing`, `list`, `spelled`, `envelope`, `write_object`, `COMMAND`, `DIRECTORIES`, `wanted` (+6)
- `running.rs` (44.7 KB) — warlock pact/refresh subcommands — Progress observer, Report of failures, Ctrl-C handling via listening(), and the descended() composition. · declares `pact`, `refresh`, `Progress`, `new`, `total`, `say`, `starting`, `documented` (+13)
- `scoping.rs` (40.5 KB) — scope_press/scope_edit/scope_submit — the `s` key's path from press to a saved scope in .warlock/pacts.toml. · declares `scope_press`, `scope_edit`, `scope_submit`, `with_scope_on`, `no_pact_message`
- `session.rs` (23.4 KB) — Scope, Watched, load_app/reload_tree/closed_scope — where the on-screen tree comes from and how it's kept true to disk and to sigils. · declares `NOT_REFRESHED`, `NOT_WATCHING`, `reload_tree`, `note`, `Scope`, `Watched`, `start`, `off_note` (+8)
- `standing.rs` (8.7 KB) — Standing — resolves repository root, working directory and home, and the FOR_* sentence tails for NoRepository errors. · declares `FOR_CLAUDE_MD`, `FOR_SIGILS`, `FOR_CHECK`, `FOR_LISTING`, `FOR_UNPACT`, `FOR_SCOPE_ADD`, `FOR_SCOPE_REMOVE`, `FOR_PACT` (+11)
- `stubs.rs` (1.4 KB) — Passing and Saying — test-only stand-in Agent/Converses implementations with no cancellation or activity reporting. · declares `Passing`, `filling`, `Saying`, `answering`, `run`, `wired`, `turn`, `raised`
- `submission.rs` (8.3 KB) — Submitted enum and submitted_for() — parses composer drafts into /brief, /write, /chat, Message or Refused. · declares `Submitted`, `refusal`, `submitted_for`, `REFUSAL`
- `template.rs` (19.0 KB) — DEFAULT_TEMPLATE, brief_template(), missing_sections() — the brief document shape and the check that a written brief carries every section. · declares `DEFAULT_TEMPLATE`, `brief_template`, `Error`, `missing_sections`, `TEMPLATE_FILE`, `template_path`, `fmt`, `source` (+2)
- `terminal.rs` (7.5 KB) — Screen trait and TerminalGuard — raw mode, alternate screen, mouse/paste setup and teardown, and the panic hook installer. · declares `Screen`, `TerminalGuard`, `enter`, `install_panic_hook`, `size`, `draw`, `suspended`, `report_mouse` (+3)
- `thread.rs` (44.2 KB) — Thread, Turn, Ending, ending_for — the conversation card's ordered turns and notes, sharing the Log clock rule with account.rs. · declares `Ending`, `line`, `ending_for`, `Turn`, `message`, `answer`, `ending`, `started` (+26)
- `ui.rs` (273.1 KB) — draw() and the measuring entry points (tree_height, panel_height, hit_test, etc.) — the one frame-rendering module, no clock or terminal reads. · declares `INDENT`, `GUIDE`, `GUIDE_BRANCH`, `GUIDE_LAST`, `SELECTION_MARKER`, `COLLAPSED_MARKER`, `EXPANDED_MARKER`, `NO_MARKER` (+1)
- `viewing.rs` (18.1 KB) — view_press() — the `v` key, reading a file into the document card via App::view_target and view_file. · declares `view_press`
- `watch.rs` (30.5 KB) — NodeSet, WatchPolicy, Watch, Watching — filesystem watching filtered to tree nodes, debounced by QUIET_PERIOD/RELOAD_CEILING. · declares `QUIET_PERIOD`, `RELOAD_CEILING`, `COALESCED_RELOADS`, `NodeSet`, `from_tree`, `accepts`, `len`, `is_empty` (+17)
- `wrap.rs` (23.9 KB) — Shape, shape(), rows(), wrapped()/folded() — word-wrapping for display versus byte-preserving folding for the composer field. · declares `Shape`, `shape`, `rows`, `wrapped`, `folded`, `continued`, `filled`, `break_at` (+1)
- `writing.rs` (64.4 KB) — The `/write` command's landing path: `write_submit` writes the shown path, `unfenced` strips a code fence, `slug_of`/`next_number` build the filename, `missing_sections` refuses an incomplete brief, `pacted_above` names the ancestor pact staled. · declares `WRITE_HEADING`, `proposed_path`, `write_opened`, `write_edit`, `write_submit`, `unfenced`, `slug_of`, `BRIEF_PREFIX` (+26)

## Structure

- main.rs dispatches subcommands (check, config, edits, running, query) before touching the terminal, then drives the event loop via input.rs, pacting.rs, editing.rs, scoping.rs, viewing.rs, chatting.rs, session.rs
- descent.rs's descend() is called by both pacting.rs (panel) and running.rs (shell) so the manifest is saved exactly once per descent
- edits.rs's Opened gate wraps boundary.rs's verdict() and is required before running.rs or edits.rs writes anything
- session.rs's closed_scope() and boundary.rs's verdict() are the two callers scoping.rs and edits.rs both consult for the same refusal wording
- panel.rs holds account.rs's Account and thread.rs's Thread as Cards, and calls wrap.rs's rows() to count and window lines
- ui.rs calls wrap.rs's shape()/rows() to draw exactly what panel.rs counted, and reads app.rs's Row/App for the tree
- app.rs's reflow derives rows from a warlock_engine::Tree; fixture.rs supplies test trees for app.rs and ui.rs tests
- writing.rs and chatting.rs call template.rs's brief_template/missing_sections before writing a brief to disk
- error.rs's one_line() and Error::Display are called from nearly every subcommand module (check, config, edits, running, query, session) to flatten engine errors
- claude.rs is the only module spawning a child process; chatting.rs and pacting.rs depend on it (or stubs.rs in tests) via the Agent/Converses/Wired traits
- terminal.rs's Screen trait is implemented by TerminalGuard and used by editing.rs's edit_press to suspend/resume the terminal for $EDITOR

## Rules

- account.rs and thread.rs never call Instant::now(); every entry point takes the instant it happened at, and reads take the `now` explicitly
- app.rs: reflow is meant to be the only thing turning all_rows into rows, except App::insert_file_row, which must apply the same three filters in the same order
- descent.rs: the manifest is saved exactly once, after the descent, never during it
- edits.rs: the boundary is asked before the path is spelled, before the existence check, before anything reads the manifest's contents
- error.rs: every Error variant's Display prints as a single line, since main prints exactly one line after the terminal is restored
- terminal.rs: raw mode, alternate screen, bracketed paste and mouse reporting are restored on every way out, including a panic on any thread
- composer.rs and prompt.rs: only key presses are handled, never releases or auto-repeats, since crossterm reports them inconsistently across platforms
- submission.rs: command matching is case-sensitive, so `/BRIEF` is refused rather than folded
- writing.rs: write_submit judges every rule before writing a single byte; the target existing or a missing section both refuse with nothing written
- watch.rs: NodeSet's filter is the last walk itself, with no re-implementation of gitignore or skip-list rules

## Where to look

- how a run's clock keeps ticking on the newest line → `account.rs` `shown_at`
- why a directory row is refused mid-pact → `pacting.rs` `Pact`
- which key does what in the panel → `input.rs` `action_for`
- how scope and sigil boundaries decide who may act where → `boundary.rs` `verdict`
- what happens when Enter is pressed in the write prompt → `writing.rs` `write_submit`
- how a stale directory is detected and coloured → `colour.rs` `colour_for`
- how the panel decides which of three cards is drawn → `panel.rs` `Showing`
- how filesystem events become a reload → `watch.rs` `WatchPolicy`
- how a keystroke wraps or breaks a long line → `wrap.rs` `wrapped`
- what `claude` is actually invoked with → `claude.rs` `ARGS`
- how errors are flattened to one shell/footer line → `error.rs` `one_line`
- how the repository root and home directory are resolved → `standing.rs` `Standing`
