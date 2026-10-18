# Get started with Warlock

This guide takes a repository that has never used Warlock to its first pull
request opened by an agent. It assumes your repository is on GitHub and your
team tracks work in Linear or Jira Cloud.

The setup has three parts:

1. Set up your machine: the tools Warlock drives.
2. Set up your tracker: the states and team keys Warlock moves tickets
   through.
3. Set up the repository: scopes, the PR template, and the CI check. One
   person does this once, and everyone else gets it from the commit.

## Set up your machine

Every person who runs Warlock needs the following.

### Claude Code

Install Claude Code and sign in. Warlock runs inside your own Claude Code
session and uses your subscription. It holds no credentials of its own.

```sh
claude --version
```

### Python 3.11 or later

The hooks and the CI check are one Python script that uses only the standard
library. You don't need to install any packages.

```sh
python3 --version   # must print 3.11 or later
```

### git

Configure your identity. Warlock commits with whatever identity git is
configured to use, and it never sets one.

```sh
git config --global user.name "Your Name"
git config --global user.email "you@example.com"
```

Warlock pushes over your existing remote. Check that you can push to the
repository:

```sh
git remote -v
git push --dry-run
```

### GitHub CLI

`/warlock:pull` opens pull requests with `gh`. Install it, then sign in with
access to the repository:

```sh
gh auth login
gh auth status
```

`gh auth status` must report that you're logged in to `github.com`.

### Tracker connection

Warlock reaches your tracker through its official MCP server. It keeps no API
keys.

1. Connect the tracker your team uses:
   - **Linear:** on claude.ai, go to **Settings** > **Connectors** and connect
     Linear. To add it to Claude Code only, run
     `claude mcp add --transport http linear https://mcp.linear.app/mcp`
     instead.
   - **Jira:** on claude.ai, go to **Settings** > **Connectors** and connect
     Atlassian.
2. In a Claude Code terminal, run `/mcp`, select the server, and complete
   sign-in.

`claude mcp list` can show a server as **Connected** before you've signed in.
In that state, sessions see only a sign-in tool, and every tracker step fails.
To check that you're signed in, run `/mcp` and look for the server's tool list.

### The plugin

To install Warlock from GitHub, run:

```sh
claude plugin marketplace add Genetic-Pottery/warlock
claude plugin install warlock@warlock
```

To install from a local clone instead, pass its path:

```sh
claude plugin marketplace add ~/src/warlock
claude plugin install warlock@warlock
```

To try the plugin for one session without installing it, run
`claude --plugin-dir ~/src/warlock`.

The hooks only act in repositories that have a `.warlock/scopes.toml` file, so
installing the plugin changes nothing in your other repositories.

## Set up your tracker

`/warlock:init` asks for these values and offers your tracker's real names. Have
them ready.

### Team keys

Each scope maps to one Linear team or Jira project. Warlock uses its key in two
places:

- the tracker calls, to file and find tickets
- branch names, to know which ticket a branch works on (`DP-42` →
  `dp-42-add-retry`)

The key is the prefix on the team's ticket numbers: `DP` in `DP-42`.

### Ticket states

Pull moves each ticket through three states. These are the defaults:

| Setting | Default | When |
|---|---|---|
| `start_state` | `In Progress` | Pull starts the ticket |
| `review_state` | `In Review` | The pull request is open |
| unstarted | any state of type unstarted, such as `Todo` or `To Do` | Pull halts the ticket and sends it back |

Each name must exist in your workflow. Jira team-managed projects start with
only **To Do**, **In Progress**, and **Done**. To add **In Review**, edit the
project's workflow. If you'd rather not add it, set `review_state` to a status
you have.

### Project states

Draft and pull also move the Linear project or Jira epic:

| Setting | Default | When |
|---|---|---|
| `project_start_state` | `In Progress` | Draft has filed the tickets |
| `project_review_state` | `In Review` | Every ticket is in review or done |

Linear has no **In Review** project status by default. Add one in Linear's
project status settings, or set
`project_review_state` to a status you have. If a status doesn't exist, Warlock
leaves the project where it is and says which setting to change.

### Label

Pull works only tickets that carry a label, `warlock` by default. Draft adds it
to every ticket it files, and creates the label if it doesn't exist.

If your tracker already has tickets with that label, assigned to you and in an
unstarted state, pull can pick them up. To keep Warlock's queue separate, choose
a label no existing ticket uses.

## Set up the repository

### Run init

In the root of your repository, start Claude Code and run init:

```sh
claude
```

```
/warlock:init
```

Init lists your directories and asks, one question at a time:

1. Which team owns which directories.
2. Each team's tracker and key.
3. Which paths every team may change, such as shared protos, lockfiles, and
   root build files. These become a scope with `shared = true`.
4. Which settings differ from the defaults: base branch, label, and states.

It writes these files:

| File | Purpose |
|---|---|
| `.warlock/scopes.toml` | The scopes and settings |
| `.github/pull_request_template.md` | The PR template. CI requires each `## ` heading in it. |
| `.github/workflows/warlock.yml` | The CI check, if you accept it |

To check the result, run:

```sh
python3 ~/.claude/plugins/cache/warlock/warlock/0.1.0/scripts/warlock.py scopes
```

The version in the path matches the installed plugin. Inside Claude Code, you
can ask Claude to run `warlock scopes` instead.

### Decide what's unscoped

