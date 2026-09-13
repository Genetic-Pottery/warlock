# Does a line written without its siblings route as well as one written with them?

Measured 2026-09-13, immediately after
`docs/warlock-aider-baseline-measurement.md`, whose harness, question set and
noise floor this reuses.

This is the assumption under per-file granularity — one model pass per file
instead of one per directory, so that a directory's size stops being a variable
and one changed file costs one line. Everything else about that refactor was a
design question. This was the part that could have killed it outright: if a pass
that cannot see a file's siblings writes a worse line, there is nothing to
discuss.

## Method

The same 36 questions, built from source, scored the same way: does the answer
name the right file, and does it name the right symbol.

The isolated arm was produced by one model call per file — 54 of them — each
shown that file's skeleton and nothing else. The instruction is warlock's own
`"files"` wording, copied verbatim out of `document.rs`'s `PROMPT`, so the only
variable between the arms is whether the pass could see the rest of the
directory. The skeletons are warlock's own: dumped from `languages::skeleton`,
the same reduction `fitting.rs` sends. Sizes and `· declares` lists are
untouched in both arms, because those are warlock's measurements and not the
pass's.

## Result

Three runs of 36 questions per arm, 108 answers each.

| arm | right file | right file + symbol | tokens |
|---|---|---|---|
| current — one pass over the directory | 90.7% (33, 33, 32) | 44.4% (17, 16, 15) | 5747 |
| isolated — one pass per file | **93.5%** (34, 33, 34) | 46.3% (17, 16, 17) | 6745 |

Isolated is ahead by three answers in 108 on the file and two on the symbol,
which is inside the noise floor the baseline measurement established — identical
content scored 91.7% and 87.0% on two runs of the same arm. So the finding is
**no detectable penalty**, not a gain. What matters is the direction it rules
out: the line does not get worse when the pass cannot see the siblings.

One effect that is not noise: **the isolated document is 17% larger**. A pass
writing a single line, with no other file competing for the answer, has no
reason to be brief and was not. If per-file ships, `ENTRY_CHARS` does more work
than it does today, not less.

## The cross-file sections, measured by accident

The same run stripped `## Structure`, `## Rules` and `## Where to look` from
both arms:

| arm | right file |
|---|---|
| current | 88.9% |
| current, no synthesis | 88.9% |
| isolated | 94.4% |
| isolated, no synthesis | 91.7% |

One run each, so read it as an absence of a large effect rather than a
measurement. Removing three sections did not measurably hurt routing.

**This is not a case for deleting them.** The harness scores one thing: given a
question, is the right file named. `## Rules` was never a routing device. A line
like *a manifest declaring any version but `SCHEMA_VERSION` is refused rather
than read leniently* exists to stop a plausible, correct-looking edit — the same
test the comment rules in `CLAUDE.md` apply. Nothing here can see that value,
and its absence from this table is evidence about the harness, not about the
line.

What the table does say is that the *routing* argument for synthesis is weak, so
if synthesis has to earn its cost, it earns it as a guardrail.

## What this unblocks, and what is still open

Per-file granularity is no longer blocked on the assumption. Of the two design
questions:

**Where a per-file line is stored and keyed — settled.** The document is the
store: parse the committed `WARLOCK.md` back into its file lines, keep a
per-file hash beside each entry in `.warlock/pacts.toml`, and re-ask only for
the files whose hashes moved. No second store, the committed document stays the
artifact, and *fresh* keeps a precise meaning — every file's line matches that
file's hash, and the synthesis matches the assembled lines. The cost is that
warlock reads its own previous document, so the rule in `agent.rs` that a
request never carries it must be restated as: the lines are reused mechanically
and are never sent to a model as evidence.

**Whether synthesis is its own pass — open, and the hazard is validation.**
`check` validates a lookup's target through `route()`, but `structure` and
`rules` get shape checks only: length, count, emptiness. Nothing verifies that a
name in them exists. A synthesis pass reading assembled lines rather than code
would produce `## Where to look` that warlock can still check and two sections
that nothing can. The candidate fix is to validate every backticked token in
those sections against `Described.declared` — the evidence is already in the
pipeline, `Defect::UnknownTarget` and `UnverifiedSymbol` already exist, and
`mend` already drops a bad list entry. Only backticked tokens: a rule naming
nothing must stay legal.

That check would close a hole that is open today, with a pass that does see the
code.

## Limits

Two directories of one Rust repository, one model at one effort, questions
derived from declarations. The isolated arm was generated once; its lines are
fixed across the three runs, so the repeats measure the reader's variance and
not the writer's.
