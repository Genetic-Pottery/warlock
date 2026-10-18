---
name: brief
description: Grill a feature idea into a brief, then push it to Linear as a project or to Jira as an epic.
disable-model-invocation: true
argument-hint: "[idea]"
---

Turn an idea into a brief the team can draft tickets from. The warlock CLI is
`python3 "${CLAUDE_PLUGIN_ROOT}/scripts/warlock.py"`, written `warlock` below.
For tracker steps, read `${CLAUDE_PLUGIN_ROOT}/reference/trackers.md`.

The template is `.warlock/templates/brief.md` if it exists, otherwise
`${CLAUDE_PLUGIN_ROOT}/templates/brief.md`. Its sections are the questions this
conversation must answer.

## 1. Ground the idea

Run `warlock scopes`. Ask which scopes the work touches, unless the idea names
them. Read the code in those scopes that the idea concerns: the files, the
current behaviour, the tests. Done when you can name the files a change would
touch.

## 2. Grill

Ask one question at a time. Push back where the idea is vague, contradicts the
code you read, or hides a decision: name the alternative and ask which one and
why. Propose an answer when you have one, and let the person correct it.

Keep each slice inside one scope. Work that spans scopes becomes one slice per
scope, linked by `depends_on`.

Done when every template section has concrete content the person agreed to, and
no question is open. "We'll figure it out" is an open question; record it under
Out of scope or Constraints with the person's reason, or keep asking.

## 3. Write

Fill the template. Show the whole brief and ask for changes. Repeat until the
person approves it. If the answers came from another agent rather than a person,
end the brief with the line `_Answers supplied by an agent._`

## 4. Push

Push the brief to the tracker of the scope that owns slice 1 (see "Push a
brief"). Write nothing to the repository. Reply with the project or epic link.
