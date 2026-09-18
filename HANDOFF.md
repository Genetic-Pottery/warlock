# Engine work: stop sending the pass prose it will repeat as fact

Work plan for a fresh agent. Starting point is this file's commit on `main`.

Read `../warlock-test-repo/HANDOFF.md` first. `check.sh` there is the
specification and this engine is what has to fit it; the two documents are meant
to be worked in that order.

Everything below was measured on 2026-09-18 against real passes over the
fixture. Where a number appears, it was counted, not estimated.

---

## The change

**A pass is shown the file's code. Comments never reach it.**

Today every byte of a file goes to the pass, comments included. That is the
input side of a defect that cannot be fixed on the output side, and the whole
justification is this loop:

1. The pass reads a comment and believes it.
2. It writes the claim into the file's line.
3. A check refuses the line.
4. The pass rewrites the same claim in a shape the check cannot see.
5. You narrow the check. Return to 4.

Observed, in a single run, both escapes:

- `engine/core` was refused for naming `Decoder::decode()` — a class two
  directories away that `balance.rs` never calls, asserted by its module
  comment. Re-asked, it returned `VAULT_LIMIT = 512 applied post-decode`: the
  same invention with the names filed off, and **no name-based check can ever
  catch that**.
- `legacy` asserted `LEDGER_VERSION` and `VAULT_LIMIT` as bare ALL-CAPS nouns,
  which `document::referenced` skips deliberately — widening it to every
  identifier refuses most true lines.

There is no bottom to that well, because what is wrong is the input.

### Where to make it

`fitting::elided_or_whole` is the **single funnel** every byte of file text
passes through on its way to a pass. Confirm that before relying on it:

```bash
grep -rn "elided_or_whole\|File::present" crates/warlock-engine/src/*.rs
```

Strip there, before the elision, and send the stripped text even when `elide`
finds nothing to drop — otherwise a file with comments but no test module is
sent whole.

`fitting::describe` builds `Described.tokens` from the file's text read off
disk, not from the request, so it needs the same stripping or a comment goes on
witnessing its own claim in the evidence even after it stops reaching the pass.

### The comment table

Comment form belongs in its own extension-keyed table, **not** as a field on
`languages::Language`. The two tables answer questions of different difficulty:
where a language writes prose is settled by its grammar and can be looked up,
while what a line must start with to be a declaration is a guess that wants a
real repository to check it against. Coupling them prices the first at the
second — covering C's comments means inventing C's declaration prefixes and
test-suffix conventions with nothing to falsify them, which is why only eight
languages had comment coverage.

Two rows encode a decision rather than a lookup, and both are traps:

- `.css` takes the block form only. `//` is not a comment in CSS, and stripping
  from it cuts `url(https://…)` off at the scheme — real code taken for prose,
  which is the one direction this may not be wrong in. The preprocessor
  dialects (`.scss`, `.sass`, `.less`) do have it.
- `.m` is Objective-C, not MATLAB, which shares the extension and writes `%`.
  Adding `%` would eat every `@"%d"` format string. Slashes cost MATLAB
  nothing, since `//` is not code there either.

An extension no row claims must be left completely alone — sent whole, comments
and all. That is the same conservatism as the rest of the module: a row that is
absent cannot be wrong about a language.

Two guardrail tests are worth writing, because nothing in the type system ties
the tables together: every row in the language table must have a comment form,
and no extension may be claimed by two comment rows.

### Doctests go too

Strip `///` and `//!` like any other comment. A carve-out to keep the fenced
examples was built and then thrown away on the right question: does it help the
reader?

It does not. A doctest is a usage example. The output is a ~280-character
routing line naming the types and functions a reader would come to the file
for; the declaration already gives the name and signature, and the example's
setup and asserts will never fit. A doctest cannot lie, but "cannot lie" is not
a reason to spend bytes — the bar is whether it improves the routing line.

### What it costs and what it buys

Measured over this repository's own `crates/`: **110 files, 2.88 MB, 30.5% of it
comments.** So this is cheaper as well as truer, and more files fit under
`PER_FILE_BYTE_CAP`.

What is given up, weighed rather than overlooked: a pass shown a comment
sometimes catches one that is wrong about its own code and says so. It did, in
the last run with comments on. But that clause spends characters a document has
for routing, and the reader is a model that will infer why a file exists from
the code. What letting comments in reliably buys is somebody's stale prose,
handed to the next reader as fact.

Measured effect on the same file, same run conditions, comments the only
variable:

```
balance.rs  in : "…is_settled(open) checks zero open accounts (doc claims account
                  count, actually returns bool); VAULT_LIMIT = 512 applied post-decode."
            out: "Defines is_settled(open: usize) checking if open == 0, and
                  VAULT_LIMIT constant set to 512."

codec.c     in : "…encode() stamping LEDGER_VERSION and dropping frames past VAULT_LIMIT."
            out: "…encode() which sums frame length with a fixed offset, tracking frames_seen."
```

