from pathlib import Path
from contextlib import redirect_stderr, redirect_stdout
import io
import json
import os
import signal
import shutil
import subprocess
import sys
import tempfile
import threading
import time
import unittest
from unittest.mock import patch

import albert_mvp.core as core_module
from albert_mvp.cli import main
from albert_mvp.agents import AgentConfigError, load_agent_registry
from albert_mvp.core import (
    AlbertError,
    AlbertMission,
    DelegationDecision,
    EvidencePackage,
    EvidenceValidationError,
    LaunchBlockedError,
    LocalAgentSession,
    LockedFieldError,
)
from albert_mvp.tui import build_tui_state, perform_tui_action, render_tui_error, render_tui_state
from albert_mvp.workspace import ReviewWorkspaceService, WorkspaceSnapshotService


ISSUE_BODY = """Status: ready-for-agent
Type: {type}
Risk: {risk}
Suggested agent: {agent}
Assigned agent: {agent}

## Parent

PRD.md

## What to build

{what}

## Acceptance criteria

- [ ] {acceptance}

## Blocked by

{blocked_by}
"""


class AlbertMvpTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.root = Path(self.tmp.name)
        self.target_repo = self.root / "target"
        self.target_repo.mkdir()
        self.tracker = self.root / "tracker"
        self.issues = self.tracker / "issues"
        self.issues.mkdir(parents=True)
        self.runtime = self.root / "runtime"
        (self.tracker / "PRD.md").write_text(
            "# Local Coding Agent MVP Product Requirements Document\n\n## Problem Statement\n\nBuild Albert.\n",
            encoding="utf-8",
        )
        self.write_issue(
            "01-root.md",
            type="AFK",
            risk="Low",
            agent="qwen-coder-local-1",
            what="Create the root mission path.",
            acceptance="Mission summary is visible.",
            blocked_by="None - can start immediately",
        )
        self.write_issue(
            "02-child.md",
            type="AFK",
            risk="Medium",
            agent="qwen-coder-local-2",
            what="Create a dependent launch path.",
            acceptance="Launch waits for the root slice.",
            blocked_by="- ISS-01",
        )

    def tearDown(self):
        self.tmp.cleanup()

    def write_issue(self, filename, **values):
        (self.issues / filename).write_text(ISSUE_BODY.format(**values), encoding="utf-8")

    def load_mission(self):
        return AlbertMission(
            target_repo=self.target_repo,
            tracker_dir=self.tracker,
            runtime_root=self.runtime,
            mission_id="mission-001",
        ).load()

    def load_mission_with_agent_config(self, config_path):
        return AlbertMission(
            target_repo=self.target_repo,
            tracker_dir=self.tracker,
            runtime_root=self.runtime,
            mission_id="mission-001",
            agent_config_path=config_path,
        ).load()

    def test_loads_tracker_metadata_after_a_markdown_title_and_blank_line(self):
        (self.issues / "01-root.md").write_text(
            """# Completed Human Review

Status: complete
Type: HITL
Risk: Low
Suggested agent: qwen-coder-local-1
Assigned agent: qwen-coder-local-1

## Parent

PRD.md

## What to build

Confirm the completed work manually.

## Acceptance criteria

- [x] Human review is complete.

## Blocked by

None - can start immediately
""",
            encoding="utf-8",
        )

        issue = self.load_mission().issues["ISS-01"]

        self.assertEqual(issue.tracker_status, "complete")
        self.assertEqual(issue.type, "HITL")
        self.assertEqual(issue.review_state, "complete")
        self.assertEqual(issue.status, "complete")
        self.assertEqual(
            self.load_mission().board_summary()["issue_slices"][0]["work_type"],
            "HITL",
        )

    def test_missing_tracker_status_and_type_fail_closed_in_issue_summary(self):
        (self.issues / "03-missing-metadata.md").write_text(
            """# Missing Metadata

Risk: Medium
Suggested agent: qwen-coder-local-1
Assigned agent: qwen-coder-local-1

## What to build

Keep malformed tracker records out of active assignment projection.

## Acceptance criteria

- [ ] Missing execution metadata fails closed.

## Blocked by

None - can start immediately
""",
            encoding="utf-8",
        )

        summary = next(
            item
            for item in self.load_mission().board_summary()["issue_slices"]
            if item["issue_id"] == "ISS-03"
        )

        self.assertEqual(summary["tracker_status"], "")
        self.assertEqual(summary["work_type"], "")

    def run_cli(self, args):
        output = io.StringIO()
        with redirect_stdout(output):
            exit_code = main(args)
        return exit_code, output.getvalue()

    def run_cli_with_stderr(self, args):
        output = io.StringIO()
        error = io.StringIO()
        with redirect_stdout(output), redirect_stderr(error):
            exit_code = main(args)
        return exit_code, output.getvalue(), error.getvalue()

    def write_agent_config(self, agents):
        path = self.root / "agents.json"
        path.write_text(json.dumps({"agents": agents}), encoding="utf-8")
        return path

    def initialize_target_git_repo(self, files):
        for relative_path, content in files.items():
            path = self.target_repo / relative_path
            path.parent.mkdir(parents=True, exist_ok=True)
            if isinstance(content, tuple) and content[0] == "symlink":
                path.symlink_to(content[1])
            else:
                path.write_text(content, encoding="utf-8")
        subprocess.run(
            ["git", "init", str(self.target_repo)],
            check=True,
            capture_output=True,
            text=True,
        )
        subprocess.run(
            ["git", "-C", str(self.target_repo), "config", "user.email", "albert@example.test"],
            check=True,
        )
        subprocess.run(
            ["git", "-C", str(self.target_repo), "config", "user.name", "Albert Test"],
            check=True,
        )
        subprocess.run(
            ["git", "-C", str(self.target_repo), "add", "--all"],
            check=True,
        )
        subprocess.run(
            ["git", "-C", str(self.target_repo), "commit", "-m", "test fixture"],
            check=True,
            capture_output=True,
            text=True,
        )

    def test_loads_prd_and_issues_into_dependency_ordered_board(self):
        mission = self.load_mission()

        summary = mission.board_summary()

        self.assertEqual(summary["prd_title"], "Local Coding Agent MVP Product Requirements Document")
        self.assertEqual(summary["issue_count"], 2)
        self.assertEqual(summary["ordered_issue_ids"], ["ISS-01", "ISS-02"])
        self.assertEqual(summary["ready_issue_ids"], [])
        self.assertEqual(summary["issue_slices"][0]["lifecycle"], "Needs review")
        self.assertEqual((self.runtime / mission.project_key / "runtime.json").exists(), True)

    def test_board_summary_preserves_ready_tracker_status_without_auto_approval(self):
        mission = self.load_mission()

        issue = mission.board_summary()["issue_slices"][0]

        self.assertEqual(issue["tracker_status"], "ready-for-agent")
        self.assertEqual(issue["lifecycle"], "Needs review")
        self.assertEqual(issue["launch_eligible"], False)

    def test_loads_path_style_blocker_references_as_issue_ids(self):
        (self.issues / "03-path-blocked.md").write_text(
            ISSUE_BODY.format(
                type="AFK",
                risk="Medium",
                agent="qwen-coder-local-2",
                what="Create a path-reference dependent slice.",
                acceptance="Path blocker is understood.",
                blocked_by="- tracker/issues/02-child.md",
            ),
            encoding="utf-8",
        )

        mission = self.load_mission()

        self.assertEqual(mission.issues["ISS-03"].blocked_by, ["ISS-02"])
        self.assertEqual(mission.board_summary()["ready_issue_ids"], [])

    def test_ignores_path_style_blocker_references_outside_current_tracker(self):
        (self.issues / "03-external-blocked.md").write_text(
            ISSUE_BODY.format(
                type="AFK",
                risk="Medium",
                agent="qwen-coder-local-2",
                what="Create an externally sequenced slice.",
                acceptance="External tracker blocker does not become a local blocker.",
                blocked_by="- .agent/issues/29-add-alfredo-release-seam-verification.md",
            ),
            encoding="utf-8",
        )

        mission = self.load_mission()

        self.assertEqual(mission.issues["ISS-03"].blocked_by, [])
        self.assertEqual(mission.board_summary()["ordered_issue_ids"], ["ISS-01", "ISS-02", "ISS-03"])

    def test_issue_directory_can_also_contain_prd_record(self):
        (self.issues / "PRD.md").write_text("# Inline Product Requirements Document\n", encoding="utf-8")
        (self.issues / "03-parent-prd.md").write_text(
            "# Numbered Product Requirements Document\n\nStatus: ready-for-agent\nType: PRD\n",
            encoding="utf-8",
        )

        mission = self.load_mission()

        self.assertEqual(mission.board_summary()["ordered_issue_ids"], ["ISS-01", "ISS-02"])

    def test_loads_backtick_wrapped_bare_filename_blockers(self):
        (self.issues / "03-backtick-blocked.md").write_text(
            ISSUE_BODY.format(
                type="AFK",
                risk="Medium",
                agent="qwen-coder-local-2",
                what="Create a backtick-reference dependent slice.",
                acceptance="Bare filename blocker is understood.",
                blocked_by="- `02-child.md`",
            ),
            encoding="utf-8",
        )

        mission = self.load_mission()

        self.assertEqual(mission.issues["ISS-03"].blocked_by, ["ISS-02"])
        self.assertEqual(mission.board_summary()["ready_issue_ids"], [])

    def test_stale_runtime_does_not_override_tracker_contract_without_explicit_override(self):
        first = self.load_mission()
        runtime_path = self.runtime / first.project_key / "runtime.json"
        data = json.loads(runtime_path.read_text(encoding="utf-8"))
        data["issues"]["ISS-02"]["blocked_by"] = []
        runtime_path.write_text(json.dumps(data), encoding="utf-8")

        reloaded = self.load_mission()

        self.assertEqual(reloaded.issues["ISS-02"].blocked_by, ["ISS-01"])
        self.assertEqual(reloaded.board_summary()["ready_issue_ids"], [])

    def test_complete_issue_status_satisfies_dependent_blockers(self):
        self.write_issue(
            "01-root.md",
            type="AFK",
            risk="Low",
            agent="qwen-coder-local-1",
            what="Create the root mission path.",
            acceptance="Mission summary is visible.",
            blocked_by="None - can start immediately",
        )
        text = (self.issues / "01-root.md").read_text(encoding="utf-8")
        (self.issues / "01-root.md").write_text(text.replace("Status: ready-for-agent", "Status: complete"), encoding="utf-8")

        mission = self.load_mission()

        self.assertEqual(mission.issues["ISS-01"].review_state, "complete")
        summary = mission.board_summary()
        self.assertEqual(summary["ready_issue_ids"], [])
        self.assertEqual(summary["issue_slices"][0]["lifecycle"], "Merged")
        mission.approve_issue("ISS-02")
        self.assertEqual(mission.board_summary()["ready_issue_ids"], ["ISS-02"])

    def test_stale_runtime_does_not_override_complete_tracker_status(self):
        first = self.load_mission()
        runtime_path = self.runtime / first.project_key / "runtime.json"
        text = (self.issues / "01-root.md").read_text(encoding="utf-8")
        (self.issues / "01-root.md").write_text(text.replace("Status: ready-for-agent", "Status: complete"), encoding="utf-8")

        data = json.loads(runtime_path.read_text(encoding="utf-8"))
        data["issues"]["ISS-01"]["status"] = "ready-for-agent"
        data["issues"]["ISS-01"]["review_state"] = "needs-review"
        runtime_path.write_text(json.dumps(data), encoding="utf-8")

        reloaded = self.load_mission()

        self.assertEqual(reloaded.issues["ISS-01"].status, "complete")
        self.assertEqual(reloaded.issues["ISS-01"].review_state, "complete")

    def test_runtime_state_is_isolated_by_tracker_and_mission(self):
        text = (self.issues / "01-root.md").read_text(encoding="utf-8")
        (self.issues / "01-root.md").write_text(text.replace("Status: ready-for-agent", "Status: complete"), encoding="utf-8")
        first = self.load_mission()
        self.assertEqual(first.issues["ISS-01"].review_state, "complete")
        second_tracker = self.root / "second-tracker"
        second_issues = second_tracker / "issues"
        second_issues.mkdir(parents=True)
        (second_tracker / "PRD.md").write_text("# Second Product Requirements Document\n", encoding="utf-8")
        (second_issues / "01-root.md").write_text(
            ISSUE_BODY.format(
                type="AFK",
                risk="Low",
                agent="qwen-coder-local-1",
                what="Create the independent mission path.",
                acceptance="Independent mission is isolated.",
                blocked_by="None - can start immediately",
            ),
            encoding="utf-8",
        )

        second = AlbertMission(
            target_repo=self.target_repo,
            tracker_dir=second_tracker,
            runtime_root=self.runtime,
            mission_id="mission-002",
        ).load()

        self.assertNotEqual(first.project_key, second.project_key)
        self.assertEqual(second.issues["ISS-01"].review_state, "needs-review")

    def test_runtime_identity_casefolds_only_wsl_windows_mount_paths(self):
        windows_alias = AlbertMission(
            target_repo=Path("/mnt/c/Users/Example/Repo"),
            tracker_dir=Path("/mnt/c/Users/Example/Repo/.scratch/mission"),
            runtime_root=self.runtime,
            mission_id="mission-case",
        )
        windows_lowercase = AlbertMission(
            target_repo=Path("/mnt/c/users/example/repo"),
            tracker_dir=Path("/mnt/c/users/example/repo/.scratch/mission"),
            runtime_root=self.runtime,
            mission_id="mission-case",
        )
        linux_alias = AlbertMission(
            target_repo=Path("/tmp/CaseIdentity/Repo"),
            tracker_dir=Path("/tmp/CaseIdentity/Repo/tracker"),
            runtime_root=self.runtime,
            mission_id="mission-case",
        )
        linux_lowercase = AlbertMission(
            target_repo=Path("/tmp/caseidentity/repo"),
            tracker_dir=Path("/tmp/caseidentity/repo/tracker"),
            runtime_root=self.runtime,
            mission_id="mission-case",
        )

        self.assertEqual(windows_alias.project_key, windows_lowercase.project_key)
        self.assertNotEqual(windows_alias.target_repo, windows_lowercase.target_repo)
        self.assertNotEqual(linux_alias.project_key, linux_lowercase.project_key)

    def test_complete_issue_cannot_be_reapproved_for_launch(self):
        text = (self.issues / "01-root.md").read_text(encoding="utf-8")
        (self.issues / "01-root.md").write_text(text.replace("Status: ready-for-agent", "Status: complete"), encoding="utf-8")
        mission = self.load_mission()

        with self.assertRaisesRegex(AlbertError, "ISS-01 is already complete"):
            mission.approve_issue("ISS-01")

    def test_pr_ready_blockers_are_launchable_and_match_board_readiness(self):
        mission = self.load_mission()
        mission.approve_issue("ISS-01")
        session = mission.launch_issue("ISS-01")
        mission.record_evidence(
            session.session_id,
            EvidencePackage(
                changed_files=["src/app.py"],
                diff_summary="Added mission board.",
                commands_run=["python -m unittest"],
                test_results="All tests passed.",
                known_risks="None.",
                proposed_context_updates="No glossary changes.",
            ),
        )
        mission.record_frontier_review(session.session_id, "Approved", reason="Meets criteria.")

        reloaded = self.load_mission()
        self.assertEqual(reloaded.issues["ISS-01"].review_state, "pr-ready")
        summary = reloaded.board_summary()
        self.assertEqual(summary["ready_issue_ids"], [])
        self.assertEqual(summary["issue_slices"][0]["lifecycle"], "Complete")

        reloaded.approve_issue("ISS-02")
        self.assertEqual(reloaded.board_summary()["ready_issue_ids"], ["ISS-02"])
        child_session = reloaded.launch_issue("ISS-02")

        self.assertEqual(child_session.issue_id, "ISS-02")

    def test_issue_detail_shows_tracker_runtime_blockers_and_next_actions(self):
        mission = self.load_mission()
        mission.assign_issue("ISS-02", "deepseek-local", notes="Use repair-capable model.")

        detail = mission.issue_detail("ISS-02")

        self.assertEqual(detail["issue_id"], "ISS-02")
        self.assertEqual(detail["tracker_status"], "ready-for-agent")
        self.assertEqual(detail["runtime_status"], "needs-review")
        self.assertEqual(detail["review_state"], "needs-review")
        self.assertEqual(detail["assigned_agent"], "deepseek-local")
        self.assertEqual(detail["blockers"], [{"issue_id": "ISS-01", "review_state": "needs-review", "satisfied": False}])
        self.assertIn("approve", detail["next_actions"])
        self.assertNotIn("launch", detail["next_actions"])

    def test_reopen_completed_issue_is_explicit_and_preserves_history(self):
        text = (self.issues / "01-root.md").read_text(encoding="utf-8")
        (self.issues / "01-root.md").write_text(text.replace("Status: ready-for-agent", "Status: complete"), encoding="utf-8")
        mission = self.load_mission()
        mission.assign_issue("ISS-01", "qwen3.6-27b", notes="Keep this assignment.")
        timeline_before = list(mission.timeline)

        mission.reopen_issue("ISS-01", reason="Acceptance criteria need another pass.")
        reloaded = self.load_mission()

        self.assertEqual(reloaded.issues["ISS-01"].review_state, "needs-review")
        self.assertEqual(reloaded.issues["ISS-01"].status, "needs-review")
        self.assertEqual(reloaded.issues["ISS-01"].locked, False)
        self.assertEqual(reloaded.issues["ISS-01"].assigned_agent, "qwen3.6-27b")
        self.assertEqual(reloaded.issues["ISS-01"].notes, "Keep this assignment.")
        self.assertGreater(len(reloaded.timeline), len(timeline_before))
        self.assertIn("reopened for re-review", reloaded.timeline[-1])
        self.assertIn("Status: needs-review", (self.issues / "01-root.md").read_text(encoding="utf-8"))

    def test_cli_reopen_and_show_report_actionable_state(self):
        text = (self.issues / "01-root.md").read_text(encoding="utf-8")
        (self.issues / "01-root.md").write_text(text.replace("Status: ready-for-agent", "Status: complete"), encoding="utf-8")
        base_args = [
            "--target-repo",
            str(self.target_repo),
            "--tracker-dir",
            str(self.tracker),
            "--runtime-root",
            str(self.runtime),
            "--mission-id",
            "mission-001",
        ]

        exit_code, output = self.run_cli(["reopen", *base_args, "ISS-01", "--reason", "Repair requested."])
        self.assertEqual(exit_code, 0)
        self.assertIn("ISS-01 reopened for re-review.", output)

        exit_code, output = self.run_cli(["show", *base_args, "ISS-01"])
        self.assertEqual(exit_code, 0)
        self.assertIn("Tracker status: needs-review", output)
        self.assertIn("Runtime status: needs-review", output)
        self.assertIn("Next actions: approve", output)

        exit_code, _, error = self.run_cli_with_stderr(["reopen", *base_args, "ISS-99", "--reason", "No such slice."])
        self.assertEqual(exit_code, 1)
        self.assertIn("Error: Unknown Issue Slice: ISS-99", error)
        self.assertNotIn("Traceback", error)

    def test_cli_board_command_renders_mission_summary(self):
        output = io.StringIO()

        exit_code, rendered = self.run_cli(
            [
                "board",
                "--target-repo",
                str(self.target_repo),
                "--tracker-dir",
                str(self.tracker),
                "--runtime-root",
                str(self.runtime),
                "--mission-id",
                "mission-001",
            ]
        )

        self.assertEqual(exit_code, 0)
        self.assertIn("Local Coding Agent MVP Product Requirements Document", rendered)
        self.assertIn("ISS-01", rendered)
        self.assertIn("Ready: None", rendered)

    def test_tui_state_lists_ordered_slices_readiness_and_next_action(self):
        mission = self.load_mission()
        mission.assign_issue("ISS-01", "qwen3.6-27b", notes="Primary big local model.")

        state = build_tui_state(mission)

        self.assertEqual(state.title, "Local Coding Agent MVP Product Requirements Document")
        self.assertEqual(state.next_action, "Approve ISS-01")
        self.assertEqual([row.issue_id for row in state.rows], ["ISS-01", "ISS-02"])
        self.assertEqual(state.rows[0].readiness, "ready")
        self.assertEqual(state.rows[0].assigned_agent, "qwen3.6-27b")
        self.assertEqual(state.rows[1].readiness, "blocked")
        self.assertEqual(state.rows[1].blockers, ["ISS-01"])

    def test_tui_state_includes_selected_issue_details(self):
        mission = self.load_mission()

        state = build_tui_state(mission, selected_issue_id="ISS-02")

        self.assertEqual(state.selected.issue_id, "ISS-02")
        self.assertEqual(state.selected.acceptance_criteria, ["Launch waits for the root slice."])
        self.assertEqual(state.selected.blocked_by, ["ISS-01"])
        self.assertEqual(state.selected.next_actions, ["approve"])

    def test_tui_renderer_exposes_futuristic_mission_control_surface(self):
        mission = self.load_mission()
        state = build_tui_state(mission, selected_issue_id="ISS-01")

        rendered = render_tui_state(state)

        self.assertIn("ALBERT // MISSION CONTROL", rendered)
        self.assertIn("NEXT: Approve ISS-01", rendered)
        self.assertIn("ISS-01", rendered)
        self.assertIn("READY", rendered)
        self.assertIn("DETAIL // ISS-01", rendered)
        self.assertIn("Mission summary is visible.", rendered)

    def test_tui_error_renderer_is_readable_for_missing_tracker_state(self):
        rendered = render_tui_error("Missing issues directory: /tmp/missing/issues")

        self.assertIn("ALBERT // MISSION CONTROL", rendered)
        self.assertIn("TUI cannot load mission state", rendered)
        self.assertIn("Missing issues directory", rendered)

    def test_cli_tui_command_renders_mission_control(self):
        exit_code, rendered = self.run_cli(
            [
                "tui",
                "--target-repo",
                str(self.target_repo),
                "--tracker-dir",
                str(self.tracker),
                "--runtime-root",
                str(self.runtime),
                "--mission-id",
                "mission-001",
                "--select",
                "ISS-01",
            ]
        )

        self.assertEqual(exit_code, 0)
        self.assertIn("ALBERT // MISSION CONTROL", rendered)
        self.assertIn("DETAIL // ISS-01", rendered)

    def test_agent_registry_loads_configured_ollama_and_command_agents(self):
        config_path = self.write_agent_config(
            [
                {
                    "id": "qwen3.6-27b",
                    "role": "frontier",
                    "provider": "ollama",
                    "runner": "ollama",
                    "model": "qwen3.6:27b",
                },
                {
                    "id": "deepseek-v4",
                    "role": "local-agent",
                    "provider": "ollama",
                    "runner": "ollama",
                    "model": "deepseek-v4",
                },
                {
                    "id": "shell-fake",
                    "role": "local-agent",
                    "provider": "local",
                    "runner": "command",
                    "command": "python3 -m albert_mvp.fake_agent",
                },
            ]
        )

        registry = load_agent_registry(config_path)

        self.assertEqual([agent.id for agent in registry.agents], ["qwen3.6-27b", "deepseek-v4", "shell-fake"])
        self.assertEqual(registry.require("qwen3.6-27b").model, "qwen3.6:27b")
        self.assertEqual(registry.require("shell-fake").command, "python3 -m albert_mvp.fake_agent")

    def test_ollama_default_runner_command_requests_machine_readable_output(self):
        config_path = self.write_agent_config(
            [
                {
                    "id": "qwen3.6-27b",
                    "role": "frontier",
                    "provider": "ollama",
                    "runner": "ollama",
                    "model": "qwen3.6:27b",
                }
            ]
        )
        mission = self.load_mission_with_agent_config(config_path)
        agent = mission.agent_registry.require("qwen3.6-27b")

        command = mission._runner_command(agent)

        self.assertEqual(command, "ollama run qwen3.6:27b --think=false --nowordwrap --format json")
        self.assertEqual(mission.classify_command(command), "auto-allowed")

    def test_mission_board_exposes_provider_neutral_unavailable_model_assignment(self):
        config_path = self.write_agent_config(
            [
                {
                    "id": "gemma4-12b",
                    "role": "local-agent",
                    "provider": "ollama",
                    "runner": "ollama",
                    "model": "gemma4:12b",
                    "availability": "unavailable",
                    "availability_reason": "Model is not installed locally.",
                }
            ]
        )
        mission = self.load_mission_with_agent_config(config_path)
        mission.assign_issue("ISS-01", "gemma4-12b")
        mission.approve_issue("ISS-01")

        issue = mission.board_summary()["issue_slices"][0]

        self.assertEqual(
            issue["model_assignment"],
            {
                "agent_id": "gemma4-12b",
                "role": "local-agent",
                "provider": "ollama",
                "model": "gemma4:12b",
                "availability": "unavailable",
                "availability_reason": "Model is not installed locally.",
                "operation_status": "idle",
                "failure": "",
            },
        )
        self.assertEqual(issue["lifecycle"], "Approved")
        self.assertEqual(issue["progress"], "Assigned model unavailable: Model is not installed locally.")
        self.assertEqual(issue["launch_eligible"], False)

    def test_provider_neutral_fake_adapter_uses_same_available_assignment_contract(self):
        config_path = self.write_agent_config(
            [
                {
                    "id": "neutral-fake",
                    "role": "local-agent",
                    "provider": "test-harness",
                    "runner": "fake",
                    "model": "deterministic-fake",
                }
            ]
        )
        mission = self.load_mission_with_agent_config(config_path)
        mission.assign_issue("ISS-01", "neutral-fake")
        mission.approve_issue("ISS-01")

        session = mission.launch_issue("ISS-01")
        mission.run_session(session.session_id)

        issue = mission.board_summary()["issue_slices"][0]
        self.assertEqual(session.status, "evidence-ready")
        self.assertEqual(
            issue["model_assignment"],
            {
                "agent_id": "neutral-fake",
                "role": "local-agent",
                "provider": "test-harness",
                "model": "deterministic-fake",
                "availability": "available",
                "availability_reason": "",
                "operation_status": "evidence-ready",
                "failure": "",
            },
        )
        self.assertEqual(issue["sessions"][0]["operation_status"], "evidence-ready")

    @unittest.skipUnless(
        os.environ.get("ALBERT_OLLAMA_SMOKE") == "1",
        "set ALBERT_OLLAMA_SMOKE=1 to run local Ollama smoke verification",
    )
    def test_local_ollama_smoke_is_separate_from_provider_neutral_ci(self):
        result = subprocess.run(
            ["ollama", "list"],
            check=False,
            capture_output=True,
            text=True,
            timeout=10,
        )

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("NAME", result.stdout)

    def test_disconnected_model_assignment_is_visible_before_launch(self):
        config_path = self.write_agent_config(
            [
                {
                    "id": "qwen3.6-27b",
                    "role": "local-agent",
                    "provider": "ollama",
                    "runner": "ollama",
                    "model": "qwen3.6:27b",
                    "availability": "disconnected",
                    "availability_reason": "Ollama daemon is not reachable.",
                }
            ]
        )
        mission = self.load_mission_with_agent_config(config_path)
        mission.assign_issue("ISS-01", "qwen3.6-27b")
        mission.approve_issue("ISS-01")

        issue = mission.board_summary()["issue_slices"][0]

        self.assertEqual(issue["model_assignment"]["availability"], "disconnected")
        self.assertEqual(issue["progress"], "Assigned model unavailable: Ollama daemon is not reachable.")
        self.assertEqual(issue["lifecycle"], "Approved")
        self.assertEqual(issue["launch_eligible"], False)

    def test_unavailable_model_blocks_launch_without_mutating_issue_lifecycle(self):
        config_path = self.write_agent_config(
            [
                {
                    "id": "gemma4-12b",
                    "role": "local-agent",
                    "provider": "ollama",
                    "runner": "ollama",
                    "model": "gemma4:12b",
                    "availability": "unavailable",
                    "availability_reason": "Model is not installed locally.",
                }
            ]
        )
        mission = self.load_mission_with_agent_config(config_path)
        mission.assign_issue("ISS-01", "gemma4-12b")
        mission.approve_issue("ISS-01")

        with self.assertRaisesRegex(LaunchBlockedError, "assigned model is unavailable"):
            mission.launch_issue("ISS-01")

        issue = mission.board_summary()["issue_slices"][0]
        self.assertEqual(issue["lifecycle"], "Approved")
        self.assertEqual(issue["model_assignment"]["operation_status"], "idle")
        self.assertEqual(mission.issues["ISS-01"].review_state, "approved")
        self.assertEqual(mission.sessions, {})

    def test_unknown_assignment_in_configured_registry_cannot_queue(self):
        config_path = self.write_agent_config(
            [
                {
                    "id": "known-worker",
                    "role": "local-agent",
                    "provider": "local",
                    "runner": "fake",
                    "routing": "worker",
                }
            ]
        )
        mission = self.load_mission_with_agent_config(config_path)
        mission.issues["ISS-01"].assigned_agent = "missing-worker"
        mission.approve_issue("ISS-01")

        with self.assertRaisesRegex(LaunchBlockedError, "assigned model is unavailable"):
            mission.launch_issue("ISS-01")

        self.assertEqual(mission.sessions, {})
        assignment = mission.board_summary()["issue_slices"][0]["model_assignment"]
        self.assertEqual(assignment["availability"], "unavailable")
        self.assertIn("not configured", assignment["availability_reason"])

    def test_missing_agent_registry_keeps_issue_metadata_assignments_allowed(self):
        missing_config = self.root / "missing-agents.json"
        mission = self.load_mission_with_agent_config(missing_config)

        mission.assign_issue("ISS-01", "qwen-coder-local-9")

        self.assertEqual(mission.issues["ISS-01"].assigned_agent, "qwen-coder-local-9")
        self.assertEqual(mission.list_agents(), [])

    def test_invalid_agent_registry_reports_broken_entry(self):
        config_path = self.write_agent_config(
            [
                {
                    "id": "deepseek-v4",
                    "role": "local-agent",
                    "provider": "ollama",
                    "runner": "ollama",
                }
            ]
        )

        with self.assertRaisesRegex(AgentConfigError, "deepseek-v4.*model"):
            load_agent_registry(config_path)

    def test_agent_registry_rejects_string_values_for_boolean_governance_fields(self):
        for field_name in ("assignable", "delegate_only", "requires_approval"):
            with self.subTest(field_name=field_name):
                config_path = self.write_agent_config(
                    [
                        {
                            "id": "unsafe-worker",
                            "role": "local-agent",
                            "provider": "local",
                            "runner": "fake",
                            field_name: "false",
                        }
                    ]
                )
                with self.assertRaisesRegex(
                    AgentConfigError,
                    f"{field_name!r} must be a JSON boolean",
                ):
                    load_agent_registry(config_path)

    def test_configured_registry_rejects_unknown_assignment(self):
        config_path = self.write_agent_config(
            [
                {
                    "id": "qwen3.6-27b",
                    "role": "frontier",
                    "provider": "ollama",
                    "runner": "ollama",
                    "model": "qwen3.6:27b",
                }
            ]
        )
        mission = self.load_mission_with_agent_config(config_path)

        with self.assertRaisesRegex(AlbertError, "Unknown configured agent: kimi-k2.6"):
            mission.assign_issue("ISS-01", "kimi-k2.6")

    def test_cli_lists_configured_agents(self):
        config_path = self.write_agent_config(
            [
                {
                    "id": "qwen3.6-27b",
                    "role": "frontier",
                    "provider": "ollama",
                    "runner": "ollama",
                    "model": "qwen3.6:27b",
                },
                {
                    "id": "kimi-k2.6",
                    "role": "local-agent",
                    "provider": "ollama",
                    "runner": "ollama",
                    "model": "kimi-k2.6",
                },
            ]
        )

        exit_code, output = self.run_cli(
            [
                "agents",
                "--target-repo",
                str(self.target_repo),
                "--tracker-dir",
                str(self.tracker),
                "--runtime-root",
                str(self.runtime),
                "--mission-id",
                "mission-001",
                "--agent-config",
                str(config_path),
            ]
        )

        self.assertEqual(exit_code, 0)
        self.assertIn("qwen3.6-27b", output)
        self.assertIn("frontier", output)
        self.assertIn("ollama:qwen3.6:27b", output)
        self.assertIn("kimi-k2.6", output)

    def test_tui_state_shows_available_configured_agents_for_assignment(self):
        config_path = self.write_agent_config(
            [
                {
                    "id": "deepseek-v4",
                    "role": "local-agent",
                    "provider": "ollama",
                    "runner": "ollama",
                    "model": "deepseek-v4",
                },
                {
                    "id": "kimi-k2.6",
                    "role": "local-agent",
                    "provider": "ollama",
                    "runner": "ollama",
                    "model": "kimi-k2.6",
                },
            ]
        )
        mission = self.load_mission_with_agent_config(config_path)

        state = build_tui_state(mission, selected_issue_id="ISS-01")

        self.assertEqual(state.selected.available_agents, ["deepseek-v4", "kimi-k2.6"])

    def test_delegate_only_agents_are_hidden_from_manual_assignment(self):
        config_path = self.write_agent_config(
            [
                {
                    "id": "qwen3.6-27b",
                    "role": "frontier",
                    "provider": "ollama",
                    "runner": "ollama",
                    "model": "qwen3.6:27b",
                    "routing": "router",
                },
                {
                    "id": "gemma4-12b",
                    "role": "local-agent",
                    "provider": "ollama",
                    "runner": "ollama",
                    "model": "gemma4:12b",
                    "routing": "worker",
                },
                {
                    "id": "kimi-k2.6-cloud",
                    "role": "delegate-agent",
                    "provider": "ollama",
                    "runner": "ollama",
                    "model": "kimi-k2.6:cloud",
                    "routing": "delegate",
                    "delegate_only": True,
                    "requires_approval": True,
                },
            ]
        )
        mission = self.load_mission_with_agent_config(config_path)

        state = build_tui_state(mission, selected_issue_id="ISS-01")

        self.assertEqual(state.selected.available_agents, ["gemma4-12b"])
        with self.assertRaisesRegex(AlbertError, "delegate-only"):
            mission.assign_issue("ISS-01", "kimi-k2.6-cloud")

    def test_tracker_assigned_controller_cannot_bypass_worker_launch_governance(self):
        self.write_issue(
            "01-root.md",
            type="AFK",
            risk="Low",
            agent="qwen-controller",
            what="Create the root mission path.",
            acceptance="Mission summary is visible.",
            blocked_by="None - can start immediately",
        )
        config_path = self.write_agent_config(
            [
                {
                    "id": "qwen-controller",
                    "role": "frontier",
                    "provider": "ollama",
                    "runner": "ollama",
                    "model": "qwen3:14b",
                    "routing": "controller",
                },
                {
                    "id": "local-worker",
                    "role": "local-agent",
                    "provider": "local",
                    "runner": "fake",
                    "routing": "worker",
                },
            ]
        )
        mission = self.load_mission_with_agent_config(config_path)
        mission.approve_issue("ISS-01")

        summary = mission.board_summary()["issue_slices"][0]
        self.assertEqual(summary["launch_eligible"], False)
        self.assertIn("not launchable", summary["progress"])
        with self.assertRaisesRegex(LaunchBlockedError, "not launchable"):
            mission.launch_issue("ISS-01")

        self.assertEqual(mission.sessions, {})

    def test_qwen_router_delegates_complex_issue_to_gated_delegate(self):
        router = self.root / "qwen_router.py"
        router.write_text(
            "import json, sys\n"
            "prompt = sys.stdin.read()\n"
            "assert 'ISS-01' in prompt\n"
            "print(json.dumps({\n"
            "  'complexity': 'high',\n"
            "  'recommended_agent': 'kimi-k2.6-cloud',\n"
            "  'requires_cloud_approval': True,\n"
            "  'reason': 'Long-horizon coding is too complex for Gemma.'\n"
            "}))\n",
            encoding="utf-8",
        )
        fake_delegate = self.root / "fake_delegate_ollama.py"
        fake_delegate.write_text(
            "import json, sys\n"
            "prompt = sys.stdin.read()\n"
            "assert 'delegation' in prompt\n"
            "print(json.dumps({\n"
            "  'summary': 'Created delegated prototype',\n"
            "  'files': [{'path': 'prototype_app.py', 'content': 'print(\"delegated\")\\n'}]\n"
            "}))\n",
            encoding="utf-8",
        )
        router_command = f"{sys.executable} {router}"
        delegate_command = f"{sys.executable} {fake_delegate}"
        config_path = self.write_agent_config(
            [
                {
                    "id": "qwen3.6-27b",
                    "role": "frontier",
                    "provider": "ollama",
                    "runner": "ollama",
                    "model": "qwen3.6:27b",
                    "command": router_command,
                    "routing": "router",
                },
                {
                    "id": "gemma4-12b",
                    "role": "local-agent",
                    "provider": "ollama",
                    "runner": "ollama",
                    "model": "gemma4:12b",
                    "routing": "worker",
                },
                {
                    "id": "kimi-k2.6-cloud",
                    "role": "delegate-agent",
                    "provider": "ollama",
                    "runner": "ollama",
                    "model": "kimi-k2.6:cloud",
                    "command": delegate_command,
                    "routing": "delegate",
                    "delegate_only": True,
                    "requires_approval": True,
                },
            ]
        )
        mission = self.load_mission_with_agent_config(config_path)
        mission.record_command_approval(router_command, "auto-allowed")
        mission.approve_issue("ISS-01")

        decision = mission.route_issue("ISS-01")

        self.assertEqual(decision.router_agent, "qwen3.6-27b")
        self.assertEqual(decision.recommended_agent, "kimi-k2.6-cloud")
        self.assertEqual(decision.complexity, "high")
        self.assertEqual(decision.reason, "Long-horizon coding is too complex for Gemma.")
        self.assertEqual(decision.requires_approval, True)
        self.assertEqual(decision.approved, False)
        self.assertEqual(mission.issues["ISS-01"].assigned_agent, "kimi-k2.6-cloud")
        self.assertIn("approve-delegation", mission.issue_detail("ISS-01")["next_actions"])
        with self.assertRaisesRegex(LaunchBlockedError, "delegation requires approval"):
            mission.launch_issue("ISS-01")

        mission.approve_delegation("ISS-01")
        session = mission.launch_issue("ISS-01", allowed_paths=["prototype_app.py"])
        mission.run_session(session.session_id)

        self.assertEqual(session.status, "evidence-ready")
        self.assertEqual(session.task_packet["delegation"]["router_agent"], "qwen3.6-27b")
        self.assertEqual(session.task_packet["delegation"]["recommended_agent"], "kimi-k2.6-cloud")
        self.assertTrue((session.worktree_path / "prototype_app.py").exists())

    def test_uppercase_cloud_delegate_requires_approval_before_deferred_launch(self):
        router = self.root / "uppercase_cloud_router.py"
        router.write_text(
            "import json, sys\n"
            "sys.stdin.read()\n"
            "print(json.dumps({\n"
            "  'complexity': 'high',\n"
            "  'recommended_agent': 'cloud-delegate',\n"
            "  'requires_approval': False,\n"
            "  'reason': 'Exercise the governed cloud boundary.'\n"
            "}))\n",
            encoding="utf-8",
        )
        router_command = f"{sys.executable} {router}"
        config_path = self.write_agent_config(
            [
                {
                    "id": "router",
                    "role": "frontier",
                    "provider": "local",
                    "runner": "command",
                    "command": router_command,
                    "routing": "router",
                },
                {
                    "id": "cloud-delegate",
                    "role": "delegate-agent",
                    "provider": "ollama",
                    "runner": "fake",
                    "model": "remote:CLOUD",
                    "routing": "delegate",
                    "delegate_only": True,
                },
            ]
        )
        mission = self.load_mission_with_agent_config(config_path)
        mission.record_command_approval(router_command, "auto-allowed")
        mission.approve_issue("ISS-01")

        decision = mission.route_issue("ISS-01")

        self.assertTrue(decision.requires_approval)
        with self.assertRaisesRegex(LaunchBlockedError, "requires approval"):
            mission.launch_issue("ISS-01")
        mission.approve_delegation("ISS-01")
        queued = mission.launch_issue("ISS-01")
        completed = mission.run_session(queued.session_id)
        self.assertEqual(completed.status, "evidence-ready")

    def test_router_command_has_read_only_repo_minimal_host_view_and_sanitized_env(self):
        secret = self.root / "router-host-secret.txt"
        secret.write_text("must-not-reach-router", encoding="utf-8")
        marker = self.target_repo / "ROUTER_MUST_NOT_WRITE.txt"
        router = self.root / "bounded_router.py"
        router.write_text(
            "import json, os, pathlib, sys\n"
            "failures = []\n"
            f"marker = pathlib.Path({str(marker)!r})\n"
            f"secret = pathlib.Path({str(secret)!r})\n"
            "try:\n"
            "    marker.write_text('escaped')\n"
            "except OSError:\n"
            "    pass\n"
            "else:\n"
            "    failures.append('repo-write')\n"
            "try:\n"
            "    secret.read_text()\n"
            "except OSError:\n"
            "    pass\n"
            "else:\n"
            "    failures.append('host-read')\n"
            "if 'OPENAI_API_KEY' in os.environ:\n"
            "    failures.append('credential')\n"
            "if failures:\n"
            "    print(','.join(failures), file=sys.stderr)\n"
            "    raise SystemExit(9)\n"
            "sys.stdin.read()\n"
            "print(json.dumps({\n"
            "  'complexity': 'low',\n"
            "  'recommended_agent': 'worker',\n"
            "  'requires_approval': False,\n"
            "  'reason': 'A bounded worker is sufficient.'\n"
            "}))\n",
            encoding="utf-8",
        )
        router_command = f"{sys.executable} {router}"
        config_path = self.write_agent_config(
            [
                {
                    "id": "controller",
                    "role": "frontier",
                    "provider": "command",
                    "runner": "command",
                    "command": router_command,
                    "routing": "router",
                },
                {
                    "id": "worker",
                    "role": "local-agent",
                    "provider": "local",
                    "runner": "fake",
                    "routing": "worker",
                },
            ]
        )
        mission = self.load_mission_with_agent_config(config_path)
        mission.record_command_approval(router_command, "auto-allowed")
        mission.approve_issue("ISS-01")

        with patch.dict(os.environ, {"OPENAI_API_KEY": "must-not-leak"}):
            decision = mission.route_issue("ISS-01")

        self.assertEqual(decision.recommended_agent, "worker")
        self.assertFalse(marker.exists())

    def test_route_uses_only_an_available_ungated_local_router(self):
        router_script = self.root / "eligible_router.py"
        router_script.write_text(
            "import json, sys\n"
            "sys.stdin.read()\n"
            "print(json.dumps({\n"
            "  'complexity': 'low',\n"
            "  'recommended_agent': 'worker',\n"
            "  'requires_approval': False,\n"
            "  'reason': 'Use the eligible local worker.'\n"
            "}))\n",
            encoding="utf-8",
        )
        router_command = f"{sys.executable} {router_script}"
        config_path = self.write_agent_config(
            [
                {
                    "id": "cloud-router",
                    "role": "frontier",
                    "provider": "ollama",
                    "runner": "command",
                    "model": "remote:CLOUD",
                    "command": router_command,
                    "routing": "router",
                },
                {
                    "id": "gated-router",
                    "role": "frontier",
                    "provider": "local",
                    "runner": "command",
                    "command": router_command,
                    "routing": "router",
                    "requires_approval": True,
                },
                {
                    "id": "unavailable-router",
                    "role": "frontier",
                    "provider": "local",
                    "runner": "command",
                    "command": router_command,
                    "routing": "router",
                    "availability": "unavailable",
                },
                {
                    "id": "misrouted-frontier",
                    "role": "frontier",
                    "provider": "local",
                    "runner": "command",
                    "command": router_command,
                    "routing": "worker",
                },
                {
                    "id": "eligible-router",
                    "role": "frontier",
                    "provider": "local",
                    "runner": "command",
                    "command": router_command,
                    "routing": "router",
                },
                {
                    "id": "worker",
                    "role": "local-agent",
                    "provider": "local",
                    "runner": "fake",
                    "routing": "worker",
                },
            ]
        )
        mission = self.load_mission_with_agent_config(config_path)
        mission.record_command_approval(router_command, "auto-allowed")
        mission.approve_issue("ISS-01")

        decision = mission.route_issue("ISS-01")

        self.assertEqual(decision.router_agent, "eligible-router")
        self.assertEqual(decision.recommended_agent, "worker")

    def test_route_rejects_a_recommendation_outside_exact_eligible_candidates(self):
        router_script = self.root / "invalid_recommendation_router.py"
        router_script.write_text(
            "import json, sys\n"
            "prompt = sys.stdin.read()\n"
            "assert '\"eligible-worker\"' in prompt\n"
            "assert '\"unavailable-worker\"' not in prompt\n"
            "assert '\"nonassignable-worker\"' not in prompt\n"
            "assert '\"misrouted-delegate\"' not in prompt\n"
            "assert '\"controller\"' not in prompt\n"
            "print(json.dumps({\n"
            "  'complexity': 'low',\n"
            "  'recommended_agent': 'unavailable-worker',\n"
            "  'requires_approval': False,\n"
            "  'reason': 'This recommendation must be rejected.'\n"
            "}))\n",
            encoding="utf-8",
        )
        router_command = f"{sys.executable} {router_script}"
        config_path = self.write_agent_config(
            [
                {
                    "id": "router",
                    "role": "frontier",
                    "provider": "local",
                    "runner": "command",
                    "command": router_command,
                    "routing": "router",
                },
                {
                    "id": "controller",
                    "role": "frontier",
                    "provider": "local",
                    "runner": "fake",
                    "routing": "controller",
                },
                {
                    "id": "eligible-worker",
                    "role": "local-agent",
                    "provider": "local",
                    "runner": "fake",
                    "routing": "worker",
                },
                {
                    "id": "unavailable-worker",
                    "role": "local-agent",
                    "provider": "local",
                    "runner": "fake",
                    "routing": "worker",
                    "availability": "unavailable",
                },
                {
                    "id": "nonassignable-worker",
                    "role": "local-agent",
                    "provider": "local",
                    "runner": "fake",
                    "routing": "worker",
                    "assignable": False,
                },
                {
                    "id": "misrouted-delegate",
                    "role": "delegate-agent",
                    "provider": "local",
                    "runner": "fake",
                    "routing": "controller",
                    "delegate_only": True,
                },
            ]
        )
        mission = self.load_mission_with_agent_config(config_path)
        original_assignment = mission.issues["ISS-01"].assigned_agent
        mission.record_command_approval(router_command, "auto-allowed")
        mission.approve_issue("ISS-01")

        with self.assertRaisesRegex(AlbertError, "eligible delegation candidate"):
            mission.route_issue("ISS-01")

        self.assertEqual(mission.issues["ISS-01"].assigned_agent, original_assignment)
        self.assertNotIn("ISS-01", mission.delegations)
        reloaded = self.load_mission_with_agent_config(config_path)
        self.assertEqual(reloaded.issues["ISS-01"].assigned_agent, original_assignment)
        self.assertNotIn("ISS-01", reloaded.delegations)

    def test_cli_routes_and_approves_gated_delegation(self):
        router = self.root / "qwen_router_cli.py"
        router.write_text(
            "import json\n"
            "print(json.dumps({'complexity': 'architectural', 'recommended_agent': 'deepseek-v4-pro-cloud', 'requires_cloud_approval': True, 'reason': 'Architecture review needs DeepSeek.'}))\n",
            encoding="utf-8",
        )
        router_command = f"{sys.executable} {router}"
        config_path = self.write_agent_config(
            [
                {
                    "id": "qwen3.6-27b",
                    "role": "frontier",
                    "provider": "local",
                    "runner": "command",
                    "command": router_command,
                    "routing": "router",
                },
                {
                    "id": "deepseek-v4-pro-cloud",
                    "role": "delegate-agent",
                    "provider": "local",
                    "runner": "fake",
                    "model": "deepseek-v4-pro:cloud",
                    "routing": "delegate",
                    "delegate_only": True,
                    "requires_approval": True,
                },
            ]
        )
        base_args = [
            "--target-repo",
            str(self.target_repo),
            "--tracker-dir",
            str(self.tracker),
            "--runtime-root",
            str(self.runtime),
            "--mission-id",
            "mission-001",
            "--agent-config",
            str(config_path),
        ]
        mission = self.load_mission_with_agent_config(config_path)
        mission.record_command_approval(router_command, "auto-allowed")
        mission.approve_issue("ISS-01")

        exit_code, output = self.run_cli(["route", *base_args, "ISS-01"])
        self.assertEqual(exit_code, 0)
        self.assertIn("ISS-01 routed by qwen3.6-27b to deepseek-v4-pro-cloud", output)
        self.assertIn("delegation approval required", output)

        exit_code, output = self.run_cli(["approve-delegation", *base_args, "ISS-01"])
        self.assertEqual(exit_code, 0)
        self.assertIn("ISS-01 delegation approved for deepseek-v4-pro-cloud.", output)

    def test_tui_action_assigns_agent_and_persists_after_restart(self):
        config_path = self.write_agent_config(
            [
                {
                    "id": "deepseek-v4",
                    "role": "local-agent",
                    "provider": "ollama",
                    "runner": "ollama",
                    "model": "deepseek-v4",
                }
            ]
        )
        mission = self.load_mission_with_agent_config(config_path)

        result = perform_tui_action(mission, "assign", "ISS-01", agent_id="deepseek-v4", notes="Use reasoning sub-agent.")
        reloaded = self.load_mission_with_agent_config(config_path)

        self.assertEqual(result.message, "ISS-01 assigned to deepseek-v4.")
        self.assertEqual(reloaded.issues["ISS-01"].assigned_agent, "deepseek-v4")
        self.assertEqual(reloaded.issues["ISS-01"].notes, "Use reasoning sub-agent.")

    def test_tui_action_approves_and_launches_ready_issue(self):
        mission = self.load_mission()

        approve = perform_tui_action(mission, "approve", "ISS-01")
        launch = perform_tui_action(mission, "launch", "ISS-01", allowed_paths=["src"])
        reloaded = self.load_mission()

        self.assertEqual(approve.message, "ISS-01 approved and locked.")
        self.assertEqual(launch.session_id, "session-ISS-01-1")
        self.assertIn("session-ISS-01-1", reloaded.sessions)
        self.assertEqual(reloaded.sessions["session-ISS-01-1"].task_packet["allowed_paths"], ["src"])

    def test_tui_action_blocks_invalid_launch_with_cli_reason(self):
        mission = self.load_mission()

        with self.assertRaisesRegex(LaunchBlockedError, "ISS-02 must be approved before launch."):
            perform_tui_action(mission, "launch", "ISS-02")

    def test_cli_tui_action_assign_and_launch_are_persisted(self):
        base_args = [
            "--target-repo",
            str(self.target_repo),
            "--tracker-dir",
            str(self.tracker),
            "--runtime-root",
            str(self.runtime),
            "--mission-id",
            "mission-001",
        ]

        self.assertEqual(self.run_cli(["tui-action", *base_args, "assign", "ISS-01", "--agent", "qwen3.6-27b"])[0], 0)
        self.assertEqual(self.run_cli(["tui-action", *base_args, "approve", "ISS-01"])[0], 0)
        exit_code, output = self.run_cli(["tui-action", *base_args, "launch", "ISS-01", "--allowed-path", "src"])

        self.assertEqual(exit_code, 0)
        self.assertIn("session-ISS-01-1", output)
        self.assertEqual(self.load_mission().sessions["session-ISS-01-1"].assigned_agent, "qwen3.6-27b")

    def test_fake_runner_launch_writes_task_packet_artifacts_and_valid_evidence(self):
        config_path = self.write_agent_config(
            [
                {
                    "id": "fake-local",
                    "role": "local-agent",
                    "provider": "local",
                    "runner": "fake",
                }
            ]
        )
        mission = self.load_mission_with_agent_config(config_path)
        mission.assign_issue("ISS-01", "fake-local")
        mission.approve_issue("ISS-01")

        session = mission.launch_issue("ISS-01", allowed_paths=["src"])
        mission.run_session(session.session_id)

        self.assertEqual(session.status, "evidence-ready")
        self.assertEqual(session.evidence_valid, True)
        self.assertTrue(str(session.worktree_path).startswith(str(self.root / ".albert-worktrees")))
        self.assertTrue((session.worktree_path / "FAKE_AGENT_RESULT.md").exists())
        artifact_dir = self.runtime / mission.project_key / "sessions" / session.session_id
        self.assertTrue((artifact_dir / "task-packet.json").exists())
        self.assertTrue((artifact_dir / "fake-agent.log").exists())
        self.assertTrue((artifact_dir / "completion.json").exists())
        self.assertIn("fake-agent.log", "\n".join(session.evidence.artifact_links))
        self.assertNotIn("task-packet.json", "\n".join(session.evidence.artifact_links))
        self.assertIn("FAKE_AGENT_RESULT.md", session.evidence.changed_files)

    def test_issue_launch_persists_queued_before_runner_is_invoked(self):
        config_path = self.write_agent_config(
            [
                {
                    "id": "fake-local",
                    "role": "local-agent",
                    "provider": "local",
                    "runner": "fake",
                }
            ]
        )
        mission = self.load_mission_with_agent_config(config_path)
        mission.assign_issue("ISS-01", "fake-local")
        mission.approve_issue("ISS-01")

        with patch.object(
            mission,
            "_run_fake_agent",
            side_effect=AssertionError("launch must not invoke the runner"),
        ):
            session = mission.launch_issue("ISS-01", allowed_paths=["src"])

        persisted = self.load_mission_with_agent_config(config_path).sessions[session.session_id]
        self.assertEqual(session.status, "queued")
        self.assertEqual(persisted.status, "queued")
        self.assertEqual(persisted.runner_started_at, "")
        self.assertFalse(session.worktree_path.exists())
        self.assertFalse((session.worktree_path / "FAKE_AGENT_RESULT.md").exists())

    def test_loading_existing_runtime_is_read_only_during_deferred_execution(self):
        mission = self.load_mission()
        mission.approve_issue("ISS-01")
        queued = mission.launch_issue("ISS-01")
        reader = AlbertMission(
            target_repo=self.target_repo,
            tracker_dir=self.tracker,
            runtime_root=self.runtime,
            mission_id="mission-001",
        )

        with patch.object(
            reader,
            "_persist",
            side_effect=AssertionError("readers must not overwrite runner lifecycle state"),
        ):
            reader.load()

        self.assertEqual(reader.sessions[queued.session_id].status, "queued")

    def test_legacy_unstarted_launched_session_migrates_to_executable_queue(self):
        config_path = self.write_agent_config(
            [
                {
                    "id": "fake-local",
                    "role": "local-agent",
                    "provider": "local",
                    "runner": "fake",
                }
            ]
        )
        mission = self.load_mission_with_agent_config(config_path)
        mission.assign_issue("ISS-01", "fake-local")
        mission.approve_issue("ISS-01")
        queued = mission.launch_issue("ISS-01")
        persisted = json.loads(mission.runtime_path.read_text(encoding="utf-8"))
        persisted["sessions"][queued.session_id]["status"] = "launched"
        mission.runtime_path.write_text(json.dumps(persisted), encoding="utf-8")

        restored = self.load_mission_with_agent_config(config_path)

        self.assertEqual(restored.sessions[queued.session_id].status, "queued")
        self.assertEqual(restored.run_session(queued.session_id).status, "evidence-ready")

    def test_run_session_persists_running_before_completion_and_final_state_after_restart(self):
        config_path = self.write_agent_config(
            [
                {
                    "id": "fake-local",
                    "role": "local-agent",
                    "provider": "local",
                    "runner": "fake",
                }
            ]
        )
        mission = self.load_mission_with_agent_config(config_path)
        mission.assign_issue("ISS-01", "fake-local")
        mission.approve_issue("ISS-01")
        queued = mission.launch_issue("ISS-01")
        real_runner = mission._run_fake_agent
        real_prepare = mission._ensure_session_worktree
        observed_statuses = []
        observed_before_prepare = []

        def inspect_running_before_prepare(session):
            reloaded = self.load_mission_with_agent_config(config_path)
            observed_before_prepare.append(
                (reloaded.sessions[session.session_id].status, session.worktree_path.exists())
            )
            real_prepare(session)

        def inspect_running_then_complete(session):
            reloaded = self.load_mission_with_agent_config(config_path)
            observed_statuses.append(reloaded.sessions[session.session_id].status)
            self.assertTrue(session.worktree_path.exists())
            real_runner(session)

        with (
            patch.object(
                mission,
                "_ensure_session_worktree",
                side_effect=inspect_running_before_prepare,
            ),
            patch.object(
                mission,
                "_run_fake_agent",
                side_effect=inspect_running_then_complete,
            ),
        ):
            completed = mission.run_session(queued.session_id)

        persisted = self.load_mission_with_agent_config(config_path).sessions[queued.session_id]
        self.assertEqual(observed_before_prepare, [("running", False)])
        self.assertEqual(observed_statuses, ["running"])
        self.assertEqual(completed.status, "evidence-ready")
        self.assertEqual(persisted.status, "evidence-ready")
        self.assertTrue(persisted.runner_started_at)
        self.assertTrue(persisted.runner_ended_at)
        self.assertTrue(persisted.evidence_valid)

    def test_run_session_persists_failed_state_when_runner_raises(self):
        config_path = self.write_agent_config(
            [
                {
                    "id": "fake-local",
                    "role": "local-agent",
                    "provider": "local",
                    "runner": "fake",
                }
            ]
        )
        mission = self.load_mission_with_agent_config(config_path)
        mission.assign_issue("ISS-01", "fake-local")
        mission.approve_issue("ISS-01")
        queued = mission.launch_issue("ISS-01")

        with patch.object(mission, "_run_fake_agent", side_effect=RuntimeError("runner exploded")):
            with self.assertRaisesRegex(AlbertError, "runner exploded"):
                mission.run_session(queued.session_id)

        persisted = self.load_mission_with_agent_config(config_path).sessions[queued.session_id]
        self.assertEqual(persisted.status, "failed")
        self.assertEqual(persisted.runner_exit_status, 1)
        self.assertTrue(persisted.runner_started_at)
        self.assertTrue(persisted.runner_ended_at)
        self.assertIn("runner exploded", persisted.task_packet["runner_failure"])

    def test_concurrent_session_runners_retain_both_final_states(self):
        self.initialize_target_git_repo({"src/app.py": "VALUE = 1\n"})
        config_path = self.write_agent_config(
            [
                {
                    "id": "fake-local",
                    "role": "local-agent",
                    "provider": "local",
                    "runner": "fake",
                }
            ]
        )
        mission = self.load_mission_with_agent_config(config_path)
        mission.assign_issue("ISS-01", "fake-local")
        mission.approve_issue("ISS-01")
        first = mission.launch_issue("ISS-01")
        second = mission.launch_issue("ISS-01")
        first_runner = self.load_mission_with_agent_config(config_path)
        second_runner = self.load_mission_with_agent_config(config_path)
        barrier = threading.Barrier(2)
        failures = []

        def execute(runner, session_id):
            real_runner = runner._run_fake_agent

            def synchronize_then_run(session):
                barrier.wait(timeout=5)
                real_runner(session)

            try:
                with patch.object(
                    runner,
                    "_run_fake_agent",
                    side_effect=synchronize_then_run,
                ):
                    runner.run_session(session_id)
            except Exception as exc:  # surfaced by the assertion below
                failures.append(exc)

        threads = [
            threading.Thread(
                target=execute,
                args=(first_runner, first.session_id),
            ),
            threading.Thread(
                target=execute,
                args=(second_runner, second.session_id),
            ),
        ]
        for thread in threads:
            thread.start()
        for thread in threads:
            thread.join(timeout=10)

        self.assertEqual([thread.is_alive() for thread in threads], [False, False])
        self.assertEqual(failures, [])
        persisted = self.load_mission_with_agent_config(config_path)
        self.assertEqual(persisted.sessions[first.session_id].status, "evidence-ready")
        self.assertEqual(persisted.sessions[second.session_id].status, "evidence-ready")
        self.assertTrue(persisted.sessions[first.session_id].evidence_valid)
        self.assertTrue(persisted.sessions[second.session_id].evidence_valid)

    def test_stale_full_runtime_write_cannot_regress_final_or_cancelled_sessions(self):
        config_path = self.write_agent_config(
            [
                {
                    "id": "fake-local",
                    "role": "local-agent",
                    "provider": "local",
                    "runner": "fake",
                }
            ]
        )
        mission = self.load_mission_with_agent_config(config_path)
        mission.assign_issue("ISS-01", "fake-local")
        mission.approve_issue("ISS-01")
        completed_session = mission.launch_issue("ISS-01")
        cancelled_session = mission.launch_issue("ISS-01")
        stale_writer = self.load_mission_with_agent_config(config_path)

        runner = self.load_mission_with_agent_config(config_path)
        runner.run_session(completed_session.session_id)
        self.load_mission_with_agent_config(config_path).cancel_session(
            cancelled_session.session_id,
            reason="Keep cancellation terminal during a stale write.",
        )
        stale_writer.record_command_approval("npm test", "auto-allowed")

        persisted = self.load_mission_with_agent_config(config_path)
        self.assertEqual(
            persisted.sessions[completed_session.session_id].status,
            "evidence-ready",
        )
        self.assertEqual(
            persisted.sessions[cancelled_session.session_id].status,
            "cancelled",
        )
        self.assertTrue(
            persisted.sessions[cancelled_session.session_id].cancel_requested_at
        )

    def test_running_session_cancel_is_terminal_before_runner_release(self):
        config_path = self.write_agent_config(
            [
                {
                    "id": "fake-local",
                    "role": "local-agent",
                    "provider": "local",
                    "runner": "fake",
                }
            ]
        )
        mission = self.load_mission_with_agent_config(config_path)
        mission.assign_issue("ISS-01", "fake-local")
        mission.approve_issue("ISS-01")
        queued = mission.launch_issue("ISS-01")
        runner = self.load_mission_with_agent_config(config_path)
        real_runner = runner._run_fake_agent
        runner_entered = threading.Event()
        release_runner = threading.Event()
        outcomes = []

        def pause_then_run(session):
            runner_entered.set()
            if not release_runner.wait(timeout=5):
                raise RuntimeError("test runner release timed out")
            real_runner(session)

        def execute():
            with patch.object(runner, "_run_fake_agent", side_effect=pause_then_run):
                outcomes.append(runner.run_session(queued.session_id).status)

        thread = threading.Thread(target=execute)
        thread.start()
        self.assertTrue(runner_entered.wait(timeout=5))
        running = self.load_mission_with_agent_config(config_path).sessions[
            queued.session_id
        ]
        self.assertEqual(running.status, "running")
        self.assertIsNotNone(running.runner_pid)

        canceller = self.load_mission_with_agent_config(config_path)
        cancelled = canceller.cancel_session(
            queued.session_id,
            reason="Operator stopped the isolated run.",
        )
        self.assertEqual(cancelled.status, "cancelled")
        self.assertTrue(cancelled.cancel_requested_at)
        self.assertEqual(cancelled.cancel_reason, "Operator stopped the isolated run.")
        self.assertIsNone(cancelled.runner_pid)
        self.assertIsNone(cancelled.runner_process_pid)
        release_runner.set()
        thread.join(timeout=5)

        self.assertFalse(thread.is_alive())
        self.assertEqual(outcomes, ["cancelled"])
        persisted = self.load_mission_with_agent_config(config_path).sessions[
            queued.session_id
        ]
        self.assertEqual(persisted.status, "cancelled")
        self.assertFalse((queued.worktree_path / "FAKE_AGENT_RESULT.md").exists())
        self.assertIsNone(persisted.evidence)
        self.assertIsNone(persisted.runner_pid)
        self.assertIsNone(persisted.runner_process_pid)

    def test_restart_fails_closed_for_legacy_runner_without_exact_recovery_identity(self):
        config_path = self.write_agent_config(
            [
                {
                    "id": "fake-local",
                    "role": "local-agent",
                    "provider": "local",
                    "runner": "fake",
                }
            ]
        )
        mission = self.load_mission_with_agent_config(config_path)
        mission.assign_issue("ISS-01", "fake-local")
        mission.approve_issue("ISS-01")
        with patch.object(mission, "_validate_target_repository_boundary"):
            legacy = mission.launch_issue("ISS-01")
        legacy.status = "running"
        legacy.runner_started_at = "2026-07-10T18:00:00Z"
        legacy.runner_pid = 999_999_999
        legacy.runner_identity = "linux:999999999:missing"
        mission._persist_session_update(
            legacy,
            expected_statuses={"queued"},
        )

        reloaded = self.load_mission_with_agent_config(config_path)

        unchanged = reloaded.sessions[legacy.session_id]
        self.assertEqual(unchanged.status, "running")
        self.assertEqual(unchanged.runner_pid, 999_999_999)
        attentions = reloaded.supervision_state()["attentions"]
        self.assertEqual(len(attentions), 1)
        attention = next(iter(attentions.values()))
        self.assertEqual(attention["next_effect"], "mission-commander-decision")
        self.assertEqual(attention["disposition"], "open")

    def test_cancel_terminates_active_command_process_before_post_cancel_write(self):
        script = self.root / "cancellable_runner.py"
        script.write_text(
            "import pathlib, time\n"
            "pathlib.Path('started-before-cancel.txt').write_text('started')\n"
            "time.sleep(30)\n"
            "pathlib.Path('MUST_NOT_EXIST_AFTER_CANCEL.txt').write_text('late')\n",
            encoding="utf-8",
        )
        command = f"{sys.executable} {script}"
        config_path = self.write_agent_config(
            [
                {
                    "id": "command-local",
                    "role": "local-agent",
                    "provider": "local",
                    "runner": "command",
                    "command": command,
                }
            ]
        )
        mission = self.load_mission_with_agent_config(config_path)
        mission.record_command_approval(command, "auto-allowed")
        mission.assign_issue("ISS-01", "command-local")
        mission.approve_issue("ISS-01")
        queued = mission.launch_issue("ISS-01")
        runner = self.load_mission_with_agent_config(config_path)
        outcomes = []

        thread = threading.Thread(
            target=lambda: outcomes.append(
                runner.run_session(queued.session_id).status
            )
        )
        thread.start()
        deadline = time.monotonic() + 5
        active = None
        observed_runner = None
        while time.monotonic() < deadline:
            observed_runner = self.load_mission_with_agent_config(config_path)
            active = observed_runner.sessions[queued.session_id]
            if (
                active.runner_process_pid is not None
                and (queued.worktree_path / "started-before-cancel.txt").exists()
            ):
                break
            time.sleep(0.02)
        self.assertIsNotNone(active)
        self.assertEqual(
            active.status,
            "running",
            {
                "runner_failure": active.task_packet.get("runner_failure"),
                "runner_exit_status": active.runner_exit_status,
                "stderr": (
                    Path(active.artifacts["stderr"]).read_text(encoding="utf-8")
                    if active.artifacts.get("stderr")
                    and Path(active.artifacts["stderr"]).exists()
                    else ""
                ),
                "timeline": observed_runner.timeline[-5:] if observed_runner else [],
            },
        )
        self.assertIsNotNone(active.runner_process_pid)

        self.load_mission_with_agent_config(config_path).cancel_session(
            queued.session_id,
            reason="Stop the active child process.",
        )
        thread.join(timeout=5)

        self.assertFalse(thread.is_alive())
        self.assertEqual(outcomes, ["cancelled"])
        self.assertFalse(
            (queued.worktree_path / "MUST_NOT_EXIST_AFTER_CANCEL.txt").exists()
        )
        persisted = self.load_mission_with_agent_config(config_path).sessions[
            queued.session_id
        ]
        self.assertEqual(persisted.status, "cancelled")
        self.assertIsNone(persisted.runner_pid)
        self.assertIsNone(persisted.runner_process_pid)

    def test_repair_launch_is_queued_and_runs_only_through_public_session_runner(self):
        config_path = self.write_agent_config(
            [
                {
                    "id": "fake-local",
                    "role": "local-agent",
                    "provider": "local",
                    "runner": "fake",
                }
            ]
        )
        mission = self.load_mission_with_agent_config(config_path)
        mission.assign_issue("ISS-01", "fake-local")
        mission.approve_issue("ISS-01")
        first = mission.launch_issue("ISS-01")
        mission.run_session(first.session_id)
        mission.record_frontier_review(
            first.session_id,
            "Needs repair",
            reason="The deterministic result needs a repair pass.",
        )

        with patch.object(
            mission,
            "_run_fake_agent",
            side_effect=AssertionError("repair acknowledgement must not invoke the runner"),
        ):
            repair = mission.launch_repair(first.session_id)

        self.assertEqual(repair.status, "queued")
        self.assertEqual(
            self.load_mission_with_agent_config(config_path).sessions[repair.session_id].status,
            "queued",
        )
        completed = mission.run_session(repair.session_id)
        self.assertEqual(completed.status, "evidence-ready")

    def test_repair_cannot_switch_to_controller_or_unrouted_delegate(self):
        config_path = self.write_agent_config(
            [
                {
                    "id": "fake-local",
                    "role": "local-agent",
                    "provider": "local",
                    "runner": "fake",
                    "routing": "worker",
                },
                {
                    "id": "controller",
                    "role": "frontier",
                    "provider": "local",
                    "runner": "fake",
                    "routing": "controller",
                },
                {
                    "id": "delegate",
                    "role": "delegate-agent",
                    "provider": "local",
                    "runner": "fake",
                    "routing": "delegate",
                    "delegate_only": True,
                },
                {
                    "id": "uppercase-cloud-worker",
                    "role": "local-agent",
                    "provider": "ollama",
                    "runner": "fake",
                    "model": "remote:CLOUD",
                    "routing": "worker",
                },
            ]
        )
        mission = self.load_mission_with_agent_config(config_path)
        mission.assign_issue("ISS-01", "fake-local")
        mission.approve_issue("ISS-01")
        first = mission.launch_issue("ISS-01")
        mission.run_session(first.session_id)
        mission.record_frontier_review(
            first.session_id,
            "Needs repair",
            reason="Use a governed worker for the repair.",
        )

        for agent_id in ("controller", "delegate", "uppercase-cloud-worker"):
            with self.subTest(agent_id=agent_id):
                with self.assertRaisesRegex(LaunchBlockedError, "not launchable"):
                    mission.launch_repair(first.session_id, agent_id=agent_id)

        self.assertEqual(list(mission.sessions), [first.session_id])

    def test_ollama_repair_inherits_prior_session_changes_inside_the_same_boundary(self):
        self.initialize_target_git_repo(
            {"src/app.py": "PRIOR_AGENT_BROKEN = False\nSTATE = 'base'\n"}
        )
        fake_ollama = self.root / "fake_ollama_repair.py"
        fake_ollama.write_text(
            "import json, sys\n"
            "prompt = sys.stdin.read()\n"
            "if 'PRIOR_AGENT_BROKEN = True' in prompt:\n"
            "    content = \"PRIOR_AGENT_BROKEN = False\\nSTATE = 'repaired'\\n\"\n"
            "else:\n"
            "    content = \"PRIOR_AGENT_BROKEN = True\\nSTATE = 'first-pass'\\n\"\n"
            "print(json.dumps({'summary': 'one-shot repair', "
            "'files': [{'path': 'src/app.py', 'content': content}]}))\n",
            encoding="utf-8",
        )
        command = f"{sys.executable} {fake_ollama}"
        config_path = self.write_agent_config(
            [
                {
                    "id": "repair-local",
                    "role": "local-agent",
                    "provider": "ollama",
                    "runner": "ollama",
                    "model": "repair:test",
                    "command": command,
                }
            ]
        )
        mission = self.load_mission_with_agent_config(config_path)
        mission.record_command_approval(command, "auto-allowed")
        mission.assign_issue("ISS-01", "repair-local")
        mission.approve_issue("ISS-01")
        first = mission.launch_issue("ISS-01", allowed_paths=["src/app.py"])
        mission.run_session(first.session_id)
        mission.record_frontier_review(
            first.session_id,
            "Needs repair",
            reason="The first pass left PRIOR_AGENT_BROKEN enabled.",
        )

        repair = mission.launch_repair(first.session_id)
        completed = mission.run_session(repair.session_id)

        self.assertNotEqual(first.worktree_path, repair.worktree_path)
        self.assertEqual(repair.task_packet["allowed_paths"], ["src/app.py"])
        self.assertFalse(first.worktree_path.exists())
        self.assertEqual(
            mission.sessions[first.session_id].retirement["phase"],
            "retired",
        )
        self.assertEqual(
            (repair.worktree_path / "src" / "app.py").read_text(encoding="utf-8"),
            "PRIOR_AGENT_BROKEN = False\nSTATE = 'repaired'\n",
        )
        self.assertEqual(completed.status, "evidence-ready")
        self.assertEqual(completed.evidence.changed_files, ["src/app.py"])

    def test_repair_preparation_resumes_without_losing_the_inherited_diff(self):
        self.initialize_target_git_repo({"src/app.py": "STATE = 'base'\n"})
        runner_script = self.root / "preserve_repair_state.py"
        runner_script.write_text(
            "from pathlib import Path\n"
            "Path('src/app.py').write_text(\"STATE = 'first-pass'\\n\")\n",
            encoding="utf-8",
        )
        command = f"{sys.executable} {runner_script}"
        config_path = self.write_agent_config(
            [
                {
                    "id": "repair-local",
                    "role": "local-agent",
                    "provider": "local",
                    "runner": "command",
                    "command": command,
                    "routing": "worker",
                }
            ]
        )
        mission = self.load_mission_with_agent_config(config_path)
        mission.record_command_approval(command, "auto-allowed")
        mission.assign_issue("ISS-01", "repair-local")
        mission.approve_issue("ISS-01")
        first = mission.launch_issue("ISS-01")
        mission.run_session(first.session_id)
        mission.record_frontier_review(
            first.session_id,
            "Needs repair",
            reason="Preserve the first pass across repair recovery.",
        )
        repair = mission.launch_repair(first.session_id)
        overlay_prior_session_state = mission._overlay_prior_session_state

        def crash_after_prior_overlay(candidate):
            overlay_prior_session_state(candidate)
            raise KeyboardInterrupt("simulated process loss after repair overlay")

        with (
            patch.object(
                mission,
                "_overlay_prior_session_state",
                side_effect=crash_after_prior_overlay,
            ),
            patch("albert_mvp.core.os.getpid", return_value=999_999_999),
            self.assertRaisesRegex(KeyboardInterrupt, "simulated process loss"),
        ):
            mission.run_session(repair.session_id)

        recovered_mission = self.load_mission_with_agent_config(config_path)
        recovered = recovered_mission.sessions[repair.session_id]
        self.assertEqual(recovered.status, "queued")
        self.assertEqual(
            recovered.repository_snapshot["preparation"]["state"],
            "baseline-captured",
        )
        completed = recovered_mission.run_session(repair.session_id)

        self.assertEqual(
            (completed.worktree_path / "src" / "app.py").read_text(
                encoding="utf-8"
            ),
            "STATE = 'first-pass'\n",
        )
        self.assertIn("src/app.py", completed.evidence.changed_files)
        self.assertEqual(
            completed.repository_snapshot["repair_overlay"]["applied_files"].count(
                "src/app.py"
            ),
            1,
        )

    def test_repair_parent_baseline_survives_process_loss_before_final_capture(self):
        self.initialize_target_git_repo({"src/app.py": "STATE = 'base'\n"})
        config_path = self.write_agent_config(
            [
                {
                    "id": "repair-baseline-local",
                    "role": "local-agent",
                    "provider": "local",
                    "runner": "fake",
                    "routing": "worker",
                }
            ]
        )
        mission = self.load_mission_with_agent_config(config_path)
        mission.assign_issue("ISS-01", "repair-baseline-local")
        mission.approve_issue("ISS-01")
        first = mission.launch_issue("ISS-01")
        mission.run_session(first.session_id)
        mission.record_frontier_review(
            first.session_id,
            "Needs repair",
            reason="Recover from process loss without changing the parent baseline.",
        )
        (self.target_repo / "src/app.py").write_text(
            "STATE = 'captured-parent'\n",
            encoding="utf-8",
        )
        repair = mission.launch_repair(first.session_id)

        with (
            patch.object(
                mission,
                "_capture_and_persist_worktree_baseline",
                side_effect=KeyboardInterrupt(
                    "simulated process loss before final baseline capture"
                ),
            ),
            patch("albert_mvp.core.os.getpid", return_value=999_999_999),
            self.assertRaisesRegex(
                KeyboardInterrupt,
                "process loss before final baseline capture",
            ),
        ):
            mission.run_session(repair.session_id)

        (self.target_repo / "src/app.py").write_text(
            "STATE = 'newer-parent-edit'\n",
            encoding="utf-8",
        )
        recovered_mission = self.load_mission_with_agent_config(config_path)
        recovered = recovered_mission.sessions[repair.session_id]
        self.assertEqual(recovered.status, "queued")
        self.assertTrue(recovered.repository_snapshot)

        completed = recovered_mission.run_session(repair.session_id)

        self.assertEqual(completed.status, "evidence-ready")
        self.assertEqual(
            (completed.worktree_path / "src/app.py").read_text(encoding="utf-8"),
            "STATE = 'captured-parent'\n",
        )
        self.assertNotIn("src/app.py", completed.evidence.changed_files)

    def test_target_untracked_baseline_recovers_from_staged_pre_effect_snapshot(self):
        self.initialize_target_git_repo({"src/app.py": "STATE = 'base'\n"})
        untracked = self.target_repo / "src/helper.py"
        untracked.write_text("STATE = 'captured-parent'\n", encoding="utf-8")
        config_path = self.write_agent_config(
            [
                {
                    "id": "untracked-baseline-local",
                    "role": "local-agent",
                    "provider": "local",
                    "runner": "fake",
                    "routing": "worker",
                }
            ]
        )
        mission = self.load_mission_with_agent_config(config_path)
        mission.assign_issue("ISS-01", "untracked-baseline-local")
        mission.approve_issue("ISS-01")
        session = mission.launch_issue("ISS-01")
        persist_preparation = mission._persist_worktree_preparation

        def crash_before_applied_marker(candidate):
            marker = candidate.repository_snapshot.get("preparation", {})
            if marker.get("state") == "target-overlay-applied":
                raise KeyboardInterrupt(
                    "simulated process loss after untracked baseline copy"
                )
            return persist_preparation(candidate)

        with (
            patch.object(
                mission,
                "_persist_worktree_preparation",
                side_effect=crash_before_applied_marker,
            ),
            patch("albert_mvp.core.os.getpid", return_value=999_999_999),
            self.assertRaisesRegex(
                KeyboardInterrupt,
                "process loss after untracked baseline copy",
            ),
        ):
            mission.run_session(session.session_id)

        untracked.write_text("STATE = 'newer-parent'\n", encoding="utf-8")
        recovered_mission = self.load_mission_with_agent_config(config_path)
        recovered = recovered_mission.sessions[session.session_id]
        self.assertEqual(recovered.status, "queued")
        self.assertEqual(
            recovered.repository_snapshot["preparation"]["state"],
            "target-overlay-pending",
        )

        completed = recovered_mission.run_session(session.session_id)

        self.assertEqual(
            (completed.worktree_path / "src/helper.py").read_text(encoding="utf-8"),
            "STATE = 'captured-parent'\n",
        )
        self.assertNotIn("src/helper.py", completed.evidence.changed_files)

    def test_repair_overlay_recovers_from_staged_pre_effect_snapshot(self):
        self.initialize_target_git_repo({"src/app.py": "STATE = 'base'\n"})
        config_path = self.write_agent_config(
            [
                {
                    "id": "repair-overlay-local",
                    "role": "local-agent",
                    "provider": "local",
                    "runner": "fake",
                    "routing": "worker",
                }
            ]
        )
        mission = self.load_mission_with_agent_config(config_path)
        mission.assign_issue("ISS-01", "repair-overlay-local")
        mission.approve_issue("ISS-01")
        first = mission.launch_issue("ISS-01")
        mission.run_session(first.session_id)
        prior_change = first.worktree_path / "src/repair.py"
        prior_change.write_text("STATE = 'captured-prior'\n", encoding="utf-8")
        mission.record_frontier_review(
            first.session_id,
            "Needs repair",
            reason="Carry the exact first-pass state into repair.",
        )
        repair = mission.launch_repair(first.session_id)
        persist_session_update = mission._persist_session_update

        def crash_before_applied_repair_marker(candidate, **kwargs):
            overlay = candidate.repository_snapshot.get("repair_overlay", {})
            if (
                isinstance(overlay, dict)
                and overlay.get("applied_files")
                and overlay.get("state") in {None, "applied"}
            ):
                raise KeyboardInterrupt(
                    "simulated process loss after repair overlay effect"
                )
            return persist_session_update(candidate, **kwargs)

        with (
            patch.object(
                mission,
                "_persist_session_update",
                side_effect=crash_before_applied_repair_marker,
            ),
            patch("albert_mvp.core.os.getpid", return_value=999_999_999),
            self.assertRaisesRegex(
                KeyboardInterrupt,
                "process loss after repair overlay effect",
            ),
        ):
            mission.run_session(repair.session_id)

        self.assertFalse(prior_change.exists())
        recovered_mission = self.load_mission_with_agent_config(config_path)
        recovered = recovered_mission.sessions[repair.session_id]
        self.assertEqual(recovered.status, "queued")

        completed = recovered_mission.run_session(repair.session_id)

        self.assertEqual(
            (completed.worktree_path / "src/repair.py").read_text(encoding="utf-8"),
            "STATE = 'captured-prior'\n",
        )

    def test_deferred_delegated_session_rejects_agent_reconfigured_as_controller(self):
        delegate = {
            "id": "mutable-delegate",
            "role": "delegate-agent",
            "provider": "local",
            "runner": "fake",
            "routing": "delegate",
            "assignable": False,
            "delegate_only": True,
        }
        config_path = self.write_agent_config([delegate])
        mission = self.load_mission_with_agent_config(config_path)
        mission.approve_issue("ISS-01")
        mission.issues["ISS-01"].assigned_agent = "mutable-delegate"
        mission.delegations["ISS-01"] = DelegationDecision(
            issue_id="ISS-01",
            router_agent="local-router",
            recommended_agent="mutable-delegate",
            complexity="high",
            reason="The agent was a valid delegated worker when routed.",
            requires_approval=False,
            approved=True,
        )
        mission._persist()
        session = mission.launch_issue("ISS-01")

        self.write_agent_config(
            [
                {
                    **delegate,
                    "role": "frontier",
                    "routing": "controller",
                }
            ]
        )
        reloaded = self.load_mission_with_agent_config(config_path)

        with self.assertRaisesRegex(
            LaunchBlockedError,
            "not authorized for deferred Local Agent execution",
        ):
            reloaded.run_session(session.session_id)

        persisted = self.load_mission_with_agent_config(config_path).sessions[
            session.session_id
        ]
        self.assertEqual(persisted.status, "failed")
        self.assertFalse((persisted.worktree_path / "FAKE_AGENT_RESULT.md").exists())

    def test_repair_rejects_delegated_agent_reconfigured_as_controller(self):
        delegate = {
            "id": "mutable-repair-delegate",
            "role": "delegate-agent",
            "provider": "local",
            "runner": "fake",
            "routing": "delegate",
            "assignable": False,
            "delegate_only": True,
        }
        config_path = self.write_agent_config([delegate])
        mission = self.load_mission_with_agent_config(config_path)
        mission.approve_issue("ISS-01")
        mission.issues["ISS-01"].assigned_agent = "mutable-repair-delegate"
        mission.delegations["ISS-01"] = DelegationDecision(
            issue_id="ISS-01",
            router_agent="local-router",
            recommended_agent="mutable-repair-delegate",
            complexity="high",
            reason="The agent was a valid delegated worker when routed.",
            requires_approval=False,
            approved=True,
        )
        mission._persist()
        prior = mission.launch_issue("ISS-01")
        mission.run_session(prior.session_id)
        mission.record_frontier_review(
            prior.session_id,
            "Needs repair",
            reason="The delegated result needs one repair pass.",
        )

        self.write_agent_config(
            [
                {
                    **delegate,
                    "role": "frontier",
                    "routing": "controller",
                }
            ]
        )
        reloaded = self.load_mission_with_agent_config(config_path)

        with self.assertRaisesRegex(
            LaunchBlockedError,
            "repair agent .* is not launchable",
        ):
            reloaded.launch_repair(prior.session_id)

        self.assertEqual(
            list(self.load_mission_with_agent_config(config_path).sessions),
            [prior.session_id],
        )

    def test_ad_hoc_delegation_can_launch_an_isolated_repair_with_prior_changes(self):
        self.initialize_target_git_repo({"src/ad_hoc.py": "STATE = 'base'\n"})
        config_path = self.write_agent_config(
            [
                {
                    "id": "fake-local",
                    "role": "local-agent",
                    "provider": "local",
                    "runner": "fake",
                    "routing": "worker",
                }
            ]
        )
        mission = self.load_mission_with_agent_config(config_path)
        prior_id = "session-ADHOC-000001-1"
        prior = LocalAgentSession(
            session_id=prior_id,
            issue_id="ADHOC-000001",
            assigned_agent="fake-local",
            worktree_path=mission._session_worktree_path(prior_id),
            task_packet={
                "issue_id": "ADHOC-000001",
                "work_kind": "ad-hoc-delegation",
                "goal": "Repair the ad hoc implementation.",
                "acceptance_criteria": ["Ad hoc state is corrected."],
                "allowed_paths": ["src"],
                "command_policy": {},
                "evidence_requirements": mission.default_evidence_requirements(),
                "assigned_agent": "fake-local",
                "agent_config": mission._agent_config_for("fake-local"),
                "originating_message_id": "console-000001",
            },
            status="running",
        )
        mission.sessions[prior_id] = prior
        mission._persist()
        mission._ensure_session_worktree(prior)
        prior.worktree_identity = mission._worktree_identity_for_session(prior)
        mission.retirement_quiescence_probe = lambda _boundary: (
            "absent",
            "absent",
        )
        (prior.worktree_path / "src" / "ad_hoc.py").write_text(
            "STATE = 'first-pass'\n", encoding="utf-8"
        )
        prior.evidence = EvidencePackage(
            changed_files=["src/ad_hoc.py"],
            diff_summary="First ad hoc pass.",
            commands_run=["fake-agent fake-local"],
            test_results="Needs repair.",
            known_risks="State is not final.",
            proposed_context_updates="None.",
            artifact_links=[],
        )
        prior.evidence_valid = True
        prior.status = "evidence-ready"
        mission._persist()
        mission.record_frontier_review(
            prior_id,
            "Needs repair",
            reason="Repair the ad hoc state without losing the first pass.",
        )

        repair = mission.launch_repair(prior_id)
        completed = mission.run_session(repair.session_id)

        self.assertEqual(repair.task_packet["work_kind"], "ad-hoc-delegation")
        self.assertEqual(repair.task_packet["originating_message_id"], "console-000001")
        self.assertEqual(
            (repair.worktree_path / "src" / "ad_hoc.py").read_text(encoding="utf-8"),
            "STATE = 'first-pass'\n",
        )
        self.assertEqual(completed.status, "evidence-ready")

    def test_cli_runs_one_queued_workstation_session_and_returns_json_lifecycle(self):
        config_path = self.write_agent_config(
            [
                {
                    "id": "fake-local",
                    "role": "local-agent",
                    "provider": "local",
                    "runner": "fake",
                }
            ]
        )
        mission = self.load_mission_with_agent_config(config_path)
        mission.assign_issue("ISS-01", "fake-local")
        mission.approve_issue("ISS-01")
        queued = mission.launch_issue("ISS-01")
        common = [
            "--target-repo",
            str(self.target_repo),
            "--tracker-dir",
            str(self.tracker),
            "--runtime-root",
            str(self.runtime),
            "--mission-id",
            "mission-001",
            "--agent-config",
            str(config_path),
        ]

        exit_code, output = self.run_cli(
            [
                "workstation-session-run",
                *common,
                "--session-id",
                queued.session_id,
                "--session-mission-id",
                "mission-001",
            ]
        )

        payload = json.loads(output)
        self.assertEqual(exit_code, 0)
        self.assertEqual(payload["schema_version"], 1)
        self.assertEqual(payload["mission_id"], "mission-001")
        self.assertEqual(payload["session_id"], queued.session_id)
        self.assertEqual(payload["status"], "evidence-ready")
        self.assertEqual(payload["evidence_valid"], True)
        self.assertTrue(payload["runner_started_at"])
        self.assertTrue(payload["runner_ended_at"])

    def test_fake_runner_session_persists_as_evidence_ready_after_restart(self):
        config_path = self.write_agent_config(
            [
                {
                    "id": "fake-local",
                    "role": "local-agent",
                    "provider": "local",
                    "runner": "fake",
                }
            ]
        )
        mission = self.load_mission_with_agent_config(config_path)
        mission.assign_issue("ISS-01", "fake-local")
        mission.approve_issue("ISS-01")
        session = mission.launch_issue("ISS-01")
        mission.run_session(session.session_id)

        reloaded = self.load_mission_with_agent_config(config_path)
        persisted = reloaded.sessions[session.session_id]

        self.assertEqual(persisted.status, "evidence-ready")
        self.assertEqual(persisted.evidence_valid, True)
        self.assertEqual(persisted.evidence.commands_run, ["fake-agent fake-local"])
        self.assertIn("Deterministic fake completion", persisted.evidence.diff_summary)

    def test_incomplete_automated_evidence_is_not_journaled_as_validated(self):
        config_path = self.write_agent_config(
            [
                {
                    "id": "fake-local",
                    "role": "local-agent",
                    "provider": "local",
                    "runner": "fake",
                }
            ]
        )
        mission = self.load_mission_with_agent_config(config_path)
        mission.assign_issue("ISS-01", "fake-local")
        mission.approve_issue("ISS-01")
        session = mission.launch_issue("ISS-01")
        recorded: list[str] = []
        mission._evidence_activity_recorder = (
            lambda _mission_id, candidate, _evidence: recorded.append(
                candidate.session_id
            )
        )

        def incomplete(candidate: LocalAgentSession) -> None:
            candidate.evidence = EvidencePackage(
                changed_files=[],
                diff_summary="No validated change.",
                commands_run=["fake-agent"],
                test_results="Tests did not run.",
                known_risks="Runner failed before validation.",
                proposed_context_updates="None.",
            )
            candidate.evidence_valid = False
            candidate.runner_exit_status = 1
            candidate.status = "failed"

        with patch.object(mission, "_run_fake_agent", side_effect=incomplete):
            mission.run_session(session.session_id)

        self.assertEqual(recorded, [])

    def test_command_runner_executes_in_worktree_with_task_packet_and_artifacts(self):
        script = self.root / "runner_success.py"
        script.write_text(
            "import json, os, pathlib\n"
            "packet = json.loads(pathlib.Path(os.environ['ALBERT_TASK_PACKET']).read_text())\n"
            "pathlib.Path('COMMAND_AGENT_RESULT.txt').write_text(packet['issue_id'])\n"
            "print('packet', packet['issue_id'])\n",
            encoding="utf-8",
        )
        command = f"{sys.executable} {script}"
        config_path = self.write_agent_config(
            [
                {
                    "id": "command-local",
                    "role": "local-agent",
                    "provider": "local",
                    "runner": "command",
                    "command": command,
                }
            ]
        )
        mission = self.load_mission_with_agent_config(config_path)
        mission.record_command_approval(command, "auto-allowed")
        mission.assign_issue("ISS-01", "command-local")
        mission.approve_issue("ISS-01")

        session = mission.launch_issue("ISS-01")
        mission.run_session(session.session_id)

        self.assertEqual(session.status, "evidence-ready")
        self.assertEqual(
            session.runner_exit_status,
            0,
            Path(session.artifacts["stderr"]).read_text(encoding="utf-8"),
        )
        self.assertEqual(session.evidence_valid, True)
        self.assertTrue((session.worktree_path / "COMMAND_AGENT_RESULT.txt").exists())
        self.assertIn("packet ISS-01", Path(session.artifacts["stdout"]).read_text(encoding="utf-8"))
        self.assertTrue(Path(session.artifacts["task_packet"]).exists())
        self.assertTrue(Path(session.artifacts["result"]).exists())

    def test_command_runner_is_sandboxed_to_worktree_and_receives_no_credentials(self):
        script = self.root / "runner_boundary.py"
        outside = self.root / "OUTSIDE_MUST_NOT_EXIST.txt"
        host_secret = self.root / "HOST_SECRET.txt"
        host_secret.write_text("must-not-be-readable", encoding="utf-8")
        script.write_text(
            "import os, pathlib, sys\n"
            "outside = pathlib.Path(sys.argv[1])\n"
            "try:\n"
            "    outside.write_text('escaped')\n"
            "except OSError:\n"
            "    pass\n"
            "try:\n"
            "    pathlib.Path(sys.argv[2]).read_text()\n"
            "except OSError:\n"
            "    pass\n"
            "else:\n"
            "    raise SystemExit(11)\n"
            "if 'OPENAI_API_KEY' in os.environ or 'ALFREDO_SECRET_SENTINEL' in os.environ:\n"
            "    raise SystemExit(10)\n"
            "pathlib.Path('INSIDE_ALLOWED.txt').write_text('bounded')\n",
            encoding="utf-8",
        )
        command = f"{sys.executable} {script} {outside} {host_secret}"
        config_path = self.write_agent_config(
            [
                {
                    "id": "command-local",
                    "role": "local-agent",
                    "provider": "local",
                    "runner": "command",
                    "command": command,
                }
            ]
        )
        mission = self.load_mission_with_agent_config(config_path)
        mission.record_command_approval(command, "auto-allowed")
        mission.assign_issue("ISS-01", "command-local")
        mission.approve_issue("ISS-01")

        with patch.dict(
            os.environ,
            {
                "OPENAI_API_KEY": "must-not-leak",
                "ALFREDO_SECRET_SENTINEL": "must-not-leak",
            },
        ):
            session = mission.launch_issue("ISS-01")
            mission.run_session(session.session_id)

        self.assertEqual(
            session.runner_exit_status,
            0,
            Path(session.artifacts["stderr"]).read_text(encoding="utf-8"),
        )
        self.assertFalse(outside.exists())
        self.assertEqual(
            (session.worktree_path / "INSIDE_ALLOWED.txt").read_text(encoding="utf-8"),
            "bounded",
        )

    def test_command_runner_mounts_only_an_absolute_custom_executable_not_its_siblings(self):
        tool_root = Path("/opt/albert-tools")
        tool = tool_root / "bin" / "custom-agent"
        sibling_secret = tool_root / "sibling-secret.txt"
        real_exists = Path.exists

        def fake_exists(path):
            if path in {tool_root, tool.parent, tool}:
                return True
            return real_exists(path)

        def fake_which(command, *args, **kwargs):
            if command == "bwrap":
                return "/usr/bin/bwrap"
            if command == str(tool):
                return str(tool)
            return None

        with (
            patch("albert_mvp.core.shutil.which", side_effect=fake_which),
            patch.object(Path, "exists", fake_exists),
        ):
            argv, sandboxed = core_module.sandboxed_process_argv(
                [str(tool), str(sibling_secret)],
                working_directory=self.target_repo,
                writable_roots=(self.target_repo,),
            )

        self.assertTrue(sandboxed)
        self.assertIsInstance(argv, list)
        readonly_sources = [
            Path(argv[index + 1])
            for index, token in enumerate(argv[:-2])
            if token == "--ro-bind"
        ]
        self.assertIn(tool, readonly_sources)
        self.assertFalse(
            any(
                sibling_secret == source or sibling_secret.is_relative_to(source)
                for source in readonly_sources
            )
        )

    def test_interpreter_script_mount_rejects_tmp_symlinks_and_outside_resolution(self):
        real_script = self.root / "real-script.py"
        real_script.write_text("print('bounded')\n", encoding="utf-8")
        script_symlink = self.root / "script-symlink.py"
        script_symlink.symlink_to(real_script)
        outside_parent = self.root / "outside-parent"
        outside_parent.symlink_to(Path("/etc"), target_is_directory=True)
        outside_resolved_script = outside_parent / "passwd"
        real_which = shutil.which

        def fake_which(command, *args, **kwargs):
            if command == "bwrap":
                return "/usr/bin/bwrap"
            return real_which(command, *args, **kwargs)

        for script, expected_error in (
            (script_symlink, "must not be a symlink"),
            (outside_resolved_script, "must resolve inside /tmp"),
        ):
            with self.subTest(script=script):
                with (
                    patch("albert_mvp.core.shutil.which", side_effect=fake_which),
                    self.assertRaisesRegex(AlbertError, expected_error),
                ):
                    core_module.sandboxed_process_argv(
                        [sys.executable, str(script)],
                        working_directory=self.target_repo,
                        writable_roots=(self.target_repo,),
                    )

    def test_custom_executable_mount_rejects_tmp_symlink_escape(self):
        executable_symlink = self.root / "custom-agent"
        executable_symlink.symlink_to(Path(sys.executable))
        real_which = shutil.which

        def fake_which(command, *args, **kwargs):
            if command == "bwrap":
                return "/usr/bin/bwrap"
            return real_which(command, *args, **kwargs)

        with (
            patch("albert_mvp.core.shutil.which", side_effect=fake_which),
            self.assertRaisesRegex(
                AlbertError,
                "Executable .* under /tmp must not be a symlink",
            ),
        ):
            core_module.sandboxed_process_argv(
                [str(executable_symlink)],
                working_directory=self.target_repo,
                writable_roots=(self.target_repo,),
            )

    def test_command_runner_does_not_mount_external_dependency_symlinks(self):
        external_dependencies = self.root / "external-dependencies"
        external_dependencies.mkdir()
        (external_dependencies / "secret.txt").write_text(
            "must-not-be-readable",
            encoding="utf-8",
        )
        (self.target_repo / "node_modules").symlink_to(
            external_dependencies,
            target_is_directory=True,
        )
        script = self.root / "runner_dependency_boundary.py"
        script.write_text(
            "import pathlib\n"
            "secret = pathlib.Path('node_modules/secret.txt')\n"
            "try:\n"
            "    secret.read_text()\n"
            "except OSError:\n"
            "    pathlib.Path('DEPENDENCY_BOUNDARY.txt').write_text('bounded')\n"
            "else:\n"
            "    pathlib.Path('DEPENDENCY_SECRET_LEAKED.txt').write_text('leaked')\n"
            "    raise SystemExit(9)\n",
            encoding="utf-8",
        )
        command = f"{sys.executable} {script}"
        config_path = self.write_agent_config(
            [
                {
                    "id": "dependency-boundary-local",
                    "role": "local-agent",
                    "provider": "local",
                    "runner": "command",
                    "command": command,
                }
            ]
        )
        mission = self.load_mission_with_agent_config(config_path)
        mission.record_command_approval(command, "auto-allowed")
        mission.assign_issue("ISS-01", "dependency-boundary-local")
        mission.approve_issue("ISS-01")

        session = mission.launch_issue("ISS-01")
        mission.run_session(session.session_id)

        self.assertEqual(session.runner_exit_status, 0)
        self.assertEqual(
            (session.worktree_path / "DEPENDENCY_BOUNDARY.txt").read_text(
                encoding="utf-8"
            ),
            "bounded",
        )
        self.assertFalse(
            (session.worktree_path / "DEPENDENCY_SECRET_LEAKED.txt").exists()
        )

    def test_command_runner_bounds_top_level_dependency_parent_discovery(self):
        self.initialize_target_git_repo(
            {
                "src/app.py": "VALUE = 1\n",
                "packages/a/app.py": "A = 1\n",
                "packages/b/app.py": "B = 1\n",
            }
        )
        command = f"{sys.executable} -c pass"
        config_path = self.write_agent_config(
            [
                {
                    "id": "bounded-dependency-local",
                    "role": "local-agent",
                    "provider": "local",
                    "runner": "command",
                    "command": command,
                }
            ]
        )
        mission = self.load_mission_with_agent_config(config_path)
        mission.record_command_approval(command, "auto-allowed")
        mission.assign_issue("ISS-01", "bounded-dependency-local")
        mission.approve_issue("ISS-01")
        session = mission.launch_issue("ISS-01")
        real_iterdir = Path.iterdir
        real_scandir = os.scandir
        scanned_entries = 0

        class ScandirGuard:
            def __init__(self, handle):
                self.handle = handle

            def __next__(self):
                nonlocal scanned_entries
                scanned_entries += 1
                if scanned_entries > 2:
                    raise AssertionError(
                        "dependency discovery exceeded its top-level entry budget"
                    )
                return next(self.handle)

            def __enter__(self):
                self.handle.__enter__()
                return self

            def __exit__(self, *args):
                return self.handle.__exit__(*args)

        def guarded_scandir(path):
            handle = real_scandir(path)
            if Path(path) == self.target_repo:
                return ScandirGuard(handle)
            return handle

        def reject_unbounded_iterdir(path):
            if path == self.target_repo:
                raise AssertionError(
                    "dependency discovery must not materialize every top-level child"
                )
            return real_iterdir(path)

        real_bounded_process = core_module._run_bounded_process

        def successful_process(argv, **kwargs):
            if isinstance(argv, list) and argv and argv[0] == "git":
                return real_bounded_process(argv, **kwargs)
            return subprocess.CompletedProcess(argv, 0, "", "")

        with (
            patch("albert_mvp.core._DEPENDENCY_PARENT_SCAN_LIMIT", 2),
            patch.object(Path, "iterdir", reject_unbounded_iterdir),
            patch("albert_mvp.core.os.scandir", side_effect=guarded_scandir),
            patch("albert_mvp.core._run_bounded_process", side_effect=successful_process),
        ):
            completed = mission.run_session(session.session_id)

        self.assertEqual(completed.runner_exit_status, 0)
        self.assertLessEqual(scanned_entries, 2)

    def test_command_runner_rejects_changes_outside_declared_allowed_paths(self):
        script = self.root / "runner_allowed_boundary.py"
        script.write_text(
            "import pathlib\n"
            "pathlib.Path('allowed.txt').write_text('allowed')\n"
            "pathlib.Path('undeclared.txt').write_text('must reject')\n",
            encoding="utf-8",
        )
        command = f"{sys.executable} {script}"
        config_path = self.write_agent_config(
            [
                {
                    "id": "command-local",
                    "role": "local-agent",
                    "provider": "local",
                    "runner": "command",
                    "command": command,
                }
            ]
        )
        mission = self.load_mission_with_agent_config(config_path)
        mission.record_command_approval(command, "auto-allowed")
        mission.assign_issue("ISS-01", "command-local")
        mission.approve_issue("ISS-01")

        session = mission.launch_issue("ISS-01", allowed_paths=["allowed.txt"])
        mission.run_session(session.session_id)

        self.assertEqual(session.status, "failed")
        self.assertEqual(session.evidence_valid, False)
        self.assertIn("outside allowed_paths", session.evidence.known_risks)
        self.assertIn("undeclared.txt", session.evidence.known_risks)

    def test_command_runner_does_not_collapse_invalid_utf8_git_paths_into_allowed_paths(self):
        script = self.root / "runner_invalid_utf8_path.py"
        script.write_text(
            "import os\n"
            "os.mkdir(b'allow\\xffed')\n"
            "descriptor = os.open(b'allow\\xffed/escape.txt', "
            "os.O_WRONLY | os.O_CREAT, 0o600)\n"
            "os.write(descriptor, b'must reject')\n"
            "os.close(descriptor)\n",
            encoding="utf-8",
        )
        command = f"{sys.executable} {script}"
        config_path = self.write_agent_config(
            [
                {
                    "id": "command-local",
                    "role": "local-agent",
                    "provider": "local",
                    "runner": "command",
                    "command": command,
                }
            ]
        )
        mission = self.load_mission_with_agent_config(config_path)
        mission.record_command_approval(command, "auto-allowed")
        mission.assign_issue("ISS-01", "command-local")
        mission.approve_issue("ISS-01")

        session = mission.launch_issue("ISS-01", allowed_paths=["allowed"])
        mission.run_session(session.session_id)

        self.assertEqual(session.status, "failed")
        self.assertFalse(session.evidence_valid)
        self.assertIn("outside allowed_paths", session.evidence.known_risks)

    def test_command_runner_fails_closed_when_process_sandbox_is_unavailable(self):
        command = f"{sys.executable} -c pass"
        config_path = self.write_agent_config(
            [
                {
                    "id": "command-local",
                    "role": "local-agent",
                    "provider": "local",
                    "runner": "command",
                    "command": command,
                }
            ]
        )
        mission = self.load_mission_with_agent_config(config_path)
        mission.record_command_approval(command, "auto-allowed")
        mission.assign_issue("ISS-01", "command-local")
        mission.approve_issue("ISS-01")
        session = mission.launch_issue("ISS-01")

        with patch(
            "albert_mvp.core.sandboxed_process_argv",
            side_effect=lambda argv, **_kwargs: (argv, False),
        ):
            mission.run_session(session.session_id)

        self.assertEqual(session.status, "failed")
        self.assertEqual(session.runner_exit_status, 126)
        self.assertIn(
            "bubblewrap (bwrap) is required",
            Path(session.artifacts["stderr"]).read_text(encoding="utf-8"),
        )

    def test_command_runner_records_nonzero_exit_without_losing_logs(self):
        script = self.root / "runner_fail.py"
        script.write_text("import sys\nprint('bad run', file=sys.stderr)\nsys.exit(7)\n", encoding="utf-8")
        command = f"{sys.executable} {script}"
        config_path = self.write_agent_config(
            [
                {
                    "id": "command-local",
                    "role": "local-agent",
                    "provider": "local",
                    "runner": "command",
                    "command": command,
                }
            ]
        )
        mission = self.load_mission_with_agent_config(config_path)
        mission.record_command_approval(command, "auto-allowed")
        mission.assign_issue("ISS-01", "command-local")
        mission.approve_issue("ISS-01")

        session = mission.launch_issue("ISS-01")
        mission.run_session(session.session_id)

        self.assertEqual(session.status, "failed")
        self.assertEqual(session.runner_exit_status, 7)
        self.assertIn("bad run", Path(session.artifacts["stderr"]).read_text(encoding="utf-8"))

    def test_command_runner_records_missing_command_as_failure(self):
        command = "missing-agent-command"
        config_path = self.write_agent_config(
            [
                {
                    "id": "command-local",
                    "role": "local-agent",
                    "provider": "local",
                    "runner": "command",
                    "command": command,
                }
            ]
        )
        mission = self.load_mission_with_agent_config(config_path)
        mission.record_command_approval(command, "auto-allowed")
        mission.assign_issue("ISS-01", "command-local")
        mission.approve_issue("ISS-01")

        session = mission.launch_issue("ISS-01")
        mission.run_session(session.session_id)

        self.assertEqual(session.status, "failed")
        self.assertEqual(session.runner_exit_status, 127)
        self.assertIn("missing-agent-command", Path(session.artifacts["stderr"]).read_text(encoding="utf-8"))

    def test_command_runner_blocks_human_required_command_before_execution(self):
        config_path = self.write_agent_config(
            [
                {
                    "id": "command-local",
                    "role": "local-agent",
                    "provider": "local",
                    "runner": "command",
                    "command": "rm -rf .",
                }
            ]
        )
        mission = self.load_mission_with_agent_config(config_path)
        mission.assign_issue("ISS-01", "command-local")
        mission.approve_issue("ISS-01")

        with self.assertRaisesRegex(LaunchBlockedError, "human-required"):
            mission.launch_issue("ISS-01")

        self.assertEqual(mission.sessions, {})

    def test_automated_evidence_collects_changed_files_and_diff_summary(self):
        script = self.root / "runner_changes.py"
        script.write_text(
            "import pathlib\n"
            "pathlib.Path('src').mkdir()\n"
            "pathlib.Path('src/app.py').write_text('print(\"hello\")\\n')\n"
            "print('changed src/app.py')\n",
            encoding="utf-8",
        )
        command = f"{sys.executable} {script}"
        config_path = self.write_agent_config(
            [
                {
                    "id": "command-local",
                    "role": "local-agent",
                    "provider": "local",
                    "runner": "command",
                    "command": command,
                }
            ]
        )
        mission = self.load_mission_with_agent_config(config_path)
        mission.record_command_approval(command, "auto-allowed")
        mission.assign_issue("ISS-01", "command-local")
        mission.approve_issue("ISS-01")

        session = mission.launch_issue("ISS-01")
        mission.run_session(session.session_id)

        self.assertEqual(session.status, "evidence-ready")
        self.assertEqual(session.evidence_valid, True)
        self.assertEqual(session.evidence.changed_files, ["src/app.py"])
        self.assertIn("src/app.py", session.evidence.diff_summary)
        self.assertNotIn("stdout.log", "\n".join(session.evidence.artifact_links))
        self.assertNotIn("task-packet.json", "\n".join(session.evidence.artifact_links))
        self.assertIn("review.diff", "\n".join(session.evidence.artifact_links))
        self.assertTrue(Path(session.artifacts["stdout"]).exists())

    def test_command_runner_bounds_stdout_and_stderr_artifacts(self):
        script = self.root / "runner_large_output.py"
        script.write_text(
            "import os, pathlib\n"
            "pathlib.Path('src').mkdir()\n"
            "pathlib.Path('src/bounded.py').write_text('VALUE = 1\\n')\n"
            "while True:\n"
            "    os.write(1, b'o' * 65536)\n"
            "    os.write(2, b'e' * 65536)\n"
            "pathlib.Path('SHOULD_NOT_EXIST.txt').write_text('runner escaped')\n",
            encoding="utf-8",
        )
        command = f"{sys.executable} {script}"
        config_path = self.write_agent_config(
            [
                {
                    "id": "bounded-command-local",
                    "role": "local-agent",
                    "provider": "local",
                    "runner": "command",
                    "command": command,
                }
            ]
        )
        mission = self.load_mission_with_agent_config(config_path)
        mission.record_command_approval(command, "auto-allowed")
        mission.assign_issue("ISS-01", "bounded-command-local")
        mission.approve_issue("ISS-01")

        session = mission.launch_issue("ISS-01")
        mission.run_session(session.session_id)

        stdout = Path(session.artifacts["stdout"]).read_bytes()
        stderr = Path(session.artifacts["stderr"]).read_bytes()
        self.assertEqual(session.runner_exit_status, 125)
        self.assertEqual(session.status, "failed")
        self.assertLessEqual(len(stdout) + len(stderr), 1_000_000)
        self.assertIn(b"output exceeded", stderr)
        self.assertIn(b"terminated", stderr)
        self.assertFalse((session.worktree_path / "SHOULD_NOT_EXIST.txt").exists())

    def test_model_file_plan_enforces_file_count_and_byte_budgets(self):
        with self.assertRaisesRegex(AlbertError, "128-file limit"):
            core_module._validate_model_file_plan(
                {
                    "files": [
                        {"path": f"src/file-{index}.py", "content": "x"}
                        for index in range(129)
                    ]
                }
            )
        with self.assertRaisesRegex(AlbertError, "512000-byte per-file limit"):
            core_module._validate_model_file_plan(
                {"files": [{"path": "src/large.py", "content": "x" * 512_001}]}
            )
        with self.assertRaisesRegex(AlbertError, "2000000-byte total limit"):
            core_module._validate_model_file_plan(
                {
                    "files": [
                        {"path": f"src/chunk-{index}.py", "content": "x" * 500_000}
                        for index in range(5)
                    ]
                }
            )

    def test_bounded_process_timeout_interrupts_a_child_that_does_not_read_stdin(self):
        started = time.monotonic()

        completed = core_module._run_bounded_process(
            [sys.executable, "-c", "import time; time.sleep(60)"],
            input_text="x" * 2_000_000,
            cwd=self.target_repo,
            timeout_seconds=0.1,
        )

        self.assertEqual(completed.returncode, 124)
        self.assertIn("timed out", completed.stderr.lower())
        self.assertLess(time.monotonic() - started, 2)

    def test_bounded_process_sanitizes_environment_when_callers_omit_env(self):
        with patch.dict(os.environ, {"OPENAI_API_KEY": "must-not-leak"}):
            completed = core_module._run_bounded_process(
                [
                    sys.executable,
                    "-c",
                    "import os; print(os.environ.get('OPENAI_API_KEY', 'missing'))",
                ],
                cwd=self.target_repo,
                timeout_seconds=2,
            )

        self.assertEqual(completed.returncode, 0)
        self.assertEqual(completed.stdout.strip(), "missing")

    def test_bounded_process_catches_descendant_output_after_leader_exits(self):
        if os.name != "posix" or not hasattr(os, "fork"):
            self.skipTest("process-group descendant coverage requires POSIX fork")
        script = (
            "import os, time; "
            "child = os.fork(); "
            "(time.sleep(0.05), os.write(1, b'x' * 100_000), os._exit(0)) "
            "if child == 0 else os._exit(0)"
        )

        completed = core_module._run_bounded_process(
            [sys.executable, "-c", script],
            cwd=self.target_repo,
            timeout_seconds=1,
            output_limit_bytes=1_024,
        )

        self.assertEqual(completed.returncode, 125)
        self.assertIn("output exceeded", completed.stderr.lower())
        self.assertLessEqual(
            len(completed.stdout.encode("utf-8"))
            + len(completed.stderr.encode("utf-8")),
            1_024,
        )

    def test_bounded_process_rejects_invalid_utf8_without_expanding_byte_budget(self):
        completed = core_module._run_bounded_process(
            [
                sys.executable,
                "-c",
                "import os; os.write(1, b'\\xff' * 400000)",
            ],
            cwd=self.target_repo,
            timeout_seconds=2,
            output_limit_bytes=1_000_000,
        )

        self.assertEqual(completed.returncode, 125)
        self.assertIn("valid UTF-8", completed.stderr)
        self.assertLessEqual(
            len(completed.stdout.encode("utf-8"))
            + len(completed.stderr.encode("utf-8")),
            1_000_000,
        )

    def test_bounded_process_times_out_descendant_after_leader_exits(self):
        if os.name != "posix" or not hasattr(os, "fork"):
            self.skipTest("process-group descendant coverage requires POSIX fork")
        script = (
            "import os, time; "
            "child = os.fork(); "
            "(time.sleep(60), os._exit(0)) if child == 0 else os._exit(0)"
        )
        process_groups: list[int] = []
        started = time.monotonic()

        try:
            completed = core_module._run_bounded_process(
                [sys.executable, "-c", script],
                cwd=self.target_repo,
                timeout_seconds=0.1,
                process_started=lambda process: process_groups.append(process.pid),
            )
        finally:
            for process_group in process_groups:
                try:
                    os.killpg(process_group, signal.SIGKILL)
                except ProcessLookupError:
                    pass

        self.assertEqual(completed.returncode, 124)
        self.assertIn("timed out", completed.stderr.lower())
        self.assertLess(time.monotonic() - started, 2)

    def test_bounded_process_terminates_descendant_that_creates_a_new_session(self):
        if os.name != "posix" or not hasattr(os, "fork"):
            self.skipTest("detached descendant coverage requires POSIX fork")
        survivor_path = self.root / "detached-descendant-survived.txt"
        detached_script = (
            "import pathlib, time; time.sleep(0.3); "
            f"pathlib.Path({str(survivor_path)!r}).write_text('survived')"
        )
        script = (
            "import os, pathlib, sys; "
            "child = os.fork(); "
            "(os.setsid(), os.execve(sys.executable, [sys.executable, '-c', "
            f"{detached_script!r}], {{'PATH': os.defpath}})) "
            "if child == 0 else os._exit(0)"
        )

        with patch("albert_mvp.core._PROCESS_DESCENDANT_GRACE_SECONDS", 0.1):
            completed = core_module._run_bounded_process(
                [sys.executable, "-c", script],
                cwd=self.target_repo,
                timeout_seconds=1,
            )
        time.sleep(0.4)

        self.assertIn(completed.returncode, {0, 124})
        self.assertFalse(survivor_path.exists())

    def test_bounded_process_applies_practical_host_resource_limits(self):
        if os.name != "posix" or core_module.shutil.which("prlimit") is None:
            self.skipTest("prlimit is unavailable on this platform")
        script = (
            "import json, resource; "
            "print(json.dumps({"
            "'address_space': resource.getrlimit(resource.RLIMIT_AS)[0], "
            "'file_size': resource.getrlimit(resource.RLIMIT_FSIZE)[0], "
            "'open_files': resource.getrlimit(resource.RLIMIT_NOFILE)[0], "
            "'processes': resource.getrlimit(resource.RLIMIT_NPROC)[0]}))"
        )

        completed = core_module._run_bounded_process(
            [sys.executable, "-c", script],
            cwd=self.target_repo,
            timeout_seconds=2,
        )

        self.assertEqual(completed.returncode, 0)
        limits = json.loads(completed.stdout)
        self.assertEqual(
            limits,
            {
                "address_space": core_module._PROCESS_ADDRESS_SPACE_BYTES_LIMIT,
                "file_size": core_module._PROCESS_FILE_SIZE_BYTES_LIMIT,
                "open_files": core_module._PROCESS_OPEN_FILE_LIMIT,
                "processes": core_module._PROCESS_COUNT_LIMIT,
            },
        )

    def test_sandboxed_process_applies_resource_limits_after_namespace_creation(self):
        helper_paths = {
            "bwrap": "/usr/bin/bwrap",
            "prlimit": "/usr/bin/prlimit",
        }
        with patch(
            "albert_mvp.core._trusted_system_executable",
            side_effect=lambda name: helper_paths.get(name),
        ):
            sandbox_argv, sandboxed = core_module.sandboxed_process_argv(
                [sys.executable, "-c", "print('bounded')"],
                working_directory=self.target_repo,
                readable_roots=(self.target_repo,),
            )
            isolated_argv = core_module._process_isolated_argv(sandbox_argv)

        self.assertTrue(sandboxed)
        self.assertEqual(isolated_argv[0], "/usr/bin/bwrap")
        payload_start = isolated_argv.index("--") + 1
        self.assertEqual(isolated_argv[payload_start], "/usr/bin/prlimit")
        self.assertIn(f"--nproc={core_module._PROCESS_COUNT_LIMIT}", isolated_argv)

    def test_process_enforcement_helpers_ignore_untrusted_path_wrappers(self):
        fake_bin = self.root / "untrusted-process-wrappers"
        fake_bin.mkdir()
        fake_bwrap = fake_bin / "bwrap"
        fake_prlimit = fake_bin / "prlimit"
        for path in (fake_bwrap, fake_prlimit):
            path.write_text("#!/bin/sh\nexit 0\n", encoding="utf-8")
            path.chmod(0o755)

        with patch.dict(os.environ, {"PATH": str(fake_bin)}):
            sandbox_argv, sandboxed = core_module.sandboxed_process_argv(
                ["/usr/bin/true"],
                working_directory=self.target_repo,
                readable_roots=(self.target_repo,),
            )
            resource_argv = core_module._resource_bounded_process_argv(
                ["/usr/bin/true"]
            )

        self.assertTrue(sandboxed)
        self.assertNotEqual(Path(sandbox_argv[0]), fake_bwrap)
        self.assertNotEqual(Path(resource_argv[0]), fake_prlimit)
        self.assertTrue(Path(sandbox_argv[0]).resolve().is_relative_to(Path("/usr")))
        self.assertTrue(Path(resource_argv[0]).resolve().is_relative_to(Path("/usr")))

    def test_review_diff_redacts_blocked_file_contents_but_keeps_metadata(self):
        script = self.root / "runner_sensitive_change.py"
        script.write_text(
            "import pathlib\n"
            "pathlib.Path('.env').write_text('TOKEN=must-never-enter-review-diff\\n')\n"
            "pathlib.Path('safe.py').write_text('VALUE = 42\\n')\n",
            encoding="utf-8",
        )
        command = f"{sys.executable} {script}"
        config_path = self.write_agent_config(
            [
                {
                    "id": "command-local",
                    "role": "local-agent",
                    "provider": "local",
                    "runner": "command",
                    "command": command,
                }
            ]
        )
        mission = self.load_mission_with_agent_config(config_path)
        mission.record_command_approval(command, "auto-allowed")
        mission.assign_issue("ISS-01", "command-local")
        mission.approve_issue("ISS-01")

        session = mission.launch_issue("ISS-01")
        mission.run_session(session.session_id)

        self.assertIn(
            "review_diff",
            session.artifacts,
            (
                session.status,
                session.runner_exit_status,
                session.task_packet.get("runner_failure"),
                Path(session.artifacts["stderr"]).read_text(encoding="utf-8"),
                session.artifacts,
            ),
        )
        review_diff = Path(session.artifacts["review_diff"]).read_text(encoding="utf-8")
        self.assertIn("+++ b/.env", review_diff)
        self.assertIn("Blocked file content omitted", review_diff)
        self.assertNotIn("must-never-enter-review-diff", review_diff)
        self.assertIn("VALUE = 42", review_diff)

    def test_review_diff_uses_the_session_baseline_when_parent_file_changes_during_run(self):
        self.initialize_target_git_repo(
            {"src/app.py": "VALUE = 'launch baseline'\n"}
        )
        script = self.root / "runner_concurrent_parent_edit.py"
        script.write_text(
            "import pathlib, time\n"
            "time.sleep(0.4)\n"
            "pathlib.Path('src/app.py').write_text(\"VALUE = 'agent result'\\n\")\n",
            encoding="utf-8",
        )
        command = f"{sys.executable} {script}"
        config_path = self.write_agent_config(
            [
                {
                    "id": "concurrent-parent-local",
                    "role": "local-agent",
                    "provider": "local",
                    "runner": "command",
                    "command": command,
                }
            ]
        )
        mission = self.load_mission_with_agent_config(config_path)
        mission.record_command_approval(command, "auto-allowed")
        mission.assign_issue("ISS-01", "concurrent-parent-local")
        mission.approve_issue("ISS-01")
        session = mission.launch_issue("ISS-01", allowed_paths=["src/app.py"])
        runner_errors: list[BaseException] = []

        def run_session():
            try:
                mission.run_session(session.session_id)
            except BaseException as exc:  # pragma: no cover - asserted below
                runner_errors.append(exc)

        runner = threading.Thread(target=run_session)
        runner.start()
        deadline = time.monotonic() + 5
        while not session.repository_snapshot and time.monotonic() < deadline:
            time.sleep(0.01)
        self.assertTrue(session.repository_snapshot)
        (self.target_repo / "src" / "app.py").write_text(
            "VALUE = 'concurrent user edit'\n",
            encoding="utf-8",
        )
        runner.join(timeout=10)

        self.assertFalse(runner.is_alive())
        self.assertEqual(runner_errors, [])
        review_diff = Path(session.artifacts["review_diff"]).read_text(
            encoding="utf-8"
        )
        self.assertIn("-VALUE = 'launch baseline'", review_diff)
        self.assertIn("+VALUE = 'agent result'", review_diff)
        self.assertNotIn("concurrent user edit", review_diff)
        self.assertEqual(
            (self.target_repo / "src" / "app.py").read_text(encoding="utf-8"),
            "VALUE = 'concurrent user edit'\n",
        )

    def test_review_diff_handles_a_large_sparse_change_without_whole_file_reads(self):
        script = self.root / "runner_sparse_change.py"
        script.write_text(
            "import pathlib\n"
            "path = pathlib.Path('src/huge_sparse.py')\n"
            "path.parent.mkdir()\n"
            "with path.open('wb') as output:\n"
            "    output.write(b'# sparse source\\n')\n"
            f"    output.truncate({core_module._REVIEW_DIFF_BYTES_LIMIT * 16})\n",
            encoding="utf-8",
        )
        command = f"{sys.executable} {script}"
        config_path = self.write_agent_config(
            [
                {
                    "id": "sparse-command-local",
                    "role": "local-agent",
                    "provider": "local",
                    "runner": "command",
                    "command": command,
                }
            ]
        )
        mission = self.load_mission_with_agent_config(config_path)
        mission.record_command_approval(command, "auto-allowed")
        mission.assign_issue("ISS-01", "sparse-command-local")
        mission.approve_issue("ISS-01")
        session = mission.launch_issue("ISS-01")

        original_open = Path.open

        class BoundedReadGuard:
            def __init__(self, handle):
                self._handle = handle

            def read(self, size=-1):
                if size is None or size < 0:
                    raise AssertionError("large sparse source must use a bounded read")
                return self._handle.read(size)

            def __enter__(self):
                self._handle.__enter__()
                return self

            def __exit__(self, *args):
                return self._handle.__exit__(*args)

            def __getattr__(self, name):
                return getattr(self._handle, name)

        def guarded_open(path, mode="r", *args, **kwargs):
            handle = original_open(path, mode, *args, **kwargs)
            if path.name == "huge_sparse.py" and "b" in mode and "r" in mode:
                return BoundedReadGuard(handle)
            return handle

        with patch.object(Path, "open", guarded_open):
            mission.run_session(session.session_id)

        review_diff = Path(session.artifacts["review_diff"]).read_text(encoding="utf-8")
        self.assertEqual(session.status, "evidence-ready")
        self.assertIn("Binary or oversized change: src/huge_sparse.py", review_diff)
        self.assertLessEqual(
            Path(session.artifacts["review_diff"]).stat().st_size,
            core_module._REVIEW_DIFF_BYTES_LIMIT,
        )

    def test_blocked_model_file_content_is_not_linked_through_raw_runner_artifacts(self):
        fake_ollama = self.root / "fake_ollama_secret.py"
        fake_ollama.write_text(
            "import json\n"
            "print(json.dumps({'summary': 'secret', 'files': ["
            "{'path': '.env', 'content': 'TOKEN=raw-model-secret\\n'}, "
            "{'path': 'safe.py', 'content': 'VALUE = 42\\n'}]}))\n",
            encoding="utf-8",
        )
        command = f"{sys.executable} {fake_ollama}"
        config_path = self.write_agent_config(
            [
                {
                    "id": "secret-model-worker",
                    "role": "local-agent",
                    "provider": "ollama",
                    "runner": "ollama",
                    "model": "secret:test",
                    "command": command,
                    "routing": "worker",
                }
            ]
        )
        mission = self.load_mission_with_agent_config(config_path)
        mission.record_command_approval(command, "auto-allowed")
        mission.assign_issue("ISS-01", "secret-model-worker")
        mission.approve_issue("ISS-01")

        session = mission.launch_issue("ISS-01", allowed_paths=["."])
        mission.run_session(session.session_id)

        raw_output = Path(session.artifacts["ollama_output"])
        self.assertIn("raw-model-secret", raw_output.read_text(encoding="utf-8"))
        self.assertNotIn(str(raw_output), session.evidence.artifact_links)
        self.assertIn(session.artifacts["review_diff"], session.evidence.artifact_links)
        self.assertNotIn(
            str(raw_output),
            ReviewWorkspaceService(WorkspaceSnapshotService(mission))
            .inspect()
            .items[0]
            .evidence.artifact_links,
        )

    def test_edit_requiring_runner_cannot_report_no_diff_as_valid_evidence(self):
        script = self.root / "runner_no_changes.py"
        script.write_text(
            "import pathlib\n"
            "pathlib.Path('__pycache__').mkdir(exist_ok=True)\n"
            "pathlib.Path('__pycache__/generated.pyc').write_bytes(b'cache only')\n"
            "print('no source file edits')\n",
            encoding="utf-8",
        )
        command = f"{sys.executable} {script}"
        config_path = self.write_agent_config(
            [
                {
                    "id": "command-local",
                    "role": "local-agent",
                    "provider": "local",
                    "runner": "command",
                    "command": command,
                }
            ]
        )
        mission = self.load_mission_with_agent_config(config_path)
        mission.record_command_approval(command, "auto-allowed")
        mission.assign_issue("ISS-01", "command-local")
        mission.approve_issue("ISS-01")

        session = mission.launch_issue("ISS-01")
        mission.run_session(session.session_id)

        self.assertEqual(session.status, "failed")
        self.assertEqual(session.evidence_valid, False)
        self.assertEqual(session.evidence.changed_files, [])
        self.assertEqual(
            session.evidence.diff_summary,
            "No agent-authored worktree file changes detected.",
        )
        self.assertIn("produced no agent-authored file changes", session.evidence.known_risks)
        self.assertEqual(session.evidence.test_results, "Not applicable: no test command configured.")

    def test_automated_evidence_captures_configured_test_success(self):
        runner = self.root / "runner_with_tests.py"
        runner.write_text("import pathlib\npathlib.Path('result.txt').write_text('ok')\n", encoding="utf-8")
        test_script = self.root / "test_success.py"
        test_script.write_text("print('tests ok')\n", encoding="utf-8")
        command = f"{sys.executable} {runner}"
        test_command = f"{sys.executable} {test_script}"
        config_path = self.write_agent_config(
            [
                {
                    "id": "command-local",
                    "role": "local-agent",
                    "provider": "local",
                    "runner": "command",
                    "command": command,
                    "test_command": test_command,
                }
            ]
        )
        mission = self.load_mission_with_agent_config(config_path)
        mission.record_command_approval(command, "auto-allowed")
        mission.record_command_approval(test_command, "auto-allowed")
        mission.assign_issue("ISS-01", "command-local")
        mission.approve_issue("ISS-01")

        session = mission.launch_issue("ISS-01")
        mission.run_session(session.session_id)

        self.assertIn("Test command passed", session.evidence.test_results)
        self.assertIn("tests ok", Path(session.artifacts["test_stdout"]).read_text(encoding="utf-8"))

    def test_automated_evidence_captures_configured_test_failure(self):
        runner = self.root / "runner_with_failing_tests.py"
        runner.write_text("import pathlib\npathlib.Path('result.txt').write_text('ok')\n", encoding="utf-8")
        test_script = self.root / "test_failure.py"
        test_script.write_text("import sys\nprint('tests failed', file=sys.stderr)\nsys.exit(5)\n", encoding="utf-8")
        command = f"{sys.executable} {runner}"
        test_command = f"{sys.executable} {test_script}"
        config_path = self.write_agent_config(
            [
                {
                    "id": "command-local",
                    "role": "local-agent",
                    "provider": "local",
                    "runner": "command",
                    "command": command,
                    "test_command": test_command,
                }
            ]
        )
        mission = self.load_mission_with_agent_config(config_path)
        mission.record_command_approval(command, "auto-allowed")
        mission.record_command_approval(test_command, "auto-allowed")
        mission.assign_issue("ISS-01", "command-local")
        mission.approve_issue("ISS-01")

        session = mission.launch_issue("ISS-01")
        mission.run_session(session.session_id)

        self.assertEqual(session.status, "failed")
        self.assertEqual(session.evidence_valid, True)
        self.assertIn("Test command failed (exit 5)", session.evidence.test_results)
        self.assertIn("tests failed", Path(session.artifacts["test_stderr"]).read_text(encoding="utf-8"))

    def test_tui_state_shows_sessions_ready_for_review_with_evidence(self):
        config_path = self.write_agent_config(
            [
                {
                    "id": "fake-local",
                    "role": "local-agent",
                    "provider": "local",
                    "runner": "fake",
                }
            ]
        )
        mission = self.load_mission_with_agent_config(config_path)
        mission.assign_issue("ISS-01", "fake-local")
        mission.approve_issue("ISS-01")
        session = mission.launch_issue("ISS-01")
        mission.run_session(session.session_id)

        state = build_tui_state(mission)

        self.assertEqual(state.review_queue[0].session_id, session.session_id)
        self.assertEqual(state.review_queue[0].issue_id, "ISS-01")
        self.assertEqual(state.review_queue[0].evidence_valid, True)
        self.assertIn("fake-agent.log", "\n".join(state.review_queue[0].artifact_links))

    def test_tui_review_action_approves_session_and_marks_issue_pr_ready(self):
        config_path = self.write_agent_config(
            [
                {
                    "id": "fake-local",
                    "role": "local-agent",
                    "provider": "local",
                    "runner": "fake",
                }
            ]
        )
        mission = self.load_mission_with_agent_config(config_path)
        mission.assign_issue("ISS-01", "fake-local")
        mission.approve_issue("ISS-01")
        session = mission.launch_issue("ISS-01")
        mission.run_session(session.session_id)

        result = perform_tui_action(
            mission,
            "review",
            "ISS-01",
            session_id=session.session_id,
            outcome="Approved",
            reason="Evidence satisfies the slice.",
            expected_revision=mission.sessions[session.session_id].revision,
        )
        reloaded = self.load_mission_with_agent_config(config_path)

        self.assertEqual(result.next_action, "prepare-pr")
        self.assertEqual(reloaded.issues["ISS-01"].review_state, "pr-ready")
        self.assertEqual(reloaded.sessions[session.session_id].status, "reviewed")

    def test_tui_review_action_normalizes_lowercase_outcome(self):
        config_path = self.write_agent_config(
            [
                {
                    "id": "fake-local",
                    "role": "local-agent",
                    "provider": "local",
                    "runner": "fake",
                }
            ]
        )
        mission = self.load_mission_with_agent_config(config_path)
        mission.assign_issue("ISS-01", "fake-local")
        mission.approve_issue("ISS-01")
        session = mission.launch_issue("ISS-01")
        mission.run_session(session.session_id)

        result = perform_tui_action(
            mission,
            "review",
            "ISS-01",
            session_id=session.session_id,
            outcome="approved",
            reason="Evidence satisfies the slice.",
            expected_revision=mission.sessions[session.session_id].revision,
        )
        reloaded = self.load_mission_with_agent_config(config_path)

        self.assertEqual(result.next_action, "prepare-pr")
        self.assertEqual(reloaded.reviews[-1].outcome, "Approved")
        self.assertEqual(reloaded.issues["ISS-01"].review_state, "pr-ready")

    def test_tui_review_action_rejects_unknown_outcome(self):
        config_path = self.write_agent_config(
            [
                {
                    "id": "fake-local",
                    "role": "local-agent",
                    "provider": "local",
                    "runner": "fake",
                }
            ]
        )
        mission = self.load_mission_with_agent_config(config_path)
        mission.assign_issue("ISS-01", "fake-local")
        mission.approve_issue("ISS-01")
        session = mission.launch_issue("ISS-01")

        with self.assertRaisesRegex(AlbertError, "Unknown review outcome"):
            perform_tui_action(
                mission,
                "review",
                "ISS-01",
                session_id=session.session_id,
                outcome="ship-it",
                reason="Typo.",
                expected_revision=mission.sessions[session.session_id].revision,
            )

    def test_tui_review_action_blocks_approval_without_valid_evidence(self):
        mission = self.load_mission()
        mission.approve_issue("ISS-01")
        session = mission.launch_issue("ISS-01")

        with self.assertRaisesRegex(EvidenceValidationError, "valid Evidence Package"):
            perform_tui_action(
                mission,
                "review",
                "ISS-01",
                session_id=session.session_id,
                outcome="Approved",
                reason="No evidence yet.",
                expected_revision=mission.sessions[session.session_id].revision,
            )

    def test_tui_review_action_routes_needs_repair_and_rejections(self):
        config_path = self.write_agent_config(
            [
                {
                    "id": "fake-local",
                    "role": "local-agent",
                    "provider": "local",
                    "runner": "fake",
                }
            ]
        )
        mission = self.load_mission_with_agent_config(config_path)
        mission.assign_issue("ISS-01", "fake-local")
        mission.approve_issue("ISS-01")
        session = mission.launch_issue("ISS-01")
        mission.run_session(session.session_id)

        repair = perform_tui_action(
            mission,
            "review",
            "ISS-01",
            session_id=session.session_id,
            outcome="Needs repair",
            reason="Acceptance detail missing.",
            expected_revision=mission.sessions[session.session_id].revision,
        )
        first_reject = perform_tui_action(
            mission,
            "review",
            "ISS-01",
            session_id=session.session_id,
            outcome="Rejected",
            reason="Still incomplete.",
            expected_revision=mission.sessions[session.session_id].revision,
        )
        second_reject = perform_tui_action(
            mission,
            "review",
            "ISS-01",
            session_id=session.session_id,
            outcome="Rejected",
            reason="Still incomplete.",
            expected_revision=mission.sessions[session.session_id].revision,
        )

        self.assertEqual(repair.next_action, "same-local-agent-repair")
        self.assertEqual(first_reject.next_action, "same-local-agent-repair")
        self.assertEqual(second_reject.next_action, "fresh-local-agent-repair")

    def test_repair_launch_reuses_local_agent_with_frontier_feedback(self):
        config_path = self.write_agent_config(
            [
                {
                    "id": "fake-local",
                    "role": "local-agent",
                    "provider": "local",
                    "runner": "fake",
                }
            ]
        )
        mission = self.load_mission_with_agent_config(config_path)
        mission.assign_issue("ISS-01", "fake-local")
        mission.approve_issue("ISS-01")
        first_session = mission.launch_issue("ISS-01")
        mission.run_session(first_session.session_id)
        mission.record_frontier_review(
            first_session.session_id,
            "Needs repair",
            reason="Acceptance detail missing.",
        )

        self.assertIn("repair", mission.issue_detail("ISS-01")["next_actions"])
        repair_session = mission.launch_repair(first_session.session_id, allowed_paths=["prototype"])

        repair_context = repair_session.task_packet["repair_context"]
        self.assertEqual(repair_session.session_id, "session-ISS-01-2")
        self.assertEqual(repair_session.issue_id, "ISS-01")
        self.assertEqual(repair_session.assigned_agent, "fake-local")
        self.assertEqual(repair_session.task_packet["allowed_paths"], ["prototype"])
        self.assertEqual(repair_context["prior_session_id"], first_session.session_id)
        self.assertEqual(repair_context["review_outcome"], "Needs repair")
        self.assertEqual(repair_context["review_reason"], "Acceptance detail missing.")
        self.assertEqual(repair_context["next_action"], "same-local-agent-repair")
        self.assertEqual(repair_context["prior_evidence"]["changed_files"], ["FAKE_AGENT_RESULT.md"])

    def test_tui_action_launches_repair_session_from_review(self):
        config_path = self.write_agent_config(
            [
                {
                    "id": "fake-local",
                    "role": "local-agent",
                    "provider": "local",
                    "runner": "fake",
                }
            ]
        )
        mission = self.load_mission_with_agent_config(config_path)
        mission.assign_issue("ISS-01", "fake-local")
        mission.approve_issue("ISS-01")
        first_session = mission.launch_issue("ISS-01")
        mission.run_session(first_session.session_id)
        mission.record_frontier_review(
            first_session.session_id,
            "Needs repair",
            reason="Acceptance detail missing.",
        )

        result = perform_tui_action(
            mission,
            "repair",
            "ISS-01",
            session_id=first_session.session_id,
            allowed_paths=["prototype"],
            expected_revision=mission.sessions[first_session.session_id].revision,
        )
        reloaded = self.load_mission_with_agent_config(config_path)

        self.assertEqual(result.message, "Launched repair for ISS-01 as session-ISS-01-2.")
        self.assertEqual(result.session_id, "session-ISS-01-2")
        self.assertEqual(reloaded.sessions["session-ISS-01-2"].task_packet["repair_context"]["review_reason"], "Acceptance detail missing.")
        with self.assertRaisesRegex(
            AlbertError,
            "Repair was already launched for session-ISS-01-1 as session-ISS-01-2",
        ):
            perform_tui_action(
                reloaded,
                "repair",
                "ISS-01",
                session_id=first_session.session_id,
                allowed_paths=["prototype"],
                expected_revision=reloaded.sessions[first_session.session_id].revision,
            )
        self.assertEqual(
            list(self.load_mission_with_agent_config(config_path).sessions),
            ["session-ISS-01-1", "session-ISS-01-2"],
        )

    def test_concurrent_public_tui_repair_launches_exactly_one_child(self):
        config_path = self.write_agent_config(
            [
                {
                    "id": "fake-local",
                    "role": "local-agent",
                    "provider": "local",
                    "runner": "fake",
                }
            ]
        )
        mission = self.load_mission_with_agent_config(config_path)
        mission.assign_issue("ISS-01", "fake-local")
        mission.approve_issue("ISS-01")
        first_session = mission.launch_issue("ISS-01")
        mission.run_session(first_session.session_id)
        mission.record_frontier_review(
            first_session.session_id,
            "Needs repair",
            reason="Acceptance detail missing.",
        )
        callers = [
            self.load_mission_with_agent_config(config_path),
            self.load_mission_with_agent_config(config_path),
        ]
        barrier = threading.Barrier(2)
        results = []
        errors: list[BaseException] = []

        def launch(candidate):
            try:
                barrier.wait(timeout=5)
                results.append(
                    perform_tui_action(
                        candidate,
                        "repair",
                        "ISS-01",
                        session_id=first_session.session_id,
                        allowed_paths=["prototype"],
                        expected_revision=candidate.sessions[
                            first_session.session_id
                        ].revision,
                    )
                )
            except BaseException as exc:  # pragma: no cover - asserted below
                errors.append(exc)

        threads = [threading.Thread(target=launch, args=(candidate,)) for candidate in callers]
        for thread in threads:
            thread.start()
        for thread in threads:
            thread.join(timeout=10)

        self.assertTrue(all(not thread.is_alive() for thread in threads))
        self.assertEqual(len(results), 1)
        self.assertEqual(results[0].session_id, "session-ISS-01-2")
        self.assertEqual(len(errors), 1)
        self.assertIsInstance(errors[0], LaunchBlockedError)
        self.assertRegex(
            str(errors[0]),
            "Repair was already launched|lifecycle revision is stale",
        )
        reloaded = self.load_mission_with_agent_config(config_path)
        repair_children = [
            session
            for session in reloaded.sessions.values()
            if session.task_packet.get("repair_context", {}).get("prior_session_id")
            == first_session.session_id
        ]
        self.assertEqual([session.session_id for session in repair_children], ["session-ISS-01-2"])

    def test_concurrent_issue_launch_and_repair_allocate_distinct_sessions(self):
        config_path = self.write_agent_config(
            [
                {
                    "id": "fake-local",
                    "role": "local-agent",
                    "provider": "local",
                    "runner": "fake",
                    "routing": "worker",
                }
            ]
        )
        mission = self.load_mission_with_agent_config(config_path)
        mission.assign_issue("ISS-01", "fake-local")
        mission.approve_issue("ISS-01")
        first = mission.launch_issue("ISS-01")
        mission.run_session(first.session_id)
        mission.record_frontier_review(
            first.session_id,
            "Needs repair",
            reason="The first result needs one canonical repair.",
        )
        mission.approve_issue("ISS-01")

        repair_caller = self.load_mission_with_agent_config(config_path)
        launch_caller = self.load_mission_with_agent_config(config_path)
        persistence_barrier = threading.Barrier(2)
        original_repair_persist = repair_caller._persist
        original_launch_persist = launch_caller._persist
        results: list[tuple[str, LocalAgentSession]] = []
        errors: list[BaseException] = []

        def synchronized_persist(original):
            try:
                persistence_barrier.wait(timeout=0.5)
            except threading.BrokenBarrierError:
                pass
            return original()

        def repair() -> None:
            try:
                results.append(
                    ("repair", repair_caller.launch_repair(first.session_id))
                )
            except BaseException as exc:  # pragma: no cover - asserted below
                errors.append(exc)

        def launch() -> None:
            try:
                results.append(("launch", launch_caller.launch_issue("ISS-01")))
            except BaseException as exc:  # pragma: no cover - asserted below
                errors.append(exc)

        with (
            patch.object(
                repair_caller,
                "_persist",
                side_effect=lambda: synchronized_persist(original_repair_persist),
            ),
            patch.object(
                launch_caller,
                "_persist",
                side_effect=lambda: synchronized_persist(original_launch_persist),
            ),
        ):
            threads = [threading.Thread(target=repair), threading.Thread(target=launch)]
            for thread in threads:
                thread.start()
            for thread in threads:
                thread.join(timeout=5)

        self.assertTrue(all(not thread.is_alive() for thread in threads))
        self.assertEqual(errors, [])
        self.assertEqual(len(results), 2)
        self.assertEqual(len({session.session_id for _, session in results}), 2)
        restored = self.load_mission_with_agent_config(config_path)
        self.assertEqual(len(restored.sessions), 3)
        self.assertEqual(
            len(
                [
                    session
                    for session in restored.sessions.values()
                    if session.task_packet.get("repair_context", {}).get(
                        "prior_session_id"
                    )
                    == first.session_id
                ]
            ),
            1,
        )

    def test_tui_state_shows_pr_ready_and_blocked_slices(self):
        config_path = self.write_agent_config(
            [
                {
                    "id": "fake-local",
                    "role": "local-agent",
                    "provider": "local",
                    "runner": "fake",
                }
            ]
        )
        mission = self.load_mission_with_agent_config(config_path)
        mission.assign_issue("ISS-01", "fake-local")
        mission.approve_issue("ISS-01")
        session = mission.launch_issue("ISS-01")
        mission.run_session(session.session_id)
        mission.record_frontier_review(session.session_id, "Approved", reason="Ready.")

        state = build_tui_state(mission)

        self.assertEqual(state.pr_queue[0].issue_id, "ISS-01")
        self.assertEqual(state.pr_queue[0].ready, True)
        self.assertEqual(state.pr_queue[0].merge_approved, False)
        self.assertEqual(state.pr_queue[1].issue_id, "ISS-02")
        self.assertEqual(state.pr_queue[1].ready, False)
        self.assertIn("not PR-ready", state.pr_queue[1].reason)

    def test_tui_prepare_pr_action_returns_manual_fallback_without_merge_approval(self):
        config_path = self.write_agent_config(
            [
                {
                    "id": "fake-local",
                    "role": "local-agent",
                    "provider": "local",
                    "runner": "fake",
                }
            ]
        )
        mission = self.load_mission_with_agent_config(config_path)
        mission.assign_issue("ISS-01", "fake-local")
        mission.approve_issue("ISS-01")
        session = mission.launch_issue("ISS-01")
        mission.run_session(session.session_id)
        mission.record_frontier_review(session.session_id, "Approved", reason="Ready.")

        result = perform_tui_action(mission, "prepare-pr", "ISS-01", gh_available=False)

        self.assertIn("Manual PR instructions", result.body)
        self.assertEqual(result.create_command, "")
        self.assertEqual(result.merge_approved, False)
        self.assertIn("Mission summary is visible.", result.body)
        self.assertIn("Approved: Ready.", result.body)

    def test_tui_prepare_pr_action_returns_github_command_when_available(self):
        config_path = self.write_agent_config(
            [
                {
                    "id": "fake-local",
                    "role": "local-agent",
                    "provider": "local",
                    "runner": "fake",
                }
            ]
        )
        mission = self.load_mission_with_agent_config(config_path)
        mission.assign_issue("ISS-01", "fake-local")
        mission.approve_issue("ISS-01")
        session = mission.launch_issue("ISS-01")
        mission.run_session(session.session_id)
        mission.record_frontier_review(session.session_id, "Approved", reason="Ready.")

        result = perform_tui_action(mission, "prepare-pr", "ISS-01", gh_available=True)

        self.assertIn("gh pr create", result.create_command)
        self.assertIn("albert/mission-001/ISS-01-root", result.branch_name)
        self.assertEqual(result.merge_approved, False)

    def test_tui_prepare_pr_action_blocks_non_pr_ready_issue(self):
        mission = self.load_mission()

        with self.assertRaisesRegex(AlbertError, "ISS-01 is not PR-ready."):
            perform_tui_action(mission, "prepare-pr", "ISS-01")

    def test_ollama_runner_writes_model_generated_files_and_collects_evidence(self):
        fake_ollama = self.root / "fake_ollama_success.py"
        fake_ollama.write_text(
            "import json, sys\n"
            "prompt = sys.stdin.read()\n"
            "assert 'ISS-01' in prompt\n"
            "print(json.dumps({\n"
            "  'summary': 'Created prototype app',\n"
            "  'files': [{'path': 'prototype/app.py', 'content': 'print(\"hello prototype\")\\n'}]\n"
            "}))\n",
            encoding="utf-8",
        )
        command = f"{sys.executable} {fake_ollama}"
        config_path = self.write_agent_config(
            [
                {
                    "id": "qwen3.6-27b",
                    "role": "local-agent",
                    "provider": "ollama",
                    "runner": "ollama",
                    "model": "qwen3.6:27b",
                    "command": command,
                }
            ]
        )
        mission = self.load_mission_with_agent_config(config_path)
        mission.record_command_approval(command, "auto-allowed")
        mission.assign_issue("ISS-01", "qwen3.6-27b")
        mission.approve_issue("ISS-01")

        session = mission.launch_issue("ISS-01")
        mission.run_session(session.session_id)

        self.assertEqual(session.status, "evidence-ready")
        self.assertEqual(session.runner_exit_status, 0)
        self.assertTrue((session.worktree_path / "prototype" / "app.py").exists())
        self.assertEqual(session.evidence.changed_files, ["prototype/app.py"])
        self.assertNotIn("ollama-prompt.txt", "\n".join(session.evidence.artifact_links))
        self.assertIn("review.diff", "\n".join(session.evidence.artifact_links))
        self.assertIn("qwen3.6:27b", Path(session.artifacts["ollama_prompt"]).read_text(encoding="utf-8"))

    def test_ollama_runner_records_malformed_model_output_as_failure(self):
        fake_ollama = self.root / "fake_ollama_bad.py"
        fake_ollama.write_text("print('not json')\n", encoding="utf-8")
        command = f"{sys.executable} {fake_ollama}"
        config_path = self.write_agent_config(
            [
                {
                    "id": "qwen3.6-27b",
                    "role": "local-agent",
                    "provider": "ollama",
                    "runner": "ollama",
                    "model": "qwen3.6:27b",
                    "command": command,
                }
            ]
        )
        mission = self.load_mission_with_agent_config(config_path)
        mission.record_command_approval(command, "auto-allowed")
        mission.assign_issue("ISS-01", "qwen3.6-27b")
        mission.approve_issue("ISS-01")

        session = mission.launch_issue("ISS-01")
        mission.run_session(session.session_id)

        self.assertEqual(session.status, "failed")
        self.assertEqual(session.evidence_valid, False)
        self.assertIn("Malformed Ollama output", session.evidence.known_risks)
        self.assertNotIn("ollama-output.txt", "\n".join(session.evidence.artifact_links))

    def test_failed_ollama_operation_is_reported_without_mutating_issue_lifecycle(self):
        fake_ollama = self.root / "fake_ollama_bad.py"
        fake_ollama.write_text("print('not json')\n", encoding="utf-8")
        command = f"{sys.executable} {fake_ollama}"
        config_path = self.write_agent_config(
            [
                {
                    "id": "qwen3.6-27b",
                    "role": "local-agent",
                    "provider": "ollama",
                    "runner": "ollama",
                    "model": "qwen3.6:27b",
                    "command": command,
                }
            ]
        )
        mission = self.load_mission_with_agent_config(config_path)
        mission.record_command_approval(command, "auto-allowed")
        mission.assign_issue("ISS-01", "qwen3.6-27b")
        mission.approve_issue("ISS-01")

        session = mission.launch_issue("ISS-01")
        mission.run_session(session.session_id)

        issue = mission.board_summary()["issue_slices"][0]
        self.assertEqual(session.status, "failed")
        self.assertEqual(issue["lifecycle"], "Ready")
        self.assertEqual(issue["model_assignment"]["operation_status"], "failed")
        self.assertEqual(
            issue["model_assignment"]["failure"],
            "Malformed Ollama output: model output is not JSON",
        )
        self.assertEqual(mission.issues["ISS-01"].review_state, "approved")

    def test_streaming_session_reports_provider_neutral_operation_status(self):
        config_path = self.write_agent_config(
            [
                {
                    "id": "qwen3.6-27b",
                    "role": "local-agent",
                    "provider": "ollama",
                    "runner": "ollama",
                    "model": "qwen3.6:27b",
                }
            ]
        )
        mission = self.load_mission_with_agent_config(config_path)
        mission.assign_issue("ISS-01", "qwen3.6-27b")
        mission.approve_issue("ISS-01")
        mission.sessions["session-ISS-01-1"] = LocalAgentSession(
            session_id="session-ISS-01-1",
            issue_id="ISS-01",
            assigned_agent="qwen3.6-27b",
            worktree_path=self.target_repo / ".albert-worktrees" / "ISS-01",
            task_packet={
                "agent_config": {
                    "role": "local-agent",
                    "provider": "ollama",
                    "model": "qwen3.6:27b",
                }
            },
            status="launched",
            runner_started_at="2026-06-25T10:00:00Z",
        )

        issue = mission.board_summary()["issue_slices"][0]

        self.assertEqual(issue["model_assignment"]["operation_status"], "streaming")
        self.assertEqual(issue["sessions"][0]["operation_status"], "streaming")
        self.assertEqual(issue["sessions"][0]["failure"], "")
        self.assertEqual(issue["lifecycle"], "Ready")

    def test_ollama_runner_accepts_thinking_text_and_fenced_final_json(self):
        fake_ollama = self.root / "fake_ollama_thinking.py"
        fake_ollama.write_text(
            "print('Thinking...')\n"
            "print('{\"schema\": \"example before final answer\"}')\n"
            "print('```json')\n"
            "print('{\"summary\": \"Created prototype\", \"files\": [{\"path\": \"prototype_app.py\", \"content\": \"def main():\\\\n    print(\\\\\"Albert prototype ready\\\\\")\\\\n\\\\nif __name__ == \\\\\"__main__\\\\\":\\\\n    main()\\\\n\"}]}')\n"
            "print('```')\n",
            encoding="utf-8",
        )
        command = f"{sys.executable} {fake_ollama}"
        config_path = self.write_agent_config(
            [
                {
                    "id": "qwen3.6-27b",
                    "role": "local-agent",
                    "provider": "ollama",
                    "runner": "ollama",
                    "model": "qwen3.6:27b",
                    "command": command,
                }
            ]
        )
        mission = self.load_mission_with_agent_config(config_path)
        mission.record_command_approval(command, "auto-allowed")
        mission.assign_issue("ISS-01", "qwen3.6-27b")
        mission.approve_issue("ISS-01")

        session = mission.launch_issue("ISS-01")
        mission.run_session(session.session_id)

        self.assertEqual(session.status, "evidence-ready")
        self.assertTrue((session.worktree_path / "prototype_app.py").exists())

    def test_ollama_runner_accepts_ansi_sequences_inside_json(self):
        fake_ollama = self.root / "fake_ollama_ansi.py"
        fake_ollama.write_text(
            "import sys\n"
            "sys.stdout.write('''Thinking...\\n"
            "{\\x1b[?25l\"summary\\x1b[?25h\": \"Created prototype\", "
            "\"files\": [{\"path\": \"prototype_app.py\", "
            "\"content\": \"def main():\\\\n    print('Albert prototype ready')\\\\n\"}]}\\n''')\n",
            encoding="utf-8",
        )
        command = f"{sys.executable} {fake_ollama}"
        config_path = self.write_agent_config(
            [
                {
                    "id": "qwen3.6-27b",
                    "role": "local-agent",
                    "provider": "ollama",
                    "runner": "ollama",
                    "model": "qwen3.6:27b",
                    "command": command,
                }
            ]
        )
        mission = self.load_mission_with_agent_config(config_path)
        mission.record_command_approval(command, "auto-allowed")
        mission.assign_issue("ISS-01", "qwen3.6-27b")
        mission.approve_issue("ISS-01")

        session = mission.launch_issue("ISS-01")
        mission.run_session(session.session_id)

        self.assertEqual(session.status, "evidence-ready")
        self.assertTrue((session.worktree_path / "prototype_app.py").exists())

    def test_cli_drives_review_launch_evidence_records_and_pr_flow(self):
        base_args = [
            "--target-repo",
            str(self.target_repo),
            "--tracker-dir",
            str(self.tracker),
            "--runtime-root",
            str(self.runtime),
            "--mission-id",
            "mission-001",
        ]

        self.assertEqual(self.run_cli(["approve", *base_args, "ISS-01"])[0], 0)
        self.assertEqual(self.run_cli(["launch", *base_args, "ISS-01", "--allowed-path", "src"])[0], 0)

        session_id = "session-ISS-01-1"
        self.assertEqual(
            self.run_cli(
                [
                    "evidence",
                    *base_args,
                    session_id,
                    "--expected-revision",
                    str(self.load_mission().sessions[session_id].revision),
                    "--changed-file",
                    "src/app.py",
                    "--diff-summary",
                    "Added mission board.",
                    "--command-run",
                    "python -m unittest",
                    "--test-results",
                    "All tests passed.",
                    "--known-risks",
                    "None.",
                    "--context-updates",
                    "No glossary changes.",
                    "--artifact-link",
                    "artifact://evidence/ISS-01",
                ]
            )[0],
            0,
        )
        self.assertEqual(
            self.run_cli(
                [
                    "review",
                    *base_args,
                    session_id,
                    "--expected-revision",
                    str(self.load_mission().sessions[session_id].revision),
                    "--outcome",
                    "Approved",
                    "--reason",
                    "Meets criteria.",
                ]
            )[0],
            0,
        )
        self.assertEqual(self.run_cli(["records", *base_args])[0], 0)

        exit_code, output = self.run_cli(["pr", *base_args, "ISS-01"])

        self.assertEqual(exit_code, 0)
        self.assertIn("Manual PR instructions", output)
        self.assertTrue((self.target_repo / "docs" / "missions" / "mission-001" / "README.md").exists())

    def test_cli_reports_launch_blockers_without_traceback(self):
        result = subprocess.run(
            [
                sys.executable,
                "-m",
                "albert_mvp",
                "launch",
                "--target-repo",
                str(self.target_repo),
                "--tracker-dir",
                str(self.tracker),
                "--runtime-root",
                str(self.runtime),
                "--mission-id",
                "mission-001",
                "ISS-01",
                "--allowed-path",
                "src",
            ],
            check=False,
            capture_output=True,
            text=True,
        )

        self.assertEqual(result.returncode, 1)
        self.assertIn("Error: ISS-01 must be approved before launch.", result.stderr)
        self.assertNotIn("Traceback", result.stderr)

    def test_approval_locks_contract_but_keeps_assignment_editable(self):
        mission = self.load_mission()

        mission.approve_issue("ISS-01")
        mission.assign_issue("ISS-01", "qwen-coder-local-3", notes="Prefer fast local model.")

        self.assertEqual(mission.issues["ISS-01"].assigned_agent, "qwen-coder-local-3")
        self.assertEqual(mission.issues["ISS-01"].notes, "Prefer fast local model.")
        with self.assertRaises(LockedFieldError):
            mission.update_issue_contract("ISS-01", acceptance_criteria=["Changed after approval."])

        mission.unlock_issue("ISS-01", reason="Acceptance criteria need correction.")
        mission.update_issue_contract("ISS-01", acceptance_criteria=["Changed after explicit unlock."])
        self.assertEqual(mission.issues["ISS-01"].review_state, "needs-review")

    def test_launch_requires_approved_unblocked_issue_and_creates_task_packet(self):
        mission = self.load_mission()

        with self.assertRaises(LaunchBlockedError):
            mission.launch_issue("ISS-02")

        mission.approve_issue("ISS-01")
        session = mission.launch_issue("ISS-01", allowed_paths=["src"], command_policy={"python -m unittest": "auto-allowed"})

        self.assertEqual(session.issue_id, "ISS-01")
        self.assertTrue(str(session.worktree_path).startswith(str(self.root / ".albert-worktrees")))
        self.assertEqual(session.task_packet["acceptance_criteria"], ["Mission summary is visible."])
        self.assertEqual(session.task_packet["allowed_paths"], ["src"])
        self.assertEqual(session.cleanup_eligible, False)

    def test_launch_bounds_the_git_repository_probe(self):
        fake_bin = self.root / "fake-git-bin"
        fake_bin.mkdir()
        fake_git = fake_bin / "git"
        fake_git.write_text(
            "#!/usr/bin/python3\n"
            "import time\n"
            "time.sleep(0.2)\n",
            encoding="utf-8",
        )
        fake_git.chmod(0o755)
        mission = self.load_mission()
        mission.approve_issue("ISS-01")

        with (
            patch.dict(os.environ, {"PATH": str(fake_bin)}),
            patch("albert_mvp.core._GIT_SNAPSHOT_TIMEOUT_SECONDS", 0.05),
            self.assertRaisesRegex(AlbertError, "Git repository.*timed out"),
        ):
            mission.launch_issue("ISS-01")

        self.assertEqual(mission.sessions, {})

    def test_launch_rejects_unexpected_git_repository_probe_failure(self):
        self.initialize_target_git_repo({"src/app.py": "VALUE = 1\n"})
        fake_bin = self.root / "failing-git-bin"
        fake_bin.mkdir()
        fake_git = fake_bin / "git"
        fake_git.write_text(
            "#!/usr/bin/python3\n"
            "import sys\n"
            "sys.stderr.write('fatal: injected repository probe failure\\n')\n"
            "raise SystemExit(42)\n",
            encoding="utf-8",
        )
        fake_git.chmod(0o755)
        mission = self.load_mission()
        mission.approve_issue("ISS-01")

        with (
            patch.dict(os.environ, {"PATH": str(fake_bin)}),
            self.assertRaisesRegex(
                AlbertError,
                "target Git repository.*injected repository probe failure",
            ),
        ):
            mission.launch_issue("ISS-01")

        self.assertEqual(mission.sessions, {})

    def test_launch_accepts_only_explicit_non_git_directory_probe(self):
        fake_bin = self.root / "non-repository-git-bin"
        fake_bin.mkdir()
        fake_git = fake_bin / "git"
        fake_git.write_text(
            "#!/usr/bin/python3\n"
            "import sys\n"
            "sys.stderr.write("
            "'fatal: not a git repository (or any of the parent directories): .git\\n'"
            ")\n"
            "raise SystemExit(128)\n",
            encoding="utf-8",
        )
        fake_git.chmod(0o755)
        mission = self.load_mission()
        mission.approve_issue("ISS-01")

        with patch.dict(os.environ, {"PATH": str(fake_bin)}):
            session = mission.launch_issue("ISS-01")

        self.assertEqual(session.status, "queued")

    def test_launch_rejects_non_repository_response_when_git_metadata_exists(self):
        (self.target_repo / ".git").mkdir()
        fake_bin = self.root / "corrupt-repository-git-bin"
        fake_bin.mkdir()
        fake_git = fake_bin / "git"
        fake_git.write_text(
            "#!/usr/bin/python3\n"
            "import sys\n"
            "sys.stderr.write("
            "'fatal: not a git repository (or any of the parent directories): .git\\n'"
            ")\n"
            "raise SystemExit(128)\n",
            encoding="utf-8",
        )
        fake_git.chmod(0o755)
        mission = self.load_mission()
        mission.approve_issue("ISS-01")

        with (
            patch.dict(os.environ, {"PATH": str(fake_bin)}),
            self.assertRaisesRegex(
                AlbertError,
                "Git metadata exists.*not a git repository",
            ),
        ):
            mission.launch_issue("ISS-01")

        self.assertEqual(mission.sessions, {})

    def test_run_bounds_git_worktree_creation_output(self):
        self.initialize_target_git_repo({"src/app.py": "VALUE = 1\n"})
        config_path = self.write_agent_config(
            [
                {
                    "id": "fake-local",
                    "role": "local-agent",
                    "provider": "local",
                    "runner": "fake",
                    "routing": "worker",
                }
            ]
        )
        mission = self.load_mission_with_agent_config(config_path)
        mission.assign_issue("ISS-01", "fake-local")
        mission.approve_issue("ISS-01")
        session = mission.launch_issue("ISS-01")
        real_git = shutil.which("git")
        self.assertIsNotNone(real_git)
        fake_bin = self.root / "bounded-git-bin"
        fake_bin.mkdir()
        fake_git = fake_bin / "git"
        fake_git.write_text(
            "#!/usr/bin/python3\n"
            "import os, sys\n"
            f"REAL_GIT = {real_git!r}\n"
            "if 'worktree' in sys.argv[1:] and 'add' in sys.argv[1:]:\n"
            "    sys.stdout.write('x' * 100_000)\n"
            "    sys.stdout.flush()\n"
            "    raise SystemExit(9)\n"
            "os.execv(REAL_GIT, [REAL_GIT, *sys.argv[1:]])\n",
            encoding="utf-8",
        )
        fake_git.chmod(0o755)

        with (
            patch.dict(
                os.environ,
                {"PATH": f"{fake_bin}{os.pathsep}{os.environ.get('PATH', '')}"},
            ),
            patch("albert_mvp.core._GIT_COMMAND_OUTPUT_BYTES_LIMIT", 1_024),
            self.assertRaisesRegex(AlbertError, "Git worktree.*output exceeded"),
        ):
            mission.run_session(session.session_id)

        persisted = self.load_mission_with_agent_config(config_path).sessions[
            session.session_id
        ]
        self.assertEqual(persisted.status, "failed")

    def test_run_rejects_unexpected_optional_git_listing_failure(self):
        self.initialize_target_git_repo({"src/app.py": "VALUE = 1\n"})
        config_path = self.write_agent_config(
            [
                {
                    "id": "fake-local",
                    "role": "local-agent",
                    "provider": "local",
                    "runner": "fake",
                    "routing": "worker",
                }
            ]
        )
        mission = self.load_mission_with_agent_config(config_path)
        mission.assign_issue("ISS-01", "fake-local")
        mission.approve_issue("ISS-01")
        session = mission.launch_issue("ISS-01")
        real_git = shutil.which("git")
        self.assertIsNotNone(real_git)
        fake_bin = self.root / "optional-git-bin"
        fake_bin.mkdir()
        fake_git = fake_bin / "git"
        fake_git.write_text(
            "#!/usr/bin/python3\n"
            "import os, sys\n"
            f"REAL_GIT = {real_git!r}\n"
            "arguments = sys.argv[1:]\n"
            "if 'diff' in arguments and '--name-only' in arguments:\n"
            "    sys.stderr.write('fatal: injected optional listing failure\\n')\n"
            "    raise SystemExit(42)\n"
            "os.execv(REAL_GIT, [REAL_GIT, *arguments])\n",
            encoding="utf-8",
        )
        fake_git.chmod(0o755)

        with (
            patch.dict(
                os.environ,
                {"PATH": f"{fake_bin}{os.pathsep}{os.environ.get('PATH', '')}"},
            ),
            self.assertRaisesRegex(
                AlbertError,
                "worktree changed-file listing.*injected optional listing failure",
            ),
        ):
            mission.run_session(session.session_id)

        persisted = self.load_mission_with_agent_config(config_path).sessions[
            session.session_id
        ]
        self.assertEqual(persisted.status, "failed")

    def test_run_uses_a_real_git_worktree_with_current_repository_contents(self):
        self.initialize_target_git_repo(
            {
                "AGENTS.md": "Use repository-native tests.\n",
                "src/existing.py": "def existing():\n    return 41\n",
            }
        )
        (self.target_repo / "src" / "existing.py").write_text(
            "def existing():\n    return 99  # current dirty source\n",
            encoding="utf-8",
        )
        (self.target_repo / "src" / "untracked_helper.py").write_text(
            "HELPER = 'current untracked source'\n",
            encoding="utf-8",
        )
        (self.target_repo / ".env").write_text("TOKEN=must-not-copy\n", encoding="utf-8")
        (self.target_repo / "UNTRACKED.bin").write_bytes(b"\x00must-not-copy")
        source_head = subprocess.run(
            ["git", "-C", str(self.target_repo), "rev-parse", "HEAD"],
            check=True,
            capture_output=True,
            text=True,
        ).stdout
        source_status = subprocess.run(
            ["git", "-C", str(self.target_repo), "status", "--porcelain=v1", "-z"],
            check=True,
            capture_output=True,
        ).stdout
        config_path = self.write_agent_config(
            [
                {
                    "id": "fake-local",
                    "role": "local-agent",
                    "provider": "local",
                    "runner": "fake",
                }
            ]
        )
        mission = self.load_mission_with_agent_config(config_path)
        mission.assign_issue("ISS-01", "fake-local")
        mission.approve_issue("ISS-01")

        session = mission.launch_issue("ISS-01")

        self.assertEqual(session.status, "queued")
        self.assertFalse(session.worktree_path.exists())
        mission.run_session(session.session_id)

        self.assertEqual(
            (session.worktree_path / "src" / "existing.py").read_text(encoding="utf-8"),
            "def existing():\n    return 99  # current dirty source\n",
        )
        self.assertEqual(
            (session.worktree_path / "src" / "untracked_helper.py").read_text(
                encoding="utf-8"
            ),
            "HELPER = 'current untracked source'\n",
        )
        self.assertTrue((session.worktree_path / ".git").exists())
        self.assertFalse((session.worktree_path / ".env").exists())
        self.assertFalse((session.worktree_path / "UNTRACKED.bin").exists())
        self.assertEqual(session.repository_snapshot["tracked_diff_applied"], True)
        self.assertEqual(
            session.repository_snapshot["untracked_files_copied"],
            ["src/untracked_helper.py"],
        )
        self.assertGreaterEqual(
            session.repository_snapshot["untracked_files_skipped_count"],
            2,
        )
        self.assertEqual(session.evidence.changed_files, ["FAKE_AGENT_RESULT.md"])
        self.assertEqual(session.worktree_path.name, session.session_id)
        inside = subprocess.run(
            ["git", "-C", str(session.worktree_path), "rev-parse", "--is-inside-work-tree"],
            check=True,
            capture_output=True,
            text=True,
        )
        self.assertEqual(inside.stdout.strip(), "true")
        self.assertEqual(
            subprocess.run(
                ["git", "-C", str(self.target_repo), "rev-parse", "HEAD"],
                check=True,
                capture_output=True,
                text=True,
            ).stdout,
            source_head,
        )
        self.assertEqual(
            subprocess.run(
                [
                    "git",
                    "-C",
                    str(self.target_repo),
                    "status",
                    "--porcelain=v1",
                    "-z",
                ],
                check=True,
                capture_output=True,
            ).stdout,
            source_status,
        )

    def test_non_git_run_copies_a_bounded_safe_source_baseline(self):
        (self.target_repo / "src").mkdir()
        (self.target_repo / "src" / "app.py").write_text(
            "VALUE = 'current directory source'\n",
            encoding="utf-8",
        )
        (self.target_repo / "README.md").write_text(
            "# Current project\n",
            encoding="utf-8",
        )
        (self.target_repo / ".env").write_text(
            "TOKEN=must-not-copy\n",
            encoding="utf-8",
        )
        (self.target_repo / "asset.bin").write_bytes(b"\x00not-source")
        config_path = self.write_agent_config(
            [
                {
                    "id": "fake-local",
                    "role": "local-agent",
                    "provider": "local",
                    "runner": "fake",
                }
            ]
        )
        mission = self.load_mission_with_agent_config(config_path)
        mission.assign_issue("ISS-01", "fake-local")
        mission.approve_issue("ISS-01")

        session = mission.launch_issue("ISS-01")
        self.assertFalse(session.worktree_path.exists())
        mission.run_session(session.session_id)

        self.assertEqual(
            (session.worktree_path / "src" / "app.py").read_text(
                encoding="utf-8"
            ),
            "VALUE = 'current directory source'\n",
        )
        self.assertTrue((session.worktree_path / "README.md").exists())
        self.assertFalse((session.worktree_path / ".env").exists())
        self.assertFalse((session.worktree_path / "asset.bin").exists())
        self.assertEqual(session.repository_snapshot["kind"], "directory")
        self.assertEqual(
            session.repository_snapshot["source_files_copied"],
            ["README.md", "src/app.py"],
        )
        self.assertEqual(session.evidence.changed_files, ["FAKE_AGENT_RESULT.md"])

    def test_non_git_source_scan_counts_directory_entries_toward_its_bound(self):
        for directory_name in ("00-empty", "01-empty", "02-empty"):
            (self.target_repo / directory_name).mkdir()
        late_source = self.target_repo / "99-late" / "app.py"
        late_source.parent.mkdir()
        late_source.write_text("MUST_NOT_BE_SCANNED = True\n", encoding="utf-8")
        config_path = self.write_agent_config(
            [
                {
                    "id": "fake-local",
                    "role": "local-agent",
                    "provider": "local",
                    "runner": "fake",
                }
            ]
        )
        mission = self.load_mission_with_agent_config(config_path)
        mission.assign_issue("ISS-01", "fake-local")
        mission.approve_issue("ISS-01")
        session = mission.launch_issue("ISS-01")

        with patch("albert_mvp.core._DIRECTORY_SOURCE_SCAN_LIMIT", 3):
            mission.run_session(session.session_id)

        self.assertFalse((session.worktree_path / "99-late" / "app.py").exists())
        self.assertEqual(session.repository_snapshot["source_files_copied"], [])
        self.assertGreaterEqual(
            session.repository_snapshot["source_files_skipped_count"],
            1,
        )

    def test_git_snapshot_fails_actionably_when_parent_diff_exceeds_capture_limit(self):
        self.initialize_target_git_repo({"src/app.py": "VALUE = 'original'\n"})
        (self.target_repo / "src" / "app.py").write_text(
            "VALUE = '" + ("changed" * 1_000) + "'\n",
            encoding="utf-8",
        )
        config_path = self.write_agent_config(
            [
                {
                    "id": "fake-local",
                    "role": "local-agent",
                    "provider": "local",
                    "runner": "fake",
                }
            ]
        )
        mission = self.load_mission_with_agent_config(config_path)
        mission.assign_issue("ISS-01", "fake-local")
        mission.approve_issue("ISS-01")
        session = mission.launch_issue("ISS-01")

        with (
            patch("albert_mvp.core._GIT_SNAPSHOT_BYTES_LIMIT", 512),
            self.assertRaisesRegex(
                AlbertError,
                "snapshot exceeds the 512-byte capture limit",
            ),
        ):
            mission.run_session(session.session_id)

        persisted = self.load_mission_with_agent_config(config_path).sessions[
            session.session_id
        ]
        self.assertEqual(persisted.status, "failed")
        self.assertIn(
            "commit, stash, or split",
            persisted.task_packet["runner_failure"],
        )

    def test_repository_file_listing_fails_actionably_when_path_output_exceeds_limit(self):
        tracked_files = {
            f"src/module_{index:03d}.py": f"VALUE = {index}\n"
            for index in range(40)
        }
        self.initialize_target_git_repo(tracked_files)
        fake_ollama = self.root / "fake_ollama_path_limit.py"
        fake_ollama.write_text(
            "import json\n"
            "print(json.dumps({'summary': 'unused', 'files': []}))\n",
            encoding="utf-8",
        )
        command = f"{sys.executable} {fake_ollama}"
        config_path = self.write_agent_config(
            [
                {
                    "id": "path-limit-local",
                    "role": "local-agent",
                    "provider": "ollama",
                    "runner": "ollama",
                    "model": "path-limit:test",
                    "command": command,
                }
            ]
        )
        mission = self.load_mission_with_agent_config(config_path)
        mission.record_command_approval(command, "auto-allowed")
        mission.assign_issue("ISS-01", "path-limit-local")
        mission.approve_issue("ISS-01")
        session = mission.launch_issue("ISS-01")

        with (
            patch("albert_mvp.core._GIT_PATH_OUTPUT_BYTES_LIMIT", 256),
            self.assertRaisesRegex(
                AlbertError,
                "repository file listing exceeds the 256-byte capture limit",
            ),
        ):
            mission.run_session(session.session_id)

        persisted = self.load_mission_with_agent_config(config_path).sessions[
            session.session_id
        ]
        self.assertEqual(persisted.status, "failed")
        self.assertIn(
            "narrow the workspace",
            persisted.task_packet["runner_failure"],
        )

    def test_dirty_parent_baseline_fingerprints_only_a_bounded_file_sample(self):
        original = "A" * (512 * 1024)
        self.initialize_target_git_repo({"src/large.py": original})
        (self.target_repo / "src" / "large.py").write_text(
            original[:-1] + "B",
            encoding="utf-8",
        )
        config_path = self.write_agent_config(
            [
                {
                    "id": "fake-local",
                    "role": "local-agent",
                    "provider": "local",
                    "runner": "fake",
                }
            ]
        )
        mission = self.load_mission_with_agent_config(config_path)
        mission.assign_issue("ISS-01", "fake-local")
        mission.approve_issue("ISS-01")
        session = mission.launch_issue("ISS-01")
        original_open = Path.open

        class FingerprintReadGuard:
            def __init__(self, handle):
                self._handle = handle
                self._sampled = 0

            def read(self, size=-1):
                if size is None or size < 0:
                    raise AssertionError("baseline reads must always be bounded")
                payload = self._handle.read(size)
                if size <= 64 * 1024:
                    self._sampled += len(payload)
                    if self._sampled > 128 * 1024:
                        raise AssertionError(
                            "fingerprint read more than the 128 KiB sample budget"
                        )
                return payload

            def __enter__(self):
                self._handle.__enter__()
                return self

            def __exit__(self, *args):
                return self._handle.__exit__(*args)

            def __getattr__(self, name):
                return getattr(self._handle, name)

        def guarded_open(path, mode="r", *args, **kwargs):
            handle = original_open(path, mode, *args, **kwargs)
            if path.name == "large.py" and "b" in mode and "r" in mode:
                return FingerprintReadGuard(handle)
            return handle

        with patch.object(Path, "open", guarded_open):
            mission.run_session(session.session_id)

        self.assertEqual(session.status, "evidence-ready")
        self.assertEqual(session.evidence_valid, True)

    def test_git_subdirectory_selection_fails_before_queue_with_root_guidance(self):
        self.initialize_target_git_repo(
            {
                "packages/app/main.py": "VALUE = 1\n",
            }
        )
        selected_subdirectory = self.target_repo / "packages" / "app"
        mission = AlbertMission(
            target_repo=selected_subdirectory,
            tracker_dir=self.tracker,
            runtime_root=self.runtime,
            mission_id="mission-001",
        ).load()
        mission.approve_issue("ISS-01")

        with self.assertRaisesRegex(
            LaunchBlockedError,
            "Git repository subdirectory.*Select the repository root",
        ):
            mission.launch_issue("ISS-01")

        self.assertEqual(mission.sessions, {})

    def test_two_sessions_for_the_same_issue_have_isolated_worktrees(self):
        self.initialize_target_git_repo({"src/existing.py": "VALUE = 1\n"})
        config_path = self.write_agent_config(
            [
                {
                    "id": "fake-local",
                    "role": "local-agent",
                    "provider": "local",
                    "runner": "fake",
                }
            ]
        )
        mission = self.load_mission_with_agent_config(config_path)
        mission.assign_issue("ISS-01", "fake-local")
        mission.approve_issue("ISS-01")

        first = mission.launch_issue("ISS-01")
        self.assertFalse(first.worktree_path.exists())
        mission.run_session(first.session_id)
        (first.worktree_path / "session-one-only.txt").write_text("first\n", encoding="utf-8")
        second = mission.launch_issue("ISS-01")
        self.assertFalse(second.worktree_path.exists())
        mission.run_session(second.session_id)

        self.assertNotEqual(first.worktree_path, second.worktree_path)
        self.assertEqual(first.worktree_path.name, first.session_id)
        self.assertEqual(second.worktree_path.name, second.session_id)
        self.assertTrue((first.worktree_path / "session-one-only.txt").exists())
        self.assertFalse((second.worktree_path / "session-one-only.txt").exists())
        self.assertTrue((second.worktree_path / "src" / "existing.py").exists())

    def test_ollama_prompt_includes_bounded_repository_instructions_tree_and_sources(self):
        self.initialize_target_git_repo(
            {
                "AGENTS.md": "Always run the focused repository tests.\n",
                "CONTEXT.md": "The domain name is Operator Workspace.\n",
                "src/existing.py": "def current_behavior():\n    return 'existing-code'\n",
                "src/huge.py": "HEAD\n" + ("x" * 60_000) + "\nTAIL_SHOULD_NOT_REACH_MODEL\n",
            }
        )
        (self.target_repo / "src" / "existing.py").write_text(
            "def current_behavior():\n    return 'current-dirty-code'\n",
            encoding="utf-8",
        )
        (self.target_repo / "src" / "untracked_context.py").write_text(
            "UNTRACKED_CONTEXT = 'available to the local agent'\n",
            encoding="utf-8",
        )
        fake_ollama = self.root / "fake_ollama_context.py"
        fake_ollama.write_text(
            "import json\n"
            "print(json.dumps({'summary': 'used repository context', "
            "'files': [{'path': 'src/generated.py', 'content': 'VALUE = 42\\n'}]}))\n",
            encoding="utf-8",
        )
        command = f"{sys.executable} {fake_ollama}"
        config_path = self.write_agent_config(
            [
                {
                    "id": "repo-aware-local",
                    "role": "local-agent",
                    "provider": "ollama",
                    "runner": "ollama",
                    "model": "repo-aware:test",
                    "command": command,
                }
            ]
        )
        mission = self.load_mission_with_agent_config(config_path)
        mission.record_command_approval(command, "auto-allowed")
        mission.assign_issue("ISS-01", "repo-aware-local")
        mission.approve_issue("ISS-01")

        session = mission.launch_issue("ISS-01", allowed_paths=["src"])
        mission.run_session(session.session_id)
        prompt = Path(session.artifacts["ollama_prompt"]).read_text(encoding="utf-8")

        self.assertIn("Repository context (bounded)", prompt)
        self.assertIn("Tracked file tree", prompt)
        self.assertIn("AGENTS.md", prompt)
        self.assertIn("Always run the focused repository tests.", prompt)
        self.assertIn("CONTEXT.md", prompt)
        self.assertIn("The domain name is Operator Workspace.", prompt)
        self.assertIn("src/existing.py", prompt)
        self.assertIn("return 'current-dirty-code'", prompt)
        self.assertIn("src/untracked_context.py", prompt)
        self.assertIn("available to the local agent", prompt)
        self.assertNotIn("TAIL_SHOULD_NOT_REACH_MODEL", prompt)
        self.assertLess(len(prompt), 40_000)
        self.assertEqual(session.evidence.changed_files, ["src/generated.py"])

    def test_ollama_prompt_resolves_preserved_use_invocation_to_bounded_skill_instructions(self):
        skill_dir = self.target_repo / ".agents" / "skills" / "focused-skill"
        skill_dir.mkdir(parents=True)
        skill_path = skill_dir / "SKILL.md"
        skill_path.write_text(
            "---\n"
            "name: focused-skill\n"
            "description: Apply the focused repository workflow.\n"
            "---\n"
            "FOLLOW_THIS_SKILL_INSTRUCTION before editing.\n"
            + ("bounded guidance\n" * 1_000)
            + "TAIL_MUST_NOT_REACH_MODEL\n",
            encoding="utf-8",
        )
        fake_ollama = self.root / "fake_ollama_skill.py"
        fake_ollama.write_text(
            "import json\n"
            "print(json.dumps({'summary': 'used selected skill', "
            "'files': [{'path': 'generated.py', 'content': 'VALUE = 7\\n'}]}))\n",
            encoding="utf-8",
        )
        command = f"{sys.executable} {fake_ollama}"
        config_path = self.write_agent_config(
            [
                {
                    "id": "skill-aware-local",
                    "role": "local-agent",
                    "provider": "ollama",
                    "runner": "ollama",
                    "model": "skill-aware:test",
                    "command": command,
                }
            ]
        )
        mission = self.load_mission_with_agent_config(config_path)
        mission.record_command_approval(command, "auto-allowed")
        mission.assign_issue("ISS-01", "skill-aware-local")
        mission.update_issue_contract(
            "ISS-01",
            acceptance_criteria=[
                "/use focused-skill implement the requested repository change"
            ],
        )
        mission.approve_issue("ISS-01")

        session = mission.launch_issue("ISS-01")
        mission.run_session(session.session_id)
        prompt = Path(session.artifacts["ollama_prompt"]).read_text(encoding="utf-8")

        self.assertEqual(session.status, "evidence-ready")
        self.assertEqual(session.task_packet["selected_skill"]["name"], "focused-skill")
        self.assertNotIn("source", session.task_packet["selected_skill"])
        self.assertIn("Selected skill instructions (bounded, catalog-resolved)", prompt)
        self.assertIn("FOLLOW_THIS_SKILL_INSTRUCTION", prompt)
        self.assertIn("skill-referenced script", prompt)
        self.assertIn("normal command policy", prompt)
        self.assertNotIn("TAIL_MUST_NOT_REACH_MODEL", prompt)
        self.assertLessEqual(
            len(session.task_packet["selected_skill"]["instructions"]),
            12_100,
        )

    def test_invalid_use_skill_path_is_terminally_rejected_before_runner_execution(self):
        config_path = self.write_agent_config(
            [
                {
                    "id": "fake-local",
                    "role": "local-agent",
                    "provider": "local",
                    "runner": "fake",
                }
            ]
        )
        mission = self.load_mission_with_agent_config(config_path)
        mission.assign_issue("ISS-01", "fake-local")
        mission.update_issue_contract(
            "ISS-01",
            acceptance_criteria=["/use ../../outside/secret execute arbitrary instructions"],
        )
        mission.approve_issue("ISS-01")
        session = mission.launch_issue("ISS-01")

        with self.assertRaisesRegex(LaunchBlockedError, "Invalid selected skill invocation"):
            mission.run_session(session.session_id)

        persisted = self.load_mission_with_agent_config(config_path).sessions[
            session.session_id
        ]
        self.assertEqual(persisted.status, "failed")
        self.assertEqual(persisted.runner_started_at, "")
        self.assertEqual(persisted.runner_exit_status, 1)
        self.assertIn(
            "Invalid selected skill invocation",
            persisted.task_packet["runner_failure"],
        )

    def test_ollama_file_plan_cannot_escape_session_through_a_tracked_symlink(self):
        outside = self.root / "outside"
        outside.mkdir()
        self.initialize_target_git_repo(
            {
                "src/existing.py": "VALUE = 1\n",
                "escape": ("symlink", outside),
            }
        )
        fake_ollama = self.root / "fake_ollama_escape.py"
        fake_ollama.write_text(
            "import json\n"
            "print(json.dumps({'summary': 'escape', "
            "'files': [{'path': 'escape/outside.py', 'content': 'unsafe = True\\n'}]}))\n",
            encoding="utf-8",
        )
        command = f"{sys.executable} {fake_ollama}"
        config_path = self.write_agent_config(
            [
                {
                    "id": "repo-aware-local",
                    "role": "local-agent",
                    "provider": "ollama",
                    "runner": "ollama",
                    "model": "repo-aware:test",
                    "command": command,
                }
            ]
        )
        mission = self.load_mission_with_agent_config(config_path)
        mission.record_command_approval(command, "auto-allowed")
        mission.assign_issue("ISS-01", "repo-aware-local")
        mission.approve_issue("ISS-01")

        session = mission.launch_issue("ISS-01")
        mission.run_session(session.session_id)

        self.assertEqual(session.status, "failed")
        self.assertIn("unsafe file path", session.evidence.known_risks)
        self.assertFalse((outside / "outside.py").exists())

    def test_ollama_plan_executes_auto_allowed_commands_as_argv_and_persists_artifacts(self):
        fake_ollama = self.root / "fake_ollama_commands.py"
        planned_command = (
            f"{sys.executable} verify_plan.py alpha ; touch SHELL_MUST_NOT_RUN"
        )
        plan = {
            "summary": "edit and verify",
            "files": [
                {
                    "path": "verify_plan.py",
                    "content": (
                        "import pathlib, sys\n"
                        "pathlib.Path('planned-command-ran.txt').write_text('|'.join(sys.argv[1:]))\n"
                        "print('planned stdout', *sys.argv[1:])\n"
                        "print('planned stderr', file=sys.stderr)\n"
                    ),
                }
            ],
            "commands": [planned_command],
        }
        fake_ollama.write_text(
            "import json\n" f"print(json.dumps({plan!r}))\n",
            encoding="utf-8",
        )
        runner_command = f"{sys.executable} {fake_ollama}"
        config_path = self.write_agent_config(
            [
                {
                    "id": "command-planning-local",
                    "role": "local-agent",
                    "provider": "ollama",
                    "runner": "ollama",
                    "model": "command-planning:test",
                    "command": runner_command,
                }
            ]
        )
        mission = self.load_mission_with_agent_config(config_path)
        mission.record_command_approval(runner_command, "auto-allowed")
        mission.record_command_approval(planned_command, "auto-allowed")
        mission.assign_issue("ISS-01", "command-planning-local")
        mission.approve_issue("ISS-01")

        session = mission.launch_issue("ISS-01")
        mission.run_session(session.session_id)

        self.assertEqual(session.status, "evidence-ready")
        self.assertEqual(session.runner_exit_status, 0)
        self.assertEqual(
            (session.worktree_path / "planned-command-ran.txt").read_text(
                encoding="utf-8"
            ),
            "alpha|;|touch|SHELL_MUST_NOT_RUN",
        )
        self.assertFalse((session.worktree_path / "SHELL_MUST_NOT_RUN").exists())
        self.assertEqual(
            session.evidence.commands_run,
            [runner_command, planned_command],
        )
        self.assertIn("Planned command passed", session.evidence.test_results)
        command_result_path = Path(session.artifacts["planned_command_01_result"])
        command_result = json.loads(command_result_path.read_text(encoding="utf-8"))
        self.assertEqual(
            command_result["argv"],
            [
                sys.executable,
                "verify_plan.py",
                "alpha",
                ";",
                "touch",
                "SHELL_MUST_NOT_RUN",
            ],
        )
        self.assertEqual(command_result["policy"], "auto-allowed")
        self.assertEqual(command_result["shell"], False)
        self.assertEqual(command_result["outcome"], "passed")
        self.assertIn(
            "planned stdout",
            Path(session.artifacts["planned_command_01_stdout"]).read_text(
                encoding="utf-8"
            ),
        )
        self.assertIn(
            "planned stderr",
            Path(session.artifacts["planned_command_01_stderr"]).read_text(
                encoding="utf-8"
            ),
        )

    def test_ollama_agent_repairs_a_failed_command_with_bounded_feedback_iteration(self):
        fake_ollama = self.root / "fake_ollama_iterative.py"
        planned_command = "python3 -m unittest -v test_generated.py"
        fake_ollama.write_text(
            "import json, sys\n"
            "prompt = sys.stdin.read()\n"
            "recovered = 'Planned command failed' in prompt\n"
            "value = 42 if recovered else 0\n"
            "plan = {\n"
            "  'summary': 'repair after test feedback' if recovered else 'first attempt',\n"
            "  'files': [\n"
            "    {'path': 'generated.py', 'content': f'VALUE = {value}\\n'},\n"
            "    {'path': 'test_generated.py', 'content': "
            "'import unittest\\nfrom generated import VALUE\\nclass GeneratedTest(unittest.TestCase):\\n    def test_value(self): self.assertEqual(VALUE, 42)\\n'},\n"
            "  ],\n"
            f"  'commands': [{planned_command!r}],\n"
            "}\n"
            "print(json.dumps(plan))\n",
            encoding="utf-8",
        )
        runner_command = f"{sys.executable} {fake_ollama}"
        config_path = self.write_agent_config(
            [
                {
                    "id": "iterative-local",
                    "role": "local-agent",
                    "provider": "ollama",
                    "runner": "ollama",
                    "model": "iterative:test",
                    "command": runner_command,
                    "routing": "worker",
                }
            ]
        )
        mission = self.load_mission_with_agent_config(config_path)
        mission.record_command_approval(runner_command, "auto-allowed")
        mission.record_command_approval(planned_command, "auto-allowed")
        mission.assign_issue("ISS-01", "iterative-local")
        mission.approve_issue("ISS-01")

        session = mission.launch_issue("ISS-01")
        mission.run_session(session.session_id)

        result = json.loads(
            Path(session.artifacts["result"]).read_text(encoding="utf-8")
        )
        self.assertEqual(session.status, "evidence-ready")
        self.assertEqual(session.runner_exit_status, 0)
        self.assertEqual(len(result["iterations"]), 2)
        self.assertEqual(result["planned_commands"][0]["outcome"], "failed")
        self.assertEqual(result["planned_commands"][1]["outcome"], "passed")
        self.assertEqual(result["planned_commands"][1]["iteration"], 2)
        self.assertEqual(
            (session.worktree_path / "generated.py").read_text(encoding="utf-8"),
            "VALUE = 42\n",
        )
        self.assertIn("Recovered after 1 failed agent iteration", session.evidence.known_risks)
        self.assertIn("planned_command_02_result", session.artifacts)

    def test_ollama_plan_rejects_non_auto_allowed_command_before_edits_or_execution(self):
        fake_ollama = self.root / "fake_ollama_rejected_command.py"
        rejected_command = "rm -rf ."
        plan = {
            "summary": "unsafe verification request",
            "files": [
                {
                    "path": "MUST_NOT_BE_WRITTEN.txt",
                    "content": "the command plan should be rejected first\n",
                }
            ],
            "commands": [rejected_command],
        }
        fake_ollama.write_text(
            "import json\n" f"print(json.dumps({plan!r}))\n",
            encoding="utf-8",
        )
        runner_command = f"{sys.executable} {fake_ollama}"
        config_path = self.write_agent_config(
            [
                {
                    "id": "command-planning-local",
                    "role": "local-agent",
                    "provider": "ollama",
                    "runner": "ollama",
                    "model": "command-planning:test",
                    "command": runner_command,
                }
            ]
        )
        mission = self.load_mission_with_agent_config(config_path)
        mission.record_command_approval(runner_command, "auto-allowed")
        mission.assign_issue("ISS-01", "command-planning-local")
        mission.approve_issue("ISS-01")

        session = mission.launch_issue("ISS-01")
        mission.run_session(session.session_id)

        self.assertEqual(session.status, "failed")
        self.assertEqual(session.runner_exit_status, 1)
        self.assertFalse(
            (session.worktree_path / "MUST_NOT_BE_WRITTEN.txt").exists()
        )
        self.assertEqual(session.evidence.commands_run, [runner_command])
        self.assertIn(
            "Planned command rejected (human-required)",
            session.evidence.test_results,
        )
        self.assertIn("auto-allowed is required", session.evidence.known_risks)
        command_result = json.loads(
            Path(session.artifacts["planned_command_01_result"]).read_text(
                encoding="utf-8"
            )
        )
        self.assertEqual(command_result["executed"], False)
        self.assertEqual(command_result["policy"], "human-required")
        self.assertEqual(command_result["outcome"], "rejected")
        self.assertTrue(
            Path(session.artifacts["planned_command_01_stdout"]).exists()
        )
        self.assertIn(
            "auto-allowed is required",
            Path(session.artifacts["planned_command_01_stderr"]).read_text(
                encoding="utf-8"
            ),
        )

    def test_ollama_plan_command_timeout_is_bounded_and_persisted(self):
        fake_ollama = self.root / "fake_ollama_timeout_command.py"
        planned_command = (
            f'{sys.executable} -c "import time; time.sleep(5)"'
        )
        plan = {
            "summary": "bounded verification request",
            "files": [{"path": "generated.txt", "content": "generated\n"}],
            "commands": [planned_command],
        }
        fake_ollama.write_text(
            "import json\n" f"print(json.dumps({plan!r}))\n",
            encoding="utf-8",
        )
        runner_command = f"{sys.executable} {fake_ollama}"
        config_path = self.write_agent_config(
            [
                {
                    "id": "command-planning-local",
                    "role": "local-agent",
                    "provider": "ollama",
                    "runner": "ollama",
                    "model": "command-planning:test",
                    "command": runner_command,
                }
            ]
        )
        mission = self.load_mission_with_agent_config(config_path)
        mission.record_command_approval(runner_command, "auto-allowed")
        mission.record_command_approval(planned_command, "auto-allowed")
        mission.assign_issue("ISS-01", "command-planning-local")
        mission.approve_issue("ISS-01")
        session = mission.launch_issue("ISS-01")

        with patch("albert_mvp.core._MODEL_COMMAND_TIMEOUT_SECONDS", 0.05):
            mission.run_session(session.session_id)

        self.assertEqual(session.status, "failed")
        self.assertEqual(session.runner_exit_status, 124)
        self.assertIn("timed out after 0.05 seconds", session.evidence.test_results)
        command_result = json.loads(
            Path(session.artifacts["planned_command_01_result"]).read_text(
                encoding="utf-8"
            )
        )
        self.assertEqual(command_result["outcome"], "timed-out")
        self.assertEqual(command_result["exit_status"], 124)
        self.assertEqual(command_result["timeout_seconds"], 0.05)

    def test_headless_run_creates_governed_session_and_evidence_package(self):
        config_path = self.write_agent_config(
            [
                {
                    "id": "fake-local",
                    "role": "local-agent",
                    "provider": "test",
                    "runner": "fake",
                    "model": "deterministic-fake",
                }
            ]
        )
        mission = self.load_mission_with_agent_config(config_path)

        session = mission.launch_headless_work(
            work_kind="run",
            agent_id="fake-local",
            prompt="Summarize the workspace",
            allowed_paths=[str(self.target_repo)],
        )

        self.assertEqual(session.issue_id, "headless-run-000001")
        self.assertEqual(session.assigned_agent, "fake-local")
        self.assertEqual(session.task_packet["work_kind"], "headless-run")
        self.assertEqual(session.task_packet["prompt"], "Summarize the workspace")
        self.assertEqual(session.task_packet["allowed_paths"], [str(self.target_repo)])
        self.assertEqual(session.task_packet["agent_config"]["model"], "deterministic-fake")
        self.assertEqual(session.evidence_valid, True)
        self.assertEqual(session.status, "evidence-ready")
        reloaded = self.load_mission_with_agent_config(config_path)
        self.assertIn(session.session_id, reloaded.sessions)
        self.assertEqual(reloaded.sessions[session.session_id].evidence_valid, True)

    def test_headless_work_rejects_controller_router_and_unapproved_delegate_roles(self):
        agent_config = self.write_agent_config(
            [
                {
                    "id": "controller",
                    "role": "frontier",
                    "provider": "test-harness",
                    "runner": "fake",
                    "routing": "controller",
                },
                {
                    "id": "router",
                    "role": "frontier",
                    "provider": "test-harness",
                    "runner": "fake",
                    "routing": "router",
                },
                {
                    "id": "frontier-routed-worker",
                    "role": "local-agent",
                    "provider": "test-harness",
                    "runner": "fake",
                    "routing": "frontier",
                },
                {
                    "id": "gated-delegate",
                    "role": "delegate-agent",
                    "provider": "test-harness",
                    "runner": "fake",
                    "routing": "delegate",
                    "delegate_only": True,
                    "requires_approval": True,
                },
                {
                    "id": "uppercase-cloud-worker",
                    "role": "local-agent",
                    "provider": "ollama",
                    "runner": "fake",
                    "model": "remote:CLOUD",
                    "routing": "worker",
                },
                {
                    "id": "remote-provider-worker",
                    "role": "local-agent",
                    "provider": "remote",
                    "runner": "fake",
                    "model": "remote-provider-worker:14b",
                    "routing": "worker",
                },
                {
                    "id": "remote-runner-worker",
                    "role": "local-agent",
                    "provider": "local",
                    "runner": "remote-api",
                    "model": "remote-runner-worker:14b",
                    "routing": "worker",
                },
                {
                    "id": "worker",
                    "role": "local-agent",
                    "provider": "test-harness",
                    "runner": "fake",
                    "routing": "worker",
                },
            ]
        )
        mission = self.load_mission_with_agent_config(agent_config)

        self.assertEqual(
            [agent.id for agent in mission.assignment_agents()],
            ["worker"],
        )

        for agent_id in (
            "controller",
            "router",
            "frontier-routed-worker",
            "gated-delegate",
            "uppercase-cloud-worker",
            "remote-provider-worker",
            "remote-runner-worker",
        ):
            with self.subTest(agent_id=agent_id):
                with self.assertRaisesRegex(
                    LaunchBlockedError,
                    "assignable worker|routed and approved",
                ):
                    mission.launch_headless_work(
                        work_kind="run",
                        agent_id=agent_id,
                        prompt="Implement the requested change.",
                        allowed_paths=["."],
                    )

        session = mission.launch_headless_work(
            work_kind="run",
            agent_id="worker",
            prompt="Implement the requested change.",
            allowed_paths=["."],
        )
        self.assertEqual(session.assigned_agent, "worker")
        self.assertEqual(session.status, "evidence-ready")

    def test_deferred_runner_revalidates_agent_role_at_claim_time(self):
        config_path = self.write_agent_config(
            [
                {
                    "id": "mutable-worker",
                    "role": "local-agent",
                    "provider": "test-harness",
                    "runner": "fake",
                    "routing": "worker",
                }
            ]
        )
        mission = self.load_mission_with_agent_config(config_path)
        mission.assign_issue("ISS-01", "mutable-worker")
        mission.approve_issue("ISS-01")
        session = mission.launch_issue("ISS-01")
        config_path.write_text(
            json.dumps(
                {
                    "agents": [
                        {
                            "id": "mutable-worker",
                            "role": "frontier",
                            "provider": "test-harness",
                            "runner": "fake",
                            "routing": "controller",
                        }
                    ]
                }
            ),
            encoding="utf-8",
        )

        claimant = self.load_mission_with_agent_config(config_path)
        with self.assertRaisesRegex(
            LaunchBlockedError,
            "not authorized for deferred Local Agent execution",
        ):
            claimant.run_session(session.session_id)

        persisted = self.load_mission_with_agent_config(config_path).sessions[
            session.session_id
        ]
        self.assertEqual(persisted.status, "failed")
        self.assertFalse(persisted.worktree_path.exists())

    def test_deferred_runner_revalidates_uppercase_cloud_boundary_at_claim_time(self):
        config_path = self.write_agent_config(
            [
                {
                    "id": "mutable-worker",
                    "role": "local-agent",
                    "provider": "test-harness",
                    "runner": "fake",
                    "routing": "worker",
                }
            ]
        )
        mission = self.load_mission_with_agent_config(config_path)
        mission.assign_issue("ISS-01", "mutable-worker")
        mission.approve_issue("ISS-01")
        session = mission.launch_issue("ISS-01")
        config_path.write_text(
            json.dumps(
                {
                    "agents": [
                        {
                            "id": "mutable-worker",
                            "role": "local-agent",
                            "provider": "ollama",
                            "runner": "fake",
                            "model": "remote:CLOUD",
                            "routing": "worker",
                        }
                    ]
                }
            ),
            encoding="utf-8",
        )

        claimant = self.load_mission_with_agent_config(config_path)
        with self.assertRaisesRegex(
            LaunchBlockedError,
            "not authorized for deferred Local Agent execution",
        ):
            claimant.run_session(session.session_id)

        persisted = self.load_mission_with_agent_config(config_path).sessions[
            session.session_id
        ]
        self.assertEqual(persisted.status, "failed")
        self.assertFalse(persisted.worktree_path.exists())

    def test_headless_run_cli_allows_untracked_workspace(self):
        config_path = self.write_agent_config(
            [
                {
                    "id": "fake-local",
                    "role": "local-agent",
                    "provider": "test",
                    "runner": "fake",
                    "model": "deterministic-fake",
                }
            ]
        )
        missing_tracker = self.root / "missing-tracker"

        exit_code, output = self.run_cli(
            [
                "headless-run",
                "--target-repo",
                str(self.target_repo),
                "--tracker-dir",
                str(missing_tracker),
                "--issues-dir",
                str(missing_tracker / "issues"),
                "--runtime-root",
                str(self.runtime),
                "--agent-config",
                str(config_path),
                "--agent",
                "fake-local",
                "Summarize this untracked workspace",
            ]
        )

        payload = json.loads(output)
        self.assertEqual(exit_code, 0)
        self.assertEqual(payload["launch"], "headless-run")
        self.assertEqual(payload["status"], "evidence-ready")

    def test_headless_review_cli_returns_machine_readable_lifecycle_output(self):
        config_path = self.write_agent_config(
            [
                {
                    "id": "fake-local",
                    "role": "local-agent",
                    "provider": "test",
                    "runner": "fake",
                    "model": "deterministic-fake",
                },
                {
                    "id": "fake-reviewer",
                    "role": "local-agent",
                    "provider": "test",
                    "runner": "fake",
                    "model": "deterministic-reviewer",
                    "routing": "worker",
                }
            ]
        )
        mission = self.load_mission_with_agent_config(config_path)
        prior_session = mission.launch_headless_work(
            work_kind="run",
            agent_id="fake-local",
            prompt="Summarize the workspace",
            allowed_paths=[str(self.target_repo)],
        )

        exit_code, output = self.run_cli(
            [
                "headless-review",
                "--target-repo",
                str(self.target_repo),
                "--tracker-dir",
                str(self.tracker),
                "--runtime-root",
                str(self.runtime),
                "--agent-config",
                str(config_path),
                "--agent",
                "fake-reviewer",
                "--allowed-path",
                str(self.target_repo),
                prior_session.session_id,
            ]
        )

        payload = json.loads(output)
        self.assertEqual(exit_code, 0)
        self.assertEqual(payload["product"], "Alfredo")
        self.assertEqual(payload["launch"], "headless-review")
        self.assertEqual(payload["review_session_id"], prior_session.session_id)
        self.assertEqual(payload["review_context"]["session_id"], prior_session.session_id)
        self.assertEqual(payload["review_context"]["evidence_valid"], True)
        self.assertEqual(payload["selected_agent"], "fake-reviewer")
        self.assertEqual(payload["selected_model"], "deterministic-reviewer")
        self.assertEqual(payload["governance"]["orchestrator"], "AlbertMission")
        self.assertEqual(payload["governance"]["evidence_package"], "valid")
        self.assertIn("session-created", payload["lifecycle"])

    def test_command_and_visibility_policy_are_enforced(self):
        mission = self.load_mission()

        self.assertEqual(mission.classify_command("python -m unittest"), "auto-allowed")
        self.assertEqual(mission.classify_command("git push origin branch"), "frontier-approvable")
        self.assertEqual(mission.classify_command("rm -rf ."), "human-required")

        mission.record_command_approval("npm test", "auto-allowed")
        reloaded = self.load_mission()

        self.assertEqual(reloaded.classify_command("npm test"), "auto-allowed")
        self.assertEqual(reloaded.classify_file_for_frontier(".env"), "Blocked")
        self.assertEqual(reloaded.classify_file_for_frontier(".local/config.json"), "Local-only")
        self.assertEqual(reloaded.classify_file_for_frontier("src/app.py"), "Normal")

    def test_evidence_package_validation_blocks_incomplete_reviews(self):
        mission = self.load_mission()
        mission.approve_issue("ISS-01")
        session = mission.launch_issue("ISS-01")

        with self.assertRaises(EvidenceValidationError):
            mission.record_evidence(session.session_id, EvidencePackage(changed_files=["src/app.py"]))

        evidence = EvidencePackage(
            changed_files=["src/app.py"],
            diff_summary="Added mission board.",
            commands_run=["python -m unittest"],
            test_results="All tests passed.",
            known_risks="None.",
            proposed_context_updates="No glossary changes.",
            artifact_links=["artifact://evidence/ISS-01"],
        )
        mission.record_evidence(session.session_id, evidence)

        self.assertEqual(mission.sessions[session.session_id].evidence_valid, True)

    def test_evidence_validation_and_projection_exclude_links_the_reader_cannot_open(self):
        mission = self.load_mission()
        session = LocalAgentSession(
            session_id="session-ISS-01-reader-boundary",
            issue_id="ISS-01",
            assigned_agent=mission.issues["ISS-01"].assigned_agent,
            worktree_path=self.target_repo / ".albert-worktrees" / "ISS-01-reader-boundary",
            task_packet={},
            status="launched",
        )
        mission.sessions[session.session_id] = session
        app_local_ref = f"app-local://evidence/{session.session_id}"
        artifact_ref = f"artifact://evidence/{session.session_id}"
        evidence = EvidencePackage(
            changed_files=["src/app.py"],
            diff_summary="Added mission board.",
            commands_run=["python -m unittest"],
            test_results="All tests passed.",
            known_risks="None.",
            proposed_context_updates="No glossary changes.",
            artifact_links=["runtime/evidence/ISS-01.json"],
        )

        with self.assertRaisesRegex(
            EvidenceValidationError,
            "unsafe artifact link",
        ):
            mission.record_evidence(session.session_id, evidence)

        evidence.artifact_links = [
            "runtime/evidence/ISS-01.json",
            app_local_ref,
            artifact_ref,
        ]
        session.evidence = evidence
        session.evidence_valid = True
        session.status = "evidence-ready"

        issue = next(
            item
            for item in mission.board_summary()["issue_slices"]
            if item["issue_id"] == "ISS-01"
        )

        self.assertEqual(
            issue["evidence"]["artifact_links"],
            [app_local_ref, artifact_ref],
        )

    def test_frontier_review_routes_repair_policy_from_evidence(self):
        mission = self.load_mission()
        mission.approve_issue("ISS-01")
        session = mission.launch_issue("ISS-01")
        evidence = EvidencePackage(
            changed_files=["src/app.py"],
            diff_summary="Added mission board.",
            commands_run=["python -m unittest"],
            test_results="All tests passed.",
            known_risks="None.",
            proposed_context_updates="No glossary changes.",
        )
        mission.record_evidence(session.session_id, evidence)

        first = mission.record_frontier_review(session.session_id, "Rejected", reason="Acceptance missing.")
        second = mission.record_frontier_review(session.session_id, "Rejected", reason="Still missing.")
        third = mission.record_frontier_review(session.session_id, "Rejected", reason="Architecture failure.", failure_type="architecture")

        self.assertEqual(first.next_action, "same-local-agent-repair")
        self.assertEqual(second.next_action, "fresh-local-agent-repair")
        self.assertEqual(third.next_action, "frontier-architect-revision")

    def test_mission_records_are_generated_without_bulky_evidence(self):
        mission = self.load_mission()
        mission.approve_issue("ISS-01")
        session = mission.launch_issue("ISS-01")
        mission.record_evidence(
            session.session_id,
            EvidencePackage(
                changed_files=["src/app.py"],
                diff_summary="Added mission board.",
                commands_run=["python -m unittest"],
                test_results="All tests passed.",
                known_risks="None.",
                proposed_context_updates="No glossary changes.",
                artifact_links=["artifact://evidence/ISS-01"],
            ),
        )

        records = mission.generate_mission_records()

        self.assertTrue((records / "README.md").exists())
        self.assertTrue((records / "timeline.md").exists())
        self.assertTrue((records / "local-agent-tracker.md").exists())
        self.assertTrue((records / "evidence-index.md").exists())
        self.assertTrue((records / "frontier-review-summary.md").exists())
        self.assertTrue((records / "issues" / "ISS-01.md").exists())
        self.assertIn("Next action", (records / "README.md").read_text(encoding="utf-8"))
        self.assertIn("artifact://evidence/ISS-01", (records / "evidence-index.md").read_text(encoding="utf-8"))
        self.assertNotIn("Added mission board.", (records / "evidence-index.md").read_text(encoding="utf-8"))

    def test_pr_readiness_never_auto_merges_and_has_github_fallback(self):
        mission = self.load_mission()
        mission.approve_issue("ISS-01")
        session = mission.launch_issue("ISS-01")
        mission.record_evidence(
            session.session_id,
            EvidencePackage(
                changed_files=["src/app.py"],
                diff_summary="Added mission board.",
                commands_run=["python -m unittest"],
                test_results="All tests passed.",
                known_risks="None.",
                proposed_context_updates="No glossary changes.",
            ),
        )
        mission.record_frontier_review(session.session_id, "Approved", reason="Meets criteria.")

        fallback = mission.prepare_pr("ISS-01", gh_available=False)
        github = mission.prepare_pr("ISS-01", gh_available=True)

        self.assertEqual(fallback.merge_approved, False)
        self.assertIn("Manual PR instructions", fallback.body)
        self.assertIn("albert/mission-001/ISS-01-root", fallback.branch_name)
        self.assertIn("gh pr create", github.create_command)


if __name__ == "__main__":
    unittest.main()
