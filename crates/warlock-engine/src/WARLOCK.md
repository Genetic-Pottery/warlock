# src

This is `warlock-engine`'s source directory: the whole domain crate, with no subdirectories of its own (there is no child `WARLOCK.md` to summarise — everything the engine does lives in these nine files). It has no dependency on the TUI or on any terminal library; the dependency edge runs the other way, TUI → engine, and nothing here opens a socket or spawns a process. What it does do is read and write `.warlock/pacts.toml`, walk directories, hash files, and shape out a request to send to a model.

Read `lib.rs` first. It is the module list and the re-export surface, and its doc comments are the authoritative one-line description of every public item — treat it as the index, not as boilerplate to skip past.

## How the pieces fit

The design this crate implements is: a hash is the mechanical trigger, a model pass is the judgement, and a human can always read and hand-edit the result.

- **`state.rs`** — the three-value vocabulary (`Unpacted` / `PactedStale` / `PactedFresh`). No fourth "unknown" state exists anywhere in the crate; unjudged *is* stale.
- **`tree.rs`** — pure shape: `Tree`, `Node`, depth-first walking, state tallying. It computes nothing about staleness; it just holds what someone else decided.
- **`manifest.rs`** — the on-disk record, `.warlock/pacts.toml`: one `PactEntry` per pacted module, holding a directory, its document path, and an optional granted hash + timestamp. Paths are stored root-relative with forward slashes so two clones at different absolute paths (or on different OSes) produce byte-identical files. An absent grant is represented by an absent key, never a sentinel.
- **`hash.rs`** — `subtree_hash`: one digest over every file at and below a directory (sorted relative paths, `.gitignore`/`.warlock/`/symlinks excluded the same way everywhere), used as the trigger for staleness. An unreadable file is a hard error here, never a skip, because skipping would hash exactly like a deletion and could manufacture a false-fresh result.
- **`decide.rs`** — `decide_state`: the small pure function that combines a `PactEntry` (or its absence) with a freshly computed hash into one of the three states. This is the only place the four-row truth table (absent → unpacted; present-but-unjudged or judged-against-different-content → stale; judged-and-matching → fresh) is spelled out.
- **`load.rs`** — `load_tree`: builds a real `Tree` from a real directory, rooted wherever it's invoked (not necessarily the repo root), while finding the manifest by walking upward to the nearest `.git/`. Every directory reached by the walk becomes a node, documented or not; a missing manifest means "never pacted," not an error.
- **`agent.rs`** — the seam to a model. Defines the `Agent` trait, `Request`/`Response`, and the file vocabulary (`present` / `omitted` / `summarised`) but implements nothing: running `claude` as a subprocess is the binary's job, not this crate's. This split is what lets every test here run with a hand-written fake and no network, no terminal, no `claude` binary.
- **`pact.rs`** (very large; summarised, not fully read) — the actual work of pacting: gathering a directory's request, demoting oversized files through a sent → summarised → listed-only ladder under two byte caps, running one agent pass, writing `WARLOCK.md` atomically, and — for a whole subtree — writing children before parents and only then hashing and granting. It also handles un-pacting (dropping manifest entries without touching files on disk). Because this file was too large to read in full, its behavior here is described from a prior summary, not from its own text — don't quote from that summary as if it were the file's actual prose or code.
- **`clock.rs`** — `now_rfc3339`: a hand-rolled, dependency-free RFC 3339 UTC timestamp generator, used only for the `granted_at` field. It exists to avoid pulling in a date/time crate for something nothing else in the workspace parses back.

## Before changing anything here

- **The state model is closed by design.** Don't add a fourth state or an "unknown" variant — unjudged-and-stale is the point, not a gap to fill.
- **Freshness is granted in exactly one place**: inside `pact_subtree`, after every document in a subtree has been written. Nothing else writes a `granted_hash`, and there is currently no refresh pass — a stale directory stays stale until repacted.
- **Hashing must never skip an unreadable file.** If you touch `hash.rs`, preserve the "error, not silent omission" behavior; it's what prevents a false-green.
- **The `Agent` seam must stay process-free.** `agent.rs` and everything upstream of it (tests included) must never need a real `claude` binary, network, or terminal. If you're tempted to add such a dependency to a test, use the fake pattern already established instead.
- **Manifest paths are always root-relative, forward-slash, portable.** Any path handling added elsewhere in this crate should go through `to_manifest_path`/`from_manifest_path` rather than reinventing normalization.
- **A file too large to send is described, never truncated.** The three-rung ladder (whole → summarised → name-and-size) in `pact.rs` is the only vocabulary for "this content didn't make it into the request." Preserve that distinction if you touch it — a summary is prose *about* a file, never a substitute for its bytes, and `File::bytes()` must keep answering `None` for it.
- **`pact.rs` was read via a prior summary here, not in full.** Don't assume implementation details beyond what's stated above without opening the file yourself.