Shorter, carries the real signature, and true. Lines got better, not worse.

Update `document::FILE_PROMPT`, which currently tells the pass it is given "the
file's name and size, and its text with function bodies elided". It is now
given code with comments removed.

### One channel stays open

`walk::own` keeps `.md` out of the file list, so no README and no previous
document reaches a pass — but it still collects `child_documents`, and every
child directory's `WARLOCK.md` goes to the parent's synthesis pass. That is
prose written by an earlier pass reaching a later one. Low risk, since a
child's lines are checked against that child's code, but it is second-order
drift rather than nothing. Not acted on. Do not claim in a commit message that
no prose reaches a pass.

---

## The trap to avoid while doing it

**Do not put a per-file line check in `document::check`.** It will not run.

`accept_synthesis` clears `parsed.files`, *then* calls `check`, and merges the
assembled per-file lines in afterwards:

```rust
parsed.files.clear();           // …
let defects = check(&parsed, expected, described);
let fill = Fill { files: lines.clone(), ..parsed };
```

So a loop over `fill.files` inside `check` sees an empty map on every real pass.
It fires only from `mend`, which runs only once an answer has already failed for
something else. This exact mistake was made, shipped with a commit message
claiming it worked, and its tests passed the whole time because they built a
`Fill` by hand and called `check` directly.

**The check belongs in `document::accept_file`**, which is the one place a
per-file line is ever looked at, and which today checks only the length and the
tool-name guard. Check the names there, against **that one file's** evidence,
not the directory's: a line naming a symbol some neighbour declares is spending
this file's characters routing a reader out of this file. It costs nothing in
true lines — a file that really calls `ledger::post()` has `post` in its own
text.

Measured: `Decoder::decode()` refused, the pass re-asked, and not one line in
seventeen directories fell back to warlock's own text.

**Verify a new check actually runs before believing it.** Put an `eprintln!`
inside the loop and run the case through. Reading the code plausibly and
confidently misses this; a probe does not.

Two more things to know before touching `Evidence`:

- `Snapshot::take` sends `File::omitted` for every file — names and sizes, no
  text. The synthesis pass never sees a file. `one_file` is the only road that
  sends text.
- `document::written` splits a qualified name into identifiers and accepts it
  when **all** parts appear in one file's token set. `Decoder::decode` is met by
  a file writing `Decoder` and `decode` anywhere in it, in any relation.

---

## Pitfalls that cost real time

**A full `../warlock-test-repo/check.sh` takes about 25 minutes** and costs real
model spend. `monolith` is 51 files and dominates it. Use `cargo test` as the
inner loop and the fixture sparingly.

**`5dba3c6` on the `a-comment-is-not-a-declaration` branch is not rustfmt-clean**
— `fitting.rs` and `tests/document.rs`. CI runs the fmt gate before the tests,
so anything built on that branch is red before a test runs. Check
`cargo fmt --all --check` before assuming a failure is yours.

**Never edit a script while a pact is reading it.** Bash reads a script
incrementally by byte offset; inserting lines shifts everything after it and the
interpreter resumes mid-token. This corrupted a run.

**`pgrep -f "warlock pact"` matches its own shell**, because the pattern is in
the command line of the process doing the matching. It reports the pact running
long after it exited. Read the log for progress.

**`grep` in a Claude Code shell is a ugrep shim** and disagrees with GNU grep on
some exit codes. Run scripts as `bash --noprofile --norc`, and assert on text
rather than exit status.

**Background long runs with `setsid nohup … & disown`** — the harness kills
plain backgrounded jobs. A "low on memory" kill notice is not real; read
`available`, not `free`.

**Restore the fixture's regenerated `WARLOCK.md` files before committing there**,
and never `git add pipeline/WARLOCK.md` — it is a planted input that `check.sh`
restores before each pact. Committing a regenerated document over it emptied
four checks for eleven days.

**Snapshot the fixture's documents before re-running** if you want to compare
output across two engine versions. A run deletes them all at the start.

---

## Measured, not acted on

`## Structure` restates `## Files` in small directories and does real work in
large ones. Verbatim copies, by directory size, across this repository and the
fixture:

```
 1-4 files:  23/29 structure lines are verbatim copies (79%)
 5-11 files: 10/11 (90%)
 12+ files:   9/36 (25%)
```

The mechanism: the synthesis pass is handed the file lines, not the code, so
below about a dozen files there is nothing to join and it restates. Above that
it produces genuine cross-file wiring — `check.rs, query.rs and running.rs
implement warlock check/stale/fresh/pact/refresh via boundary.rs and
descent.rs` — which is the one part of a document grep cannot reproduce.

A floor of "an entry must relate at least two files of the directory" would drop
the restatements and keep the wiring, and `Entry` already carries `names`. It
was **not** done, and it is polish rather than value: it removes noise without
adding routing. Not a recommendation — a measurement, so the next person
arguing about that section argues from numbers.
