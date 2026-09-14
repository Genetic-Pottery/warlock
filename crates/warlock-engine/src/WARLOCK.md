<!-- warlock -->
> Written by a model pass over this directory alone, to be read before its source and to say which source to read. A map, not a specification: check anything you are about to rely on against the files themselves, and where this document and the code disagree, the code is right.

# src

The warlock-engine crate's core: the domain vocabulary for pacting a repository's directories with WARLOCK.md documents, including the manifest of grants, the freshness decision, the model-agent port, fitting a directory's contents into a request budget, language-aware elision, and the document schema a model pass fills in.

## Files

- `agent.rs` (27.8 KB) — Defines the `Agent` port trait, `Request`, `File` (present/omitted/elided/summarised), `ChildDocument`, `Response` and `Error`; the seam the crate never crosses with a real subprocess. · declares `Agent`, `DEFAULT_CONTEXT_TOKENS`, `Request`, `new`, `with_files`, `with_child_documents`, `with_prompt`, `prompt`, `directory`, `files`, `child_documents`, `File`, `present`, `omitted`, `elided`, `summarised` (+18)
- `briefs.rs` (15.7 KB) — Reads and validates `briefs.toml`'s `directory` setting via `load_briefs` and `briefs_path`, refusing absolute or `..` paths; `DEFAULT_BRIEF_DIRECTORY` is `"docs"`. · declares `DEFAULT_BRIEF_DIRECTORY`, `briefs_path`, `load_briefs`, `Error`, `BRIEFS_FILE`, `check_relative`, `Briefs`, `default_directory`, `fmt`, `source`
- `claude_md.rs` (31.4 KB) — Splices a fixed orientation section (`BODY`, between `BEGIN`/`END` markers) into `CLAUDE.md` via `write_claude_md`, returning `Written::Created` or `Updated`. · declares `write_claude_md`, `Written`, `path`, `Error`, `FILE`, `BEGIN`, `END`, `BODY`, `section`, `splice`, `fmt`, `source`
- `clock.rs` (10.9 KB) — Hand-rolled UTC clock: `now_rfc3339` and the proleptic-Gregorian `civil_from_days`, with no date/time dependency. · declares `now_rfc3339`, `SECONDS_PER_DAY`, `DAYS_FROM_SHIFTED_EPOCH_TO_UNIX_EPOCH`, `DAYS_PER_ERA`, `MIN_REPRESENTABLE`, `MAX_REPRESENTABLE`, `rfc3339_from_unix_seconds`, `civil_from_days`
- `decide.rs` (6.9 KB) — The pure freshness rule `decide_state`, combining a `PactEntry` and a computed hash into a `NodeState` with no filesystem or clock. · declares `decide_state`
- `document.rs` (110.2 KB) — The document schema: `Fill`, `Entry`, `Lookup`, the `PROMPT`, `accept`/`check`/`mend` validation and repair pipeline, `render` to markdown, and constants like `ENTRY_CHARS`, `ATTEMPTS`, `LIST_CAP`, `STAMP`. · declares `ENTRY_CHARS`, `ENTRY_MINIMUM`, `PURPOSE_CHARS`, `LIST_CAP`, `DECLARED_SHOWN`, `ATTEMPTS`, `STAMP`, `Fill`, `Entry`, `Lookup`, `Described`, `stub`, `to_json`, `stub_answer`, `Expected`, `Defect` (+48)
- `fitting.rs` (68.2 KB) — Gathers a directory's files and children's documents into a byte-budgeted `agent::Request` via `fit`/`gather_request`, demoting, sampling or eliding files over caps; also `carry_hash` for the early-cutoff optimisation. · declares `Fitted`, `fit`, `PER_FILE_BYTE_CAP`, `request_byte_cap`, `REQUEST_BYTE_CAP`, `gather_request`, `carry_hash`, `byte_count`, `carried_bytes`, `Gathered`, `Problem`, `Omission`, `work`, `WALK_DEPTH`, `BYTES_PER_TOKEN_NUMERATOR`, `BYTES_PER_TOKEN_DENOMINATOR` (+48)
- `hash.rs` (24.1 KB) — `subtree_hash`, the deterministic content digest of a directory obeying the same ignore rules as the rest of the crate. · declares `subtree_hash`, `length`, `Error`, `MANIFEST_DIR`, `HASH_CONTEXT`, `files_under`, `fmt`, `source`
- `ignores.rs` (3.1 KB) — `.warlockignore` handling via the `ignore` crate; `is_ignored` answers whether a directory's own name is excluded by its ancestors' rules. · declares `FILENAME`, `is_ignored`, `walk_one_deep`
- `languages.rs` (28.2 KB) — Per-language tables (`TABLE`, `Language`, `Block`) driving `elide`, `skeleton` and `declared_names` — test-body stripping and declaration extraction without a real parser. · declares `Block`, `Language`, `declared_names`, `skeleton`, `Elided`, `elide`, `real`, `work`, `between`, `Manifest`, `to_manifest_path`, `load`, `later`, `after`, `COALESCED_RELOADS`, `Agent` (+40)
- `lib.rs` (2.0 KB) — The crate root: module declarations and the full set of `pub use` re-exports forming the public API surface.
- `load.rs` (56.2 KB) — `load_tree` walks a repository into a `Tree` of `Node`s, deciding each node's `NodeState` and scope via the manifest, without hashing unpacted subtrees. · declares `load_tree`, `Loaded`, `Problem`, `ProblemCause`, `repository_root`, `Error`, `GIT_DIR`, `MANIFEST_DIR`, `DOCUMENT_FILE`, `fmt`, `source`, `walk`, `builder`, `mark_excluded`, `Directory`, `Builder` (+5)
- `manifest.rs` (46.8 KB) — `Manifest` and `PactEntry`: TOML (de)serialisation of `.warlock/pacts.toml`, `SCHEMA_VERSION`, path helpers `to_manifest_path`/`from_manifest_path`, and atomic save via `write_and_sync`. · declares `ROOT_MODULE`, `SCHEMA_VERSION`, `Manifest`, `new`, `with_entries`, `version`, `entries`, `push`, `entry`, `to_toml_string`, `from_toml_str`, `save`, `load`, `PactEntry`, `with_grant`, `with_carry_hash` (+26)
- `pact.rs` (189.2 KB) — The largest module: `pact_subtree`, `refresh_subtree`, `pact_directory`, `unpact_subtree`, `unpact_ignored`, `view_file`, the `Observer` trait, and the early-cutoff/mend/repair orchestration tying `document.rs` and `fitting.rs` together. · declares `MANIFEST_DIR`, `DOCUMENT_FILE`, `pact_subtree`, `refresh_subtree`, `unpact_subtree`, `unpact_ignored`, `closed_scopes_at_or_below`, `pact_directory`, `pactable_directories`, `view_file`, `Observer`, `Pacting`, `Unwatched`, `PactedSubtree`, `Pacted`, `Repaired` (+32)
- `scope.rs` (24.5 KB) — Scope and sigil validation and matching: `validate_scope`, `validate_sigil`, `scope_covering`, `scope_opens_to`, and the `Rule` violations with `RULES`. · declares `RULES`, `Rule`, `validate_scope`, `validate_sigil`, `scope_covering`, `valid_scope`, `scope_opens_to`, `MAXIMUM_CHARACTERS`, `WILDCARD`, `fmt`, `valid_scope_on`, `at_or_above`, `is_scope_character`, `is_separator`
- `sigils.rs` (22.0 KB) — Per-project sigil storage under a derived home directory: `project_directory`, `sigils_path`, `load_sigils`, `save_sigils`. · declares `project_directory`, `sigils_path`, `load_sigils`, `save_sigils`, `Error`, `SIGIL_DIR`, `SIGIL_FILE`, `PROJECT_CONTEXT`, `DIGEST_CHARACTERS`, `MAXIMUM_NAME_CHARACTERS`, `UNNAMED_ROOT`, `project_dir`, `readable_name`, `Config`, `fmt`, `source`
- `state.rs` (2.7 KB) — `NodeState`: the three-colour enum (`Unpacted`, `PactedStale`, `PactedFresh`) with no fourth variant. · declares `NodeState`, `ALL`, `is_pacted`
- `tree.rs` (22.4 KB) — `Node`, `Tree`, `DepthFirst` walker and `StateCounts`: the in-memory shape of a loaded repository tree. · declares `Node`, `new`, `with_children`, `with_files`, `with_ignored`, `is_ignored`, `with_scope`, `is_leaf`, `IntoDocument`, `Tree`, `root_path`, `walk`, `counts`, `find`, `DepthFirst`, `StateCounts` (+6)

