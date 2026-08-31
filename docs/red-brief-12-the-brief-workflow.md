# Red brief: The brief workflow

Warlock can be talked to and remembers nothing. Ask it what `pact.rs` does and
it answers well; quit, and the conversation is gone, because brief 11 put the
session in memory on purpose and wrote no transcript. That is the right call for
a chat. It is the wrong call for the thing this project's own `README.md` says
happens first:

> A change does not start with a file. It starts with a conversation. You talk
> the change through with the patron, and it pushes back … What comes out is a
> brief and a set of tickets.

Nothing in warlock produces the first of those. The eleven documents in `docs/`
were all written the same way — many turns of proposing, arguing and narrowing
against a model, ending in one long message that became a file — and every one
of them was produced somewhere other than warlock, by a tool warlock is
otherwise the front door for.

Brief 11 saw this coming and parked it, with one decision carried forward:

> **warlock writes the file from the text the model returned.** The agent is not
> given `Write` to do it, for the same reason a pass is not given `Write` to
> write a `WARLOCK.md`.

This is that item, done as a mode rather than as a keystroke. The thread gets a
second way to be: the default is a conversation about the repository, and one
word turns it into a conversation aimed at an artifact. Same card, same session,
same history — you do not get moved to a new window, and nothing you already
said stops being on screen. What changes is what the model has been told it is
doing, and that there is now a file the conversation is converging on.

## How you get there, which is the part the keyboard decides

Warlock's commands are single unshifted letters and it has run out of them.
`p r s v e m o f g G j k q` are taken, `Tab` and `Shift-Tab` move focus and swap
cards, `Space` folds, and the footer's keys line already drops entries from the
middle at eighty columns — `KEY_DROP_ORDER` exists because that line does not
fit. Shift-Tab, the obvious analogy from other tools, is the card swap and is
about to have three cards to swap between.

So the entry point is a **slash command typed into the composer**, and the
argument for it is not that the keyboard is full. It is that the composer is the
one place on this screen where a keystroke costs nothing: brief 11 established
that while the composer holds the keyboard every letter types itself, with a
test per key asserting it, so a leading `/` in an empty draft cannot collide
with anything that exists or anything added later.

The second argument is about where the mode belongs. Entering brief mode is
something done while talking, and no row in the tree is its subject. A footer
key would mean reaching out of the conversation in order to change the
conversation, and it would put the mode's home on the half of the screen the
mode is not about.

Considered and refused: **`b` starts a brief about the selected directory.** `b`
is genuinely free, and using the row is superficially tempting — a brief with a
module footprint already attached. But the cursor sitting on
`crates/warlock-tui/src` is not a statement of intent, and inferring one puts a
wrong assumption into the artifact before a word has been typed. The
conversation decides the footprint. The cursor is where the cursor is.

## Outcome

You are talking to warlock about the repository, as you can today. Then:

```
> /brief
  0:00  brief mode · nothing written yet
  0:04  alright — what is the change, and what is it for?
```

The card's header says which mode it is in and that no file exists yet. Nothing
was cleared to get there: the turns above are still on screen, the session is
the one it always was, and the model still holds every file it read and every
opinion it formed in the twenty minutes before the word was typed. What changed
is that it has been told what the conversation is now for, and it says so
immediately rather than waiting to be asked something.

It argues. It proposes three ways to do the thing and says which one it would
pick, it says what a decision costs, and it pushes back when the answer is that
the idea is bigger than it looks. Twenty turns later:

```
> /write
write to: docs/warlock-brief-12-the-brief-workflow.md
```

The path is proposed and editable. `NN` is one above the highest already there,
the slug comes from the brief's own title line, Enter accepts, Esc changes
nothing. The model returns the whole document as one message, warlock writes it,
and the thread says so:

```
 18:42  wrote docs/warlock-brief-12-the-brief-workflow.md — 14.2 KB
 18:42  docs is now stale
```

Then the file sits there doing nothing. It is a `.md` in a directory, readable
with `v`, editable with `e`, and no part of warlock consults it or acts on it.
That is the whole of what an artifact is in this brief.

## Success criteria

**Slash commands**

