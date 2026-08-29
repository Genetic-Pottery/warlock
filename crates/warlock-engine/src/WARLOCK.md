# src

`warlock-engine`'s whole domain crate, flat — no subdirectories of its own. It has no dependency on the TUI or on any terminal library (the dependency edge runs TUI → engine, never back), and it spawns no subprocess and opens no socket. What it does do is read and write `.warlock/pacts.toml`, walk directories, hash files, and shape requests to send to a model.

Read `lib.rs` first: it is the module list and re-export surface, and its doc comments are the authoritative one-line description of every public item.

## How the pieces fit

The design this crate implements: a hash is the mechanical trigger, a model pass is the judgement, and a human can always read and hand-edit the result. Scopes and sigils are a second, independent vocabulary layered on top — a scope is a repo-committed fact about a directory, a sigil a machine-local fact about a person — and nothing in this crate yet matches one against the other.

- **`state.rs`** — the three-value vocabulary (`Unpacted`/`PactedStale`/`PactedFresh`). No fourth "unknown" state exists; unjudged *is* stale.
- **`tree.rs`** — pure shape: `Tree`, `Node`, depth-first walking, state tallying. A node carries a stored `ignored` flag, a stored own-only `scope`, and a `files` listing that is never hashed and never counts as a child.
- **`manifest.rs`** — `.warlock/pacts.toml`: one `PactEntry` per pacted module (directory, document path, optional scope, optional granted hash + timestamp). Paths stored root-relative, forward-slash. A stored scope is preserved exactly as written, never validated here. Owns the atomic write idiom (temp file + sync + rename) used everywhere else.
- **`hash.rs`** — `subtree_hash`: one digest over every file at and below a directory, used as the staleness trigger. An unreadable file is a hard error, never silently skipped. A repository-excluded directory hashes as empty.
- **`ignores.rs`** — `.warlockignore`: gitignore syntax via the `ignore` crate at every level, letting a repository exclude content from both the walk and the hash without touching git.
- **`decide.rs`** — `decide_state`: the pure function combining a `PactEntry` (or its absence) with a computed hash into one of the three states.
- **`load.rs`** — `load_tree`: builds a real `Tree`, rooted wherever invoked, finding the manifest by walking upward to the nearest `.git/`. Missing manifest means "never pacted." Hash failures and invalid scopes during load are non-fatal, recorded as `Problem`s.
- **`agent.rs`** — the seam to a model: the `Agent` trait, `Request`/`Response`, and the file vocabulary (present/omitted/summarised). Implements nothing — the binary runs `claude` as a subprocess — which is what lets every test here run with a hand-written fake.
- **`pact.rs`** (very large) — the actual work of pacting: gathering a request, demoting oversized files through a sent → summarised → listed ladder under two byte caps, running one agent pass, writing `WARLOCK.md` atomically, and — for a subtree — writing children before parents, then hashing and granting. A disk-backed, content-keyed summary cache under `.warlock/summaries/` avoids re-summarising unchanged files. Also handles refreshing (redescribe only stale directories) and un-pacting (drop entries, losing scopes, without touching files on disk).
- **`scope.rs`** — the sole judge of whether a string is a valid scope (`validate_scope`) or sigil (`validate_sigil`, plus wildcard `*`). Judges only, never normalizes. Also `scope_covering`: nearest valid scope at or above a path, segment-wise.
- **`sigils.rs`** — machine-local sigils read from/written to `<home>/.warlock/<project>/config.toml`, never inside the repository. `home` is always a caller-supplied parameter, never read from `HOME`. `project_directory` derives a per-checkout directory name from a hashed canonical path.
- **`claude_md.rs`** — writes `<root>/CLAUDE.md` (deliberately renamed from `AGENTS.md` because it's what Claude Code loads automatically): a delimited splice-in-place block covering warlock's basics and scopes/sigils. Everything outside the markers is preserved byte for byte. Writes exactly this one file — no manifest, no document, no model pass.
- **`clock.rs`** — `now_rfc3339`: hand-rolled, dependency-free RFC 3339 UTC timestamp generator, used only for `granted_at`.

## Before changing anything here

- **The state model is closed by design** — don't add a fourth state.
- **Freshness is granted in exactly one place**: inside `pact_subtree`/`refresh_subtree`, after every document in a subtree has been written.
- **Hashing must never skip an unreadable file** — preserve "error, not silent omission."
- **The `Agent` seam must stay process-free** — no real `claude` binary, network, or terminal in tests.
- **Manifest paths are always root-relative, forward-slash** — go through `to_manifest_path`/`from_manifest_path`.
- **A file too large to send is described, never truncated** — the whole/summarised/name-and-size ladder is the only vocabulary for missing content.
- **`.warlockignore` is real content, not metadata** — it's read like any other ignore file but is also hashed as an ordinary file itself.
- **`CLAUDE.md` is owned in part, not in whole** — `claude_md.rs` may only touch text between its own markers.
- **A scope is judged, never normalized, at the point it is stored** — case-folding belongs to whatever UI takes the string from a person.
- **`scope_covering` returns at most one scope, nearest wins** — don't build accumulating logic on top of it.
- **Sigils never live inside a repository, and scopes never live outside one** — don't let `sigils.rs` and `manifest.rs`/`scope.rs` cross into each other's territory.
- **Nothing in this crate enforces a scope boundary** — no code here checks a sigil against a scope and blocks or refuses anything; `claude_md.rs`'s body says so explicitly and a test pins that promise.