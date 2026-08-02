from __future__ import annotations

from contextlib import redirect_stderr, redirect_stdout
import io
import json
from pathlib import Path
import subprocess
import tempfile
import unittest

from albert_mvp.cli import main


class MissionSelectionCliTests(unittest.TestCase):
    def setUp(self) -> None:
        self.tmp = tempfile.TemporaryDirectory()
        self.root = Path(self.tmp.name)
        self.starting_location = self.root / "projects"
        self.coding_workspace = self.starting_location / "known-project"
        self.starting_location.mkdir()
        self.coding_workspace.mkdir()
        subprocess.run(
            ["git", "init", "--quiet", str(self.coding_workspace)],
            check=True,
            capture_output=True,
            text=True,
        )
        self.tracker = self.coding_workspace / ".agent" / "issues"
        self.tracker.mkdir(parents=True)
        (self.tracker / "PRD.md").write_text("# Existing Mission\n", encoding="utf-8")
        (self.tracker / "01-first.md").write_text(
            "Status: ready-for-agent\n"
            "Type: AFK\n\n"
            "## What to build\n\nA known Mission.\n\n"
            "## Acceptance criteria\n\n- [ ] Keep the Mission.\n\n"
            "## Blocked by\n\nNone - can start immediately\n",
            encoding="utf-8",
        )
        self.runtime_root = self.root / "runtime"

    def tearDown(self) -> None:
        self.tmp.cleanup()

    def run_cli(self, *argv: str) -> tuple[int, dict[str, object], str]:
        stdout = io.StringIO()
        stderr = io.StringIO()
        with redirect_stdout(stdout), redirect_stderr(stderr):
            exit_code = main(list(argv))
        payload = json.loads(stdout.getvalue()) if stdout.getvalue().strip() else {}
        return exit_code, payload, stderr.getvalue()

    def select_workspace(self) -> dict[str, object]:
        exit_code, acknowledgement, stderr = self.run_cli(
            "coding-workspace-select",
            "--starting-location",
            str(self.starting_location),
            "--workspace-path",
            str(self.coding_workspace),
            "--selection-mode",
            "existing",
            "--runtime-root",
            str(self.runtime_root),
            "--correlation-id",
            "select-known-project",
        )
        self.assertEqual(exit_code, 0, stderr)
        self.assertIsNone(acknowledgement["active_mission"])
        return acknowledgement

    def test_resume_known_mission_is_explicit_and_survives_a_fresh_cli_process(self) -> None:
        self.select_workspace()

        exit_code, options, stderr = self.run_cli(
            "mission-options",
            "--starting-location",
            str(self.starting_location),
            "--coding-workspace",
            str(self.coding_workspace),
            "--runtime-root",
            str(self.runtime_root),
        )
        self.assertEqual(exit_code, 0, stderr)
        self.assertEqual(options["phase"], "mission-choice-required")
        self.assertEqual(
            options["missions"],
            [{"id": "agent-issues", "title": "Existing Mission"}],
        )

        exit_code, acknowledgement, stderr = self.run_cli(
            "mission-choice",
            "--starting-location",
            str(self.starting_location),
            "--coding-workspace",
            str(self.coding_workspace),
            "--runtime-root",
            str(self.runtime_root),
            "--correlation-id",
            "resume-existing-mission",
            "--expected-revision",
            "1",
            "--choice",
            "resume",
            "--mission-id",
            "agent-issues",
        )
        self.assertEqual(exit_code, 0, stderr)
        self.assertEqual(acknowledgement["active_mission"], "agent-issues")
        self.assertEqual(acknowledgement["revision"], 2)

        # A new CLI invocation must read canonical state rather than infer the
        # Mission from the invocation directory or a recent-workspace hint.
        exit_code, restored, stderr = self.run_cli(
            "workspace-context",
            "--starting-location",
            str(self.starting_location),
            "--runtime-root",
            str(self.runtime_root),
        )
        self.assertEqual(exit_code, 0, stderr)
        self.assertEqual(restored["phase"], "workspace-ready")
        self.assertEqual(restored["coding_workspace"], str(self.coding_workspace.resolve()))
        self.assertEqual(restored["active_mission"], "agent-issues")

    def test_resumed_journey_catalog_produces_one_canonical_workspace_snapshot(self) -> None:
        self.select_workspace()
        exit_code, _, stderr = self.run_cli(
            "mission-choice",
            "--starting-location",
            str(self.starting_location),
            "--coding-workspace",
            str(self.coding_workspace),
            "--runtime-root",
            str(self.runtime_root),
            "--correlation-id",
            "resume-for-snapshot",
            "--expected-revision",
            "1",
            "--choice",
            "resume",
            "--mission-id",
            "agent-issues",
        )
        self.assertEqual(exit_code, 0, stderr)
        exit_code, context, stderr = self.run_cli(
            "workspace-context",
            "--starting-location",
            str(self.starting_location),
            "--runtime-root",
            str(self.runtime_root),
        )
        self.assertEqual(exit_code, 0, stderr)

        exit_code, snapshot, stderr = self.run_cli(
            "workspace-snapshot",
            "--target-repo",
            str(self.coding_workspace),
            "--tracker-dir",
            str(self.tracker),
            "--issues-dir",
            str(self.tracker),
            "--runtime-root",
            str(self.runtime_root),
            "--mission-id",
            "agent-issues",
            "--mission-catalog",
            str(context["mission_catalog"]),
        )

        self.assertEqual(exit_code, 0, stderr)
        self.assertEqual(snapshot["active_mission"]["id"], "agent-issues")
        self.assertEqual(
            [mission["id"] for mission in snapshot["missions"]],
            ["agent-issues"],
        )

    def test_start_new_mission_creates_distinct_identity_and_exact_replay(self) -> None:
        self.select_workspace()

        exit_code, created, stderr = self.run_cli(
            "mission-choice",
            "--starting-location",
            str(self.starting_location),
            "--coding-workspace",
            str(self.coding_workspace),
            "--runtime-root",
            str(self.runtime_root),
            "--correlation-id",
            "start-new-mission",
            "--expected-revision",
            "1",
            "--choice",
            "new",
            "--mission-id",
            "modernization",
            "--mission-title",
            "Modernization Mission",
        )
        self.assertEqual(exit_code, 0, stderr)
        self.assertEqual(created["active_mission"], "modernization")
        self.assertTrue((self.coding_workspace / ".alfredo/missions/modernization/PRD.md").exists())

        exit_code, replayed, stderr = self.run_cli(
            "mission-choice",
            "--starting-location",
            str(self.starting_location),
            "--coding-workspace",
            str(self.coding_workspace),
            "--runtime-root",
            str(self.runtime_root),
            "--correlation-id",
            "start-new-mission",
            "--expected-revision",
            "1",
            "--choice",
            "new",
            "--mission-id",
            "modernization",
            "--mission-title",
            "Modernization Mission",
        )
        self.assertEqual(exit_code, 0, stderr)
        self.assertTrue(replayed["replayed"])
        self.assertEqual(replayed["active_mission"], "modernization")

        exit_code, duplicate, stderr = self.run_cli(
            "mission-choice",
            "--starting-location",
            str(self.starting_location),
            "--coding-workspace",
            str(self.coding_workspace),
            "--runtime-root",
            str(self.runtime_root),
            "--correlation-id",
            "different-new-mission",
            "--expected-revision",
            "2",
            "--choice",
            "new",
            "--mission-id",
            "modernization",
            "--mission-title",
            "Modernization Mission",
        )
        self.assertEqual(exit_code, 1)
        self.assertEqual(duplicate, {})
        self.assertIn('"code": "mission-duplicate"', stderr)

        exit_code, restored, stderr = self.run_cli(
            "workspace-context",
            "--starting-location",
            str(self.starting_location),
            "--runtime-root",
            str(self.runtime_root),
        )
        self.assertEqual(exit_code, 0, stderr)
        self.assertEqual(restored["phase"], "workspace-ready")
        self.assertEqual(restored["active_mission"], "modernization")

    def test_invalid_resume_is_structured_and_leaves_mission_choice_pending(self) -> None:
        self.select_workspace()

        exit_code, _, stderr = self.run_cli(
            "mission-choice",
            "--starting-location",
            str(self.starting_location),
            "--coding-workspace",
            str(self.coding_workspace),
            "--runtime-root",
            str(self.runtime_root),
            "--correlation-id",
            "missing-mission",
            "--expected-revision",
            "1",
            "--choice",
            "resume",
            "--mission-id",
            "does-not-exist",
        )
        self.assertEqual(exit_code, 1)
        self.assertIn('"code": "mission-not-found"', stderr)

        exit_code, context, stderr = self.run_cli(
            "workspace-context",
            "--starting-location",
            str(self.starting_location),
            "--runtime-root",
            str(self.runtime_root),
        )
        self.assertEqual(exit_code, 0, stderr)
        self.assertEqual(context["phase"], "mission-choice-required")
        self.assertIsNone(context["active_mission"])


if __name__ == "__main__":
    unittest.main()
