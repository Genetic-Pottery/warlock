<!-- warlock -->
> Written by a model pass over this directory alone, to be read before its source and to say which source to read. A map, not a specification: check anything you are about to rely on against the files themselves, and where this document and the code disagree, the code is right.

# src

The warlock-tui crate: warlock's terminal front end and the binary's own subcommand and event-loop logic, minus the terminal itself — the flattened tree view, the panel, the model seams, the boundary and confirmation prompts, and the headless subcommands (init, config, check, stale/fresh, pact/refresh/unpact, scope) that share its error vocabulary.

## Files

- `account.rs` (93.1 KB) — not read by the pass, over the size cap; name and size only · declares `THINKING`, `WAITING`, `WRITING`, `Log`, `opened_at`, `started`, `closed_at`, `is_closed` (+45)
- `app.rs` (443.7 KB) — App, Row, Chrome, Focus, Sigils, Run, RunHeader, PactToggle: the front end's whole state — flattened tree rows, selection, scrolling, panel/account/thread carrying, focus, pact toggling, reseat_on for reloads. · declares `Row`, `new`, `file`, `with_child_count`, `with_ignored`, `with_document_row`, `with_scope`, `has_children` (+56)
- `boundary.rs` (14.0 KB) — verdict, Verdict, Reach, closed_scope_message, blocking_scopes_message: the one scope-boundary decision shared by the panel's keys and the headless writes. · declares `Reach`, `Verdict`, `message`, `verdict`, `closed_scope_message`, `blocking_scopes_message`
- `chatting.rs` (161.7 KB) — Chat, Chatting, Asked, TurnEvent, spawn_turn, apply_turn: driving a model conversation turn from composer submit through to the thread card. · declares `Chat`, `new`, `with_agent`, `composer`, `set_composer_width`, `write_prompt`, `directory`, `answering` (+31)
- `check.rs` (30.0 KB) — warlock check <path> subcommand: Checked, checked, prose/object rendering of which scope covers a path and whether held sigils open it. · declares `check`, `CHECK`, `PATH`, `SCOPE`, `SIGILS`, `OPENS`, `Checked`, `checked_onto` (+6)
- `claude.rs` (230.4 KB) — ClaudeAgent, ChatAgent, Cancel, Activities, Activity, Converses, Wired: running the claude CLI as a child process for both passes and chat turns. · declares `INVOCATION_TIMEOUT`, `BRIEF_EFFORT`, `BRIEF_MODEL`, `brief_instruction`, `CHAT_INSTRUCTION`, `WRITE_INSTRUCTION`, `Cancel`, `cancel` (+56)
- `colour.rs` (6.2 KB) — colour_for, FOCUS_COLOUR, GUIDE_COLOUR: the fixed 256-colour mapping from NodeState (and focus/guides) to what is drawn. · declares `colour_for`, `FOCUS_COLOUR`, `GUIDE_COLOUR`
- `composer.rs` (118.0 KB) — Composer, ComposerWindow, Composed, Pasted, compose_for, paste_for: the multi-line draft field at the foot of the panel. · declares `COMPOSER_MAX_ROWS`, `Composer`, `new`, `at`, `cursor`, `set_width`, `width`, `set_muted` (+16)
- `config.rs` (31.3 KB) — warlock config subcommand: prompted, sigils_in, hold, preamble — reading and writing the machine-local sigil set for a repository. · declares `configure`, `PROMPT`, `NOTHING`, `RULES`, `prompted`, `Held`, `held_for`, `preamble` (+5)
- `confirm.rs` (16.2 KB) — QuitConfirm, Answer, Answered, answer_for: the quit confirmation gate on Esc/q, independent of App state. · declares `Answer`, `QuitConfirm`, `is_open`, `highlighted`, `Answered`, `answer_for`
- `descent.rs` (14.4 KB) — Descent, descend, carry_on: the one shared middle for pact/refresh/unpact runs used by both the panel's keys and the shell subcommands. · declares `Descent`, `wanted`, `descend`, `carry_on`
- `editing.rs` (44.7 KB) — not read by the pass, over the size cap; name and size only
- `edits.rs` (74.6 KB) — not read by the pass, over the size cap; name and size only
- `error.rs` (41.0 KB) — Error, one_line: the binary's whole error vocabulary, printed as a single line by main for every subcommand and the tree loader. · declares `Error`, `from_problems`, `one_line`, `fmt`, `source`, `from`
- `fixture.rs` (14.7 KB) — tree, tree_after_a_run: hand-written test-only Tree fixtures used by app.rs and ui.rs tests instead of a real load. · declares `tree`, `tree_after_a_run`, `files`
- `input.rs` (193.9 KB) — Action, action_for, Pressed, press_for, MouseAction, mouse_action: mapping raw key/mouse events to intentions across tree, panel, composer and prompts. · declares `Action`, `action_for`, `Pressed`, `press_for`, `MouseAction`, `mouse_action`, `is_ctrl_c`, `is_tab` (+3)
- `lib.rs` (24.3 KB) — Crate root: module declarations and the full public re-export surface (App, Chrome, Account, Thread, Composer, ClaudeAgent, watch types, etc).
- `main.rs` (135.6 KB) — The warlock binary: Cli/Command parsing, main, run, init, Session event loop tying input, drawing and the terminal together. · declares `POLL_INTERVAL`, `CREATED`, `UPDATED`, `Cli`, `Command`, `ScopeCommand`, `main`, `status_for` (+11)
- `pacting.rs` (294.5 KB) — Pact, Running, Work, PactEvent, spawn_pact, pressed_p, refresh_press, Toggled: the p/r keys' worker-thread-driven pact/refresh/unpact runs and their reporting into the account card. · declares `Pact`, `Reloaded`, `new`, `with_run`, `with_agent`, `running`, `stop`, `press` (+42)
- `panel.rs` (54.7 KB) — not read by the pass, over the size cap; name and size only
- `prompt.rs` (26.0 KB) — ScopePrompt, ScopeField, Edited, edit_for: the s-key scope-editing text field, judging nothing about what a scope may be. · declares `ScopeField`, `new`, `refused`, `directory`, `text`, `rule`, `ScopePrompt`, `is_open` (+4)
- `query.rs` (26.0 KB) — warlock stale/fresh subcommands: Listing, listed, state_word, envelope, write_object — filtering a loaded tree by NodeState. · declares `Listing`, `list`, `spelled`, `envelope`, `write_object`, `COMMAND`, `DIRECTORIES`, `wanted` (+6)
- `running.rs` (69.6 KB) — not read by the pass, over the size cap; name and size only
- `scoping.rs` (47.8 KB) — not read by the pass, over the size cap; name and size only
- `session.rs` (34.1 KB) — Scope, Watched, load_app, reload_tree, closed_scope, sigils_held/sigils_under, note: where the on-screen tree came from and how it stays true to disk. · declares `NOT_REFRESHED`, `NOT_WATCHING`, `reload_tree`, `note`, `Scope`, `Watched`, `start`, `off_note` (+8)
- `standing.rs` (13.5 KB) — Standing: the shared subcommand prologue — working dir, repo root, manifest, home directory — and the FOR_* error-tail constants. · declares `FOR_CLAUDE_MD`, `FOR_SIGILS`, `FOR_CHECK`, `FOR_LISTING`, `FOR_UNPACT`, `FOR_SCOPE_ADD`, `FOR_SCOPE_REMOVE`, `FOR_PACT` (+11)
- `stubs.rs` (4.3 KB) — Passing, Saying: in-memory stand-ins for the Agent and Converses seams, for tests not about the model transport itself. · declares `Passing`, `filling`, `Saying`, `answering`, `run`, `wired`, `turn`, `raised`
- `submission.rs` (12.8 KB) — Submitted, submitted_for: parsing a composer draft into /brief, /write, /chat, a message, or a Refused with one explanatory line. · declares `Submitted`, `refusal`, `submitted_for`, `REFUSAL`
- `template.rs` (24.3 KB) — DEFAULT_TEMPLATE, brief_template, missing_sections, Error: the brief document's required shape, read fresh from .warlock/brief-template.md each call. · declares `DEFAULT_TEMPLATE`, `brief_template`, `Error`, `missing_sections`, `TEMPLATE_FILE`, `template_path`, `fmt`, `source` (+2)
- `terminal.rs` (14.3 KB) — Screen, TerminalGuard, install_panic_hook: taking and restoring raw mode/alternate screen/mouse capture on every exit path, including child-process suspension. · declares `Screen`, `TerminalGuard`, `enter`, `install_panic_hook`, `size`, `draw`, `suspended`, `report_mouse` (+3)
- `thread.rs` (59.9 KB) — not read by the pass, over the size cap; name and size only
- `ui.rs` (406.5 KB) — draw, hit_test, tree/panel/composer sizing and layout, header/footer/confirm/scope-window rendering: the whole frame-drawing module. · declares `PANEL_INDENT`, `SAID_MARKER`, `NOTE_MARKER`, `draw`, `composer_on_screen`, `tree_height`, `panel_height`, `run_header_height` (+56)
- `viewing.rs` (24.5 KB) — view_press: the v key's read-only file-view flow, from App::view_target through view_file to App::show_document. · declares `view_press`
- `watch.rs` (41.4 KB) — not read by the pass, over the size cap; name and size only
- `wrap.rs` (33.7 KB) — Shape, shape, rows, wrapped, folded: breaking one panel/account/composer line into on-screen rows at a given width. · declares `Shape`, `shape`, `rows`, `wrapped`, `folded`, `continued`, `filled`, `break_at` (+1)
- `writing.rs` (90.4 KB) — not read by the pass, over the size cap; name and size only

