# Pull the next ticket in a scope and work it to a pull request

`warlock draft` ends at a set of issues. Each one sits in the team's `Backlog`,
labelled with the scope record's `label`, attached to the project and blocked by
the issues its slice depends on. Warlock has no command that turns one of those
issues into code. The workflow is brief, draft, pull, and the third step does
not exist.

Forman is the only thing that executes a ticket today, and it misses these
issues on two counts. `forman pull` selects only issues assigned to the person
running it and carrying its own provenance label, `forman` by default.
`create_issue` in `crates/warlock-tui/src/linear.rs` sends no assignee, and the
scope record says `warlock`. So Forman never sees a single issue that `draft`
files.

Forman also has no idea what a scope is. It reads one key, one team and one label
from `~/.config/forman/.env`, which is the arrangement brief 20 replaced with
`[[scope]]` records and machine-local sigils. A Forman session pulled for
`data-plane` work can edit a `control-plane` directory, commit the edit and open
the pull request, and nothing on the way notices. The boundary warlock draws is
real in the tree and at `p`, `r` and `s`. It isn't real where code is written,
which is the one place it was drawn for.

Brief 23 ruled execution out by name: "warlock is not becoming an agent runner."
This brief overturns that on purpose. Executing a ticket is where the scope
boundary has to hold. An agent runner that can't read the boundary ignores it,
and the only runner that can read it is warlock.

## Outcome

Somebody writes a brief, pushes it, and cuts it with `/draft`. Every issue lands
assigned to them. In the panel they type `/pull warlock-team`. A dialog names
the ticket warlock chose, `WAR-131 The Linear queue query`, with the scope, the
team, the branch it creates, `war-131/the-linear-queue-query`, and No under
their finger. They press Left and Enter.

The ticket moves to `In Progress` on the board, and the panel turns into the
run's output window, the account card a pact writes to. Its first section is the
split, which ends in three sub-tasks. Each sub-task gets a section of its own,
`WAR-131.01 Add the issues query beside fetch_project`, with the session's
reads, greps, edits and test runs logged under it as they happen. The composer
doesn't start a chat while this runs; there is nothing to say to a model while
another one is editing the tree. After each finished sub-task warlock commits,
and the thread records the commit. After the last one it refreshes the `WARLOCK.md` of
every pacted directory the branch made stale and commits those separately. Then
it pushes, opens the pull request and comments the link on the ticket. The
ticket moves to `In Review`, and the thread shows the URL.

Partway through a different ticket, a session tries to edit
`crates/control/src/lib.rs`. That directory is scoped `control-plane`, and this
machine holds only `warlock-team`. The edit is refused before it lands, and the
session reports itself blocked. Warlock checks the working tree and finds
nothing outside what it holds. It works the sub-tasks that don't depend on the
blocked one, then halts. The ticket gets a comment naming the path, the
scope that covers it and the sigil the work wants. It stays `In Progress`.
`~/.warlock/<project>/pulls/WAR-140/manifest.md` shows which sub-tasks finished
and which one was stopped, and why.

The operator decides what to do. They can take the `control-plane` part to
whoever holds that sigil, or hold it themselves and record it with
`warlock config`. Until they decide, `/pull warlock-team` passes `WAR-140` over
and names it as halted, so the rest of their queue keeps moving. Once the
blocker is dealt with, they run `/resume WAR-140`. It puts the stopped sub-task
back to pending and says so. Then `/pull warlock-team` takes `WAR-140` before
any new ticket and carries on from the sub-task that stopped, or
`/pull warlock-team WAR-140` names it outright.

At a shell, `warlock pull warlock-team` does the same with no dialog and prints
one line per step. `warlock pull warlock-team --dry-run` prints the ticket it
would take, and every assigned ticket it passed over with the reason. It touches
nothing and runs no model.

When nothing is ready, pull says so. It names what it found, what blocks it and
who holds the blocker, then exits 0.

## Success criteria

**Draft assigns what it files**

