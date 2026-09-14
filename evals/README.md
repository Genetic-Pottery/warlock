# evals

Measurements of whether a `WARLOCK.md` is any good, which the Rust test suite
cannot answer. Those tests check the machinery — that `check` rejects a claim
naming something absent, that `mend` converges, that `render` writes these exact
bytes. Nothing in them says whether the document that comes out helps anybody
find a file.

This is not a test suite and must never gate anything. It is stochastic, it
costs model calls, and it produces a number with noise around it rather than a
verdict. **The same content has scored 91.7% and 87.0% on two runs with nothing
changed**, so about four answers in 108 is the smallest difference worth
claiming. Every conclusion drawn here is written up with its date in `docs/`.

## What it measures

A document exists to answer one question: given something I want to do, which
file do I open? So that question is put 36 times, to one map at a time, and the
answers are counted.

The questions come from the source and never from a document — a document is one
of the things on trial, and a question phrased out of its own `## Where to look`
line would be scoring the arms on a test one of them wrote. Each has a gold file
and symbol taken from a declaration unique to that file.

## Running it

Needs the `claude` CLI on `PATH`. One arm is 36 model calls.

    python3 evals/run.py my-run document listing
    python3 evals/score.py evals/runs/my-run.json

    # the noise floor: the same arm, three times
    python3 evals/run.py steadiness document --runs 3

Arms are built from the **currently committed documents**, so a run measures the
repository as it stands. Available: `document` (as committed, sixteen declared
names), `declares8` / `declares32` / any count, `isolated` (file lines written
one file at a time), `document_no_synthesis` and `isolated_no_synthesis` (the
three cross-file sections stripped), `listing` (names and sizes, the floor), and
`repomap`.

`repomap` is aider's tree-sitter and PageRank map at a matched token budget — the
free bar a written line has to beat. It needs its own virtualenv, because aider
will not build on Python 3.14 and scipy's wheel cannot find libstdc++ on NixOS:

    nix-shell -p python312 --run 'python3.12 -m venv /tmp/av && /tmp/av/bin/pip install aider-chat'
    nix-shell -p gcc --run 'LD_LIBRARY_PATH=$(dirname $(gcc -print-file-name=libstdc++.so.6)) \
        /tmp/av/bin/python evals/run.py baseline document repomap listing'

## The files

- `common.py` — paths, the model and effort a document pass runs at, the one
  place a question is put to a model.
- `questions.py` — builds `questions.json`. Roughly two calls per kept question;
  about half are thrown away for naming their own answer. Run only when the set
  needs rebuilding.
- `arms.py` — every map under test. Parses warlock's own rendered file lines to
  vary one thing and hold the rest still.
- `isolated.py` — writes `isolated_lines.json`: one line per file from a pass
  shown that file alone. One call per file.
- `run.py`, `score.py` — ask and count.
- `runs/` — the answers behind the write-ups in `docs/`, kept so a claim can be
  checked without paying for it again.

## The two dumps

`declared.json` and `skeletons.json` are warlock's own measurements — the
declared names behind a `· declares` list, and the reduced source a pass is
actually shown. Both are snapshots taken through a temporary test in
`crates/warlock-engine/src/languages.rs`, because `declared_names` and `skeleton`
are `pub(crate)` and making them public to serve a measuring tool is a worse
trade than a probe that is deleted afterwards:

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

`skeletons.json` is the same shape with `super::skeleton(&path, &text)` and its
`.text`. Both go stale as the source changes; regenerate before a run that
depends on them being current.

## What has been settled here

- `DECLARED_SHOWN` went from 8 to 16: same routing to the file, 33.3% to 45.4%
  naming the right symbol, over three runs.
- A written line beats the free map it has to beat: 91.7% against aider's 38.9%
  and a bare listing's 58.3%, at a matched budget.
- A line written without its siblings routes as well as one written with them,
  which is what unblocked per-file granularity.

See `docs/warlock-aider-baseline-measurement.md` and
`docs/warlock-per-file-isolation-measurement.md`.
