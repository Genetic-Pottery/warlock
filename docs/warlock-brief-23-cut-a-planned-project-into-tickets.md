# Cut a planned project into tickets

`warlock push` ends at a URL. `.warlock/filed.toml` records brief 22 as project
`65fcabef-373b-4c2e-82bc-3e98fe7accbe`, and that project holds the whole
document — including the `## Scope` section the brief template asks for, six
slices, each `### N.` heading followed by its `depends_on` line. Warlock wrote
those slices, sent them, and has no way to cut one.

Red cuts them. The evidence is committed in this repository:
`.red/030f7dcb317a/state.json` records the same brief as a second Linear project,
`b7ec8be2-5c1c-4f83-bb88-e9af53740875`, with its six slices filed as `WAR-106`
through `WAR-111`. So brief 22 exists twice on the board under one title, and the
copy with the tickets is the one warlock did not file. Red reads a project back
out of Linear, parses the scope, walks the slices blockers-first, and hands each
one to `forman.push.push_interactive`, which drafts the tickets and files them.
All of it runs from `~/.config/forman/.env` — one key, one team, one machine — the
arrangement brief 20 refused by name and replaced with `[[scope]]` records and
machine-local sigils.

Leaving it alone keeps the loop half-built. Warlock names the board, files the
project and records the URL, and then the work it described is cut by another
tool holding another credential, against a team read out of a flat file. The
ledger `.warlock/filed.toml` says a brief was filed and stays silent about
whether anything was ever built from it.

## Outcome

Somebody pushes a brief, opens it in Linear, edits the scope where the slices
are wrong, and moves the project to `Planned`. Back in the panel they type
`/pull`. A dialog names the project, its status, the six slices, the team `WAR`
and the key `warlock`, with No under their finger. They press Left and Enter.

The thread then works one slice at a time. It says
`slice 1 of 6: The Linear client`, and a session reads the repository and drafts
the tickets for that slice alone. When the brief does not settle something the
draft needs, the question lands on the thread in the words it was asked. Under it
warlock puts its own attempt at the answer, read out of the brief and the
repository, and the composer holds that text with the cursor at the end: Enter
sends it, and typing replaces it. When the brief covers none of it warlock says so
in those words instead of guessing, and the operator is the only one who can
answer. Three rounds at most, after which the session drafts with what it has. The
drafts land as titles on the thread, and a window asks to create them, skip the
slice, or take feedback typed into the composer as a redraft.

Each created issue lands in the team's `Backlog`, labelled `warlock`, attached to
the project, and blocked by the issues of every slice its `depends_on` names. The
thread says `WAR-112, WAR-113`. When the last slice is settled warlock comments on
the project naming what it filed, and the project is still in `Planned`, because
moving it is nobody's job but the operator's.

At a shell the same project is cut with
`warlock pull docs/warlock-brief-22-push-a-brief-to-the-board-the-sigil-names.md`,
which asks nothing: each slice drafts in one shot and files. `--dry-run` prints
the project, the status, the slices and their order, and opens no socket.

A project still in `Backlog` refuses, naming the status it found and the status it
wants. Run `pull` again after a run that filed three of six slices and only the
other three are offered.

## Success criteria

**Reading the project back**

- The brief's path resolves to a project through `.warlock/filed.toml`. A path
  the file does not record refuses and names `warlock push`.
- The project is read by id in one query: its name, `content`, `url` and
  `status { name }`. No walk of the workspace, and no match on a name.
- A project whose status is not `Planned` refuses, naming both statuses, and
  nothing else is read or sent.
- The status name is compared trimmed and case-insensitively, and `Planned` is
  the only spelling warlock accepts.
- A project id the API does not know is a refusal naming the id and
  `.warlock/filed.toml`.

**The slices**

- The content splits at the first `## Scope` heading: everything above is the
  brief, everything below is the scope block.
