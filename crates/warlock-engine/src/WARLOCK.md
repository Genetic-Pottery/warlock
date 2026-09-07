<!-- warlock -->
> Written by a model pass over this directory alone, to be read before its source and to say which source to read. A map, not a specification: check anything you are about to rely on against the files themselves, and where this document and the code disagree, the code is right.

# src

Core engine crate for warlock: the domain vocabulary for pacting a repository — the pact manifest, the freshness ledger, the tree of module documents, the agent seam, fitting a directory into a model request, and the document shape a pass fills in. No terminal, no subprocess, no network.

## Files

- `agent.rs` (46.9 KB) — The Agent trait, Request/Response, File and ChildDocument: the port the engine asks a model through, with no process type crossing it; Error enum for transport failures. · declares `Agent`, `DEFAULT_CONTEXT_TOKENS`, `Request`, `new`, `with_files`, `with_child_documents`, `with_prompt`, `prompt` (+26)
- `briefs.rs` (21.2 KB) — Reads `.warlock/briefs.toml`: briefs_path, load_briefs, DEFAULT_BRIEF_DIRECTORY, and the Error variants for a malformed or unsafe brief directory. · declares `DEFAULT_BRIEF_DIRECTORY`, `briefs_path`, `load_briefs`, `Error`, `BRIEFS_FILE`, `check_relative`, `Briefs`, `default_directory` (+2)
- `claude_md.rs` (44.5 KB) — Writes/splices warlock's delimited section into a repository's CLAUDE.md: write_claude_md, Written, BEGIN/END markers, Error. · declares `write_claude_md`, `Written`, `path`, `Error`, `FILE`, `BEGIN`, `END`, `BODY` (+4)
- `clock.rs` (15.3 KB) — now_rfc3339: the wall clock as an RFC 3339 UTC timestamp, using hand-rolled Gregorian calendar arithmetic (civil_from_days) instead of a date/time dependency. · declares `now_rfc3339`, `SECONDS_PER_DAY`, `DAYS_FROM_SHIFTED_EPOCH_TO_UNIX_EPOCH`, `DAYS_PER_ERA`, `MIN_REPRESENTABLE`, `MAX_REPRESENTABLE`, `rfc3339_from_unix_seconds`, `civil_from_days`
- `decide.rs` (10.9 KB) — decide_state: the staleness verdict from a PactEntry and a computed hash — Unpacted, PactedStale or PactedFresh, with no fourth state. · declares `decide_state`
- `document.rs` (71.0 KB) — The WARLOCK.md schema: Fill, Lookup, Expected, Defect, accept, Repair, render — checks a model's JSON answer against the request and lays out the final document. · declares `ENTRY_CHARS`, `ENTRY_MINIMUM`, `PURPOSE_CHARS`, `LIST_CAP`, `DECLARED_SHOWN`, `ATTEMPTS`, `STAMP`, `Fill` (+43)
- `fitting.rs` (130.1 KB) — fit: builds an agent::Request for a directory under byte caps, degrading files through elide/summarise/list rungs; Problem reports what was demoted.
- `hash.rs` (31.2 KB) — subtree_hash: blake3 digest over every file at and below a directory, sorted relative paths, honouring ignore rules; the mechanical trigger for staleness. · declares `subtree_hash`, `length`, `Error`, `MANIFEST_DIR`, `HASH_CONTEXT`, `files_under`, `fmt`, `source`
- `ignores.rs` (5.1 KB) — is_ignored and FILENAME (`.warlockignore`): whether a path is excluded by the repository's own ignore file, checked against its parent since a walker skips its own root. · declares `FILENAME`, `is_ignored`, `walk_one_deep`
- `languages.rs` (39.6 KB) — Per-language table (TABLE, Language, Block) for eliding test bodies and extracting declared_names/skeleton from source files to shrink requests without truncation. · declares `Block`, `Language`, `declared_names`, `skeleton`, `Elided`, `elide`, `real`, `work` (+48)
- `lib.rs` (15.7 KB) — Crate root: declares all modules and re-exports the public API surface (Agent, Manifest, Tree, subtree_hash, pact_directory, pact_subtree, scope functions, etc).
- `load.rs` (72.5 KB) — load_tree: builds a coloured Tree from a real directory, finding the repository root, reading the manifest, walking files, and reporting non-fatal Problems. · declares `load_tree`, `Loaded`, `Problem`, `ProblemCause`, `repository_root`, `Error`, `GIT_DIR`, `MANIFEST_DIR` (+13)
- `manifest.rs` (64.9 KB) — Manifest and PactEntry: the `.warlock/pacts.toml` schema, TOML round-tripping, atomic save/load, and manifest-relative path conversion (to/from_manifest_path). · declares `ROOT_MODULE`, `SCHEMA_VERSION`, `Manifest`, `new`, `with_entries`, `version`, `entries`, `push` (+34)
- `pact.rs` (226.8 KB) — pact_directory, pact_subtree, refresh_subtree, unpact_subtree, view_file, Observer/Pacting: the operations that run model passes over directories and record grants.
- `scope.rs` (36.1 KB) — validate_scope, validate_sigil, scope_covering, scope_opens_to, Rule: what a scope/sigil string may be and which boundary covers a path. · declares `RULES`, `Rule`, `validate_scope`, `validate_sigil`, `scope_covering`, `valid_scope`, `scope_opens_to`, `MAXIMUM_CHARACTERS` (+6)
- `sigils.rs` (32.1 KB) — Machine-local sigils under `<home>/.warlock/<project>/config.toml`: project_directory, sigils_path, load_sigils, save_sigils, Error. · declares `project_directory`, `sigils_path`, `load_sigils`, `save_sigils`, `Error`, `SIGIL_DIR`, `SIGIL_FILE`, `PROJECT_CONTEXT` (+8)
- `state.rs` (3.8 KB) — NodeState: the three-state vocabulary (Unpacted, PactedStale, PactedFresh) with no fourth 'unknown' state. · declares `NodeState`, `ALL`, `is_pacted`
- `tree.rs` (34.8 KB) — Node, Tree, DepthFirst, StateCounts: the pure shape of the project tree, with ignored/scope as stored facts rather than derived ones. · declares `Node`, `new`, `with_children`, `with_files`, `with_ignored`, `is_ignored`, `with_scope`, `is_leaf` (+14)