- A draft whose first character is `/` and whose first word names a command is a
  command, matched on submit. It never reaches the model and never appears as a
  turn.
- Three commands exist: `/brief` enters brief mode, `/write` converges and
  writes, `/chat` leaves brief mode without writing anything.
- A leading `/` that is *not* a known command is text. `/home/cole/notes` sends
  as a message, because `home/cole/notes` is not a command word.
- A `/word` that looks like a command and is not one — `/breif`, `/plan` — is
  refused with one line in the thread naming the three that exist. It does not
  reach the model: a typo should not cost a turn to discover.
- `/` on its own does the same thing, so the list is reachable without knowing
  anything.
- **No command takes an argument.** No flags, no quoting, no second word. A
  command word with anything after it is refused in the same line as an unknown
  one.
- Every command works only from the composer. Nothing on the tree's keyboard
  changes, and `action_for` gains no entry.

**Brief mode**

- The mode is a state of the thread, not a card and not a session. The turns
  already on screen stay on screen; nothing is cleared, hidden or reordered.
- The card's header states the mode and the artifact's state — that nothing has
  been written yet, or the path of what was.
- **The system prompt never changes and the session is never replaced.** Both
  registers run under one prompt for the life of the process, and `/brief` is a
  synthesized message sent into the conversation already in progress — the same
  mechanism `/write` uses. Nothing depends on whether `--system-prompt`
  re-applies to a continued `--session-id`, which the CLI does not say.
- The `/brief` instruction names the artifact being converged on, states the
  shape it must take, and says the job is to argue toward a decision rather than
  to agree. It is sent when the command is typed, so the model's reply is its
  opening question and the mode is visibly in effect before anything else is
  typed.
- `/chat` sends the matching instruction the other way. Neither command clears,
  hides or reorders a turn already on the card.
- A turn in brief mode runs at a higher effort than a chat turn. `EFFORT` is
  `low` today, which is right for "what does this file do" and wrong for the
  work that produces a 15 KB argued document.
- The tools are unchanged in both modes: `Read`, `Grep`, `Glob`, asserted on the
  argument vector exactly as they are asserted today, with the same list of
  smuggled writers checked against the whole vector.
- A run in flight changes nothing here. A pact writes nothing into the thread
  and does not mute the composer, so brief mode simply persists across one.

**The artifact**

- `/write` sends a synthesized instruction, not a typed message: return the
  whole document, in the shape given, as the entire reply.
- The reply is written **verbatim**, with two exceptions and no others: a
  trailing newline is ensured, and a reply that is *entirely* one fenced code
  block is unwrapped. Nothing else is parsed, reformatted, or inspected.
- Warlock holds the pen. The agent's tool grant is unchanged by `/write`, and
  the test that proves it is the one that already exists.
- The written path is proposed and editable: `<directory>/warlock-brief-NN-slug.md`,
  where `NN` is one above the highest number already in that directory and the
  slug is derived from the document's first `# ` heading.
- **A path that already exists is refused**, with the rule on the prompt's rule
  line and the field still holding the path, through `ScopeField::refused` as
  the scope prompt already does. A brief is not overwritten by accident.
- A directory that does not exist is created. Setting it is asking for it.
- A turn that fails during `/write` — the CLI missing, a non-zero exit, a
  timeout, a cancel — writes no file, puts one line in the thread, and leaves
  the mode exactly as it was. The conversation is not lost because the write
  failed.
- After a successful write, the thread names the file and its size, and names
  the nearest pacted directory that just went stale, if there is one.

**Where it goes**

- The output directory is one key in `.warlock/briefs.toml`, committed with the
  repository, defaulting to `docs/` when the file is absent.
- It is **not** in `pacts.toml`. Every run rebuilds those entries — the
  carry-across hazard brief 08 found and brief 09 paid for again with scopes —
  and a preference living in a file a run rewrites is a preference waiting to be
  erased.
- A `.warlock/briefs.toml` that will not parse is an error said in the loader's
  own words, in the shape a bad manifest is already said in. It is never
  degraded to "no setting".
- The template is a file, not a setting: `.warlock/brief-template.md` is used
  when it exists and a built-in default is used when it does not. Nothing has to
  be configured to change the shape of the artifact, and nothing has to be
  configured to get one.

