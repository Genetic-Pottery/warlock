<!-- warlock -->
> Written by a model pass over this directory alone, to be read before its source and to say which source to read. A map, not a specification: check anything you are about to rely on against the files themselves, and where this document and the code disagree, the code is right.

# src

The warlock-tui crate's source: the terminal front end's data model, key/mouse translation, panel cards, boundary and write logic, rendering, and the two model-seam agents, plus the binary entry point and its headless subcommands — everything that shows and edits a warlock tree, minus the terminal itself where it can be helped.

## Files

- `account.rs` (91.0 KB) — Account: what one pact did, as an ordered list of Section under directories each holding a clocked Log of Line rows — Entry, Outcome (Wrote/Refused/Cancelled), size, money, clock, tool_line, writing_line, plural. · declares `Log`, `row_count`, `freeze`, `push`, `extend_or_open`, `rewrite_or_open`, `rows`, `Outcome` (+32)
- `app.rs` (443.7 KB) — The front end's whole state: App (rows, selection, panel) plus Row, Focus, Chrome/Sigils, Run/RunHeader, PactToggle, and the movement/pact/scope/view/insert_file_row logic driving them. · declares `Row`, `new`, `file`, `with_scope`, `PactToggle`, `Run`, `RunHeader`, `directory` (+56)
- `boundary.rs` (14.0 KB) — verdict()/Verdict/Reach: the one boundary judgement (scope_covering, closed_scopes_at_or_below) both the panel and shell doors share, plus its footer/error wording. · declares `Reach`, `Verdict`, `message`, `verdict`, `closed_scope_message`, `blocking_scopes_message`
- `chatting.rs` (161.7 KB) — Chat: the conversation as one turn at a time — Chat, Chatting, TurnEvent, Asked; say/ask send a turn, keep_up drains it, compose handles /brief, /chat, /write. · declares `Chat`, `new`, `with_agent`, `directory`, `ask`, `say`, `stop`, `compose` (+19)
- `check.rs` (30.0 KB) — `warlock check <path>` subcommand: prints scope coverage, held sigils and whether they open it, prose or --json. · declares `check`, `Checked`, `checked_onto`, `checked`, `prose`, `covering_line`, `holding_line`, `verdict_line` (+1)
- `claude.rs` (230.4 KB) — Runs `claude` as a child process for both a pass (ClaudeAgent) and a chat turn (ChatAgent), via invoke, Session, Cancel, Activities; render lays a Request out on stdin. · declares `brief_instruction`, `Cancel`, `cancel`, `is_cancelled`, `Activity`, `Activities`, `none`, `report` (+51)
- `colour.rs` (6.2 KB) — colour_for(NodeState) and the fixed indexed colours (focus, guide) — the whole state-to-colour mapping.
- `composer.rs` (118.0 KB) — The composer field: `Composer` (draft, `cursor`, `width`, `is_muted`), `compose_for` and `paste_for` turning a key or paste into `Composed`/`Pasted`, `Composer::window` and `ComposerWindow` for what is drawn. · declares `Composer`, `new`, `at`, `draft`, `is_submittable`, `height`, `window`, `ComposerWindow` (+8)
- `config.rs` (31.3 KB) — `warlock config` subcommand: prompts for and writes this machine's sigils to the home-directory config. · declares `configure`, `prompted`, `Held`, `held_for`, `preamble`, `fmt`, `holding`, `sigils_in` (+2)
- `confirm.rs` (16.2 KB) — QuitConfirm/Answer/Answered and answer_for: the quit confirmation gate as a pure value and key function. · declares `Answer`, `QuitConfirm`, `Answered`, `answer_for`
- `descent.rs` (14.4 KB) — Descent enum (Pact/Refresh/Unpact) and descend(): the one place both TUI and shell run an engine descent and save once. · declares `Descent`, `descend`, `carry_on`
- `editing.rs` (44.7 KB) — The edit key: edit_press resolves edit_target, splits $EDITOR via editor_command into an Editor, runs it in run_editor, and came_back reloads the tree and re-reads the edited document card. · declares `edit_press`, `Editor`, `came_back`, `edit_target`, `editor_command`, `run_editor`
- `edits.rs` (74.6 KB) — The headless writes `warlock unpact <path>`, `warlock scope add <path> <scope>` and `warlock scope remove <path>`, gated through `Opened::new`, `opened`, `Opened::unpacted`, `Opened::scoped`, `Opened::unscoped` and `Opened::scope_on`. · declares `Opened`, `new`, `repo_root`, `target`, `opened`, `unpact`, `scope_add`, `scope_remove` (+7)
- `error.rs` (41.0 KB) — Error: the binary's whole error vocabulary, one Display line each, plus one_line flattening and exit-status mapping support. · declares `Error`, `from_problems`, `one_line`, `fmt`, `source`, `from`
- `fixture.rs` (14.7 KB) — Hand-written test-only Tree fixtures (tree/tree_after_a_run) used by app.rs and ui.rs tests. · declares `tree`, `tree_after_a_run`, `files`
- `input.rs` (193.9 KB) — Keyboard and mouse input turned into intent: Action, MouseAction, action_for, press_for, mouse_action, plus Pressed for the gated loop (quit-confirm, scope/write prompts, composer) and wheel/click helpers. · declares `Action`, `action_for`, `Pressed`, `press_for`, `MouseAction`, `mouse_action`, `is_ctrl_c`, `is_tab` (+2)
- `lib.rs` (24.3 KB) — Crate root: module declarations and the full curated re-export surface (App, Account, Thread, agents, watch, wrap, etc.).
- `main.rs` (135.6 KB) — The binary's shell: Cli/Command/ScopeCommand parsing, main dispatch, init, run's event loop and Session (size/draw/point/press/paste/keep_up), apply_mouse, status_for. · declares `Cli`, `Command`, `ScopeCommand`, `main`, `init`, `run`, `Session`, `size` (+6)
- `pacting.rs` (299.5 KB) — The pact key's whole run: `Pact` starts, drains and stops a worker via `press`/`keep_up`/`stop`; `Running`, `Work`, `CancelGuard`, `PactEvent` and `spawn_pact` carry it; `apply_toggle`/`described`/`pact_message` produce `Toggled`. · declares `Pact`, `Reloaded`, `new`, `with_run`, `with_agent`, `stop`, `press`, `keep_up` (+35)
- `panel.rs` (54.7 KB) — Panel: three cards (account, thread, document), Showing selects one, Mode is Chat/Brief; Card<T> holds offset/follows, Shown trait counts and windows rows via panel_offset_for. · declares `Panel`, `Showing`, `Mode`, `panel_offset_for`, `window_of`, `document_lines`, `open_account`, `show_document` (+20)
- `prompt.rs` (26.0 KB) — ScopePrompt/ScopeField/Edited and edit_for: the scope-editing text field as a pure value and key function. · declares `ScopeField`, `new`, `refused`, `directory`, `text`, `rule`, `ScopePrompt`, `Edited` (+1)
- `query.rs` (26.0 KB) — `warlock stale`/`warlock fresh` subcommands: filtered directory listings, prose or --json, plus shared envelope/write_object helpers. · declares `Listing`, `list`, `spelled`, `envelope`, `write_object`, `Listed`, `listed_onto`, `listed` (+1)
- `running.rs` (69.2 KB) — Headless `warlock pact <path>` and `warlock refresh <path>`: `pact`, `refresh`, `started`, `descended`, `ran`, `listening`, `ending`, `report`, `Report`, `Progress`, `Descent`. · declares `pact`, `refresh`, `Progress`, `say`, `starting`, `documented`, `named`, `Report` (+9)
- `scoping.rs` (47.8 KB) — The scope key end to end: scope_press opens the prompt (refusing on a closed scope or a run in flight), scope_edit dispatches keys, and scope_submit judges and writes .warlock/pacts.toml via with_scope_on. · declares `scope_press`, `scope_edit`, `scope_submit`, `with_scope_on`, `no_pact_message`
- `session.rs` (34.1 KB) — Scope/Watched/load_app/reload_tree/closed_scope/sigils_held: where the tree came from and how it stays true to disk. · declares `reload_tree`, `note`, `Scope`, `Watched`, `start`, `off_note`, `round`, `caught_up` (+6)
- `standing.rs` (13.5 KB) — Standing: the shared prologue (working dir, repo root, manifest, home) every headless subcommand opens with, plus its FOR_* tails. · declares `Standing`, `here`, `repo_root`, `target`, `manifest`, `home`, `spelled`
- `stubs.rs` (4.3 KB) — Passing/Saying: in-memory stand-in Agent/Converses implementations for tests not about the model transport. · declares `Passing`, `filling`, `Saying`, `answering`, `run`, `wired`, `turn`, `raised`
- `submission.rs` (12.8 KB) — Submitted and submitted_for: what a submitted composer draft is — /brief, /write, /chat, a message, or a refusal. · declares `Submitted`, `submitted_for`
- `template.rs` (24.3 KB) — brief_template/DEFAULT_TEMPLATE/missing_sections: the shape a brief takes and the one check on a document's sections. · declares `brief_template`, `Error`, `missing_sections`, `template_path`, `fmt`, `source`, `sections_of`, `carries`
- `terminal.rs` (14.3 KB) — Screen trait, TerminalGuard, restore_terminal, install_panic_hook: the terminal's lifecycle taken and given back whole. · declares `TerminalGuard`, `enter`, `install_panic_hook`, `size`, `draw`, `suspended`, `report_mouse`, `take_terminal` (+2)
- `thread.rs` (59.9 KB) — The chat conversation card: `Thread` of `Entry` (`Turn` or a note), `Ending` for how a turn dies, `ending_for` mapping agent errors, `Thread::ask`/`note`/`record`/`answer`/`end`, and `lines`/`window`/`turns`/`in_flight` for reading it back. · declares `Ending`, `line`, `ending_for`, `Turn`, `message`, `answer`, `Thread`, `ask` (+20)
- `ui.rs` (406.5 KB) — Draws one frame — panel, tree pane, footer, composer and the confirm/scope/path windows — via draw(), and answers layout queries (tree_height, panel_height, hit_test) off the same areas() cut. · declares `draw`, `composer_on_screen`, `tree_height`, `panel_height`, `run_header_height`, `composer_height`, `panel_width`, `Hit` (+44)
- `viewing.rs` (24.5 KB) — view_press: the `v` key's read-only file-to-panel path, no worker, no write, no reload. · declares `view_press`
- `watch.rs` (41.4 KB) — Filesystem watching: NodeSet filters events to walked directories, WatchPolicy times reloads (QUIET_PERIOD, RELOAD_CEILING, COALESCED_RELOADS), Watch/Watching run the real notify watcher. · declares `NodeSet`, `from_tree`, `accepts`, `len`, `is_empty`, `WatchPolicy`, `new`, `follow` (+14)
- `wrap.rs` (33.7 KB) — wrapped/folded/rows/shape: breaking one card line into the rows a panel of given width draws it in. · declares `Shape`, `shape`, `rows`, `wrapped`, `folded`, `continued`, `filled`, `break_at` (+1)
- `writing.rs` (90.4 KB) — The `/write` prompt's path proposal and its submit: proposed_path, write_opened, write_edit, write_submit, unfenced, slug_of, next_number, put, pacted_above, wrote_line, stale_line. · declares `proposed_path`, `write_opened`, `write_edit`, `write_submit`, `unfenced`, `slug_of`, `refused`, `document` (+19)

