from pathlib import Path
from contextlib import redirect_stderr, redirect_stdout
import io
import json
import subprocess
import sys
import tempfile
import unittest

from albert_mvp.cli import main
from albert_mvp.agents import AgentConfigError, load_agent_registry
from albert_mvp.core import (
    AlbertError,
    AlbertMission,
    EvidencePackage,
    EvidenceValidationError,
    LaunchBlockedError,
    LocalAgentSession,
    LockedFieldError,
)
from albert_mvp.tui import build_tui_state, perform_tui_action, render_tui_error, render_tui_state


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

    def test_loads_prd_and_issues_into_dependency_ordered_board(self):
        mission = self.load_mission()

        summary = mission.board_summary()

        self.assertEqual(summary["prd_title"], "Local Coding Agent MVP Product Requirements Document")
        self.assertEqual(summary["issue_count"], 2)
        self.assertEqual(summary["ordered_issue_ids"], ["ISS-01", "ISS-02"])
        self.assertEqual(summary["ready_issue_ids"], [])
        self.assertEqual(summary["issue_slices"][0]["lifecycle"], "Needs review")
        self.assertEqual((self.runtime / mission.project_key / "runtime.json").exists(), True)

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

        self.assertEqual(command, "ollama run qwen3.6:27b --think false --nowordwrap --format json")
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

        self.assertEqual(session.status, "evidence-ready")
        self.assertEqual(session.task_packet["delegation"]["router_agent"], "qwen3.6-27b")
        self.assertEqual(session.task_packet["delegation"]["recommended_agent"], "kimi-k2.6-cloud")
        self.assertTrue((session.worktree_path / "prototype_app.py").exists())

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
                    "provider": "ollama",
                    "runner": "ollama",
                    "model": "qwen3.6:27b",
                    "command": router_command,
                    "routing": "router",
                },
                {
                    "id": "deepseek-v4-pro-cloud",
                    "role": "delegate-agent",
                    "provider": "ollama",
                    "runner": "ollama",
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

        self.assertEqual(session.status, "evidence-ready")
        self.assertEqual(session.evidence_valid, True)
        self.assertTrue(str(session.worktree_path).startswith(str(self.root / ".albert-worktrees")))
        self.assertTrue((session.worktree_path / "FAKE_AGENT_RESULT.md").exists())
        artifact_dir = self.runtime / mission.project_key / "sessions" / session.session_id
        self.assertTrue((artifact_dir / "task-packet.json").exists())
        self.assertTrue((artifact_dir / "fake-agent.log").exists())
        self.assertTrue((artifact_dir / "completion.json").exists())
        self.assertIn("fake-agent.log", "\n".join(session.evidence.artifact_links))
        self.assertIn("FAKE_AGENT_RESULT.md", session.evidence.changed_files)

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

        reloaded = self.load_mission_with_agent_config(config_path)
        persisted = reloaded.sessions[session.session_id]

        self.assertEqual(persisted.status, "evidence-ready")
        self.assertEqual(persisted.evidence_valid, True)
        self.assertEqual(persisted.evidence.commands_run, ["fake-agent fake-local"])
        self.assertIn("Deterministic fake completion", persisted.evidence.diff_summary)

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

        self.assertEqual(session.status, "evidence-ready")
        self.assertEqual(session.runner_exit_status, 0)
        self.assertEqual(session.evidence_valid, True)
        self.assertTrue((session.worktree_path / "COMMAND_AGENT_RESULT.txt").exists())
        self.assertIn("packet ISS-01", Path(session.artifacts["stdout"]).read_text(encoding="utf-8"))
        self.assertTrue(Path(session.artifacts["task_packet"]).exists())
        self.assertTrue(Path(session.artifacts["result"]).exists())

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

        self.assertEqual(session.status, "evidence-ready")
        self.assertEqual(session.evidence_valid, True)
        self.assertEqual(session.evidence.changed_files, ["src/app.py"])
        self.assertIn("src/app.py", session.evidence.diff_summary)
        self.assertIn("stdout.log", "\n".join(session.evidence.artifact_links))

    def test_automated_evidence_records_explicit_no_diff_result(self):
        script = self.root / "runner_no_changes.py"
        script.write_text("print('no file edits')\n", encoding="utf-8")
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

        self.assertEqual(session.evidence.changed_files, ["No worktree file changes detected."])
        self.assertEqual(session.evidence.diff_summary, "No worktree file changes detected.")
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

        result = perform_tui_action(
            mission,
            "review",
            "ISS-01",
            session_id=session.session_id,
            outcome="Approved",
            reason="Evidence satisfies the slice.",
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

        result = perform_tui_action(
            mission,
            "review",
            "ISS-01",
            session_id=session.session_id,
            outcome="approved",
            reason="Evidence satisfies the slice.",
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

        repair = perform_tui_action(
            mission,
            "review",
            "ISS-01",
            session_id=session.session_id,
            outcome="Needs repair",
            reason="Acceptance detail missing.",
        )
        first_reject = perform_tui_action(
            mission,
            "review",
            "ISS-01",
            session_id=session.session_id,
            outcome="Rejected",
            reason="Still incomplete.",
        )
        second_reject = perform_tui_action(
            mission,
            "review",
            "ISS-01",
            session_id=session.session_id,
            outcome="Rejected",
            reason="Still incomplete.",
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
        )
        reloaded = self.load_mission_with_agent_config(config_path)

        self.assertEqual(result.message, "Launched repair for ISS-01 as session-ISS-01-2.")
        self.assertEqual(result.session_id, "session-ISS-01-2")
        self.assertEqual(reloaded.sessions["session-ISS-01-2"].task_packet["repair_context"]["review_reason"], "Acceptance detail missing.")

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

        self.assertEqual(session.status, "evidence-ready")
        self.assertEqual(session.runner_exit_status, 0)
        self.assertTrue((session.worktree_path / "prototype" / "app.py").exists())
        self.assertEqual(session.evidence.changed_files, ["prototype/app.py"])
        self.assertIn("ollama-prompt.txt", "\n".join(session.evidence.artifact_links))
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

        self.assertEqual(session.status, "failed")
        self.assertEqual(session.evidence_valid, True)
        self.assertIn("Malformed Ollama output", session.evidence.known_risks)
        self.assertIn("ollama-output.txt", "\n".join(session.evidence.artifact_links))

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
        self.assertEqual(self.run_cli(["review", *base_args, session_id, "--outcome", "Approved", "--reason", "Meets criteria."])[0], 0)
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
            artifact_links=["runtime/evidence/ISS-01.json"],
        )
        mission.record_evidence(session.session_id, evidence)

        self.assertEqual(mission.sessions[session.session_id].evidence_valid, True)

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
