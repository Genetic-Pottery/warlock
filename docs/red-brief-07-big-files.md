# Red brief: No file too large

A pact that read everything. Today any file over `PER_FILE_BYTE_CAP` — 128
KiB — is invisible to the pass that documents its directory: it goes into the
request as a name and a byte size, the prompt tells the model never to guess
what is inside it, and the document that comes back can say nothing true about
its contents. The hash side of freshness is whole — `subtree_hash` digests
every byte of every file, so editing an enormous file still stales its pact —
but the regeneration side is blind: the pact refreshes, and the refreshed
document still cannot describe the file it refreshed over. "Fresh" currently
means "fresh, except for the biggest things here." This project closes that
gap, and prices it so it stays closed: the first pact over a big file pays for
reading it, and every pact after that pays only if its bytes changed.

The mechanism is a map-reduce inside the engine. A file too large to send
whole is read in chunks, each chunk summarised by its own model pass, the
chunk summaries reduced to one account of the file, and that account rides
along in the directory's request the way a child's `WARLOCK.md` already does:
finished text standing in for content the pass will never hold at once. The
account is cached in `.warlock/`, keyed by the blake3 of the file's bytes, so
an unchanged file never buys a second map-reduce — a changed file simply
misses the cache, which is the whole of change detection. No step in this
project ever asks "what changed?"; every step asks "do I already have work for
exactly these bytes?", and stale work disappears on its own because its key no
longer matches anything on disk.

Read `docs/warlock-design-doc.md` before drafting, sections 6 and 11 in
particular. Nothing here touches hash-as-trigger — the subtree hash's input,
context string and meaning do not move — and nothing here changes how a model
is reached: the same `Agent` trait, run more times. Section 11's "maximal
relevant context, minimal waste" is the sentence this whole brief is spending.

Unlike brief 06, **this project is mostly engine work**. The seam holds — the
engine still spawns nothing, and the map-reduce is domain logic stated against
the `Agent` trait — but the vocabulary of a request, the shape of a pass and
what `pact_directory` does between gathering and asking all change. The TUI's
share is showing the work (slice 5). Nothing here depends on brief 06's
watcher; slice 5 depends on the footer and panel brief 05 built.

**The refresh pass is still not in this project.** Brief 06 named it the
obvious candidate for 07; this took the slot because the refresh pass will be
priced by what a re-pact costs, and this brief is what makes that cost small.
A directory's route back to green is still a full re-pact — but a re-pact that
finds every unchanged big file already summarised. Reconciliation — what a
refresh shows the model, whether it rewrites or only re-grants, whether fresh
directories inside a stale subtree are skipped — is brief 08's problem, and it
will be a cheaper problem than it would have been.

## Outcome

Pact a directory holding a two-megabyte lockfile and the `WARLOCK.md` that
lands can say what the lockfile pins, because a model actually read all of it,
one window at a time. While that happens the footer says so — which file,
which part, how many parts — so minutes of chunk passes read as work rather
than as a hang.

Pact the same subtree again tomorrow. The lockfile has not changed, so not one
model pass is spent on it: its summary comes out of `.warlock/` by content
hash, and the only passes that run are the directory passes themselves. Change
one big file among several and exactly that file is re-read; the others still
hit the cache.

A teammate clones the repository and their first pact finds the cache already
warm, because the summaries are committed state, like the documents and the
manifest. Context lives with the code.

And the floor never gives way. A binary blob is still a name and a size — no
pass is spent pretending bytes are prose. A file so large that reading it
would cost hundreds of passes is still a name and a size, with the reason
disclosed. A chunk pass that fails mid-file demotes that file back to a name
and a size and the pact carries on. Nothing new is fatal, and nothing is ever
truncated: every file is sent whole, summarised whole, or listed — never
half-shown as if it were all there.

## Success criteria

- Pacting a directory containing a UTF-8 file over `PER_FILE_BYTE_CAP` puts
  that file into the request as a name, a size and a summary produced from its
  entire contents. No file's bytes are ever partially included: whole,
  summarised, or listed.
- A second pact over the same unchanged file runs zero summarisation passes —
  pinned in engine tests by a fake agent that counts its calls.
- When one of several over-cap files changes, only that file is re-summarised;
  the rest resolve from the cache.
- A fresh clone carrying a committed cache gets cache hits on its first pact.
- A file that is not valid UTF-8 is never chunked or summarised: name and
  size, as today, with its cause disclosed.
- A file over the chunk-count ceiling is never summarised: name and size, with
  its cause disclosed.
- A map or reduce pass that fails — agent error, empty answer, unusable
  answer — demotes the file to name-and-size, is reported alongside the pact,
  and fails nothing. `pact_subtree` completes.
- Every chunk sent is valid UTF-8, split on line boundaries, and identified to
  the model as part *i* of *N* of the file — never presented as the whole.
- Summaries count toward `REQUEST_BYTE_CAP`. When trimming is needed,
  over-budget files are demoted to summaries before they are demoted to names,
  and the request converges.