- Every issue `draft` creates, from the panel and at a shell, is assigned to the
  user the scope's bound key belongs to. That user comes from one `viewer { id }`
  query per run.
- The assignee is the only field `draft` gains. Priority, estimate, cycle and
  milestone stay unwritten, and `NewIssue`'s doc comment is rewritten to say why
  the assignee is the exception: the assignee is the claim `pull` reads.

**Starting a pull**

- `warlock pull <SCOPE>` and `/pull <SCOPE>` take the scope name as a required
  argument. Leaving it off is a clap error, exit 2, at a shell, and a refusal
  naming the scopes this machine holds in the panel.
- A scope name that no `[[scope]]` record in `.warlock/pacts.toml` holds refuses
  and names the recorded scopes.
- A scope this machine's sigils don't open refuses with exit 3, naming the scope
  and the sigils held, before any request is made. `scope_opens_to` in
  `crates/warlock-engine/src/scope.rs` answers the question, as it does for `r`.
- A scope with no key bound to this checkout refuses the way `push` and `draft`
  refuse, naming `warlock key use`.
- A dirty working tree refuses and names what is dirty. Pull never stashes,
  resets, or cleans.
- The default branch is detected, never assumed to be `main`.

**Choosing the ticket**

- The queue is every issue on the scope record's `team` that carries the record's
  `label`, is assigned to the key's user, and whose state type is neither
  `completed` nor `canceled`. It is read in one query.
- The query carries each issue's blocking relations and the state type of each
  blocker. An issue is ready when every issue that blocks it is `completed` or
  `canceled`, whoever it is assigned to and whatever label it carries.
- An issue in the workflow state named by the record's `review_state` is waiting
  on a human and is never selected.
- An issue in `In Progress` is selected only when this machine holds a run record
  for it. Otherwise it is skipped and named as in progress elsewhere.
- A run record for the scope whose status is `resumed` is taken before any issue
  without a record. Two or more resumed runs are taken in the queue's order.
- A run record whose status is `halted` is skipped and named as halted, with the
  `warlock resume <TICKET>` that releases it.
- Ready issues are ordered by Linear priority (urgent, high, medium, low, then
  none), then by how many issues in the queue each one blocks, then by the number
  in its identifier as a number, so `WAR-9` comes before `WAR-10`.
- Selection is a pure function over what the query returned, and it is tested
  without a socket.
- The query reads one page with a cap held in a constant. When the page is full,
  the output says the cap was reached.
- When nothing is ready, the output names each skipped issue and why: blocked by
  which identifier and whose it is, in review, in progress elsewhere, or halted.
  This exits 0.

**Naming a ticket**

- `warlock pull <SCOPE> --ticket <TICKET>` and `/pull <SCOPE> <TICKET>` take that
  ticket instead of choosing one.
- A named ticket still has to be in the scope's queue: on the record's team,
  carrying the record's label, and assigned to the key's user. A ticket that
  fails any of the three refuses and names which one. Naming a ticket picks
  among your own work. It never reaches into somebody else's.
- A named ticket that is blocked refuses and names each open blocker and whose
  it is. One in the review state refuses and names the state. One whose run is
  `halted` refuses and names `warlock resume <TICKET>`.

**Resuming**

- `warlock resume <TICKET>` and `/resume <TICKET>` put every `failed`, `blocked`
  and `crossed` sub-task of that ticket's run back to `pending`, and set the run's
  status to `resumed`. Resuming is the operator saying they have looked. `done`
  sub-tasks are never touched.
- `--failed-only` resets only `failed` sub-tasks, for the case where a blocker is
  still unresolved and a transient failure is not.
- Resume prints each sub-task it changed with its old status, then the command
  that picks the run up: `warlock pull <SCOPE> --ticket <TICKET>`.
- Resume writes the run record and nothing else. It makes no request, runs no
  git command and moves no ticket, so it is not gated by the scope. The pull that
  follows is.
