<!-- warlock -->
> Written by a model pass over this directory alone, to be read before its source and to say which source to read. A map, not a specification: check anything you are about to rely on against the files themselves, and where this document and the code disagree, the code is right.

# src

The engine crate's source: it walks a repo into a freshness ledger of directories, decides each node's pact state, runs model passes to fill and render WARLOCK.md documents, and persists manifests, scopes and sigils to disk.

## Files

- `agent.rs` (27.2 KB) — Defines the Agent trait and Request/Response/File/ChildDocument/Error types forming the model-pass port, with no subprocess behind it. · declares `Agent`, `Request`, `new`, `with_files`, `with_child_documents`, `with_prompt`, `prompt`, `directory`, `files`, `child_documents`, `File`, `present`, `omitted`, `elided`, `summarised`, `path` (+16)
- `briefs.rs` (15.7 KB) — Reads briefs.toml via briefs_path/load_briefs, defaulting to DEFAULT_BRIEF_DIRECTORY and rejecting absolute or `..` directory settings. · declares `DEFAULT_BRIEF_DIRECTORY`, `briefs_path`, `load_briefs`, `Error`, `BRIEFS_FILE`, `check_relative`, `Briefs`, `default_directory`, `fmt`, `source`
- `claude_md.rs` (31.4 KB) — Writes/updates CLAUDE.md's warlock section via write_claude_md(), splicing BODY between BEGIN/END markers; returns Written::Created/Updated or Error. · declares `write_claude_md`, `Written`, `path`, `Error`, `FILE`, `BEGIN`, `END`, `BODY`, `section`, `splice`, `fmt`, `source`
- `clock.rs` (10.9 KB) — now_rfc3339 turns SystemTime into a fixed 20-byte UTC RFC3339 string via civil_from_days, a closed-form proleptic Gregorian calendar with no date-parsing dependency. · declares `now_rfc3339`, `SECONDS_PER_DAY`, `DAYS_FROM_SHIFTED_EPOCH_TO_UNIX_EPOCH`, `DAYS_PER_ERA`, `MIN_REPRESENTABLE`, `MAX_REPRESENTABLE`, `rfc3339_from_unix_seconds`, `civil_from_days`
- `decide.rs` (6.9 KB) — decide_state(entry, computed_hash) -> NodeState: the pure rule combining a manifest PactEntry with a computed hash into Unpacted, PactedStale or PactedFresh. · declares `decide_state`
- `document.rs` (151.9 KB) — Fill/Expected/Described model, prompts (PROMPT, SYNTHESIS_PROMPT, FILE_PROMPT), accept/mend/Repair machinery and render for WARLOCK.md generation and validation · declares `ENTRY_CHARS`, `ENTRY_MINIMUM`, `PURPOSE_CHARS`, `LIST_CAP`, `DECLARED_SHOWN`, `ATTEMPTS`, `STAMP`, `Fill`, `Entry`, `Lookup`, `Described`, `writes`, `written_anywhere`, `mentions_tool`, `identifiers`, `stub` (+78)
- `fitting.rs` (22.0 KB) — Builds a directory's pact request from disk: own_files, child_documents, measured, one_file, and carry_hash, plus the Problem/Omission types explaining a file left out. · declares `PER_FILE_BYTE_CAP`, `own_files`, `child_documents`, `measured`, `one_file`, `carry_hash`, `byte_count`, `Problem`, `Omission`, `work`, `WALK_DEPTH`, `elided_or_whole`, `tokens_of`, `PROSE_EXTENSIONS`, `is_prose`, `CARRY_HASH_CONTEXT` (+14)
- `hash.rs` (25.5 KB) — subtree_hash and file_hash: blake3 digests over sorted relative paths and file bytes, path-independent and domain-separated by HASH_CONTEXT and FILE_CONTEXT · declares `file_hash`, `subtree_hash`, `length`, `Error`, `MANIFEST_DIR`, `HASH_CONTEXT`, `FILE_CONTEXT`, `files_under`, `fmt`, `source`
- `ignores.rs` (3.1 KB) — Defines FILENAME (".warlockignore") and is_ignored(), a one-deep parent walk that answers whether a directory's own name survives its ancestors' gitignore-syntax rules. · declares `FILENAME`, `is_ignored`, `walk_one_deep`
- `languages.rs` (21.3 KB) — Per-language table (TABLE) driving declared_names and elide: extensions, test suffixes/prefixes, and inline-test Block markers used to find declarations and elide test bodies outside them. · declares `Block`, `Language`, `declared_names`, `Elided`, `elide`, `is_test_file`, `declares`, `VISIBILITY`, `KEYWORDS`, `outside_blocks`, `without_visibility`, `past_receiver`, `first_identifier`, `language_of`, `marker_for`, `byte_length` (+4)
- `lib.rs` (2.2 KB) — Crate root declaring the engine's modules and re-exporting their public API (Agent, decide_state, Pacted, Manifest, Tree, NodeState, and the rest) for the TUI to consume.
- `load.rs` (56.2 KB) — load.rs — load_tree walks a repo into a Tree of Node, reading .warlock manifest state, scopes, and .warlockignore exclusion via Builder, repository_root, Loaded, Problem/ProblemCause, Error. · declares `load_tree`, `Loaded`, `Problem`, `ProblemCause`, `repository_root`, `Error`, `GIT_DIR`, `MANIFEST_DIR`, `DOCUMENT_FILE`, `fmt`, `source`, `walk`, `builder`, `mark_excluded`, `Directory`, `Builder` (+5)
- `manifest.rs` (48.5 KB) — Manifest and PactEntry: reads and atomically saves .warlock/pacts.toml, tracking per-module scope, granted_hash, carry_hash and per-file lines. · declares `ROOT_MODULE`, `SCHEMA_VERSION`, `Manifest`, `new`, `with_entries`, `version`, `entries`, `push`, `entry`, `to_toml_string`, `from_toml_str`, `save`, `load`, `PactEntry`, `with_grant`, `with_carry_hash` (+29)
- `pact.rs` (184.5 KB) — Pacts and refreshes a directory subtree, running per-file then synthesis passes and recording grants; defines pact_subtree, refresh_subtree, pact_directory, assemble_lines, synthesise, unpact_subtree, closed_scopes_at_or_below. · declares `MANIFEST_DIR`, `DOCUMENT_FILE`, `pact_subtree`, `refresh_subtree`, `unpact_subtree`, `unpact_ignored`, `closed_scopes_at_or_below`, `pact_directory`, `pactable_directories`, `synthesise`, `Synthesised`, `assemble_lines`, `Assembled`, `describe_file`, `DescribedFile`, `view_file` (+42)
- `scope.rs` (24.5 KB) — Defines Rule, validate_scope and validate_sigil for the scope character rules, plus scope_covering (nearest-ancestor lookup) and scope_opens_to for sigil matching · declares `RULES`, `Rule`, `validate_scope`, `validate_sigil`, `scope_covering`, `valid_scope`, `scope_opens_to`, `MAXIMUM_CHARACTERS`, `WILDCARD`, `fmt`, `valid_scope_on`, `at_or_above`, `is_scope_character`, `is_separator`
- `sigils.rs` (22.0 KB) — Derives a per-checkout sigil config directory from a home and root, with project_directory, sigils_path, load_sigils and save_sigils reading/writing config.toml's sigils list. · declares `project_directory`, `sigils_path`, `load_sigils`, `save_sigils`, `Error`, `SIGIL_DIR`, `SIGIL_FILE`, `PROJECT_CONTEXT`, `DIGEST_CHARACTERS`, `MAXIMUM_NAME_CHARACTERS`, `UNNAMED_ROOT`, `project_dir`, `readable_name`, `Config`, `fmt`, `source`
- `state.rs` (2.7 KB) — Defines NodeState — Unpacted, PactedStale, PactedFresh — the three freshness variants, with ALL for exhaustive iteration and is_pacted() to test pacted-ness. · declares `NodeState`, `ALL`, `is_pacted`
- `tree.rs` (22.4 KB) — Node and Tree: the freshness ledger's tree, with NodeState-tallying StateCounts, depth-first DepthFirst walk, and find/walk/counts over Node's path, document, state, scope, ignored, children and files. · declares `Node`, `new`, `with_children`, `with_files`, `with_ignored`, `is_ignored`, `with_scope`, `is_leaf`, `IntoDocument`, `Tree`, `root_path`, `walk`, `counts`, `find`, `DepthFirst`, `StateCounts` (+6)

