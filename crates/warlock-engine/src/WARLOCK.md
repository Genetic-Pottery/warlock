<!-- warlock -->
> Written by a model pass over this directory alone, to be read before its source and to say which source to read. A map, not a specification: check anything you are about to rely on against the files themselves, and where this document and the code disagree, the code is right.

# src

The core of warlock-engine: the port to a model agent, the pact/refresh/document lifecycle for a directory's WARLOCK.md, the manifest and sigil/scope files, and the tree-loading and hashing that decide freshness.

## Files

- `agent.rs` (27.8 KB) — Defines the `Agent` trait, `Request`/`Response`, `File` (present/omitted/elided/summarised), `ChildDocument` and `Error`; the port a binary implements to reach a model, kept free of any transport type. · declares `Agent`, `DEFAULT_CONTEXT_TOKENS`, `Request`, `new`, `with_files`, `with_child_documents`, `with_prompt`, `prompt`, `directory`, `files`, `child_documents`, `File`, `present`, `omitted`, `elided`, `summarised` (+18)
- `briefs.rs` (15.7 KB) — Reads and writes `briefs.toml` beside the manifest: `briefs_path`, `load_briefs`, `DEFAULT_BRIEF_DIRECTORY`, and rules refusing an absolute or `..`-bearing directory. · declares `DEFAULT_BRIEF_DIRECTORY`, `briefs_path`, `load_briefs`, `Error`, `BRIEFS_FILE`, `check_relative`, `Briefs`, `default_directory`, `fmt`, `source`
- `claude_md.rs` (31.4 KB) — Writes and splices the `CLAUDE.md` orientation block: `write_claude_md`, `Written`, the `BEGIN`/`END` markers and the fixed `BODY` explaining colours, scopes and sigils. · declares `write_claude_md`, `Written`, `path`, `Error`, `FILE`, `BEGIN`, `END`, `BODY`, `section`, `splice`, `fmt`, `source`
- `clock.rs` (10.9 KB) — A dependency-free UTC clock to the second: `now_rfc3339` and the Howard Hinnant `civil_from_days` calendar arithmetic. · declares `now_rfc3339`, `SECONDS_PER_DAY`, `DAYS_FROM_SHIFTED_EPOCH_TO_UNIX_EPOCH`, `DAYS_PER_ERA`, `MIN_REPRESENTABLE`, `MAX_REPRESENTABLE`, `rfc3339_from_unix_seconds`, `civil_from_days`
- `decide.rs` (6.9 KB) — The trigger rule alone: `decide_state` maps a `PactEntry` and a computed hash to a `NodeState` with no filesystem or clock involved. · declares `decide_state`
- `document.rs` (110.2 KB) — The document schema and its validation/repair machinery: `Fill`, `Entry`, `Lookup`, `Defect`, `accept`, `Repair`, `mend`, `render`, and the caps `ENTRY_CHARS`, `PURPOSE_CHARS`, `LIST_CAP`, `ATTEMPTS`. · declares `ENTRY_CHARS`, `ENTRY_MINIMUM`, `PURPOSE_CHARS`, `LIST_CAP`, `DECLARED_SHOWN`, `ATTEMPTS`, `STAMP`, `Fill`, `Entry`, `Lookup`, `Described`, `stub`, `to_json`, `stub_answer`, `Expected`, `Defect` (+48)
- `fitting.rs` (68.2 KB) — Fits a directory's files and children's documents into an agent's byte budget: `fit`, `gather_request`, `request_byte_cap`, `Problem`/`Omission`, and the elide/sample/demote ladder. · declares `Fitted`, `fit`, `PER_FILE_BYTE_CAP`, `request_byte_cap`, `REQUEST_BYTE_CAP`, `gather_request`, `carry_hash`, `byte_count`, `carried_bytes`, `Gathered`, `Problem`, `Omission`, `work`, `WALK_DEPTH`, `BYTES_PER_TOKEN_NUMERATOR`, `BYTES_PER_TOKEN_DENOMINATOR` (+48)
- `hash.rs` (24.1 KB) — Computes `subtree_hash`, the content digest driving freshness, walking files under a directory with the same ignore rules as the rest of the crate. · declares `subtree_hash`, `length`, `Error`, `MANIFEST_DIR`, `HASH_CONTEXT`, `files_under`, `fmt`, `source`
- `ignores.rs` (3.1 KB) — `.warlockignore` matching via the `ignore` crate: the `FILENAME` constant and `is_ignored`, answering whether a directory itself is excluded. · declares `FILENAME`, `is_ignored`, `walk_one_deep`
- `languages.rs` (28.2 KB) — The per-language table (`TABLE`) driving `elide`, `skeleton` and `declared_names`, recognising test files and block openers/closers for Rust, Go, TypeScript, Python and more. · declares `Block`, `Language`, `declared_names`, `skeleton`, `Elided`, `elide`, `real`, `work`, `between`, `Manifest`, `to_manifest_path`, `load`, `later`, `after`, `COALESCED_RELOADS`, `Agent` (+40)
- `lib.rs` (2.0 KB) — The crate root: declares every module and re-exports the public API surface (`Agent`, `Manifest`, `Tree`, `pact_subtree`, `load_tree`, etc.).
- `load.rs` (53.8 KB) — Walks a repository into a `Tree` of `Node`s: `load_tree`, `repository_root`, `Loaded`/`Problem`, deciding each node's state and scope from the manifest. · declares `load_tree`, `Loaded`, `Problem`, `ProblemCause`, `repository_root`, `Error`, `GIT_DIR`, `MANIFEST_DIR`, `DOCUMENT_FILE`, `fmt`, `source`, `walk`, `builder`, `mark_excluded`, `Directory`, `Builder` (+5)
- `manifest.rs` (46.8 KB) — The `.warlock/pacts.toml` schema and I/O: `Manifest`, `PactEntry`, `SCHEMA_VERSION`, `to_manifest_path`/`from_manifest_path`, atomic `save`/`load`. · declares `ROOT_MODULE`, `SCHEMA_VERSION`, `Manifest`, `new`, `with_entries`, `version`, `entries`, `push`, `entry`, `to_toml_string`, `from_toml_str`, `save`, `load`, `PactEntry`, `with_grant`, `with_carry_hash` (+26)
- `pact.rs` (179.8 KB) — The pact/refresh engine: `pact_subtree`, `refresh_subtree`, `pact_directory`, `unpact_subtree`, `closed_scopes_at_or_below`, `view_file`, `Observer`, and the early-cutoff carry-hash logic. · declares `MANIFEST_DIR`, `DOCUMENT_FILE`, `pact_subtree`, `refresh_subtree`, `unpact_subtree`, `closed_scopes_at_or_below`, `pact_directory`, `pactable_directories`, `view_file`, `Observer`, `Pacting`, `Unwatched`, `PactedSubtree`, `Pacted`, `Repaired`, `Viewed` (+30)
- `scope.rs` (24.5 KB) — Scope and sigil validation and matching: `validate_scope`, `validate_sigil`, `scope_covering`, `scope_opens_to`, `Rule`, and the `RULES` character-class description. · declares `RULES`, `Rule`, `validate_scope`, `validate_sigil`, `scope_covering`, `valid_scope`, `scope_opens_to`, `MAXIMUM_CHARACTERS`, `WILDCARD`, `fmt`, `valid_scope_on`, `at_or_above`, `is_scope_character`, `is_separator`
- `sigils.rs` (22.0 KB) — Per-machine sigil storage under a derived project directory: `project_directory`, `sigils_path`, `load_sigils`, `save_sigils`. · declares `project_directory`, `sigils_path`, `load_sigils`, `save_sigils`, `Error`, `SIGIL_DIR`, `SIGIL_FILE`, `PROJECT_CONTEXT`, `DIGEST_CHARACTERS`, `MAXIMUM_NAME_CHARACTERS`, `UNNAMED_ROOT`, `project_dir`, `readable_name`, `Config`, `fmt`, `source`
- `state.rs` (2.7 KB) — `NodeState`, the three-colour freshness enum (`Unpacted`, `PactedStale`, `PactedFresh`) with no fourth variant. · declares `NodeState`, `ALL`, `is_pacted`
- `tree.rs` (22.4 KB) — `Node`, `Tree`, `DepthFirst` walk and `StateCounts`: the in-memory tree shape produced by `load.rs` and consumed by rendering and pacting. · declares `Node`, `new`, `with_children`, `with_files`, `with_ignored`, `is_ignored`, `with_scope`, `is_leaf`, `IntoDocument`, `Tree`, `root_path`, `walk`, `counts`, `find`, `DepthFirst`, `StateCounts` (+6)

