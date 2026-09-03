# src

The whole of `warlock-engine`'s domain crate, flat — no subdirectories. It is the only side of the TUI ↔ engine boundary that ever changes: the dependency edge runs TUI → engine and never back. It reads and writes `.warlock/pacts.toml`, walks directories, hashes files, and shapes requests to send to a model — but it never depends on a terminal, spawns no subprocess, and opens no socket.

Read `lib.rs` first. It is the module list and re-export surface, and its doc comments are the authoritative one-line description of every public item.

## How the pieces fit

The design: a hash is the mechanical trigger, a model pass is the judgement, and a human can always read and hand-edit the result. Scopes and sigils are a second, independent vocabulary layered on top — a scope is a repo-committed fact about a directory, a sigil a machine-local fact about a person — and nothing in this crate matches one against the other; that comparison is left to callers.

- **`state.rs`** — the closed three-value vocabulary (`Unpacted`/`PactedStale`/`PactedFresh`). No fourth "unknown" state; unjudged *is* stale.
- **`tree.rs`** — pure shape: `Tree`, `Node`, depth-first walking, state tallying. A node carries a stored `ignored` flag, a stored own-only `scope`, and a `files` listing that is never hashed and never counts as a child.
- **`manifest.rs`** — `.warlock/pacts.toml`: one `PactEntry` per pacted module (directory, document path, optional scope, optional granted hash + timestamp). Paths stored root-relative, forward-slash. A stored scope is preserved exactly as written, never validated here. Owns the atomic write idiom (temp file + sync + rename) used everywhere else in the crate.
- **`briefs.rs`** — `.warlock/briefs.toml`: the one setting a repository can write, saying which directory freestanding briefs go in. Absent file or absent key is the default (`docs`); anything unreadable is a named error, never a silent fallback. Reads one file, writes nothing.
- **`hash.rs`** — `subtree_hash`: one digest over every file at and below a directory, used as the staleness trigger. An unreadable file is a hard error, never silently skipped. An excluded directory hashes as empty.
- **`ignores.rs`** — `.warlockignore`: gitignore syntax via the `ignore` crate, letting a repository exclude content from both the walk and the hash without touching git. Also holds `is_ignored`, the one check that catches a directory being selected directly rather than descended into (a walker applies no rule to its own root).
- **`decide.rs`** — `decide_state`: the pure function combining a `PactEntry` (or its absence) with a computed hash into one of the three states.
- **`load.rs`** — `load_tree`: builds a real `Tree`, rooted wherever invoked, finding the manifest by walking upward to the nearest `.git/`. Missing manifest means "never pacted." Hash failures and invalid scopes during load are non-fatal, recorded as `Problem`s. `.warlockignore` deliberately prunes nothing from this walk — it marks a `Node` as `ignored` rather than hiding it, so a reader can see what the repository excluded.
- **`agent.rs`** — the seam to a model: the `Agent` trait, `Request`/`Response`, and the file vocabulary (present/omitted/summarised). Implements nothing itself — the binary runs `claude` as a subprocess — which is what lets every test here run with a hand-written fake. `Request` also carries a directory's own previous `WARLOCK.md` in a slot separate from its files, so a claim from an earlier pass can never be mistaken for evidence.
- **`pact.rs`** (by far the largest file) — the actual work of pacting: gathering a request, demoting oversized files through a sent → summarised → listed ladder under two byte caps (`PER_FILE_BYTE_CAP`, `REQUEST_BYTE_CAP`), running model passes, writing `WARLOCK.md` atomically, and — for a subtree — writing children before parents, then hashing and granting. Over-cap files are summarised by a map-reduce over chunks through the same `Agent`, and a disk-backed, content-keyed summary cache under `.warlock/summaries/` (keyed on file bytes, never path/name/mtime) avoids re-summarising unchanged files, and is itself invisible to every walk and hash in the crate. Also handles refreshing (redescribe only stale directories), un-pacting (drop entries, losing scopes, without touching files on disk), and progress/cancellation through an `Observer` trait (asked only between directories, and also told about summarising passes and about the byte-weight of each directory's request as it is sent).
- **`scope.rs`** — the sole judge of whether a string is a valid scope (`validate_scope`) or sigil (`validate_sigil`, plus wildcard `*`). Judges only, never normalizes. Also `scope_covering` (nearest valid scope at or above a path, segment-wise, nearest wins) and `scope_opens_to` (membership test against held sigils; unscoped is always open, holding nothing opens nothing scoped).
- **`sigils.rs`** — machine-local sigils read from/written to `<home>/.warlock/<project>/config.toml`, never inside the repository. `home` is always a caller-supplied parameter, never read from `HOME`. `project_directory` derives a per-checkout directory name from a hashed canonical path, so two clones of one repo hold sigils separately.
- **`claude_md.rs`** — writes `<root>/CLAUDE.md`: a delimited splice-in-place block covering warlock's basics and scopes/sigils. Everything outside the markers is preserved byte for byte. Writes exactly this one file — no manifest, no document, no model pass. States explicitly that nothing in the workspace enforces a scope boundary, so this document never promises a wall warlock does not build.
- **`clock.rs`** — `now_rfc3339`: hand-rolled, dependency-free RFC 3339 UTC timestamp generator, used only for `granted_at`.

## Before changing anything here

- **The state model is closed by design** — don't add a fourth state.
- **Freshness is granted in exactly one place**: inside `pact_subtree`/`refresh_subtree`, after every document in a subtree has been written. Never per-directory, because a directory's hash covers its children's just-written documents.
- **Hashing must never skip an unreadable file** — preserve "error, not silent omission."
- **The `Agent` seam must stay process-free** — no real `claude` binary, network, or terminal in tests.
- **Manifest paths are always root-relative, forward-slash** — go through `to_manifest_path`/`from_manifest_path`.
- **A file too large to send is described (summarised) or listed by name and size — never truncated.**
- **`.warlockignore` is real content, not metadata** — it's read like any other ignore file but is also hashed as an ordinary file itself.
- **`CLAUDE.md` is owned in part, not in whole** — `claude_md.rs` may only touch text between its own markers.
- **A scope is judged, never normalized, at the point it is stored** — case-folding belongs to whatever UI takes the string from a person.
- **`scope_covering` returns at most one scope, nearest wins** — don't build accumulating logic on top of it.
- **Sigils never live inside a repository, and scopes never live outside one** — don't let `sigils.rs` and `manifest.rs`/`scope.rs` cross into each other's territory.
- **Nothing in this crate enforces a scope boundary** — no code here checks a sigil against a scope and blocks or refuses anything; `claude_md.rs`'s written text says so explicitly and a test pins that promise.
- **Neither byte cap (`PER_FILE_BYTE_CAP`, `REQUEST_BYTE_CAP`) may fail a pact** — going over either always degrades gracefully (summarise, then list) rather than erroring.
- **The summary cache is bookkeeping, not a source of truth** — a cache miss, corrupt entry, or failed write must never change what a pact produces, only what it costs.
- **A previous document is a claim, not evidence** — it lives in `Request::previous_document`, never among `Request::files`, and prompts are written to tell a pass to check it against the files rather than trust it.