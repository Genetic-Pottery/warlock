#!/usr/bin/env python3
"""Scope checks for AI work in a shared repository.

Run `warlock.py --help` for the commands. Needs Python 3.11 or later and git.
"""

from __future__ import annotations

import argparse
import json
import os
import re
import subprocess
import sys
import tomllib
from dataclasses import asdict, dataclass
from pathlib import Path, PurePosixPath

MANIFEST = PurePosixPath(".warlock/scopes.toml")
PR_TEMPLATE = PurePosixPath(".github/pull_request_template.md")
PROTECTED = PurePosixPath(".warlock")
TRACKERS = ("linear", "jira")
TEAM_KEY = re.compile(r"[A-Z][A-Z0-9_]*")
PR_LABEL = "warlock"
TICKET = re.compile(r"(?<![A-Za-z0-9])([A-Za-z][A-Za-z0-9_]*)-(\d+)(?![A-Za-z0-9])")
GATED_TOOLS = ("Edit", "Write", "MultiEdit", "NotebookEdit")
# Set at the top of the manifest as defaults, and per scope as overrides.
SETTINGS = {
    "base": "main",
    "label": "warlock",
    "start_state": "In Progress",
    "review_state": "In Review",
    "project_start_state": "In Progress",
    "project_review_state": "In Review",
}

OK, CROSSED, CONFIG_ERROR = 0, 1, 2


class ManifestError(Exception):
    pass


@dataclass(frozen=True)
class Scope:
    name: str
    paths: tuple[PurePosixPath, ...]
    tracker: str
    team: str
    shared: bool
    settings: dict[str, str]


@dataclass(frozen=True)
class Manifest:
    scopes: tuple[Scope, ...]
    defaults: dict[str, str]
    project_branches: bool = False

    @property
    def teams(self) -> set[str]:
        return {scope.team for scope in self.scopes}

    def base_for(self, ticket: str | None) -> str:
        """The branch a ticket's work leaves from and its pull request targets."""
        team = ticket.split("-", 1)[0] if ticket else None
        for scope in self.scopes:
            if scope.team == team and not scope.shared:
                return scope.settings["base"]
        return self.defaults["base"]

    def covering(self, path: PurePosixPath) -> Scope | None:
        """The scope whose path is the nearest ancestor of `path`."""
        best: tuple[int, Scope] | None = None
        for scope in self.scopes:
            for root in scope.paths:
                if root == PurePosixPath(".") or path == root or root in path.parents:
                    depth = 0 if root == PurePosixPath(".") else len(root.parts)
                    if best is None or depth > best[0]:
                        best = (depth, scope)
        return best[1] if best else None


def parse_manifest(text: str) -> Manifest:
    try:
        data = tomllib.loads(text)
    except tomllib.TOMLDecodeError as error:
        raise ManifestError(f"{MANIFEST} is not valid TOML: {error}") from error

    defaults = read_settings(data, SETTINGS, "the manifest")
    project_branches = data.get("project_branches", False)
    if not isinstance(project_branches, bool):
        raise ManifestError("`project_branches` must be true or false")

    entries = data.get("scope", [])
    if not isinstance(entries, list) or not entries:
        raise ManifestError("the manifest needs at least one [[scope]]")

    scopes: list[Scope] = []
    claimed: dict[PurePosixPath, str] = {}
    for index, entry in enumerate(entries, start=1):
        where = f"[[scope]] #{index}"
        name = entry.get("name")
        if not isinstance(name, str) or not name:
            raise ManifestError(f"{where} needs a `name`")
        where = f"scope `{name}`"
        if any(scope.name == name for scope in scopes):
            raise ManifestError(f"{where} is defined twice")

        tracker = entry.get("tracker")
        if tracker not in TRACKERS:
            raise ManifestError(f"{where}: `tracker` must be one of {', '.join(TRACKERS)}")

        team = entry.get("team")
        if not isinstance(team, str) or not TEAM_KEY.fullmatch(team):
            raise ManifestError(
                f"{where}: `team` must be the Linear team key or Jira project key, "
                "in capitals (for example `DP`)"
            )

        raw_paths = entry.get("paths")
        if not isinstance(raw_paths, list) or not raw_paths:
            raise ManifestError(f"{where} needs a non-empty `paths` list")
        paths = []
        for raw in raw_paths:
            path = normalize(raw, where)
            if path in claimed:
                raise ManifestError(f"`{path}` is claimed by both `{claimed[path]}` and `{name}`")
            claimed[path] = name
            paths.append(path)

        shared = entry.get("shared", False)
        if not isinstance(shared, bool):
            raise ManifestError(f"{where}: `shared` must be true or false")

        # One project spans several teams' scopes, so a per-scope switch would land
        # part of a project on its project branch and the rest on `base`.
        if "project_branches" in entry:
            raise ManifestError(f"{where}: set `project_branches` at the top of the manifest, not per scope")
        settings = read_settings(entry, defaults, where)
        for other in scopes:
            if other.team == team and not other.shared and not shared and other.settings["base"] != settings["base"]:
                raise ManifestError(
                    f"scopes `{other.name}` and `{name}` are both team {team} with different `base` branches"
                )
        scopes.append(
            Scope(name=name, paths=tuple(paths), tracker=tracker, team=team, shared=shared, settings=settings)
        )
    return Manifest(scopes=tuple(scopes), defaults=defaults, project_branches=project_branches)


