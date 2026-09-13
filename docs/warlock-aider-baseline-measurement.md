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

Two limits worth stating. Symbol accuracy is low for every arm — 33% at best,
against 92% for naming the file — because the gold name is often internal and a
file line carries only its first `DECLARED_SHOWN` names. The follow-up below is
what came of pulling on that. And the repomap was run over one directory with no
repo-wide graph and no chat context, which is a fair reading of "a deterministic
map of this directory at this budget" but is not how aider uses it in a session:
its PageRank is built to rank a whole repository, personalised by the files and
identifiers already in the conversation. Stripped of both, it is doing its
weakest job. This measures enumeration against ranking for directory-scoped
routing; it is not a verdict on aider.

## Follow-up: what the baseline suggested about `DECLARED_SHOWN`

The repomap's one clear win was precision on what it covered — 14 of 14 when the
gold file was in the map, against signatures rather than prose. The cheapest way
to buy some of that is the `· declares` list, which is rendered from what warlock
measured and costs no model pass to lengthen. So the same 36 questions were run
against documents identical but for that cap.

| declares | tokens | right file | right file + symbol |
|---|---|---|---|
| 8 | 5030 | 87.0% | 33.3% |
| 16 | 5747 | 90.7% | **45.4%** |
| 32 | 6673 | 91.7% | 44.4% |
| 64 | 7878 | 91.7% | 41.7% |

The 8 and 16 rows are three runs of 36 questions each; 32 and 64 are one run.

File routing does not move: 87.0% against 90.7% is four answers in 108, inside
the spread between runs of the same arm (31, 32, 31 against 33, 32, 33). The
same arm rendered from the committed document scored 91.7% in the first
experiment, which is the noise floor stated plainly.

Naming the right symbol does move, and held in every run: 12, 12, 12 correct at
eight names against 19, 15, 15 at sixteen. Twelve points for fourteen percent
more tokens. Past sixteen it reverses while the tokens keep climbing, which is
the whole argument for a cap existing at all.

`DECLARED_SHOWN` is now 16.

## What changed, and the noise floor to hold later claims to

`DECLARED_SHOWN` went from 8 to 16, and nothing else. The measurement argued for
no change to what a pass is asked for, no change to the schema, and no change to
the caps in `fitting.rs`: the written line already clears the free bar by 33
points over a listing and 53 over the repomap, for one pass of about 45 seconds
per directory at Sonnet/low.

Every document in every repository renders differently from that one constant,
so the first pass over any directory after it will produce a changed file. That
is a re-render, not a re-judgement.

The noise floor is the number to keep. Identical content scored 91.7% in the
first experiment and 87.0% in the second — four answers in 108 apart, from
nothing but rerunning. No claim from this harness smaller than about four
answers is worth making, and the 8-to-16 file-routing difference is exactly that
size, which is why it is reported as no change rather than as a gain.

## Running it again

The harness is in the session scratchpad, not in this repository: `harness.py`
(questions), `repomap.py` (the baseline arm), `run_eval.py` and `run_variants.py`
(the arms), `repeats.py` (the same arm three times, which is where the noise
floor came from), `score.py`, and `pyrun.sh`, which exists only because scipy's
wheel needs a libstdc++ that a NixOS venv has no path to. Aider needs Python
3.12; 3.14 cannot build it.

The question set (`questions.json`, 36 questions with their gold file and
symbol) is the reusable part: rebuilding it costs 73 model calls and a third of
them are thrown away by the leak filter.