- A slice is a `### ` heading with an optional leading `N.` or `N)`, an optional
  `depends_on:` line as its first non-blank line after the heading, and prose to
  the next heading. Everything through `for_the_board` survives this round trip —
  reflowed paragraphs, fenced blocks and `- [ ]` bullets included.
- A slice's number is its position in the document, not the number it was written
  with, and `depends_on` references resolve through the written numbers to
  positions. A reference to a slice that is not there, or to itself, is dropped.
- Slices run in dependency order, ties broken by position as a number. Ten slices
  order 9 before 10.
- A cycle in `depends_on` refuses and names the slices in it. Nothing is drafted
  from a scope warlock cannot order.
- A heading warlock cannot read as a slice is counted and named on the thread. A
  scope block yielding no slices refuses.

**Drafting a slice**

- A slice is drafted by one session that sees the brief above the scope, the one
  slice, and the repository through `Read`, `Grep` and `Glob`. It writes nothing.
- The session answers with one JSON object or with prose. Prose is a question;
  the object is the drafts, and it ends the asking.
- Three question rounds, then one final instruction to draft with what it has.
- The object holds an array of drafts, each with a title, a body, and
  `blocked_by` and `blocks` as indices within this slice. Every field has a cap,
  every cap is written into the prompt, and the answer is checked against them.
- A defective answer is repaired rather than refused, by the rules brief 16 set:
  one line where one line was asked for, cut to the cap, list truncated, missing
  filled from the slice itself. Each repair is named on the thread.
- An answer that is not JSON is retried up to `ATTEMPTS`, and the last attempt's
  defects are listed back to the session as defects not to repeat.
- A reference outside the slice's own draft list is dropped, not resolved against
  another slice.
- Nothing about the drafting reaches the engine's `Agent` port: the port takes a
  request and gives an answer, and a session that asks questions is a
  conversation.

**Answering a question**

- A question puts warlock's own attempt at the answer into the composer as an
  ordinary draft: the cursor sits at the end, every editing key works on it, Enter
  sends whatever the composer holds, and clearing it and typing sends that.
- The attempt comes from one read-only session over the brief, the slice and the
  repository. It writes nothing and it never reaches Linear.
- A question the brief and the repository do not settle produces one fixed
  sentence saying so, not a guess. That sentence goes on the thread and the
  composer is left empty, because a sentinel nobody meant to send is worse than an
  empty line.
- A failed or timed-out attempt is a line on the thread and an empty composer. The
  slice's session stays alive and the operator answers it themselves.
- The thread shows the question and what was sent, marked apart from each other,
  so reading back says who answered.
- Drafting an answer is bounded the way every other session is: its own timeout,
  and no second attempt if it fails.

**What is filed**

- One issue per draft, on the record's team, attached to the project by id,
  carrying the record's `label`, in the team's workflow state named `Backlog`.
- A team with no workflow state named `Backlog` refuses before any issue is
  created, naming the team.
- The record's `label` resolves against the team's issue labels and is created
  there when absent. Project labels are a different type on the wire and brief
  22's resolver does not answer for issues.
- Relations are written after every issue in the slice exists: the drafts'
  `blocked_by` and `blocks` within the slice, and one edge from every issue of
  each slice named in `depends_on` to every issue of this slice.
- A relation the API refuses is a line on the thread and does not fail the slice.
  An issue that exists with a missing edge is recoverable; a lost identifier is
  not.
- Warlock never writes the project's status, an assignee, a priority, an
  estimate, a cycle or a milestone.
- After the last slice settles, and only when something was filed, warlock
  comments once on the project naming every issue it created and saying the status
  was not moved. A comment that fails is a line on the thread, not a failure.

**The cut record**

- Each filed slice is recorded against its brief in `.warlock/filed.toml`: the
  slice's title, the title folded to a key, the issue identifiers and the time.
- A slice is matched to its record by the folded title, not by position, so
  reordering the scope in Linear does not re-cut anything.
- A record whose key is no longer in the project is kept, and named on the thread
  as filed but gone.