def read_settings(table: dict, fallback: dict[str, str], where: str) -> dict[str, str]:
    settings = {}
    for key, default in fallback.items():
        value = table.get(key, default)
        if not isinstance(value, str) or not value.strip():
            raise ManifestError(f"{where}: `{key}` must be a non-empty string")
        settings[key] = value
    return settings


def normalize(raw: object, where: str) -> PurePosixPath:
    if not isinstance(raw, str) or not raw.strip():
        raise ManifestError(f"{where}: every path must be a non-empty string")
    path = PurePosixPath(raw.strip().rstrip("/") or ".")
    if path.is_absolute() or ".." in path.parts:
        raise ManifestError(f"{where}: `{raw}` must be relative to the repository root")
    return path


def git(root: Path, *args: str, check: bool = True) -> str:
    result = subprocess.run(
        ["git", "-C", str(root), *args], capture_output=True, text=True, check=False
    )
    if check and result.returncode != 0:
        raise RuntimeError(f"git {' '.join(args)} failed: {result.stderr.strip()}")
    return result.stdout if result.returncode == 0 else ""


def repo_root(start: Path) -> Path | None:
    out = git(start, "rev-parse", "--show-toplevel", check=False).strip()
    return Path(out).resolve() if out else None


def load_manifest(root: Path, ref: str | None = None) -> Manifest | None:
    if ref is None:
        file = root / MANIFEST
        if not file.is_file():
            return None
        return parse_manifest(file.read_text(encoding="utf-8"))
    text = git(root, "show", f"{ref}:{MANIFEST}", check=False)
    return parse_manifest(text) if text else None


def find_ticket(texts: list[str], teams: set[str]) -> str | None:
    """The first `TEAM-123` key whose prefix is a known team.

    Only known prefixes count, so `UTF-8` or `py-3` in a branch name is not a ticket.
    """
    for text in texts:
        for match in TICKET.finditer(text or ""):
            team = match.group(1).upper()
            if team in teams:
                return f"{team}-{match.group(2)}"
    return None


def current_ticket(root: Path, manifest: Manifest) -> str | None:
    branch = git(root, "rev-parse", "--abbrev-ref", "HEAD", check=False).strip()
    return find_ticket([os.environ.get("WARLOCK_TICKET", ""), branch], manifest.teams)


def refusal(manifest: Manifest, ticket: str | None, path: PurePosixPath) -> str | None:
    """Why the work for `ticket` may not change `path`, or None when it may."""
    if path == PROTECTED or PROTECTED in path.parents:
        return f"`{path}` is warlock's own configuration; a person edits it, not an agent"
    scope = manifest.covering(path)
    if scope is None:
        return f"no scope covers `{path}`, and unscoped paths are closed to AI work"
    if scope.shared:
        return None
    if ticket is None:
        return (
            f"`{path}` is in scope `{scope.name}`, and Claude changes scoped code only "
            f"through a ticket; file a {scope.team} ticket and run /warlock:pull"
        )
    team = ticket.split("-", 1)[0]
    if team != scope.team:
        return (
            f"`{path}` is in scope `{scope.name}` (team {scope.team}), "
            f"and ticket {ticket} belongs to team {team}"
        )
    return None


