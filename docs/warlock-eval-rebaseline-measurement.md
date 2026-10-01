# Rebaselining the routing eval — 2026-10-01

The 36-question set from 2026-09-14 no longer described the code. It covered 36
of the 90 files now in the two measured directories. Two of its gold symbols no
longer existed, and five more were no longer unique to their file. The question
set was rebuilt and every surviving arm was run again, three times each.

The new numbers are lower than the old ones, and the two sets can't be
compared. The new set asks across 90 files instead of 54, and the larger
directory has 61 files to choose from.

## Setup

- **Questions:** 70, rebuilt with `questions.py` over seeds 11, 12, and 13:
  23 in `crates/warlock-engine/src` and 47 in `crates/warlock-tui/src`, with one
  question per file. Of the 86 files that declare a usable name, 16 got no
  question because every generated question named its own answer.
- **Documents:** both directories refreshed with `warlock refresh` immediately
  before the run, so `declared.json` and the committed documents agree byte for
  byte.
- **Reader:** `claude-sonnet-5` at low effort, no tools.

The arms:

- **document:** the committed `WARLOCK.md`, sixteen declared names
- **declares8:** the same document with eight declared names
- **no_structure:** the committed document without `## Structure`
- **listing:** names and sizes
- **repomap:** aider's map, sized to the document's token count (3,631 against
  3,424, and 6,365 against 5,951)

## Result

| arm | right file | right file + symbol |
|---|---|---|
| document | 161/210 — 76.7% (55, 51, 55) | 93/210 — 44.3% |
| no_structure | 163/210 — 77.6% (56, 52, 55) | 85/210 — 40.5% |
| declares8 | 148/210 — 70.5% (48, 51, 49) | 79/210 — 37.6% |
| listing | 56/210 — 26.7% (18, 19, 19) | 3/210 — 1.4% |
| repomap | 36/210 — 17.1% (13, 12, 11) | 13/210 — 6.2% |

The noise floor on this set is about four answers in 70, the spread between
the document's runs.

## Reading

- The ordering from 2026-09-14 holds. A written line beats a bare listing by
  about 50 points, and aider's map comes in below the listing.
- Sixteen declared names still route better than eight. The gap is 13 answers
  over three runs, and no `declares8` run scored above any `document` run.
- `## Structure` routes nobody to a file. Stripping it moves file routing by
  two answers in 210, inside the noise. That is the result `## Where to look`
  got before it was removed.

Runs: `evals/runs/baseline-2026-10.json` and `evals/runs/repomap-2026-10.json`.