## Structure

- load.rs calls decide.rs's decide_state and hash.rs's subtree_hash to colour each Node in tree.rs
- pact.rs calls fitting.rs's fit to build an agent::Request, then agent.rs's Agent::run, then document.rs's accept/render to produce a WARLOCK.md
- fitting.rs calls languages.rs's elide/skeleton/declared_names to shrink files before document.rs checks the model's answer against them
- manifest.rs's temp_file_name and write_and_sync are reused by claude_md.rs and sigils.rs for the same atomic write idiom
- scope.rs's valid_scope is called by load.rs to fill Node::scope and report scope Problems
- lib.rs re-exports the public items of every other module in this directory; no module here depends on lib.rs

## Rules

- Never a truncated file: content is sent whole, elided by whole lines, summarised as prose, or listed as name+size — never cut mid-byte (fitting.rs, languages.rs)
- A model pass never sees its own directory's previous WARLOCK.md (agent.rs, fitting.rs)
- Freshness (PactedFresh) is only ever granted by a pass that just ran; unjudged is stale, never a fourth 'unknown' state (decide.rs, state.rs)
- An unreadable file is a hash Error, never silently skipped (hash.rs)
- Manifest paths are stored relative with forward slashes so two clones at different absolute paths agree (manifest.rs, hash.rs)
- A scope string is never validated or normalised when read from the manifest — read back exactly as written (manifest.rs, scope.rs)
- This crate spawns no subprocess and opens no socket; Agent is a trait implemented by the binary (agent.rs, lib.rs)
- The manifest schema version is fixed at SCHEMA_VERSION; a mismatched version is refused outright, never guessed at (manifest.rs)

## Where to look

- how a model pass is invoked and its transport-independent errors → `agent.rs` `Agent`
- why a WARLOCK.md is checked instead of trusted → `document.rs` `accept`
- how staleness is decided from a hash and manifest entry → `decide.rs` `decide_state`
- the blake3 subtree digest and what goes into it → `hash.rs` `subtree_hash`
- how a directory's files are packed into a request under byte caps → `fitting.rs` `fit`
- eliding test bodies from source files by language → `languages.rs` `elide`
- the pact manifest file format and atomic save/load → `manifest.rs` `Manifest`
- running a pact or refresh over a whole subtree → `pact.rs` `pact_subtree`
- scope and sigil validation rules and boundary matching → `scope.rs` `scope_opens_to`
- where machine-local sigils are stored per checkout → `sigils.rs` `sigils_path`
- the project tree's node type and depth-first walk → `tree.rs` `DepthFirst`
- computing an RFC 3339 timestamp without a date/time dependency → `clock.rs` `now_rfc3339`
