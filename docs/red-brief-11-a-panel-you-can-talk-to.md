# Red brief: A panel you can talk to

Two halves of one change: the panel stops reporting at a reader and starts
holding a conversation with them. One half is warlock finally saying what it is
doing during a pact. The other is the reader saying something back.

The first half is not a missing feature; it is a misplaced one. Pact a directory
of this repository and the panel says:

```
crates/warlock-tui/src
  8:55 writing
```

Which is true for thirty-six of those five hundred and thirty-five seconds. The
other eight and a half minutes were fourteen summarising passes over three
files, every one of them announced on the event channel as
`PactEvent::Summarising { file, part, parts }` with a fraction attached, and
every one of them drawn nowhere. The fraction that says how far through the run
you are — `position` of `total`, from `Observer::starting`, monotonic and known
before the first pass — is drawn on the footer, at the bottom of the screen,
while the eye is on the panel. So the panel shows the smallest part of the wait
and calls it the whole thing, and the number that would explain the wait is on a
different line of the screen. Nothing has to be invented or estimated to fix
this. It has to be moved, and the ladder underneath it has to be admitted to.

The arithmetic is worth writing down, because it is the whole of why a pact is
slow and it is not what it looks like. `PER_FILE_BYTE_CAP` is 128 KB and
`CHUNK_BYTE_CAP` is 96 KB, so a file over the cap costs one pass per chunk plus
one reduce. In this repository:

| directory | over-cap files | model passes |
|---|---|---|
| `crates/warlock-tui/src` | `app.rs` 403 KB, `ui.rs` 281 KB, `pacting.rs` 222 KB | ~14 + 1 = **~15** |
| `crates/warlock-engine/src` | `pact.rs` 418 KB | ~6 + 1 = **~7** |

Measured: 8:55 and 3:22 — about 36 and 29 seconds a pass. The model is not slow;
there are fifteen of it. A directory's own pass, the one the panel currently
narrates, is the last thirty-six seconds of nine minutes.

The second half reverses a rule this project wrote down in brief 05: *no
free-text prompt in the left panel, ever.* That rule was aimed at a sidebar chat
bolted onto a ledger — a window in the corner to talk to Claude that turns the
tree into navigation and the colours into furniture — and against that it was
right. It is not what this is. The destination, stated so the constraints below
make sense: you talk to warlock to define what work should be done, the way you
would talk to any agent that can read the repository; that definition becomes a
written plan; the plan goes to a Red-shaped service that writes tickets; a
Foreman-shaped service pulls them and does the work. Warlock is the front door
of that pipeline, and the front door is a conversation. The ledger is what the
conversation is *about* — you are talking to the thing that knows which
documents can still be trusted, in the tree that says which ones cannot.

So the rule is amended rather than abandoned, and the amendment is precise. The
**account** card still shows facts and not prose: one line per action, no tool
results, no model text. The **thread** card is a conversation and shows what a
conversation shows. They are different cards and they look different, and a
reader can always tell which one they are on. What is refused is the account
turning into a chat log, not the existence of a place to type.

One consequence to name early, because it is the thing most likely to bite:
warlock's keys are single letters. `p` pacts, `r` refreshes, `v` views, `f`
toggles files. The moment there is a text field on screen, `p` has to be able to
mean the letter p. Focus stops being a convenience for scrolling and becomes
load-bearing, and every key in `action_for` has to be reachable only when the
composer does not have the keyboard. That is a change to the input model, not a
widget.

## Outcome

Open warlock and press `p` on a directory with a large file in it. The panel
says what the run is, where it is, and what it is spending the time on:

```
PACT · 3/14  ▓▓▓▓▓▓░░░░░░░░░░░░░
crates/warlock-tui/src
  0:00  summarising app.rs (2/6)
  2:31  summarising ui.rs (1/4)
  5:02  summarising pacting.rs (1/4)
  8:19  waiting · 11 files, 34 KB
  8:41  writing · 1.8 KB
  8:55  documented · $0.0021
```

Every line of that is a fact the run already produced. The bar moves when a
directory finishes, not on a timer, and it only goes forwards. Nothing on screen
is an estimate: there is no "about four minutes remaining", because nothing in
the system knows that.

Below the panel there is one empty line with a prompt on it. Type into it and it
grows with what you type, up to a bounded window, and scrolls inside itself past
that. Press Enter and the thread starts: your turn is on the card, and under it
the model's work arrives as it happens — `grep`, `read`, `glob`, thinking — the
same one-line-per-action shape the account has always drawn, because it is the
same code drawing it, finally given a run that makes tool calls. The answer
arrives as text under the work.

Press `p` while the thread is up. The composer dims and stops taking keys: the
run has the turn. The account of that run is written into the same thread, in
place, so what you see is one history — you asked something, it answered, you
pacted a directory, that is what the pact did. When the run ends the composer
comes back.