- A ticket with no run record on this machine refuses and names the directory it
  looked in. A run with nothing to reset refuses and names the run's status.
  Both exit 1.
- Taking a resumed run checks out the run's branch and requires a clean tree.
  That is how a halted sub-task's uncommitted work gets a human decision rather
  than a silent commit.

**The board**

- On start, the ticket moves to the team's workflow state named `In Progress`,
  matched trimmed and case-insensitively. It moves before the split runs, so the
  board is honest while the model works. A team with no such state gets a line
  in the output, and the run goes on.
- On finish, the ticket moves to the state named by the record's `review_state`,
  matched the same way. A missing state is a line, not a failure.
- On a halt, the ticket stays where it is and gets one comment. The comment lists
  the done sub-tasks, then each one that stopped with its status and reason, then
  the ones never started. It ends with the two commands that carry the run on:
  `warlock resume <TICKET>`, then `warlock pull <SCOPE> --ticket <TICKET>`.
- Pull never merges, never moves a ticket to a completed state, never writes a
  project's status, and never changes an assignee.

**The run record**

- Each run lives at `~/.warlock/<project>/pulls/<TICKET>/`, the per-checkout
  directory `sigils.rs` already names. It holds `state.json`, `manifest.md` and
  one `<TICKET>.NN.md` brief per sub-task.
- `state.json` is the source of truth. It records the ticket identifier and
  title, the scope, the branch, the start time, the pull request URL, the status
  and the sub-tasks. `manifest.md` is rendered from it on every save and is never
  read back.
- A run's status is `pulled`, `in_progress`, `halted`, `resumed` or `in_review`.
  Only `resume` moves a run from `halted` to `resumed`, and only a pull moves it
  on from `resumed`. A
  sub-task's status is `pending`, `in_progress`, `done`, `blocked`, `failed` or
  `crossed`, and every status except `pending`, `in_progress` and `done` carries
  a reason.
- Each sub-task records its session id and its cost when the stream's `result`
  message carries them, for every outcome and not only `done`.
- The record is saved before and after every sub-task, so a run killed at any
  point resumes from its last save.
- Nothing about a run is written inside the repository.

**Splitting the ticket**

- One read-only session splits the ticket into between one and eight sub-tasks.
  It sees the ticket's title and description and the repository through `Read`,
  `Grep` and `Glob`, and it writes nothing.
- The session fills one JSON object. Each sub-task has a goal, `depends_on` as
  1-based indices, a definition of done, likely files, a test plan and notes. Only
  the goal is required, every field has a cap, and every cap is written into the
  prompt.
- The contract, meaning the shape, its caps, its check, its repair and its prompt
  text, is the engine's, as `drafting.rs` owns the ticket-drafting contract.
  Defects are repaired by the rules brief 16 set, and each repair is named in the
  output.
- Sub-tasks are topologically sorted, then numbered `<TICKET>.01`, `.02` and so on,
  so reading the manifest top to bottom is a legal order. An unresolvable
  `depends_on` reference is dropped. A cycle halts the run and names the
  sub-tasks in it.
- A split that fails halts the run with a comment on the ticket. No branch work
  is lost, because none has been done.

**Working a sub-task**

- Each sub-task runs in a fresh `claude` session that knows nothing of any other
  session. It receives its own brief, the ticket's title and description as
  context and not as a to-do list, and the summaries of finished sibling
  sub-tasks. It never sees the orchestrator's history.
- The session's system prompt names the scope the ticket was pulled under and the
  sigils this machine holds. It says that a write refused by the hook means
  reporting `blocked` with the refusal as the reason, and that routing around a
  refusal is not an option.
- The session may use `Read`, `Grep`, `Glob`, `Edit`, `Write` and `Bash`. Its tool
  set is restricted with `--tools`, and the same list goes to `--allowedTools` so
  that nothing prompts. In print mode, a prompt is a denial.
