<!-- warlock -->
> Written by a model pass over this directory alone, to be read before its source and to say which source to read. A map, not a specification: check anything you are about to rely on against the files themselves, and where this document and the code disagree, the code is right.

# src

The core engine crate for warlock: the domain vocabulary for pacting directories, hashing subtrees, deciding freshness, and laying out the WARLOCK.md documents a model pass fills in, with no dependency on any terminal or TUI.

## Files

- `agent.rs` (46.9 KB) — The Agent trait, Request/Response and File/ChildDocument types: the port through which the engine reaches a model without spawning a process itself. · declares `Request`, `new`, `with_files`, `with_child_documents`, `with_prompt`, `prompt`, `directory`, `files` (+23)
- `briefs.rs` (21.2 KB) — briefs_path and load_briefs read .warlock/briefs.toml, answering DEFAULT_BRIEF_DIRECTORY where a repository states no preference. · declares `briefs_path`, `load_briefs`, `Error`, `check_relative`, `Briefs`, `default_directory`, `fmt`, `source`
- `claude_md.rs` (44.5 KB) — write_claude_md splices warlock's orientation block, bounded by BEGIN/END markers, into a repository's CLAUDE.md via Written::Created/Updated. · declares `write_claude_md`, `Written`, `path`, `Error`, `section`, `splice`, `fmt`, `source`
- `clock.rs` (15.3 KB) — now_rfc3339 renders the wall clock as an RFC 3339 UTC timestamp using hand-written calendar arithmetic (civil_from_days), no date/time dependency. · declares `now_rfc3339`, `rfc3339_from_unix_seconds`, `civil_from_days`
- `decide.rs` (10.9 KB) — decide_state maps a PactEntry and a computed hash to a NodeState: absent is Unpacted, mismatched or unjudged is PactedStale, matched is PactedFresh. · declares `decide_state`
- `document.rs` (83.4 KB) — Fill, FileFill, Lookup and Described define the WARLOCK.md schema; accept/check/render enforce shape and lay out the document from a model's answer. · declares `Fill`, `Lookup`, `FileFill`, `FileLookup`, `to_json`, `stub`, `Described`, `retain_accounts_in` (+37)
- `fitting.rs` (187.5 KB) — not read by the pass, over the size cap; name and size only
- `hash.rs` (31.1 KB) — subtree_hash computes the blake3 digest of everything at and below a directory, honouring ignore rules and .warlockignore, used as the staleness trigger. · declares `subtree_hash`, `Error`, `files_under`, `length`, `fmt`, `source`
- `ignores.rs` (5.1 KB) — is_ignored and FILENAME (".warlockignore") answer whether a directory itself is excluded by the repository's own ignore file. · declares `is_ignored`, `walk_one_deep`
- `languages.rs` (32.7 KB) — The per-language table (Language, Block, TABLE) behind elision: declared_names and elide drop test bodies while keeping declaration lines verbatim. · declares `Block`, `Language`, `declared_names`, `Elided`, `elide`, `real`, `work`, `between` (+32)
- `lib.rs` (15.7 KB) — The crate root: declares all modules and re-exports the public API (Agent, Manifest, Tree, pact_subtree, scope_covering, sigils functions, etc.).
- `load.rs` (72.5 KB) — load_tree and repository_root build a Tree from disk, colouring nodes via the manifest and hashing pacted subtrees, reporting Problem/ProblemCause. · declares `load_tree`, `Loaded`, `Problem`, `ProblemCause`, `repository_root`, `Error`, `fmt`, `source` (+10)
- `manifest.rs` (61.8 KB) — Manifest and PactEntry: the in-memory shape of .warlock/pacts.toml, with to/from_manifest_path, save/load, and SCHEMA_VERSION. · declares `Manifest`, `new`, `with_entries`, `entries`, `push`, `entry`, `to_toml_string`, `from_toml_str` (+27)
- `pact.rs` (309.0 KB) — Pacting: pact_directory writes one directory's WARLOCK.md; pact_subtree/refresh_subtree/unpact_subtree operate whole subtrees via describe_and_grant, rewrite; view_file, closed_scopes_at_or_below, Observer, Pacted, Failure, Refusal, Error, Unviewable. · declares `pact_subtree`, `refresh_subtree`, `unpact_subtree`, `closed_scopes_at_or_below`, `pact_directory`, `pactable_directories`, `view_file`, `Pacting` (+28)
- `scope.rs` (36.1 KB) — validate_scope, validate_sigil, scope_covering and scope_opens_to define what a scope string may be and whether held sigils open it. · declares `Rule`, `validate_scope`, `validate_sigil`, `scope_covering`, `valid_scope`, `scope_opens_to`, `fmt`, `valid_scope_on` (+3)
- `sigils.rs` (32.1 KB) — project_directory, sigils_path, load_sigils and save_sigils manage the machine-local <home>/.warlock/<project>/config.toml holding held sigils. · declares `project_directory`, `sigils_path`, `load_sigils`, `save_sigils`, `Error`, `project_dir`, `readable_name`, `Config` (+2)
- `state.rs` (3.8 KB) — NodeState: the three-state vocabulary (Unpacted, PactedStale, PactedFresh) with no fourth 'unknown' state. · declares `NodeState`
- `tree.rs` (34.8 KB) — Node, Tree, DepthFirst and StateCounts: the pure shape of the project tree, independent of how it was built. · declares `Node`, `new`, `with_children`, `with_files`, `with_ignored`, `is_ignored`, `with_scope`, `is_leaf` (+10)