- A slice with a record is never offered again. A project whose every slice has a
  record refuses before anything is read from the repository, and says so.
- The record is written after each slice, not once at the end, so a run that dies
  on slice four does not re-file slices one to three.
- `filed.toml` moves to version 2. Version 1 reads as a brief filed with nothing
  cut; version 2 is what warlock writes. Every other promise brief 22 made about
  this file holds: the atomic write, and records warlock did not add coming back
  byte-identical.
- The panel's `/push` path checks the record before sending, the way
  `push.rs:88` already does and `pushing.rs:427` does not. A second writer to this
  file is the wrong moment to leave a known double-file open.

**`warlock pull`**

- `warlock pull <PATH>` takes the brief's path, required, and `--scope <NAME>`
  picks among candidates, as `push` does.
- No questions at a shell. Each slice drafts in one shot, under a contract that
  says so, and files.
- `--dry-run` prints the project, its status, the slices in order and the ones
  already cut, opens no socket and writes no record.
- Progress prints one line per slice as it is drafted and one naming what it
  filed, in the shape `running.rs`'s `Progress` already prints passes.
- Every refusal is the ordinary **1**. The boundary's **3** is not spent on any of
  them.
- `pull` takes no `--json`, matching `push` and the other writing subcommands.
- `HEADLESS-CLI.md` documents the subcommand, the flag, every refusal and the exit
  status, in the table and in a section of its own.

**`/pull` in the panel**

- `/pull` is a fifth command, and `Submitted::refusal`'s one sentence names all
  five.
- `/pull` pulls the brief `/write` wrote in this session, or the path typed after
  it. A session that has written nothing and named nothing refuses and names
  `/write`.
- The dialog names the project, its status, the slice count, the team and the key
  name, and answers No to Esc and to an immediate Enter.
- While a slice's session is in flight, the composer answers its question instead
  of starting a chat turn, and the panel says which slice it is answering for. A
  question arrives with warlock's attempt already in the composer.
- The review window offers create, skip and feedback. Skip records nothing and
  moves to the next slice. Feedback redrafts the same slice once more.
- Skipping a slice asks whether to carry on with the rest, and a No leaves the
  remaining slices unoffered.
- The run drives off the event loop, the way a pass and a push do. Frames keep
  drawing, one pull is in flight at a time, and quitting cancels the session in
  flight.
- Every identifier, every repair and every refusal lands on the thread. A failure
  lands there in one line and tears nothing down.

## Constraints

- No new dependency. The socket is `linear.rs`'s `Posts` and `ureq`, the
  subprocess is `claude.rs`, and both already carry everything this needs.
- `warlock-engine` gains no HTTP and no Linear vocabulary. The drafting contract —
  the JSON shape, its caps, its check, its repair and its prompt text — is the
  engine's, exactly as `document.rs` owns the document contract. `issueCreate` is
  the TUI's.
- No async runtime, and no second long-lived session. `claude.rs` states the rule
  and names the three deadlocks it avoids.
- A key reaches the `Authorization` header and nowhere else. Only key names are
  printed, as brief 20 settled.
- Linear personal keys take the key as the bare `Authorization` value, with no
  `Bearer` prefix.
- Warlock does not move a project's status, in either direction. `Backlog` to
  `Planned` is the operator saying they have read it, and anything after that is
  theirs too.
- The record's `review_state` stays unread. It names the state a finished ticket
  moves to, which is a later brief's business; nothing here files work into it.
- One request per operation, one timeout, no retry and no backoff. A retry around
  a create is the shape of mistake that files two issues.
- `.warlock/pacts.toml` is not written by any of this.
- Existing command spellings, `--json` envelopes and exit statuses do not move.
- The comment rules in `CLAUDE.md` apply. The two label types on the wire earn a
  comment; a walk through the slice loop does not.
- The repository root carries the `warlock-team` scope, so every part of this work
  is under it. Confirm what this machine holds with `warlock config` before
  starting.

## Out of scope

