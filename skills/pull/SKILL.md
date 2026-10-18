---
name: pull
description: Work the next unblocked ticket inside its scope and open a pull request, or pause or halt the ticket with a reason.
disable-model-invocation: true
argument-hint: "[ticket key | scope]"
---

Take one ticket to a pull request. You are the orchestrator: you plan, dispatch
each sub-task to a fresh subagent, and commit. Keep your own context small, and
leave reading and writing code to the subagents.

Every run ends in exactly one outcome:

- **finished**: a pull request is open and the ticket is in `review_state`
- **paused**: something transient stopped the run. The ticket stays in
  `start_state` with a `Warlock paused:` comment, and the next pull resumes it.
- **halted**: the ticket needs a person: a decision, an answer, or work outside
  the scope. It carries a `Warlock halted:` comment, and the queue skips it until
  someone pulls it by key.

A run that dies without a word, from a crash or a usage limit, leaves the ticket
the way a pause does, and the next pull resumes it.

The warlock CLI is `python3 "${CLAUDE_PLUGIN_ROOT}/scripts/warlock.py"`, written
`warlock` below. For tracker steps, read
`${CLAUDE_PLUGIN_ROOT}/reference/trackers.md`. Settings in backticks (`base`,
`label`, `start_state`, `review_state`, `project_review_state`) come from
`warlock scopes --json`, under the scope whose `team` matches the ticket.

## 1. Preflight

Check both. If one fails, stop and say which; no ticket is touched yet.

- `warlock scopes` lists the scopes with no error
- `gh auth status` succeeds

## 2. Pick the ticket

With a ticket key, read that ticket. Otherwise read, for that scope's team or
for every non-shared scope's team:

1. tickets in `start_state` that are assigned to you, carry `label`, and have a
   `Warlock plan` comment. These are paused or interrupted runs; take the first.
2. the queue (see "The queue"). Take the first ticket.

In both lists, skip a ticket whose newest comment starting `Warlock` is a
`Warlock halted:` comment. Pulled by key, a halted ticket resumes like any
other.

If both are empty, say so and stop.

## 3. Start or resume

Read the ticket's comments. A comment starting `Warlock plan` means an earlier
run planned it: **resume**. Otherwise, **start**.

**Start:**

1. If `git status --porcelain` prints anything, stop and say so.
2. Run `git fetch origin`. If a local branch starting `<key in lowercase>-`
   exists, an earlier run halted before planning: switch to it. Otherwise run
   `git switch -c <key in lowercase>-<short slug> origin/<base>`.
3. Move the ticket to `start_state`.

**Resume:**

1. Move the ticket to `start_state` if it isn't there.
2. Run `git fetch origin`. Switch to the plan's branch: the local one if it
   exists, otherwise `git switch -c <branch> origin/<branch>`. If neither
   exists, the earlier run pushed nothing: delete nothing, and start over from
   step 2 of **Start** on the same branch name.
3. If `git status --porcelain` prints anything, it's the interrupted
   sub-task's partial work. Run
   `git stash push -u -m "warlock: interrupted <KEY>"` and redo that sub-task.

Done when `warlock scopes` reports the ticket key on this branch.

## 4. Plan

Skip this step when resuming.

Read the ticket and its comments. A person's comment after a `Warlock halted:`
comment answers it, and binds the plan the way the description does. Then read
only enough code to cut the work: names, file layout, where the tests live. Cut
the work into sub-tasks, in the order they're done. Each one:

- changes one coherent thing a subagent finishes in one sitting
- has a goal in one line, in the ticket's words
- has a definition of done: checkable statements, including the commands
  that show it holds
- lists likely files, every one inside the scope (`warlock where <file>...`)
- copies any rule the ticket fixes (a value, a name, what to leave alone) into
  the definition of done word for word

Between 1 and 8 sub-tasks. A ticket that's one small change is one sub-task.

Post the plan as a ticket comment in this shape, and keep the comment's id:

```markdown
Warlock plan
branch: <branch>

- [ ] 1. <goal>
  - done when: <statement>; <statement>
  - files: `<path>`, `<path>`
  - notes: <what the subagent needs that the ticket doesn't say>
- [ ] 2. ...
```

## 5. Work the sub-tasks

Run `warlock progress`. Its `finished` list holds the sub-tasks already
committed on this branch. For each sub-task not in it, in order:

1. Dispatch a subagent with the Agent tool. Its prompt holds:
   - the line: "Read `${CLAUDE_PLUGIN_ROOT}/reference/subtask.md` and follow it."
   - the ticket key and title
   - this sub-task's entry from the plan
   - one line per finished sub-task saying what it changed
2. Read the report's first line.
   - **done:** run `warlock check`. If it prints `ok`, commit everything with
     the message `<KEY>.<N>: <goal>`, using the repository's git identity as
     configured. Run `git push -u origin HEAD`. Tick the sub-task in the plan
     comment.
   - **failed:** dispatch one more fresh subagent with the same prompt plus the
     failure reason. If that one fails too, pause.
   - **blocked**, or `warlock check` lists a crossing: halt.

Done when `warlock progress` lists every sub-task in the plan.

## 6. Finish

1. Run the ticket's test plan and `warlock check`. Done when both pass. If they
   fail, dispatch a subagent to fix what fails, and commit its work as
   `<KEY>.<N+1>: fix <what>`.
2. Run `gh label create warlock --description "Opened by Warlock" --force`, so
   the label exists. Open the pull request with
   `gh pr create --base <base> --label warlock`, titled `<KEY>: <ticket title>`.
   Fill every section of `.github/pull_request_template.md` from the sub-task
   reports. CI checks only pull requests with this label.
3. Move the ticket to `review_state` and comment with the pull request link.
4. If the ticket belongs to a project or epic, list its tickets. When every one
   is in `review_state` or finished, move the project or epic to
   `project_review_state` (see "Move a project").

Reply with the pull request link.

## 7. Pause

Pause when a sub-task fails twice, or when a tracker, `git`, or `gh` step fails
after the ticket moved. These are transient: a later run can succeed with no one
changing anything.

1. If the ticket's newest two `Warlock` comments are both `Warlock paused:`,
   this would be the third pause in a row. Halt instead, giving the three
   reasons.
2. Run `git stash push -u -m "warlock: paused <KEY>.<N>"` so the branch stays
   clean.
3. Comment on the ticket. Start with `Warlock paused:` and the reason in one
   sentence, naming the sub-task. Then say which sub-tasks are committed, and
   that the next pull resumes it.
4. Leave the ticket in `start_state`.

Reply with the reason and the ticket link.

## 8. Halt

Halt when the ticket needs a person: a sub-task is blocked or crosses the
scope, the ticket is ambiguous and nobody can answer, or it has paused three
times in a row.

1. Run `git stash push -u -m "warlock: halted <KEY>.<N>"` so the branch stays
   clean.
2. Comment on the ticket. Start with `Warlock halted:` and the reason in one
   sentence, naming the sub-task. Then say which sub-tasks are committed and
   what the ticket needs. Ask a question when the ticket needs an answer.
3. Move the ticket back to the unstarted state it came from.

Reply with the reason and the ticket link.