**Everything else**

- `warlock-engine` gains nothing. It opens no sockets, spawns no subprocesses,
  and its tests still run with no terminal, no network and no `claude` binary.
  Everything in this brief is the front end's.
- Nothing new is written to disk except the artifact. No transcript, no draft,
  no `.warlock/` file that warlock writes on its own.
- The gate — `cargo fmt --all --check`, `cargo clippy --workspace --all-targets
  -- -D warnings`, `cargo test --workspace` — stays clean.

## Constraints

- Everything from briefs 01 through 11 still holds: stable Rust 2024, system
  toolchain and no `rust-toolchain.toml`, Apache-2.0, no `unsafe`, no HTTP
  client, no Anthropic SDK, no mocking framework, no async runtime. The `claude`
  CLI is the only way a model is reached, and `warlock-engine` stays ignorant of
  it.
- **No new dependency.** `serde` and `toml` are already in the workspace for
  `pacts.toml`, and `.warlock/briefs.toml` is one key. The path prompt is
  `prompt.rs`, which already exists, already handles a refusal, and is already
  tested without a terminal.
- **The composer does not become a shell.** A leading word, matched on submit,
  against a list of three. No arguments, no flags, no quoting, no history, no
  completion. Brief 11 said that if multi-line editing wants more than
  append/backspace/newline/submit the answer is that it does not get more; this
  reopens that narrowly and on purpose, and the narrowness is the whole of why
  it is affordable.
- **The agent still cannot write.** `Read`, `Grep`, `Glob`, and the permission
  flags stay absent from the vector. `/write` is warlock writing a file, not the
  model being handed a pen for one message. This is the criterion that keeps the
  writer list — `WARLOCK.md`, the manifest, the scope key, `init`, and now the
  artifact — a list rather than a capability.
- **One system prompt, one session, for the life of the process.** A mode is
  something a conversation is told, not a second configuration of the agent, and
  nothing in warlock depends on whether the CLI re-applies a system prompt to a
  session it is continuing.
- **The thread says nothing about money**, unchanged. A turn's cost is heard and
  discarded, as `Thread::record` already has it, and a brief conversation being
  more expensive than a question does not change what a per-turn number is worth
  on screen.
- **The artifact is inert.** Warlock does not read it back, index it, act on it,
  or know it exists after the write returns. Anything that treats a brief as
  input is a later brief and probably a different product.
- **The output directory is not special-cased out of the hash.** A brief written
  into a pacted subtree makes it stale, exactly as hand-editing a `WARLOCK.md`
  makes its own directory stale, and for the same reason: the bytes moved. The
  lever for a team that does not want that already exists and is
  `.warlockignore`; warlock does not write to it, mention itself in it, or
  quietly exclude its own output from the digest it asks everyone else to trust.
- **No transcript on disk.** The session stays per-process and in memory. The
  artifact is the record, and a conversation that ends before `/write` leaves
  nothing behind — which is a property to state in the documentation, not a
  problem to solve with a cache.
- The pact is untouched. Its request is built by the engine from the directory
  and the engine's own prompt, with no knowledge of a mode, a template or a
  conversation, and brief 11's test holding two requests up against each other
  keeps passing unchanged.

## Out of scope

**Tickets, and everything downstream of the artifact.** Red, Foreman, Linear,
Jira, a task type, an export, a footprint. The `.md` is where this brief stops,
and it stops there because the artifact is the thing that has to exist before
anything can carry it.

**The paid seam.** Its shape was settled in the conversation that produced this
brief and is written down here so it stops being an open question, but none of
it is built: **the paid layer is a sidecar process, discovered the way `claude`
is and spoken to over the same stream-json discipline — not a plugin ABI.**
Dynamic loading needs `unsafe`, which this workspace denies as its stated hard
rule, and Rust has no stable ABI to load against; WASM is a heavy dependency
against a five-crate workspace where every entry carries a justification
comment; a compiled-in private crate blurs exactly the boundary the design doc
says to keep architectural. The deciding argument is not any of those: a
subscription needs an API key, auth and a network, and warlock has no HTTP
client, no async runtime, and a stated promise that it holds no credentials of
its own. A sidecar owns all of that and warlock keeps every constraint the
eleven briefs before this one were written under. It gets briefed when there is
real traffic to carry, which is what this brief produces.