- The observer is told about every summarisation pass before it runs, and the
  TUI footer shows it: file, part, part count. No model pass is silent.
- `subtree_hash` is byte-for-byte unaffected: same input rules, same
  `HASH_CONTEXT`, and `.warlock/summaries/` invisible to every walk and every
  hash, as everything under `.warlock/` already is.
- `warlock-engine` still opens no sockets and spawns no subprocesses, and its
  tests still run with no terminal, no network and no `claude` binary present.
- The gate — `cargo fmt --all --check`, `cargo clippy --workspace
  --all-targets -- -D warnings`, `cargo test --workspace` — stays clean.

## Constraints

- Everything from briefs 01 through 06 still holds: stable Rust 2024, system
  toolchain and no `rust-toolchain.toml`, Apache-2.0, no `unsafe`, no HTTP
  client, no Anthropic SDK, no mocking framework, no async runtime. The
  dependency edge still runs TUI to engine and never back.
- **No new dependency.** The cache keys with blake3, which the engine already
  has for the subtree hash. If a slice finds itself wanting a crate, the thing
  it wants is probably already in the workspace table.
- **The `Agent` trait keeps its one method.** A map pass, a reduce pass and a
  directory pass are the same thing to the transport: a request in, text out.
  What kind of pass it is lives in the prompt and the request's contents, not
  in a second method the binary would have to implement.
- **The prompts are code.** The map prompt and the reduce prompt sit beside
  `PROMPT` in `pact.rs`, reviewed in a diff like everything else. No
  configuration, no template, no override.
- **Omit-and-list is the floor, and truncation is still forbidden.** Every way
  summarisation can decline — binary bytes, the ceiling, a failed pass — lands
  on today's behaviour: name and size, cause disclosed, pact intact. A chunk
  is never presented as the whole file, and a summary is never presented as
  the file's text.
- **Nothing new is fatal.** Section 3 of the design doc still governs: the cap
  was never a wall and the summariser is not one either. There is no input, no
  cache state and no agent behaviour that turns a pact into an error this
  brief introduces.
- **The cache is keyed by bytes alone.** No paths, no mtimes, no sizes in the
  key. Consequence, made deliberate: a renamed file keeps its summary, so the
  map and reduce prompts must ask for an account of the *contents* and the
  directory pass must be the one that attaches the name. A summary that bakes
  in a filename is wrong under rename and the prompts are where that is
  prevented.
- **Cache writes use the write-and-rename idiom** the manifest and the
  documents already use, for the same reason: a killed pact must not leave a
  half-written summary that a later pact would trust. Content addressing makes
  concurrent writers idempotent — two warlocks writing the same key are
  writing the same fact.
- **Refresh is still manual and nothing runs by itself.** The cache is written
  during a pact and read during a pact. No background filling, no
  summarise-on-save, no model call the user did not ask for with a keystroke.
- **Cost stays disclosed.** Summarisation multiplies model passes, which is
  exactly why the observer learns about each one before it runs. If a slice
  adds a pass the footer cannot account for, that pass is in the wrong place.

## Out of scope

**The refresh pass**, as above. Brief 08 at the earliest, and deliberately
after this one: reconciliation rules should be designed against re-pact costs
as this brief leaves them, not as they are today.

**Per-file hashes in the manifest.** The obvious other route to "which file
changed" — record a hash per file at grant time, diff on refresh — is
rejected, not deferred. It grows `pacts.toml` from one line per directory to
one per file, complicates grant semantics that are currently one hash against
one subtree, and buys nothing the cache key does not already give: a changed
file misses, an unchanged file hits, and nobody ever has to name the culprit.

**Incremental prose updates.** The directory pass rewrites its whole
`WARLOCK.md`, as it always has. Prose does not decompose by file — one edit
can falsify a sentence anywhere in the document — and the directory pass was
never the expensive part. What keeps an unchanged big file's paragraph stable
across rewrites is that the pass is fed the identical cached summary, not that
any machinery diffs the document.

**Summarising binary files.** A model pass over a PNG's bytes is spent money
and invented prose. Name and size is the honest account, and stays.

**Cache eviction.** A stale entry — its file changed or gone — is a few
kilobytes of text that nothing will ever read again. Sweeping them needs a
notion of "referenced by nothing current", which wants the whole tree's big
files in one place, which is a root-pact concern. Named here so it is a
decision rather than an omission; not taken now.

**Making the caps configurable.** `PER_FILE_BYTE_CAP` and `REQUEST_BYTE_CAP`
hold their values and their non-configurability. This brief changes what
happens at the edge, not where the edge is.

Also out, as before: blessing and the gate, pact expansion proposals and
footprint prediction, Linear and Red/Forman integration, the journal at
`~/.warlock/`, the pre-push guard, the CI validator, non-interactive entry
points, the paid crate, and publishing anything.

## Scope

### 1. A third thing a file can be

depends_on: []