## Structure

- main.rs builds Session, which holds App (app.rs), Pact (pacting.rs), Chat (chatting.rs) and dispatches key/mouse events via input.rs
- pacting.rs and edits.rs both call descent.rs::descend, which is the one save point for both doors
- boundary.rs::verdict is called from session.rs::closed_scope (panel) and edits.rs (shell), and worded by error.rs via the same message functions
- scoping.rs, editing.rs, viewing.rs each implement one key end to end, all operating on App from app.rs
- ui.rs renders App/Panel/Chrome using wrap.rs to break lines and colour.rs for node colours
- session.rs::load_app resolves Scope and calls session.rs::sigils_held before the event loop starts, feeding Chrome
- watch.rs feeds session.rs::Watched, which calls reload_tree on the event loop's own thread
- claude.rs implements warlock_tui's Wired/Converses used by chatting.rs and main.rs's Session<P,C>
- template.rs and submission.rs feed chatting.rs's /brief and /write dispatch, and writing.rs performs the actual document write

## Rules

- Every clock in account.rs and thread.rs is measured from a caller-supplied Instant, never a module-owned clock
- The manifest is saved exactly once per descent, after the walk, never mid-run (descent.rs)
- ClosedScope and ClosedScopeBelow are worded identically by TUI footer and shell error via boundary.rs's shared functions
- Only KeyEventKind::Press is acted on in confirm.rs, prompt.rs, composer.rs and input.rs — releases/repeats are no-ops
- Ctrl-C is answered by the event loop before confirm.rs/prompt.rs/composer.rs ever see the key
- The four screen colours (colour.rs) are pinned 256-colour indices, deliberately not theme-respecting named colours
- terminal.rs guarantees raw mode/alternate screen/mouse capture are restored on quit, error, panic and suspension alike
- A read (viewing.rs) is never refused during an in-flight run and never triggers a reload