## Structure

- main.rs's Session drives input.rs's action_for/press_for/mouse_action, which produce intentions applied to app.rs's App
- pacting.rs, scoping.rs, viewing.rs and editing.rs are the four per-key modules main.rs's loop dispatches into
- descent.rs's descend() is the one save-once entry point shared by pacting.rs's worker path and running.rs/edits.rs's shell path
- boundary.rs's verdict() is called from session.rs's closed_scope() (panel) and from edits.rs (shell), and its messages are reused by error.rs's Display
- session.rs's load_app/reload_tree build and refresh Scope+App+Tree, feeding app.rs::reseat_on and ui.rs::draw
- chatting.rs's Chat wraps claude.rs's ChatAgent/Converses and writes into thread.rs's Thread, drawn by ui.rs via wrap.rs
- ui.rs calls wrap.rs::rows to lay out every card's lines before drawing, and colour.rs::colour_for to colour tree rows
- config.rs, check.rs, query.rs and standing.rs share error.rs's Error and standing.rs's Standing for the headless subcommands
- lib.rs re-exports the public surface that main.rs and the other binary-only modules (app.rs et al.) consume

## Rules

- colour.rs pins unpacted/stale/fresh/focus/guide colours as fixed 256-indexed values, never named ANSI colours
- error.rs: every Error variant must render as exactly one line (one_line flattens multi-line diagnostics)
- terminal.rs: every exit path (quit, error, panic, child-process suspend) restores the terminal through the same restore_terminal
- boundary.rs: Reach::HereAndBelow (used only by un-pact) is asked only after Reach::Here succeeds — the sharper refusal is said first
- submission.rs: command word matching is case-sensitive and a command word with anything after it is Refused
- prompt.rs and confirm.rs: their modes are values external to App, never fields on App, so an abandoned edit leaves App untouched
- descent.rs: the manifest is saved exactly once, after the descent, never during it
- template.rs: brief_template is read from disk on every call, never cached

## Where to look

- why is a directory refused when I press p, r, or s → `boundary.rs` `verdict`
- what happens when the quit key is pressed → `confirm.rs` `answer_for`
- how a scope gets written to the manifest from the panel → `scoping.rs`
- how /brief, /write, /chat are recognized → `submission.rs` `submitted_for`
- how a long panel line gets broken across rows → `wrap.rs` `wrapped`
- where the CLI's exit status codes are decided → `main.rs` `status_for`
- what warlock check prints → `check.rs` `checked`
- how the terminal survives a child editor process → `terminal.rs` `suspended`
- what happens when a file is read with the v key → `viewing.rs` `view_press`
- where sigils are loaded and cached for the header → `session.rs` `sigils_held`
- how a pact/refresh/unpact run is executed once and saved → `descent.rs` `descend`
- which subcommand needs a repository root and how that's resolved → `standing.rs` `Standing`
