import io
import json
import os
import subprocess
import sys
import tempfile
import textwrap
import unittest
from contextlib import redirect_stderr, redirect_stdout
from pathlib import Path, PurePosixPath
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "scripts"))

import warlock  # noqa: E402

MANIFEST = textwrap.dedent(
    """
    base = "main"

    [[scope]]
    name = "data-plane"
    paths = ["services/data"]
    tracker = "linear"
    team = "DP"

    [[scope]]
    name = "control-plane"
    paths = ["services/control"]
    tracker = "jira"
    team = "CP"

    [[scope]]
    name = "shared"
    paths = ["libs/proto"]
    tracker = "linear"
    team = "PLAT"
    shared = true
    """
)

PR_TEMPLATE = "## Ticket\n\n## What changed\n\n## How I tested it\n"


def manifest() -> warlock.Manifest:
    return warlock.parse_manifest(MANIFEST)


class ManifestTests(unittest.TestCase):
    def test_nearest_scope_wins(self):
        text = MANIFEST + '\n[[scope]]\nname = "data-api"\npaths = ["services/data/api"]\ntracker = "linear"\nteam = "API"\n'
        parsed = warlock.parse_manifest(text)
        self.assertEqual(parsed.covering(PurePosixPath("services/data/api/x.rs")).name, "data-api")
        self.assertEqual(parsed.covering(PurePosixPath("services/data/x.rs")).name, "data-plane")

    def test_a_path_named_like_a_scope_prefix_is_not_inside_it(self):
        self.assertIsNone(manifest().covering(PurePosixPath("services/database/x.rs")))

    def test_root_scope_covers_everything_else(self):
        text = MANIFEST + '\n[[scope]]\nname = "root"\npaths = ["."]\ntracker = "linear"\nteam = "OPS"\n'
        parsed = warlock.parse_manifest(text)
        self.assertEqual(parsed.covering(PurePosixPath("Cargo.toml")).name, "root")
        self.assertEqual(parsed.covering(PurePosixPath("services/data/x")).name, "data-plane")

    def test_rejects_bad_manifests(self):
        cases = {
            "lowercase team": MANIFEST.replace('team = "DP"', 'team = "dp"'),
            "unknown tracker": MANIFEST.replace('tracker = "jira"', 'tracker = "asana"'),
            "escaping path": MANIFEST.replace('"services/data"', '"../other"'),
            "absolute path": MANIFEST.replace('"services/data"', '"/etc"'),
            "claimed twice": MANIFEST.replace('"services/control"', '"services/data/"'),
            "no scopes": 'base = "main"\n',
            "not toml": "[[scope",
        }
        for name, text in cases.items():
            with self.subTest(name), self.assertRaises(warlock.ManifestError):
                warlock.parse_manifest(text)


class SettingsTests(unittest.TestCase):
    def test_scopes_inherit_top_level_settings(self):
        parsed = warlock.parse_manifest('label = "agent"\n' + MANIFEST)
        self.assertEqual(parsed.covering(PurePosixPath("services/data/x")).settings["label"], "agent")
        self.assertEqual(parsed.covering(PurePosixPath("services/data/x")).settings["review_state"], "In Review")

    def test_a_scope_overrides_a_setting(self):
        text = MANIFEST.replace('team = "CP"', 'team = "CP"\nbase = "release"\nreview_state = "Code Review"')
        parsed = warlock.parse_manifest(text)
        self.assertEqual(parsed.base_for("CP-1"), "release")
        self.assertEqual(parsed.base_for("DP-1"), "main")
        self.assertEqual(parsed.base_for(None), "main")

    def test_one_team_has_one_base(self):
        text = MANIFEST + '\n[[scope]]\nname = "data-2"\npaths = ["x"]\ntracker = "linear"\nteam = "DP"\nbase = "other"\n'
        with self.assertRaises(warlock.ManifestError):
            warlock.parse_manifest(text)

    def test_rejects_an_empty_setting(self):
        with self.assertRaises(warlock.ManifestError):
            warlock.parse_manifest('start_state = ""\n' + MANIFEST)