- The session is told not to commit, push, switch branches or rewrite history.
  Warlock checks that `HEAD` hasn't moved after every session. A moved `HEAD`
  halts the run as `failed` and names the commit.
- The session's MCP servers and machine settings are kept out, as Forman keeps
  them out, so a session cannot reach Linear or anything else except through
  warlock.
- The session answers with one JSON object as its last message:
  `{"status": "done" | "blocked" | "failed", "summary": "...", "blocked_reason": null | "..."}`.
  Missing or malformed JSON is `failed`, with the raw text kept.
- A sub-task session runs under a turn limit and a timeout of its own, each
  held in a named constant. `INVOCATION_TIMEOUT`'s five minutes is sized for a
  pass, not for a session that edits code and runs tests.
- Only `failed` is retried, twice in total at most. A turn limit gives the retry
  twice the turns. A usage limit, a rate limit or a bad credential isn't
  retried. `blocked` and `crossed` are never retried. The retry runs in the
  working tree the failed attempt left, and its prompt says so.
- A sub-task that finishes `done` and passes the diff check is committed with
  `git add -A` and the message `<TICKET> <TICKET>.NN: <goal>`.
- Sub-tasks run serially. The run continues past a `blocked` or `failed` sub-task
  to any sibling that doesn't depend on it, and halts when nothing is runnable.
  A `crossed` sub-task halts the run immediately, because the tree now holds
  work that must not be built on.

**The boundary while a session writes**

- `warlock check` gains `--gate`. With a path, `--gate` exits 3 when the scope
  covering the path doesn't open to this machine, and 0 when it does.
- With no path, `warlock check --gate` reads a Claude Code `PreToolUse` payload on
  stdin, takes `tool_input.file_path`, and answers on stdout with
  `{"hookSpecificOutput": {"hookEventName": "PreToolUse", "permissionDecision": "deny", "permissionDecisionReason": "..."}}`
  when the scope is closed. The reason names the path, the covering scope and the
  sigil the write wants. It exits 0 in both cases.
- Every sub-task session carries a `PreToolUse` hook on `Edit`, `Write`,
  `MultiEdit` and `NotebookEdit` that runs `warlock check --gate`. The hook goes
  in on the invocation with `--settings`, and no file on disk is written for it.
- After every sub-task session, before any commit and whatever the session
  reported, warlock reads the working tree's changes with
  `git status --porcelain=v1 -z --untracked-files=all`, taking both sides of a
  rename. It asks `scope_covering` and `scope_opens_to` about each path.
- Any changed path under a scope this machine doesn't hold makes the sub-task
  `crossed`. The reason names every such path with its scope. Nothing is
  committed, the tree is left as it is for the operator, the ticket gets the halt
  comment, and the CLI exits 3.
- A path under a scope this machine does hold, other than the one the ticket was
  pulled under, isn't a crossing. The pull request body names each such scope
  and the paths under it.
- A path under no scope, or in an unpacted directory, is open.

**Freshness before the pull request**

- After the last sub-task is committed, warlock refreshes every pacted directory
  that is stale and has a path the branch changed at or below it. It uses the
  same run that `warlock refresh` makes, children before parents.
- Each directory passes the same boundary check as `r`, through
  `boundary::permits` with `Operation::Refresh`. A directory closed to this
  machine isn't refreshed, and the pull request body names it as stale.
- A failed pass is named in the pull request body and doesn't stop the pull
  request.
- The refreshed documents and `.warlock/pacts.toml` go in one commit of their
  own, `<TICKET>: refresh WARLOCK.md`, so a reviewer can read the code change
  without the document churn. No refresh commit is made when nothing was stale.

**The pull request**

- The branch is `<team-key lowercased>-<number>/<title slug>`, cut from the
  default branch after a fast-forward-only pull.
- Warlock pushes the branch and opens the pull request with `gh pr create`
  against the default branch. The title is `<TICKET>: <title>`. The body gives
  the ticket's description, each sub-task with its summary, the scopes crossed
  while held, the directories left stale and why, and a line saying that the
  pull request and the ticket's review state are the human gate.
