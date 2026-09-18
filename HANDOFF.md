# Engine work: stop sending the pass prose it will repeat as fact

Work plan. Starting point is this file's commit on `main`.

Read `../warlock-test-repo/HANDOFF.md` alongside it. That one used to be the
specification this engine had to fit; it has been cut down, because most of what
it asked for was work on the harness rather than on warlock.

Everything below with a number attached was measured on 2026-09-18 against real
passes over the fixture. Where a number appears, it was counted.

## The plan, in order — both done

1. **Verify and merge `a-comment-is-not-a-declaration`.** Merged to `main`,
   1,850 tests, fmt and clippy clean. Two defects were fixed on the way in; see
   below.
2. **Cut the fixture's lie checks** and the plants they existed to catch, and
   the over-cap pair with them. Done in the fixture repository at `8c842e0`;
   `../warlock-test-repo/HANDOFF.md` records what went and what stayed. The
   suite is 28 checks, all positive, all passing.

That is all of it. The head sample for over-cap files was planned here and has
been dropped on a better argument than the one it was planned on — see "The
over-cap file, and why it is parked".

**Nothing here is owed further work.** Both repository roots are stale, because
editing a `HANDOFF.md` changes the digest of the directory its `WARLOCK.md`
describes; so is every fixture directory whose plant was rewritten. That is the
ledger being honest and not a defect — it is paid off by a pass when someone
wants these documents fresh, and a fixture pass costs 25 minutes and real model
spend, so it is worth batching with the next real change rather than run for
its own sake.

## Decisions already made — do not reopen these

- **Comments do not reach a pass.** Settled. Do not re-argue it, and do not
  build a check that polices comment-derived claims in the output instead.
- **An over-cap file stays named and sized**, and is not described from a
  sample. This reverses what this document said earlier on the day it was
  written; the reasoning is below and it is stronger than what it replaced.

---

## Step 1 — the comment branch, merged

`a-comment-is-not-a-declaration`, 6 commits, merged to `main`. It is where every
measurement in this file came from. 1,850 tests pass, fmt and clippy are clean,
and the per-file name check sits in `document::accept_file`, which is the
correct home for it — see "The trap" below.

Two defects were found reviewing it and fixed in `75f4ca8` before the merge.
Both are worth knowing about, because neither was visible from the branch's
commit messages and one would have been silent:

- **`5dba3c6` dropped the `warlock-team` scope from the repository root.** It
  un-pacted to regrant from scratch, and un-pacting takes the scope with it. Its
  own message says `warlock scope` on the root is the whole of putting it back —
  and that was never run. Nothing below the root carries a scope of its own, so
  merging as it stood would have opened every directory in the repository to
  anyone, with no line in any diff saying so.
- **The comment above `fitting::tokens_of` had gone false.** `3d56ebb` wrote it
  when the change kept comments out of the *evidence* only; `8333d39` then
  stopped sending them to the pass at all and did not reread the block, whose
  last clause still read "the text that reaches the model is untouched — a pass
  still reads every comment". Exactly the half-true block `CLAUDE.md` is about,
  in the file that argues for the change.

One thing in it is known to be wasted work, and was kept anyway: it grew a
comment-form table covering C, Terraform, SQL, Lua and a dozen more, built to
close a channel this change closes completely. The breadth costs nothing but is
not why the table should exist.

### What the change is

**A pass is shown the file's code. Comments never reach it.**

Before it, every byte of a file went to the pass, comments included. That is the
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

### Where it is made

`fitting::elided_or_whole` is the **single funnel** every byte of file text
passes through on its way to a pass. Stripping happens there, before the
elision, and the stripped text is sent even when `elide` finds nothing to drop —
otherwise a file with comments but no test module goes whole.

`fitting::describe` builds `Described.tokens` from the file's text read off
disk, not from the request, so it is stripped too; otherwise a comment goes on
witnessing its own claim in the evidence after it stops reaching the pass.

### The comment table

Comment form lives in its own extension-keyed table, **not** as a field on
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

An extension no row claims is left completely alone — sent whole, comments and
all. That is the same conservatism as the rest of the module: a row that is
absent cannot be wrong about a language.

Two guardrail tests earn their place, because nothing in the type system ties
the tables together: every row in the language table must have a comment form,
and no extension may be claimed by two comment rows.

### Doctests go too

`///` and `//!` are stripped like any other comment. A carve-out to keep the
fenced examples was built and then thrown away on the right question: does it
help the reader?

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

`document::FILE_PROMPT` is updated with it — it previously told the pass it was
given "the file's name and size, and its text with function bodies elided".

### One channel stays open

