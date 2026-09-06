<!-- warlock -->
> Written by a model pass over this directory alone, to be read before its source and to say which source to read. A map, not a specification: check anything you are about to rely on against the files themselves, and where this document and the code disagree, the code is right.

# src

Core engine for warlock: the domain vocabulary for pacting a codebase's directories with model-written WARLOCK.md documents, deciding staleness by content hash, and enforcing scope/sigil boundaries — with no dependency on any terminal front end or subprocess.

## Files

- `agent.rs` (46.9 KB) — The Agent trait, Request/Response/File/ChildDocument types and Error enum: the port through which a model pass is reached, implemented by the binary as a subprocess. · declares `Agent`, `DEFAULT_CONTEXT_TOKENS`, `Request`, `new`, `with_files`, `with_child_documents`, `with_prompt`, `prompt` (+26)
- `briefs.rs` (21.2 KB) — Reads `.warlock/briefs.toml`: briefs_path, load_briefs, DEFAULT_BRIEF_DIRECTORY, and the Error enum for a malformed or non-relative directory setting. · declares `DEFAULT_BRIEF_DIRECTORY`, `briefs_path`, `load_briefs`, `Error`, `BRIEFS_FILE`, `check_relative`, `Briefs`, `default_directory` (+2)
- `claude_md.rs` (44.5 KB) — Writes/splices warlock's orientation block into a repository's CLAUDE.md: write_claude_md, splice, section, Written and Error. · declares `write_claude_md`, `Written`, `path`, `Error`, `FILE`, `BEGIN`, `END`, `BODY` (+4)
- `clock.rs` (15.3 KB) — now_rfc3339 and the hand-rolled civil_from_days calendar arithmetic used to stamp grants without a date/time dependency. · declares `now_rfc3339`, `SECONDS_PER_DAY`, `DAYS_FROM_SHIFTED_EPOCH_TO_UNIX_EPOCH`, `DAYS_PER_ERA`, `MIN_REPRESENTABLE`, `MAX_REPRESENTABLE`, `rfc3339_from_unix_seconds`, `civil_from_days`
- `decide.rs` (10.9 KB) — decide_state: the pure function turning a PactEntry and a computed hash into a NodeState (Unpacted/PactedStale/PactedFresh). · declares `decide_state`
- `document.rs` (67.6 KB) — The Fill schema a model pass fills in, Expected, Defect, accept/check, Repair, and render: how a WARLOCK.md's shape is checked and laid out. · declares `ENTRY_CHARS`, `ENTRY_MINIMUM`, `PURPOSE_CHARS`, `LIST_CAP`, `DECLARED_SHOWN`, `ATTEMPTS`, `STAMP`, `Fill` (+43)
- `fitting.rs` (118.8 KB) — Fits a directory's files and children's documents into one request under a byte budget: fit, gather_request, Problem, Omission, PER_FILE_BYTE_CAP. · declares `Fitted`, `fit`, `PER_FILE_BYTE_CAP`, `request_byte_cap`, `REQUEST_BYTE_CAP`, `gather_request`, `carry_hash`, `byte_count` (+56)
- `hash.rs` (31.2 KB) — subtree_hash: the blake3 digest over every file at and below a directory, honouring .gitignore/.warlockignore, used as the staleness trigger. · declares `subtree_hash`, `length`, `Error`, `MANIFEST_DIR`, `HASH_CONTEXT`, `files_under`, `fmt`, `source`
- `ignores.rs` (5.1 KB) — FILENAME (`.warlockignore`) and is_ignored: whether a directory itself is excluded by rules above it. · declares `FILENAME`, `is_ignored`, `walk_one_deep`
- `languages.rs` (39.6 KB) — The per-language table (Language, Block, TABLE) behind elision: declared_names, skeleton, elide drop test bodies while keeping declaration lines. · declares `Block`, `Language`, `declared_names`, `skeleton`, `Elided`, `elide`, `real`, `work` (+48)
- `lib.rs` (15.7 KB) — Crate root: declares the modules and re-exports the public API surface (Agent, Manifest, Tree, NodeState, pact/scope/sigils functions, etc.).
- `load.rs` (72.5 KB) — load_tree and repository_root: builds a coloured Tree from a real directory, walking the filesystem and consulting the manifest; Loaded, Problem. · declares `load_tree`, `Loaded`, `Problem`, `ProblemCause`, `repository_root`, `Error`, `GIT_DIR`, `MANIFEST_DIR` (+13)
- `manifest.rs` (64.9 KB) — Manifest and PactEntry: the TOML schema of `.warlock/pacts.toml`, its load/save, path conversion helpers, SCHEMA_VERSION and Error. · declares `ROOT_MODULE`, `SCHEMA_VERSION`, `Manifest`, `new`, `with_entries`, `version`, `entries`, `push` (+34)
- `pact.rs` (227.8 KB) — pact_directory, pact_subtree, refresh_subtree, unpact_subtree, view_file, closed_scopes_at_or_below, Observer/Pacting, Pacted/PactedSubtree, Refusal/Error. · declares `MANIFEST_DIR`, `DOCUMENT_FILE`, `pact_subtree`, `refresh_subtree`, `unpact_subtree`, `closed_scopes_at_or_below`, `pact_directory`, `pactable_directories` (+33)
- `scope.rs` (36.1 KB) — validate_scope, validate_sigil, scope_covering, scope_opens_to, Rule: the boundary vocabulary of scopes and sigils. · declares `RULES`, `Rule`, `validate_scope`, `validate_sigil`, `scope_covering`, `valid_scope`, `scope_opens_to`, `MAXIMUM_CHARACTERS` (+6)
- `sigils.rs` (32.1 KB) — project_directory, sigils_path, load_sigils, save_sigils: machine-local sigil storage under a home directory, separate from repository scopes. · declares `project_directory`, `sigils_path`, `load_sigils`, `save_sigils`, `Error`, `SIGIL_DIR`, `SIGIL_FILE`, `PROJECT_CONTEXT` (+8)
- `state.rs` (3.8 KB) — NodeState: the three-state enum (Unpacted, PactedStale, PactedFresh) and its ALL constant and is_pacted helper. · declares `NodeState`, `ALL`, `is_pacted`
- `tree.rs` (34.8 KB) — Node, Tree, DepthFirst, StateCounts, IntoDocument: the pure tree shape a load builds and a renderer walks. · declares `Node`, `new`, `with_children`, `with_files`, `with_ignored`, `is_ignored`, `with_scope`, `is_leaf` (+14)

