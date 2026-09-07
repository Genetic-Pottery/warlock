# Delete the source prose and write back only the decisions

The Rust sources under `crates/warlock-engine/src` and `crates/warlock-tui/src` carry roughly 3,700 `//!` lines across 54 files, plus item docs of comparable weight — `edits.rs` alone has 157 header lines, `pact.rs` 209, `fitting.rs` 152. Most of it is descriptive: what a function does, in what order, what the ladder in `fit` steps through, which slots `document.rs` lays out. That prose was accurate when written and much of it no longer is, because the code moved and the paragraphs did not.

The cost lands in two places. `fitting.rs` sends each file's bytes into the pass, so every stale paragraph is evidence in front of the model that writes a `WARLOCK.md`; the document repeats the falsehood, and the judging step then flags a document that faithfully reported what the source told it. `document.rs` already drops a directory's own previous document for exactly this reason — "a pass shown it carried its sentences forward whether or not the files still supported them" — and the module headers are that same leak through a door nothing closes. The second cost is ordinary work: hours spent arguing with a model about what the code does, because the model believes the comment.

Rewriting the prose fresh would produce 3,700 new lines that begin rotting the same afternoon. The code says what it does, and is read by machines that derive that faster from the code than from a paragraph about it. What the code cannot say is why it is not something else: `HASH_CONTEXT` is `"warlock subtree hash v1 2026-08-19"` and a reader adding a new ignore source will bump it to v2 and restale every repository in existence; the `?` on the file read in `hash.rs` looks like a robustness gap and turning it into a skip hands back a false green. Those two paragraphs are not documentation, they are guardrails against a specific, plausible, wrong edit. Leaving this alone means the descriptive prose keeps costing arguments and keeps poisoning passes, while the handful of load-bearing decisions stay buried in it where nobody can find them.

## Outcome

Open `crates/warlock-engine/src/hash.rs`. The 49-line header is gone. Near the top of the file, `HASH_CONTEXT` carries a few lines saying when the version moves and when it does not, and the file read that returns an error carries a line saying why it is an error rather than a skip. Nothing else in the file is prose. Scroll `fitting.rs`, `pact.rs`, `edits.rs`: source, and long stretches with no comment at all.

`cargo test` is green and reports the same number of doctests it did before. `cargo doc` builds. `rg '^\s*//!' crates --stats` reports a few hundred lines where it reported 3,679.

In the warlock tree, `crates/warlock-engine/src` and `crates/warlock-tui/src` are yellow, along with everything above them — every byte under those directories moved, so the pacts are owed a pass.

## Success criteria

**The wipe**

- Every `//!` module header in `crates/warlock-engine/src` and `crates/warlock-tui/src` is deleted or replaced; none survives unexamined.
- Every `///` item doc in those directories is deleted or replaced; none survives unexamined.
- Each file is worked from its code, not by editing its existing prose.
- `rg '^\s*//!' crates` returns fewer than 500 lines.

**What survives**

- Every doctest present before the change is present after it, unchanged in what it asserts.
- `cargo test` passes, and the doctest count in its output matches the count taken before the change.
- `cargo doc` builds without warnings that the change introduced.

**What is written back**

- A comment is written back only where its absence would let a plausible, correct-looking edit break something, or where it names an alternative that was considered and rejected.
- `HASH_CONTEXT` carries the rule that the version moves when a repository that changed nothing would hash differently, and not otherwise.
- The unreadable-file path in `hash.rs` carries the reason it fails rather than skips.
- No comment written back restates what the code beneath it does.

**The repository still builds**

- `cargo build` succeeds for both crates.
- `cargo clippy` reports no new warnings attributable to the change.

## Constraints

- No behaviour changes. This work touches comments and doc comments only; any change to an expression, a signature, a type or a control path is out of bounds, including changes that look like obvious improvements found while reading.
- No dependency is added, removed or upgraded.
- Doctests are code and are preserved verbatim, including where the prose surrounding them is deleted.
- The `.warlock/pacts.toml` entries are not hand-edited to hide the staleness the wipe causes. The directories go yellow and stay yellow until a pass grants them.
- `fitting.rs` is not taught to strip comments from requests. The leak is real, but the fix here is that the comments stop being wrong, not that the pass stops seeing them.
- All of this sits under the `warlock-team` scope carried by the root pact, with nothing nearer overriding it. Confirm that sigil is held before starting; `r` on these directories will refuse without it.

## Out of scope

- **Tests.** The 1,714 tests in these crates were appended over time and their reasoning may no longer hold, but auditing them is a larger change with a different method — it requires deciding what each test is for, which the comment wipe does not. Deferred deliberately, not forgotten.
- **Re-judging the documents afterwards.** Pacting is known to work; the judging step reliably calls a documented falsehood a lie, which is the behaviour that surfaced this problem. Re-running it would measure something already measured.
- **`WARLOCK.md` files.** They are generated by a pass, and they will be regenerated from cleaner sources when the restaled directories are granted again. Editing them by hand restales the directory anyway.
- **Files outside `crates/warlock-engine/src` and `crates/warlock-tui/src`.** `docs/` is explicitly a record of the past and is not held to current truth; build scripts and the workspace manifest carry little prose and no leak.
- **A lint or CI rule capping comment volume.** Tempting as a way to keep this from returning, but it would police the thing that is cheap to measure rather than the thing that matters, and it would fight the decision comments this change is preserving.
- **Whether the surviving comments should live somewhere other than beside the code.** Left open. The rule above says which comments survive, not where they should ultimately sit.