## Structure

- load.rs calls decide.rs's decide_state and hash.rs's subtree_hash to colour each Node built from tree.rs's types
- pact.rs (not detailed here) drives document.rs's Fill/accept/render together with agent.rs's Agent trait to produce and write WARLOCK.md files
- manifest.rs's temp_file_name and write_and_sync are reused by claude_md.rs and sigils.rs for atomic writes
- document.rs's fitting decisions rely on languages.rs's elide and declared_names to shrink oversized files before a pass
- scope.rs reads PactEntry/Manifest from manifest.rs to compute coverage, and load.rs consults it per node
- lib.rs re-exports the public surface of every other module in this directory

## Rules

- agent.rs: this crate spawns no subprocess; Agent::run is implemented only by the binary
- hash.rs: a file that cannot be read is a fatal Error, never silently skipped
- manifest.rs: an unsupported schema version is rejected outright rather than guessed at
- manifest.rs: paths are stored relative to the manifest directory with forward slashes
- scope.rs: a scope is 1 to 24 chars, ASCII lowercase/digits/-/_, starts with a letter, no trailing separator
- document.rs: ENTRY_CHARS caps a slot at 280 characters, ENTRY_MINIMUM floors it at 20
- document.rs: ATTEMPTS limits a directory to two model passes before its answer is given up on
- sigils.rs: a missing sigil file is Error::NotFound, never an invented empty set
- load.rs: an unpacted node is never hashed

## Where to look

- how a model pass is actually invoked → `agent.rs` `Agent`
- the shape and validation of a WARLOCK.md → `document.rs` `Fill`
- what makes a directory stale or fresh → `decide.rs` `decide_state`
- computing the subtree hash → `hash.rs` `subtree_hash`
- reading or writing .warlock/pacts.toml → `manifest.rs` `Manifest`
- walking a directory into a coloured tree → `load.rs` `load_tree`
- scopes and sigils and who can act where → `scope.rs` `scope_opens_to`
- where a person's sigils are stored → `sigils.rs` `sigils_path`
- cutting test bodies out of a big source file → `languages.rs` `elide`
- the CLAUDE.md orientation block → `claude_md.rs` `write_claude_md`
- where briefs get written → `briefs.rs` `load_briefs`
- the tree's node and walk types → `tree.rs` `DepthFirst`
- how does warlock decide which directories get re-described on an edit instead of repacting everything → `pact.rs` `refresh_subtree`
- why did un-pacting a directory not delete its WARLOCK.md → `pact.rs` `unpact_subtree`
- what stops a machine outside a scope from un-pacting across it → `pact.rs` `closed_scopes_at_or_below`
- how does a long pact report progress or get cancelled from a UI → `pact.rs` `Observer`