- Without `gh` on the path, warlock comments the body on the ticket and names
  the branch. The run still counts as finished.
- The pull request URL is commented on the ticket and recorded in `state.json`.
  The run's status becomes `in_review`.

**`warlock pull`**

- `warlock pull <SCOPE>` takes `--ticket <TICKET>` and `--dry-run`, and nothing
  else. The dry run reads the queue and the run records, and prints the ticket it
  would take and every skipped ticket with its reason. With `--ticket`, it says
  whether that ticket would be taken or why not. It makes no write and no git
  command, and it runs no model.
- Progress prints on stdout in the shape `running.rs`'s `Progress` prints passes:
  a header per section (split, each sub-task, each refreshed directory, the pull
  request), then the session's activity lines under it.
- The exit status is 0 when a pull request was opened or nothing was ready, 3
  when the scope is closed at the start or a sub-task crossed, and 1 for every
  other refusal and halt.
- `warlock resume <TICKET>` takes `--failed-only` and nothing else, and exits 0
  when it changed something.
- `pull` and `resume` take no `--json`, matching `push` and `draft`.
- `HEADLESS-CLI.md` documents both subcommands, their flags, the run record's
  location, every refusal and the exit statuses, in the table and in a section
  of their own. It documents `check --gate` in `check`'s section.

**`/pull` and `/resume` in the panel**

- `/pull <SCOPE> [TICKET]` and `/resume <TICKET>` are the sixth and seventh slash
  commands, and `Submitted::refusal`'s one sentence names all seven.
- `/resume` behaves as `warlock resume` does. It prints its changes on the
  thread, and the composer offers `/pull <SCOPE> <TICKET>` as a draft, with
  the cursor at the end.
- `/resume` refuses while a pull is in flight.
- The `/pull` dialog names the ticket, its title, the scope, the team and the
  branch. For a resumed run, it says so and names the sub-task the run carries on
  from. It answers No to Esc and to an immediate Enter.
- The run drives off the event loop the way a pass and a draft do. Frames keep
  drawing, and one pull is in flight at a time.
- The run's output is the account card, the one a pact or refresh writes to. The
  panel switches to it when the pull starts. The account has one section each
  for the split, each sub-task (headed by its id and goal), each directory the
  refresh passes, and the pull request. Each section logs the session's thinking,
  tool calls, writing and cost through `Account::record`, the way a pass's
  section does.
- While a pull is in flight, the composer starts no chat turn. Submitting puts
  one line on the thread naming the pull in flight and how to cancel it. The
  thread card stays readable.
- The thread gets only milestones: the ticket taken or resumed, each commit, each
  halt or crossing, and the pull request URL. Tool activity never lands on the
  thread.
- Each sub-task's activity log is also appended under the execution log heading
  of its `<TICKET>.NN.md` in the run record, so it is still readable after the
  panel closes.
- While a pull is in flight, `p`, `r`, `s`, `/draft` and `/resume` refuse and
  name the pull.
  A pass hashing directories that a session is editing records a digest nobody
  can trust.
- Quitting cancels the session in flight through a `CancelGuard`. The sub-task is
  recorded `failed` with the reason `cancelled`, and the run is `halted`. The tree
  is left as it is, and the ticket gets no comment, because the operator is the
  one who stopped it. `/resume` releases it like any other halt.
- The panel asks no questions during a pull. A sub-task that needs a decision
  reports `blocked`, and the run halts.
- Every identifier, commit, crossing, repair and refusal lands on the thread. A
  failure lands there in one line and tears nothing down.

**`warlock check`'s review line**

- The line that reads `work here is filed to WAR, as In Review, labelled warlock`
  is rewritten to be true: work is filed to the team with the label, and a
  finished pull moves it to the review state. Brief 23 left this line for the
  brief that gives `review_state` a job, which is this one.

## Constraints