## Structure

- `pact.rs` calls `fitting::fit` to build an `agent::Request`, then loops calling `Agent::run` and `document::accept`/`document::mend` to produce a `Fill` rendered by `document::render`.
- `load.rs` builds a `Tree` of `Node`s by consulting `manifest::Manifest` for each directory's state via `decide::decide_state` and `hash::subtree_hash`.
- `fitting.rs` reduces files using `languages::elide`/`skeleton`/`declared_names` before falling back to samples or omission.
- `pact.rs` reads and writes scopes through `scope::validate_scope`/`scope_opens_to` and sigils through `sigils::load_sigils`.
- `lib.rs` declares every other module and re-exports their public items as the crate's API.

## Rules

- No transport type may cross the `Agent` trait's seam; exit codes and stderr arrive as `Option<i32>` and `String`, never a `std::process` type.
- A pass is never shown the directory's own previous WARLOCK.md; only files and children's documents form the request.
- `MEND_PASSES` bounds the repair fixpoint at 4, the longest chain any rule can start plus one margin pass.
- `ATTEMPTS` is 4: one first pass plus three cheap repair passes before falling back to mending.
- A manifest declaring any version other than `SCHEMA_VERSION` is refused rather than read leniently.
- A scope must be 1 to 24 lowercase/digit/`-`/`_` characters, never the wildcard, per `RULES`.
- Files over `PER_FILE_BYTE_CAP` (1 MiB) are never read whole and are listed by name and size only.
- A manifest write, sigil save, and document write are all written to a temp file and renamed atomically, never left half-written.

## Where to look

- how a WARLOCK.md gets written and validated → `document.rs` `accept`
- how a directory's request is assembled and trimmed to budget → `fitting.rs` `fit`
- what happens when Agent::run fails or an answer is malformed → `pact.rs` `Refusal`
- how freshness is computed from a hash → `decide.rs` `decide_state`
- how scopes and sigils gate an operation → `scope.rs` `scope_opens_to`
- how the repository tree is walked into nodes → `load.rs` `load_tree`
- which languages get code elision and how → `languages.rs` `TABLE`
- the CLAUDE.md orientation text warlock installs → `claude_md.rs` `BODY`
