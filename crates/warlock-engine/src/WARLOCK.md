<!-- warlock -->
> Written by a model pass over this directory alone, to be read before its source and to say which source to read. A map, not a specification: check anything you are about to rely on against the files themselves, and where this document and the code disagree, the code is right.

# src

Core engine crate for warlock: the domain logic for pacting directories into WARLOCK.md documents, tracking a three-colour freshness ledger over a pact manifest, scoping who may change what, and fitting a directory's files into a model request.

## Files

- `agent.rs` (46.9 KB) — The Agent trait, Request/Response and File/ChildDocument types, and agent::Error — the subprocess-free port between engine and the binary that runs a model. · declares `Agent`, `DEFAULT_CONTEXT_TOKENS`, `Request`, `new`, `with_files`, `with_child_documents`, `with_prompt`, `prompt` (+26)
- `briefs.rs` (21.2 KB) — briefs_path, load_briefs and DEFAULT_BRIEF_DIRECTORY: reads .warlock/briefs.toml, absent-is-default, refuses absolute or `..` directories. · declares `DEFAULT_BRIEF_DIRECTORY`, `briefs_path`, `load_briefs`, `Error`, `BRIEFS_FILE`, `check_relative`, `Briefs`, `default_directory` (+2)
- `claude_md.rs` (44.5 KB) — write_claude_md and Written: splices warlock's delimited section into a repository's CLAUDE.md via BEGIN/END markers, never touching bytes outside them. · declares `write_claude_md`, `Written`, `path`, `Error`, `FILE`, `BEGIN`, `END`, `BODY` (+4)
- `clock.rs` (15.3 KB) — now_rfc3339: dependency-free RFC 3339 UTC timestamp via civil_from_days, Howard Hinnant's calendar arithmetic. · declares `now_rfc3339`, `SECONDS_PER_DAY`, `DAYS_FROM_SHIFTED_EPOCH_TO_UNIX_EPOCH`, `DAYS_PER_ERA`, `MIN_REPRESENTABLE`, `MAX_REPRESENTABLE`, `rfc3339_from_unix_seconds`, `civil_from_days`
- `decide.rs` (10.9 KB) — decide_state: the four-case table turning a PactEntry and a computed hash into a NodeState — unjudged and mismatched both read as stale. · declares `decide_state`
- `document.rs` (67.6 KB) — Fill, Lookup, Defect, Expected, accept/check/render: the schema a model pass fills in, the validation of its answer, and the WARLOCK.md layout logic; ENTRY_CHARS, ATTEMPTS, STAMP, PROMPT. · declares `ENTRY_CHARS`, `ENTRY_MINIMUM`, `PURPOSE_CHARS`, `LIST_CAP`, `DECLARED_SHOWN`, `ATTEMPTS`, `STAMP`, `Fill` (+43)
- `fitting.rs` (130.2 KB) — fit and Problem/Omission: builds an agent::Request from a directory under two byte caps via a gather/reduce/summarise/list ladder, never truncating.
- `hash.rs` (31.2 KB) — subtree_hash: the blake3 digest over every file at and below a directory, sorted relative paths, honouring ignore rules; hash::Error. · declares `subtree_hash`, `length`, `Error`, `MANIFEST_DIR`, `HASH_CONTEXT`, `files_under`, `fmt`, `source`
- `ignores.rs` (5.1 KB) — is_ignored and FILENAME (.warlockignore): the root-check missing from a walker's own rule application. · declares `FILENAME`, `is_ignored`, `walk_one_deep`
- `languages.rs` (39.6 KB) — language_of, declared_names, elide, skeleton: the per-language TABLE for dropping test bodies and keeping declaration lines, used by fitting's elision rung. · declares `Block`, `Language`, `declared_names`, `skeleton`, `Elided`, `elide`, `real`, `work` (+48)
- `lib.rs` (15.7 KB) — Crate root: re-exports the whole public API (Agent, Manifest, Tree, pact_subtree, scope_covering, sigils, etc.) and states the no-subprocess, no-symlink-following promises.
- `load.rs` (72.5 KB) — load_tree, repository_root, Loaded/Problem: walks a directory into a coloured Tree using the manifest above it, marking .warlockignore exclusions without pruning them. · declares `load_tree`, `Loaded`, `Problem`, `ProblemCause`, `repository_root`, `Error`, `GIT_DIR`, `MANIFEST_DIR` (+13)
- `manifest.rs` (64.9 KB) — Manifest, PactEntry, SCHEMA_VERSION, to_manifest_path/from_manifest_path, manifest_path: the .warlock/pacts.toml schema, atomic save/load, and manifest::Error. · declares `ROOT_MODULE`, `SCHEMA_VERSION`, `Manifest`, `new`, `with_entries`, `version`, `entries`, `push` (+34)
- `pact.rs` (228.0 KB) — pact_directory, pact_subtree, refresh_subtree, unpact_subtree, view_file, Observer/Pacting: the operations that run model passes over one or many directories and write documents/grants.
- `scope.rs` (36.1 KB) — validate_scope, validate_sigil, scope_covering, scope_opens_to, Rule, RULES: the scope/sigil vocabulary and boundary-question logic, judging only, never enforcing. · declares `RULES`, `Rule`, `validate_scope`, `validate_sigil`, `scope_covering`, `valid_scope`, `scope_opens_to`, `MAXIMUM_CHARACTERS` (+6)
- `sigils.rs` (32.1 KB) — project_directory, sigils_path, load_sigils, save_sigils: machine-local sigil storage under <home>/.warlock/<project>/config.toml, separate from repository-side scopes. · declares `project_directory`, `sigils_path`, `load_sigils`, `save_sigils`, `Error`, `SIGIL_DIR`, `SIGIL_FILE`, `PROJECT_CONTEXT` (+8)
- `state.rs` (3.8 KB) — NodeState enum (Unpacted, PactedStale, PactedFresh) and StateCounts's per-state vocabulary, with ALL and is_pacted. · declares `NodeState`, `ALL`, `is_pacted`
- `tree.rs` (34.8 KB) — Node, Tree, DepthFirst, StateCounts, IntoDocument: the pure tree shape — path, document, state, ignored, scope, children, files — with no filesystem access. · declares `Node`, `new`, `with_children`, `with_files`, `with_ignored`, `is_ignored`, `with_scope`, `is_leaf` (+14)