def relative_to_root(root: Path, path: Path) -> PurePosixPath | None:
    try:
        return PurePosixPath(path.resolve().relative_to(root).as_posix())
    except ValueError:
        return None


def resolve_base(root: Path, base: str) -> str | None:
    for candidate in (f"origin/{base}", base):
        if git(root, "rev-parse", "--verify", "--quiet", candidate, check=False).strip():
            return candidate
    return None


def branch_base(root: Path, manifest: Manifest, ticket: str | None) -> str:
    """The branch this ticket branch left from: its `warlock-base` git config, else the scope's base.

    Pull records a project branch in git config rather than passing `--base`,
    because the stop hook runs with no arguments.
    """
    branch = git(root, "rev-parse", "--abbrev-ref", "HEAD", check=False).strip()
    recorded = git(root, "config", "--get", f"branch.{branch}.warlock-base", check=False).strip()
    return recorded or manifest.base_for(ticket)


def changed_paths(root: Path, base: str | None, committed_only: bool = False) -> list[PurePosixPath]:
    """Paths changed since the branch left `base`: commits, staged, unstaged and untracked.

    `--no-renames` reports a rename as a delete plus an add, so moving a file
    out of another team's scope counts against that scope too.
    """
    names: set[str] = set()
    if committed_only and base:
        names.update(git(root, "diff", "--name-only", "--no-renames", "-z", f"{base}...HEAD").split("\0"))
    else:
        start = git(root, "merge-base", "HEAD", base, check=False).strip() if base else ""
        names.update(git(root, "diff", "--name-only", "--no-renames", "-z", start or "HEAD").split("\0"))
        names.update(git(root, "ls-files", "--others", "--exclude-standard", "-z").split("\0"))
    return sorted(PurePosixPath(name) for name in names if name)


def crossings(manifest: Manifest, ticket: str | None, paths: list[PurePosixPath]) -> list[str]:
    return [reason for path in paths if (reason := refusal(manifest, ticket, path))]


def read_hook_input() -> dict:
    try:
        data = json.load(sys.stdin)
    except json.JSONDecodeError:
        return {}
    return data if isinstance(data, dict) else {}


def gate_off() -> bool:
    return os.environ.get("WARLOCK_GATE", "").lower() == "off"


def command_gate(_: argparse.Namespace) -> int:
    """PreToolUse hook: refuse a file write the current ticket may not make."""
    event = read_hook_input()
    if gate_off() or event.get("tool_name") not in GATED_TOOLS:
        return OK
    tool_input = event.get("tool_input") or {}
    target = tool_input.get("file_path") or tool_input.get("notebook_path")
    if not target:
        return OK
    cwd = Path(event.get("cwd") or os.getcwd())
    file = Path(target) if Path(target).is_absolute() else cwd / target

    root = repo_root(cwd)
    if root is None:
        return OK
    path = relative_to_root(root, file)
    if path is None:
        return OK

    try:
        manifest = load_manifest(root)
    except ManifestError as error:
        return deny(f"warlock: {error}. Fix {MANIFEST} before AI work continues.")
    if manifest is None:
        return OK

    reason = refusal(manifest, current_ticket(root, manifest), path)
    return deny(f"warlock: {reason}.") if reason else OK


def deny(reason: str) -> int:
    print(
        json.dumps(
            {
                "hookSpecificOutput": {
                    "hookEventName": "PreToolUse",
                    "permissionDecision": "deny",
                    "permissionDecisionReason": reason,
                }
            }
        )
    )
    return OK


def command_stop(_: argparse.Namespace) -> int:
    """Stop hook: catch out-of-scope changes the gate cannot see, such as shell writes."""
    event = read_hook_input()
    if gate_off():
        return OK
    root = repo_root(Path(event.get("cwd") or os.getcwd()))
    if root is None:
        return OK
    try:
        manifest = load_manifest(root)
    except ManifestError:
        return OK
    if manifest is None:
        return OK

    ticket = current_ticket(root, manifest)
    found = crossings(manifest, ticket, changed_paths(root, resolve_base(root, branch_base(root, manifest, ticket))))
    if not found:
        return OK

    listing = "\n".join(f"- {reason}" for reason in found)
    # Blocking only on a ticket branch: elsewhere the changes are likely a
    # person's own edits, and a block on every turn would loop. A second block in
    # a row would loop too, since the agent cannot undo a change it did not make.
    if ticket is None or event.get("stop_hook_active"):
        print(json.dumps({"systemMessage": f"warlock: changes outside scope remain:\n{listing}"}))
        return OK
    print(
        f"warlock: this branch has changes outside what {ticket} may touch:\n{listing}\n"
        "Revert the ones you made. If you did not make one, leave it and say so. "
        "If the work needs those paths, stop and report the ticket as blocked with this reason.",
        file=sys.stderr,
    )
    return 2