class TicketTests(unittest.TestCase):
    def test_finds_known_keys_in_branch_names(self):
        self.assertEqual(warlock.find_ticket(["dp-42-add-retry"], {"DP"}), "DP-42")
        self.assertEqual(warlock.find_ticket(["feature/CP-7"], {"CP"}), "CP-7")

    def test_ignores_unknown_prefixes(self):
        self.assertIsNone(warlock.find_ticket(["utf-8-fix", "py-3"], {"DP"}))

    def test_first_text_wins(self):
        self.assertEqual(warlock.find_ticket(["dp-1", "CP-2"], {"DP", "CP"}), "DP-1")


class RefusalTests(unittest.TestCase):
    def test_own_scope_is_writable(self):
        self.assertIsNone(warlock.refusal(manifest(), "DP-1", PurePosixPath("services/data/a.rs")))

    def test_other_team_is_refused(self):
        reason = warlock.refusal(manifest(), "DP-1", PurePosixPath("services/control/a.rs"))
        self.assertIn("team CP", reason)

    def test_shared_scope_is_open_to_any_ticket(self):
        self.assertIsNone(warlock.refusal(manifest(), "CP-1", PurePosixPath("libs/proto/a.proto")))

    def test_unscoped_path_is_closed(self):
        self.assertIn("no scope", warlock.refusal(manifest(), "DP-1", PurePosixPath("README.md")))

    def test_no_ticket_is_refused(self):
        self.assertIn("through a ticket", warlock.refusal(manifest(), None, PurePosixPath("services/data/a")))

    def test_warlock_config_is_closed_to_agents(self):
        text = MANIFEST + '\n[[scope]]\nname = "root"\npaths = ["."]\ntracker = "linear"\nteam = "DP"\n'
        reason = warlock.refusal(warlock.parse_manifest(text), "DP-1", PurePosixPath(".warlock/scopes.toml"))
        self.assertIn("configuration", reason)


class RepoTestCase(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.root = Path(self.tmp.name).resolve()
        self.git("init", "-q", "-b", "main")
        self.git("config", "user.email", "test@example.com")
        self.git("config", "user.name", "Test")
        self.write(".warlock/scopes.toml", MANIFEST)
        self.write(".github/pull_request_template.md", PR_TEMPLATE)
        self.write("services/data/a.rs", "a")
        self.write("services/control/b.rs", "b")
        self.git("add", ".")
        self.git("commit", "-q", "-m", "init")
        self.env = mock.patch.dict(os.environ, {"WARLOCK_GATE": "", "WARLOCK_TICKET": ""})
        self.env.start()

    def tearDown(self):
        self.env.stop()
        self.tmp.cleanup()

    def git(self, *args):
        subprocess.run(["git", "-C", str(self.root), *args], check=True, capture_output=True)

    def write(self, rel, text):
        path = self.root / rel
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text)

    def run_cli(self, *args, stdin=None):
        out, err = io.StringIO(), io.StringIO()
        cwd = os.getcwd()
        os.chdir(self.root)
        try:
            with redirect_stdout(out), redirect_stderr(err), mock.patch("sys.stdin", io.StringIO(stdin or "")):
                code = warlock.main(list(args))
        finally:
            os.chdir(cwd)
        return code, out.getvalue(), err.getvalue()

    def gate(self, rel, tool="Edit"):
        event = {"tool_name": tool, "tool_input": {"file_path": str(self.root / rel)}, "cwd": str(self.root)}
        code, out, _ = self.run_cli("gate", stdin=json.dumps(event))
        self.assertEqual(code, 0)
        return json.loads(out)["hookSpecificOutput"]["permissionDecisionReason"] if out else None