## Structure

- load.rs builds a Tree using decide.rs's decide_state and hash.rs's subtree_hash, reading manifest.rs's Manifest
- pact.rs calls fitting.rs's fit to build an agent::Request, runs it through an Agent, then checks the answer with document.rs's accept and lays it out with document.rs's render
- fitting.rs uses languages.rs's elide/skeleton to shrink oversized files before falling back to summarising or listing them
- pact_subtree and refresh_subtree in pact.rs both funnel through one shared describe_and_grant step, then write manifest.rs's Manifest via decide.rs's verdicts
- scope.rs and sigils.rs are independent halves of the boundary question: scope.rs reads manifest.rs entries, sigils.rs reads/writes machine-local config, and pact.rs composes both
- hash.rs and load.rs both use ignores.rs's is_ignored to honor .warlockignore at their own root
- claude_md.rs and briefs.rs each reuse manifest.rs's temp_file_name/write_and_sync atomic-write idiom

## Rules

- Never judged and judged-against-other-content both decide as PactedStale; freshness is only ever granted (decide.rs)
- A subtree hash treats an unreadable file as a fatal error, never a skip, to avoid a false-fresh match (hash.rs)
- No pass is ever shown its own directory's previous WARLOCK.md (agent.rs, fitting.rs)
- A file is never truncated: it is sent whole, elided, summarised, or listed as a name and size (agent.rs, languages.rs, fitting.rs)
- document.rs's Fill schema is checked, not trusted: unknown keys, empty/multiline/too-long entries, unverifiable lookup symbols are all defects
- A scope is a term of the pact: only a pacted directory may carry one, and it is never inherited onto a node from an ancestor (scope.rs, tree.rs)
- A missing briefs.toml or sigils config is the explicit default/NotFound, never silently invented as empty (briefs.rs, sigils.rs)
- Every atomic write goes through a temp file in the same directory plus fs::rename, cleaning up the temp on failure (manifest.rs, claude_md.rs, sigils.rs)
- The manifest schema version is checked before any entry is parsed; an unsupported version is refused outright (manifest.rs)

## Where to look

- how a model pass is actually run → `agent.rs` `Agent`
- why a directory is colored stale or fresh → `decide.rs` `decide_state`
- what goes into a WARLOCK.md and how it's validated → `document.rs` `accept`
- how a big source file gets shrunk before sending to a model → `languages.rs` `skeleton`
- deciding which files/directories go into a model request → `fitting.rs` `fit`
- computing whether a directory's content changed → `hash.rs` `subtree_hash`
- reading or writing .warlock/pacts.toml → `manifest.rs` `Manifest`
- running a pact or refresh over a whole subtree → `pact.rs` `pact_subtree`
- checking whether a scope is well-formed or open to held sigils → `scope.rs` `scope_opens_to`
- where a person's held sigils are stored on this machine → `sigils.rs` `sigils_path`
- the tree data structure and depth-first walking → `tree.rs` `DepthFirst`
- what CLAUDE.md gets written and how → `claude_md.rs` `write_claude_md`