## Structure

- load_tree (load.rs) calls hash::subtree_hash and decide::decide_state to colour each Node, then builds a tree.rs::Tree
- pact.rs calls fitting::gather_request to build an agent::Request, runs it through an Agent, then document.rs::accept/render to write WARLOCK.md
- pact.rs grants freshness by calling hash::subtree_hash and writing into manifest::PactEntry via Manifest::save
- fitting.rs calls languages.rs::elide/skeleton to shrink oversized files before falling back to document.rs's Fill checks
- scope.rs and sigils.rs are consulted by pact.rs to refuse mutating operations across a closed boundary
- lib.rs re-exports the public surface of every other module in this directory; nothing here depends on warlock-tui

## Rules

- The engine spawns no subprocess and opens no socket; Agent is a trait implemented by the binary
- Never judged and judged-against-other-content are both PactedStale; freshness is only ever granted
- A subtree hash covers only files kept by ignore rules, sorted by relative path, never mtime or permissions
- An unreadable file during hashing is a fatal error, never silently skipped
- A manifest with an unsupported schema version is refused outright rather than guessed at
- Entries other than scope are the only ones a pact run may overwrite (PactEntry::overwrite_run_fields)
- A document's shape is checked in code (document.rs); truth of its prose is never checked
- A file's bytes are never truncated mid-byte; omission is always name+size, elision is whole lines only

## Where to look

- how a WARLOCK.md gets written from a model's answer → `document.rs` `render`
- why a directory is stale or fresh → `decide.rs` `decide_state`
- what goes into the subtree hash → `hash.rs` `subtree_hash`
- how a big file is shrunk before being sent to a model → `languages.rs` `elide`
- the request/response types sent to a model → `agent.rs` `Request`
- pacting or refreshing a directory tree → `pact.rs` `pact_subtree`
- the manifest file format → `manifest.rs` `PactEntry`
- whether an operator's sigil opens a scope → `scope.rs` `scope_opens_to`
- where machine-local sigils are stored → `sigils.rs` `sigils_path`
- building a tree from disk → `load.rs` `load_tree`
- the writer for CLAUDE.md orientation text → `claude_md.rs` `write_claude_md`
- fitting files and children into a request budget → `fitting.rs` `gather_request`
