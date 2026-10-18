# Contributing to Warlock

Warlock is a small plugin: one Python script, four skills, two reference files,
and some templates. Most changes touch one of those, and this guide covers how
to change and test each.

## Set up

You need the following:

- Claude Code, signed in
- Python 3.11 or later
- `git`, and `gh` signed in, to test `/warlock:pull`
- A Linear or Jira workspace you can fill with test tickets, connected through
  `/mcp`, to test the tracker skills

Clone the repository, then load your working copy into Claude Code from any
other repository:

```sh
git clone https://github.com/Genetic-Pottery/warlock.git
cd ~/some/test-repo
claude --plugin-dir ~/path/to/warlock
```

Edits take effect the next time you start Claude Code this way. While you
develop, don't also install Warlock from the marketplace. Two copies make it
unclear which one ran.

## Layout

| Path | What it is |
|---|---|
| `scripts/warlock.py` | The only code: the gate and stop hooks, the CI check, and the CLI the skills call |
| `hooks/hooks.json` | Wires the hooks to `PreToolUse`, `Stop`, and `SubagentStop` |
| `skills/` | `init`, `brief`, `draft`, and `pull`, as instructions Claude follows |
| `reference/trackers.md` | Tracker operations shared by the skills, with real MCP tool names |
| `reference/subtask.md` | What each pull subagent reads before it works a sub-task |
| `templates/` | Default brief, ticket, PR, manifest, and workflow files |
| `action.yml` | The GitHub Action that runs the CI check |
| `tests/` | Unit tests for `warlock.py`, run against temporary git repositories |

## Change the script

`scripts/warlock.py` uses the Python standard library only. The hooks and the CI
check then run anywhere Python runs, with no install step. Keep it that way.

Add or update a test in `tests/test_warlock.py` for every behavior you change.
The tests build real git repositories in temporary directories, so you can test
branches, commits, and untracked files as they are.

## Change a skill or reference file

Skills are instructions, not code. A unit test can't show that a skill works.
To test a skill change, run the skill against a real tracker:

- **Interactive:** start Claude Code with `--plugin-dir`, and run the skill.
- **Headless:** run it with `claude -p`, for example:

  ```sh
  claude -p '/warlock:pull No person is available.' \
    --plugin-dir ~/path/to/warlock \
    --allowedTools "Bash,Read,Grep,Glob,Edit,Write,Agent,mcp__claude_ai_Linear"
  ```

Run tracker tests on a team or project you can fill with junk, and use a label
no real ticket carries, so pull never picks up real work.

MCP tool names change between server versions. When a skill fails because a
tool is missing, look up the current name, and fix it in
`reference/trackers.md` rather than in each skill.

Write skills in the positive: say what Claude does, not what it avoids. End
each step on a condition Claude can check, such as "Done when `warlock check`
prints `ok`."

## Write comments

Write a comment only when its absence would let a plausible edit break
something, or when it names a rejected alternative and why. Models read this
code, and they treat a comment as fact. When a change makes a comment false,
fix the comment in the same change.

## Check your work

Run both commands, and read each exit status:

```sh
python3 -m unittest discover -s tests
claude plugin validate .
```

`claude plugin validate` warns that `CLAUDE.md` at the plugin root isn't loaded
as context for plugin users. That warning is expected: `CLAUDE.md` is for people
working on Warlock itself.

## Open a pull request

`main` requires a pull request and passing checks: `test (3.11)` and
`test (3.13)`. Describe what changed and how you tested it. For a skill change,
say which tracker you ran it against and what the run ended in.

## Release a version

The plugin and the CI action are pinned separately:

- **Plugin:** installs from a tag. Users on `v0.1.0` stay there until they
  choose a newer tag.
- **CI action:** user workflows reference `@v0`, which moves with each `0.x`
  release.

To release, follow these steps:

1. Bump `version` in `.claude-plugin/plugin.json`, for example to `0.1.1`.
2. Tag the release and move `v0` to it:

   ```sh
   git tag -a v0.1.1 -m "warlock 0.1.1"
   git tag -f v0 v0.1.1^{}
   git push origin v0.1.1
   git push -f origin v0
   ```

3. Create a GitHub release for the tag, with install steps for that version.

Moving `v0` changes every user's CI check at once, so release only what you've
tested.

## License

By contributing, you agree that your contributions are licensed under the
Apache License 2.0. See [LICENSE](LICENSE).
