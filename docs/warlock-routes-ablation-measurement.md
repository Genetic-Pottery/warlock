# Does `## Where to look` route anybody? — 2026-09-14

No. Stripped from the committed documents, file routing and symbol routing
both hold, inside the noise.

## Setup

The 36 questions from `docs/warlock-aider-baseline-measurement.md`, over
`crates/warlock-engine/src` and `crates/warlock-tui/src`, as they stood after
the per-file merge: each document carrying 12 routes. Two arms, three runs
each, `claude-sonnet-5` at low effort:

- **document** — the committed `WARLOCK.md`, sixteen declared names
- **no_routes** — the same document with `## Where to look` and nothing else
  removed; `## Structure` and every file line stay

The routes were about 240 tokens of each map, roughly 8%.

## Result

| arm | right file | right file + symbol |
|---|---|---|
| document | 101/108 — 93.5% (33, 34, 34) | 42/108 — 38.9% |
| no_routes | 102/108 — 94.4% (33, 34, 35) | 44/108 — 40.7% |

No question was answered right by one arm in every run and wrong by the other
in every run. The known noise floor is about four answers in 108; this
difference is one.

## Reading

It agrees with what the baseline already said about why the document wins:
enumerating every file with one line beats ranking a subset in depth. A
route list is a ranked subset of twelve, repeating what `## Files` and
`## Directories` already enumerate in full, and the `· declares` names carry
the symbol routing (33% to 45% when they went from eight to sixteen).

## What changed

`## Where to look` is removed: the `lookups` slot, its checks, its prompt
paragraphs, and the lifting of child routes into parents added earlier the
same day to fix a parent routing everything to its own `Cargo.toml`.

## Limit

The questions are all within a leaf-ish directory. Nothing measured whether a
parent's routes help a reader pick a child. Routes not helping where they are
most specific is the reason not to expect a twelve-route sample at a parent to
help either; it is not a measurement of it.

Arm: `no_routes` in `evals/arms.py`. Run: `evals/runs/routes-ablation.json`.