Press `v` on a file. The document takes the panel as it does today and the
composer goes with it: you are reading a file, not talking. Shift-Tab comes back
to the thread, on the line you left.

The chat can read the repository and cannot write to it. Ask it to change a file
and it will tell you it cannot, which is the honest answer for this brief.

## Success criteria

**The run says what it is doing**

- The panel draws a run header for a pact or refresh in flight: which run it is,
  which directory, and `position` of `total`, with a bar whose fill is that
  fraction. It is drawn from `PactEvent::Starting`, which already carries both
  numbers, and it advances only when a directory does.
- A summarising pass draws a line in the account naming the file and its
  `part`/`parts` fraction. A directory whose wait is fourteen summarising passes
  shows fourteen lines, not one `writing`.
- `waiting` names what was sent: the file count and the byte count of the
  request the engine built for that directory.
- `writing` names how much has arrived, and updates while it arrives, from
  `--include-partial-messages` on the existing `stream-json` invocation.
- No estimate, anywhere: no percentage that is not `position/total` or
  `part/parts`, no remaining-time, no spinner. The section clock is the only
  thing on screen that moves without an event behind it.
- The footer keeps saying what it says today. This brief moves nothing off it;
  it stops the panel being the half of the screen that does not know.

**The composer**

- The panel's bottom edge is a one-line composer. It grows a line at a time with
  the text typed into it, to a bounded maximum, and scrolls within itself beyond
  that. The panel above it loses exactly the rows the composer takes.
- Enter submits. A newline inside a message is `Alt+Enter` — not `Shift+Enter`,
  which most terminals do not distinguish from Enter, and warlock does not ship
  a keybinding that works on one terminal.
- **No single-letter key acts while the composer has the keyboard.** `p`, `r`,
  `s`, `v`, `e`, `f`, `g`, `G`, `j`, `k` all type their letter. Every one of them
  is reachable again by leaving the composer, and there is a test per key.
- Esc leaves the composer with its text intact; Tab cycles tree → panel →
  composer → tree. A draft survives leaving and coming back, and survives a pact
  running in between.
- The composer is hidden, not disabled, whenever the document card is showing,
  and the panel gets those rows back.
- Empty submissions do nothing and say nothing.

**The thread**

- A submitted message starts or continues one session — the same session for the
  life of the warlock process, by `--session-id`, so the model remembers the
  conversation without warlock keeping a transcript of its own.
- The chat agent runs with `Read`, `Grep` and `Glob` and nothing else. No
  `Write`, no `Edit`, no `Bash`, no `WebFetch`. This is asserted on the argument
  vector in a test, the way `NO_TOOLS` is asserted today.
- Tool calls, thinking and writing draw one line each, in the shape the account
  already uses, with the clock of the turn they belong to.
- The model's answer is drawn as text, wrapped to the panel's width by the
  document wrapping added after brief 10. It is the one place model prose is
  drawn, and it is on the thread card only.
- A turn that fails — the CLI missing, a non-zero exit, a timeout — puts one line
  in the thread and one in the footer, and the session survives it. Nothing about
  a failed turn is fatal and nothing takes down the event loop.
- Ctrl-C during a turn cancels that turn, as it cancels a run, through the same
  `Cancel` handle. The thread keeps what arrived before the cancel.

**A pact is a turn, and the thread is not its context**

- Starting a pact or refresh while the thread is showing mutes the composer,
  writes the run's account into the thread as that turn's content, and unmutes
  when the run ends.
- Keys that already refuse mid-run go on refusing, unchanged and worded the same.
- **The request the engine builds for a directory is byte-identical whether or
  not a conversation has happened.** The pact stays one-shot, toolless, on the
  engine's own prompt and system prompt. There is a test that asserts the two
  requests are equal, because this is the criterion that keeps green meaning what
  it means.
- The pact's cost line and the chat's cost line are separate. Two things are
  being paid for and they are not the same thing.

**Everything else**

- `warlock-engine` still opens no sockets and spawns no subprocesses, and its
  tests still run with no terminal, no network and no `claude` binary present.
  The chat agent is the front end's, like `ClaudeAgent` is.
- The gate — `cargo fmt --all --check`, `cargo clippy --workspace --all-targets
  -- -D warnings`, `cargo test --workspace` — stays clean.

## Constraints

- Everything from briefs 01 through 10 still holds: stable Rust 2024, system
  toolchain and no `rust-toolchain.toml`, Apache-2.0, no `unsafe`, no HTTP
  client, no Anthropic SDK, no mocking framework, no async runtime. The `claude`
  CLI remains the only way a model is reached, and `warlock-engine` remains
  ignorant of it.