## Where to look

- why does un-pacting refuse more often than pact/refresh → `boundary.rs` `ClosedBelow`
- how does warlock decide a stale vs fresh colour on screen → `colour.rs` `colour_for`
- what happens when I press p on a directory → `pacting.rs` `pact_press`
- how is a scope typed and validated at the s prompt → `prompt.rs` `edit_for`
- what does /write actually write to disk → `writing.rs` `write_submit`
- how does a long account or thread line get wrapped → `wrap.rs` `rows`
- exit status meaning for warlock pact → `running.rs` `Report`
- what does warlock check print → `check.rs` `Checked`
- how does the editor key suspend the terminal → `terminal.rs` `suspended`
- what counts as a filesystem event worth a reload → `watch.rs` `WatchPolicy`
- how is a /brief conversation shaped → `template.rs` `brief_template`
- top-level CLI commands and exit codes → `main.rs` `status_for`
- how the pact/refresh/scope/view keys decide what a keystroke on a given row means → `app.rs` `toggle_pact`
- how a reload preserves the user's collapsed/scrolled/selected view → `app.rs` `reseat_on`
- where the header's repo-relative path and held-sigils text come from → `app.rs` `Chrome`
- how movement keys route to tree vs panel depending on focus → `app.rs` `Focus`
- how the app tells the frame how tall its scroll window is → `ui.rs` `tree_height`
- why a mouse click landed on the wrong row → `ui.rs` `hit_test`
- how the footer shortens its key list on a narrow terminal → `ui.rs` `laid_out_keys`
- how the composer's rows are carved out of the panel's column → `ui.rs` `split_column`
- what happens when Esc is pressed during a pact → `pacting.rs` `CancelGuard`
- why un-pacting a directory can be refused even without an in-flight run → `pacting.rs` `blocked_unpact`
- how the footer's one-line summary of a partly-failed pact is built → `pacting.rs` `pact_message`
- what a refresh run actually iterates over versus a pact → `pacting.rs` `Work`
- how a model pass's stdin text is assembled from files and child documents → `claude.rs` `render`
- how cancelling a running pass actually kills the child process → `claude.rs` `Cancel`
- why a chat turn resumes instead of opening a new claude session → `claude.rs` `Session`
- how tool calls and thinking are reported to the UI without leaking prose → `claude.rs` `Activity`
- how Esc's meaning changes when a pact is running → `input.rs` `action_for`
- what a keystroke does while a scope/write prompt or composer has focus → `input.rs` `press_for`
- what a mouse click or wheel notch does to the tree or panel → `input.rs` `mouse_action`
- why releases and key-repeats never move the selection → `input.rs` `KeyEventKind`
- how a /brief or /write command differs from an ordinary chat message → `chatting.rs` `compose`
- what happens when a turn's worker thread panics without reporting → `chatting.rs` `TURN_LOST`
- how the model's effort/model level changes between chat and brief mode → `chatting.rs` `asking`
- what value comes back to the loop when a /write turn finishes → `chatting.rs` `apply_turn`
- where is the event loop that polls keys and redraws → `main.rs` `run`
- what happens when Ctrl-C is pressed during a run → `main.rs` `CANCELLED`
- how does a mouse click get turned into an app action → `main.rs` `apply_mouse`
- what happens when Enter is pressed on an empty or whitespace-only draft → `composer.rs` `is_submittable`
- how a paste differs from typing the same characters → `composer.rs` `paste_for`
- how the caret position and visible rows are computed together for drawing → `composer.rs` `ComposerWindow`
- why Ctrl-C or other modifier keys never get typed into the draft → `composer.rs` `CHORD`
- how is elapsed time shown for a running pass → `account.rs` `clock`
- how are byte counts formatted for the panel → `account.rs` `size`
- what happens when a directory's pass is turned down by the engine → `account.rs` `record_rejected`
- what line closes a section and how much did it cost → `account.rs` `Outcome`
- where does the /write prompt get its default filename from → `writing.rs` `proposed_path`
- how is a model's fenced-code-block reply unwrapped before writing → `writing.rs` `unfenced`
- which pact does a newly written file mark stale → `writing.rs` `pacted_above`
- how does warlock un-pact a directory from the command line → `edits.rs` `unpact`
- what stops a write from touching a scope this machine doesn't hold → `edits.rs` `Opened::new`
- where is the boundary asked relative to reading the manifest → `edits.rs` `opened`
- does the shell agree with the TUI's p key on un-pacting → `edits.rs` `a_key_press_and_a_shell_prompt_answer_the_same_un_pact_alike`
- why does pact print nothing while a document generation is happening inside a directory → `running.rs` `Progress`
- how does a run decide exit status 3 vs 4 vs 130 → `running.rs` `ending`
- where is the failure list for a pact run assembled and formatted → `running.rs` `report`
- how does Ctrl-C interrupt an in-progress pact run → `running.rs` `listening`
- how a failed or cancelled model call is worded on screen → `thread.rs` `Ending`
- converting a model/agent error into a panel-facing value → `thread.rs` `ending_for`
- why chat turn cost never shows up anywhere → `thread.rs` `record`
- how warlock's own one-off remarks (refusals, staleness warnings) are shown in the chat → `thread.rs` `note`
- how the panel decides which row to scroll to → `panel.rs` `panel_offset_for`
- what happens when a run finishes and another starts → `panel.rs` `open_account`
- how a submitted question reaches the conversation card → `panel.rs` `start_turn`
- why a document read again doesn't change which card is showing → `panel.rs` `refill_document`
- what happens when a scope key is pressed while a pact is running → `scoping.rs` `scope_press`
- how a scope edit gets written to disk → `scoping.rs` `scope_submit`
- how the manifest entry for a module is rebuilt with a new scope → `scoping.rs` `with_scope_on`
- the message shown when a directory has no pact entry to scope → `scoping.rs` `no_pact_message`
- how does warlock decide what program to run for editing → `editing.rs` `editor_command`
- what happens to the document panel after saving in an external editor → `editing.rs` `came_back`
- why is a run in flight refused before the row is checked → `editing.rs` `edit_target`
- what message shows when $EDITOR is unset → `editing.rs` `NO_EDITOR`
- why doesn't a cargo build trigger a reload storm → `watch.rs` `NodeSet`
- how many times does one editor save reload the tree → `watch.rs` `QUIET_PERIOD`
- what happens to events that arrive while a reload is running → `watch.rs` `COALESCED_RELOADS`
- what does it mean for the watcher to fail to start → `watch.rs` `Watching`