- No new crate. `git` and `gh` are subprocesses run the way `claude.rs` runs
  `claude`, with the same reader threads and the same `try_wait` polling, so
  none of the three deadlocks its module doc names comes back.
- `warlock-engine` gains no git, no HTTP and no Linear vocabulary. The split
  contract and the sub-task result contract are the engine's, as `document.rs` and
  `drafting.rs` own theirs. Selection over a queue is pure and can live in the
  engine. Every request and every subprocess is the TUI crate's.
- The orchestration loop runs no I/O of its own. Linear, git, the run record, the
  split, the sub-task session and the diff check each sit behind a seam. The whole
  run is tested against fakes with no socket, no repository and no model call, as
  `Posts` and `Converses` are tested today.
- No async runtime. One model session is alive at a time.
- `tests/claude.rs` asserts that no session is given a writing tool. That stays
  true of every session kind that exists before this brief, and the sub-task
  session is the one named exception, with a test that it gets exactly the six
  tools listed above.
- A key reaches the `Authorization` header and nowhere else. It is never passed to
  a `claude`, `git` or `gh` child, in arguments or in the environment.
- One request per Linear operation, one timeout, no retry. A retried create is how
  a comment gets posted twice.
- Pull never force-pushes, never deletes a branch, never stashes, never resets
  and never cleans.
- `.warlock/pacts.toml` is written only by the refresh pass, through the code that
  writes it today.
- The `In Progress` state name is a fixed constant.
- Existing command spellings, `--json` envelopes and exit statuses don't move.
  `check` without `--gate` behaves exactly as it does today.
- The comment rules in `CLAUDE.md` apply. A few things earn a comment: exit 2 is
  the only exit that blocks a Claude Code hook, and 3 does not; the diff check
  is the authority because `Bash` writes past the hook; and a `crossed` sub-task
  halts rather than letting siblings run.
- It isn't known whether hooks passed with `--settings` load when
  `--setting-sources ""` is also passed. The Claude Code documentation doesn't
  say. The slice that builds the sub-task session establishes it with a test
  against the real CLI before relying on it. If it doesn't hold, the session
  keeps the hook and gives up `--setting-sources`, not the other way round.
- The repository root carries the `warlock-team` scope, so every part of this
  work is under it. Confirm what this machine holds with `warlock check .` before
  starting.

## Out of scope

- **Working a ticket that isn't yours.** Forman's `--ticket` works anything by
  name, and its `--any` drops the label filter. Here `--ticket` picks only among
  your own queue, and there is no `--any`. To work a ticket, assign it to
  yourself in Linear; to hand one on, reassign it. A flag that works somebody
  else's ticket is the one way two people end up on the same branch.
- **Resuming without being asked.** A halted run waits for `resume` rather than
  being retried by the next pull. Forman's pull picks a halted ticket up again
  with its failed sub-tasks still failed, finds nothing runnable, and halts a
  second time having done nothing. Making the reset a separate command means a
  pull only ever starts work somebody has said is ready.
- **Questions during execution.** A sub-task that needs a human reports
  `blocked`, and the run halts with the reason on the ticket. A relay into a
  session that is editing code is a session holding a dirty tree while it
  waits for somebody.
- **Parallel sub-tasks or parallel pulls.** Serial is the simplest correct model,
  and two sessions editing one tree is a merge conflict warlock would be making
  for itself.
- **Merging, or moving a ticket to done.** The pull request and the review state
  are the human gate, and nothing past them is automated.
- **Treating the hook or the diff check as security.** Sigils are self-asserted
  and machine-local, and a `Bash` command can write anywhere the operator can.
  Both checks protect against mistakes; they don't stop a determined person,
  which is the posture `p`, `r` and `s` already take.
- **Retrying a crossed sub-task by itself.** The tree holds work under a scope
  this machine doesn't hold, and deciding what happens to it is the operator's
  call. `resume` is where they say they have made it.