- **No new dependency for the composer.** `prompt.rs` already holds a text field
  — `ScopeField`, `edit_for`, append/backspace/submit/close and a cursor always
  at the end — built pure and tested without a terminal. The composer is that
  shape grown to several lines, not a text-editing crate. If multi-line editing
  wants more than append/backspace/newline/submit, the answer is that it does not
  get more in this brief.
- **The account card does not become prose.** One line per action, no tool
  results, no model text, `thinking` with no content. Model text lives on the
  thread card and nowhere else. A pact's account rendered into the thread is
  still the account: lines, clocks, no prose.
- **The thread is never context for a pact.** Not the transcript, not a summary
  of it, not "the user seems interested in error paths". A pact is `make`; it
  reads the directory and the engine's prompt and nothing that happened to be
  said in a chat window, or two people with the same repository get different
  documents and green stops being a fact.
- **The chat agent reads and does not write.** `Read`, `Grep`, `Glob`. Warlock's
  writers are still the pact, the refresh, the manifest, the scope key and
  `warlock init` — a list this brief does not extend. When the plan file lands
  (out of scope, below), warlock writes it from what the model returned, exactly
  as the engine writes a `WARLOCK.md` from stdout. The model never holds the pen.
- **No estimates.** `position/total` and `part/parts` are counts the run knows.
  Anything else — remaining time, percentage of a directory, a bar over files
  inside one pass — is a number with nothing behind it. There is no honest
  denominator inside a single pass: one directory is one request and one
  document, so files are payload, not steps.
- Cancellation, timeouts and the pipe discipline in `claude.rs` are not
  re-litigated. A chat turn is a child process with the same three-thread
  arrangement, the same `Cancel`, and the same timeout policy as a pass.
- The chat session is per-process and in memory. Warlock writes no transcript to
  disk, and `.warlock/` gains no new file.

## Out of scope

**The plan file.** `w`-writes-the-answer-to-a-`.md`, a path prompt, a
`docs/plan-*.md` convention — none of it here. It is the seam to the pipeline
and it deserves the brief where the pipeline is designed, not a keystroke bolted
to the end of this one. The decision that survives into that brief: **warlock
writes the file from the text the model returned.** The agent is not given
`Write` to do it, for the same reason a pass is not given `Write` to write a
`WARLOCK.md`.

**Red, Foreman, Jira and Linear.** Every wire to a ticketing service, every
notion of a task, every export. This brief builds the front door and stops
there.

**A chat that can change the repository.** `Write`, `Edit` and `Bash` for the
chat agent, in any permission mode. This is deferred and not refused, and it is
deferred on purpose: a read-only chat is a reviewer that can answer *is this
document still true*, which is the judgement warlock reserves for a human and
currently cannot help with at all. A chat that can edit makes warlock an agent
harness. That is a decision to take on its own evidence, after living with the
first one, and it is cheap to take later and expensive to walk back.

**Parallel summarising passes.** The fourteen passes that make a directory nine
minutes long are independent — chunks feed their own file's reduce, and files do
not interact — and running them concurrently would take a directory down toward
its slowest file rather than the sum of all of them. That is the fix for the
*wait*; this brief is the fix for the *mystery*. It brings rate limits, cost
bursts and cancellation semantics with it, and it should be measured and argued
in a brief of its own.

**Multiple threads, saved conversations, history.** One thread, one session, the
life of the process. No list of past conversations, no naming them, no reopening
one. The panel's card model is deliberately small and this brief adds exactly one
card to it.

**Rendering markdown in the model's answer.** Text, wrapped, as it arrives. No
headings, no bold, no code-block styling — the same rule the document card has
had since brief 10.

**A second panel for pacting.** Considered and refused. Two panels means two
clocks, two scroll positions, and a reader asking which one to look at every time
a run starts. One thread with two kinds of turn is the smaller idea and the one
that survives the pipeline arriving.

**Streaming the model's answer token by token.** The work lines stream; the
answer lands when it is whole. Partial prose appearing a word at a time is a chat
client's aesthetic, and it costs a redraw per token to say what a `writing` line
already says.

Also out, unchanged from brief 10: an editor in the panel; more than one document
at a time; a preview pane that follows the selection; syntax highlighting and
search within a document; any route from an edit back to green that is not a
pass; telling a document that is behind its code from one that is not. And
unchanged from brief 09: telling direct staleness from inherited staleness;
configurable staleness; cache eviction; configurable caps; enforcement of scopes
against sigils; crossings as records; blessing and the gate; pact expansion
proposals; the journal; the pre-push guard; the CI validator; `warlock status`
and every other non-interactive entry point; the paid crate; and publishing
anything.

## Scope

### 1. The run's own numbers, in the panel

depends_on: []

