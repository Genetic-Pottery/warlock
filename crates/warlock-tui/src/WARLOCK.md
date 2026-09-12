<!-- warlock -->
> Written by a model pass over this directory alone, to be read before its source and to say which source to read. A map, not a specification: check anything you are about to rely on against the files themselves, and where this document and the code disagree, the code is right.

# src

The front end of warlock minus the terminal: pure values for the tree row list, the account and thread cards, the composer, the scope prompt, colours, wrapping and layout, plus the impure claude.rs child-process adapter and watch.rs filesystem watcher, all assembled by src/main.rs one directory up.

## Files

- `account.rs` (67.5 KB) — Account, Section, Log, Line, Outcome — what one pact did as clocked, ordered per-directory logs; size(), money() and clock() formatting. · declares `THINKING`, `WAITING`, `WRITING`, `Log`, `opened_at`, `started`, `closed_at`, `is_closed` (+46)
- `app.rs` (287.8 KB) — App, Row, Chrome, Focus, PactIntent, PactToggle, Run, RunHeader, Sigils — the flattened tree, row selection, toggles and reseat_on. · declares `Row`, `new`
- `boundary.rs` (10.3 KB) — Reach, Verdict, verdict() — the one boundary judgement shared by the panel's keys and the headless subcommands, plus closed_scope_message and blocking_scopes_message. · declares `Reach`, `Verdict`, `message`, `verdict`, `closed_scope_message`, `blocking_scopes_message`
- `chatting.rs` (108.5 KB) — not shown to this directory's reader; the conversation the binary keeps: Chat, ChatAgent wiring, /brief and /chat register handling. · declares `TURN_LOST`, `BRIEF_COMMAND`, `CHAT_COMMAND`, `WRITE_COMMAND`, `BRIEF_NOTE`, `CHAT_NOTE`, `ALREADY_CHATTING`, `NOT_BRIEFING`
- `check.rs` (23.0 KB) — not part of this crate — belongs to the binary crate; described only if present here. · declares `check`, `CHECK`, `PATH`, `SCOPE`, `SIGILS`, `OPENS`, `Checked`, `checked_onto` (+6)
- `claude.rs` (165.5 KB) — Running `claude` as a child process — INVOCATION_TIMEOUT, ARGS, MODEL, ClaudeAgent, Activity/Activities streaming, Cancel. · declares `INVOCATION_TIMEOUT`, `BRIEF_EFFORT`, `BRIEF_MODEL`, `PROGRAM`, `ARGS`, `MODEL`, `CONTEXT_TOKENS`, `EFFORT`
- `colour.rs` (3.7 KB) — colour_for(NodeState), FOCUS_COLOUR, GUIDE_COLOUR — the four pinned indexed terminal colours. · declares `colour_for`, `FOCUS_COLOUR`, `GUIDE_COLOUR`
- `composer.rs` (83.0 KB) — Composer, Composed, Pasted, compose_for, paste_for, COMPOSER_MAX_ROWS — the multi-line draft and its keystroke/paste handling. · declares `COMPOSER_MAX_ROWS`, `Composer`, `MARGIN`, `CHORD`
- `config.rs` (23.8 KB) — not part of this crate; the sigil-holding subcommand lives in the binary crate. · declares `configure`, `PROMPT`, `NOTHING`, `RULES`, `prompted`, `Held`, `held_for`, `preamble` (+5)
- `confirm.rs` (11.6 KB) — Answer, QuitConfirm, Answered, answer_for — the quit confirmation dialog's key handling, Esc-answers-No. · declares `Answer`, `QuitConfirm`, `is_open`, `highlighted`, `Answered`, `answer_for`
- `descent.rs` (11.1 KB) — not part of this crate; pact/refresh descent orchestration lives in the binary crate. · declares `Descent`, `wanted`, `descend`, `carry_on`
- `editing.rs` (32.5 KB) — not part of this crate; the `e` key handler lives in the binary crate. · declares `edit_press`, `EDITOR_VAR`, `NO_EDITOR`, `Editor`, `came_back`, `edit_target`, `editor_command`, `run_editor`
- `edits.rs` (55.1 KB) — not part of this crate; headless unpact/scope writes live in the binary crate. · declares `Opened`, `new`, `repo_root`, `manifest`, `target`, `opened`, `unpact`, `scope_add` (+8)
- `error.rs` (24.3 KB) — not part of this crate; the binary's Error enum and one_line() live there. · declares `Error`, `from_problems`, `one_line`, `fmt`, `source`, `from`
- `fixture.rs` (12.4 KB) — test-only: tree() and tree_after_a_run() — hand-built Tree fixtures backing app.rs and ui.rs assertions. · declares `tree`, `tree_after_a_run`, `files`
- `input.rs` (146.8 KB) — not part of this crate; Action, action_for, press_for, mouse_action live in the binary crate. · declares `Action`, `action_for`
- `lib.rs` (2.6 KB) — The crate root: module declarations and the full re-export surface (Account, App, Panel, Thread, Composer, ui::draw, watch::Watch, etc).
- `main.rs` (91.6 KB) — not part of this crate; the binary's entry point, terminal lifecycle and event loop live in src/main.rs one level up.
- `pacting.rs` (243.9 KB) — not part of this crate; the `p`/`r` worker-thread orchestration lives in the binary crate. · declares `Pact`, `Reloaded`, `PACT_LOST`, `PACT_CANCELLED`
- `panel.rs` (31.0 KB) — Panel, Showing, Mode, Card, Shown, panel_offset_for — the right-hand pane holding Account/Thread/document cards and card-swap logic. · declares `Panel`, `Showing`, `Mode`, `panel_offset_for`, `showing`, `window_of`, `document_lines`, `show` (+39)
- `prompt.rs` (19.5 KB) — ScopeField, ScopePrompt, Edited, edit_for — the scope-prompt text field and its key handling. · declares `ScopeField`, `new`, `refused`, `directory`, `text`, `rule`, `ScopePrompt`, `is_open` (+4)
- `query.rs` (18.7 KB) — not part of this crate; `warlock stale`/`fresh` subcommand logic lives in the binary crate. · declares `Listing`, `list`, `spelled`, `envelope`, `write_object`, `COMMAND`, `DIRECTORIES`, `wanted` (+6)
- `running.rs` (52.9 KB) — not part of this crate; `warlock pact`/`refresh` subcommand orchestration lives in the binary crate. · declares `pact`, `refresh`, `Progress`, `new`, `total`, `say`, `starting`, `repaired` (+14)
- `scoping.rs` (40.5 KB) — not part of this crate; the `s` key handler lives in the binary crate. · declares `scope_press`, `scope_edit`, `scope_submit`, `with_scope_on`, `no_pact_message`
- `session.rs` (23.4 KB) — not part of this crate; tree loading and Scope/Watched live in the binary crate. · declares `NOT_REFRESHED`, `NOT_WATCHING`, `reload_tree`, `note`, `Scope`, `Watched`, `start`, `off_note` (+8)
- `standing.rs` (8.7 KB) — not part of this crate; repo-root/home resolution lives in the binary crate. · declares `FOR_CLAUDE_MD`, `FOR_SIGILS`, `FOR_CHECK`, `FOR_LISTING`, `FOR_UNPACT`, `FOR_SCOPE_ADD`, `FOR_SCOPE_REMOVE`, `FOR_PACT` (+11)
- `stubs.rs` (1.4 KB) — test-only stand-in Agent/Wired/Converses implementations (Passing, Saying) for binary-crate tests, not exercising cancel or activity reporting. · declares `Passing`, `filling`, `Saying`, `answering`, `run`, `wired`, `turn`, `raised`
- `submission.rs` (8.3 KB) — Submitted, submitted_for — case-sensitive parsing of /brief, /write, /chat versus an ordinary message, plus the one refusal sentence. · declares `Submitted`, `refusal`, `submitted_for`, `REFUSAL`
- `template.rs` (19.0 KB) — DEFAULT_TEMPLATE, brief_template, missing_sections, Error — the brief document shape read from .warlock/brief-template.md. · declares `DEFAULT_TEMPLATE`, `brief_template`, `Error`, `missing_sections`, `TEMPLATE_FILE`, `template_path`, `fmt`, `source` (+2)
- `terminal.rs` (7.5 KB) — not part of this crate; TerminalGuard and Screen live in the binary crate's terminal handling. · declares `Screen`, `TerminalGuard`, `enter`, `install_panic_hook`, `size`, `draw`, `suspended`, `report_mouse` (+3)
- `thread.rs` (44.2 KB) — Thread, Turn, Ending, ending_for — the conversation card of ordered turns and notes built on the account's Log. · declares `Ending`, `line`, `ending_for`, `Turn`, `message`, `answer`, `ending`, `started` (+26)
- `ui.rs` (273.1 KB) — draw() and the whole frame-layout module: tree_height, panel_height, run_header_height, composer_height, panel_width, hit_test, the ASCII MARK. · declares `INDENT`, `GUIDE`, `GUIDE_BRANCH`, `GUIDE_LAST`, `SELECTION_MARKER`, `COLLAPSED_MARKER`, `EXPANDED_MARKER`, `NO_MARKER` (+1)
- `viewing.rs` (18.1 KB) — not part of this crate; the `v` key handler lives in the binary crate. · declares `view_press`
- `watch.rs` (30.5 KB) — NodeSet, WatchPolicy, Watch, Watching — filesystem-change filtering and debounce policy, QUIET_PERIOD/RELOAD_CEILING/COALESCED_RELOADS. · declares `QUIET_PERIOD`, `RELOAD_CEILING`, `COALESCED_RELOADS`, `NodeSet`, `from_tree`, `accepts`, `len`, `is_empty` (+17)
- `wrap.rs` (23.9 KB) — Shape, shape(), rows(), wrapped(), folded() — word-wrap for drawn rows versus byte-preserving fill-wrap for the composer field. · declares `Shape`, `shape`, `rows`, `wrapped`, `folded`, `continued`, `filled`, `break_at` (+1)
- `writing.rs` (64.4 KB) — the `/write` path: proposed_path, write_submit, unfenced, slug_of and next_number turn a thread's answer into a document on disk, refusing unmatched sections rather than repairing them · declares `WRITE_HEADING`, `proposed_path`, `write_opened`, `write_edit`, `write_submit`, `unfenced`, `slug_of`, `BRIEF_PREFIX` (+26)