## Structure

- `lib.rs` declares every module and re-exports their public items as the crate's API surface.
- `load.rs` builds a `Tree` of `Node`s from the filesystem, consulting `manifest.rs` for entries and `decide.rs` to compute each `NodeState`.
- `pact.rs` orchestrates a pact: it calls `fitting.rs::fit` to build a request, `document.rs` to validate and mend the model's answer, `hash.rs::subtree_hash` to grant, and writes through `manifest.rs`.
- `fitting.rs` calls into `languages.rs` to elide test bodies and skeletons before a file counts against the request's byte budget.
- `document.rs` defines the `PROMPT` an `agent::Request` is built with and checks a model's JSON answer against what `fitting.rs`'s `Expected` recorded as shown.
- `pact.rs` and `load.rs` both depend on `scope.rs` to read and validate a directory's scope from its `PactEntry`.
- Nothing here implements `Agent` from `agent.rs`; the binary supplies the real implementation, keeping this crate free of subprocess or network calls.

## Rules

- No transport type may cross the `Agent` seam: errors carry `Option<i32>` and `String`, never a `std::process` type.
- A `Request` carries no slot for the directory's own previous `WARLOCK.md`, so no pass is shown its predecessor's claims.
- `briefs.toml`'s `directory` may not be absolute or contain a `..` component.
- `Manifest` refuses to load any file whose `version` is not `SCHEMA_VERSION`.
- A save is always written to a dot-prefixed temp file and renamed over the target, then `sync_all`ed, so a crash cannot leave a half-written manifest, document, or CLAUDE.md.
- A scope must be 1 to 24 lowercase/digit/`-`/`_` characters beginning with a letter and not ending in a separator.
- The wildcard sigil `*` is valid as a sigil but is refused as a directory scope.
- A repository's root is identified by walking up to the nearest `.git` directory, never a `.git` file.
- `ATTEMPTS` caps a document pass at 4 tries before falling back to mechanical mending rather than refusing.
- Every rendered document opens with the fixed `STAMP` warning it is a map, not a specification.
- `PER_FILE_BYTE_CAP` (1 MiB) bounds any single file read whole, clamped further by the request's own byte cap.

## Where to look

- how a model is actually reached at runtime → `agent.rs` `Agent`
- why a directory's file is sent as a summary instead of its bytes → `agent.rs` `Content`
- the JSON schema a model pass must fill in → `document.rs` `Fill`
- how a defective model answer gets repaired instead of refused → `document.rs` `mend`
- how test bodies get stripped from a source file before it's sent → `languages.rs` `elide`
- how a directory's files are trimmed to fit the model's context window → `fitting.rs` `fit`
- what makes a subtree's freshness hash → `hash.rs` `subtree_hash`
- granting or refreshing a pact end to end → `pact.rs` `pact_subtree`
- whether an operator's sigil opens a scoped directory → `scope.rs` `scope_opens_to`
- where sigils are stored on disk → `sigils.rs` `sigils_path`
- the three colours a directory can be → `state.rs` `NodeState`
- walking or counting the loaded tree → `tree.rs` `DepthFirst`