`walk::own` keeps `.md` out of the file list, so no README and no previous
document reaches a pass — but it still collects `child_documents`, and every
child directory's `WARLOCK.md` goes to the parent's synthesis pass. That is
prose written by an earlier pass reaching a later one. Low risk, since a child's
lines are checked against that child's code, but it is second-order drift rather
than nothing. Not acted on, and not planned. Do not claim in a commit message
that no prose reaches a pass, and do not let the fixture cleanup in step 3 be
written up as though this were closed.

---

## The over-cap file, and why it is parked

`data/inventory.json` is 1,755,356 bytes and its line reads *"contents not
loaded, structure and fields unknown"*, which routes nowhere. This document
planned to fix it by sending a head sample, and that was started and reverted
the same day. **Do not pick it back up without answering the objection below.**

The mechanics were never the problem and are all in place: `pact::read_capped`
and `view_file` already truncate, flag the cut and handle a cut landing
mid-character, and `agent::Content` takes a fourth variant cleanly. Nor is the
cap the problem — `PER_FILE_BYTE_CAP` is `1024 * 1024` with nothing justifying
it, so the number is arbitrary and may be changed, though raising it far enough
to admit this file admits every file, which is what a cap is for.

**What stops it is that a sampled line cannot be checked.** Every other claim in
a document rests on a name witnessed in that file's own token set — that is what
`accept_file` enforces and what the whole comment change above was for. A line
written from 8 KB of a 1.7 MB file is a claim about the 1.7 MB, and warlock has
no way to tell a good one from a bad one. It would be the one line in every
document that is trusted rather than checked, in a tool whose entire subject is
not trusting documents.

Marking the sample as a sample softens how the claim reads. It does not make it
checkable, which is the part that matters.

There was also a recorded rejection of truncation, in `agent.rs` above
`Content`, which this plan would have overturned: "sending the first n bytes of
a source file was rejected because it invites confident wrong conclusions about
the part that never arrived". It is still there and still right.

**The shape a future attempt should take** is reading the file rather than
describing it — parsing an over-cap file's structure deterministically, so
`inventory.json` yields the field names `id`, `sku`, `qty` as extracted facts
that the existing name check can witness, the same as any other file's
declarations. That is a real design and a much larger one; a per-format parser
is a different project from a freshness ledger. It is written down here so the
next person starts from it instead of from the sample.

What must **not** come back under any of this: a chunked map-reduce over the
whole file with a disk cache under `.warlock/summaries/`. That was removed with
the budget ladder and `fitting.rs` still argues for its removal. It is a model
pass per chunk to learn what 2 KB already says — and it produces the same
uncheckable claim, more expensively.

## The trap, if a per-file check is ever touched

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

The merged branch fixes it: **the check lives in `document::accept_file`**,
the one place a per-file line is ever looked at, checked against **that one
file's** evidence and not the directory's. A line naming a symbol some neighbour
declares is spending this file's characters routing a reader out of this file.
It costs nothing in true lines — a file that really calls `ledger::post()` has
`post` in its own text. Do not move it back, and do not add a second copy in
`check`.

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

## Considered and dropped

These were written up as work and are not being done. Recorded so the next
person does not rediscover them as open.

**Two of the fixture's four sections are work on the harness.** Replacing
`present` with a word-exact `declares` helper, and reading the pacted directory
list from `.warlock/pacts.toml` instead of a hardcoded list. Both findings are
real — five of eleven symbol checks are answered by an English word in prose,
and `monolith` has never been checked. Neither makes warlock better at its job.
A session spent making a test suite test itself is a session not spent on the
two capabilities above. If they are ever done, they are an afternoon, not a
project.

**`## Structure` restates `## Files` in small directories** and does real work
in large ones. Verbatim copies, by directory size, across this repository and
the fixture:

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
removes noise without adding routing, which is why it is here and not above. Not
a recommendation — a measurement, so the next person arguing about that section
argues from numbers.

---

## Pitfalls that cost real time

**A full `../warlock-test-repo/check.sh` takes about 25 minutes** and costs real
model spend. `monolith` is 51 files and dominates it. Use `cargo test` as the
inner loop and the fixture sparingly — in particular, a stripping regression
belongs in `cargo test`, not in a 25-minute fixture run.

**`5dba3c6` on the `a-comment-is-not-a-declaration` branch is not rustfmt-clean**
— `fitting.rs` and `tests/document.rs`. `4d4a4bc` pays it off, so the branch tip
is clean and only a build from that one commit is red. CI runs the fmt gate
before the tests, so check `cargo fmt --all --check` before assuming a failure
is yours.

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