## Structure

- lib.rs — Crate root declaring the engine's modules and re-exporting their public API (Agent, decide_state, Pacted, Manifest, Tree, NodeState, and the rest) for the TUI to consume.
- load.rs — load_tree walks a repo into a Tree of Node, reading .warlock manifest state, scopes, and .warlockignore exclusion via Builder, repository_root, Loaded, Problem/ProblemCause, Error.
- tree.rs — Node and Tree: the freshness ledger's tree, with NodeState-tallying StateCounts, depth-first DepthFirst walk, and find/walk/counts over Node's path, document, state, scope, ignored, children and files.
- decide.rs — decide_state(entry, computed_hash) -> NodeState: the pure rule combining a manifest PactEntry with a computed hash into Unpacted, PactedStale or PactedFresh.
- pact.rs — Pacts and refreshes a directory subtree, running per-file then synthesis passes and recording grants; defines pact_subtree, refresh_subtree, pact_directory, assemble_lines, synthesise, unpact_subtree, closed_scopes_at_or_below.
- fitting.rs — Builds a directory's pact request from disk: own_files, child_documents, measured, one_file, and carry_hash, plus the Problem/Omission types explaining a file left out.
- agent.rs — Defines the Agent trait and Request/Response/File/ChildDocument/Error types forming the model-pass port, with no subprocess behind it.
- document.rs — Fill/Expected/Described model, prompts (PROMPT, SYNTHESIS_PROMPT, FILE_PROMPT), accept/mend/Repair machinery and render for WARLOCK.md generation and validation
- manifest.rs — Manifest and PactEntry: reads and atomically saves .warlock/pacts.toml, tracking per-module scope, granted_hash, carry_hash and per-file lines.
- hash.rs — subtree_hash and file_hash: blake3 digests over sorted relative paths and file bytes, path-independent and domain-separated by HASH_CONTEXT and FILE_CONTEXT
- scope.rs — Defines Rule, validate_scope and validate_sigil for the scope character rules, plus scope_covering (nearest-ancestor lookup) and scope_opens_to for sigil matching
- sigils.rs — Derives a per-checkout sigil config directory from a home and root, with project_directory, sigils_path, load_sigils and save_sigils reading/writing config.toml's sigils list.

## Where to look

- how is a directory's freshness state determined → `decide.rs` `decide_state`
- how does the repo tree get walked and built → `load.rs` `load_tree`
- what does a WARLOCK.md pact request look like before it hits the model → `fitting.rs`
- what shape does a model implementation have to satisfy → `agent.rs` `Agent`
- how is CLAUDE.md's warlock section written or updated → `claude_md.rs` `write_claude_md`
- where are per-directory pact grants stored → `manifest.rs` `Manifest`
- how is a subtree's content hashed for staleness checks → `hash.rs` `subtree_hash`
- how does .warlockignore affect a directory's own visibility → `ignores.rs` `is_ignored`
- how are scopes and sigils validated against each other → `scope.rs` `scope_opens_to`
- where do sigils get loaded or saved per checkout → `sigils.rs` `load_sigils`
- how does pacting a directory run per-file then synthesis passes → `pact.rs` `pact_subtree`
- where do briefs.toml settings get read and defaulted → `briefs.rs` `load_briefs`