class GateTests(RepoTestCase):
    def test_allows_writes_in_the_ticket_scope(self):
        self.git("checkout", "-q", "-b", "dp-12-retry")
        self.assertIsNone(self.gate("services/data/new.rs", tool="Write"))

    def test_denies_writes_in_another_team_scope(self):
        self.git("checkout", "-q", "-b", "dp-12-retry")
        self.assertIn("team CP", self.gate("services/control/b.rs"))

    def test_denies_everything_scoped_on_a_branch_with_no_ticket(self):
        self.assertIn("through a ticket", self.gate("services/data/a.rs"))

    def test_env_ticket_overrides_the_branch(self):
        os.environ["WARLOCK_TICKET"] = "CP-3"
        self.assertIsNone(self.gate("services/control/b.rs"))

    def test_off_switch(self):
        os.environ["WARLOCK_GATE"] = "off"
        self.assertIsNone(self.gate("services/control/b.rs"))

    def test_ignores_files_outside_the_repository(self):
        event = {"tool_name": "Write", "tool_input": {"file_path": "/tmp/elsewhere.txt"}, "cwd": str(self.root)}
        code, out, _ = self.run_cli("gate", stdin=json.dumps(event))
        self.assertEqual((code, out), (0, ""))

    def test_ignores_repositories_without_a_manifest(self):
        (self.root / ".warlock/scopes.toml").unlink()
        self.assertIsNone(self.gate("anything.txt"))

    def test_a_broken_manifest_denies(self):
        self.write(".warlock/scopes.toml", "[[scope")
        self.assertIn("not valid TOML", self.gate("services/data/a.rs"))

    def test_ignores_tools_that_do_not_write(self):
        event = {"tool_name": "Read", "tool_input": {"file_path": str(self.root / "README.md")}, "cwd": str(self.root)}
        self.assertEqual(self.run_cli("gate", stdin=json.dumps(event))[:2], (0, ""))


class ScopesTests(RepoTestCase):
    def test_json_reports_resolved_settings(self):
        self.git("checkout", "-q", "-b", "cp-4-thing")
        code, out, _ = self.run_cli("scopes", "--json")
        data = json.loads(out)
        self.assertEqual((code, data["ticket"], data["base"]), (0, "CP-4", "main"))
        self.assertEqual(data["scopes"][0]["settings"]["project_review_state"], "In Review")


class ProgressTests(RepoTestCase):
    def test_reads_finished_subtasks_from_commit_subjects(self):
        self.git("checkout", "-q", "-b", "dp-12-retry")
        for number in (1, 3):
            self.write("services/data/a.rs", str(number))
            self.git("commit", "-qam", f"DP-12.{number}: step {number}")
        self.git("commit", "-q", "--allow-empty", "-m", "DP-120.2: another ticket")
        self.write("services/data/a.rs", "half done")
        code, out, _ = self.run_cli("progress")
        data = json.loads(out)
        self.assertEqual((code, data["ticket"], data["finished"], data["dirty"]), (0, "DP-12", [1, 3], True))

    def test_needs_a_ticket(self):
        self.assertEqual(self.run_cli("progress")[0], 2)


class WhereTests(RepoTestCase):
    def test_reports_scopes_without_a_ticket(self):
        code, out, _ = self.run_cli("where", "services/data/a.rs", "services/control/b.rs")
        self.assertEqual(code, 0, out)

    def test_fails_an_unscoped_file(self):
        self.assertEqual(self.run_cli("where", "README.md")[0], 1)

    def test_fails_a_file_outside_the_ticket_scope(self):
        self.git("checkout", "-q", "-b", "dp-12-retry")
        self.assertEqual(self.run_cli("where", "services/control/b.rs")[0], 1)


