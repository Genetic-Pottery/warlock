<!-- warlock -->
> Written by a model pass over this directory alone, to be read before its source and to say which source to read. A map, not a specification: check anything you are about to rely on against the files themselves, and where this document and the code disagree, the code is right.

# src

The warlock-tui front end minus the terminal: pure event/state modules driving the panel, tree, composer and conversation cards, plus the impure `claude` child-process seam and the `watch` filesystem watcher, all assembled behind lib.rs's public re-exports.

## Files

- `account.rs` (69.2 KB) — Account, Section, Log, Line, Voice, Outcome — the clocked, ordered record of what one pact did per directory, plus size/money formatting helpers. · declares `THINKING`, `WAITING`, `WRITING`, `Log`, `opened_at`, `started`, `closed_at`, `is_closed`, `row_count`, `freeze`, `push`, `extend_or_open`, `rewrite_or_open`, `rows`, `Outcome`, `Section` (+40)
- `app.rs` (287.8 KB) — App, Row, walk_of/reflow — flattens the engine's Tree into drawable rows, holds selection, collapse and filter state. · declares `Row`, `new`
- `boundary.rs` (10.3 KB) — Reach, Verdict, verdict() — the one shared function judging whether an operator's sigils open a scope, used by both the panel and the shell. · declares `Reach`, `Verdict`, `message`, `verdict`, `closed_scope_message`, `blocking_scopes_message`
- `chatting.rs` (108.4 KB) — Chat, the conversation register (chat/brief) and worker dispatch for /brief, /chat and /write commands. · declares `TURN_LOST`, `BRIEF_COMMAND`, `CHAT_COMMAND`, `WRITE_COMMAND`, `BRIEF_NOTE`, `CHAT_NOTE`, `ALREADY_CHATTING`, `NOT_BRIEFING`
- `check.rs` (23.0 KB) — warlock check subcommand: prints which scope covers a path and whether this machine's sigils open it. · declares `check`, `CHECK`, `PATH`, `SCOPE`, `SIGILS`, `OPENS`, `Checked`, `checked_onto`, `checked`, `prose`, `covering_line`, `holding_line`, `verdict_line`, `sigils_value`
- `claude.rs` (165.5 KB) — ClaudeAgent, Cancel, Activity, Wired — spawns and streams the `claude` CLI child process; the crate's one process-spawning module. · declares `INVOCATION_TIMEOUT`, `BRIEF_EFFORT`, `BRIEF_MODEL`, `PROGRAM`, `ARGS`, `MODEL`, `CONTEXT_TOKENS`, `EFFORT`
- `colour.rs` (6.7 KB) — colour_for and the pinned indexed Color constants (FOCUS_COLOUR, SYSTEM_COLOUR, GUIDE_COLOUR, CONVERSATION_COLOUR) for node states and voices. · declares `colour_for`, `FOCUS_COLOUR`, `SYSTEM_COLOUR`, `GUIDE_COLOUR`, `CONVERSATION_COLOUR`
- `composer.rs` (83.0 KB) — Composer, Pasted, compose_for — the multi-line draft field at the foot of the panel and its keystroke/paste handling. · declares `COMPOSER_MAX_ROWS`, `Composer`, `MARGIN`, `CHORD`
- `config.rs` (23.8 KB) — warlock config subcommand: prompts for and writes this machine's sigils under the home directory. · declares `configure`, `PROMPT`, `NOTHING`, `RULES`, `prompted`, `Held`, `held_for`, `preamble`, `fmt`, `holding`, `sigils_in`, `hold`, `read_line`
- `confirm.rs` (11.6 KB) — QuitConfirm, Answer, Answered, answer_for — the Esc-to-quit Yes/No confirmation dialog. · declares `Answer`, `QuitConfirm`, `is_open`, `highlighted`, `Answered`, `answer_for`
- `descent.rs` (11.1 KB) — Descent enum and descend() — the one shared pact/refresh/unpact descent and single manifest save used by both panel and shell. · declares `Descent`, `wanted`, `descend`, `carry_on`
- `editing.rs` (32.5 KB) — edit_press — the `e` key, spawning $EDITOR on the selected file and reloading the tree afterward. · declares `edit_press`, `EDITOR_VAR`, `NO_EDITOR`, `Editor`, `came_back`, `edit_target`, `editor_command`, `run_editor`
- `edits.rs` (55.1 KB) — Opened, unpact/scope_add/scope_remove — the headless boundary-gated manifest writes. · declares `Opened`, `new`, `repo_root`, `manifest`, `target`, `opened`, `unpact`, `scope_add`, `scope_remove`, `unpacted`, `scoped`, `unscoped`, `scope_on`, `unpacted_line`, `scoped_line`, `unscoped_line`
- `error.rs` (24.3 KB) — Error enum and its Display — the one-line error vocabulary shared by every subcommand and the panel's footer. · declares `Error`, `from_problems`, `one_line`, `fmt`, `source`, `from`
- `fixture.rs` (12.4 KB) — tree() and tree_after_a_run() — hand-written Tree fixtures used by app/ui tests. · declares `tree`, `tree_after_a_run`, `files`
- `input.rs` (146.8 KB) — Action, action_for/press_for/mouse_action — turns key and mouse events into intentions. · declares `Action`, `action_for`
- `lib.rs` (2.6 KB) — Crate root: module declarations and the full list of public re-exports.
- `main.rs` (91.6 KB) — Binary entry point: CLI parsing, subcommand dispatch, and the terminal event loop.
- `pacting.rs` (243.9 KB) — Pact, PactEvent, Work — the worker thread and channel driving the `p`/`r` keys' long-running pact/refresh runs. · declares `Pact`, `Reloaded`, `PACT_LOST`, `PACT_CANCELLED`
- `panel.rs` (31.0 KB) — Panel, Card, Showing, Mode — the right-hand pane holding the account, thread and document cards and their scroll windows. · declares `Panel`, `Showing`, `Mode`, `panel_offset_for`, `showing`, `window_of`, `document_lines`, `show`, `open_account`, `show_document`, `has_content`, `next_card`, `write_run`, `refill_document`, `start_turn`, `note` (+31)
- `prompt.rs` (19.5 KB) — ScopePrompt, ScopeField, Edited, edit_for — the scope-editing text field and its keystroke handling. · declares `ScopeField`, `new`, `refused`, `directory`, `text`, `rule`, `ScopePrompt`, `is_open`, `field`, `Edited`, `edit_for`, `CHORD`
- `query.rs` (18.7 KB) — Listing, list() — warlock stale/fresh subcommands and the shared JSON envelope() helper. · declares `Listing`, `list`, `spelled`, `envelope`, `write_object`, `COMMAND`, `DIRECTORIES`, `wanted`, `word`, `Listed`, `state_word`, `listed_onto`, `listed`, `listing`
- `running.rs` (52.9 KB) — Progress, Report, started/descended — warlock pact and warlock refresh subcommands with Ctrl-C handling. · declares `pact`, `refresh`, `Progress`, `new`, `total`, `say`, `starting`, `repaired`, `documented`, `unchanged`, `skipped`, `named`, `Report`, `onto`, `status`, `report` (+6)
- `scoping.rs` (40.5 KB) — scope_press/scope_edit/scope_submit — the `s` key's full path from press to saved pacts.toml. · declares `scope_press`, `scope_edit`, `scope_submit`, `with_scope_on`, `no_pact_message`
- `session.rs` (23.4 KB) — Scope, Watched, load_app/reload_tree/closed_scope — how the tree on screen is loaded, reloaded and kept boundary-checked. · declares `NOT_REFRESHED`, `NOT_WATCHING`, `reload_tree`, `note`, `Scope`, `Watched`, `start`, `off_note`, `round`, `caught_up`, `start_watching`, `closed_scope`, `sigils_under`, `load_manifest`, `load_app`, `sigils_held`
- `standing.rs` (8.7 KB) — Standing — resolves the repository root, working directory and home directory a subcommand stands on. · declares `FOR_CLAUDE_MD`, `FOR_SIGILS`, `FOR_CHECK`, `FOR_LISTING`, `FOR_UNPACT`, `FOR_SCOPE_ADD`, `FOR_SCOPE_REMOVE`, `FOR_PACT`, `FOR_REFRESH`, `Standing`, `here`, `at`, `repo_root`, `target`, `manifest`, `home` (+3)
- `stubs.rs` (1.4 KB) — Passing and Saying — test-only stand-in Agent/Converses implementations. · declares `Passing`, `filling`, `Saying`, `answering`, `run`, `wired`, `turn`, `raised`
- `submission.rs` (8.3 KB) — Submitted, submitted_for — parses a composer draft into /brief, /write, /chat, Message or Refused. · declares `Submitted`, `refusal`, `submitted_for`, `REFUSAL`
- `template.rs` (19.0 KB) — DEFAULT_TEMPLATE, brief_template, missing_sections — the brief document shape and its section-completeness check. · declares `DEFAULT_TEMPLATE`, `brief_template`, `Error`, `missing_sections`, `TEMPLATE_FILE`, `template_path`, `fmt`, `source`, `sections_of`, `carries`
- `terminal.rs` (7.5 KB) — Screen trait, TerminalGuard, install_panic_hook — raw mode, alternate screen and mouse/paste lifecycle management. · declares `Screen`, `TerminalGuard`, `enter`, `install_panic_hook`, `size`, `draw`, `suspended`, `report_mouse`, `take_terminal`, `drop`, `restore_terminal`
- `thread.rs` (44.2 KB) — Thread, Turn, Ending, ending_for — the conversation card's ordered turns and notes. · declares `Ending`, `line`, `ending_for`, `Turn`, `message`, `answer`, `ending`, `started`, `is_closed`, `Thread`, `new`, `ask`, `note`, `record`, `end`, `turns` (+18)
- `ui.rs` (281.0 KB) — draw() and the measuring entry points (tree_height, panel_height, hit_test, etc.) — the one frame-drawing function. · declares `INDENT`, `GUIDE`, `GUIDE_BRANCH`, `GUIDE_LAST`, `SELECTION_MARKER`, `COLLAPSED_MARKER`, `EXPANDED_MARKER`, `NO_MARKER`, `MARK`
- `viewing.rs` (18.1 KB) — view_press — the `v` key, reading a file's lines onto the document card. · declares `view_press`
- `watch.rs` (30.5 KB) — NodeSet, WatchPolicy, Watch, Watching — the filesystem watcher and its debounce/coalescing policy. · declares `QUIET_PERIOD`, `RELOAD_CEILING`, `COALESCED_RELOADS`, `NodeSet`, `from_tree`, `accepts`, `len`, `is_empty`, `WatchPolicy`, `new`, `follow`, `watched`, `saw`, `accepted`, `due`, `reload_started` (+9)
- `wrap.rs` (24.3 KB) — Shape, shape(), rows(), wrapped(), folded() — word-wrapping for the panel versus column-filling for the composer field. · declares `Shape`, `shape`, `rows`, `wrapped`, `folded`, `continued`, `filled`, `break_at`, `first_character`
- `writing.rs` (57.9 KB) — the `/write` window: proposed_path proposes a filename from a reply's `# ` title and the directory's next number, write_submit writes the path shown at Enter, unfenced strips a wrapping code fence, missing_sections gates it against brief_template. · declares `WRITE_HEADING`, `proposed_path`, `write_opened`, `write_edit`, `write_submit`, `unfenced`, `slug_of`, `BRIEF_PREFIX`, `UNTITLED`, `SLUG_MAX`, `NO_PATH`, `NOTHING_TO_WRITE`, `refused`, `document`, `document_on`, `put` (+13)