**Resuming a brief.** Quit mid-conversation and it is gone. No draft on disk, no
"continue where you left off", no list of past briefs. One thread, one session,
the life of the process, unchanged from brief 11.

**Editing the artifact inside warlock.** `e` already opens `$EDITOR` on a
selected file and `v` already reads one into the panel. A brief is a file like
any other file the moment it is written, and warlock does not grow an editor for
its own output.

**A second conversation, or two briefs at once.** One mode, one artifact in
flight.

**Rendering markdown.** The answer is text, wrapped, as it has been since brief
10. A brief-mode reply that is full of headings draws as text with hashes in it.

**A brief scoped to the selected directory.** Argued above and refused.

**Gating `/write` on a sigil.** The four keys that refuse across a closed
boundary are the ones that mutate the ledger — `p` in both directions, `r`, `s`.
Writing a document into a directory is not one of them, and adding a fifth would
make the gated set a rule of thumb rather than a definition. Considered, and
refused deliberately rather than by omission.

**Streaming the answer.** Unchanged from brief 11: work lines stream, prose
lands whole. A 15 KB document arriving a word at a time is a redraw per token to
say what a `writing` line already says.

Also out, unchanged from earlier briefs: parallel summarising passes; an editor
in the panel; more than one document at a time; syntax highlighting; telling a
document that is behind its code from one that is not; telling direct staleness
from inherited staleness; the journal; the pre-push guard; the CI validator;
`warlock status` and every other non-interactive entry point; the paid crate;
and publishing anything.

## Scope

### 1. A leading word is a command

depends_on: []

`compose_for` returns `Composed::Submit` and the loop takes
`Composer::draft` from the field. That draft becomes a question today. It
becomes one of two things here: a command, or a question.

Pure, in the shape `action_for`, `edit_for` and `compose_for` all have —
a draft in, one intention out, no terminal and no `App`. The match is on the
first word of a draft whose first character is `/`, against a list of three, with
nothing after it permitted. Everything else is a message, including a draft that
starts with a slash and continues into a path.

An unrecognised `/word`, and a bare `/`, produce one line naming what exists.
That line is the whole of the discovery mechanism, and it is why no fourth
command called `/help` is needed.

Nothing acts on the commands in this slice. It ends with a function that tells
them apart, a test per command, a test for the path case, and a loop that puts
the unknown-command line in the thread.

### 2. The mode

depends_on: [1]

The thread gains a mode, the card's header states it, and the agent's one system
prompt learns to cover both registers.

**The mode is not a second system prompt, and that is the decision this slice
exists to make.** The obvious design gives brief mode its own `--system-prompt`
and keeps the `--session-id`, and it rests on a behaviour nothing documents:
whether a prompt supplied on a later invocation replaces the one a session was
created with. The CLI's help says only that `--session-id` uses a specific id
for the conversation. Warlock already reads that CLI's `stream-json` through
`Value` with defensive field access because it is a schema written elsewhere;
betting the feel of a mode on the same vendor's unwritten resumption semantics
is that bet taken worse. And the failure direction is the expensive one — if the
prompt does not re-apply, brief mode needs a fresh session, and a fresh session
discards the conversation at the exact moment it became the material the
document is made of.

So `/brief` is a **synthesized message into the conversation already in
progress**, which is the mechanism `/write` already uses. The session is never
replaced, the history is entirely intact, and this brief has one mechanism
rather than two.

`CHAT_SYSTEM_PROMPT` grows to cover both registers, and growing it fixes
something that is wrong the moment brief mode exists. It currently ends *"You
cannot change that repository, and nothing you say is put in a file."* In brief
mode the second half is false. A second prompt would have hidden that; one
prompt has to say when it is true — that a document asked for in the agreed
shape may be written verbatim into a file whose path the model does not choose,
and that this is the one thing it says which becomes bytes on disk. That is
exactly what a model should know while writing one, and the two-prompt design
would have let it stay unsaid.

