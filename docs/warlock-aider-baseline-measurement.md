# The aider baseline: is a WARLOCK.md more useful to an agent than a free map?

Measured 2026-09-13 against warlock's own two source directories.

The bar, from the literature notes: aider's repomap gets routing value from
tree-sitter and PageRank with no model in the loop, ranked and truncated to a
token budget. A written line costs a model pass; a repomap costs nothing. The
question is whether the written line beats free by enough to matter.

## Method

36 questions, built from the **source**, never from a document: a document is
one of the things on trial, so a question phrased out of its own
`## Where to look` line would be scoring the arms on a test one of them wrote.
Each question was generated from a slice of Rust around one declaration that is
unique within its directory, phrased to describe behaviour rather than name, and
dropped if the generated text contained the symbol or the filename. 37 of 73
generated questions were dropped by that filter.

Three arms, same model (`claude-sonnet-5`, effort low), same question, one call
each:

- **document** — the directory's committed `WARLOCK.md`
- **repomap** — aider 0.87's `RepoMap` over the same directory, `map_tokens` set
  to the document's own token count (2026 vs 1852, and 2904 vs 2917)
- **listing** — file names and sizes, nothing else: the floor

Scored on whether the answer names the right file, and separately the right
file *and* symbol.

## Result

| arm | right file | right file + symbol |
|---|---|---|
| document | 33/36 — **91.7%** | 12/36 — 33.3% |
| repomap | 14/36 — 38.9% | 7/36 — 19.4% |
| listing | 21/36 — 58.3% | 3/36 — 8.3% |

On the 15 questions the bare listing cannot answer — where routing actually
matters rather than the filename giving it away — the document gets 13/15
(86.7%) against the repomap's 8/15 (53.3%).

The repomap never won a question the document lost. The reverse happened 19
times.

## Why, and it is not that the prose is smarter

The repomap's ranking is excellent. **When the gold file appears in the repomap
at all, it answers 14 of 14 — 100%.** Its problem is coverage at the budget:

| directory | files | named in document | named in repomap |
|---|---|---|---|
| `crates/warlock-engine/src` | 18 | 18 | 9 |
| `crates/warlock-tui/src` | 36 | 36 | 10 |

The gold file was absent from the repomap in 21 of 36 questions, so those were
unanswerable before the model read a word. Spending the same tokens on
signatures for the top-ranked half buys precision on what it covers and nothing
at all on the rest.

That is also why the free listing beat the repomap. For routing, enumerating
every file with one line each beats ranking a subset in depth.

## What this settles, and what it does not

It settles the bar: the document beats free by 33 points over a listing and 53
over the repomap, for one pass of about 45 seconds per directory at Sonnet/low.
The per-file direction is not threatened by the free baseline.

It does not settle the assumption underneath per-file granularity — that a
routing line written without its siblings routes as well as one written with
them. Every document here was written the current way. That needs its own
measurement, and this harness will run it unchanged once such documents exist.

Two limits worth stating. Symbol accuracy is low for every arm (33% at best)
because the gold name is often internal and `DECLARED_SHOWN` caps a file line at
eight names — the file is the unit that routes, not the symbol. And the repomap
was run over one directory with no repo-wide graph and no chat context, which is
a fair reading of "a deterministic map of this directory at this budget" but is
not how aider uses it in a session.

## Running it again

The harness is in the session scratchpad, not in this repository: `harness.py`
(questions), `repomap.py` (the baseline arm), `run_eval.py`, `score.py`, and
`pyrun.sh`, which exists only because scipy's wheel needs a libstdc++ that a
NixOS venv has no path to. Aider needs Python 3.12; 3.14 cannot build it.