def command_check(args: argparse.Namespace) -> int:
    """Report every changed path the current ticket may not touch."""
    if (found := here()) is None:
        return CONFIG_ERROR
    root, manifest = found
    ticket = args.ticket.upper() if args.ticket else current_ticket(root, manifest)
    base = resolve_base(root, args.base or branch_base(root, manifest, ticket))
    crossed = crossings(manifest, ticket, changed_paths(root, base))
    print(f"ticket: {ticket or 'none'}  base: {base or 'none (working tree only)'}")
    for reason in crossed:
        print(f"crossed: {reason}")
    if not crossed:
        print("ok: every change is inside the ticket's scopes")
    return CROSSED if crossed else OK


def command_ci(args: argparse.Namespace) -> int:
    """Fail a Warlock pull request that names no ticket, crosses scopes, or skips the PR template.

    Only pull requests carrying the `warlock` label are checked: the guardrails
    are for AI work, and a person's own pull request is theirs. The manifest and
    PR template come from the base ref, so a pull request cannot widen its own
    scopes by editing them.
    """
    labels = json.loads(os.environ.get("PR_LABELS") or "[]")
    if PR_LABEL not in labels:
        print(f"::notice::warlock: no `{PR_LABEL}` label, so this pull request is not Warlock's to check")
        return OK

    root = repo_root(Path.cwd())
    if root is None:
        print("::error::warlock: not inside a git repository")
        return CONFIG_ERROR
    try:
        manifest = load_manifest(root, args.base)
    except ManifestError as error:
        print(f"::error file={MANIFEST}::warlock: {error}")
        return CONFIG_ERROR
    if manifest is None:
        # The pull request that adds warlock to a repository has no manifest on its base.
        print(f"::notice::warlock: no {MANIFEST} on {args.base} yet, so nothing to check")
        return OK

    title = os.environ.get("PR_TITLE", "")
    body = os.environ.get("PR_BODY", "")
    branch = os.environ.get("PR_HEAD_REF", "")
    failures: list[str] = []

    ticket = find_ticket([branch, title, body], manifest.teams)
    if ticket is None:
        keys = ", ".join(sorted(f"{team}-123" for team in manifest.teams))
        failures.append(f"the branch, title or body must name a ticket (for example {keys})")

    else:
        for path in changed_paths(root, args.base, committed_only=True):
            if reason := refusal(manifest, ticket, path):
                print(f"::error file={path}::warlock: {reason}")
                failures.append(reason)

    template = git(root, "show", f"{args.base}:{PR_TEMPLATE}", check=False)
    for heading in required_headings(template):
        if heading not in {line.strip() for line in body.splitlines()}:
            failures.append(f"the PR description is missing the `{heading}` section")

    for failure in failures:
        print(f"::error::warlock: {failure}")
    if not failures:
        print(f"warlock: {ticket} stays inside its scopes")
    return CROSSED if failures else OK


def required_headings(template: str) -> list[str]:
    return [line.strip() for line in template.splitlines() if line.startswith("## ")]


def command_scopes(args: argparse.Namespace) -> int:
    """Validate the manifest and list its scopes."""
    if (found := here()) is None:
        return CONFIG_ERROR
    root, manifest = found
    ticket = current_ticket(root, manifest)
    if args.json:
        scopes = [{**asdict(scope), "paths": [str(path) for path in scope.paths]} for scope in manifest.scopes]
        report = {
            "ticket": ticket,
            "base": manifest.base_for(ticket),
            "project_branches": manifest.project_branches,
            "scopes": scopes,
        }
        print(json.dumps(report, indent=2))
        return OK
    for scope in manifest.scopes:
        kind = "shared" if scope.shared else f"{scope.tracker} {scope.team}"
        paths = ", ".join(str(path) for path in scope.paths)
        settings = ", ".join(f"{key}={value!r}" for key, value in scope.settings.items())
        print(f"{scope.name}\t{kind}\t{paths}\n\t{settings}")
    print(f"ticket on this branch: {ticket or 'none'}")
    return OK


