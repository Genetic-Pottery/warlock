<!-- warlock -->
> Written by a model pass over this directory alone, to be read before its source and to say which source to read. A map, not a specification: check anything you are about to rely on against the files themselves, and where this document and the code disagree, the code is right.

# src

The warlock-engine crate's core: pacting a directory means running a model pass over its files and children's documents to fill in a WARLOCK.md, judging freshness by content hash, and persisting the result in a pact manifest.

## Files

- `agent.rs` (27.8 KB) — The `Agent` trait (the port to a model), `Request`/`Response`, `File` states (present, omitted, elided, summarised), `ChildDocument`, and `agent::Error`; no subprocess is spawned here. · declares `Agent`, `DEFAULT_CONTEXT_TOKENS`, `Request`, `new`, `with_files`, `with_child_documents`, `with_prompt`, `prompt` (+26)
- `briefs.rs` (15.7 KB) — `briefs_path`, `load_briefs` and `DEFAULT_BRIEF_DIRECTORY`: reads `.warlock/briefs.toml`'s optional `directory` setting, refusing absolute or `..` paths. · declares `DEFAULT_BRIEF_DIRECTORY`, `briefs_path`, `load_briefs`, `Error`, `BRIEFS_FILE`, `check_relative`, `Briefs`, `default_directory` (+2)
- `claude_md.rs` (31.4 KB) — `write_claude_md` and `Written`: splices a fixed `<!-- warlock:begin -->`/`end` orientation section into the repo's `CLAUDE.md`. · declares `write_claude_md`, `Written`, `path`, `Error`, `FILE`, `BEGIN`, `END`, `BODY` (+4)
- `clock.rs` (10.9 KB) — `now_rfc3339`: a hand-rolled, infallible Gregorian-calendar clock producing a fixed twenty-byte UTC timestamp with no date/time dependency. · declares `now_rfc3339`, `SECONDS_PER_DAY`, `DAYS_FROM_SHIFTED_EPOCH_TO_UNIX_EPOCH`, `DAYS_PER_ERA`, `MIN_REPRESENTABLE`, `MAX_REPRESENTABLE`, `rfc3339_from_unix_seconds`, `civil_from_days`
- `decide.rs` (6.9 KB) — `decide_state`: the pure function combining a `PactEntry` and a computed hash into `NodeState::Unpacted`/`PactedStale`/`PactedFresh`. · declares `decide_state`
- `document.rs` (101.3 KB) — `Fill`, `Lookup`, `Expected`, `Defect`, `accept`, `Repair`, `mend` and `render`: the schema a model pass fills in, its validation, repair-instruction generation and the mend fixpoint that guarantees a defect-free document. · declares `ENTRY_CHARS`, `ENTRY_MINIMUM`, `PURPOSE_CHARS`, `LIST_CAP`, `DECLARED_SHOWN`, `ATTEMPTS`, `STAMP`, `Fill` (+56)
- `fitting.rs` (68.2 KB) — `fit`, `gather_request`, `carry_hash` and the `Omission`/`Problem` types: builds an `agent::Request` within a byte budget (`request_byte_cap`, `PER_FILE_BYTE_CAP`), demoting, eliding or sampling files as needed. · declares `Fitted`, `fit`, `PER_FILE_BYTE_CAP`, `request_byte_cap`, `REQUEST_BYTE_CAP`, `gather_request`, `carry_hash`, `byte_count` (+56)
- `hash.rs` (24.1 KB) — `subtree_hash`: the length-prefixed BLAKE3 digest of a directory's sorted relative paths and file bytes, used to detect staleness. · declares `subtree_hash`, `length`, `Error`, `MANIFEST_DIR`, `HASH_CONTEXT`, `files_under`, `fmt`, `source`
- `ignores.rs` (3.1 KB) — `is_ignored` and the `.warlockignore` filename constant: answers whether a directory itself (not just its descendants) is excluded by ignore rules. · declares `FILENAME`, `is_ignored`, `walk_one_deep`
- `languages.rs` (28.2 KB) — The per-language `TABLE` of test/declaration patterns, `elide`, `skeleton` and `declared_names`: strips test bodies and extracts declared symbol names for many languages. · declares `Block`, `Language`, `declared_names`, `skeleton`, `Elided`, `elide`, `real`, `work` (+48)
- `lib.rs` (2.0 KB) — The crate root: declares every module and re-exports the public API; engine depends on nothing from warlock-tui.
- `load.rs` (53.8 KB) — `load_tree`, `repository_root` and `Loaded`/`Problem`: walks a repository from `.git` upward, builds the `Tree` of `Node`s, colouring each by `decide_state` and scope. · declares `load_tree`, `Loaded`, `Problem`, `ProblemCause`, `repository_root`, `Error`, `GIT_DIR`, `MANIFEST_DIR` (+13)
- `manifest.rs` (46.8 KB) — `Manifest`, `PactEntry`, `SCHEMA_VERSION`, `to_manifest_path`/`from_manifest_path`, `manifest_path`: the `.warlock/pacts.toml` schema, atomic save/load and path conversion. · declares `ROOT_MODULE`, `SCHEMA_VERSION`, `Manifest`, `new`, `with_entries`, `version`, `entries`, `push` (+34)
- `pact.rs` (173.5 KB) — `pact_subtree`, `refresh_subtree`, `pact_directory`, `unpact_subtree`, `view_file`, `Observer`/`Pacting`, `Refusal`/`Failure`/`Repaired`: orchestrates the whole pact/refresh/attempt-repair-mend pipeline and manifest rewriting. · declares `MANIFEST_DIR`, `DOCUMENT_FILE`, `pact_subtree`, `refresh_subtree`, `unpact_subtree`, `closed_scopes_at_or_below`, `pact_directory`, `pactable_directories` (+38)
- `scope.rs` (24.5 KB) — `validate_scope`, `validate_sigil`, `scope_covering`, `scope_opens_to` and `RULES`: the scope/sigil naming rules and the nearest-ancestor-wins boundary lookup. · declares `RULES`, `Rule`, `validate_scope`, `validate_sigil`, `scope_covering`, `valid_scope`, `scope_opens_to`, `MAXIMUM_CHARACTERS` (+6)
- `sigils.rs` (22.0 KB) — `project_directory`, `sigils_path`, `load_sigils`, `save_sigils`: derives a per-checkout machine-local sigil config path under a given home directory and persists held sigils. · declares `project_directory`, `sigils_path`, `load_sigils`, `save_sigils`, `Error`, `SIGIL_DIR`, `SIGIL_FILE`, `PROJECT_CONTEXT` (+8)
- `state.rs` (2.7 KB) — `NodeState`: the exhaustive three-variant freshness enum (`Unpacted`, `PactedStale`, `PactedFresh`) and `ALL`/`is_pacted`. · declares `NodeState`, `ALL`, `is_pacted`
- `tree.rs` (22.4 KB) — `Node`, `Tree`, `DepthFirst`, `StateCounts`, `IntoDocument`: the in-memory directory tree with per-node state, scope, files and children, plus depth-first walking and counting. · declares `Node`, `new`, `with_children`, `with_files`, `with_ignored`, `is_ignored`, `with_scope`, `is_leaf` (+14)

