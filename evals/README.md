# evals

Measurements of whether a `WARLOCK.md` is any good, which the Rust test suite
cannot answer. Those tests check the machinery — that `check` rejects a claim
naming something absent, that `mend` converges, that `render` writes these exact
bytes. Nothing in them says whether the document that comes out helps anybody
find a file.

This is not a test suite and must never gate anything. It is stochastic, it
costs model calls, and it produces a number with noise around it rather than a
verdict. **The same document has scored 55, 51, and 55 out of 70 on three runs
with nothing changed**, so about four answers in 70 is the smallest difference
worth claiming. Every conclusion drawn here is written up with its date in
`docs/`.

## What it measures

A document exists to answer one question: given something I want to do, which
file do I open? So that question is put 70 times, to one map at a time, and the
answers are counted.

The questions come from the source and never from a document. A document is one
of the things on trial, and a question phrased out of its own lines would be
scoring the arms on a test one of them wrote. Each question has a gold file and
a symbol declared in that file alone.

## Running it

Needs the `claude` CLI on `PATH`. One arm is one model call per question.

    python3 evals/run.py my-run document listing
    python3 evals/score.py evals/runs/my-run.json

    # the noise floor: the same arm, three times
    python3 evals/run.py steadiness document --runs 3

Arms are built from the **currently committed documents**, so a run measures the
repository as it stands. Before a run, refresh the measured directories with
`warlock refresh` and regenerate `declared.json`; a stale document measures the
code as it was. Available arms:

- `document`: the committed document, sixteen declared names.
- `declares8`, `declares32`, or any count: the same document with that many
  declared names.
- `no_structure`: the committed document without `## Structure`.
- `listing`: names and sizes, the floor.
- `repomap`: aider's map at a matched token budget.

`repomap` is aider's tree-sitter and PageRank map, the free bar a written line
has to beat. It needs its own virtualenv in `evals/av/`, because aider won't
build on Python 3.14 and scipy's wheel can't find libstdc++ on NixOS:

    nix-shell -p python312 --run 'python3.12 -m venv evals/av && evals/av/bin/pip install aider-chat'
    nix-shell -p gcc --run 'LD_LIBRARY_PATH=$(dirname $(gcc -print-file-name=libstdc++.so.6)) \
        evals/av/bin/python evals/run.py baseline document repomap listing'

## The files

- `common.py`: paths, the model and effort a document pass runs at, and the one
  place a question is put to a model.
- `questions.py`: builds `questions.json`, one question per file at most. It
  costs one call per file sampled, and about a third of the questions are thrown
  away for naming their own answer. A later seed only adds questions for files
  the set doesn't cover yet. To rebuild from nothing, delete `questions.json`
  first.
- `arms.py`: every map under test. It parses warlock's own rendered file lines
  to vary one thing and hold the rest still.
- `run.py`, `score.py`: ask and count.
- `runs/`: the answers behind the write-ups in `docs/`, kept so a claim can be
  checked without paying for it again.

## The declared-names dump

`declared.json` is warlock's own measurement: the declared names behind a
`· declares` list. It's a snapshot taken through a temporary test appended to
`crates/warlock-engine/src/tests/languages.rs`, because `declared_names` is
`pub(crate)`, and making it public to serve a measuring tool is a worse trade
than a probe that's deleted afterwards:

```rust
#[test]
#[ignore]
fn dump_declared_names() {
    let dirs = std::env::var("WARLOCK_DUMP_DIRS").expect("directories to dump");
    let mut out: std::collections::BTreeMap<String, std::collections::BTreeMap<String, Vec<String>>> =
        std::collections::BTreeMap::new();
    for dir in dirs.split(':') {
        let mut per = std::collections::BTreeMap::new();
        for entry in std::fs::read_dir(dir).expect("reads") {
            let path = entry.expect("an entry").path();
            if path.is_file() {
                let text = std::fs::read_to_string(&path).unwrap_or_default();
                per.insert(
                    path.file_name().expect("a name").to_string_lossy().to_string(),
                    super::declared_names(&path, &text),
                );
            }
        }
        out.insert(dir.to_owned(), per);
    }
    println!("DUMP{}", serde_json::to_string(&out).expect("json"));
}
```

    WARLOCK_DUMP_DIRS="$PWD/crates/warlock-engine/src:$PWD/crates/warlock-tui/src" \
        cargo test -p warlock-engine --lib dump_declared_names -- --ignored --nocapture \
        | grep '^DUMP' | sed 's/^DUMP//' > evals/declared.json

The dump goes stale as the source changes. To check that it's current, compare
`declares(rel, 16)` from `arms.py` against the committed document; they must
match byte for byte.

## What has been settled here

On the 70-question set, 2026-10-01:

- A written line routes to the right file 76.7% of the time, against 26.7% for
  a bare listing and 17.1% for aider's map at a matched budget.
- Sixteen declared names route better than eight: 76.7% against 70.5%.
- `## Structure` routes nobody to a file: 77.6% without it.

On the earlier 36-question set, 2026-09-14, which can't be compared with the
numbers above:

- `DECLARED_SHOWN` went from 8 to 16 on these results.
- A line written without its siblings routes as well as one written with them,
  which unblocked per-file granularity.
- `## Where to look` routed nobody, and was removed.

See `docs/warlock-eval-rebaseline-measurement.md`,
`docs/warlock-aider-baseline-measurement.md`,
`docs/warlock-per-file-isolation-measurement.md`, and
`docs/warlock-routes-ablation-measurement.md`.