The instruction is the rest of the substance: what is being converged on, the
shape it has to take (slice 3), and that the job is to propose options, name
what each costs, recommend one, and push back when the answer is that the idea
is bigger than it looks. Agreement is not the product.

Effort is raised for a brief turn. `low` is right for a question about a file
and wrong for this, and both values stay overridable by `WARLOCK_EFFORT` as they
are today.

The cost, stated because it is real: a message steers less hard than a system
prompt, and over twenty turns the register can drift back toward chat. Two
things blunt it — `/write` restates the shape at the moment it matters, and a
conversation somebody is actively arguing with corrects its own drift in a way
an autonomous run cannot. If that proves insufficient in practice, the answer is
a firmer instruction, not a second session.

Rejected and worth recording: **a fresh session with the prior conversation
replayed into it.** `Thread` holds every message and every answer in memory, so
warlock could open a new session and hand it the history. It works. It also
doubles the token cost at the switch, needs a policy for a history too large to
resend, and has warlock reconstructing a conversation the model already had.
Strictly worse than not needing it.

### 3. The template

depends_on: [2]

A built-in default template, and `.warlock/brief-template.md` overriding it when
it exists.

The default is this document's own skeleton, because eleven briefs have now
converged on it: an opening that states the problem, `## Outcome` written as
something a reader can see happen, `## Success criteria` in bolded groups,
`## Constraints`, `## Out of scope` with the reasoning for each refusal, and
`## Scope` as numbered slices carrying `depends_on`.

The template rides in the `/brief` instruction, not in the `/write` request. A
conversation that does not know the shape it is aiming at converses without
aiming, and the shape is cheap to state once. `/write` restates it, and the
small duplication is worth more than a model discovering the required form after
the thinking is done.

A template file that cannot be read is an error in the loader's words. A
template file that is empty is a template that says nothing, which is the user's
business and not warlock's.

### 4. `/write`, the prompt, and the file

depends_on: [2]

`/write` sends the synthesized instruction; the reply comes back as an ordinary
answer on an ordinary turn; then the path prompt opens over it.

The prompt is `ScopeField` and `edit_for`, unchanged — a one-line editable field
with a rule line, already built pure and already tested without a terminal. It
opens pre-filled with `docs/warlock-brief-NN-slug.md`: the directory is the
built-in default in this slice, `NN` is one above the highest number already
present, and the slug comes from the reply's first `# ` heading, lowercased and
hyphenated. A reply with no heading gets a slug of the mode's own choosing and
the field is editable, which is what the field is for.

Enter writes. A path that already exists reopens the field through
`ScopeField::refused` with the rule on it. Esc writes nothing and keeps the
reply on the card, so a cancelled write is not a lost document.

The write itself: create the parent directory, write the reply verbatim, ensure
one trailing newline, unwrap the reply if it is entirely one fenced block. Then
one line in the thread naming the path and the size, and a second naming the
nearest pacted directory that just went stale, if there is one.

### 5. Where briefs go

depends_on: [4]

`.warlock/briefs.toml`, one key, committed, read where the manifest is read and
carried on the same value the loader already hands the front end.

Absent is `docs/`. Present and unparseable is an error, never a default. Present
and naming a directory outside the repository is refused when it is read, not
when it is written — a setting that cannot work should fail where it is stated.

This is the slice where the pre-filled path stops being a constant, and it is
last because everything above it works without it.

### 6. The footer and the documentation

depends_on: [1, 4]

The keys line gains one entry, in the composer's group, saying that `/` exists.
One entry rather than three, because the unknown-command line is the list and a
reader who has typed `/` has already been told everything the footer could have
said. `KEY_DROP_ORDER` already loses the composer's keys first at eighty
columns, which is still the right order: a reader looking at a composer has a
cursor in front of them saying it takes typing.

`README.md` gains the commands in the composer's table, and one short section on
the workflow: what brief mode is for, that the artifact is inert once written,
that nothing survives a quit before `/write`, and where the template and the
output directory live. That last point is the whole of the discoverability
story for both files, and it is the reason they are named in prose rather than
printed by a subcommand that does not exist yet.
