<!-- warlock -->
> Written by a model pass over this directory alone, to be read before its source and to say which source to read. A map, not a specification: check anything you are about to rely on against the files themselves, and where this document and the code disagree, the code is right.

# src

The warlock-engine crate's core module: the domain logic for pacting a codebase's directories with WARLOCK.md documents, tracking freshness through hashes, and routing a model pass into a validated Fill that renders as a document, all with no subprocess or network access of its own.

## Files

- `agent.rs` (27.8 KB) — Defines the `Agent` port and `Request`/`Response`/`File`/`ChildDocument` types the engine hands a model through, plus `agent::Error` for a failed pass; the binary implements `Agent` by spawning `claude`, this crate never does. · declares `Agent`, `DEFAULT_CONTEXT_TOKENS`, `Request`, `new`, `with_files`, `with_child_documents`, `with_prompt`, `prompt`, `directory`, `files`, `child_documents`, `File`, `present`, `omitted`, `elided`, `summarised` (+18)
- `briefs.rs` (15.7 KB) — Loads and validates `briefs.toml`'s `directory` setting via `load_briefs` and `briefs_path`, refusing absolute or `..`-bearing paths; `DEFAULT_BRIEF_DIRECTORY` is `docs`. · declares `DEFAULT_BRIEF_DIRECTORY`, `briefs_path`, `load_briefs`, `Error`, `BRIEFS_FILE`, `check_relative`, `Briefs`, `default_directory`, `fmt`, `source`
- `claude_md.rs` (31.4 KB) — `write_claude_md` splices a fixed `BODY` section (marked by `BEGIN`/`END`) about warlock's colours, scopes and sigils into the repo's `CLAUDE.md`, returning `Written::Created` or `Updated`. · declares `write_claude_md`, `Written`, `path`, `Error`, `FILE`, `BEGIN`, `END`, `BODY`, `section`, `splice`, `fmt`, `source`
- `clock.rs` (10.9 KB) — `now_rfc3339` is a leap-second-free, dependency-free UTC timestamp formatter built on `civil_from_days`, the Howard Hinnant calendar algorithm. · declares `now_rfc3339`, `SECONDS_PER_DAY`, `DAYS_FROM_SHIFTED_EPOCH_TO_UNIX_EPOCH`, `DAYS_PER_ERA`, `MIN_REPRESENTABLE`, `MAX_REPRESENTABLE`, `rfc3339_from_unix_seconds`, `civil_from_days`
- `decide.rs` (6.9 KB) — `decide_state` is the pure trigger rule: combines an optional `PactEntry` with a computed hash into a `NodeState` (Unpacted/PactedStale/PactedFresh). · declares `decide_state`
- `document.rs` (110.1 KB) — Defines `Fill`, the JSON schema a pass fills in; `check`/`accept`/`Defect` validate it, `Repair` asks for just the bad slots again, and `mend` mechanically fixes what's left using `fallback`; `render` lays out the final markdown with `STAMP`. · declares `ENTRY_CHARS`, `ENTRY_MINIMUM`, `PURPOSE_CHARS`, `LIST_CAP`, `DECLARED_SHOWN`, `ATTEMPTS`, `STAMP`, `Fill`, `Entry`, `Lookup`, `Described`, `stub`, `to_json`, `stub_answer`, `Expected`, `Defect` (+48)
- `fitting.rs` (68.2 KB) — `fit` gathers a directory's files and children's documents into an `agent::Request` and reduces it to fit an agent's context window via elision, sampling and demotion, tracking `Problem`/`Omission`; also computes `carry_hash` for the early-cutoff optimisation. · declares `Fitted`, `fit`, `PER_FILE_BYTE_CAP`, `request_byte_cap`, `REQUEST_BYTE_CAP`, `gather_request`, `carry_hash`, `byte_count`, `carried_bytes`, `Gathered`, `Problem`, `Omission`, `work`, `WALK_DEPTH`, `BYTES_PER_TOKEN_NUMERATOR`, `BYTES_PER_TOKEN_DENOMINATOR` (+48)
- `hash.rs` (24.1 KB) — `subtree_hash` computes the deterministic content digest of a directory tree (sorted relative paths and bytes), respecting `.warlockignore` and `.warlock/` exclusion; version-pinned via `HASH_CONTEXT`. · declares `subtree_hash`, `length`, `Error`, `MANIFEST_DIR`, `HASH_CONTEXT`, `files_under`, `fmt`, `source`
- `ignores.rs` (3.1 KB) — `is_ignored` answers whether a directory itself is excluded by `.warlockignore` rules read from its parent; `FILENAME` names that ignore file. · declares `FILENAME`, `is_ignored`, `walk_one_deep`
- `languages.rs` (28.2 KB) — A per-language `TABLE` of test-file patterns, brace-delimited test `Block`s and declaration prefixes, driving `elide` (drop test bodies), `skeleton` (drop all bodies) and `declared_names` (extract symbol names) used by `fitting.rs`. · declares `Block`, `Language`, `declared_names`, `skeleton`, `Elided`, `elide`, `real`, `work`, `between`, `Manifest`, `to_manifest_path`, `load`, `later`, `after`, `COALESCED_RELOADS`, `Agent` (+40)
- `lib.rs` (2.0 KB) — The crate root: declares all modules and re-exports their public items; small, no logic of its own.
- `load.rs` (53.8 KB) — `load_tree` walks a repository from `.git` down, building a `Tree` of `Node`s with state, scope and files, consulting the pact `Manifest` and reporting `Problem`s for unhashable or invalidly-scoped directories. · declares `load_tree`, `Loaded`, `Problem`, `ProblemCause`, `repository_root`, `Error`, `GIT_DIR`, `MANIFEST_DIR`, `DOCUMENT_FILE`, `fmt`, `source`, `walk`, `builder`, `mark_excluded`, `Directory`, `Builder` (+5)
- `manifest.rs` (46.8 KB) — `Manifest` and `PactEntry` model `.warlock/pacts.toml`: schema-versioned TOML with per-module grants (`granted_hash`, `granted_at`, `carry_hash`) and scopes; also `to_manifest_path`/`from_manifest_path` and atomic `save`/`load`. · declares `ROOT_MODULE`, `SCHEMA_VERSION`, `Manifest`, `new`, `with_entries`, `version`, `entries`, `push`, `entry`, `to_toml_string`, `from_toml_str`, `save`, `load`, `PactEntry`, `with_grant`, `with_carry_hash` (+26)
- `pact.rs` (179.8 KB) — The orchestration core: `pact_subtree`, `refresh_subtree`, `pact_directory` and `unpact_subtree`, using `Observer`, `Repaired`/`Failure`/`Refusal`, and `view_file` to read a document safely. · declares `MANIFEST_DIR`, `DOCUMENT_FILE`, `pact_subtree`, `refresh_subtree`, `unpact_subtree`, `closed_scopes_at_or_below`, `pact_directory`, `pactable_directories`, `view_file`, `Observer`, `Pacting`, `Unwatched`, `PactedSubtree`, `Pacted`, `Repaired`, `Viewed` (+30)
- `scope.rs` (24.5 KB) — `validate_scope`/`validate_sigil` judge scope-name syntax (`RULES`, `Rule`), `scope_covering` finds the nearest ancestor's scope, and `scope_opens_to` tests whether held sigils open it. · declares `RULES`, `Rule`, `validate_scope`, `validate_sigil`, `scope_covering`, `valid_scope`, `scope_opens_to`, `MAXIMUM_CHARACTERS`, `WILDCARD`, `fmt`, `valid_scope_on`, `at_or_above`, `is_scope_character`, `is_separator`
- `sigils.rs` (22.0 KB) — `load_sigils`/`save_sigils` read and write a machine-local `.warlock/config.toml` of held sigils under a derived `project_directory`, keyed off a canonicalised path digest. · declares `project_directory`, `sigils_path`, `load_sigils`, `save_sigils`, `Error`, `SIGIL_DIR`, `SIGIL_FILE`, `PROJECT_CONTEXT`, `DIGEST_CHARACTERS`, `MAXIMUM_NAME_CHARACTERS`, `UNNAMED_ROOT`, `project_dir`, `readable_name`, `Config`, `fmt`, `source`
- `state.rs` (2.7 KB) — `NodeState`, the three-colour enum (Unpacted, PactedStale, PactedFresh) with no fourth variant; `ALL` and `is_pacted`. · declares `NodeState`, `ALL`, `is_pacted`
- `tree.rs` (22.4 KB) — `Node` and `Tree`: the in-memory directory tree with document, state, scope and files, plus `DepthFirst` walking and `StateCounts` tallying. · declares `Node`, `new`, `with_children`, `with_files`, `with_ignored`, `is_ignored`, `with_scope`, `is_leaf`, `IntoDocument`, `Tree`, `root_path`, `walk`, `counts`, `find`, `DepthFirst`, `StateCounts` (+6)