- **Moving the project's status.** Red locks this and says why in its module
  docstring: the statuses are how a person tells warlock what they have decided,
  and a tool that writes them is a tool that can tell you it has your permission.
  The comment at the end of a run is how warlock says what it did.
- **Rewriting or reconciling issues already filed.** A slice edited in Linear
  after its issues exist is a slice whose tickets are edited where they are.
  Warlock would be the second writer of a document people are working in, and the
  one that overwrites without reading.
- **Editing the drafts before they are filed.** Red offers `$EDITOR` on a temp
  file. Feedback redrafts, and an issue is editable in Linear the moment it
  exists, so the round trip buys one revision of prose nobody has read twice.
- **Making the issues `forman pull` will pick up.** Forman starts only tickets
  carrying its own provenance label, which defaults to `forman`, and the record
  here says `warlock`. Pointing `FORMAN_LABEL` at the record's label is one line
  of machine configuration and not warlock's to write.
- **Executing a ticket.** Warlock files and records. Decomposing a ticket, running
  agents on it and opening a pull request is what Forman is, and warlock is not
  becoming an agent runner.
- **Choosing the project by searching the workspace.** Red lists every project and
  filters on status in Python, and refuses when two share a name.
  `.warlock/filed.toml` already holds the id the push returned, which is one query
  and cannot match the wrong project.
- **Question rounds at a shell.** A pipeline that blocks on a model's question is
  a pipeline that hangs. The CLI drafts in one shot, as Forman's own
  non-interactive path does, and the panel is where a question has somebody to
  reach.
- **Estimates and priorities.** Forman asks its model for t-shirt sizes and then
  drops every one of them, because the field it writes takes an integer. Either
  half of that is a field warlock would be inventing.
- **A `--json` envelope on `pull`.** The writing subcommands take none, and what
  is worth parsing is in `.warlock/filed.toml`.
- **Fixing `warlock check`'s review-state line.** It reads `work here is filed to
  WAR, as In Review, labelled warlock`, and after this brief nothing is filed as
  `In Review`. The sentence is wrong and the brief that gives `review_state` a job
  is where it gets rewritten, with the behaviour that makes it true.

## Scope

<!-- red:scope -->
### 1. Reading a project back

depends_on: []

The query and the gate: `project(id:)` for the name, `content`, `url` and
`status { name }`, and the refusal for a project that is not in `Planned`. It goes
beside brief 22's four operations in `linear.rs`, against the same `Posts` seam,
so no test in this slice opens a socket.

The decision is that the path names the project through `.warlock/filed.toml`
rather than through a search. Red searches because it has nowhere to look;
warlock recorded the id when it filed the project, which makes the lookup exact
and makes a second project with the same title — the situation this repository is
already in — something warlock cannot walk into.

<!-- red:scope -->
### 2. The scope block as slices

depends_on: []

Splitting the content at `## Scope` and parsing the block into ordered slices,
beside `brief.rs` as the inverse of `for_the_board`. Positions win over written
numbers, references resolve through the written numbers, ties break numerically,
and a cycle refuses.

Two decisions. A heading warlock cannot read is counted and named rather than
dropped in silence — Red's parser drops them deliberately and its own docstring
says the cost, that a project which fails to parse gives the person no way to see
why. And a cycle refuses here where Red falls back to document order: a brief
whose slices depend on each other in a loop is a brief with a mistake in it, and
filing tickets in an arbitrary order hides it.

<!-- red:scope -->
### 3. The drafting contract

depends_on: []

The engine-side shape: the struct a draft session fills, its caps, its `check`,
its place in `mend`'s `Slot` arms, and the prompt text that states the shape and
the caps. It mirrors `document.rs` field for field, including the untagged
deserialiser that turns a bare string where an object was asked for into a
repairable defect rather than `NotJson`.

This slice decides that ticket drafting is held to the same bargain as document
writing: the model fills a JSON object, warlock validates it, repairs what it can
from what it already measured, and lays out the result itself. The alternative —
taking the session's prose as the ticket body — is the thing brief 15 removed from
the document road.

