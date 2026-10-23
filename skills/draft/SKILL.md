---
name: draft
description: Cut a brief (a Linear project or Jira epic) into tickets with dependencies, one team per ticket.
disable-model-invocation: true
argument-hint: "[project or epic]"
---

Cut a pushed brief into tickets an agent can pull. The warlock CLI is
`python3 "${CLAUDE_PLUGIN_ROOT}/scripts/warlock.py"`, written `warlock` below.
For tracker steps, read `${CLAUDE_PLUGIN_ROOT}/reference/trackers.md`.

The ticket template is `.warlock/templates/ticket.md` if it exists, otherwise
`${CLAUDE_PLUGIN_ROOT}/templates/ticket.md`.

## 1. Pick the brief

With an argument, find that project or epic. Without one, run `warlock scopes`,
list the briefs to draft in those teams, and ask which one.

## 2. Read the slices

Read the brief's Scope section. Each `### N.` heading is a slice with
`depends_on` and `scope` lines. Check each `scope` names a scope in
`warlock scopes`. A slice in a shared scope needs an owning team: ask which one.

Done when every slice has one team.

## 3. Draft

Write one or more tickets per slice in the template's shape. Each ticket:

- lands on its own, in a pull request a reviewer reads in one sitting
- takes its acceptance criteria from the brief's success criteria, in the
  brief's words
- lists likely files, and `warlock where <file>...` puts every one inside the
  ticket's scope
- depends on the tickets it needs first: the slice's `depends_on`, plus order
  within the slice
- when `project_branches` is true, ends with a `## Project branch` section
  that holds only `project/<slug>`. The slug is the brief's title in
  lowercase, with every run of characters that aren't letters or digits
  replaced by `-`. Every ticket of the brief names the same branch, whichever
  team it belongs to.

Where the brief leaves a choice open, ask the person. In a session with nobody
to ask, file the ticket with the question at the top of its Context section.

## 4. Review

Show every draft: title, team, dependencies, and body. Ask for changes. Done
when the person approves the set.

## 5. File

File each ticket (see "File a ticket"), then create the `blocked by` relations.
Move the project or epic to `project_start_state` (see "Move a project").
Reply with each ticket's key, title, and blockers.

Settings such as `label` and `project_start_state` come from
`warlock scopes --json`, under the scope that owns slice 1.
`project_branches` is at the top level of the same JSON.