## Structure

- `lib.rs` declares every module and re-exports their public items as the crate's API surface.
- `pact.rs` orchestrates `fitting::fit` to build a request, then loops calling `Agent::run` and validating the answer with `document::accept`/`document::mend` before writing the document.
- `fitting.rs` uses `languages.rs` to elide test bodies or build skeletons of oversized files when trimming a request to budget.
- `load.rs` builds a `Tree` of `tree.rs`'s `Node`s by consulting `manifest.rs`'s `Manifest` and `decide.rs`'s `decide_state`, using `hash.rs`'s `subtree_hash`.
- `pact.rs` and `load.rs` both consult `scope.rs` to resolve and validate scopes stored in `manifest.rs`.
- `hash.rs`, `load.rs`, `pact.rs` and `fitting.rs` all apply the same exclusion rules via `ignores.rs`.
- `sigils.rs` and `manifest.rs` share the atomic write idiom (`temp_file_name`, `write_and_sync`) defined in `manifest.rs`.

## Rules

- No transport type may cross the `Agent` seam; errors arrive as `Option<i32>` and `String`, never a `std::process` type.
- A pass is never shown the directory's own previous `WARLOCK.md`, so no request carries the prior document.
- `Manifest::load` on a missing manifest is `NotFound`, never an empty manifest, since absent and empty are different facts.
- A manifest declaring a schema version other than `SCHEMA_VERSION` is refused rather than read leniently.
- The per-file byte cap is clamped to never exceed the whole-request budget, so a huge file is never read only to be discarded.
- A repair loop runs at most `ATTEMPTS` model passes, and mending runs at most `MEND_PASSES` mechanical fixups, both bounded to guarantee termination.
- A scope string must match `RULES`: 1 to 24 lowercase/digit/-/_ characters, validated but never normalised.
- The wildcard sigil `*` is valid only as a held sigil, never as a directory's scope.
- The hash version string moves only when an unchanged repository would otherwise hash differently.
- Every atomic write (manifest, document, CLAUDE.md, sigils) writes to a dot-prefixed temp file and syncs before renaming over the target.

## Where to look

- how a model pass's answer gets validated and turned into markdown → `document.rs` `render`
- what happens when a directory is too big for the context window → `fitting.rs` `demote_to_budget`
- how test bodies get stripped out of a source file before sending it → `languages.rs` `elide`
- the top-level entry points for pacting or refreshing a subtree → `pact.rs` `pact_subtree`
- how freshness (gray/yellow/green) is decided from a hash → `decide.rs` `decide_state`
- where sigils and scopes are checked against each other → `scope.rs` `scope_opens_to`
- how the repository tree is walked into Node objects → `load.rs` `load_tree`
- where a machine's held sigils are stored → `sigils.rs` `load_sigils`