- **A configurable in-progress state.** `In Progress` is the default on every
  Linear team, and a `progress_state` field on the scope record is a field
  nobody has asked for yet.
- **Resuming on another machine.** The run record is machine-local, beside the
  sigils, because it describes this machine's working tree. Another machine sees
  the ticket in progress and skips it.
- **Rewriting a ticket that turned out to be wrong.** A ticket edited in Linear
  mid-run is picked up by the next run's split only if the run record is deleted.
  Reconciling a split against an edited ticket is its own problem.
- **Cost caps.** Cost is recorded per sub-task and shown in the manifest.
  Stopping on a budget is a later decision, made with real numbers.

## Scope

### 1. Draft assigns what it files

depends_on: []

The `viewer { id }` query, `assigneeId` on `issueCreate`, and the rewritten doc
comment on `NewIssue`. The panel's `/draft` and `warlock draft` both go through
`Filing::file`, so one change covers both.

The decision is that assignment is the claim. Brief 23 left the assignee
unwritten because a field warlock invents is a decision taken from the person
who owns the board. Here the person running `draft` is that person, and
assigning the work to them is what keeps `pull` from ever starting somebody
else's ticket. Handing work to a teammate is a reassignment in Linear, and a
human makes it.

### 2. The queue and the choice

depends_on: []

The Linear query for a scope's queue: issues on the team, carrying the label,
assigned to the viewer, not completed or cancelled, each with its state, priority
and blocking relations and each blocker's state type. It goes beside
`fetch_project` in `linear.rs`, against the same `Posts` seam. Then selection as a
pure function: readiness, the review-state skip, the in-progress-elsewhere skip,
the halted skip, resumed runs first, the ordering, the checks a named ticket
must pass, and the reasons for everything skipped or refused.

The decision is that a blocker counts whoever holds it. Filtering the graph to
the operator's own tickets makes a teammate's unfinished blocker vanish, and a
vanished blocker reads as a satisfied one. Forman's docstring on `select_ticket`
records the same trap.

### 3. Moving and commenting on an issue

depends_on: []

Resolving a team workflow state by name, `issueUpdate` for the state, and
`commentCreate` on an issue rather than a project. Each is one request with no
retry.

The decision is that neither move is ever fatal. The comment is the record of
what happened. A board without `In Progress` or without the record's
`review_state` gets a line in the output and a run that carries on, because a
workflow column is a courtesy to people who aren't watching the terminal.

### 4. The run record

depends_on: []

`state.json`, `manifest.md` and the sub-task briefs under
`~/.warlock/<project>/pulls/<TICKET>/`: the types, the statuses, the save that
renders the manifest, the next-ready query, the resume reset with and without
`--failed-only`, and the lookup of halted and resumed runs by scope.

The decision is where the record lives. Inside the repository, it would either
be committed or need an ignore rule, and every save would move a digest and turn
a directory stale. Beside the sigils, it is machine-local, which is what it
describes: this machine's working tree and this machine's sessions.

### 5. The split

depends_on: [4]

The engine-side contract for the split, meaning the struct, its caps, its check,
its repair and its prompt text, and the read-only session that fills it at
`BRIEF_MODEL` and `BRIEF_EFFORT`. Then the sort, the numbering and the sub-task
briefs written into the run record.

The decision is that the split is held to the same bargain as a document and a
draft: the model fills a JSON object, and warlock checks it, repairs what it can
and lays out the result itself. Forman parses whatever the decomposer printed and
fails the run on the first defect. Brief 16 showed that refusing a repairable
answer costs a whole session to learn nothing.

### 6. The sub-task session

depends_on: []

A new session kind in `claude.rs` with the six tools, `--allowedTools`, the turn
limit, its own timeout, the `--settings` hook, and MCP and machine settings kept
out. The prompt: the sub-task brief, the ticket as context, the finished
siblings' summaries, the scope and the sigils, and the retry note on a second
attempt. The result contract, and the reading of a turn limit, a usage limit and
malformed output into `failed` with a retryable flag.

