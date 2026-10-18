---
name: init
description: Set up warlock scopes, the PR template, and the CI check in this repository.
disable-model-invocation: true
---

Set up warlock in the current repository. Each step ends when its check passes.
The warlock CLI is `python3 "${CLAUDE_PLUGIN_ROOT}/scripts/warlock.py"`, written
`warlock` below.

## 1. Check the starting point

Run `git rev-parse --show-toplevel`. Outside a git repository, stop and say so.

If `.warlock/scopes.toml` exists, run `warlock scopes` and ask what to change.
The gate refuses agent writes to `.warlock/`, so the person edits it by hand or
restarts Claude Code with `WARLOCK_GATE=off` for this one change. Stop here.

## 2. Map the teams

List the directories two levels down (`git ls-tree -d --name-only HEAD` and
again inside each result). Ask the person, one question at a time:

1. Which teams own which directories?
2. For each team: Linear or Jira, and the team key or project key. When the
   tracker's MCP server is connected, list its teams or projects so the person
   picks instead of types.
3. Which paths every team may change (shared protos, lockfiles, root build
   files)? These go in a scope with `shared = true`.
4. The settings at the top of `${CLAUDE_PLUGIN_ROOT}/templates/scopes.toml`:
   which branch pull requests target, the label that marks pullable tickets,
   and the ticket and project states. Show the defaults, and when the tracker's
   MCP server is connected, list the team's real states so the person picks
   existing names. Ask which teams differ from the defaults.

Done when every top-level directory is either owned by a scope, shared, or
named by the person as closed to AI work.

## 3. Write the manifest

Write `.warlock/scopes.toml` in the shape of
`${CLAUDE_PLUGIN_ROOT}/templates/scopes.toml`. Run `warlock scopes`. Done when it
lists every scope with the settings the person chose and no error.

## 4. Add the PR template

If `.github/pull_request_template.md` doesn't exist, copy
`${CLAUDE_PLUGIN_ROOT}/templates/pr.md` there. If it exists, show the person
that CI will require every `## ` heading in it, and ask whether to keep it,
replace it, or add warlock's headings.

## 5. Offer the CI check

Explain in two sentences: the check fails a pull request opened by
`/warlock:pull` that names no ticket, changes files outside the ticket team's
scopes, or is missing a PR template section. People's own pull requests aren't
checked.

If the person wants it, copy `${CLAUDE_PLUGIN_ROOT}/templates/workflow.yml` to
`.github/workflows/warlock.yml`.

## 6. Hand over

List the files you wrote. Tell the person:

- To customize the brief or ticket shape, copy `brief.md` or `ticket.md` from
  `${CLAUDE_PLUGIN_ROOT}/templates/` into `.warlock/templates/` and edit it.
- To turn on the CI check, they make it a required status check in the
  repository's branch protection.
- The next steps are `/warlock:brief`, `/warlock:draft`, and `/warlock:pull`.

Leave committing to the person.
