<!-- warlock -->
> Written by a model pass over this directory alone, to be read before its source and to say which source to read. A map, not a specification: check anything you are about to rely on against the files themselves, and where this document and the code disagree, the code is right.

# src

The core engine crate for warlock: a freshness ledger over a codebase's documentation. It owns the domain vocabulary — pacts, scopes, sigils, hashing, fitting a directory into a model's context, and rendering the resulting document — with the TUI depending on it and never the reverse.

## Files

- `agent.rs` (27.8 KB) — The Agent trait and Request/Response/File/ChildDocument/Error types: the port to a model, with no transport type crossing the seam and DEFAULT_CONTEXT_TOKENS as the fallback window. · declares `Agent`, `DEFAULT_CONTEXT_TOKENS`, `Request`, `new`, `with_files`, `with_child_documents`, `with_prompt`, `prompt` (+26)
- `briefs.rs` (15.7 KB) — load_briefs and briefs_path read briefs.toml's optional `directory` setting, with DEFAULT_BRIEF_DIRECTORY as the answer when nothing was ever set. · declares `DEFAULT_BRIEF_DIRECTORY`, `briefs_path`, `load_briefs`, `Error`, `BRIEFS_FILE`, `check_relative`, `Briefs`, `default_directory` (+2)
- `claude_md.rs` (31.4 KB) — write_claude_md splices a fixed BODY section between BEGIN/END markers into CLAUDE.md, returning Written::Created or Written::Updated. · declares `write_claude_md`, `Written`, `path`, `Error`, `FILE`, `BEGIN`, `END`, `BODY` (+4)
- `clock.rs` (10.9 KB) — now_rfc3339, an infallible UTC-to-the-second stamp built from Howard Hinnant's civil_from_days, with no date/time dependency. · declares `now_rfc3339`, `SECONDS_PER_DAY`, `DAYS_FROM_SHIFTED_EPOCH_TO_UNIX_EPOCH`, `DAYS_PER_ERA`, `MIN_REPRESENTABLE`, `MAX_REPRESENTABLE`, `rfc3339_from_unix_seconds`, `civil_from_days`
- `decide.rs` (6.9 KB) — decide_state, the pure trigger rule combining a PactEntry with a computed hash into a NodeState. · declares `decide_state`
- `document.rs` (52.3 KB) — Fill, Lookup, Defect, Accepted, Repair and the PROMPT/instructions/accept/render machinery that turns a model's JSON answer into a checked WARLOCK.md. · declares `ENTRY_CHARS`, `ENTRY_MINIMUM`, `PURPOSE_CHARS`, `LIST_CAP`, `DECLARED_SHOWN`, `ATTEMPTS`, `STAMP`, `Fill` (+43)
- `fitting.rs` (68.2 KB) — fit, gather_request, carry_hash and the demotion ladder (trim_to_budget, lift_over_cap, demote_to_budget) that fit a directory's files into request_byte_cap; Problem and Omission report what was left out. · declares `Fitted`, `fit`, `PER_FILE_BYTE_CAP`, `request_byte_cap`, `REQUEST_BYTE_CAP`, `gather_request`, `carry_hash`, `byte_count` (+56)
- `hash.rs` (24.1 KB) — subtree_hash, the deterministic content digest of a directory's own files under blake3, keyed by HASH_CONTEXT. · declares `subtree_hash`, `length`, `Error`, `MANIFEST_DIR`, `HASH_CONTEXT`, `files_under`, `fmt`, `source`
- `ignores.rs` (3.1 KB) — is_ignored and FILENAME (.warlockignore), the one-deep-walk check for whether a directory itself is excluded by its ancestors' rules. · declares `FILENAME`, `is_ignored`, `walk_one_deep`
- `languages.rs` (28.2 KB) — The per-language TABLE of Block/Language rows behind elide, skeleton and declared_names, which reduce source files to declarations plus elision markers. · declares `Block`, `Language`, `declared_names`, `skeleton`, `Elided`, `elide`, `real`, `work` (+48)
- `lib.rs` (2.0 KB) — The crate root: re-exports every public item from the other modules and states the TUI -> engine dependency direction and the no-subprocess rule.
- `load.rs` (53.8 KB) — load_tree and repository_root build a Tree of every directory under a .git root, deciding each Node's NodeState and scope via the manifest. · declares `load_tree`, `Loaded`, `Problem`, `ProblemCause`, `repository_root`, `Error`, `GIT_DIR`, `MANIFEST_DIR` (+13)
- `manifest.rs` (46.8 KB) — Manifest and PactEntry: the pacts.toml schema (SCHEMA_VERSION), load/save, and to_manifest_path/from_manifest_path path conversion. · declares `ROOT_MODULE`, `SCHEMA_VERSION`, `Manifest`, `new`, `with_entries`, `version`, `entries`, `push` (+34)
- `pact.rs` (154.0 KB) — pact_subtree, refresh_subtree, pact_directory, unpact_subtree, view_file and the Observer trait: the full pact/describe/grant pipeline and its Failure/Error/Refusal types. · declares `MANIFEST_DIR`, `DOCUMENT_FILE`, `pact_subtree`, `refresh_subtree`, `unpact_subtree`, `closed_scopes_at_or_below`, `pact_directory`, `pactable_directories` (+36)
- `scope.rs` (24.5 KB) — validate_scope, validate_sigil, scope_covering and scope_opens_to: the boundary rules — RULES, Rule, and the nearest-scope-wins walk. · declares `RULES`, `Rule`, `validate_scope`, `validate_sigil`, `scope_covering`, `valid_scope`, `scope_opens_to`, `MAXIMUM_CHARACTERS` (+6)
- `sigils.rs` (22.0 KB) — load_sigils, save_sigils, project_directory and sigils_path: the machine-local config.toml holding an operator's held sigils. · declares `project_directory`, `sigils_path`, `load_sigils`, `save_sigils`, `Error`, `SIGIL_DIR`, `SIGIL_FILE`, `PROJECT_CONTEXT` (+8)
- `state.rs` (2.7 KB) — NodeState, the three-variant enum (Unpacted, PactedStale, PactedFresh) with no fourth colour, plus its ALL constant. · declares `NodeState`, `ALL`, `is_pacted`
- `tree.rs` (22.4 KB) — Node, Tree, DepthFirst and StateCounts: the in-memory tree structure walked and counted by loaders and front ends. · declares `Node`, `new`, `with_children`, `with_files`, `with_ignored`, `is_ignored`, `with_scope`, `is_leaf` (+14)

