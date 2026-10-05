# PageIndex against warlock: does a tree of summaries route an agent to code?

Measured 2026-09-29 and 2026-09-30 against warlock's own `crates/` tree.

[PageIndex](https://github.com/VectifyAI/PageIndex) is "vectorless,
reasoning-based RAG": it builds a tree over a document, summarises every node
with a model, and lets an agent reason its way down the tree instead of
searching embeddings. That is warlock's navigation premise. PageIndex is built
for long PDFs — filings, manuals, textbooks — and takes its tree from document
layout. The question is how the same idea does on a codebase, against a
`WARLOCK.md` tree and against grep with no index at all.

## Method

49 questions, built from **git history**, not from a document or a person.
Each question is the subject line of a real warlock commit after `6f546b2`
(2026-09-20); its answer is the set of non-test `.rs` files under `crates/`
that the commit modified and that already existed at `6f546b2`. Commits that
added a source file, or touched none or more than four, were dropped: 49 of
the 99 commits in range survived, averaging 1.7 gold files each.

Every arm works on the same snapshot: `crates/`, `Cargo.toml` and `Cargo.lock`
at `6f546b2`, with a committed aider cache removed. The agent is `claude -p` on
`claude-sonnet-5` with user settings, project instructions and MCP servers
off, capped at $2 a question. It is asked for at most five files, most likely
first. Four arms:

- **baseline** — `Read`, `Grep` and `Glob` over the snapshot, no index.
- **warlock** — `Read` and `Glob` over the snapshot with its six committed
  `WARLOCK.md` files (31 KB in all), told to start at the root one. No grep, so
  the score is the documents' and not grep's.
- **warlock + grep** — the same, with `Grep`. The realistic setup.
- **pageindex** — only two tools mirroring PageIndex's own
  `agent_tools.py`: a paginated structure without node text, and a node reader,
  both under its 95,000-character response budget. No file access.

The PageIndex index was built with its markdown mode (`md_to_tree`, commit
`619cbd8`) over the snapshot flattened into one document: a heading per
directory and per file, each file's contents fenced so its own lines are not
read as headings. Its two model-call functions were replaced with `claude -p`
on warlock's document model, Sonnet 5 at low effort, so both tools summarise
with the same model. 126 summary calls cost $6.66 and produced 245 KB of
summaries across 133 nodes.

Scored on recall@5 (the share of gold files in the answer), on whether every
gold file was found, on whether the first file was right, and on cost, turns
and wall time.

## Result

| arm | recall@5 | every file found | top file right | at least one found | $/question | turns | seconds |
|---|---|---|---|---|---|---|---|
| **warlock** | **0.68** | **53%** | **55%** | **80%** | $0.53 | **20.6** | 108 |
| warlock + grep | 0.64 | 47% | 51% | 80% | $0.45 | 24.0 | 123 |
| baseline | 0.61 | 47% | 49% | 73% | $0.50 | 24.4 | **89** |
| pageindex | 0.48 | 37% | 43% | 61% | **$0.43** | 31.6 | 120 |

Paired against the baseline on the same 49 questions, with 95% bootstrap
intervals over the per-question differences:

| arm | recall@5 against baseline |
|---|---|
| warlock | +0.07 [−0.02, +0.17] |
| warlock + grep | +0.03 [−0.06, +0.13] |
| pageindex | −0.12 [−0.26, +0.01] |

Six questions were missed by every arm, which caps any arm at about 88%.

## What it says

**Warlock ranks first on every accuracy measure and in turns, and the lead is
not significant.** The interval crosses zero at 49 questions. A first run of
20 of these questions, three arms, put warlock 18 points ahead (0.75 against
0.57 and 0.58); the gap shrank to 7 as the sample grew, so part of it was
luck. Read it as consistently ahead, not proven ahead.

**PageIndex is the weakest arm, 12 points behind grep, and that is close to
significant.** It spent the most turns, most of them paging a 245 KB structure
against warlock's 31 KB. Its low cost per question came from giving up sooner,
not from working more efficiently.

The reason is structural, not a matter of prompt quality. A PageIndex tree over
code is a node per file, each carrying a model's summary of that file: a flat
index, not a map. The summaries restate what grep can already find, at seven
times the size of a document that says what each directory is *for*. PageIndex
recovers structure that a PDF only implies; a repository's structure is already
explicit in its paths and declarations, so there is little left for the tree to
add.

**Grep does not improve warlock.** Given both, the agent does about as well as
with grep alone, and cheaper per question. One reading is that grep lets it take
keyword matches and skip the documents. The transcripts were not read to confirm
that.

## Limits

- **The questions leak.** Commit subjects often name what changed — 10 of the 49
  contain a gold file's stem. That favours grep, and unlike the aider
  baseline's source-built questions, nothing filtered it out.
- **The PageIndex arm is an adapter.** PageIndex was built for PDFs; this runs
  its markdown mode over a flattened repository, through tools written to mirror
  its own. A poor score measures the adapter as well as the tool.
- **The warlock documents were whatever was committed at `6f546b2`**, stale or
  not. That is the realistic condition, and it is not the best case.
- **One run per question per arm.** The aider baseline put its noise floor at
  about four answers in 108 from rerunning identical content; no repeats were
  run here, so per-question variance is unmeasured.
- **Two of the questions are commits written in the same session as the run,**
  by the same model family that answered them.
- The run was interrupted once by a usage limit. The 100 runs that came back as
  API errors were discarded and rerun; no errored row is in the result.

## What this settles, and what it does not

It settles that PageIndex's approach does not transfer to code for routing:
per-file summaries cost more to read and route worse than grep with no index.
Warlock's directory documents do not share the problem.

It does not settle that warlock beats grep. It points that way on every measure
and is inside the noise. Showing a significant lead needs more questions, and
questions from repositories other than warlock's own, since every question here
is about the code the documents were tuned on.

## Running it again

The harness lives outside this repository:
`sample.py` (questions from git history), `pi_build.py` (the PageIndex index,
with its model calls routed through `claude -p`), `pi_tool.py` (the PageIndex
arm's tools), `run.py` (the arms, resumable, skipping pairs that already have a
clean result), and `score.py`. The PageIndex source is vendored as `pi/`,
because its own `types.py` shadows Python's standard `types` module when its
directory is on the path.

`questions_all.json` is the reusable part. `results49.jsonl` is the result
above; `results.jsonl` is the 20-question first run.

Total spend: $6.66 on the index, $33.14 on the first run, $92.89 on the
49-question run, and $2.55 on runs lost to the usage limit.