The decision is that this is the only session warlock gives a writing tool, and
it is told so by a test rather than by a comment. The first thing the slice
establishes against the real CLI is whether `--settings` hooks survive
`--setting-sources ""`, because the rest of the boundary work assumes one answer
or the other.

### 7. The gate

depends_on: []

`warlock check --gate` in both forms: a path with exit 3, and a `PreToolUse`
payload on stdin with the deny object on stdout. Then the post-session diff check,
which reads `git status`, asks `scope_covering` and `scope_opens_to` about every
changed path, and returns the crossings and the held-but-foreign scopes.

The decision is that the hook stops a write before it lands and the diff check
is the authority. The hook sees `Edit` and `Write` and never sees a `sed` or a
heredoc in `Bash`. The diff sees everything the session left in the tree. The
hook answers with the deny object rather than exit 2, so one flag serves a
shell, which reads exit statuses, and a hook, which only honours 2.

### 8. Git and the pull request

depends_on: []

The `git` and `gh` subprocesses: the clean-tree check, default-branch detection,
the fast-forward pull, the branch cut or checkout, `HEAD` before and after a
session, `add -A` and commit, push, and `gh pr create` with the fallback when
`gh` is absent. Each sits behind a seam the loop is tested through.

The decision is that warlock owns every commit and the session owns none. A
commit per finished sub-task leaves a halted run as a readable history and a
clean point to resume from. A session that commits for itself leaves the diff
check nothing to read.

### 9. Freshness before the pull request

depends_on: [7, 8]

Finding the pacted directories that are stale and hold a path the branch
changed, gating each through `Operation::Refresh`, running the refresh, and
committing the documents and the manifest on their own.

The decision is that a pull request from warlock arrives fresh. The ledger is
the thing warlock exists for, and a branch that leaves every directory it
touched stale hands the reviewer a map that is out of date on the very
directories they're reviewing. The refresh commit is separate so the code diff
reads on its own.

### 10. The loop and `warlock pull`

depends_on: [1, 2, 3, 4, 5, 6, 7, 8, 9]

The orchestration loop over the seams: start a new run or carry on a resumed
one, move to `In Progress`, split, work the sub-tasks with the retry rules and
the diff check, halt with a comment or finish with the refresh, the pull
request, the comment and the move to `review_state`. Then the subcommand,
`--ticket`, `--dry-run`, the progress lines, the exit statuses,
`HEADLESS-CLI.md`, and `check`'s rewritten review line.

The decision is that the loop runs no I/O, and every outcome, including the
crossing, is tested against fakes. A loop whose real runs cost money and edit
somebody's code is only trustworthy if its halts are tested as thoroughly as its
successes.

### 11. `warlock resume`

depends_on: [4]

The subcommand, `--failed-only`, the printed changes, the next command it
names, its refusals, and its section in `HEADLESS-CLI.md`.

The decision is that resuming is its own command and not something a pull does
by itself. A halt means a human has to look, and `resume` is that human saying
they have. It writes only the machine-local run record, so it needs no scope
check. The pull it hands off to is gated, and that is where the boundary
applies.

### 12. `/pull` and `/resume` in the panel

depends_on: [10, 11]

The sixth and seventh slash commands, the one sentence that lists them, the
dialog, `/resume` offering the `/pull` that follows it as a draft, the run off
the event loop writing to the account card, the milestones on the thread, the
locked composer, the refusal of `p`, `r`, `s`, `/draft` and `/resume` while a
pull is in flight, and the cancel on quit.

Two decisions. The panel is a view onto the same loop the CLI runs and not a
second implementation: it shows what the loop reports, gates the start behind one
dialog, and stays out of the way until the run halts or finishes. And a pull in
flight turns the panel from a conversation into an output window, because there
is nothing to say to a model while another one is editing the tree. The account
card already is that window for passes. The thread keeps the milestones so the
conversation around the pull stays readable instead of burying itself under
hundreds of tool lines.