def finished_subtasks(log: str, ticket: str) -> list[int]:
    """Sub-task numbers from commit subjects shaped `KEY.N: goal`."""
    pattern = re.compile(rf"{re.escape(ticket)}\.(\d+):", re.IGNORECASE)
    return sorted({int(match.group(1)) for line in log.splitlines() if (match := pattern.match(line))})


def command_progress(args: argparse.Namespace) -> int:
    """Print which sub-tasks of the branch's ticket are committed, as JSON."""
    if (found := here()) is None:
        return CONFIG_ERROR
    root, manifest = found
    ticket = args.ticket.upper() if args.ticket else current_ticket(root, manifest)
    if ticket is None:
        print("warlock: this branch names no ticket", file=sys.stderr)
        return CONFIG_ERROR
    base = resolve_base(root, branch_base(root, manifest, ticket))
    log = git(root, "log", "--format=%s", f"{base}..HEAD" if base else "HEAD", check=False)
    dirty = bool(git(root, "status", "--porcelain", check=False).strip())
    print(json.dumps({"ticket": ticket, "base": base, "finished": finished_subtasks(log, ticket), "dirty": dirty}))
    return OK


def command_where(args: argparse.Namespace) -> int:
    """Name the scope covering each path, and whether this branch may change it."""
    if (found := here()) is None:
        return CONFIG_ERROR
    root, manifest = found
    ticket = current_ticket(root, manifest)
    status = OK
    for raw in args.paths:
        path = relative_to_root(root, Path(raw))
        if path is None:
            print(f"{raw}\toutside the repository")
            continue
        scope = manifest.covering(path)
        # With no ticket, `where` answers which scope a file is in, which is what
        # drafting asks; "this branch names no ticket" would fail every file.
        stand_in = ticket or (f"{scope.team}-0" if scope else None)
        reason = refusal(manifest, stand_in, path)
        print(f"{path}\t{scope.name if scope else '(none)'}\t{reason or 'writable'}")
        status = status or (CROSSED if reason else OK)
    return status


def here() -> tuple[Path, Manifest] | None:
    """The repository around the working directory and its manifest, or None after saying why."""
    root = repo_root(Path.cwd())
    if root is None:
        print("warlock: not inside a git repository", file=sys.stderr)
        return None
    try:
        manifest = load_manifest(root)
    except ManifestError as error:
        print(f"warlock: {error}", file=sys.stderr)
        return None
    if manifest is None:
        print(f"warlock: no {MANIFEST} here; run the warlock init skill", file=sys.stderr)
        return None
    return root, manifest


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(prog="warlock", description="Scope checks for AI work in a shared repository.")
    commands = parser.add_subparsers(dest="command", required=True)

    commands.add_parser("gate", help="PreToolUse hook (reads hook JSON on stdin)").set_defaults(run=command_gate)
    commands.add_parser("stop", help="Stop hook (reads hook JSON on stdin)").set_defaults(run=command_stop)

    check = commands.add_parser("check", help="list changes the current ticket may not touch")
    check.add_argument("--ticket", help="ticket key; default is the one in the branch name")
    check.add_argument("--base", help="branch to diff against; default is the branch's warlock-base, else the ticket scope's base")
    check.set_defaults(run=command_check)

    ci = commands.add_parser("ci", help="pull request check (reads PR_LABELS, PR_TITLE, PR_BODY, PR_HEAD_REF)")
    ci.add_argument("--base", required=True, help="base ref, for example origin/main")
    ci.set_defaults(run=command_ci)

    scopes = commands.add_parser("scopes", help="validate and list the scopes with their settings")
    scopes.add_argument("--json", action="store_true", help="print the resolved settings as JSON")
    scopes.set_defaults(run=command_scopes)

    progress = commands.add_parser("progress", help="sub-tasks committed on this branch, as JSON")
    progress.add_argument("--ticket", help="ticket key; default is the one in the branch name")
    progress.set_defaults(run=command_progress)

    where = commands.add_parser("where", help="name the scope covering each path")
    where.add_argument("paths", nargs="+")
    where.set_defaults(run=command_where)

    args = parser.parse_args(argv)
    return args.run(args)


if __name__ == "__main__":
    sys.exit(main())