## Structure

- lib.rs declares every module and re-exports their public items as the crate's API surface
- panel.rs's Card<T> is generic over Shown, implemented by Account (account.rs), Thread (thread.rs) and Vec<Line>, unifying the three cards' windowing
- ui.rs draws from account.rs's Line via wrap.rs's shape()/rows(), and from app.rs's Row/Chrome
- wrap.rs's rows() and ui.rs's panel_row are required to agree on Shape so wrapping and drawing use one width calculation
- thread.rs and account.rs both build on Log, thread.rs also reusing account.rs's tool_line, THINKING and WRITING constants
- composer.rs's compose_for/paste_for call into wrap.rs's folded() for byte-preserving field wrapping
- template.rs's missing_sections is called by the binary crate's write path after brief_template supplies the shape
- watch.rs's WatchPolicy and NodeSet are driven by the binary crate's session.rs on the event-loop thread; nothing here reloads

## Rules

- account.rs and thread.rs never call Instant::now(); every clock reads a `now`/`at` passed in by the caller
- confirm.rs: Esc always answers No, and only KeyEventKind::Press is ever acted on
- prompt.rs and composer.rs: Ctrl-C is never handled here — raw mode already turns it into SIGINT before either module sees it
- wrap.rs: `wrapped` never returns an empty Vec — a blank line is one blank row, never swallowed
- wrap.rs: `folded` keeps every byte, so its rows concatenate back to the original text exactly
- claude.rs: INVOCATION_TIMEOUT is 5 minutes, and --print/--output-format stream-json/--verbose/--include-partial-messages are always passed together
- colour.rs: colour_for has no catch-all arm, so a new NodeState variant fails to compile rather than draw silently
- panel.rs: a Card that is showing is never thrown away by a run finishing with nothing recorded, or by a document card being refilled

## Where to look

- why does the run's newest line keep counting up while a pass is silent → `account.rs` `shown_at`
- how a byte count like 1.6 MB gets formatted → `account.rs` `size`
- which three slash commands exist and how a typo is refused → `submission.rs` `submitted_for`
- how the panel decides which of Account/Thread/Document is drawn → `panel.rs` `Showing`
- how a long line gets broken across rows on screen → `wrap.rs` `rows`
- what a keystroke does to the multi-line draft at the foot of the panel → `composer.rs` `compose_for`
- which colour a stale vs fresh directory is drawn → `colour.rs` `colour_for`
- how debounce and reload timing over filesystem events works → `watch.rs` `WatchPolicy`
- what happens when claude exits non-zero or times out → `claude.rs` `INVOCATION_TIMEOUT`
- the fixed test fixture tree used by app and ui tests → `fixture.rs` `tree`
- how a five-way turn failure becomes one line on the thread card → `thread.rs` `Ending`
- what sections a brief document must carry before it can be written → `template.rs` `missing_sections`