## Structure

- main.rs owns the terminal and event loop and is the only caller of most other modules' entry points.
- account.rs and thread.rs both build on the shared Log clock rule, which panel.rs and wrap.rs consume via the Shown trait and rows_of.
- boundary.rs's verdict() is called by session.rs's closed_scope for the panel and by edits.rs's Opened::new for the shell, so both doors refuse the same way.
- descent.rs's descend() is called by both pacting.rs (panel worker) and running.rs (shell), saving the manifest exactly once per run.
- edits.rs's Opened gates running.rs's ran(), so a pact or refresh cannot proceed without the boundary already having been asked.
- input.rs's action_for and press_for consult composer.rs, confirm.rs and prompt.rs to decide which window claims a keystroke.
- scoping.rs and writing.rs both reuse edit_for from prompt.rs and the same ScopePrompt/ScopeField shape for their own text fields.
- check.rs, query.rs and edits.rs all funnel their JSON output through query.rs's envelope() and write_object().
- session.rs's load_app and reload_tree are the two places a Tree reaches app.rs's reseat_on, keeping row state stable across reloads.
- ui.rs draws Line values produced by account.rs, thread.rs and panel.rs, wrapping them through wrap.rs's rows().
- chatting.rs drives claude.rs's ChatAgent/Converses seam and writes results through writing.rs's write_submit.