`PactEvent::Starting` carries `position` and `total` and has since brief 05's
progress port;
`App::set_run_in_flight` already stores both on `InFlight` and words them onto
the footer through `pacting_message`. Nothing new is measured here. The panel
grows a header line for a run in flight — which run, which directory, the
fraction, and a bar filled to it — and it goes away when the run does.

The bar is drawn from the fraction and nothing else. It does not move between
directories, it does not animate, and it does not interpolate against a clock: a
bar that creeps while nothing is happening is an estimate wearing a costume.

Decide here whether the header is a line of the account or a fixed line above it.
Recommendation: fixed above, outside the scrolling window, because it is about
the run rather than about anything in it, and a reader scrolled back through a
long account should not lose the answer to "how far along is this".

### 2. Summarising is most of the wait, so it is most of the lines

depends_on: []

`PactEvent::Summarising { file, part, parts }` reaches the front end already and
lands on the footer as a parenthetical. It is, for any directory holding a file
over 128 KB, the overwhelming majority of the elapsed time. It becomes a line in
the account: the file, the fraction, the clock, in the shape every other account
line has.

The line replaces nothing and hides nothing. A directory with no over-cap file
draws exactly what it draws today, because no such event is sent for it.

### 3. What went in, and what is coming out

depends_on: [2]

Two quantities, both real, both currently thrown away.

`waiting` becomes `waiting · 11 files, 34 KB`: the engine knows exactly what it
put in the request it is about to send, and a reader who knows the size of what
was sent has a reason for the wait. This needs the count to reach the front end,
which is a new field on the starting event or a new observer call — decide which
in this slice, and prefer widening `Starting` to adding a fifth event.

`writing` becomes `writing · 1.8 KB` and climbs, from `--include-partial-messages`
added to the existing `--print --output-format stream-json --verbose`
invocation. This is the flag that also makes `writing` appear at the first token
rather than when the whole assistant message lands, which on a two-minute pass is
most of the silence a reader currently sits through. Handle the new event kind in
the stream reader; ignore the ones that say nothing.

### 4. A composer that takes the keyboard

depends_on: []

A multi-line text field, pure, tested with no terminal, in the shape `prompt.rs`
already established: append, backspace, newline, submit, close, cursor at the
end. One line tall when empty, growing with the text, capped at a maximum, and
scrolling inside itself past that. It draws at the bottom of the panel's column
and the panel above it takes the remaining rows — which is the panel's height
changing every time the composer grows, and `set_panel_height` is already called
every frame for exactly that reason.

The input model is the real work of this slice. `Focus` becomes three: tree,
panel, composer. `action_for` is only consulted when the composer does not hold
the keyboard, and there is a test for every single-letter key asserting it types
rather than acts. Tab cycles; Esc leaves with the draft intact.

Nothing is sent anywhere in this slice. It ends with a field that holds text, a
panel that gets shorter, and every existing key still working when the composer
is not focused.

### 5. The thread

depends_on: [4]

The panel's third card and the agent behind it.

A chat agent is `ClaudeAgent`'s sibling and not a second copy of it: the same
spawn, the same three threads, the same `Cancel`, the same stream reader. What
differs is the argument vector — `--session-id` for continuity, `Read`/`Grep`/
`Glob` for tools, warlock's own system prompt naming what this program is and
what the tree on screen means — and that a turn is a message rather than a
directory.

The card holds turns. A turn is what the reader typed, the work lines the pass
produced as they arrive, and the answer when it lands. The work lines are the
existing `Activity` shape and the existing renderer; this is the first run in
warlock's history that makes a tool call, and the drawing for it was written in
brief 05.

Submitting mutes the composer for that turn: one question at a time. Ctrl-C
cancels the turn.

### 6. A pact is a turn in the thread

depends_on: [5]

The reconciliation, and the slice this brief exists to get right.

Starting a run while the thread is showing does not swap the card, does not open
a second history and does not clear anything. The run's account is written into
the thread as the content of a turn nobody typed, the composer dims for the
duration, and the reader sees one sequence of things that happened in the order
they happened.

The account card does not disappear. Shift-Tab still reaches it, and a reader who
wants the run on its own still has it — the thread is where a run appears when
there is a conversation around it, not a replacement for the record.

What must not change, and what the tests here are for: the request the engine
builds is identical with and without a conversation; the run refuses the keys it
already refuses; a run that fails or is cancelled leaves the thread intact and
the composer live again.

### 7. The footer, once more

depends_on: [4]

The keys line was rebuilt in brief 10 to lay itself out for the width it has, and
this brief hands it three more things to say — the composer's focus, Enter and
Alt+Enter, Esc — while the run line at the bottom keeps saying what it says.

The rule from brief 10 stands: drop from the middle, never the end, so the way
out is the last thing to go. If the line cannot hold the new keys at eighty
columns, the composer's keys are the ones to drop, because a reader who is
looking at a composer has a cursor in front of them telling them it takes typing.