`File` in the engine's agent vocabulary is `Present` — bytes — or `Omitted` —
a size. This slice adds the summarised file: a name, a size, and an account of
its contents that is explicitly not its text. The builder surface grows the
way it is documented to grow — additively — so every existing `Agent`
implementation and every canned test agent compiles untouched.

The prompt learns the new state. The paragraph that says some files appear as
a name and a size gains a sibling: some files appear with a summary produced
by an earlier pass over their whole contents — trust it as an account of the
file, and never quote it as the file's text. The omitted paragraph stays,
because omission stays (binary files, the ceiling, failures).

Decide the reporting. Today a too-large file is a `Problem`, because its
contents were invisible to the pass. A successfully summarised file is not
that — the pass saw an account of all of it — so it should stop being
reported as one. What remains a `Problem` is every fallback: unreadable,
binary, over the ceiling, summarisation failed. The panel's "not sent" lines
follow from this, so make the call here, in the vocabulary, not in the UI.

Summary text counts toward the request budget the way child documents do.
Nothing in this slice runs a pass; it is the vocabulary the next three slices
speak.

### 2. The map-reduce

depends_on: [1]

The new step in `pact_directory`, between `gather_request` and the directory
pass: for each file gathered as too large, read it in chunks, run one map pass
per chunk, one reduce pass over the map answers, and replace the omission with
the summarised file. One chunk skips the reduce — the map answer is the
summary. The same `&dyn Agent` runs every pass; the engine still spawns
nothing, and the engine's tests drive all of it with counting fakes.

Chunking is where correctness lives. Chunks split on line boundaries, at a
chunk cap below `PER_FILE_BYTE_CAP` — the map prompt needs room in the same
window. Every chunk is valid UTF-8 because the file was checked before
chunking began (see the guardrails), and every map request says what it is
holding: part *i* of *N* of this file, by name — while asking, per the cache
constraint, for an account of the contents rather than a restatement of the
name.

Three guardrails, each landing on omit-and-list:

- **Binary bytes.** A file that is not valid UTF-8 is not summarised. Decide
  the sniff — invalid UTF-8 is the obvious test and the read is already
  happening — and disclose the cause in its `Problem`.
- **A ceiling on chunks.** Some number of chunks — a few dozen — beyond which
  a file is left as a name and a size, because a single file must not turn
  into hundreds of passes. Pick the number and say in the diff what it is
  protecting against, brief 06's rule for numbers that matter more than they
  look.
- **Failure, anywhere.** A map pass errors, a reduce pass errors, an answer
  comes back empty or absurdly short: the file demotes to omitted, the cause
  is reported, and the pact continues. The rule that no byte cap is fatal
  extends to every pass the caps now cause.

The map and reduce prompts land beside `PROMPT` with the same documentation
discipline: what they assume, what they forbid, and why they are code.

### 3. The cache

depends_on: [2]

Before the map-reduce runs, hash the file's bytes with blake3 and look in
`.warlock/summaries/` for a file named by that digest. A hit is the summary,
for free. A miss runs slice 2 and writes the answer under that key,
write-and-rename. That is the entire protocol: the miss *is* the change
detection, and no code anywhere compares old state to new.

The cache is repo state, committed to git — the same decision the design doc's
section 4 already made for the manifest and the documents: context lives with
the code, and a clone should not re-pay for reading an unchanged lockfile.
Everything under `.warlock/` is already pruned by name from every walk, every
request and every hash, so the cache is invisible to freshness by
construction; a test should pin that anyway, because the property is
load-bearing.

`pact_directory` needs to know where `.warlock/` is, which it currently does
not — the manifest root is `pact_subtree`'s knowledge. Hand it down as a
parameter. The engine still resolves nothing from the environment.

### 4. Demotion instead of the cliff

depends_on: [2]

`trim_to_budget` currently turns the largest files into names until the
request fits. With summaries in the vocabulary it has a gentler step: demote
the largest file to its summary — through the cache, so repeated trims of an
unchanged file cost one map-reduce ever — and only fall to name-and-size when
even the summaries do not fit, which the arithmetic makes nearly impossible: a
summary is a couple of kilobytes against a 256 KiB budget. Largest-first stays,
ties broken by path stay, children's documents still never give way. The cliff
at both caps is now a slope: whole, then summarised, then listed — and every
step of it disclosed.

### 5. Show the work

depends_on: [2]

A two-megabyte file is around sixteen map passes, each a real model
invocation. Silent, that is brief 06's "nothing happened, is it broken?"
failure, minutes long. The `Observer` trait gains a method — told before each
summarisation pass: which file, which part, how many — with a default that
does nothing, so the trait's existing implementors, the no-op observer
included, compile untouched.

The TUI puts it on the footer where the pacting progress already lives:
pacting stays "directory 3 of 40", and while chunks run the line carries the
file and the part count too. The panel's account of problems follows slice
1's reporting decision: fallbacks show with their causes, summarised files are
not listed as problems. No new pane, no new key, no scrolling log — the
footer's one line, saying what is being paid for right now.