## Rules

- Nothing in this crate opens a terminal, reads a key, or owns an event loop except main.rs; claude.rs and watch.rs are the two deliberate exceptions that reach past that.
- account.rs never calls Instant::now — every entry point takes the instant it happened at, which is what keeps its clock tests exact.
- The clock rule is Log::shown_at: a line shows the next line's arrival instant, or the log's freeze instant, or now.
- reflow is meant to be the only thing turning all_rows into rows; App::insert_file_row is the sole exception and must apply the same three filters in the same order.
- Esc always answers No in the quit confirmation, so the key meaning 'not this' cannot also be the key that leaves.
- Colours are indexed, never named, so the panel's state-versus-fresh-versus-stale signalling can't drift under a reader's terminal theme.
- colour_for and Line::voice have no catch-all match arm, so a new NodeState or Line variant breaks the build rather than silently taking a fallback colour.
- $EDITOR is split on whitespace only, never handed to a shell, so quoting or injection via a shell is impossible.
- A boundary an operator does not open exits with status 3, never 1, because re-running would never fix it; there is no --force.
- Ctrl-C is answered above raw mode, never inside confirm.rs or prompt.rs, so it cannot be swallowed as text.
- write_submit writes the path shown on screen at Enter, never the originally proposed one, and unfenced is the same function used to build both the proposal and the write.
- The terminal's raw mode, alternate screen, bracketed paste and mouse capture are restored on every exit path including a panic on any thread.

## Where to look

- why a run's cost sometimes shows 'no cost reported' → `account.rs` `spend`
- how a keystroke becomes a tree action versus text typed into a field → `input.rs` `press_for`
- why un-pacting a directory can be refused even though the directory itself is open → `boundary.rs` `blocking_scopes_message`
- how the panel decides which of the three cards is drawn → `panel.rs` `Showing`
- why the panel refuses actions during a run → `pacting.rs` `Pact::press`
- how a brief's proposed filename and number are chosen → `writing.rs` `proposed_path`
- why editing a WARLOCK.md immediately restales its directory → `editing.rs` `came_back`
- how live filesystem updates are debounced and coalesced → `watch.rs` `WatchPolicy`
- what happens to a long line that doesn't fit the panel width → `wrap.rs` `rows`
- how JSON output for CLI subcommands is shaped consistently → `query.rs` `envelope`
- why /brief and /chat change so little about the conversation → `panel.rs` `Mode`
- how a scope string typed into the panel is validated and saved → `scoping.rs` `scope_submit`