class StopTests(RepoTestCase):
    def test_passes_a_clean_branch(self):
        self.git("checkout", "-q", "-b", "dp-12-retry")
        self.write("services/data/a.rs", "changed")
        self.assertEqual(self.run_cli("stop", stdin="{}")[0], 0)

    def test_blocks_once_on_a_shell_write_outside_scope(self):
        self.git("checkout", "-q", "-b", "dp-12-retry")
        self.write("services/control/b.rs", "changed by sed")
        code, _, err = self.run_cli("stop", stdin=json.dumps({"cwd": str(self.root)}))
        self.assertEqual(code, 2)
        self.assertIn("services/control/b.rs", err)

        code, out, _ = self.run_cli("stop", stdin=json.dumps({"cwd": str(self.root), "stop_hook_active": True}))
        self.assertEqual(code, 0)
        self.assertIn("systemMessage", out)

    def test_catches_committed_and_untracked_changes(self):
        self.git("checkout", "-q", "-b", "dp-12-retry")
        self.write("services/control/b.rs", "committed")
        self.git("commit", "-qam", "sneak")
        self.write("README.md", "untracked")
        _, _, err = self.run_cli("stop", stdin="{}")
        self.assertIn("services/control/b.rs", err)
        self.assertIn("README.md", err)


    def test_only_warns_on_a_branch_with_no_ticket(self):
        self.write("services/control/b.rs", "a person's edit")
        code, out, _ = self.run_cli("stop", stdin="{}")
        self.assertEqual(code, 0)
        self.assertIn("systemMessage", out)


class CiTests(RepoTestCase):
    def pr(self, branch, body, title="Add retry", labels=("warlock",)):
        env = {"PR_TITLE": title, "PR_BODY": body, "PR_HEAD_REF": branch, "PR_LABELS": json.dumps(labels)}
        with mock.patch.dict(os.environ, env):
            return self.run_cli("ci", "--base", "main")

    def test_passes_a_scoped_pr_that_follows_the_template(self):
        self.git("checkout", "-q", "-b", "dp-12-retry")
        self.write("services/data/a.rs", "changed")
        self.git("commit", "-qam", "change")
        code, out, _ = self.pr("dp-12-retry", PR_TEMPLATE)
        self.assertEqual(code, 0, out)

    def test_fails_a_crossing(self):
        self.git("checkout", "-q", "-b", "dp-12-retry")
        self.write("services/control/b.rs", "changed")
        self.git("commit", "-qam", "change")
        code, out, _ = self.pr("dp-12-retry", PR_TEMPLATE)
        self.assertEqual(code, 1)
        self.assertIn("::error file=services/control/b.rs", out)

    def test_skips_a_pull_request_without_the_label(self):
        self.git("checkout", "-q", "-b", "retry")
        self.write("README.md", "a person's edit, outside every scope")
        self.git("add", ".")
        self.git("commit", "-qm", "edit")
        code, out, _ = self.pr("retry", "", labels=())
        self.assertEqual(code, 0)
        self.assertIn("not Warlock's to check", out)

    def test_fails_without_a_ticket(self):
        self.git("checkout", "-q", "-b", "retry")
        code, out, _ = self.pr("retry", PR_TEMPLATE)
        self.assertEqual(code, 1)
        self.assertIn("must name a ticket", out)

    def test_fails_a_missing_template_section(self):
        self.git("checkout", "-q", "-b", "dp-12-retry")
        code, out, _ = self.pr("dp-12-retry", "## Ticket\n")
        self.assertEqual(code, 1)
        self.assertIn("## How I tested it", out)

    def test_reads_scopes_from_the_base_so_a_pr_cannot_widen_them(self):
        self.git("checkout", "-q", "-b", "dp-12-retry")
        self.write(".warlock/scopes.toml", MANIFEST.replace('"services/data"', '"services"'))
        self.write("services/control/b.rs", "changed")
        self.git("commit", "-qam", "widen")
        code, out, _ = self.pr("dp-12-retry", PR_TEMPLATE)
        self.assertEqual(code, 1)
        self.assertIn("services/control/b.rs", out)


    def test_passes_the_pr_that_adds_warlock(self):
        self.git("checkout", "-q", "-b", "setup")
        self.git("rm", "-q", "-r", ".warlock")
        self.git("commit", "-qm", "remove")
        self.git("branch", "-f", "main", "HEAD")
        self.write(".warlock/scopes.toml", MANIFEST)
        self.git("add", ".")
        self.git("commit", "-qm", "add warlock")
        code, out, _ = self.pr("setup", "")
        self.assertEqual(code, 0)
        self.assertIn("nothing to check", out)

if __name__ == "__main__":
    unittest.main()
