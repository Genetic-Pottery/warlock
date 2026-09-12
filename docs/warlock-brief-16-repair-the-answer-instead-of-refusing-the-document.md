# Repair the answer instead of refusing the document

On 2026-09-12 a refresh of this repository refused `crates/warlock-tui/src`. The directory is 36 files and 365 KB; the pass came back with an 8.4 KB answer describing 35 of them and no entry for `writing.rs`. Warlock rejected the whole answer, asked again for the missing slot, got it back at 300 characters against a cap of 280, and rejected the whole answer a second time. Two passes, `$0.00` of usable output, and a directory left stale. The document that was thrown away was correct in 35 of 36 lines.

The cost does not stop at that directory. A refusal takes the grant off everything above it: `crates/warlock-tui`, `crates` and the repository root were described, recorded without grants, and had to be described again on the next run — `$0.23` paid twice for documents that came back byte-identical. Brief 15's wipe left those directories owed a pass anyway, so the second run was owed; the point is that the first run's work above the failure was never going to survive, and warlock spent on it knowing the directory below had failed.

What makes this self-inflicted is that warlock is the only tool in this space that validates at all. CodeWiki, Agent4cs and RepoAgent all generate bottom-up the way warlock does and none of them checks its model's output — which is why CodeWiki tops out at 68.79% with inconsistency and drift as its named failure modes. The gate is warlock's advantage and the reason a `WARLOCK.md` cannot name a file or a symbol that is not there. Aider, which has no model in the loop at all, handles a budget it cannot meet by ranking and dropping rather than failing. The gate should behave the same way: it is a shape the document has to hold, not a test the model has to pass.

Every defect warlock currently refuses on has a repair warlock can apply from evidence it already holds. It knows the filename and its size, because it renders those itself. It knows which symbols a file declares, because `languages.rs` extracted them. It has the model's own text for every slot that was filled. An entry twenty characters over the cap is a truncation, not a failure. Leaving this alone means every run of warlock over a large directory is a coin toss, and the product's visible behaviour on a bad toss is three directories going yellow for a reason the reader cannot see from the tree.

## Outcome

Run `warlock refresh .` over a directory large enough that a pass gets a slot wrong. It does not refuse. The document is written, every cap is held, and the run names what it repaired: `crates/warlock-tui/src — files["writing.rs"] was 300 characters and was cut to 280`. The directory goes green, and so does everything above it. `.warlock/pacts.toml` carries a grant for all of them.

Open the document. `writing.rs` has a line. Where the pass supplied prose the line reads as it always did; where it did not, the line is the file's name, its size and the symbols it declares, and nothing invented.

A refusal still exists, and it now takes a model that cannot emit a JSON object four times running. `warlock pact` over the fixture at `../warlock-test-repo` reports 39 of 39 checks passing, as it does today.

## Success criteria

**Every defect with an answer behind it is repaired**

- `Defect::TooLong` cuts the value to the cap rather than rejecting the answer.
- `Defect::Multiline` keeps the first line.
- `Defect::TooMany` keeps the first `LIST_CAP` entries.
- `Defect::Missing`, `Defect::Empty` and `Defect::TooShort` fall to a line built from what warlock already holds: the entry's name, its size where it has one, and its declared symbols.
- `Defect::UnknownTarget`, `Defect::UnverifiedSymbol` and `Defect::ToolNamed` drop the offending entry rather than the document.
- After the repairs, the `Fill` satisfies every check `accept` makes of it — a repaired answer cannot itself be defective.

**One refusal survives, and only one**

- `Defect::NotJson` on every attempt is the only path to `Refusal::Malformed`.
- A refusal still writes no document and still leaves the previous one on disk.
- No repair invents a claim. A repair may use only the model's own text, the filename, the file's size, and the symbols `languages.rs` extracted — never a sentence about what a file is for.

**A repaired document is a granted document**

- A directory whose answer was repaired is granted, hashed and recorded like any other: the pass read the directory and described it, and the document says what it knows.
- The stamp at the top of the document is unchanged. A repaired document makes the same claim every document makes and no weaker one.

**Repairs are visible**

- Every repair is reported to the caller the way `fitting::Problem` reports an omission, naming the directory, the slot and what was done to it.
- The panel shows them in the directory's section, and a headless run prints them.
- A run with repairs is not a run with failures: the exit status is the clean one, and the report at the end does not count it.

**Nothing regressed**

- `./check.sh` against `../warlock-test-repo` reports 39 of 39.
- `cargo test`, `cargo clippy` and `cargo fmt --check` are clean across the workspace.
- A directory whose pass gets everything right produces the same document it does today.

## Constraints

- The caps do not move. `ENTRY_CHARS`, `ENTRY_MINIMUM`, `PURPOSE_CHARS` and `LIST_CAP` keep their current values; this brief changes what happens when an answer misses them, not what the document is allowed to be.
- No repair reaches the model. A repair is mechanical or it is not a repair — asking again is what the attempt loop already does, and it happens before any repair is applied.
- The attempt loop stays. A repair is the floor under it, not a replacement for it: warlock still asks once more for real prose before settling for a mechanical line.
- No change to `fitting.rs`. The request side is a separate problem with a separate brief behind it.
- All of this sits under the `warlock-team` scope carried by the root pact, with nothing nearer overriding it. Confirm that sigil is held before starting; `r` on these directories will refuse without it.

## Out of scope

- **Removing the skip.** `AboveFailure::Skip` stops firing for the ordinary case once this lands, but `NotJson` keeps the refusal path alive and the skip is what makes it cheap. It stays. Deleting it belongs to whatever change removes the last refusal, if one ever does.
- **`ATTEMPTS`.** It was raised from 2 to 4 on 2026-09-12 to buy margin against exactly the oscillation this brief removes, and with a repair underneath it the extra attempts no longer prevent anything. Lowering it is a judgement about prose quality per dollar, measurable on its own, and not part of this change.
- **Per-file granularity.** Describing one file per pass would make most of this unnecessary, and it is the direction the request-side problem points in. It is blocked on two decisions nobody has made — where a per-file line is stored, and whether the directory synthesis is its own pass — and asserting them here would be guessing.
- **Measuring against aider's baseline.** Whether a model-written routing line beats tree-sitter and PageRank with no model at all is the question underneath the whole product, and it is a measurement rather than a change.
- **Judging.** The judging step already calls a documented falsehood a lie. A repaired line says less than a written one; whether it says anything false is the judge's question and is unaffected by this.
- **Whether a mechanical line is worth having at all.** Left open. A file line with no prose may be worse than no line, in which case the repair for `Missing` should be to drop the file from the table instead. The criteria above take the position that a name, a size and a symbol list route better than silence; if the implementation finds otherwise, say so rather than working around it.