A path that no scope covers is closed to AI work. That's deliberate: nobody
owns it, so no ticket may change it.

To let tickets change root files such as `README.md`, add a scope with
`paths = ["."]`. The nearest scope wins, so team scopes still own their
directories.

### Commit the setup

Init leaves committing to you. Commit the files on a branch and open a pull
request:

```sh
git switch -c add-warlock
git add .warlock .github
git commit -m "Add Warlock scopes and CI check"
git push -u origin add-warlock
gh pr create --fill
```

The CI check skips this pull request, because it has no `warlock` label. Merge
it.

### Make the check required

Until it's required, the CI check only reports. To enforce it:

1. On GitHub, open the repository's **Settings** > **Branches**.
2. Add a branch protection rule for your base branch.
3. Select **Require status checks to pass before merging**, and add the
   `scopes` check.

The check reads the scopes from the base branch. A pull request that edits
`.warlock/scopes.toml` can't widen its own scopes.

## Use Warlock

### Brief a feature

```
/warlock:brief Add retry with backoff to the data-plane ingest client
```

Brief reads the code in the scopes the idea touches, then questions you one
question at a time until every section of the brief has a concrete answer. It
cuts the work into slices, one scope per slice. It shows you the whole brief,
and when you approve it, pushes it to Linear as a project or to Jira as an epic.
Nothing is written to the repository.

### Draft tickets

```
/warlock:draft
```

Without an argument, draft lists the briefs that have no tickets yet. It cuts
the chosen brief into tickets, checks that each ticket's files are inside its
scope, and shows you the set. When you approve it, draft files the tickets with
the label, assigned to you, linked by `blocked by`, and moves the project or
epic to `project_start_state`.

### Pull a ticket

```
/warlock:pull
```

Pull takes the first ticket that's assigned to you, carries the label, is
unstarted, and has no unfinished blocker. To pull from one scope, pass its name.
To pull one ticket, pass its key.

Pull plans sub-tasks and posts the plan on the ticket. It works each sub-task in
a fresh subagent, commits it as `KEY.N: goal`, and pushes. It ends in one of two
outcomes:

- **Finished:** a pull request is open against `base`, and the ticket is in
  `review_state`. When it's the project's last open ticket, the project moves to
  `project_review_state`.
- **Halted:** the ticket needs a person. It has a `Warlock halted:` comment
  with the reason, and it's back in an unstarted state.

To resume a halted ticket, pull it by key. If the halt asked a question,
answer it on the ticket first, and pull reads your answer. The queue skips
halted tickets, so a block can't repeat on its own.

Transient failures don't need you. When a run crashes, hits a usage limit, or
pauses because a sub-task failed twice, the ticket stays in `start_state`. The
next `/warlock:pull` resumes it at the first unfinished sub-task, with no key
and no comment. After three pauses in a row, pull halts the ticket instead.

### Run unattended

To run pull with nobody watching, pass the tools it needs and tell it nobody can
answer questions:

```sh
claude -p '/warlock:pull No person is available.' \
  --allowedTools "Bash,Read,Grep,Glob,Edit,Write,Agent,mcp__claude_ai_Linear"
```

For Jira, replace `mcp__claude_ai_Linear` with `mcp__claude_ai_Atlassian_MCP`.
With nobody to ask, an ambiguous ticket halts with its questions on the ticket.

The hooks don't stop an agent from running `git push` or `gh` itself. On a
machine nobody watches, give the agent a GitHub token that can't push to
protected branches, and make the CI check required.

## Day to day

### Claude outside pull

Outside `/warlock:pull`, Claude can read, explain, plan, and brief, but the gate
refuses its edits to scoped files. To change code with Claude, file a ticket and
pull it. To skip Warlock for a session, start Claude with `WARLOCK_GATE=off`.

### Your own edits

Warlock never checks you. Your editor, your terminal, and the pull requests you
open yourself aren't gated or checked. The CI check looks only at pull requests
with the `warlock` label, which `/warlock:pull` adds.

### Other changes

- **Changing scopes:** edit `.warlock/scopes.toml` yourself. The gate refuses
  agent writes to `.warlock/`. To let Claude make the change, restart it with
  `WARLOCK_GATE=off`.
- **Templates:** to change the brief or ticket shape, copy `brief.md` or
  `ticket.md` from the plugin's `templates/` directory into
  `.warlock/templates/` and edit the copy.

## Troubleshooting

**"Claude changes scoped code only through a ticket"**

You asked Claude to edit scoped code outside `/warlock:pull`. File a ticket and
pull it, or start Claude with `WARLOCK_GATE=off` to skip Warlock.

**"no scope covers `<path>`"**

The file is in no scope, so it's closed to AI work. Add it to a scope, or make
the change yourself.

**"is in scope `<name>` (team X), and ticket Y-1 belongs to team Y"**

The ticket's team doesn't own that file. Split the work: draft a ticket for
team X, and link the two with `blocked by`.

**Tracker steps fail, or the skill says the connector needs authorization**

Run `/mcp` in a Claude Code terminal and sign in to the server. A remote
session can't open the sign-in menu; use the terminal on your machine, or
connect the server on claude.ai.

**Pull says a state or status doesn't exist**

A setting names a state your workflow doesn't have. Change the setting in
`.warlock/scopes.toml`, or add the state in your tracker.

**Pull picked up an old ticket**

The ticket carries the label, is assigned to you, and is unstarted. Remove the
label from it, or switch Warlock to a different label.