<!-- red:scope -->
### 4. The drafting session

depends_on: [3]

One conversation per slice: the system prompt, the opening that carries the brief
and the one slice, the rule that prose is a question and an object is the drafts,
the three rounds, and the one-shot contract the CLI uses instead. It sits beside
`claude.rs`'s `ChatAgent`, at `BRIEF_MODEL` and `BRIEF_EFFORT` through
`Converses::raised`, because drafting a ticket set is the same register as
writing the brief it comes from.

The decision is that this is not a `Mode`. `panel.rs` states what a mode is — a
register of the one chat session, not a second session and not a second system
prompt — and a pull runs a fresh session per slice under a different prompt with a
different job. So the question relay is a state of the pull in flight, the way
`Pushes::sending` is a state of a push, and the composer routes to it while it
lives.

<!-- red:scope -->
### 5. The answer warlock proposes

depends_on: [4]

One read-only session over the brief, the slice and the question, and the fixed
sentence it returns when none of them settle it. Its answer goes into the composer
as a draft rather than onto the thread as text to accept blind.

Two decisions. The proposal is a draft and not a prompt: warlock has had a cursor
in the composer since brief 14, so the operator edits a sentence they can see
instead of pressing Enter on words scrolled up the page — which is also why Red's
`--verbatim` has no counterpart here, since deleting the draft is the same
keystroke. And a question nothing covers gets the sentence rather than the
model's best effort, because an answer invented from nothing is the one kind of
answer that reaches Linear looking like a decision somebody made.

<!-- red:scope -->
### 6. Filing the issues

depends_on: []

`issueCreate`, the team's workflow state named `Backlog`, the issue-label
resolver, `issueRelationCreate` for both the within-slice edges and the
cross-slice ones, and the closing `commentCreate` on the project.

Issue labels and project labels are different types with different queries, and
brief 22's `label_id` answers only for projects. That is a wire fact and the one
thing in this slice worth a comment. The relations go after every issue in the
slice exists, for the reason Forman writes them the same way: an identifier that
was never printed is unrecoverable, and a missing edge is a line somebody adds.

<!-- red:scope -->
### 7. The cut record

depends_on: []

`.warlock/filed.toml` grows a per-slice record under each filed brief — title,
folded key, identifiers, time — at version 2, reading version 1 as a brief with
nothing cut. Saved after each slice rather than at the end. This slice also closes
the panel's missing already-filed check.

The decisions are that the cut belongs in the brief's own record rather than in a
second file, because a separate file would need the project id as a foreign key
and would give two files the chance to disagree about one brief; and that identity
is the folded title, as Red has it, so that reordering the scope in Linear re-cuts
nothing and a retitled slice is honestly a new one.

<!-- red:scope -->
### 8. `warlock pull`

depends_on: [1, 2, 4, 6, 7]

The subcommand, `--scope`, `--dry-run`, the per-slice progress on stdout, the
refusals and the documentation. It follows the conventions the other headless
verbs set: resolve the environment once through `Standing`, return a `Result`, let
`main` print the failure, and add the new `Error` variants to `status_for` in the
ordinary **1**.

The decision is the one-shot contract. A shell gets no questions, so the CLI is a
pipeline step and never a thing that waits for somebody who is not there.

<!-- red:scope -->
### 9. `/pull` in the panel

depends_on: [1, 2, 4, 5, 6, 7]

The fifth slash command, the one sentence that lists them, the confirmation
dialog, the question relay through the composer, the per-slice review window, and
the whole run driven off the event loop with a cancel on quit.

The decision is that the panel is where the questions are answered and the drafts
are gated, because it is the only place warlock has somebody's attention on this
brief. The dialog answers No to Esc and to a reflex Enter, as brief 22's does, and
the run reports every identifier, repair and refusal on the thread rather than
anywhere a person would have to go and look.
