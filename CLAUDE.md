## Before you report work finished

Run both, and read each exit status:

    python3 -m unittest discover -s tests
    claude plugin validate .

## Layout

- `scripts/warlock.py` is the only code. It uses the standard library only, so
  the hooks and CI need no install step. Keep it that way.
- Skills reach Linear and Jira through their MCP servers. Shared tracker steps
  live in `reference/trackers.md`, not in each skill.
- Templates in `templates/` are defaults. A repository overrides them in
  `.warlock/templates/`, except the PR template, which is GitHub's
  `.github/pull_request_template.md`.

## Comments

Write a comment only when its absence would let a plausible edit break
something, or when it names a rejected alternative and why.