## Structure

- lib.rs re-exports every public item from the other modules and is the crate's only public surface map
- load.rs calls decide_state (decide.rs) and subtree_hash (hash.rs) and reads scope_covering-style validation via scope.rs to build a Tree (tree.rs)
- pact.rs calls fit and carry_hash from fitting.rs, then document.rs's instructions/accept/render to turn an Agent response into a written WARLOCK.md, then manifest.rs to record the grant
- fitting.rs calls languages.rs's elide/skeleton/declared_names to reduce files before or during demotion
- hash.rs and load.rs both depend on ignores.rs to decide whether a directory is excluded before walking or hashing it
- sigils.rs and manifest.rs share write_and_sync and temp_file_name from manifest.rs for atomic writes
- claude_md.rs and pact.rs both write documents via the same write-beside-and-rename idiom

## Rules

- No implementation of Agent lives in this module; the engine spawns no subprocess (agent.rs)
- A Request never carries the directory's own previous document (agent.rs)
- No fourth colour exists: NodeState is exactly Unpacted, PactedStale, PactedFresh (state.rs)
- A manifest declaring any version but SCHEMA_VERSION is refused rather than read leniently (manifest.rs)
- A scope on a pacted directory is never normalised, only validated (scope.rs)
- ATTEMPTS is 4: one full pass plus three cheaper repairs over the first answer's defects (document.rs)
- Every problem given up in fitting is reported as a Problem/Omission rather than silently dropped (fitting.rs)
- The HASH_CONTEXT version only changes when an unchanged repository would hash differently (hash.rs)

## Where to look

- how a model is reached without a subprocess → `agent.rs` `Agent`
- why a document's own previous text is never fed back to the pass → `agent.rs` `ChildDocument`
- how a big directory is fit into a model's context window → `fitting.rs` `request_byte_cap`
- how a source file gets reduced to its declarations → `languages.rs` `skeleton`
- what makes a directory fresh vs stale → `decide.rs` `decide_state`
- the JSON schema a model pass must fill in → `document.rs` `Fill`
- how a directory actually gets pacted end to end → `pact.rs` `pact_directory`
- where scopes and sigils are validated → `scope.rs` `validate_scope`
- where an operator's held sigils are stored → `sigils.rs` `load_sigils`
- how CLAUDE.md gets its warlock section → `claude_md.rs` `write_claude_md`
- the pact manifest's on-disk format → `manifest.rs` `PactEntry`
- how the whole repository tree is loaded → `load.rs` `load_tree`