## Structure

- lib.rs declares every module and re-exports their public items as the crate's API
- load.rs calls hash.rs's subtree_hash and decide.rs's decide_state to build tree.rs's Tree of Node/NodeState
- pact.rs calls fitting.rs's fit to build an agent::Request, then document.rs's instructions/accept/mend to fill and render a WARLOCK.md, then manifest.rs to persist a PactEntry
- fitting.rs calls languages.rs's elide/skeleton for per-file reduction and document.rs's Described to record declared names
- pact.rs and load.rs both call scope.rs to resolve and validate scopes/sigils
- manifest.rs's temp_file_name/write_and_sync are reused by claude_md.rs and sigils.rs for atomic writes

## Rules

- agent.rs: no type in this module may cross the seam back into std::process; only Option<i32>/String reach agent::Error
- document.rs: ATTEMPTS is 4 and MEND_PASSES is 4, chosen so the mend fixpoint always terminates
- manifest.rs: SCHEMA_VERSION mismatches are refused rather than read leniently
- hash.rs and fitting.rs walks use follow_links(false), require_git(false), and exclude the .warlock directory by name
- scope.rs: a scope is 1 to 24 characters of lowercase letters, digits, `-` and `_`, validated but never normalised
- pact.rs: a pass is never shown the directory's own previous WARLOCK.md; documents are written before hashing, not read first
- languages.rs: an extension with no table row is sent whole rather than guessed at with a fallback brace-counter

## Where to look

- why a model pass never sees the old document → `agent.rs` `ChildDocument`
- how a directory's freshness is decided → `decide.rs` `decide_state`
- how an oversized file gets reduced or dropped → `fitting.rs` `request_byte_cap`
- the JSON schema a model pass fills in → `document.rs` `Fill`
- how a defective answer is repaired without a second full pass → `document.rs` `Repair`
- what makes a scope closed to an operator → `scope.rs` `scope_opens_to`
- where per-machine sigils are stored → `sigils.rs` `sigils_path`
- how test bodies are stripped from source before sending → `languages.rs` `elide`
- the early cutoff that skips a pass on an unchanged directory → `pact.rs` `carried_document`
