from __future__ import annotations

from contextlib import redirect_stderr, redirect_stdout
from dataclasses import asdict, replace
from datetime import datetime, timezone
import io
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import threading
from typing import Callable
import unittest
from unittest.mock import patch

import albert_mvp.workspace as workspace_module
from albert_mvp.cli import main
from albert_mvp.core import (
    AlbertError,
    AlbertMission,
    DelegationDecision,
    EvidencePackage,
    EvidenceValidationError,
    LocalAgentSession,
)
from albert_mvp.tui import perform_tui_action
from albert_mvp.workspace import (
    AgentConsoleHistoryService,
    AgentConsoleResponseService,
    ActivityJournalService,
    ConversationScope,
    MissionDraftService,
    ReviewWorkspaceService,
    SessionArtifactReadError,
    SessionArtifactService,
    ShellTerminalService,
    WorkspaceAction,
    WorkspaceQueueService,
    WorkspacePersistenceError,
    WorkspaceRevisionGapError,
    WorkspaceSnapshotService,
    WorkspaceScopeMismatchError,
    WorkspaceStaleActionError,
    WorkspaceSyncService,
    WorkstationActionService,
    WorkingContextCurationError,
    WorkingContextService,
)


ISSUE = """Status: ready-for-agent
Type: AFK

## Parent

PRD.md

## What to build

Restore the workspace session.

## Acceptance criteria

- [ ] The canonical snapshot is visible.

## Blocked by

None - can start immediately
"""


class WorkspaceSnapshotTest(unittest.TestCase):
    def setUp(self) -> None:
        self.tmp = tempfile.TemporaryDirectory()
        self.root = Path(self.tmp.name)
        self.target_repo = self.root / "target"
        self.target_repo.mkdir()
        self.tracker = self.root / "tracker"
        (self.tracker / "issues").mkdir(parents=True)
        (self.tracker / "PRD.md").write_text("# Command Deck Mission\n", encoding="utf-8")
        (self.tracker / "issues" / "01-restore.md").write_text(ISSUE, encoding="utf-8")
        self.runtime = self.root / "runtime"

    def tearDown(self) -> None:
        self.tmp.cleanup()

    def test_cli_returns_versioned_canonical_snapshot(self) -> None:
        output = io.StringIO()

        with redirect_stdout(output):
            exit_code = main(
                [
                    "workspace-snapshot",
                    "--target-repo",
                    str(self.target_repo),
                    "--tracker-dir",
                    str(self.tracker),
                    "--runtime-root",
                    str(self.runtime),
                    "--mission-id",
                    "command-deck",
                ]
            )

        snapshot = json.loads(output.getvalue())
        self.assertEqual(exit_code, 0)
        self.assertEqual(snapshot["schema_version"], 1)
        self.assertEqual(snapshot["revision"], 1)
        self.assertEqual(snapshot["workspace_session"]["status"], "ready")
        self.assertEqual(snapshot["active_mission"]["id"], "command-deck")
        self.assertEqual(snapshot["active_mission"]["title"], "Command Deck Mission")
        self.assertEqual(snapshot["conversation_scope"]["kind"], "working-directory")
        self.assertEqual(snapshot["operations_view"], "mission-board")

    def test_shell_terminal_executes_auto_allowed_command_without_accepting_output(
        self,
    ) -> None:
        snapshots = self.load_service()
        canonical_before = snapshots.snapshot().to_dict()

        result = ShellTerminalService(snapshots).submit(
            correlation_id="terminal-auto-allowed-1",
            command="python3 -m unittest --help",
            working_directory=str(self.target_repo),
            requested_paths=[],
            requester="mission-commander",
        )

        self.assertEqual(result.command_id, "terminal-command-000001")
        self.assertEqual(result.classification, "auto-allowed")
        self.assertEqual(result.status, "completed")
        self.assertEqual(result.exit_code, 0)
        self.assertIn("usage:", result.stdout.lower())
        self.assertEqual(snapshots.snapshot().to_dict(), canonical_before)
        restored_history = AgentConsoleHistoryService(self.load_service()).history()
        restored_journal = ActivityJournalService(self.load_service()).inspect()
        self.assertEqual(
            [message.content for message in restored_history],
            ["Shell Terminal command completed with exit code 0: terminal-command-000001."],
        )
        self.assertEqual(len(restored_journal.entries), 1)
        self.assertEqual(restored_journal.entries[0].action_type, "shell-command-completed")
        self.assertNotIn("usage:", restored_journal.entries[0].summary.lower())

    def test_shell_terminal_submission_replays_after_lost_audit_response_without_reexecution(
        self,
    ) -> None:
        snapshots = self.load_service()
        request = {
            "correlation_id": "terminal-submit-replay-1",
            "command": "python3 -m unittest --help",
            "working_directory": str(self.target_repo),
            "requested_paths": [],
            "requester": "mission-commander",
        }
        completed = subprocess.CompletedProcess(
            args=[],
            returncode=0,
            stdout="transient output",
            stderr="",
        )

        with (
            patch(
                "albert_mvp.workspace._trusted_system_executable",
                return_value="/usr/bin/bwrap",
            ),
            patch("albert_mvp.workspace._run_bounded_process", return_value=completed) as run,
            patch.object(
                ActivityJournalService,
                "record_shell_command_finished",
                side_effect=OSError("simulated Shell audit response loss"),
            ),
        ):
            with self.assertRaisesRegex(OSError, "Shell audit response loss"):
                ShellTerminalService(snapshots).submit(**request)

        with patch("albert_mvp.workspace._run_bounded_process") as replay_run:
            replayed = ShellTerminalService(self.load_service()).submit(**request)
            replayed_again = ShellTerminalService(self.load_service()).submit(**request)

        self.assertEqual(replayed, replayed_again)
        self.assertEqual(replayed.command_id, "terminal-command-000001")
        self.assertEqual(replayed.status, "completed")
        self.assertEqual(replayed.exit_code, 0)
        self.assertEqual(replayed.stdout, "")
        self.assertEqual(replayed.stderr, "")
        self.assertEqual(run.call_count, 1)
        replay_run.assert_not_called()
        self.assertEqual(len(ShellTerminalService(self.load_service()).inspect().commands), 1)
        self.assertEqual(len(ActivityJournalService(self.load_service()).inspect().entries), 1)
        self.assertEqual(len(AgentConsoleHistoryService(self.load_service()).history()), 1)
        with self.assertRaisesRegex(AlbertError, "different request"):
            ShellTerminalService(self.load_service()).submit(
                **{**request, "command": "python3 -m unittest discover"}
            )

    def test_shell_terminal_completion_store_loss_never_reexecutes_command(self) -> None:
        snapshots = self.load_service()
        terminal = ShellTerminalService(snapshots)
        request = {
            "correlation_id": "terminal-outcome-unknown-1",
            "command": "python3 -m unittest --help",
            "working_directory": str(self.target_repo),
            "requested_paths": [],
            "requester": "mission-commander",
        }
        original_persist = terminal._persist_terminal

        def lose_completed_record(**payload: object) -> None:
            commands = payload["commands"]
            self.assertIsInstance(commands, list)
            if commands[-1]["status"] in {"completed", "failed"}:
                raise OSError("simulated completion store loss after command effect")
            original_persist(**payload)

        completed = subprocess.CompletedProcess(
            args=[],
            returncode=0,
            stdout="transient output",
            stderr="",
        )
        with (
            patch.object(terminal, "_sandbox_argv", return_value=(["bwrap"], "")),
            patch.object(
                terminal,
                "_persist_terminal",
                side_effect=lose_completed_record,
            ),
            patch(
                "albert_mvp.workspace._run_bounded_process",
                return_value=completed,
            ) as run,
        ):
            with self.assertRaisesRegex(OSError, "completion store loss"):
                terminal.submit(**request)

        with patch("albert_mvp.workspace._run_bounded_process") as replay_run:
            replayed = ShellTerminalService(self.load_service()).submit(**request)
            replayed_again = ShellTerminalService(self.load_service()).submit(**request)

        self.assertEqual(replayed, replayed_again)
        self.assertEqual(replayed.status, "outcome-unknown")
        self.assertIsNone(replayed.exit_code)
        self.assertEqual(replayed.stdout, "")
        self.assertIn("not be retried", replayed.stderr)
        self.assertEqual(run.call_count, 1)
        replay_run.assert_not_called()
        persisted = ShellTerminalService(self.load_service()).inspect().commands
        self.assertEqual(len(persisted), 1)
        self.assertEqual(persisted[0].status, "outcome-unknown")
        history = AgentConsoleHistoryService(self.load_service()).history()
        self.assertEqual(
            [message.action_phase for message in history],
            ["shell-outcome-unknown"],
        )
        journal = ActivityJournalService(self.load_service()).inspect()
        self.assertEqual(
            [entry.action_type for entry in journal.entries],
            ["shell-command-outcome-unknown"],
        )

    def test_shell_terminal_post_start_oserror_is_outcome_unknown_and_never_reexecutes(
        self,
    ) -> None:
        snapshots = self.load_service()
        terminal = ShellTerminalService(snapshots)
        effect = self.target_repo / "post-start-effect.txt"
        request = {
            "correlation_id": "terminal-post-start-oserror-1",
            "command": "python3 -m unittest --help",
            "working_directory": str(self.target_repo),
            "requested_paths": [],
            "requester": "mission-commander",
        }

        def effect_then_oserror(*args: object, **kwargs: object) -> subprocess.CompletedProcess:
            effect.write_text("effect happened", encoding="utf-8")
            raise OSError("simulated post-start transport failure")

        with (
            patch.object(terminal, "_sandbox_argv", return_value=(["bwrap"], "")),
            patch(
                "albert_mvp.workspace._run_bounded_process",
                side_effect=effect_then_oserror,
            ) as run,
        ):
            with self.assertRaisesRegex(OSError, "post-start transport failure"):
                terminal.submit(**request)

        self.assertTrue(effect.exists())
        persisted = ShellTerminalService(self.load_service()).inspect().commands
        self.assertEqual(len(persisted), 1)
        self.assertEqual(persisted[0].status, "outcome-unknown")
        with patch("albert_mvp.workspace._run_bounded_process") as replay_run:
            replayed = ShellTerminalService(self.load_service()).submit(**request)
        self.assertEqual(replayed.status, "outcome-unknown")
        self.assertIn("will not be retried", replayed.stderr)
        self.assertEqual(run.call_count, 1)
        replay_run.assert_not_called()

    def test_shell_terminal_denial_replays_after_lost_audit_response(self) -> None:
        snapshots = self.load_service()
        terminal = ShellTerminalService(snapshots)
        pending = terminal.submit(
            correlation_id="terminal-denial-replay-1",
            command="git push origin main",
            working_directory=str(self.target_repo),
            requested_paths=[],
            requester="mission-commander",
            access_level="write",
        )

        with patch.object(
            ActivityJournalService,
            "record_shell_command_denied",
            side_effect=OSError("simulated denial audit response loss"),
        ):
            with self.assertRaisesRegex(OSError, "denial audit response loss"):
                terminal.deny(
                    command_id=pending.command_id,
                    decider="mission-commander",
                    reason="Release is not authorized.",
                )

        replayed = ShellTerminalService(self.load_service()).deny(
            command_id=pending.command_id,
            decider="mission-commander",
            reason="Release is not authorized.",
        )
        self.assertEqual(replayed.status, "denied")
        with self.assertRaisesRegex(AlbertError, "already denied"):
            ShellTerminalService(self.load_service()).deny(
                command_id=pending.command_id,
                decider="mission-commander",
                reason="A different denial boundary.",
            )
        history = AgentConsoleHistoryService(self.load_service()).history()
        self.assertEqual(
            [message.action_phase for message in history],
            ["shell-approval-request", "shell-denied"],
        )
        journal = ActivityJournalService(self.load_service()).inspect()
        self.assertEqual(
            [entry.action_type for entry in journal.entries],
            ["shell-command-approval-requested", "shell-command-denied"],
        )

    def test_shell_terminal_denial_repairs_console_after_late_audit_failure(self) -> None:
        snapshots = self.load_service()
        terminal = ShellTerminalService(snapshots)
        pending = terminal.submit(
            correlation_id="terminal-denial-console-replay-1",
            command="git push origin main",
            working_directory=str(self.target_repo),
            requested_paths=[],
            requester="mission-commander",
            access_level="write",
        )

        with patch.object(
            AgentConsoleHistoryService,
            "record_shell_command_denied",
            side_effect=OSError("simulated denial console response loss"),
        ):
            with self.assertRaisesRegex(OSError, "denial console response loss"):
                terminal.deny(
                    command_id=pending.command_id,
                    decider="mission-commander",
                    reason="Release is not authorized.",
                )

        replayed = ShellTerminalService(self.load_service()).deny(
            command_id=pending.command_id,
            decider="mission-commander",
            reason="Release is not authorized.",
        )
        self.assertEqual(replayed.status, "denied")
        history = AgentConsoleHistoryService(self.load_service()).history()
        self.assertEqual(
            [message.action_phase for message in history],
            ["shell-approval-request", "shell-denied"],
        )
        journal = ActivityJournalService(self.load_service()).inspect()
        self.assertEqual(
            [entry.action_type for entry in journal.entries],
            ["shell-command-approval-requested", "shell-command-denied"],
        )

    def test_shell_terminal_projects_live_attempt_as_executing(self) -> None:
        snapshots = self.load_service()
        terminal = ShellTerminalService(snapshots)
        execution_started = threading.Event()
        release_execution = threading.Event()
        results: list[object] = []

        def blocked_process(*args: object, **kwargs: object) -> subprocess.CompletedProcess:
            execution_started.set()
            self.assertTrue(release_execution.wait(timeout=3))
            return subprocess.CompletedProcess(args=[], returncode=0, stdout="done", stderr="")

        def submit() -> None:
            results.append(
                terminal.submit(
                    correlation_id="terminal-live-executing-1",
                    command="python3 -m unittest --help",
                    working_directory=str(self.target_repo),
                    requested_paths=[],
                    requester="mission-commander",
                )
            )

        with (
            patch.object(terminal, "_sandbox_argv", return_value=(["bwrap"], "")),
            patch("albert_mvp.workspace._run_bounded_process", side_effect=blocked_process),
        ):
            worker = threading.Thread(target=submit)
            worker.start()
            self.assertTrue(execution_started.wait(timeout=3))
            projected = ShellTerminalService(self.load_service()).inspect().commands
            self.assertEqual(len(projected), 1)
            self.assertEqual(projected[0].status, "executing")
            release_execution.set()
            worker.join(timeout=3)

        self.assertFalse(worker.is_alive())
        self.assertEqual(len(results), 1)
        self.assertEqual(results[0].status, "completed")

    def test_shell_terminal_inspect_durably_recovers_orphaned_execution_and_audit(
        self,
    ) -> None:
        snapshots = self.load_service()
        terminal = ShellTerminalService(snapshots)
        pending = terminal.submit(
            correlation_id="terminal-orphaned-inspect-1",
            command="git push origin main",
            working_directory=str(self.target_repo),
            requested_paths=[],
            requester="mission-commander",
            access_level="write",
        )

        with (
            patch.object(terminal, "_sandbox_argv", return_value=(["bwrap"], "")),
            patch(
                "albert_mvp.workspace._run_bounded_process",
                side_effect=KeyboardInterrupt("simulated process death"),
            ),
            patch.object(terminal, "_best_effort_mark_outcome_unknown"),
        ):
            with self.assertRaisesRegex(KeyboardInterrupt, "process death"):
                terminal.approve(
                    command_id=pending.command_id,
                    approver="frontier-model",
                )

        self.assertEqual(terminal._load_terminal()["commands"][0]["status"], "executing")
        with patch(
            "albert_mvp.workspace._process_identity_is_live",
            return_value=False,
        ):
            AgentConsoleHistoryService(self.load_service()).append(
                role="user",
                content="Discuss the next task.",
                outcome="proposed",
                source="mission-commander",
            )
            projected = ShellTerminalService(self.load_service()).inspect()

        self.assertEqual(projected.commands[0].status, "outcome-unknown")
        durable = ShellTerminalService(self.load_service())._load_terminal()["commands"]
        self.assertEqual(durable[0]["status"], "outcome-unknown")
        self.assertEqual(
            [message.action_phase for message in AgentConsoleHistoryService(self.load_service()).history()],
            ["shell-approval-request", "shell-approved", "shell-outcome-unknown", ""],
        )
        self.assertEqual(
            [entry.action_type for entry in ActivityJournalService(self.load_service()).inspect().entries],
            [
                "shell-command-approval-requested",
                "shell-command-approved",
                "shell-command-outcome-unknown",
            ],
        )

    def test_shell_terminal_repairs_denial_audit_before_later_history(self) -> None:
        snapshots = self.load_service()
        terminal = ShellTerminalService(snapshots)
        pending = terminal.submit(
            correlation_id="terminal-denial-causal-repair-1",
            command="git push origin main",
            working_directory=str(self.target_repo),
            requested_paths=[],
            requester="mission-commander",
            access_level="write",
        )
        with patch.object(
            ActivityJournalService,
            "record_shell_command_denied",
            side_effect=OSError("simulated denial journal loss"),
        ):
            with self.assertRaisesRegex(OSError, "denial journal loss"):
                terminal.deny(
                    command_id=pending.command_id,
                    decider="mission-commander",
                    reason="Do not release.",
                )

        history = AgentConsoleHistoryService(self.load_service())
        history.append(
            role="user",
            content="Discuss the next task.",
            outcome="proposed",
            source="mission-commander",
        )
        journal = ActivityJournalService(self.load_service())
        journal.record_workspace_action(
            correlation_id="later-navigation-1",
            snapshot=self.load_service().snapshot(),
        )

        self.assertEqual(
            [message.action_phase for message in history.history()],
            ["shell-approval-request", "shell-denied", ""],
        )
        self.assertEqual(
            [entry.action_type for entry in journal.inspect().entries],
            [
                "shell-command-approval-requested",
                "shell-command-denied",
                "operations-view-selected",
            ],
        )

    def test_shell_terminal_linearizes_decision_effect_with_later_history(self) -> None:
        snapshots = self.load_service()
        terminal = ShellTerminalService(snapshots)
        pending = terminal.submit(
            correlation_id="terminal-denial-linearized-1",
            command="git push origin main",
            working_directory=str(self.target_repo),
            requested_paths=[],
            requester="mission-commander",
            access_level="write",
        )
        reconciliation_paused = threading.Event()
        release_append = threading.Event()
        denial_started = threading.Event()
        prompt_errors: list[BaseException] = []
        denial_errors: list[BaseException] = []
        original_reconcile = ShellTerminalService.reconcile_audit

        def pause_after_reconciliation(service: ShellTerminalService) -> None:
            original_reconcile(service)
            if not reconciliation_paused.is_set():
                reconciliation_paused.set()
                self.assertTrue(release_append.wait(timeout=3))

        def append_prompt() -> None:
            try:
                AgentConsoleHistoryService(self.load_service()).append(
                    role="user",
                    content="First later prompt.",
                    outcome="proposed",
                    source="mission-commander",
                )
            except BaseException as exc:  # pragma: no cover - asserted below
                prompt_errors.append(exc)

        def deny_command() -> None:
            denial_started.set()
            try:
                ShellTerminalService(self.load_service()).deny(
                    command_id=pending.command_id,
                    decider="mission-commander",
                    reason="Do not release.",
                )
            except BaseException as exc:  # pragma: no cover - asserted below
                denial_errors.append(exc)

        with (
            patch.object(
                ShellTerminalService,
                "reconcile_audit",
                autospec=True,
                side_effect=pause_after_reconciliation,
            ),
            patch.object(
                ActivityJournalService,
                "record_shell_command_denied",
                side_effect=OSError("simulated denial journal loss"),
            ),
        ):
            prompt_thread = threading.Thread(target=append_prompt)
            prompt_thread.start()
            self.assertTrue(reconciliation_paused.wait(timeout=3))
            denial_thread = threading.Thread(target=deny_command)
            denial_thread.start()
            self.assertTrue(denial_started.wait(timeout=3))
            denial_thread.join(timeout=0.05)
            self.assertTrue(denial_thread.is_alive())
            self.assertEqual(
                terminal._load_terminal()["commands"][0]["status"],
                "pending-approval",
            )
            release_append.set()
            prompt_thread.join(timeout=3)
            denial_thread.join(timeout=3)

        self.assertFalse(prompt_thread.is_alive())
        self.assertFalse(denial_thread.is_alive())
        self.assertEqual(prompt_errors, [])
        self.assertEqual(len(denial_errors), 1)
        self.assertRegex(str(denial_errors[0]), "denial journal loss")
        AgentConsoleHistoryService(self.load_service()).append(
            role="user",
            content="Second later prompt.",
            outcome="proposed",
            source="mission-commander",
        )
        self.assertEqual(
            [
                message.action_phase
                for message in AgentConsoleHistoryService(self.load_service()).history()
            ],
            ["shell-approval-request", "", "shell-denied", ""],
        )

    def test_shell_terminal_repairs_finished_audit_before_later_history(self) -> None:
        snapshots = self.load_service()
        terminal = ShellTerminalService(snapshots)
        pending = terminal.submit(
            correlation_id="terminal-finished-causal-repair-1",
            command="git push origin main",
            working_directory=str(self.target_repo),
            requested_paths=[],
            requester="mission-commander",
            access_level="write",
        )
        completed = subprocess.CompletedProcess(
            args=[], returncode=0, stdout="transient", stderr=""
        )
        with (
            patch.object(terminal, "_sandbox_argv", return_value=(["bwrap"], "")),
            patch("albert_mvp.workspace._run_bounded_process", return_value=completed) as run,
            patch.object(
                AgentConsoleHistoryService,
                "record_shell_command_finished",
                side_effect=OSError("simulated finished console loss"),
            ),
        ):
            with self.assertRaisesRegex(OSError, "finished console loss"):
                terminal.approve(
                    command_id=pending.command_id,
                    approver="frontier-model",
                )

        history = AgentConsoleHistoryService(self.load_service())
        history.append(
            role="user",
            content="Discuss the next task.",
            outcome="proposed",
            source="mission-commander",
        )

        self.assertEqual(run.call_count, 1)
        self.assertEqual(
            [message.action_phase for message in history.history()],
            ["shell-approval-request", "shell-approved", "shell-finished", ""],
        )

    @unittest.skipUnless(shutil.which("bwrap"), "bubblewrap is not installed")
    def test_shell_terminal_process_sandbox_blocks_absolute_write_outside_workspace(
        self,
    ) -> None:
        snapshots = self.load_service()
        external = self.root / "undeclared-host-directory"
        external.mkdir()
        marker = external / "escape.txt"
        inside = self.target_repo / "inside-sandbox.txt"
        script = (
            "from pathlib import Path; "
            "Path('inside-sandbox.txt').write_text('inside'); "
            f"Path({str(marker)!r}).write_text('outside')"
        )
        command = f"{sys.executable} -c {json.dumps(script)}"
        snapshots._primary_mission.record_command_approval(command, "auto-allowed")

        result = ShellTerminalService(snapshots).submit(
            correlation_id="terminal-sandbox-absolute-write-1",
            command=command,
            working_directory=str(self.target_repo),
            requested_paths=[],
            requester="mission-commander",
            access_level="write",
        )

        self.assertEqual(result.status, "failed")
        self.assertNotEqual(result.exit_code, 0)
        self.assertTrue(inside.exists(), "the authorized workspace must remain writable")
        self.assertFalse(marker.exists(), "an undeclared host path must remain unchanged")
        self.assertRegex(
            result.stderr.lower(),
            r"no such file|read-only|permission denied",
        )
        host_probe = external / "host-mount-remains-writable.txt"
        host_probe.write_text("writable", encoding="utf-8")
        self.assertEqual(host_probe.read_text(encoding="utf-8"), "writable")

    @unittest.skipUnless(shutil.which("bwrap"), "bubblewrap is not installed")
    def test_shell_terminal_read_access_cannot_write_workspace_or_read_undeclared_host_path(
        self,
    ) -> None:
        snapshots = self.load_service()
        secret = self.root / "undeclared-secret.txt"
        secret.write_text("must-not-be-readable", encoding="utf-8")
        workspace_marker = self.target_repo / "READ_MUST_NOT_WRITE.txt"
        write_script = "from pathlib import Path; Path('READ_MUST_NOT_WRITE.txt').write_text('blocked')"
        write_command = f"{sys.executable} -c {json.dumps(write_script)}"
        read_script = f"from pathlib import Path; print(Path({str(secret)!r}).read_text())"
        read_command = f"{sys.executable} -c {json.dumps(read_script)}"
        snapshots._primary_mission.record_command_approval(write_command, "auto-allowed")
        snapshots._primary_mission.record_command_approval(read_command, "auto-allowed")
        terminal = ShellTerminalService(snapshots)

        write_result = terminal.submit(
            correlation_id="terminal-read-boundary-1",
            command=write_command,
            working_directory=str(self.target_repo),
            requested_paths=[],
            requester="mission-commander",
            access_level="read",
        )
        read_result = terminal.submit(
            correlation_id="terminal-read-boundary-2",
            command=read_command,
            working_directory=str(self.target_repo),
            requested_paths=[],
            requester="mission-commander",
            access_level="read",
        )

        self.assertEqual(write_result.status, "failed")
        self.assertEqual(read_result.status, "failed")
        self.assertFalse(workspace_marker.exists())
        self.assertNotIn("must-not-be-readable", read_result.stdout)
        self.assertRegex(write_result.stderr.lower(), r"read-only|permission denied")
        self.assertRegex(read_result.stderr.lower(), r"permission denied|no such file")

    @unittest.skipUnless(shutil.which("bwrap"), "bubblewrap is not installed")
    def test_shell_terminal_write_grant_mounts_only_requested_external_path_writable(
        self,
    ) -> None:
        snapshots = self.load_service()
        external = self.root / "write-granted"
        external.mkdir()
        terminal = ShellTerminalService(snapshots)
        terminal.create_path_grant(
            correlation_id="terminal-write-grant-1",
            expected_revision=0,
            path=str(external),
            access_level="write",
            duration_seconds=300,
            requester="mission-commander",
        )
        marker = external / "allowed.txt"
        script = f"from pathlib import Path; Path({str(marker)!r}).write_text('allowed')"
        command = f"{sys.executable} -c {json.dumps(script)}"
        snapshots._primary_mission.record_command_approval(command, "auto-allowed")

        result = terminal.submit(
            correlation_id="terminal-write-granted-command-1",
            command=command,
            working_directory=str(self.target_repo),
            requested_paths=[str(external)],
            requester="mission-commander",
            access_level="write",
        )

        self.assertEqual(result.status, "completed")
        self.assertEqual(marker.read_text(encoding="utf-8"), "allowed")

    @unittest.skipUnless(shutil.which("bwrap"), "bubblewrap is not installed")
    def test_shell_terminal_read_grant_mounts_requested_external_path_read_only(
        self,
    ) -> None:
        snapshots = self.load_service()
        external = self.root / "read-granted"
        external.mkdir()
        terminal = ShellTerminalService(snapshots)
        terminal.create_path_grant(
            correlation_id="terminal-read-grant-1",
            expected_revision=0,
            path=str(external),
            access_level="read",
            duration_seconds=300,
            requester="mission-commander",
        )
        marker = external / "blocked.txt"
        script = f"from pathlib import Path; Path({str(marker)!r}).write_text('blocked')"
        command = f"{sys.executable} -c {json.dumps(script)}"
        snapshots._primary_mission.record_command_approval(command, "auto-allowed")

        result = terminal.submit(
            correlation_id="terminal-read-granted-command-1",
            command=command,
            working_directory=str(self.target_repo),
            requested_paths=[str(external)],
            requester="mission-commander",
            access_level="read",
        )

        self.assertEqual(result.status, "failed")
        self.assertFalse(marker.exists())
        self.assertRegex(result.stderr.lower(), r"read-only|permission denied")

    def test_shell_terminal_timeout_returns_actionable_failure_and_persists_exit_124(
        self,
    ) -> None:
        snapshots = self.load_service()
        command = f"{sys.executable} -c {json.dumps('import time; time.sleep(60)')}"
        snapshots._primary_mission.record_command_approval(command, "auto-allowed")

        with (
            patch(
                "albert_mvp.workspace._trusted_system_executable",
                return_value="/usr/bin/bwrap",
            ),
            patch("albert_mvp.workspace._run_bounded_process") as run,
        ):
            run.return_value = subprocess.CompletedProcess(
                args=[],
                returncode=124,
                stdout="partial stdout",
                stderr="partial stderr\nProcess timed out after 30 seconds.",
            )
            result = ShellTerminalService(snapshots).submit(
                correlation_id="terminal-timeout-1",
                command=command,
                working_directory=str(self.target_repo),
                requested_paths=[],
                requester="mission-commander",
            )

        self.assertEqual(result.status, "failed")
        self.assertEqual(result.exit_code, 124)
        self.assertEqual(result.stdout, "partial stdout")
        self.assertIn("partial stderr", result.stderr)
        self.assertIn("timed out", result.stderr.lower())
        persisted = ShellTerminalService(self.load_service()).inspect().commands[0]
        self.assertEqual(persisted.status, "failed")
        self.assertEqual(persisted.exit_code, 124)

    def test_shell_terminal_bounds_returned_stdout_and_stderr(self) -> None:
        snapshots = self.load_service()
        command = "python3 -m unittest --help"
        noisy_command = [
            sys.executable,
            "-c",
            "import os\nwhile True: os.write(1, b'o' * 65536)",
        ]

        with patch.object(
            ShellTerminalService,
            "_sandbox_argv",
            return_value=(noisy_command, ""),
        ):
            result = ShellTerminalService(snapshots).submit(
                correlation_id="terminal-bounded-output-1",
                command=command,
                working_directory=str(self.target_repo),
                requested_paths=[],
                requester="mission-commander",
            )

        self.assertEqual(result.status, "failed")
        self.assertEqual(result.exit_code, 125)
        self.assertLessEqual(
            len(result.stdout.encode("utf-8")) + len(result.stderr.encode("utf-8")),
            1_000_000,
        )
        self.assertIn("output exceeded", result.stderr)
        self.assertIn("terminated", result.stderr)

    def test_shell_terminal_fails_closed_when_bubblewrap_is_unavailable(self) -> None:
        snapshots = self.load_service()
        command = "python3 -m unittest --help"

        with (
            patch("albert_mvp.workspace._trusted_system_executable", return_value=None),
            patch("albert_mvp.workspace._run_bounded_process") as run,
        ):
            result = ShellTerminalService(snapshots).submit(
                correlation_id="terminal-no-bwrap-1",
                command=command,
                working_directory=str(self.target_repo),
                requested_paths=[],
                requester="mission-commander",
            )

        run.assert_not_called()
        self.assertEqual(result.status, "failed")
        self.assertEqual(result.exit_code, 126)
        self.assertIn("sandbox unavailable", result.stderr.lower())
        self.assertIn("not executed", result.stderr.lower())
        persisted = ShellTerminalService(self.load_service()).inspect().commands[0]
        self.assertEqual(persisted.status, "failed")
        self.assertEqual(persisted.exit_code, 126)

    @unittest.skipUnless(shutil.which("bwrap"), "bubblewrap is not installed")
    def test_shell_terminal_does_not_inherit_sensitive_environment(self) -> None:
        snapshots = self.load_service()
        script = (
            "import os; "
            "print(os.environ.get('OPENAI_API_KEY', 'missing')); "
            "print(os.environ.get('HOME', 'missing'))"
        )
        command = f"{sys.executable} -c {json.dumps(script)}"
        snapshots._primary_mission.record_command_approval(command, "auto-allowed")

        with patch.dict(os.environ, {"OPENAI_API_KEY": "must-not-reach-child"}):
            result = ShellTerminalService(snapshots).submit(
                correlation_id="terminal-sanitized-environment-1",
                command=command,
                working_directory=str(self.target_repo),
                requested_paths=[],
                requester="mission-commander",
            )

        self.assertEqual(result.status, "completed")
        self.assertEqual(result.stdout.splitlines(), ["missing", "/tmp"])
        self.assertNotIn("must-not-reach-child", result.stdout)

    @unittest.skipUnless(shutil.which("bwrap"), "bubblewrap is not installed")
    def test_shell_terminal_does_not_implicitly_mount_an_undeclared_tmp_script(self) -> None:
        snapshots = self.load_service()
        script = self.root / "undeclared-host-script.py"
        script.write_text("print('UNDECLARED_TMP_EXECUTED')\n", encoding="utf-8")
        command = f"{sys.executable} {script}"
        snapshots._primary_mission.record_command_approval(command, "auto-allowed")

        result = ShellTerminalService(snapshots).submit(
            correlation_id="terminal-undeclared-tmp-script-1",
            command=command,
            working_directory=str(self.target_repo),
            requested_paths=[],
            requester="mission-commander",
        )

        self.assertEqual(result.status, "failed")
        self.assertNotIn("UNDECLARED_TMP_EXECUTED", result.stdout)

    def test_shell_terminal_waits_for_frontier_approval_before_execution(self) -> None:
        snapshots = self.load_service()
        command = "python3 -c \"print('frontier approved')\""
        snapshots._primary_mission.record_command_approval(
            command,
            "frontier-approvable",
        )
        terminal = ShellTerminalService(snapshots)

        pending = terminal.submit(
            correlation_id="terminal-frontier-pending-1",
            command=command,
            working_directory=str(self.target_repo),
            requested_paths=[],
            requester="mission-commander",
        )

        self.assertEqual(pending.classification, "frontier-approvable")
        self.assertEqual(pending.status, "pending-approval")
        self.assertIsNone(pending.exit_code)
        self.assertEqual(pending.stdout, "")
        approved = terminal.approve(
            command_id=pending.command_id,
            approver="frontier-model",
        )
        self.assertEqual(approved.status, "completed")
        self.assertEqual(approved.exit_code, 0)
        self.assertEqual(approved.stdout.strip(), "frontier approved")

    def test_shell_terminal_completion_retains_submitting_mission_after_switch(self) -> None:
        primary = AlbertMission(
            target_repo=self.target_repo,
            tracker_dir=self.tracker,
            runtime_root=self.runtime,
            mission_id="command-deck",
        ).load()
        background_tracker = self.root / "terminal-background-tracker"
        (background_tracker / "issues").mkdir(parents=True)
        (background_tracker / "PRD.md").write_text(
            "# Background Terminal Mission\n",
            encoding="utf-8",
        )
        (background_tracker / "issues" / "01-background.md").write_text(
            ISSUE.replace("Restore the workspace session.", "Run background work."),
            encoding="utf-8",
        )
        background = AlbertMission(
            target_repo=self.target_repo,
            tracker_dir=background_tracker,
            runtime_root=self.runtime,
            mission_id="background-terminal",
        ).load()
        snapshots = WorkspaceSnapshotService(primary, missions=(background,))
        terminal = ShellTerminalService(snapshots)
        command = "python3 -c \"print('mission retained')\""

        pending = terminal.submit(
            correlation_id="terminal-mission-attribution-1",
            command=command,
            working_directory=str(self.target_repo),
            requested_paths=[],
            requester="mission-commander",
            access_level="read",
        )
        before_switch = snapshots.snapshot()
        snapshots.update_preferences(
            active_mission_id="background-terminal",
            conversation_scope=before_switch.conversation_scope,
            operations_view=before_switch.operations_view,
            event_metadata={"correlation_id": "terminal-switch-background"},
        )

        completed = terminal.approve(
            command_id=pending.command_id,
            approver="mission-commander",
        )

        self.assertEqual(completed.status, "completed")
        record = terminal.inspect().commands[0]
        self.assertEqual(record.mission_id, "command-deck")
        history = AgentConsoleHistoryService(snapshots).history()
        self.assertEqual(history[-1].scope.mission_id, "command-deck")
        journal = ActivityJournalService(snapshots).inspect()
        mission_entities = [
            entity.entity_id
            for entity in journal.entries[-1].affected_entities
            if entity.entity_type == "mission"
        ]
        self.assertEqual(mission_entities, ["command-deck"])

    def test_shell_terminal_denial_prevents_human_required_command_execution(self) -> None:
        snapshots = self.load_service()
        marker = self.target_repo / "denied.txt"
        command = (
            "python3 -c \"from pathlib import Path; "
            "Path('denied.txt').write_text('must not run')\""
        )
        terminal = ShellTerminalService(snapshots)
        pending = terminal.submit(
            correlation_id="terminal-human-deny-1",
            command=command,
            working_directory=str(self.target_repo),
            requested_paths=[],
            requester="mission-commander",
        )

        denied = terminal.deny(
            command_id=pending.command_id,
            decider="mission-commander",
            reason="This command is outside the intended task.",
        )

        self.assertEqual(pending.classification, "human-required")
        self.assertEqual(denied.status, "denied")
        self.assertIsNone(denied.exit_code)
        self.assertEqual(denied.stdout, "")
        self.assertFalse(marker.exists())
        with self.assertRaisesRegex(AlbertError, "already denied"):
            terminal.approve(
                command_id=pending.command_id,
                approver="mission-commander",
            )

    def test_additional_path_grant_authorizes_bounded_external_read_access(self) -> None:
        snapshots = self.load_service()
        external = self.root / "external-docs"
        external.mkdir()
        terminal = ShellTerminalService(snapshots)

        with patch("albert_mvp.workspace.datetime") as clock:
            clock.now.return_value = datetime(2026, 6, 27, 8, 0, tzinfo=timezone.utc)
            grant = terminal.create_path_grant(
                correlation_id="path-grant-read-1",
                expected_revision=0,
                path=str(external),
                access_level="read",
                duration_seconds=300,
                requester="mission-commander",
            )
            result = terminal.submit(
                correlation_id="terminal-external-read-1",
                command="python3 -m unittest --help",
                working_directory=str(external),
                requested_paths=[str(external)],
                requester="mission-commander",
                access_level="read",
            )

        self.assertEqual(grant.grant_id, "path-grant-000001")
        self.assertEqual(grant.path, str(external.resolve()))
        self.assertEqual(grant.access_level, "read")
        self.assertEqual(grant.duration_seconds, 300)
        self.assertEqual(grant.expires_at, "2026-06-27T08:05:00Z")
        self.assertEqual(result.status, "completed")
        self.assertEqual(result.exit_code, 0)

    def test_additional_path_grant_rejects_stale_terminal_revision(self) -> None:
        snapshots = self.load_service()
        external = self.root / "external-stale"
        external.mkdir()
        terminal = ShellTerminalService(snapshots)

        with self.assertRaises(WorkspaceStaleActionError) as raised:
            terminal.create_path_grant(
                correlation_id="path-grant-stale-1",
                expected_revision=7,
                path=str(external),
                access_level="read",
                duration_seconds=300,
                requester="mission-commander",
            )

        self.assertEqual(raised.exception.expected_revision, 7)
        self.assertEqual(raised.exception.current_revision, 0)

    def test_additional_path_grant_exact_retry_reconciles_missing_audit(self) -> None:
        snapshots = self.load_service()
        external = self.root / "grant-replay-external"
        external.mkdir()
        terminal = ShellTerminalService(snapshots)

        with patch.object(
            ActivityJournalService,
            "record_additional_path_grant_created",
            side_effect=WorkspacePersistenceError("injected journal failure"),
        ):
            with self.assertRaisesRegex(
                WorkspacePersistenceError,
                "injected journal failure",
            ):
                terminal.create_path_grant(
                    correlation_id="path-grant-replay-1",
                    expected_revision=0,
                    path=str(external),
                    access_level="read",
                    duration_seconds=300,
                    requester="mission-commander",
                )

        replayed = ShellTerminalService(self.load_service()).create_path_grant(
            correlation_id="path-grant-replay-1",
            expected_revision=0,
            path=str(external),
            access_level="read",
            duration_seconds=300,
            requester="mission-commander",
        )
        projection = ShellTerminalService(self.load_service()).inspect()
        history = AgentConsoleHistoryService(self.load_service()).history()
        journal = ActivityJournalService(self.load_service()).inspect()

        self.assertEqual(replayed.grant_id, "path-grant-000001")
        self.assertEqual(len(projection.grants), 1)
        self.assertEqual(
            [entry.action_type for entry in journal.entries].count(
                "additional-path-grant-created"
            ),
            1,
        )
        self.assertEqual(
            [message.action_phase for message in history].count(
                "shell-path-grant-created"
            ),
            1,
        )

    def test_blocked_shell_submission_persists_typed_path_grant_requests(self) -> None:
        snapshots = self.load_service()
        terminal = ShellTerminalService(snapshots)
        first_external = self.root / "typed-grant-first"
        second_external = self.root / "typed-grant-second"
        first_external.mkdir()
        second_external.mkdir()
        command = "python3 -m unittest --help"

        with self.assertRaisesRegex(AlbertError, "Additional Path Grant"):
            terminal.submit(
                correlation_id="typed-grant-shell-1",
                command=command,
                working_directory=str(self.target_repo),
                requested_paths=[str(first_external), str(second_external)],
                requester="mission-commander",
                access_level="read",
            )

        pending = ShellTerminalService(self.load_service()).inspect()
        self.assertEqual(len(pending.path_grant_requests), 1)
        first_request = pending.path_grant_requests[0]
        self.assertEqual(first_request.correlation_id, "typed-grant-shell-1")
        self.assertEqual(first_request.path, str(first_external.resolve()))
        self.assertEqual(first_request.access_level, "read")
        self.assertEqual(first_request.duration_seconds, 900)
        self.assertEqual(first_request.affected_action, command)
        self.assertEqual(first_request.status, "pending")

        terminal = ShellTerminalService(self.load_service())
        terminal.create_path_grant(
            correlation_id="typed-grant-accept-1",
            request_id=first_request.request_id,
            expected_revision=pending.revision,
            path=first_request.path,
            access_level=first_request.access_level,
            duration_seconds=first_request.duration_seconds,
            requester="mission-commander",
        )
        with self.assertRaisesRegex(AlbertError, "Additional Path Grant"):
            terminal.submit(
                correlation_id="typed-grant-shell-1",
                command=command,
                working_directory=str(self.target_repo),
                requested_paths=[str(first_external), str(second_external)],
                requester="mission-commander",
                access_level="read",
            )

        second_projection = ShellTerminalService(self.load_service()).inspect()
        self.assertEqual(
            [request.status for request in second_projection.path_grant_requests],
            ["granted", "pending"],
        )
        second_request = second_projection.path_grant_requests[1]
        self.assertEqual(second_request.path, str(second_external.resolve()))
        ShellTerminalService(self.load_service()).deny_path_grant_request(
            correlation_id="typed-grant-deny-1",
            request_id=second_request.request_id,
            expected_revision=second_projection.revision,
            path=second_request.path,
            access_level=second_request.access_level,
            duration_seconds=second_request.duration_seconds,
            requester="mission-commander",
            reason=second_request.reason,
            affected_action=second_request.affected_action,
        )

        restored = ShellTerminalService(self.load_service()).inspect()
        self.assertEqual(
            [request.status for request in restored.path_grant_requests],
            ["granted", "denied"],
        )
        history = AgentConsoleHistoryService(self.load_service()).history()
        self.assertEqual(
            [message.action_phase for message in history].count(
                "shell-path-grant-requested"
            ),
            2,
        )

    def test_contextual_path_grant_denial_is_durable_and_auditable_after_restart(
        self,
    ) -> None:
        snapshots = self.load_service()
        external = self.root / "denied-external"
        external.mkdir()

        with patch("albert_mvp.workspace.datetime") as clock:
            clock.now.return_value = datetime(2026, 7, 11, 10, 30, tzinfo=timezone.utc)
            denial = ShellTerminalService(snapshots).deny_path_grant_request(
                correlation_id="path-grant-denial-1",
                request_id="contextual-grant-1",
                expected_revision=0,
                path=str(external),
                access_level="write",
                duration_seconds=900,
                requester="mission-commander",
                reason="The blocked command needs write access outside the workspace.",
                affected_action="python3 tools/export.py",
            )

        restored_terminal = ShellTerminalService(self.load_service()).inspect()
        restored_history = AgentConsoleHistoryService(self.load_service()).history()
        restored_journal = ActivityJournalService(self.load_service()).inspect()

        self.assertEqual(denial.denial_id, "path-grant-denial-000001")
        self.assertEqual(denial.denied_at, "2026-07-11T10:30:00Z")
        self.assertEqual(restored_terminal.revision, 1)
        self.assertEqual(restored_terminal.grants, ())
        self.assertEqual(restored_terminal.grant_denials, (denial,))
        self.assertEqual(restored_history[-1].source, "mission-commander")
        self.assertEqual(restored_history[-1].outcome, "rejected")
        self.assertIn("contextual-grant-1", restored_history[-1].content)
        self.assertEqual(restored_journal.entries[-1].actor, "mission-commander")
        self.assertEqual(
            restored_journal.entries[-1].action_type,
            "additional-path-grant-denied",
        )
        self.assertEqual(
            restored_journal.entries[-1].correlation_id,
            "path-grant-denial-1",
        )

    def test_path_grant_denial_exact_retry_reconciles_missing_console_audit(self) -> None:
        snapshots = self.load_service()
        external = self.root / "grant-denial-replay-external"
        external.mkdir()
        request = {
            "correlation_id": "path-grant-denial-replay-1",
            "request_id": "contextual-grant-replay-1",
            "expected_revision": 0,
            "path": str(external),
            "access_level": "write",
            "duration_seconds": 600,
            "requester": "mission-commander",
            "reason": "The requested authority is too broad.",
            "affected_action": "python3 tools/export.py",
        }

        with patch.object(
            AgentConsoleHistoryService,
            "record_additional_path_grant_denied",
            side_effect=WorkspacePersistenceError("injected console failure"),
        ):
            with self.assertRaisesRegex(
                WorkspacePersistenceError,
                "injected console failure",
            ):
                ShellTerminalService(snapshots).deny_path_grant_request(**request)

        replayed = ShellTerminalService(self.load_service()).deny_path_grant_request(
            **request
        )
        projection = ShellTerminalService(self.load_service()).inspect()
        history = AgentConsoleHistoryService(self.load_service()).history()
        journal = ActivityJournalService(self.load_service()).inspect()

        self.assertEqual(replayed.denial_id, "path-grant-denial-000001")
        self.assertEqual(len(projection.grant_denials), 1)
        self.assertEqual(
            [entry.action_type for entry in journal.entries].count(
                "additional-path-grant-denied"
            ),
            1,
        )
        self.assertEqual(
            [message.action_phase for message in history].count(
                "shell-path-grant-denied"
            ),
            1,
        )

    def test_expired_additional_path_grant_stops_authorizing_external_access(self) -> None:
        snapshots = self.load_service()
        external = self.root / "expired-external"
        external.mkdir()
        terminal = ShellTerminalService(snapshots)
        with patch("albert_mvp.workspace.datetime") as clock:
            clock.now.return_value = datetime(2026, 6, 27, 8, 0, tzinfo=timezone.utc)
            terminal.create_path_grant(
                correlation_id="path-grant-expiry-1",
                expected_revision=0,
                path=str(external),
                access_level="read",
                duration_seconds=60,
                requester="mission-commander",
            )
            clock.now.return_value = datetime(2026, 6, 27, 8, 2, tzinfo=timezone.utc)
            with self.assertRaisesRegex(AlbertError, "expired"):
                terminal.submit(
                    correlation_id="terminal-expired-path-1",
                    command="python3 -m unittest --help",
                    working_directory=str(external),
                    requested_paths=[str(external)],
                    requester="mission-commander",
                    access_level="read",
                )

    def test_shell_terminal_rejects_malformed_persisted_path_grant_before_authorization(
        self,
    ) -> None:
        terminal = ShellTerminalService(self.load_service())
        external = self.root / "forged-grant-external"
        external.mkdir()
        terminal.terminal_path.write_text(
            json.dumps(
                {
                    "schema_version": 1,
                    "revision": 1,
                    "commands": [],
                    "grants": [
                        {
                            "grant_id": "path-grant-000001",
                            "correlation_id": "forged-grant-1",
                            "path": str(external.resolve()),
                            "access_level": "read",
                            "duration_seconds": 300,
                            "granted_by": "mission-commander",
                            "granted_at": "not-a-time",
                            "expires_at": "zzzz",
                        }
                    ],
                    "grant_denials": [],
                }
            ),
            encoding="utf-8",
        )

        with self.assertRaisesRegex(
            WorkspacePersistenceError,
            "Additional Path Grant.*timestamp",
        ):
            ShellTerminalService(self.load_service()).inspect()

    def test_local_agent_cannot_expand_or_renew_additional_path_grant(self) -> None:
        snapshots = self.load_service()
        external = self.root / "agent-grant"
        external.mkdir()
        terminal = ShellTerminalService(snapshots)
        grant = terminal.create_path_grant(
            correlation_id="path-grant-agent-boundary-1",
            expected_revision=0,
            path=str(external),
            access_level="read",
            duration_seconds=60,
            requester="mission-commander",
        )

        with self.assertRaisesRegex(
            AlbertError,
            "cannot broaden, renew, or change",
        ):
            terminal.change_path_grant(
                grant_id=grant.grant_id,
                path=str(self.root),
                access_level="write",
                duration_seconds=3600,
                requester="local-agent",
            )

        with self.assertRaisesRegex(AlbertError, "no active write"):
            terminal.submit(
                correlation_id="terminal-agent-write-denied-1",
                command="python3 -m unittest --help",
                working_directory=str(external),
                requested_paths=[str(external)],
                requester="local-agent",
                access_level="write",
            )

    def test_cli_executes_shell_terminal_command_and_restores_metadata_without_bytes(
        self,
    ) -> None:
        common = [
            "--target-repo",
            str(self.target_repo),
            "--tracker-dir",
            str(self.tracker),
            "--runtime-root",
            str(self.runtime),
            "--mission-id",
            "command-deck",
        ]
        submit_output = io.StringIO()
        with redirect_stdout(submit_output):
            submit_exit = main(
                [
                    "shell-terminal-submit",
                    *common,
                    "--correlation-id",
                    "terminal-cli-submit-1",
                    "--command-text",
                    "python3 -m unittest --help",
                    "--working-directory",
                    str(self.target_repo),
                    "--requester",
                    "mission-commander",
                    "--access-level",
                    "read",
                ]
            )
        submitted = json.loads(submit_output.getvalue())
        inspect_output = io.StringIO()
        with redirect_stdout(inspect_output):
            inspect_exit = main(["shell-terminal", *common])
        projection = json.loads(inspect_output.getvalue())

        self.assertEqual(submit_exit, 0)
        self.assertEqual(inspect_exit, 0)
        self.assertEqual(submitted["status"], "completed")
        self.assertIn("usage:", submitted["stdout"].lower())
        self.assertEqual(projection["schema_version"], 1)
        self.assertEqual(projection["revision"], 2)
        self.assertEqual(projection["commands"][0]["command_id"], "terminal-command-000001")
        self.assertEqual(projection["commands"][0]["status"], "completed")
        self.assertNotIn("stdout", projection["commands"][0])
        self.assertNotIn("stderr", projection["commands"][0])

    def test_cli_creates_and_restores_additional_path_grant(self) -> None:
        external = self.root / "cli-external"
        external.mkdir()
        common = [
            "--target-repo",
            str(self.target_repo),
            "--tracker-dir",
            str(self.tracker),
            "--runtime-root",
            str(self.runtime),
            "--mission-id",
            "command-deck",
        ]
        create_output = io.StringIO()
        with redirect_stdout(create_output):
            create_exit = main(
                [
                    "additional-path-grant-create",
                    *common,
                    "--correlation-id",
                    "path-grant-cli-1",
                    "--expected-terminal-revision",
                    "0",
                    "--path",
                    str(external),
                    "--access-level",
                    "write",
                    "--duration-seconds",
                    "900",
                    "--requester",
                    "mission-commander",
                ]
            )
        created = json.loads(create_output.getvalue())
        inspect_output = io.StringIO()
        with redirect_stdout(inspect_output):
            inspect_exit = main(["shell-terminal", *common])
        projection = json.loads(inspect_output.getvalue())

        self.assertEqual(create_exit, 0)
        self.assertEqual(inspect_exit, 0)
        self.assertEqual(created["grant_id"], "path-grant-000001")
        self.assertEqual(projection["grants"][0]["path"], str(external.resolve()))
        self.assertEqual(projection["grants"][0]["access_level"], "write")
        self.assertEqual(projection["grants"][0]["duration_seconds"], 900)
        self.assertEqual(projection["grants"][0]["granted_by"], "mission-commander")

    def test_cli_denies_and_restores_contextual_additional_path_grant_request(
        self,
    ) -> None:
        external = self.root / "cli-denied-external"
        external.mkdir()
        common = [
            "--target-repo",
            str(self.target_repo),
            "--tracker-dir",
            str(self.tracker),
            "--runtime-root",
            str(self.runtime),
            "--mission-id",
            "command-deck",
        ]
        denial_output = io.StringIO()
        with redirect_stdout(denial_output):
            denial_exit = main(
                [
                    "additional-path-grant-deny",
                    *common,
                    "--correlation-id",
                    "path-grant-cli-denial-1",
                    "--request-id",
                    "contextual-grant-cli-1",
                    "--expected-terminal-revision",
                    "0",
                    "--path",
                    str(external),
                    "--access-level",
                    "read",
                    "--duration-seconds",
                    "300",
                    "--requester",
                    "mission-commander",
                    "--reason",
                    "The command requested external documentation.",
                    "--affected-action",
                    "python3 docs/check.py",
                ]
            )
        denial = json.loads(denial_output.getvalue())
        inspect_output = io.StringIO()
        with redirect_stdout(inspect_output):
            inspect_exit = main(["shell-terminal", *common])
        projection = json.loads(inspect_output.getvalue())

        self.assertEqual(denial_exit, 0)
        self.assertEqual(inspect_exit, 0)
        self.assertEqual(denial["denial_id"], "path-grant-denial-000001")
        self.assertEqual(denial["request_id"], "contextual-grant-cli-1")
        self.assertEqual(projection["revision"], 1)
        self.assertEqual(projection["grants"], [])
        self.assertEqual(projection["grant_denials"], [denial])

    def test_cli_approves_pending_human_required_shell_terminal_command(self) -> None:
        common = [
            "--target-repo",
            str(self.target_repo),
            "--tracker-dir",
            str(self.tracker),
            "--runtime-root",
            str(self.runtime),
            "--mission-id",
            "command-deck",
        ]
        pending_output = io.StringIO()
        with redirect_stdout(pending_output):
            pending_exit = main(
                [
                    "shell-terminal-submit",
                    *common,
                    "--correlation-id",
                    "terminal-cli-human-1",
                    "--command-text",
                    "python3 -c \"print('human approved')\"",
                    "--working-directory",
                    str(self.target_repo),
                    "--requester",
                    "mission-commander",
                ]
            )
        pending = json.loads(pending_output.getvalue())
        decision_output = io.StringIO()
        with redirect_stdout(decision_output):
            decision_exit = main(
                [
                    "shell-terminal-decision",
                    *common,
                    "--command-id",
                    pending["command_id"],
                    "--decision",
                    "approve",
                    "--actor",
                    "mission-commander",
                ]
            )
        approved = json.loads(decision_output.getvalue())

        self.assertEqual(pending_exit, 0)
        self.assertEqual(pending["status"], "pending-approval")
        self.assertEqual(pending["classification"], "human-required")
        self.assertEqual(decision_exit, 0)
        self.assertEqual(approved["status"], "completed")
        self.assertEqual(approved["stdout"].strip(), "human approved")

    def test_workspace_preferences_restore_after_service_restart(self) -> None:
        first = self.load_service()
        first.update_preferences(
            active_mission_id="command-deck",
            conversation_scope=ConversationScope(
                kind="issue-slice",
                target_id="ISS-01",
                label="Restore workspace session",
            ),
            operations_view="review-workspace",
        )

        restored = self.load_service().snapshot()

        self.assertEqual(restored.active_mission.id, "command-deck")
        self.assertEqual(restored.conversation_scope.kind, "issue-slice")
        self.assertEqual(restored.conversation_scope.target_id, "ISS-01")
        self.assertEqual(restored.operations_view, "review-workspace")

    def test_workstation_continuity_restores_meaningful_state_after_backend_restart(self) -> None:
        snapshots = self.load_service()
        mission = snapshots._primary_mission
        mission.approve_issue("ISS-01")
        AgentConsoleHistoryService(snapshots).append(
            role="user",
            content="Keep Alfredo continuity across restart.",
            outcome="proposed",
            source="mission-commander",
            expected_revision=1,
            expected_scope=snapshots.snapshot().conversation_scope,
        )
        launch = workspace_module.WorkstationActionService(snapshots).submit(
            correlation_id="continuity-launch-1",
            action_type="issue-launch",
            actor="mission-commander",
            expected_revision=1,
            target_kind="issue-slice",
            target_id="ISS-01",
            issue_id="ISS-01",
            allowed_paths=["src"],
        )
        evidence_link = f"app-local://evidence/{launch.session_id}"
        mission.record_evidence(
            launch.session_id,
            EvidencePackage(
                changed_files=["src/app.py"],
                diff_summary="Persisted workstation continuity.",
                commands_run=["python3 -m unittest tests.test_workspace_snapshot"],
                test_results="Focused persistence test passed.",
                known_risks="None.",
                proposed_context_updates="No domain model update.",
                artifact_links=[evidence_link],
            ),
        )
        proposal = WorkspaceQueueService(snapshots).propose_issue_contract_change(
            correlation_id="continuity-approval-1",
            expected_revision=launch.revision,
            issue_id="ISS-01",
            source="workstation-card",
            acceptance_criteria=["Continuity restores pending approval records."],
        )
        snapshots.update_preferences(
            active_mission_id="command-deck",
            conversation_scope=ConversationScope(
                kind="issue-slice",
                target_id="ISS-01",
                label="Restore workspace session",
            ),
            operations_view="workspace-queue",
            event_metadata={"correlation_id": "continuity-side-pane-1"},
        )

        restored = self.load_service()
        snapshot = restored.snapshot()
        restored_issue = snapshot.mission_board["issue_slices"][0]
        restored_queue = WorkspaceQueueService(restored).inspect()
        restored_history = AgentConsoleHistoryService(restored).history()
        restored_journal = ActivityJournalService(restored).inspect()

        self.assertEqual(snapshot.workspace_session.workspace_path, str(self.target_repo))
        self.assertEqual(snapshot.conversation_scope.kind, "issue-slice")
        self.assertEqual(snapshot.conversation_scope.target_id, "ISS-01")
        self.assertEqual(snapshot.operations_view, "workspace-queue")
        self.assertEqual(restored_history[0].content, "Keep Alfredo continuity across restart.")
        self.assertEqual(snapshot.missions[0].sessions[0].session_id, launch.session_id)
        self.assertEqual(snapshot.missions[0].sessions[0].status, "evidence-ready")
        self.assertEqual(restored_issue["evidence"]["artifact_links"], [evidence_link])
        self.assertEqual(restored_queue.items[0].item_id, proposal.item_id)
        self.assertEqual(restored_queue.items[0].status, "pending")
        self.assertEqual(
            [entry.action_type for entry in restored_journal.entries],
            ["issue-launch", "evidence-package-submitted"],
        )
        self.assertEqual(restored_journal.entries[1].evidence_links, (evidence_link,))
        self.assertNotIn("raw terminal byte", restored_journal.entries[0].summary)

    def test_review_workspace_lists_complete_and_incomplete_evidence_packages(self) -> None:
        mission = self.load_service()._primary_mission
        mission.approve_issue("ISS-01")
        complete_session = mission.launch_issue("ISS-01")
        mission.record_evidence(
            complete_session.session_id,
            EvidencePackage(
                changed_files=["src/app.py", ".env"],
                diff_summary="Added the review workspace projection.",
                commands_run=["python3 -m unittest tests.test_workspace_snapshot"],
                test_results="Focused workspace tests passed.",
                known_risks="None.",
                proposed_context_updates="Document Review Workspace as the evidence decision surface.",
                artifact_links=["app-local://evidence/session-ISS-01-1"],
            ),
        )
        incomplete_session = mission.launch_issue("ISS-01")
        incomplete_session.status = "failed"
        incomplete_session.runner_ended_at = "2026-07-11T10:00:00+00:00"
        mission._persist()

        projection = ReviewWorkspaceService(WorkspaceSnapshotService(mission)).inspect()
        items = {item.session_id: item for item in projection.items}

        self.assertEqual(projection.schema_version, 1)
        self.assertEqual(projection.revision, 1)
        self.assertEqual(set(items), {complete_session.session_id, incomplete_session.session_id})
        complete = items[complete_session.session_id]
        self.assertEqual(complete.issue_id, "ISS-01")
        self.assertEqual(complete.evidence_complete, True)
        self.assertEqual(complete.missing_evidence, [])
        self.assertEqual(complete.evidence.diff_summary, "Added the review workspace projection.")
        self.assertEqual(
            complete.evidence.proposed_context_updates,
            "Document Review Workspace as the evidence decision surface.",
        )
        self.assertEqual(complete.visibility_limitations[0].path, ".env")
        self.assertEqual(complete.visibility_limitations[0].classification, "Blocked")
        self.assertEqual(complete.can_accept, True)
        incomplete = items[incomplete_session.session_id]
        self.assertEqual(incomplete.evidence_complete, False)
        self.assertEqual(
            incomplete.missing_evidence,
            [
                "changed_files",
                "diff_summary",
                "commands_run",
                "test_results",
                "known_risks",
                "proposed_context_updates",
            ],
        )
        self.assertEqual(incomplete.can_accept, False)

    def test_evidence_rejects_unregistered_host_paths_and_projects_safe_links(self) -> None:
        snapshots = self.load_service()
        mission = snapshots._primary_mission
        mission.approve_issue("ISS-01")
        session = mission.launch_issue("ISS-01")
        evidence = EvidencePackage(
            changed_files=["src/app.py"],
            diff_summary="Added the bounded implementation.",
            commands_run=["python3 -m unittest tests.test_workspace_snapshot"],
            test_results="Focused tests passed.",
            known_risks="None.",
            proposed_context_updates="None.",
            artifact_links=["/etc/passwd"],
        )

        with self.assertRaisesRegex(EvidenceValidationError, "unsafe artifact link"):
            mission.record_evidence(session.session_id, evidence)

        self.assertEqual(mission.sessions[session.session_id].status, "queued")
        self.assertFalse(mission.sessions[session.session_id].evidence_valid)
        safe_link = f"app-local://evidence/{session.session_id}"
        evidence.artifact_links = [safe_link]
        mission.record_evidence(session.session_id, evidence)

        review = ReviewWorkspaceService(snapshots).inspect().items[0]
        board_session = mission.board_summary()["issue_slices"][0]["sessions"][0]
        journal = ActivityJournalService(snapshots).inspect().entries
        self.assertEqual(review.evidence.artifact_links, [safe_link])
        self.assertEqual(board_session["evidence"]["artifact_links"], [safe_link])
        self.assertEqual(journal[-1].evidence_links, (safe_link,))

    def test_session_artifact_reader_returns_only_bounded_registered_review_text(self) -> None:
        snapshots = self.load_service()
        mission = snapshots._primary_mission
        mission.approve_issue("ISS-01")
        session = mission.launch_issue("ISS-01")
        artifact_dir = mission.runtime_dir / "sessions" / session.session_id
        artifact_dir.mkdir(parents=True, exist_ok=True)
        review_diff = artifact_dir / "review.diff"
        review_diff.write_text("diff --git a/app.py b/app.py\n" + ("+safe line\n" * 20_000), encoding="utf-8")
        session.artifacts["review_diff"] = str(review_diff)
        session.evidence = EvidencePackage(
            changed_files=["app.py"],
            diff_summary=f"Review artifact at {review_diff}",
            commands_run=["python3 -m unittest"],
            test_results="Tests passed.",
            known_risks="None.",
            proposed_context_updates="None.",
            artifact_links=[str(review_diff)],
        )
        session.evidence_valid = True
        mission._persist_session_update(session)
        projected_ref = mission.review_artifact_links(session)[0]

        projection = SessionArtifactService(snapshots).read(
            mission_id=mission.mission_id,
            session_id=session.session_id,
            artifact_ref=projected_ref,
        )

        self.assertEqual(projection.schema_version, 1)
        self.assertEqual(projection.artifact_id, "review_diff")
        self.assertEqual(projection.media_type, "text/x-diff")
        self.assertTrue(projection.truncated)
        self.assertLessEqual(projection.byte_count, projection.content_limit_bytes)
        self.assertIn("diff --git", projection.content)
        payload = asdict(projection)
        self.assertNotIn(str(self.runtime), json.dumps(payload))
        self.assertNotIn("artifact_ref", payload)
        self.assertNotIn("path", payload)

    def test_session_artifact_reader_truncates_valid_utf8_at_a_codepoint_boundary(self) -> None:
        snapshots = self.load_service()
        mission = snapshots._primary_mission
        mission.approve_issue("ISS-01")
        session = mission.launch_issue("ISS-01")
        artifact_dir = mission.runtime_dir / "sessions" / session.session_id
        artifact_dir.mkdir(parents=True, exist_ok=True)
        review_diff = artifact_dir / "review.diff"
        review_diff.write_bytes((b"a" * 127_999) + "🙂".encode("utf-8") + b"tail")
        session.artifacts["review_diff"] = str(review_diff)
        mission._persist_session_update(session)

        projection = SessionArtifactService(snapshots).read(
            mission_id=mission.mission_id,
            session_id=session.session_id,
            artifact_ref=mission.review_artifact_reference(session, "review_diff"),
        )

        self.assertTrue(projection.truncated)
        self.assertEqual(projection.content_limit_bytes, 128_000)
        self.assertLessEqual(projection.byte_count, projection.content_limit_bytes)
        self.assertNotIn("\ufffd", projection.content)
        self.assertTrue(projection.content.endswith("a"))

    def test_session_artifact_reader_rejects_unregistered_and_escaped_host_paths(self) -> None:
        snapshots = self.load_service()
        mission = snapshots._primary_mission
        mission.approve_issue("ISS-01")
        session = mission.launch_issue("ISS-01")
        session.artifacts["review_diff"] = "/etc/passwd"
        mission._persist_session_update(session)
        reader = SessionArtifactService(snapshots)

        with self.assertRaises(SessionArtifactReadError) as escaped:
            reader.read(
                mission_id=mission.mission_id,
                session_id=session.session_id,
                artifact_ref="/etc/passwd",
            )
        artifact_dir = mission.runtime_dir / "sessions" / session.session_id
        artifact_dir.mkdir(parents=True, exist_ok=True)
        binary_artifact = artifact_dir / "review.diff"
        binary_artifact.write_bytes(b"\xff\xfe\x01")
        session.artifacts["review_diff"] = str(binary_artifact)
        mission._persist_session_update(session)
        with self.assertRaises(SessionArtifactReadError) as unsupported:
            reader.read(
                mission_id=mission.mission_id,
                session_id=session.session_id,
                artifact_ref=mission.review_artifact_reference(session, "review_diff"),
            )
        with self.assertRaises(SessionArtifactReadError) as unregistered:
            reader.read(
                mission_id=mission.mission_id,
                session_id=session.session_id,
                artifact_ref="app-local://evidence/not-this-session",
            )

        self.assertEqual(escaped.exception.code, "session-artifact-forbidden")
        self.assertNotIn("/etc/passwd", str(escaped.exception))
        self.assertEqual(unsupported.exception.code, "session-artifact-unsupported")
        self.assertEqual(unregistered.exception.code, "session-artifact-not-found")

    def test_session_artifact_reader_rejects_unsafe_bytes_after_displayed_prefix(
        self,
    ) -> None:
        snapshots = self.load_service()
        mission = snapshots._primary_mission
        mission.approve_issue("ISS-01")
        session = mission.launch_issue("ISS-01")
        artifact_dir = mission.runtime_dir / "sessions" / session.session_id
        artifact_dir.mkdir(parents=True, exist_ok=True)
        review_diff = artifact_dir / "review.diff"
        session.artifacts["review_diff"] = str(review_diff)
        mission._persist_session_update(session)
        artifact_ref = mission.review_artifact_reference(session, "review_diff")
        reader = SessionArtifactService(snapshots)

        for label, unsafe_tail in [
            ("NUL", b"\0unsafe-tail"),
            ("invalid UTF-8", b"\xffunsafe-tail"),
        ]:
            with self.subTest(label=label):
                review_diff.write_bytes(b"a" * 130_000 + unsafe_tail)
                with self.assertRaises(SessionArtifactReadError) as rejected:
                    reader.read(
                        mission_id=mission.mission_id,
                        session_id=session.session_id,
                        artifact_ref=artifact_ref,
                    )
                self.assertEqual(
                    rejected.exception.code,
                    "session-artifact-unsupported",
                )
                self.assertFalse(rejected.exception.recoverable)

    def test_session_artifact_reader_projects_registered_runtime_evidence_without_raw_links(self) -> None:
        snapshots = self.load_service()
        mission = snapshots._primary_mission
        mission.approve_issue("ISS-01")
        session = mission.launch_issue("ISS-01")
        evidence_ref = f"app-local://evidence/{session.session_id}"
        session.evidence = EvidencePackage(
            changed_files=["app.py"],
            diff_summary=f"Runtime summary under {mission.runtime_dir}.",
            commands_run=["python3 -m unittest"],
            test_results="Tests passed.",
            known_risks="None.",
            proposed_context_updates="None.",
            artifact_links=[evidence_ref],
        )
        session.evidence_valid = True
        mission._persist_session_update(session)

        projection = SessionArtifactService(snapshots).read(
            mission_id=mission.mission_id,
            session_id=session.session_id,
            artifact_ref=evidence_ref,
        )

        self.assertEqual(projection.artifact_id, "evidence-package")
        self.assertIn('"changed_files"', projection.content)
        self.assertNotIn(evidence_ref, projection.content)
        self.assertNotIn(str(mission.runtime_dir), projection.content)

    def test_cli_reads_registered_session_artifact_without_returning_a_host_path(self) -> None:
        snapshots = self.load_service()
        mission = snapshots._primary_mission
        mission.approve_issue("ISS-01")
        session = mission.launch_issue("ISS-01")
        artifact_dir = mission.runtime_dir / "sessions" / session.session_id
        artifact_dir.mkdir(parents=True, exist_ok=True)
        review_diff = artifact_dir / "review.diff"
        review_diff.write_text("--- a/app.py\n+++ b/app.py\n+fixed\n", encoding="utf-8")
        session.artifacts["review_diff"] = str(review_diff)
        session.evidence = EvidencePackage(
            changed_files=["app.py"],
            diff_summary="One safe change.",
            commands_run=["python3 -m unittest"],
            test_results="Tests passed.",
            known_risks="None.",
            proposed_context_updates="None.",
            artifact_links=[str(review_diff)],
        )
        session.evidence_valid = True
        mission._persist_session_update(session)
        artifact_ref = mission.review_artifact_links(session)[0]
        output = io.StringIO()

        with redirect_stdout(output):
            exit_code = main(
                [
                    "session-artifact",
                    "--target-repo",
                    str(self.target_repo),
                    "--tracker-dir",
                    str(self.tracker),
                    "--runtime-root",
                    str(self.runtime),
                    "--mission-id",
                    mission.mission_id,
                    "--artifact-mission-id",
                    mission.mission_id,
                    "--session-id",
                    session.session_id,
                    "--artifact-ref",
                    artifact_ref,
                ]
            )

        payload = json.loads(output.getvalue())
        self.assertEqual(exit_code, 0)
        self.assertEqual(payload["artifact_id"], "review_diff")
        self.assertIn("+fixed", payload["content"])
        self.assertNotIn(str(self.runtime), output.getvalue())

    def test_review_workspace_excludes_active_sessions_and_rejects_active_decisions(
        self,
    ) -> None:
        mission = self.load_service()._primary_mission
        mission.approve_issue("ISS-01")
        queued = mission.launch_issue("ISS-01")
        service = ReviewWorkspaceService(WorkspaceSnapshotService(mission))

        self.assertEqual(service.inspect().items, ())
        with self.assertRaisesRegex(AlbertError, "cannot be reviewed from queued"):
            service.decide(
                correlation_id="review-active-session-1",
                expected_revision=1,
                session_id=queued.session_id,
                decision="repair",
                reason="Do not race the active runner.",
            )
        self.assertEqual(mission.reviews, [])
        self.assertEqual(mission.sessions[queued.session_id].status, "queued")

    def test_review_decision_rechecks_runtime_state_atomically_against_runner_transition(
        self,
    ) -> None:
        mission = self.load_service()._primary_mission
        mission.approve_issue("ISS-01")
        session = mission.launch_issue("ISS-01")
        session.status = "failed"
        mission._persist()
        stale_service = ReviewWorkspaceService(WorkspaceSnapshotService(mission))

        runner_mission = self.load_service()._primary_mission
        running = runner_mission.sessions[session.session_id]
        running.status = "running"
        runner_mission._persist_session_update(
            running,
            expected_statuses={"failed"},
        )

        with self.assertRaisesRegex(AlbertError, "cannot be reviewed from running"):
            stale_service.decide(
                correlation_id="review-runner-race-1",
                expected_revision=1,
                session_id=session.session_id,
                decision="repair",
                reason="Do not overwrite a runner transition.",
            )

        persisted = json.loads(mission.runtime_path.read_text(encoding="utf-8"))
        self.assertEqual(
            persisted["sessions"][session.session_id]["status"],
            "running",
        )
        self.assertEqual(persisted["reviews"], [])

    def test_review_decision_replays_after_acknowledgement_write_failure(self) -> None:
        snapshots = self.load_service()
        mission = snapshots._primary_mission
        mission.approve_issue("ISS-01")
        session = mission.launch_issue("ISS-01")
        session.status = "failed"
        session.runner_ended_at = "2026-07-11T10:00:00+00:00"
        mission._persist()
        request = {
            "correlation_id": "review-replay-1",
            "expected_revision": 1,
            "session_id": session.session_id,
            "mission_id": mission.mission_id,
            "decision": "repair",
            "reason": "Retry the bounded implementation.",
        }
        original_write = WorkspaceSnapshotService._write_json_atomically

        def fail_acknowledgement(path: Path, payload: dict[str, object]) -> None:
            if path == snapshots.preferences_path:
                raise OSError("simulated Review acknowledgement failure")
            original_write(path, payload)

        with patch.object(
            WorkspaceSnapshotService,
            "_write_json_atomically",
            side_effect=fail_acknowledgement,
        ):
            with self.assertRaisesRegex(OSError, "Review acknowledgement failure"):
                ReviewWorkspaceService(snapshots).decide(**request)

        restored = self.load_service()
        acknowledgement = ReviewWorkspaceService(restored).decide(**request)
        replayed = ReviewWorkspaceService(restored).decide(**request)

        self.assertEqual(replayed, acknowledgement)
        self.assertEqual(acknowledgement.revision, 2)
        self.assertEqual(acknowledgement.review_outcome, "Needs repair")
        self.assertEqual(len(restored._primary_mission.reviews), 1)
        self.assertEqual(
            restored._primary_mission.sessions[session.session_id].status,
            "reviewed",
        )
        entries = ActivityJournalService(self.load_service()).inspect().entries
        self.assertEqual(
            [entry.correlation_id for entry in entries if entry.action_type == "review-decision"],
            ["review-replay-1"],
        )
        with self.assertRaisesRegex(AlbertError, "different request"):
            ReviewWorkspaceService(restored).decide(
                **{**request, "reason": "A changed review must not replay."}
            )

    def test_locked_issue_contract_edit_creates_queue_proposal_without_mutating_state(self) -> None:
        snapshots = self.load_service()
        mission = snapshots._primary_mission
        mission.approve_issue("ISS-01")
        accepted_criteria = list(mission.issues["ISS-01"].acceptance_criteria)

        acknowledgement = WorkspaceQueueService(snapshots).propose_issue_contract_change(
            correlation_id="proposal-acceptance-criteria-1",
            expected_revision=1,
            issue_id="ISS-01",
            source="issue-slice-inspector",
            acceptance_criteria=[
                "The canonical snapshot is visible.",
                "Queue proposals preserve accepted state until approval.",
            ],
        )
        reloaded = WorkspaceQueueService(self.load_service()).inspect()

        self.assertEqual(acknowledgement.outcome, "acknowledged")
        self.assertEqual(acknowledgement.item_id, "issue-change-command-deck-ISS-01-000001")
        self.assertEqual(mission.issues["ISS-01"].acceptance_criteria, accepted_criteria)
        self.assertEqual(reloaded.revision, acknowledgement.revision)
        self.assertEqual(len(reloaded.items), 1)
        proposal = reloaded.items[0]
        self.assertEqual(proposal.item_id, "issue-change-command-deck-ISS-01-000001")
        self.assertEqual(proposal.item_type, "issue-change-proposal")
        self.assertEqual(proposal.status, "pending")
        self.assertEqual(proposal.mission_id, "command-deck")
        self.assertEqual(proposal.issue_id, "ISS-01")
        self.assertEqual(proposal.source, "issue-slice-inspector")
        self.assertEqual(proposal.requested_action, "Change accepted Issue Slice contract")
        self.assertEqual(proposal.affected_boundary, "acceptance_criteria")
        self.assertEqual(
            proposal.consequence,
            "Approval will reopen ISS-01 for re-review with the proposed governed-field changes.",
        )
        self.assertEqual(
            proposal.proposed_changes["acceptance_criteria"],
            [
                "The canonical snapshot is visible.",
                "Queue proposals preserve accepted state until approval.",
            ],
        )

    def test_issue_change_proposal_replays_after_lost_persistence_response(self) -> None:
        snapshots = self.load_service()
        snapshots._primary_mission.approve_issue("ISS-01")
        queue = WorkspaceQueueService(snapshots)
        request = {
            "correlation_id": "issue-change-proposal-replay-1",
            "expected_revision": 1,
            "issue_id": "ISS-01",
            "source": "issue-slice-inspector",
            "acceptance_criteria": ["Persist one proposal across response loss."],
        }
        original_write = WorkspaceSnapshotService._write_json_atomically
        failed = False

        def persist_then_lose_response(path: Path, payload: dict[str, object]) -> None:
            nonlocal failed
            original_write(path, payload)
            if path == queue.queue_path and not failed:
                failed = True
                raise OSError("simulated Issue Change proposal response loss")

        with patch.object(
            WorkspaceSnapshotService,
            "_write_json_atomically",
            side_effect=persist_then_lose_response,
        ):
            with self.assertRaisesRegex(OSError, "Issue Change proposal response loss"):
                queue.propose_issue_contract_change(**request)

        replayed = WorkspaceQueueService(self.load_service()).propose_issue_contract_change(
            **request
        )
        replayed_again = WorkspaceQueueService(
            self.load_service()
        ).propose_issue_contract_change(**request)

        self.assertEqual(replayed, replayed_again)
        self.assertEqual(replayed.item_id, "issue-change-command-deck-ISS-01-000001")
        self.assertEqual(len(WorkspaceQueueService(self.load_service()).inspect().items), 1)
        with self.assertRaisesRegex(AlbertError, "different request"):
            WorkspaceQueueService(self.load_service()).propose_issue_contract_change(
                **{
                    **request,
                    "acceptance_criteria": ["A changed proposal must not replay."],
                }
            )

    def test_workspace_queue_approves_issue_change_proposal_and_reopens_slice(self) -> None:
        snapshots = self.load_service()
        mission = snapshots._primary_mission
        mission.approve_issue("ISS-01")
        service = WorkspaceQueueService(snapshots)
        proposal = service.propose_issue_contract_change(
            correlation_id="proposal-approve-1",
            expected_revision=1,
            issue_id="ISS-01",
            source="issue-slice-inspector",
            acceptance_criteria=["Approval applies this proposal."],
        )

        acknowledgement = service.decide(
            correlation_id="proposal-approve-decision-1",
            expected_revision=proposal.revision,
            item_id=proposal.item_id,
            decision="approve",
            reason="Accepted by Mission Commander.",
        )
        projected = service.inspect().items[0]

        self.assertEqual(acknowledgement.outcome, "acknowledged")
        self.assertEqual(acknowledgement.item_status, "approved")
        self.assertEqual(acknowledgement.revision, proposal.revision + 1)
        self.assertEqual(projected.status, "approved")
        self.assertEqual(mission.issues["ISS-01"].acceptance_criteria, ["Approval applies this proposal."])
        self.assertEqual(mission.issues["ISS-01"].review_state, "needs-review")
        self.assertEqual(mission.issues["ISS-01"].status, "needs-review")
        self.assertEqual(mission.issues["ISS-01"].locked, False)

    def test_issue_change_approval_refreshes_runtime_before_mutating_governed_fields(
        self,
    ) -> None:
        stale_snapshots = self.load_service()
        stale_mission = stale_snapshots._primary_mission
        stale_mission.approve_issue("ISS-01")
        stale_queue = WorkspaceQueueService(stale_snapshots)
        proposal = stale_queue.propose_issue_contract_change(
            correlation_id="stale-issue-change-proposal-1",
            expected_revision=1,
            issue_id="ISS-01",
            source="issue-slice-inspector",
            acceptance_criteria=["Apply only the governed criteria change."],
        )

        writer = self.load_service()._primary_mission
        writer.assign_issue(
            "ISS-01",
            "qwen-coder-local-1",
            notes="A newer process recorded this assignment and note.",
        )

        stale_queue.decide(
            correlation_id="stale-issue-change-decision-1",
            expected_revision=proposal.revision,
            item_id=proposal.item_id,
            decision="approve",
            reason="Apply the exact governed-field proposal.",
        )

        restored = self.load_service()._primary_mission.issues["ISS-01"]
        self.assertEqual(restored.assigned_agent, "qwen-coder-local-1")
        self.assertEqual(
            restored.notes,
            "A newer process recorded this assignment and note.",
        )
        self.assertEqual(
            restored.acceptance_criteria,
            ["Apply only the governed criteria change."],
        )

    def test_issue_change_retry_does_not_reopen_after_lost_queue_write(
        self,
    ) -> None:
        snapshots = self.load_service()
        snapshots._primary_mission.approve_issue("ISS-01")
        queue = WorkspaceQueueService(snapshots)
        proposal = queue.propose_issue_contract_change(
            correlation_id="issue-change-lost-write-proposal-1",
            expected_revision=1,
            issue_id="ISS-01",
            source="issue-slice-inspector",
            acceptance_criteria=["Persist the governed change exactly once."],
        )
        decision = {
            "correlation_id": "issue-change-lost-write-decision-1",
            "expected_revision": proposal.revision,
            "item_id": proposal.item_id,
            "decision": "approve",
            "reason": "Apply once even when the queue response is lost.",
        }
        original_write = WorkspaceSnapshotService._write_json_atomically
        failed = False

        def fail_queue_write(path: Path, payload: dict[str, object]) -> None:
            nonlocal failed
            if path == queue.queue_path and not failed:
                failed = True
                raise OSError("simulated queue write failure after Issue mutation")
            original_write(path, payload)

        with patch.object(
            WorkspaceSnapshotService,
            "_write_json_atomically",
            side_effect=fail_queue_write,
        ):
            with self.assertRaisesRegex(OSError, "after Issue mutation"):
                queue.decide(**decision)

        with self.assertRaisesRegex(
            WorkspacePersistenceError,
            "approval effect is already durable",
        ):
            WorkspaceQueueService(self.load_service()).decide(
                correlation_id="issue-change-contradictory-reject-1",
                expected_revision=proposal.revision,
                item_id=proposal.item_id,
                decision="reject",
                reason="A later request must not contradict the durable effect.",
            )
        WorkspaceQueueService(self.load_service()).request_frontier_confirmation(
            correlation_id="advance-after-issue-effect-1",
            expected_revision=1,
            issue_id="ISS-01",
            source="frontier-model",
            requested_action="Inspect an unrelated boundary",
            affected_boundary="unrelated",
            consequence="No change to the recovered Issue approval.",
            payload={"note": "advance queue revision"},
        )

        reloaded = self.load_service()
        reloaded._primary_mission.approve_issue("ISS-01")
        acknowledgement = WorkspaceQueueService(reloaded).decide(**decision)

        restored = self.load_service()._primary_mission.issues["ISS-01"]
        self.assertEqual(acknowledgement.item_status, "approved")
        self.assertEqual(restored.review_state, "approved")
        self.assertEqual(restored.status, "approved")
        self.assertTrue(restored.locked)
        self.assertEqual(
            restored.acceptance_criteria,
            ["Persist the governed change exactly once."],
        )

    def test_workspace_queue_rejects_duplicate_persisted_item_ids(self) -> None:
        snapshots = self.load_service()
        snapshots._primary_mission.approve_issue("ISS-01")
        queue = WorkspaceQueueService(snapshots)
        queue.propose_issue_contract_change(
            correlation_id="duplicate-queue-item-proposal-1",
            expected_revision=1,
            issue_id="ISS-01",
            source="issue-slice-inspector",
            risk="Keep queue item identities unique.",
        )
        payload = json.loads(queue.queue_path.read_text(encoding="utf-8"))
        payload["items"].append(dict(payload["items"][0]))
        queue.queue_path.write_text(json.dumps(payload), encoding="utf-8")

        with self.assertRaisesRegex(
            WorkspacePersistenceError,
            "item ids must be unique",
        ):
            WorkspaceQueueService(self.load_service()).inspect()

    def test_workspace_queue_rejects_forged_projected_receipt_identities(self) -> None:
        snapshots = self.load_service()
        snapshots._primary_mission.approve_issue("ISS-01")
        queue = WorkspaceQueueService(snapshots)
        proposal = queue.propose_issue_contract_change(
            correlation_id="projected-receipt-proposal-1",
            expected_revision=1,
            issue_id="ISS-01",
            source="issue-slice-inspector",
            risk="Receipt identities remain bound to canonical records.",
        )
        queue.decide(
            correlation_id="projected-receipt-decision-1",
            expected_revision=proposal.revision,
            item_id=proposal.item_id,
            decision="reject",
            reason="Keep the accepted boundary unchanged.",
        )
        persisted = json.loads(queue.queue_path.read_text(encoding="utf-8"))

        for field_name, forged_value in (
            ("proposal_correlation_id", "projected-receipt-decision-1"),
            ("decision_correlation_id", "projected-receipt-proposal-1"),
        ):
            with self.subTest(field_name=field_name):
                forged = json.loads(json.dumps(persisted))
                forged["items"][0][field_name] = forged_value
                queue.queue_path.write_text(json.dumps(forged), encoding="utf-8")
                with self.assertRaisesRegex(
                    WorkspacePersistenceError,
                    "receipt identity",
                ):
                    WorkspaceQueueService(self.load_service()).inspect()

    def test_workspace_queue_backfills_legacy_projected_receipt_identities(self) -> None:
        snapshots = self.load_service()
        snapshots._primary_mission.approve_issue("ISS-01")
        queue = WorkspaceQueueService(snapshots)
        proposal = queue.propose_issue_contract_change(
            correlation_id="legacy-projected-proposal-1",
            expected_revision=1,
            issue_id="ISS-01",
            source="issue-slice-inspector",
            risk="Backfill only from the exact canonical receipt chain.",
        )
        queue.decide(
            correlation_id="legacy-projected-decision-1",
            expected_revision=proposal.revision,
            item_id=proposal.item_id,
            decision="reject",
            reason="Exercise a resolved legacy projection.",
        )
        persisted = json.loads(queue.queue_path.read_text(encoding="utf-8"))
        persisted["items"][0].pop("proposal_correlation_id")
        persisted["items"][0].pop("decision_correlation_id")
        queue.queue_path.write_text(json.dumps(persisted), encoding="utf-8")

        item = WorkspaceQueueService(self.load_service()).inspect().items[0]

        self.assertEqual(item.proposal_correlation_id, "legacy-projected-proposal-1")
        self.assertEqual(item.decision_correlation_id, "legacy-projected-decision-1")

    def test_queue_replay_rejects_forged_inner_acknowledgement_boundaries(
        self,
    ) -> None:
        snapshots = self.load_service()
        snapshots._primary_mission.approve_issue("ISS-01")
        queue = WorkspaceQueueService(snapshots)
        proposal = queue.propose_issue_contract_change(
            correlation_id="forged-ack-proposal-1",
            expected_revision=1,
            issue_id="ISS-01",
            source="issue-slice-inspector",
            acceptance_criteria=["Reject forged receipt acknowledgements."],
        )
        queue.decide(
            correlation_id="forged-ack-decision-1",
            expected_revision=proposal.revision,
            item_id=proposal.item_id,
            decision="reject",
            reason="Keep the accepted contract unchanged.",
        )
        persisted = queue._load_queue()
        receipt = next(
            item
            for item in persisted["receipts"]
            if item["correlation_id"] == "forged-ack-decision-1"
        )

        for label, forged_fields in (
            ("outer correlation", {"correlation_id": "forged-inner-correlation"}),
            ("unrelated session", {"session_id": "session-ISS-01-unrelated"}),
            ("noncanonical revision", {"revision": 1}),
            ("future revision", {"revision": persisted["revision"] + 1}),
            ("forged effect", {"effect_summary": "Launch an unrelated session."}),
        ):
            with self.subTest(label=label):
                forged_receipt = {
                    **receipt,
                    "acknowledgement": {
                        **receipt["acknowledgement"],
                        **forged_fields,
                    },
                }
                forged_queue = {
                    **persisted,
                    "receipts": [
                        forged_receipt if item is receipt else item
                        for item in persisted["receipts"]
                    ],
                }
                with self.assertRaisesRegex(
                    WorkspacePersistenceError,
                    "Workspace Queue",
                ):
                    queue._replay_queue_request(
                        forged_queue,
                        correlation_id="forged-ack-decision-1",
                        request_kind="workspace-queue-decision",
                        request_payload=receipt["request"],
                    )

    def test_queue_replay_binds_proposal_receipt_to_its_canonical_item(self) -> None:
        snapshots = self.load_service()
        snapshots._primary_mission.approve_issue("ISS-01")
        queue = WorkspaceQueueService(snapshots)
        queue.propose_issue_contract_change(
            correlation_id="canonical-proposal-receipt-1",
            expected_revision=1,
            issue_id="ISS-01",
            source="issue-slice-inspector",
            acceptance_criteria=["Bind the durable request to its queue item."],
        )
        persisted = queue._load_queue()
        receipt = persisted["receipts"][0]

        for label, request_change in (
            ("mission", {"mission_id": "unrelated-mission"}),
            ("issue", {"issue_id": "ISS-99"}),
            ("source", {"source": "forged-source"}),
            (
                "proposed changes",
                {"proposed_changes": {"risk": "forged canonical mutation"}},
            ),
        ):
            with self.subTest(label=label):
                forged_request = {**receipt["request"], **request_change}
                forged_receipt = {**receipt, "request": forged_request}
                forged_queue = {
                    **persisted,
                    "receipts": [forged_receipt],
                }
                with self.assertRaisesRegex(
                    WorkspacePersistenceError,
                    "Workspace Queue",
                ):
                    queue._replay_queue_request(
                        forged_queue,
                        correlation_id="canonical-proposal-receipt-1",
                        request_kind="issue-change-proposal",
                        request_payload=forged_request,
                    )

    def test_workspace_queue_rejects_and_stale_decisions_preserve_authoritative_state(self) -> None:
        snapshots = self.load_service()
        mission = snapshots._primary_mission
        mission.approve_issue("ISS-01")
        service = WorkspaceQueueService(snapshots)
        accepted_criteria = list(mission.issues["ISS-01"].acceptance_criteria)
        rejected = service.propose_issue_contract_change(
            correlation_id="proposal-reject-1",
            expected_revision=1,
            issue_id="ISS-01",
            source="issue-slice-inspector",
            acceptance_criteria=["Rejected proposal must not apply."],
        )

        rejection = service.decide(
            correlation_id="proposal-reject-decision-1",
            expected_revision=rejected.revision,
            item_id=rejected.item_id,
            decision="reject",
            reason="Does not match the accepted mission boundary.",
        )
        stale = service.propose_issue_contract_change(
            correlation_id="proposal-stale-1",
            expected_revision=1,
            issue_id="ISS-01",
            source="agent-console",
            acceptance_criteria=["Stale approval must not apply."],
        )
        service.propose_issue_contract_change(
            correlation_id="proposal-stale-bump-1",
            expected_revision=1,
            issue_id="ISS-01",
            source="background-attention",
            acceptance_criteria=["This proposal only bumps the queue revision."],
        )

        with self.assertRaises(WorkspaceStaleActionError):
            service.decide(
                correlation_id="proposal-stale-decision-1",
                expected_revision=stale.revision,
                item_id=stale.item_id,
                decision="approve",
                reason="Stale approval.",
            )

        projected = {item.item_id: item for item in service.inspect().items}
        self.assertEqual(rejection.item_status, "rejected")
        self.assertEqual(projected[rejected.item_id].status, "rejected")
        self.assertEqual(projected[stale.item_id].status, "pending")
        self.assertEqual(mission.issues["ISS-01"].acceptance_criteria, accepted_criteria)
        self.assertEqual(mission.issues["ISS-01"].review_state, "approved")
        self.assertEqual(mission.issues["ISS-01"].locked, True)

    def test_workspace_queue_decision_accepts_typed_workstation_metadata(self) -> None:
        snapshots = self.load_service()
        mission = snapshots._primary_mission
        mission.approve_issue("ISS-01")
        service = WorkspaceQueueService(snapshots)
        proposal = service.propose_issue_contract_change(
            correlation_id="proposal-workstation-typed-1",
            expected_revision=1,
            issue_id="ISS-01",
            source="workstation-card",
            acceptance_criteria=["Route decisions from the workstation card."],
        )

        acknowledgement = service.decide(
            correlation_id="proposal-workstation-typed-decision-1",
            action_type="workspace-queue-decision",
            actor="mission-commander",
            expected_revision=proposal.revision,
            target_kind="workspace-queue-item",
            target_id=proposal.item_id,
            item_id=proposal.item_id,
            decision="approve",
            reason="Approved from the workstation card.",
        )

        self.assertEqual(acknowledgement.item_status, "approved")
        entry = ActivityJournalService(self.load_service()).inspect().entries[0]
        self.assertEqual(entry.actor, "mission-commander")
        self.assertEqual(entry.action_type, "workspace-queue-decision")
        self.assertEqual(entry.correlation_id, "proposal-workstation-typed-decision-1")

    def test_workspace_queue_decision_rejects_mismatched_workstation_target(self) -> None:
        snapshots = self.load_service()
        mission = snapshots._primary_mission
        mission.approve_issue("ISS-01")
        service = WorkspaceQueueService(snapshots)
        proposal = service.propose_issue_contract_change(
            correlation_id="proposal-workstation-invalid-1",
            expected_revision=1,
            issue_id="ISS-01",
            source="workstation-card",
            acceptance_criteria=["Reject mismatched workstation targets."],
        )

        with self.assertRaisesRegex(AlbertError, "target id must match item id"):
            service.decide(
                correlation_id="proposal-workstation-invalid-decision-1",
                action_type="workspace-queue-decision",
                actor="mission-commander",
                expected_revision=proposal.revision,
                target_kind="workspace-queue-item",
                target_id="another-item",
                item_id=proposal.item_id,
                decision="approve",
                reason="This must not apply.",
            )

        reloaded = WorkspaceQueueService(self.load_service()).inspect()
        self.assertEqual(reloaded.items[0].status, "pending")

    def test_workstation_action_launches_issue_through_expected_revision_guard(self) -> None:
        (self.target_repo / ".albert").mkdir()
        (self.target_repo / ".albert" / "agents.json").write_text(
            json.dumps(
                {
                    "agents": [
                        {
                            "id": "qwen-coder-local-1",
                            "role": "local-agent",
                            "provider": "test-harness",
                            "runner": "fake",
                        }
                    ]
                }
            ),
            encoding="utf-8",
        )
        snapshots = self.load_service()
        mission = snapshots._primary_mission
        mission.approve_issue("ISS-01")

        with patch.object(
            mission,
            "_run_fake_agent",
            side_effect=AssertionError("acknowledgement must return before runner work"),
        ):
            acknowledgement = workspace_module.WorkstationActionService(snapshots).submit(
                correlation_id="workstation-launch-1",
                action_type="issue-launch",
                actor="mission-commander",
                expected_revision=1,
                target_kind="issue-slice",
                target_id="ISS-01",
                issue_id="ISS-01",
                allowed_paths=["src"],
            )

        self.assertEqual(acknowledgement.action_type, "issue-launch")
        self.assertEqual(acknowledgement.issue_id, "ISS-01")
        self.assertEqual(acknowledgement.session_id, "session-ISS-01-1")
        self.assertEqual(acknowledgement.revision, 2)
        self.assertIn("queued", acknowledgement.effect_summary)
        self.assertIn("session-ISS-01-1", mission.sessions)
        self.assertEqual(mission.sessions["session-ISS-01-1"].status, "queued")
        self.assertEqual(mission.sessions["session-ISS-01-1"].task_packet["allowed_paths"], ["src"])
        entry = ActivityJournalService(self.load_service()).inspect().entries[0]
        self.assertEqual(entry.actor, "orchestrator")
        self.assertEqual(entry.action_type, "issue-launch")
        self.assertEqual(entry.correlation_id, "workstation-launch-1")

    def test_live_agent_availability_snapshot_gates_projection_launch_and_runner_claim(
        self,
    ) -> None:
        agent_config = self.root / "availability-agents.json"
        agent_config.write_text(
            json.dumps(
                {
                    "agents": [
                        {
                            "id": "ollama-worker",
                            "role": "local-agent",
                            "provider": "ollama",
                            "runner": "ollama",
                            "model": "missing:test",
                            "routing": "worker",
                        }
                    ]
                }
            ),
            encoding="utf-8",
        )
        available = AlbertMission(
            target_repo=self.target_repo,
            tracker_dir=self.tracker,
            runtime_root=self.runtime,
            mission_id="availability-probe",
            agent_config_path=agent_config,
            allow_empty_tracker=True,
            agent_availability_snapshot={"ollama-worker": ("available", "")},
        ).load()
        available.assign_issue("ISS-01", "ollama-worker")
        available.approve_issue("ISS-01")
        queued = available.launch_issue("ISS-01", allowed_paths=["src"])

        disconnected = AlbertMission(
            target_repo=self.target_repo,
            tracker_dir=self.tracker,
            runtime_root=self.runtime,
            mission_id="availability-probe",
            agent_config_path=agent_config,
            allow_empty_tracker=True,
            agent_availability_snapshot={
                "ollama-worker": ("disconnected", "Ollama probe is offline")
            },
        ).load()
        assignment = disconnected.board_summary()["issue_slices"][0]["model_assignment"]
        self.assertEqual(assignment["availability"], "disconnected")
        self.assertIn("Ollama probe is offline", assignment["availability_reason"])
        with self.assertRaisesRegex(AlbertError, "assigned model is unavailable"):
            disconnected.run_session(queued.session_id)
        self.assertEqual(
            AlbertMission(
                target_repo=self.target_repo,
                tracker_dir=self.tracker,
                runtime_root=self.runtime,
                mission_id="availability-probe",
                agent_config_path=agent_config,
                allow_empty_tracker=True,
                agent_availability_snapshot={
                    "ollama-worker": ("disconnected", "Ollama probe is offline")
                },
            ).load().sessions[queued.session_id].status,
            "failed",
        )

        fresh_runtime = self.root / "fresh-availability-runtime"
        blocked = AlbertMission(
            target_repo=self.target_repo,
            tracker_dir=self.tracker,
            runtime_root=fresh_runtime,
            mission_id="availability-blocked",
            agent_config_path=agent_config,
            allow_empty_tracker=True,
            agent_availability_snapshot={
                "ollama-worker": ("unavailable", "Model is not installed")
            },
        ).load()
        blocked.assign_issue("ISS-01", "ollama-worker")
        blocked.approve_issue("ISS-01")
        with self.assertRaisesRegex(AlbertError, "Model is not installed"):
            blocked.launch_issue("ISS-01")

    def test_workstation_action_approves_ready_for_agent_ticket_for_governed_launch(self) -> None:
        snapshots = self.load_service()
        mission = snapshots._primary_mission

        mission.issues["ISS-01"].tracker_status = "ready-for-human"
        with self.assertRaisesRegex(AlbertError, "not ready for agent approval"):
            workspace_module.WorkstationActionService(snapshots).submit(
                correlation_id="workstation-approve-human-1",
                action_type="issue-approve",
                actor="mission-commander",
                expected_revision=1,
                target_kind="issue-slice",
                target_id="ISS-01",
                issue_id="ISS-01",
            )
        mission.issues["ISS-01"].tracker_status = "ready-for-agent"

        acknowledgement = workspace_module.WorkstationActionService(snapshots).submit(
            correlation_id="workstation-approve-1",
            action_type="issue-approve",
            actor="mission-commander",
            expected_revision=1,
            target_kind="issue-slice",
            target_id="ISS-01",
            issue_id="ISS-01",
        )

        self.assertEqual(acknowledgement.action_type, "issue-approve")
        self.assertEqual(acknowledgement.revision, 2)
        self.assertEqual(mission.issues["ISS-01"].review_state, "approved")
        self.assertIn("ISS-01", snapshots.snapshot().mission_board["ready_issue_ids"])
        self.assertIn("approved", acknowledgement.effect_summary)
        entry = ActivityJournalService(self.load_service()).inspect().entries[0]
        self.assertEqual(entry.action_type, "issue-approve")

    def test_workstation_action_rejects_stale_launch_without_mutating_sessions(self) -> None:
        snapshots = self.load_service()
        mission = snapshots._primary_mission
        mission.approve_issue("ISS-01")
        WorkspaceSyncService(snapshots).submit_action(
            WorkspaceAction(
                correlation_id="revision-bump-before-launch",
                expected_revision=1,
                active_mission_id="command-deck",
                conversation_scope=snapshots.snapshot().conversation_scope,
                operations_view="mission-board",
            )
        )

        with self.assertRaises(WorkspaceStaleActionError):
            workspace_module.WorkstationActionService(snapshots).submit(
                correlation_id="workstation-launch-stale-1",
                action_type="issue-launch",
                actor="mission-commander",
                expected_revision=1,
                target_kind="issue-slice",
                target_id="ISS-01",
                issue_id="ISS-01",
            )

        self.assertEqual(mission.sessions, {})
        restored_history = AgentConsoleHistoryService(self.load_service()).history()
        stale_turns = [
            message
            for message in restored_history
            if message.correlation_id == "workstation-launch-stale-1"
        ]
        self.assertEqual(
            [message.action_phase for message in stale_turns],
            ["request", "rejection"],
        )
        self.assertEqual(stale_turns[-1].source, "orchestrator")
        self.assertEqual(stale_turns[-1].outcome, "rejected")
        self.assertIn("expected revision 1", stale_turns[-1].content)

    def test_workstation_launch_retry_reuses_correlation_after_acknowledgement_write_failure(
        self,
    ) -> None:
        snapshots = self.load_service()
        mission = snapshots._primary_mission
        mission.approve_issue("ISS-01")
        stale_restarted = self.load_service()
        original_write = WorkspaceSnapshotService._write_json_atomically

        def fail_acknowledgement_write(path: Path, data: dict[str, object]) -> None:
            if path == snapshots.preferences_path:
                raise OSError("simulated acknowledgement persistence failure")
            original_write(path, data)

        with patch.object(
            WorkspaceSnapshotService,
            "_write_json_atomically",
            side_effect=fail_acknowledgement_write,
        ):
            with self.assertRaisesRegex(OSError, "acknowledgement persistence failure"):
                WorkstationActionService(snapshots).submit(
                    correlation_id="idempotent-workstation-launch-1",
                    action_type="issue-launch",
                    actor="mission-commander",
                    expected_revision=1,
                    target_kind="issue-slice",
                    target_id="ISS-01",
                    issue_id="ISS-01",
                    allowed_paths=["src"],
                )

        self.assertEqual(list(self.load_service()._primary_mission.sessions), ["session-ISS-01-1"])
        acknowledgement = WorkstationActionService(stale_restarted).submit(
            correlation_id="idempotent-workstation-launch-1",
            action_type="issue-launch",
            actor="mission-commander",
            expected_revision=1,
            target_kind="issue-slice",
            target_id="ISS-01",
            issue_id="ISS-01",
            allowed_paths=["src"],
        )

        restored = self.load_service()
        self.assertEqual(acknowledgement.session_id, "session-ISS-01-1")
        self.assertEqual(restored.snapshot().revision, 2)
        self.assertEqual(list(restored._primary_mission.sessions), ["session-ISS-01-1"])
        with self.assertRaisesRegex(AlbertError, "different request boundary"):
            WorkstationActionService(restored).submit(
                correlation_id="idempotent-workstation-launch-1",
                action_type="issue-launch",
                actor="mission-commander",
                expected_revision=2,
                target_kind="issue-slice",
                target_id="ISS-01",
                issue_id="ISS-01",
                allowed_paths=["different-boundary"],
            )

    def test_workstation_approve_recovers_after_acknowledgement_write_failure(
        self,
    ) -> None:
        snapshots = self.load_service()
        request = {
            "correlation_id": "idempotent-workstation-approve-1",
            "action_type": "issue-approve",
            "actor": "mission-commander",
            "expected_revision": 1,
            "target_kind": "issue-slice",
            "target_id": "ISS-01",
            "issue_id": "ISS-01",
        }
        original_write = WorkspaceSnapshotService._write_json_atomically

        def fail_acknowledgement_write(path: Path, data: dict[str, object]) -> None:
            if path == snapshots.preferences_path:
                raise OSError("simulated approval acknowledgement failure")
            original_write(path, data)

        with patch.object(
            WorkspaceSnapshotService,
            "_write_json_atomically",
            side_effect=fail_acknowledgement_write,
        ):
            with self.assertRaisesRegex(OSError, "approval acknowledgement failure"):
                WorkstationActionService(snapshots).submit(**request)

        restored = self.load_service()
        self.assertEqual(restored._primary_mission.issues["ISS-01"].review_state, "approved")
        with self.assertRaisesRegex(AlbertError, "different request boundary"):
            WorkstationActionService(restored).submit(
                **{**request, "allowed_paths": ["different-boundary"]}
            )

        acknowledgement = WorkstationActionService(self.load_service()).submit(**request)
        replayed = WorkstationActionService(self.load_service()).submit(**request)

        self.assertEqual(replayed, acknowledgement)
        self.assertEqual(acknowledgement.revision, 2)
        final = self.load_service()
        self.assertEqual(
            final._primary_mission.timeline.count("ISS-01 approved and locked."),
            1,
        )
        self.assertEqual(len(ActivityJournalService(final).inspect().entries), 1)
        self.assertEqual(
            [
                message.action_phase
                for message in AgentConsoleHistoryService(final).history()
                if message.correlation_id == request["correlation_id"]
            ],
            ["request", "rejection", "acknowledgement"],
        )

    def test_workstation_assignment_recovers_after_acknowledgement_write_failure(
        self,
    ) -> None:
        snapshots = self.load_service()
        request = {
            "correlation_id": "idempotent-workstation-assignment-1",
            "action_type": "model-assignment-change",
            "actor": "mission-commander",
            "expected_revision": 1,
            "target_kind": "issue-slice",
            "target_id": "ISS-01",
            "issue_id": "ISS-01",
            "agent_id": "qwen3.6-27b",
            "reason": "Use the stronger local model.",
        }
        original_write = WorkspaceSnapshotService._write_json_atomically

        def fail_acknowledgement_write(path: Path, data: dict[str, object]) -> None:
            if path == snapshots.preferences_path:
                raise OSError("simulated assignment acknowledgement failure")
            original_write(path, data)

        with patch.object(
            WorkspaceSnapshotService,
            "_write_json_atomically",
            side_effect=fail_acknowledgement_write,
        ):
            with self.assertRaisesRegex(OSError, "assignment acknowledgement failure"):
                WorkstationActionService(snapshots).submit(**request)

        restored = self.load_service()
        self.assertEqual(
            restored._primary_mission.issues["ISS-01"].assigned_agent,
            "qwen3.6-27b",
        )
        with self.assertRaisesRegex(AlbertError, "different request boundary"):
            WorkstationActionService(restored).submit(
                **{
                    **request,
                    "agent_id": "different-worker",
                    "reason": "This changed boundary must not be accepted.",
                }
            )
        self.assertEqual(
            self.load_service()._primary_mission.issues["ISS-01"].assigned_agent,
            "qwen3.6-27b",
        )

        acknowledgement = WorkstationActionService(self.load_service()).submit(**request)
        replayed = WorkstationActionService(self.load_service()).submit(**request)

        self.assertEqual(replayed, acknowledgement)
        final = self.load_service()
        self.assertEqual(
            final._primary_mission.timeline.count("ISS-01 assigned to qwen3.6-27b."),
            1,
        )
        self.assertEqual(len(ActivityJournalService(final).inspect().entries), 1)
        self.assertEqual(
            [
                message.action_phase
                for message in AgentConsoleHistoryService(final).history()
                if message.correlation_id == request["correlation_id"]
            ],
            ["request", "rejection", "acknowledgement"],
        )

    def test_workstation_cancel_recovers_after_acknowledgement_write_failure(
        self,
    ) -> None:
        snapshots = self.load_service()
        mission = snapshots._primary_mission
        mission.approve_issue("ISS-01")
        session = mission.launch_issue("ISS-01")
        request = {
            "correlation_id": "idempotent-workstation-cancel-1",
            "action_type": "session-cancel",
            "actor": "mission-commander",
            "expected_revision": 1,
            "target_kind": "agent-session",
            "target_id": session.session_id,
            "issue_id": "ISS-01",
            "session_id": session.session_id,
            "reason": "Stop this queued runner.",
        }
        original_write = WorkspaceSnapshotService._write_json_atomically

        def fail_acknowledgement_write(path: Path, data: dict[str, object]) -> None:
            if path == snapshots.preferences_path:
                raise OSError("simulated cancellation acknowledgement failure")
            original_write(path, data)

        with patch.object(
            WorkspaceSnapshotService,
            "_write_json_atomically",
            side_effect=fail_acknowledgement_write,
        ):
            with self.assertRaisesRegex(OSError, "cancellation acknowledgement failure"):
                WorkstationActionService(snapshots).submit(**request)

        restored = self.load_service()
        self.assertEqual(
            restored._primary_mission.sessions[session.session_id].status,
            "cancelled",
        )
        with self.assertRaisesRegex(AlbertError, "different request boundary"):
            WorkstationActionService(restored).submit(
                **{**request, "reason": "A changed cancellation reason."}
            )

        acknowledgement = WorkstationActionService(self.load_service()).submit(**request)
        replayed = WorkstationActionService(self.load_service()).submit(**request)

        self.assertEqual(replayed, acknowledgement)
        final = self.load_service()
        cancellation_entry = (
            f"ISS-01 session {session.session_id} cancelled: Stop this queued runner."
        )
        self.assertEqual(final._primary_mission.timeline.count(cancellation_entry), 1)
        self.assertEqual(len(ActivityJournalService(final).inspect().entries), 1)
        self.assertEqual(
            [
                message.action_phase
                for message in AgentConsoleHistoryService(final).history()
                if message.correlation_id == request["correlation_id"]
            ],
            ["request", "rejection", "acknowledgement"],
        )

    def test_workstation_rejects_unreceipted_correlation_collision_across_action_types(
        self,
    ) -> None:
        snapshots = self.load_service()
        mission = snapshots._primary_mission
        mission.approve_issue("ISS-01")
        original_assignment = mission.issues["ISS-01"].assigned_agent
        original_write = WorkspaceSnapshotService._write_json_atomically

        def fail_acknowledgement_write(path: Path, data: dict[str, object]) -> None:
            if path == snapshots.preferences_path:
                raise OSError("simulated launch acknowledgement failure")
            original_write(path, data)

        with patch.object(
            WorkspaceSnapshotService,
            "_write_json_atomically",
            side_effect=fail_acknowledgement_write,
        ):
            with self.assertRaisesRegex(OSError, "launch acknowledgement failure"):
                WorkstationActionService(snapshots).submit(
                    correlation_id="unreceipted-cross-action-collision-1",
                    action_type="issue-launch",
                    actor="mission-commander",
                    expected_revision=1,
                    target_kind="issue-slice",
                    target_id="ISS-01",
                    issue_id="ISS-01",
                    allowed_paths=["src"],
                )

        restored = self.load_service()
        with self.assertRaisesRegex(AlbertError, "different request boundary"):
            WorkstationActionService(restored).submit(
                correlation_id="unreceipted-cross-action-collision-1",
                action_type="model-assignment-change",
                actor="mission-commander",
                expected_revision=1,
                target_kind="issue-slice",
                target_id="ISS-01",
                issue_id="ISS-01",
                agent_id="different-worker",
                reason="This correlation belongs to the launch.",
            )

        final = self.load_service()
        self.assertEqual(final._primary_mission.issues["ISS-01"].assigned_agent, original_assignment)
        self.assertEqual(list(final._primary_mission.sessions), ["session-ISS-01-1"])
        self.assertEqual(final.snapshot().revision, 1)

    def test_workstation_recovery_precedes_stale_revision_rejection(self) -> None:
        snapshots = self.load_service()
        mission = snapshots._primary_mission
        mission.approve_issue("ISS-01")
        session = mission.launch_issue("ISS-01")
        request = {
            "correlation_id": "late-workstation-cancel-recovery-1",
            "action_type": "session-cancel",
            "actor": "mission-commander",
            "expected_revision": 1,
            "target_kind": "agent-session",
            "target_id": session.session_id,
            "issue_id": "ISS-01",
            "session_id": session.session_id,
            "reason": "Cancel before another workspace action advances.",
        }
        original_write = WorkspaceSnapshotService._write_json_atomically

        def fail_acknowledgement_write(path: Path, data: dict[str, object]) -> None:
            if path == snapshots.preferences_path:
                raise OSError("simulated late cancellation acknowledgement failure")
            original_write(path, data)

        with patch.object(
            WorkspaceSnapshotService,
            "_write_json_atomically",
            side_effect=fail_acknowledgement_write,
        ):
            with self.assertRaisesRegex(OSError, "late cancellation acknowledgement failure"):
                WorkstationActionService(snapshots).submit(**request)

        advanced = self.load_service()
        current = advanced.snapshot()
        WorkspaceSyncService(advanced).submit_action(
            WorkspaceAction(
                correlation_id="advance-before-workstation-recovery",
                expected_revision=current.revision,
                active_mission_id=current.active_mission.id,
                conversation_scope=current.conversation_scope,
                operations_view=current.operations_view,
            )
        )

        acknowledgement = WorkstationActionService(self.load_service()).submit(**request)

        self.assertEqual(acknowledgement.revision, 3)
        final = self.load_service()
        self.assertEqual(final.snapshot().revision, 3)
        self.assertEqual(final._primary_mission.sessions[session.session_id].status, "cancelled")
        cancellation_entries = [
            item
            for item in final._primary_mission.timeline
            if f"session {session.session_id} cancelled:" in item
        ]
        self.assertEqual(len(cancellation_entries), 1)

    def test_workstation_action_exact_replay_does_not_duplicate_side_effects(self) -> None:
        snapshots = self.load_service()
        mission = snapshots._primary_mission
        mission.approve_issue("ISS-01")
        request = {
            "correlation_id": "workstation-exact-replay-1",
            "action_type": "issue-launch",
            "actor": "mission-commander",
            "expected_revision": 1,
            "target_kind": "issue-slice",
            "target_id": "ISS-01",
            "mission_id": mission.mission_id,
            "issue_id": "ISS-01",
            "allowed_paths": ["src"],
        }
        service = WorkstationActionService(snapshots)

        acknowledgement = service.submit(**request)
        history_before = AgentConsoleHistoryService(snapshots).history()
        journal_before = ActivityJournalService(snapshots).inspect().entries
        replayed = service.submit(**request)

        self.assertEqual(replayed, acknowledgement)
        self.assertEqual(snapshots.snapshot().revision, 2)
        self.assertEqual(list(mission.sessions), ["session-ISS-01-1"])
        self.assertEqual(AgentConsoleHistoryService(snapshots).history(), history_before)
        self.assertEqual(ActivityJournalService(snapshots).inspect().entries, journal_before)
        with self.assertRaisesRegex(AlbertError, "different request boundary"):
            service.submit(
                **{
                    **request,
                    "allowed_paths": ["different-boundary"],
                }
            )

    def test_workstation_receipt_replay_recovers_missing_journal_and_console_audit(
        self,
    ) -> None:
        snapshots = self.load_service()
        mission = snapshots._primary_mission
        mission.approve_issue("ISS-01")
        request = {
            "correlation_id": "workstation-audit-recovery-after-receipt-1",
            "action_type": "issue-launch",
            "actor": "mission-commander",
            "expected_revision": 1,
            "target_kind": "issue-slice",
            "target_id": "ISS-01",
            "mission_id": mission.mission_id,
            "issue_id": "ISS-01",
            "allowed_paths": ["src"],
        }
        journal_path = ActivityJournalService(snapshots).journal_path
        original_write = WorkspaceSnapshotService._write_json_atomically

        def fail_journal_write(path: Path, data: dict[str, object]) -> None:
            if path == journal_path:
                raise OSError("simulated journal write failure after receipt")
            original_write(path, data)

        with patch.object(
            WorkspaceSnapshotService,
            "_write_json_atomically",
            side_effect=fail_journal_write,
        ):
            with self.assertRaisesRegex(
                WorkspacePersistenceError,
                "Activity Journal persistence write failed",
            ):
                WorkstationActionService(snapshots).submit(**request)

        interrupted = self.load_service()
        self.assertEqual(interrupted.snapshot().revision, 2)
        self.assertEqual(len(ActivityJournalService(interrupted).inspect().entries), 0)
        self.assertEqual(len(AgentConsoleHistoryService(interrupted).history()), 0)

        acknowledgement = WorkstationActionService(interrupted).submit(**request)
        replayed = WorkstationActionService(self.load_service()).submit(**request)
        final = self.load_service()
        journal = ActivityJournalService(final).inspect().entries
        history = AgentConsoleHistoryService(final).history()

        self.assertEqual(replayed, acknowledgement)
        self.assertEqual(len(journal), 1)
        self.assertEqual(journal[0].correlation_id, request["correlation_id"])
        self.assertEqual(len(history), 2)
        self.assertEqual(
            [(item.correlation_id, item.action_phase) for item in history],
            [
                (request["correlation_id"], "request"),
                (request["correlation_id"], "acknowledgement"),
            ],
        )

    def test_workstation_receipt_replay_recovers_console_after_journal_append(
        self,
    ) -> None:
        snapshots = self.load_service()
        mission = snapshots._primary_mission
        mission.approve_issue("ISS-01")
        request = {
            "correlation_id": "workstation-audit-recovery-after-journal-1",
            "action_type": "issue-launch",
            "actor": "mission-commander",
            "expected_revision": 1,
            "target_kind": "issue-slice",
            "target_id": "ISS-01",
            "mission_id": mission.mission_id,
            "issue_id": "ISS-01",
            "allowed_paths": ["src"],
        }
        history_path = AgentConsoleHistoryService(snapshots).history_path
        original_write = WorkspaceSnapshotService._write_json_atomically

        def fail_console_write(path: Path, data: dict[str, object]) -> None:
            if path == history_path:
                raise OSError("simulated console write failure after journal")
            original_write(path, data)

        with patch.object(
            WorkspaceSnapshotService,
            "_write_json_atomically",
            side_effect=fail_console_write,
        ):
            with self.assertRaisesRegex(OSError, "console write failure after journal"):
                WorkstationActionService(snapshots).submit(**request)

        interrupted = self.load_service()
        self.assertEqual(len(ActivityJournalService(interrupted).inspect().entries), 1)
        self.assertEqual(len(AgentConsoleHistoryService(interrupted).history()), 0)

        acknowledgement = WorkstationActionService(interrupted).submit(**request)
        replayed = WorkstationActionService(self.load_service()).submit(**request)
        final = self.load_service()
        journal = ActivityJournalService(final).inspect().entries
        history = AgentConsoleHistoryService(final).history()

        self.assertEqual(replayed, acknowledgement)
        self.assertEqual(len(journal), 1)
        self.assertEqual(len(history), 2)
        self.assertEqual(
            [(item.correlation_id, item.action_phase) for item in history],
            [
                (request["correlation_id"], "request"),
                (request["correlation_id"], "acknowledgement"),
            ],
        )

    def test_workstation_receipt_replay_recovers_only_missing_console_action_phase(
        self,
    ) -> None:
        snapshots = self.load_service()
        mission = snapshots._primary_mission
        mission.approve_issue("ISS-01")
        request = {
            "correlation_id": "workstation-audit-recovery-after-console-request-1",
            "action_type": "issue-launch",
            "actor": "mission-commander",
            "expected_revision": 1,
            "target_kind": "issue-slice",
            "target_id": "ISS-01",
            "mission_id": mission.mission_id,
            "issue_id": "ISS-01",
            "allowed_paths": ["src"],
        }
        history_path = AgentConsoleHistoryService(snapshots).history_path
        original_write = WorkspaceSnapshotService._write_json_atomically
        console_writes = 0

        def fail_second_console_write(path: Path, data: dict[str, object]) -> None:
            nonlocal console_writes
            if path == history_path:
                console_writes += 1
                if console_writes == 2:
                    raise OSError("simulated acknowledgement-turn write failure")
            original_write(path, data)

        with patch.object(
            WorkspaceSnapshotService,
            "_write_json_atomically",
            side_effect=fail_second_console_write,
        ):
            with self.assertRaisesRegex(
                OSError,
                "acknowledgement-turn write failure",
            ):
                WorkstationActionService(snapshots).submit(**request)

        interrupted = self.load_service()
        interrupted_history = AgentConsoleHistoryService(interrupted).history()
        self.assertEqual(len(ActivityJournalService(interrupted).inspect().entries), 1)
        self.assertEqual(len(interrupted_history), 1)
        self.assertEqual(
            (
                interrupted_history[0].correlation_id,
                interrupted_history[0].action_phase,
            ),
            (request["correlation_id"], "request"),
        )

        acknowledgement = WorkstationActionService(interrupted).submit(**request)
        replayed = WorkstationActionService(self.load_service()).submit(**request)
        final = self.load_service()
        journal = ActivityJournalService(final).inspect().entries
        history = AgentConsoleHistoryService(final).history()

        self.assertEqual(replayed, acknowledgement)
        self.assertEqual(len(journal), 1)
        self.assertEqual(len(history), 2)
        self.assertEqual(
            [(item.correlation_id, item.action_phase) for item in history],
            [
                (request["correlation_id"], "request"),
                (request["correlation_id"], "acknowledgement"),
            ],
        )

    def test_workstation_action_changes_model_assignment_with_typed_target(self) -> None:
        snapshots = self.load_service()
        mission = snapshots._primary_mission

        acknowledgement = workspace_module.WorkstationActionService(snapshots).submit(
            correlation_id="workstation-model-assignment-1",
            action_type="model-assignment-change",
            actor="mission-commander",
            expected_revision=1,
            target_kind="issue-slice",
            target_id="ISS-01",
            issue_id="ISS-01",
            agent_id="qwen3.6-27b",
            reason="Use the stronger local model.",
        )

        self.assertEqual(acknowledgement.action_type, "model-assignment-change")
        self.assertEqual(acknowledgement.issue_id, "ISS-01")
        self.assertEqual(acknowledgement.revision, 2)
        self.assertEqual(mission.issues["ISS-01"].assigned_agent, "qwen3.6-27b")
        entry = ActivityJournalService(self.load_service()).inspect().entries[0]
        self.assertEqual(entry.action_type, "model-assignment-change")
        self.assertIn("qwen3.6-27b", entry.summary)

    def test_workstation_action_rejects_mismatched_model_assignment_target(self) -> None:
        snapshots = self.load_service()
        mission = snapshots._primary_mission

        with self.assertRaisesRegex(AlbertError, "target id must match issue id"):
            workspace_module.WorkstationActionService(snapshots).submit(
                correlation_id="workstation-model-assignment-invalid-1",
                action_type="model-assignment-change",
                actor="mission-commander",
                expected_revision=1,
                target_kind="issue-slice",
                target_id="ISS-02",
                issue_id="ISS-01",
                agent_id="qwen3.6-27b",
                reason="This target does not match.",
            )

        self.assertNotEqual(mission.issues["ISS-01"].assigned_agent, "qwen3.6-27b")

    def test_workstation_action_retries_repairable_session_and_cancels_active_session(self) -> None:
        snapshots = self.load_service()
        mission = snapshots._primary_mission
        mission.approve_issue("ISS-01")
        first_session = mission.launch_issue("ISS-01")
        mission.record_frontier_review(
            first_session.session_id,
            "Needs repair",
            reason="Acceptance criteria are not met.",
        )

        retry = workspace_module.WorkstationActionService(snapshots).submit(
            correlation_id="workstation-retry-1",
            action_type="issue-retry",
            actor="mission-commander",
            expected_revision=1,
            target_kind="agent-session",
            target_id=first_session.session_id,
            issue_id="ISS-01",
            session_id=first_session.session_id,
            reason="Run the repair.",
        )

        self.assertEqual(retry.action_type, "issue-retry")
        self.assertEqual(retry.session_id, "session-ISS-01-2")
        self.assertIn("session-ISS-01-2", mission.sessions)

        cancel = workspace_module.WorkstationActionService(snapshots).submit(
            correlation_id="workstation-cancel-1",
            action_type="session-cancel",
            actor="mission-commander",
            expected_revision=retry.revision,
            target_kind="agent-session",
            target_id="session-ISS-01-2",
            issue_id="ISS-01",
            session_id="session-ISS-01-2",
            reason="Stop this repair run.",
        )

        self.assertEqual(cancel.action_type, "session-cancel")
        self.assertEqual(cancel.session_id, "session-ISS-01-2")
        self.assertEqual(mission.sessions["session-ISS-01-2"].status, "cancelled")
        entries = ActivityJournalService(self.load_service()).inspect().entries
        self.assertEqual([entry.action_type for entry in entries[-2:]], ["issue-retry", "session-cancel"])

    def test_workstation_retry_inherits_paths_unless_explicit_governed_paths_are_sent(
        self,
    ) -> None:
        snapshots = self.load_service()
        mission = snapshots._primary_mission
        mission.approve_issue("ISS-01")
        first_session = mission.launch_issue("ISS-01", allowed_paths=["src/app.py"])
        mission.record_frontier_review(
            first_session.session_id,
            "Needs repair",
            reason="Acceptance criteria are not met.",
        )

        inherited = WorkstationActionService(snapshots).submit(
            correlation_id="workstation-retry-inherited-paths-1",
            action_type="issue-retry",
            actor="mission-commander",
            expected_revision=1,
            target_kind="agent-session",
            target_id=first_session.session_id,
            issue_id="ISS-01",
            session_id=first_session.session_id,
            reason="Retry without changing authority.",
        )
        mission.record_frontier_review(
            inherited.session_id,
            "Needs repair",
            reason="The first repair still needs a bounded follow-up.",
        )
        explicit = WorkstationActionService(snapshots).submit(
            correlation_id="workstation-retry-explicit-paths-1",
            action_type="issue-retry",
            actor="mission-commander",
            expected_revision=inherited.revision,
            target_kind="agent-session",
            target_id=inherited.session_id,
            issue_id="ISS-01",
            session_id=inherited.session_id,
            reason="Explicitly widen the governed repair boundary.",
            allowed_paths=["src", "tests"],
        )

        self.assertEqual(
            mission.sessions[inherited.session_id].task_packet["allowed_paths"],
            ["src/app.py"],
        )
        self.assertEqual(
            mission.sessions[explicit.session_id].task_packet["allowed_paths"],
            ["src", "tests"],
        )

    def test_workstation_action_qualifies_colliding_session_id_by_mission(self) -> None:
        primary = self.load_service()._primary_mission
        primary.approve_issue("ISS-01")
        primary_session = primary.launch_issue("ISS-01")
        background_tracker = self.root / "background-action-tracker"
        (background_tracker / "issues").mkdir(parents=True)
        (background_tracker / "PRD.md").write_text(
            "# Background Action Mission\n", encoding="utf-8"
        )
        (background_tracker / "issues" / "01-background.md").write_text(
            ISSUE, encoding="utf-8"
        )
        background = AlbertMission(
            target_repo=self.target_repo,
            tracker_dir=background_tracker,
            runtime_root=self.runtime,
            mission_id="background-action",
            allow_empty_tracker=True,
        ).load()
        background.approve_issue("ISS-01")
        background_session = background.launch_issue("ISS-01")
        self.assertEqual(primary_session.session_id, background_session.session_id)
        snapshots = WorkspaceSnapshotService(primary, missions=(background,))

        acknowledgement = WorkstationActionService(snapshots).submit(
            correlation_id="background-cancel-1",
            action_type="session-cancel",
            actor="mission-commander",
            expected_revision=1,
            target_kind="agent-session",
            target_id=background_session.session_id,
            mission_id="background-action",
            issue_id="ISS-01",
            session_id=background_session.session_id,
            reason="Cancel only the background runner.",
        )

        self.assertEqual(acknowledgement.revision, 2)
        self.assertEqual(background.sessions[background_session.session_id].status, "cancelled")
        self.assertEqual(primary.sessions[primary_session.session_id].status, "queued")
        self.assertEqual(snapshots.snapshot().active_mission.id, "command-deck")

    def test_workstation_action_rejects_retry_and_cancel_without_required_reason(self) -> None:
        snapshots = self.load_service()
        mission = snapshots._primary_mission
        mission.approve_issue("ISS-01")
        first_session = mission.launch_issue("ISS-01")
        first_session.status = "failed"
        first_session.runner_ended_at = "2026-07-11T10:00:00+00:00"
        mission._persist_session_update(first_session)

        with self.assertRaisesRegex(AlbertError, "Retry requires a reason"):
            workspace_module.WorkstationActionService(snapshots).submit(
                correlation_id="workstation-retry-without-reason-1",
                action_type="issue-retry",
                actor="mission-commander",
                expected_revision=1,
                target_kind="agent-session",
                target_id=first_session.session_id,
                issue_id="ISS-01",
                session_id=first_session.session_id,
            )

        self.assertNotIn("session-ISS-01-2", mission.sessions)
        mission.record_frontier_review(
            first_session.session_id,
            "Needs repair",
            reason="Acceptance criteria are not met.",
        )
        repair_session = mission.launch_repair(first_session.session_id)

        with self.assertRaisesRegex(AlbertError, "Session cancellation requires a reason"):
            workspace_module.WorkstationActionService(snapshots).submit(
                correlation_id="workstation-cancel-without-reason-1",
                action_type="session-cancel",
                actor="mission-commander",
                expected_revision=1,
                target_kind="agent-session",
                target_id=repair_session.session_id,
                issue_id="ISS-01",
                session_id=repair_session.session_id,
            )

        self.assertEqual(mission.sessions[repair_session.session_id].status, "queued")

    def test_cli_submits_workstation_action_as_json(self) -> None:
        mission = self.load_service()._primary_mission
        mission.approve_issue("ISS-01")
        output = io.StringIO()

        with redirect_stdout(output):
            exit_code = main(
                [
                    "workstation-action",
                    "--target-repo",
                    str(self.target_repo),
                    "--tracker-dir",
                    str(self.tracker),
                    "--runtime-root",
                    str(self.runtime),
                    "--mission-id",
                    "command-deck",
                    "--correlation-id",
                    "workstation-cli-launch-1",
                    "--expected-revision",
                    "1",
                    "--action-type",
                    "issue-launch",
                    "--actor",
                    "mission-commander",
                    "--target-kind",
                    "issue-slice",
                    "--target-id",
                    "ISS-01",
                    "--issue-id",
                    "ISS-01",
                    "--allowed-path",
                    "src",
                ]
            )

        payload = json.loads(output.getvalue())
        self.assertEqual(exit_code, 0)
        self.assertEqual(payload["action_type"], "issue-launch")
        self.assertEqual(payload["session_id"], "session-ISS-01-1")
        self.assertEqual(payload["revision"], 2)

    def test_cli_reports_stale_workstation_action_as_structured_json(self) -> None:
        snapshots = self.load_service()
        mission = snapshots._primary_mission
        mission.approve_issue("ISS-01")
        WorkspaceSyncService(snapshots).submit_action(
            WorkspaceAction(
                correlation_id="workstation-cli-stale-bump-1",
                expected_revision=1,
                active_mission_id="command-deck",
                conversation_scope=snapshots.snapshot().conversation_scope,
                operations_view="mission-board",
            )
        )
        output = io.StringIO()
        error = io.StringIO()

        with redirect_stdout(output), redirect_stderr(error):
            exit_code = main(
                [
                    "workstation-action",
                    "--target-repo",
                    str(self.target_repo),
                    "--tracker-dir",
                    str(self.tracker),
                    "--runtime-root",
                    str(self.runtime),
                    "--mission-id",
                    "command-deck",
                    "--correlation-id",
                    "workstation-cli-launch-stale-1",
                    "--expected-revision",
                    "1",
                    "--action-type",
                    "issue-launch",
                    "--actor",
                    "mission-commander",
                    "--target-kind",
                    "issue-slice",
                    "--target-id",
                    "ISS-01",
                    "--issue-id",
                    "ISS-01",
                ]
            )

        payload = json.loads(error.getvalue())
        self.assertEqual(exit_code, 1)
        self.assertEqual(output.getvalue(), "")
        self.assertEqual(payload["error"]["code"], "stale-action")
        self.assertEqual(payload["error"]["expected_revision"], 1)
        self.assertEqual(payload["error"]["current_revision"], 2)
        self.assertEqual(mission.sessions, {})

    def test_workspace_queue_groups_and_filters_items_by_type_and_mission(self) -> None:
        primary = self.load_service()._primary_mission
        primary.approve_issue("ISS-01")
        background_tracker = self.root / "queue-background-tracker"
        (background_tracker / "issues").mkdir(parents=True)
        (background_tracker / "PRD.md").write_text("# Background Queue Mission\n", encoding="utf-8")
        (background_tracker / "issues" / "01-background.md").write_text(
            ISSUE.replace("Restore the workspace session.", "Run background queue work."),
            encoding="utf-8",
        )
        background = AlbertMission(
            target_repo=self.target_repo,
            tracker_dir=background_tracker,
            runtime_root=self.runtime,
            mission_id="background-queue",
            allow_empty_tracker=True,
        ).load()
        background.approve_issue("ISS-01")
        service = WorkspaceQueueService(
            WorkspaceSnapshotService(primary, missions=(background,))
        )
        service.propose_issue_contract_change(
            correlation_id="proposal-primary-filter-1",
            expected_revision=1,
            issue_id="ISS-01",
            source="issue-slice-inspector",
            acceptance_criteria=["Primary Mission proposal."],
        )
        service.propose_issue_contract_change(
            correlation_id="proposal-background-filter-1",
            expected_revision=1,
            mission_id="background-queue",
            issue_id="ISS-01",
            source="mission-board",
            acceptance_criteria=["Background Mission proposal."],
        )

        projection = service.inspect(
            item_type="issue-change-proposal",
            mission_id="background-queue",
        )

        self.assertEqual(len(projection.items), 1)
        self.assertEqual(projection.items[0].mission_id, "background-queue")
        self.assertEqual(projection.items[0].item_type, "issue-change-proposal")
        self.assertEqual(len(projection.groups), 1)
        self.assertEqual(projection.groups[0].group_id, "issue-change-proposal:background-queue")
        self.assertEqual(projection.groups[0].item_type, "issue-change-proposal")
        self.assertEqual(projection.groups[0].mission_id, "background-queue")
        self.assertEqual(projection.groups[0].item_count, 1)
        self.assertEqual(projection.groups[0].items[0].source, "mission-board")

    def test_frontier_confirmation_remains_pending_without_applying_risky_action(self) -> None:
        snapshots = self.load_service()
        mission = snapshots._primary_mission
        mission.approve_issue("ISS-01")

        acknowledgement = WorkspaceQueueService(snapshots).request_frontier_confirmation(
            correlation_id="frontier-confirm-launch-boundary-1",
            expected_revision=1,
            issue_id="ISS-01",
            source="frontier-router",
            requested_action="Expand launch boundary for deployment files",
            affected_boundary="launch-boundary",
            consequence="Approval allows the next launch to include deployment paths.",
            payload={"allowed_paths": ["deploy/", "infra/"]},
        )
        projection = WorkspaceQueueService(snapshots).inspect(item_type="frontier-confirmation")

        self.assertEqual(acknowledgement.outcome, "acknowledged")
        self.assertEqual(acknowledgement.item_status, "pending")
        self.assertIsNone(acknowledgement.session_id)
        self.assertEqual(len(projection.items), 1)
        confirmation = projection.items[0]
        self.assertEqual(confirmation.item_id, "frontier-confirmation-command-deck-ISS-01-000001")
        self.assertEqual(confirmation.status, "pending")
        self.assertEqual(confirmation.source, "frontier-router")
        self.assertEqual(confirmation.requested_action, "Expand launch boundary for deployment files")
        self.assertEqual(confirmation.affected_boundary, "launch-boundary")
        self.assertEqual(
            confirmation.consequence,
            "Approval allows the next launch to include deployment paths.",
        )
        self.assertEqual(confirmation.proposed_changes["allowed_paths"], ["deploy/", "infra/"])
        self.assertEqual(mission.issues["ISS-01"].review_state, "approved")
        self.assertEqual(mission.sessions, {})

    def test_frontier_confirmation_replays_after_lost_audit_response(self) -> None:
        snapshots = self.load_service()
        snapshots._primary_mission.approve_issue("ISS-01")
        request = {
            "correlation_id": "frontier-confirmation-replay-1",
            "expected_revision": 1,
            "issue_id": "ISS-01",
            "source": "frontier-router",
            "requested_action": "Confirm the bounded launch path",
            "affected_boundary": "allowed_paths",
            "consequence": "Approval permits one generated report.",
            "payload": {"allowed_paths": ["generated/report.md"]},
        }

        with patch.object(
            ActivityJournalService,
            "record_frontier_confirmation_requested",
            side_effect=OSError("simulated Frontier audit response loss"),
        ):
            with self.assertRaisesRegex(OSError, "Frontier audit response loss"):
                WorkspaceQueueService(snapshots).request_frontier_confirmation(**request)

        replayed = WorkspaceQueueService(
            self.load_service()
        ).request_frontier_confirmation(**request)
        replayed_again = WorkspaceQueueService(
            self.load_service()
        ).request_frontier_confirmation(**request)

        self.assertEqual(replayed, replayed_again)
        self.assertEqual(len(WorkspaceQueueService(self.load_service()).inspect().items), 1)
        entries = ActivityJournalService(self.load_service()).inspect().entries
        self.assertEqual(
            [entry.correlation_id for entry in entries],
            ["frontier-confirmation-replay-1"],
        )
        with self.assertRaisesRegex(AlbertError, "different request"):
            WorkspaceQueueService(self.load_service()).request_frontier_confirmation(
                **{
                    **request,
                    "payload": {"allowed_paths": ["different/report.md"]},
                }
            )

    def test_frontier_decision_repairs_lost_proposal_audit_before_its_own_entry(
        self,
    ) -> None:
        snapshots = self.load_service()
        snapshots._primary_mission.approve_issue("ISS-01")
        queue = WorkspaceQueueService(snapshots)
        request = {
            "correlation_id": "frontier-causal-proposal-1",
            "expected_revision": 1,
            "issue_id": "ISS-01",
            "source": "frontier-router",
            "requested_action": "Confirm causal audit ordering",
            "affected_boundary": "allowed_paths",
            "consequence": "Approval preserves proposal-before-decision chronology.",
            "payload": {"allowed_paths": ["generated/causal.md"]},
        }
        with patch.object(
            ActivityJournalService,
            "record_frontier_confirmation_requested",
            side_effect=OSError("simulated lost Frontier proposal audit"),
        ):
            with self.assertRaisesRegex(OSError, "lost Frontier proposal audit"):
                queue.request_frontier_confirmation(**request)

        proposal = queue.inspect().items[0]
        queue.decide(
            correlation_id="frontier-causal-decision-1",
            expected_revision=2,
            item_id=proposal.item_id,
            decision="reject",
            reason="Keep the current boundary.",
        )
        queue.request_frontier_confirmation(**request)

        entries = ActivityJournalService(self.load_service()).inspect().entries
        self.assertEqual(
            [(entry.correlation_id, entry.action_type) for entry in entries],
            [
                (
                    "frontier-causal-proposal-1",
                    "frontier-confirmation-requested",
                ),
                ("frontier-causal-decision-1", "workspace-queue-decision"),
            ],
        )

    def test_frontier_confirmation_rejects_stale_workspace_revision(self) -> None:
        snapshots = self.load_service()
        snapshots._primary_mission.approve_issue("ISS-01")

        with self.assertRaises(WorkspaceStaleActionError):
            WorkspaceQueueService(snapshots).request_frontier_confirmation(
                correlation_id="frontier-confirmation-stale-1",
                expected_revision=99,
                issue_id="ISS-01",
                source="frontier-router",
                requested_action="Confirm a stale launch boundary",
                affected_boundary="allowed_paths",
                consequence="This stale request must not persist.",
                payload={"allowed_paths": ["generated/stale.md"]},
            )

        self.assertEqual(WorkspaceQueueService(self.load_service()).inspect().items, ())

    def test_ad_hoc_delegation_proposal_records_boundaries_without_launching(self) -> None:
        snapshots = self.load_service()
        history = AgentConsoleHistoryService(snapshots)
        origin = history.append(
            role="user",
            content="Have a local agent update the smoke-test notes.",
            outcome="proposed",
            source="mission-commander",
            expected_revision=1,
            expected_scope=snapshots.snapshot().conversation_scope,
        )

        acknowledgement = WorkspaceQueueService(snapshots).propose_ad_hoc_delegation(
            correlation_id="ad-hoc-delegation-1",
            expected_revision=1,
            source="agent-console",
            scope=snapshots.snapshot().conversation_scope,
            acceptance_criteria=["Smoke-test notes describe the latest verification command."],
            allowed_paths=["docs/smoke-tests.md"],
            command_policy={"python3 -m unittest tests.test_workspace_snapshot": "auto-allowed"},
            proposed_agent="qwen-coder-local-1",
            originating_message_id=origin.message_id,
        )
        projection = WorkspaceQueueService(self.load_service()).inspect(
            item_type="ad-hoc-delegation"
        )

        self.assertEqual(acknowledgement.outcome, "acknowledged")
        self.assertEqual(acknowledgement.item_status, "pending")
        self.assertEqual(acknowledgement.item_id, "ad-hoc-delegation-command-deck-000001")
        self.assertEqual(len(projection.items), 1)
        item = projection.items[0]
        self.assertEqual(item.item_type, "ad-hoc-delegation")
        self.assertEqual(item.status, "pending")
        self.assertEqual(item.issue_id, "ADHOC-000001")
        self.assertEqual(item.source, "agent-console")
        self.assertEqual(item.requested_action, "Approve Ad Hoc Delegation")
        self.assertEqual(item.affected_boundary, "ad-hoc-delegation")
        self.assertEqual(
            item.consequence,
            "Approval will launch ADHOC-000001 within the proposed scope, permissions, and acceptance criteria.",
        )
        self.assertEqual(item.proposed_changes["scope"]["kind"], "working-directory")
        self.assertEqual(
            item.proposed_changes["acceptance_criteria"],
            ["Smoke-test notes describe the latest verification command."],
        )
        self.assertEqual(item.proposed_changes["allowed_paths"], ["docs/smoke-tests.md"])
        self.assertEqual(item.proposed_changes["proposed_agent"], "qwen-coder-local-1")
        self.assertEqual(item.proposed_changes["originating_message_id"], origin.message_id)
        self.assertEqual(item.proposal_correlation_id, "ad-hoc-delegation-1")
        self.assertEqual(item.decision_correlation_id, "")
        self.assertEqual(snapshots._primary_mission.sessions, {})

    def test_rejected_ad_hoc_delegation_preserves_no_launch_state(self) -> None:
        snapshots = self.load_service()
        origin = AgentConsoleHistoryService(snapshots).append(
            role="user",
            content="Try a bounded documentation update.",
            outcome="proposed",
            source="mission-commander",
            expected_revision=1,
            expected_scope=snapshots.snapshot().conversation_scope,
        )
        service = WorkspaceQueueService(snapshots)
        proposal = service.propose_ad_hoc_delegation(
            correlation_id="ad-hoc-delegation-reject-1",
            expected_revision=1,
            source="agent-console",
            scope=snapshots.snapshot().conversation_scope,
            acceptance_criteria=["Documentation is updated."],
            allowed_paths=["docs/smoke-tests.md"],
            command_policy={},
            proposed_agent="qwen-coder-local-1",
            originating_message_id=origin.message_id,
        )

        acknowledgement = service.decide(
            correlation_id="ad-hoc-delegation-reject-decision-1",
            expected_revision=proposal.revision,
            item_id=proposal.item_id,
            decision="reject",
            reason="Not approved for this Mission.",
        )
        item = service.inspect(item_type="ad-hoc-delegation").items[0]

        self.assertEqual(acknowledgement.item_status, "rejected")
        self.assertEqual(item.status, "rejected")
        self.assertEqual(
            item.proposal_correlation_id,
            "ad-hoc-delegation-reject-1",
        )
        self.assertEqual(
            item.decision_correlation_id,
            "ad-hoc-delegation-reject-decision-1",
        )
        self.assertEqual(snapshots._primary_mission.sessions, {})

    def test_approved_ad_hoc_delegation_launches_bounded_session_without_issue_slice(self) -> None:
        snapshots = self.load_service()
        mission = snapshots._primary_mission
        origin = AgentConsoleHistoryService(snapshots).append(
            role="user",
            content="Run a bounded docs update through a local agent.",
            outcome="proposed",
            source="mission-commander",
            expected_revision=1,
            expected_scope=snapshots.snapshot().conversation_scope,
        )
        service = WorkspaceQueueService(snapshots)
        proposal = service.propose_ad_hoc_delegation(
            correlation_id="ad-hoc-delegation-approve-1",
            expected_revision=1,
            source="agent-console",
            scope=snapshots.snapshot().conversation_scope,
            acceptance_criteria=["Docs mention the focused unit test command."],
            allowed_paths=["docs/smoke-tests.md"],
            command_policy={"python3 -m unittest tests.test_workspace_snapshot": "auto-allowed"},
            proposed_agent="qwen-coder-local-1",
            originating_message_id=origin.message_id,
        )

        acknowledgement = service.decide(
            correlation_id="ad-hoc-delegation-approve-decision-1",
            expected_revision=proposal.revision,
            item_id=proposal.item_id,
            decision="approve",
            reason="Approved for bounded docs update.",
        )
        session = mission.sessions["session-ADHOC-000001-1"]

        self.assertEqual(acknowledgement.item_status, "approved")
        self.assertEqual(acknowledgement.session_id, session.session_id)
        self.assertNotIn("ADHOC-000001", mission.issues)
        self.assertEqual(session.issue_id, "ADHOC-000001")
        self.assertEqual(session.status, "queued")
        self.assertFalse(
            session.worktree_path.exists(),
            "queue acknowledgement must return before creating the session workspace",
        )
        self.assertEqual(session.task_packet["work_kind"], "ad-hoc-delegation")
        self.assertEqual(session.task_packet["originating_message_id"], origin.message_id)
        self.assertEqual(session.task_packet["conversation_scope"]["kind"], "working-directory")
        self.assertEqual(
            session.task_packet["goal"],
            "Run a bounded docs update through a local agent.",
        )
        self.assertEqual(session.task_packet["acceptance_criteria"], ["Docs mention the focused unit test command."])
        self.assertEqual(session.task_packet["allowed_paths"], ["docs/smoke-tests.md"])
        self.assertEqual(
            session.task_packet["command_policy"],
            {"python3 -m unittest tests.test_workspace_snapshot": "auto-allowed"},
        )
        self.assertEqual(session.assigned_agent, "qwen-coder-local-1")
        reloaded = self.load_service()._primary_mission.sessions[session.session_id]
        self.assertEqual(reloaded.status, "queued")

    def test_ad_hoc_approval_preserves_newer_unrelated_issue_runtime(self) -> None:
        stale_snapshots = self.load_service()
        origin = AgentConsoleHistoryService(stale_snapshots).append(
            role="user",
            content="Launch without overwriting newer issue state.",
            outcome="proposed",
            source="mission-commander",
            expected_revision=1,
            expected_scope=stale_snapshots.snapshot().conversation_scope,
        )
        stale_queue = WorkspaceQueueService(stale_snapshots)
        proposal = stale_queue.propose_ad_hoc_delegation(
            correlation_id="ad-hoc-preserve-runtime-proposal-1",
            expected_revision=1,
            source="agent-console",
            scope=stale_snapshots.snapshot().conversation_scope,
            acceptance_criteria=["The session is queued without stale overwrites."],
            allowed_paths=["docs/runtime.md"],
            command_policy={},
            proposed_agent="qwen-coder-local-1",
            originating_message_id=origin.message_id,
        )
        writer = self.load_service()._primary_mission
        writer.assign_issue(
            "ISS-01",
            "qwen-coder-local-1",
            notes="Newer runtime state must survive the Ad Hoc transaction.",
        )

        stale_queue.decide(
            correlation_id="ad-hoc-preserve-runtime-decision-1",
            expected_revision=proposal.revision,
            item_id=proposal.item_id,
            decision="approve",
            reason="Approve the bounded session.",
        )

        restored = self.load_service()._primary_mission
        self.assertEqual(
            restored.issues["ISS-01"].notes,
            "Newer runtime state must survive the Ad Hoc transaction.",
        )
        self.assertEqual(len(restored.sessions), 1)

    def test_queue_replay_rejects_an_unrelated_real_ad_hoc_session(self) -> None:
        snapshots = self.load_service()
        history = AgentConsoleHistoryService(snapshots)
        origin = history.append(
            role="user",
            content="Launch one bounded local task.",
            outcome="proposed",
            source="mission-commander",
            expected_revision=1,
            expected_scope=snapshots.snapshot().conversation_scope,
        )
        queue = WorkspaceQueueService(snapshots)

        first = queue.propose_ad_hoc_delegation(
            correlation_id="session-binding-proposal-1",
            expected_revision=1,
            source="agent-console",
            scope=snapshots.snapshot().conversation_scope,
            acceptance_criteria=["The first bounded task is represented."],
            allowed_paths=["docs/first.md"],
            command_policy={},
            proposed_agent="qwen-coder-local-1",
            originating_message_id=origin.message_id,
        )
        first_decision = queue.decide(
            correlation_id="session-binding-decision-1",
            expected_revision=first.revision,
            item_id=first.item_id,
            decision="approve",
            reason="Approve the first bounded task.",
        )
        first_session = snapshots._primary_mission.sessions[first_decision.session_id]
        malformed_packet_session = LocalAgentSession.from_dict(first_session.to_dict())
        malformed_packet_session.task_packet["issue_id"] = "ADHOC-FORGED"
        first_item_projection = queue.inspect().items[0]
        self.assertFalse(
            queue._session_matches_ad_hoc_delegation(
                first_item_projection,
                malformed_packet_session,
                approval_correlation_id="session-binding-decision-1",
                decision_request={
                    "item_id": first.item_id,
                    "decision": "approve",
                    "reason": "Approve the first bounded task.",
                    "action_type": "",
                    "actor": "",
                    "target_kind": "",
                    "target_id": "",
                },
            )
        )
        second = queue.propose_ad_hoc_delegation(
            correlation_id="session-binding-proposal-2",
            expected_revision=1,
            source="agent-console",
            scope=snapshots.snapshot().conversation_scope,
            acceptance_criteria=["The second bounded task is represented."],
            allowed_paths=["docs/second.md"],
            command_policy={},
            proposed_agent="qwen-coder-local-1",
            originating_message_id=origin.message_id,
        )
        second_decision = queue.decide(
            correlation_id="session-binding-decision-2",
            expected_revision=second.revision,
            item_id=second.item_id,
            decision="approve",
            reason="Approve the second bounded task.",
        )
        self.assertNotEqual(first_decision.session_id, second_decision.session_id)

        persisted = queue._load_queue()
        first_item = next(item for item in persisted["items"] if item.item_id == first.item_id)
        receipt = next(
            item
            for item in persisted["receipts"]
            if item["correlation_id"] == "session-binding-decision-1"
        )
        forged_ack = {
            **receipt["acknowledgement"],
            "session_id": second_decision.session_id,
            "effect_summary": (
                f"Approved {first_item.item_id}; queued {first_item.issue_id} as "
                f"{second_decision.session_id}."
            ),
        }
        forged_receipt = {**receipt, "acknowledgement": forged_ack}
        forged_queue = {
            **persisted,
            "receipts": [
                forged_receipt if item is receipt else item
                for item in persisted["receipts"]
            ],
        }
        journal = ActivityJournalService(snapshots)
        journal.journal_path.unlink(missing_ok=True)

        acknowledgement = queue._replay_queue_request(
            forged_queue,
            correlation_id="session-binding-decision-1",
            request_kind="workspace-queue-decision",
            request_payload=receipt["request"],
        )
        self.assertIsNotNone(acknowledgement)
        with self.assertRaisesRegex(
            WorkspacePersistenceError,
            "does not match the approved delegation",
        ):
            queue._reconcile_queue_decision_audit(forged_queue, acknowledgement)
        self.assertEqual(journal.inspect().entries, ())

    def test_ad_hoc_proposal_and_approval_replay_after_lost_queue_write_response(
        self,
    ) -> None:
        snapshots = self.load_service()
        mission = snapshots._primary_mission
        origin = AgentConsoleHistoryService(snapshots).append(
            role="user",
            content="Retry this bounded docs task safely.",
            outcome="proposed",
            source="mission-commander",
            expected_revision=1,
            expected_scope=snapshots.snapshot().conversation_scope,
        )
        service = WorkspaceQueueService(snapshots)
        proposal_args = {
            "correlation_id": "ad-hoc-transport-proposal-1",
            "expected_revision": 1,
            "source": "agent-console",
            "scope": snapshots.snapshot().conversation_scope,
            "acceptance_criteria": ["Docs are updated once."],
            "allowed_paths": ["docs/smoke-tests.md"],
            "command_policy": {},
            "proposed_agent": "qwen-coder-local-1",
            "originating_message_id": origin.message_id,
        }
        with self.assertRaisesRegex(AlbertError, "scope and Mission"):
            service.propose_ad_hoc_delegation(
                **{
                    **proposal_args,
                    "correlation_id": "ad-hoc-scope-mismatch-1",
                    "scope": ConversationScope(
                        kind="mission",
                        target_id=mission.mission_id,
                        label="Command Deck Mission",
                        mission_id=mission.mission_id,
                    ),
                }
            )
        original_write = WorkspaceSnapshotService._write_json_atomically
        proposal_response_dropped = False

        def drop_proposal_response(path: Path, payload: dict[str, object]) -> None:
            nonlocal proposal_response_dropped
            original_write(path, payload)
            if path == service.queue_path and not proposal_response_dropped:
                proposal_response_dropped = True
                raise ConnectionError("proposal response lost after durable queue write")

        with patch.object(
            WorkspaceSnapshotService,
            "_write_json_atomically",
            side_effect=drop_proposal_response,
        ):
            with self.assertRaisesRegex(ConnectionError, "proposal response lost"):
                service.propose_ad_hoc_delegation(**proposal_args)

        current = snapshots.snapshot()
        snapshots.update_preferences(
            active_mission_id=current.active_mission.id,
            conversation_scope=current.conversation_scope,
            operations_view=current.operations_view,
            event_metadata={"correlation_id": "advance-after-lost-proposal-response"},
        )
        proposal = service.propose_ad_hoc_delegation(**proposal_args)
        self.assertEqual(len(service.inspect(item_type="ad-hoc-delegation").items), 1)
        with self.assertRaisesRegex(AlbertError, "different request"):
            service.propose_ad_hoc_delegation(
                **{
                    **proposal_args,
                    "acceptance_criteria": ["A different task must not replay."],
                }
            )

        decision_args = {
            "correlation_id": "ad-hoc-transport-decision-1",
            "expected_revision": proposal.revision,
            "item_id": proposal.item_id,
            "decision": "approve",
            "reason": "Approve one bounded launch.",
        }
        decision_response_dropped = False

        def fail_decision_queue_write(path: Path, payload: dict[str, object]) -> None:
            nonlocal decision_response_dropped
            if path == service.queue_path and not decision_response_dropped:
                decision_response_dropped = True
                raise ConnectionError("decision queue write failed after session persistence")
            original_write(path, payload)

        with patch.object(
            WorkspaceSnapshotService,
            "_write_json_atomically",
            side_effect=fail_decision_queue_write,
        ):
            with self.assertRaisesRegex(ConnectionError, "decision queue write failed"):
                service.decide(**decision_args)

        with self.assertRaisesRegex(
            WorkspacePersistenceError,
            "approval effect is already durable",
        ):
            WorkspaceQueueService(self.load_service()).decide(
                correlation_id="ad-hoc-contradictory-defer-1",
                expected_revision=proposal.revision,
                item_id=proposal.item_id,
                decision="defer",
                reason="A different decision must not contradict the durable session.",
            )
        WorkspaceQueueService(self.load_service()).request_frontier_confirmation(
            correlation_id="advance-after-ad-hoc-effect-1",
            expected_revision=2,
            issue_id="ISS-01",
            source="frontier-model",
            requested_action="Inspect an unrelated boundary",
            affected_boundary="unrelated",
            consequence="No change to the recovered Ad Hoc session.",
            payload={"note": "advance queue revision"},
        )

        acknowledgement = service.decide(**decision_args)
        replayed_acknowledgement = service.decide(**decision_args)

        self.assertEqual(acknowledgement.item_status, "approved")
        self.assertEqual(acknowledgement.session_id, "session-ADHOC-000001-1")
        self.assertEqual(replayed_acknowledgement, acknowledgement)
        self.assertEqual(len(mission.sessions), 1)
        restored_item = service.inspect(item_type="ad-hoc-delegation").items[0]
        self.assertEqual(restored_item.status, "approved")
        self.assertEqual(
            restored_item.proposal_correlation_id,
            "ad-hoc-transport-proposal-1",
        )
        self.assertEqual(
            restored_item.decision_correlation_id,
            "ad-hoc-transport-decision-1",
        )
        with self.assertRaisesRegex(AlbertError, "different request"):
            service.decide(
                **{
                    **decision_args,
                    "reason": "A changed decision payload must not replay.",
                }
            )

    def test_ad_hoc_proposal_replay_keeps_its_original_mission_after_switch(self) -> None:
        primary = self.load_service()._primary_mission
        background_tracker = self.root / "ad-hoc-replay-background"
        (background_tracker / "issues").mkdir(parents=True)
        (background_tracker / "PRD.md").write_text(
            "# Background Replay Mission\n",
            encoding="utf-8",
        )
        (background_tracker / "issues" / "01-background.md").write_text(
            ISSUE,
            encoding="utf-8",
        )
        background = AlbertMission(
            target_repo=self.target_repo,
            tracker_dir=background_tracker,
            runtime_root=self.runtime,
            mission_id="background-replay",
            allow_empty_tracker=True,
        ).load()
        snapshots = WorkspaceSnapshotService(primary, missions=(background,))
        scope = snapshots.snapshot().conversation_scope
        origin = AgentConsoleHistoryService(snapshots).append(
            role="user",
            content="Keep this task attached to the original Mission.",
            outcome="proposed",
            source="mission-commander",
            expected_revision=1,
            expected_scope=scope,
        )
        queue = WorkspaceQueueService(snapshots)
        request = {
            "correlation_id": "ad-hoc-mission-stable-replay-1",
            "expected_revision": 1,
            "source": "agent-console",
            "scope": scope,
            "acceptance_criteria": ["The original Mission remains authoritative."],
            "allowed_paths": ["."],
            "command_policy": {},
            "proposed_agent": "qwen-coder-local-1",
            "originating_message_id": origin.message_id,
            "mission_id": primary.mission_id,
        }

        acknowledgement = queue.propose_ad_hoc_delegation(**request)
        snapshots.update_preferences(
            active_mission_id=background.mission_id,
            conversation_scope=scope,
            operations_view="mission-board",
            event_metadata={"correlation_id": "switch-after-ad-hoc-proposal"},
        )
        replayed = queue.propose_ad_hoc_delegation(**request)

        self.assertEqual(replayed, acknowledgement)
        self.assertEqual(queue.inspect().items[0].mission_id, primary.mission_id)
        self.assertEqual(len(queue.inspect().items), 1)
        with self.assertRaisesRegex(AlbertError, "different request"):
            queue.propose_ad_hoc_delegation(
                **{
                    **request,
                    "acceptance_criteria": ["A changed payload must not replay."],
                }
            )

    def test_approved_ad_hoc_delegation_is_executable_by_deferred_session_runner(self) -> None:
        (self.target_repo / ".albert").mkdir()
        (self.target_repo / ".albert" / "agents.json").write_text(
            json.dumps(
                {
                    "agents": [
                        {
                            "id": "qwen-coder-local-1",
                            "role": "local-agent",
                            "provider": "test-harness",
                            "runner": "fake",
                            "model": "deterministic-fake",
                        }
                    ]
                }
            ),
            encoding="utf-8",
        )
        snapshots = self.load_service()
        mission = snapshots._primary_mission
        history = AgentConsoleHistoryService(snapshots)
        origin = history.append(
            role="user",
            content="Execute a bounded ad hoc task after acknowledgement.",
            outcome="proposed",
            source="mission-commander",
            expected_revision=1,
            expected_scope=snapshots.snapshot().conversation_scope,
        )
        queue = WorkspaceQueueService(snapshots)
        proposal = queue.propose_ad_hoc_delegation(
            correlation_id="ad-hoc-deferred-run-proposal-1",
            expected_revision=1,
            source="agent-console",
            scope=snapshots.snapshot().conversation_scope,
            acceptance_criteria=["Deferred runner produces an Evidence Package."],
            allowed_paths=["docs/result.md"],
            command_policy={},
            proposed_agent="qwen-coder-local-1",
            originating_message_id=origin.message_id,
        )

        acknowledgement = queue.decide(
            correlation_id="ad-hoc-deferred-run-approve-1",
            expected_revision=proposal.revision,
            item_id=proposal.item_id,
            decision="approve",
            reason="Execute after the queue acknowledgement.",
        )

        self.assertIsNotNone(acknowledgement.session_id)
        queued = mission.sessions[acknowledgement.session_id]
        self.assertEqual(queued.status, "queued")
        completed = mission.run_session(queued.session_id)
        self.assertEqual(completed.status, "evidence-ready")
        self.assertTrue(completed.evidence_valid)

    def test_ad_hoc_approval_rejects_controller_and_delegate_only_agents(self) -> None:
        (self.target_repo / ".albert").mkdir()
        (self.target_repo / ".albert" / "agents.json").write_text(
            json.dumps(
                {
                    "agents": [
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
                            "id": "worker",
                            "role": "local-agent",
                            "provider": "local",
                            "runner": "fake",
                            "routing": "worker",
                        },
                    ]
                }
            ),
            encoding="utf-8",
        )
        snapshots = self.load_service()
        origin = AgentConsoleHistoryService(snapshots).append(
            role="user",
            content="Do not route this task to a controller.",
            outcome="proposed",
            source="mission-commander",
            expected_revision=1,
            expected_scope=snapshots.snapshot().conversation_scope,
        )
        service = WorkspaceQueueService(snapshots)

        for index, agent_id in enumerate(("controller", "delegate"), start=1):
            proposal = service.propose_ad_hoc_delegation(
                correlation_id=f"ad-hoc-invalid-agent-{index}",
                expected_revision=1,
                source="agent-console",
                scope=snapshots.snapshot().conversation_scope,
                acceptance_criteria=["Only an assignable worker may launch."],
                allowed_paths=["docs"],
                command_policy={},
                proposed_agent=agent_id,
                originating_message_id=origin.message_id,
            )
            with self.assertRaisesRegex(AlbertError, "not assignable"):
                service.decide(
                    correlation_id=f"ad-hoc-invalid-agent-decision-{index}",
                    expected_revision=proposal.revision,
                    item_id=proposal.item_id,
                    decision="approve",
                )

        self.assertEqual(snapshots._primary_mission.sessions, {})
        self.assertTrue(
            all(
                item.status == "pending"
                for item in service.inspect(item_type="ad-hoc-delegation").items
            )
        )

    def test_ad_hoc_automatic_approval_rejects_gated_and_cloud_workers(self) -> None:
        (self.target_repo / ".albert").mkdir()
        (self.target_repo / ".albert" / "agents.json").write_text(
            json.dumps(
                {
                    "agents": [
                        {
                            "id": "gated-worker",
                            "role": "local-agent",
                            "provider": "test-harness",
                            "runner": "fake",
                            "routing": "worker",
                            "requires_approval": True,
                        },
                        {
                            "id": "cloud-worker",
                            "role": "local-agent",
                            "provider": "remote",
                            "runner": "fake",
                            "routing": "worker",
                            "model": "remote-worker:CLOUD",
                        },
                    ]
                }
            ),
            encoding="utf-8",
        )
        snapshots = self.load_service()
        origin = AgentConsoleHistoryService(snapshots).append(
            role="user",
            content="Run this bounded task only after explicit agent approval.",
            outcome="proposed",
            source="mission-commander",
            expected_revision=1,
            expected_scope=snapshots.snapshot().conversation_scope,
        )
        queue = WorkspaceQueueService(snapshots)
        for index, agent_id in enumerate(("gated-worker", "cloud-worker"), start=1):
            proposal = queue.propose_ad_hoc_delegation(
                correlation_id=f"gated-ad-hoc-proposal-{index}",
                expected_revision=1,
                source="agent-console",
                scope=origin.scope,
                acceptance_criteria=["The approval gate is preserved."],
                allowed_paths=["."],
                command_policy={},
                proposed_agent=agent_id,
                originating_message_id=origin.message_id,
            )
            with self.assertRaisesRegex(
                AlbertError,
                "requires explicit delegation approval",
            ):
                queue.decide(
                    correlation_id=f"gated-ad-hoc-decision-{index}",
                    expected_revision=proposal.revision,
                    item_id=proposal.item_id,
                    decision="approve",
                    reason="Automatic task dispatch must not waive the agent gate.",
                )

        self.assertTrue(all(item.status == "pending" for item in queue.inspect().items))
        self.assertEqual(snapshots._primary_mission.sessions, {})

    def test_ad_hoc_delegation_denies_launch_when_command_policy_is_not_auto_allowed(self) -> None:
        snapshots = self.load_service()
        origin = AgentConsoleHistoryService(snapshots).append(
            role="user",
            content="Prepare a deploy note, but do not run deploy commands.",
            outcome="proposed",
            source="mission-commander",
            expected_revision=1,
            expected_scope=snapshots.snapshot().conversation_scope,
        )
        service = WorkspaceQueueService(snapshots)
        proposal = service.propose_ad_hoc_delegation(
            correlation_id="ad-hoc-delegation-deny-command-1",
            expected_revision=1,
            source="agent-console",
            scope=snapshots.snapshot().conversation_scope,
            acceptance_criteria=["Deploy note is drafted without pushing changes."],
            allowed_paths=["docs/deploy-note.md"],
            command_policy={"git push origin main": "frontier-approvable"},
            proposed_agent="qwen-coder-local-1",
            originating_message_id=origin.message_id,
        )

        with self.assertRaisesRegex(AlbertError, "requires auto-allowed command policy"):
            service.decide(
                correlation_id="ad-hoc-delegation-deny-command-decision-1",
                expected_revision=proposal.revision,
                item_id=proposal.item_id,
                decision="approve",
                reason="Attempted approval outside accepted command boundary.",
            )

        item = service.inspect(item_type="ad-hoc-delegation").items[0]
        self.assertEqual(item.status, "pending")
        self.assertEqual(snapshots._primary_mission.sessions, {})

    def test_ad_hoc_delegation_session_summary_exposes_status_and_model_provenance(self) -> None:
        (self.target_repo / ".albert").mkdir()
        (self.target_repo / ".albert" / "agents.json").write_text(
            json.dumps(
                {
                    "agents": [
                        {
                            "id": "qwen-coder-local-1",
                            "role": "local-agent",
                            "provider": "ollama",
                            "runner": "ollama",
                            "model": "qwen2.5-coder:14b",
                            "availability": "available",
                        }
                    ]
                }
            ),
            encoding="utf-8",
        )
        snapshots = self.load_service()
        origin = AgentConsoleHistoryService(snapshots).append(
            role="user",
            content="Launch a bounded docs-only session.",
            outcome="proposed",
            source="mission-commander",
            expected_revision=1,
            expected_scope=snapshots.snapshot().conversation_scope,
        )
        service = WorkspaceQueueService(snapshots)
        proposal = service.propose_ad_hoc_delegation(
            correlation_id="ad-hoc-delegation-session-summary-1",
            expected_revision=1,
            source="agent-console",
            scope=snapshots.snapshot().conversation_scope,
            acceptance_criteria=["Docs-only session is visible in workspace summary."],
            allowed_paths=["docs/smoke-tests.md"],
            command_policy={},
            proposed_agent="qwen-coder-local-1",
            originating_message_id=origin.message_id,
        )

        service.decide(
            correlation_id="ad-hoc-delegation-session-summary-decision-1",
            expected_revision=proposal.revision,
            item_id=proposal.item_id,
            decision="approve",
            reason="Approved for workspace session visibility.",
        )
        session_summary = snapshots.snapshot().missions[0].sessions[0]

        self.assertEqual(session_summary.session_id, "session-ADHOC-000001-1")
        self.assertEqual(session_summary.issue_id, "ADHOC-000001")
        self.assertEqual(session_summary.status, "queued")
        self.assertEqual(session_summary.role, "local-agent")
        self.assertEqual(session_summary.provider, "ollama")
        self.assertEqual(session_summary.model, "qwen2.5-coder:14b")

    def test_workspace_session_summary_exposes_timestamp_backed_last_activity(self) -> None:
        snapshots = self.load_service()
        mission = snapshots._primary_mission
        mission.approve_issue("ISS-01")
        session = mission.launch_issue("ISS-01")
        session.runner_started_at = "2026-07-12T08:30:00+00:00"
        session.runner_ended_at = "2026-07-12T08:31:45+00:00"
        session.status = "failed"
        mission._persist()

        session_summary = self.load_service().snapshot().missions[0].sessions[0]

        self.assertEqual(
            session_summary.last_activity_at,
            "2026-07-12T08:31:45+00:00",
        )
        self.assertEqual(
            session_summary.runner_started_at,
            "2026-07-12T08:30:00+00:00",
        )

    def test_workspace_session_summary_uses_cancel_request_as_last_activity(self) -> None:
        snapshots = self.load_service()
        mission = snapshots._primary_mission
        mission.approve_issue("ISS-01")
        session = mission.launch_issue("ISS-01")
        session.runner_started_at = "2026-07-12T08:30:00+00:00"
        session.cancel_requested_at = "2026-07-12T08:31:00+00:00"
        session.status = "failed"
        mission._persist()

        session_summary = self.load_service().snapshot().missions[0].sessions[0]

        self.assertEqual(
            session_summary.last_activity_at,
            "2026-07-12T08:31:00+00:00",
        )

    def test_workspace_session_summary_rejects_malformed_last_activity(self) -> None:
        snapshots = self.load_service()
        mission = snapshots._primary_mission
        mission.approve_issue("ISS-01")
        session = mission.launch_issue("ISS-01")
        session.runner_started_at = "2026-07-12T08:30:00+00:00"
        session.runner_ended_at = "0"
        session.status = "failed"
        mission._persist()

        session_summary = self.load_service().snapshot().missions[0].sessions[0]

        self.assertEqual(
            session_summary.last_activity_at,
            "2026-07-12T08:30:00+00:00",
        )

    def test_background_ad_hoc_summary_restores_task_operation_and_evidence_detail(
        self,
    ) -> None:
        (self.target_repo / ".albert").mkdir()
        (self.target_repo / ".albert" / "agents.json").write_text(
            json.dumps(
                {
                    "agents": [
                        {
                            "id": "background-worker",
                            "role": "local-agent",
                            "provider": "test-harness",
                            "runner": "fake",
                            "model": "background:test",
                            "routing": "worker",
                        }
                    ]
                }
            ),
            encoding="utf-8",
        )
        primary = self.load_service()._primary_mission
        background_tracker = self.root / "background-evidence-tracker"
        (background_tracker / "issues").mkdir(parents=True)
        (background_tracker / "PRD.md").write_text(
            "# Background Evidence Mission\n", encoding="utf-8"
        )
        background = AlbertMission(
            target_repo=self.target_repo,
            tracker_dir=background_tracker,
            runtime_root=self.runtime,
            mission_id="background-evidence",
            allow_empty_tracker=True,
        ).load()
        snapshots = WorkspaceSnapshotService(primary, missions=(background,))
        background_scope = ConversationScope(
            kind="mission",
            target_id="background-evidence",
            label="Background Evidence Mission",
            mission_id="background-evidence",
        )
        origin = AgentConsoleHistoryService(snapshots).append(
            role="user",
            content="Update the background docs.",
            outcome="proposed",
            source="mission-commander",
            expected_revision=1,
            expected_scope=snapshots.snapshot().conversation_scope,
            recorded_scope=background_scope,
        )
        queue = WorkspaceQueueService(snapshots)
        proposal = queue.propose_ad_hoc_delegation(
            correlation_id="background-ad-hoc-proposal-1",
            expected_revision=1,
            source="agent-console",
            scope=background_scope,
            acceptance_criteria=["Background docs are updated."],
            allowed_paths=["docs/background.md"],
            command_policy={},
            proposed_agent="background-worker",
            originating_message_id=origin.message_id,
            mission_id="background-evidence",
        )
        launched = queue.decide(
            correlation_id="background-ad-hoc-approve-1",
            expected_revision=proposal.revision,
            item_id=proposal.item_id,
            decision="approve",
            reason="Run the bounded background task.",
        )
        background.record_evidence(
            launched.session_id,
            EvidencePackage(
                changed_files=["docs/background.md"],
                diff_summary="Updated background documentation.",
                commands_run=["python3 -m unittest tests.test_workspace_snapshot"],
                test_results="Focused tests passed.",
                known_risks="None.",
                proposed_context_updates="None.",
                artifact_links=["app-local://evidence/background/review.diff"],
            ),
        )

        restored_primary = self.load_service()._primary_mission
        restored_background = AlbertMission(
            target_repo=self.target_repo,
            tracker_dir=background_tracker,
            runtime_root=self.runtime,
            mission_id="background-evidence",
            allow_empty_tracker=True,
        ).load()
        restored = WorkspaceSnapshotService(
            restored_primary,
            missions=(restored_background,),
        ).snapshot()
        background_summary = next(
            mission for mission in restored.missions if mission.id == "background-evidence"
        ).sessions[0]

        self.assertEqual(background_summary.task_title, "Update the background docs.")
        self.assertEqual(background_summary.operation_status, "evidence-ready")
        self.assertEqual(background_summary.failure, "")
        self.assertEqual(background_summary.changed_files, ("docs/background.md",))
        self.assertEqual(
            background_summary.commands_run,
            ("python3 -m unittest tests.test_workspace_snapshot",),
        )
        self.assertEqual(background_summary.test_results, "Focused tests passed.")
        self.assertEqual(background_summary.risks, "None.")
        self.assertEqual(
            background_summary.artifact_links,
            ("app-local://evidence/background/review.diff",),
        )

    def test_review_workspace_accepts_ad_hoc_delegation_evidence_without_issue_slice(self) -> None:
        snapshots = self.load_service()
        mission = snapshots._primary_mission
        origin = AgentConsoleHistoryService(snapshots).append(
            role="user",
            content="Run a bounded docs update and submit evidence.",
            outcome="proposed",
            source="mission-commander",
            expected_revision=1,
            expected_scope=snapshots.snapshot().conversation_scope,
        )
        service = WorkspaceQueueService(snapshots)
        proposal = service.propose_ad_hoc_delegation(
            correlation_id="ad-hoc-delegation-review-1",
            expected_revision=1,
            source="agent-console",
            scope=snapshots.snapshot().conversation_scope,
            acceptance_criteria=["Evidence package proves the docs update."],
            allowed_paths=["docs/smoke-tests.md"],
            command_policy={"python3 -m unittest tests.test_workspace_snapshot": "auto-allowed"},
            proposed_agent="qwen-coder-local-1",
            originating_message_id=origin.message_id,
        )
        service.decide(
            correlation_id="ad-hoc-delegation-review-decision-1",
            expected_revision=proposal.revision,
            item_id=proposal.item_id,
            decision="approve",
            reason="Approved for evidence review.",
        )
        evidence = EvidencePackage(
            changed_files=["docs/smoke-tests.md"],
            diff_summary="Updated smoke-test notes.",
            commands_run=["python3 -m unittest tests.test_workspace_snapshot"],
            test_results="Focused workspace tests passed.",
            known_risks="None.",
            proposed_context_updates="Document ad hoc delegation evidence handling.",
            artifact_links=["app-local://evidence/session-ADHOC-000001-1"],
        )
        mission.record_evidence("session-ADHOC-000001-1", evidence)
        mission.record_evidence("session-ADHOC-000001-1", evidence)
        with self.assertRaisesRegex(AlbertError, "different package"):
            mission.record_evidence(
                "session-ADHOC-000001-1",
                replace(evidence, diff_summary="A changed package must not replay."),
            )
        review_service = ReviewWorkspaceService(snapshots)
        review_item = review_service.inspect().items[0]

        acknowledgement = review_service.decide(
            correlation_id="ad-hoc-delegation-review-accept-1",
            expected_revision=snapshots.snapshot().revision,
            session_id="session-ADHOC-000001-1",
            decision="accept",
            reason="Evidence accepted.",
        )

        self.assertEqual(review_item.issue_id, "ADHOC-000001")
        self.assertEqual(review_item.evidence_complete, True)
        self.assertEqual(acknowledgement.review_outcome, "Approved")
        self.assertEqual(acknowledgement.next_action, "complete")
        self.assertEqual(acknowledgement.issue_lifecycle, "Complete")
        self.assertNotIn("ADHOC-000001", mission.issues)
        self.assertEqual(mission.sessions["session-ADHOC-000001-1"].status, "reviewed")
        self.assertEqual(mission.sessions["session-ADHOC-000001-1"].cleanup_eligible, True)
        session_summary = snapshots.snapshot().missions[0].sessions[0]
        self.assertEqual(
            session_summary.launch_correlation_id,
            "ad-hoc-delegation-review-decision-1",
        )
        self.assertEqual(
            session_summary.evidence_correlation_id,
            "evidence:command-deck:session-ADHOC-000001-1",
        )
        evidence_entries = [
            entry
            for entry in ActivityJournalService(snapshots).inspect().entries
            if entry.action_type == "evidence-package-submitted"
        ]
        self.assertEqual(
            [entry.correlation_id for entry in evidence_entries],
            [session_summary.evidence_correlation_id],
        )
        self.assertEqual(
            session_summary.review_correlation_id,
            "ad-hoc-delegation-review-accept-1",
        )

    def test_workspace_session_summary_suppresses_unvalidated_runtime_receipt_identities(
        self,
    ) -> None:
        snapshots = self.load_service()
        mission = snapshots._primary_mission
        origin = AgentConsoleHistoryService(snapshots).append(
            role="user",
            content="Run bounded work with canonical lifecycle receipts.",
            outcome="proposed",
            source="mission-commander",
            expected_revision=1,
            expected_scope=snapshots.snapshot().conversation_scope,
        )
        queue = WorkspaceQueueService(snapshots)
        proposal = queue.propose_ad_hoc_delegation(
            correlation_id="runtime-receipt-proposal-1",
            expected_revision=1,
            source="agent-console",
            scope=snapshots.snapshot().conversation_scope,
            acceptance_criteria=["Receipt identities remain canonical."],
            allowed_paths=["docs/smoke-tests.md"],
            command_policy={},
            proposed_agent="qwen-coder-local-1",
            originating_message_id=origin.message_id,
        )
        queue.decide(
            correlation_id="runtime-receipt-decision-1",
            expected_revision=proposal.revision,
            item_id=proposal.item_id,
            decision="approve",
            reason="Approve bounded receipt validation.",
        )
        session_id = "session-ADHOC-000001-1"
        mission.record_evidence(
            session_id,
            EvidencePackage(
                changed_files=["docs/smoke-tests.md"],
                diff_summary="Updated receipt notes.",
                commands_run=["python3 -m unittest tests.test_workspace_snapshot"],
                test_results="Focused tests passed.",
                known_risks="None.",
                proposed_context_updates="None.",
                artifact_links=[f"app-local://evidence/{session_id}"],
            ),
        )
        ReviewWorkspaceService(snapshots).decide(
            correlation_id="runtime-receipt-review-1",
            expected_revision=snapshots.snapshot().revision,
            session_id=session_id,
            decision="accept",
            reason="Evidence accepted.",
        )
        runtime = json.loads(mission.runtime_path.read_text(encoding="utf-8"))
        runtime["sessions"][session_id]["task_packet"]["queue_approval"][
            "correlation_id"
        ] = "forged-launch-receipt"
        runtime["sessions"][session_id][
            "evidence_correlation_id"
        ] = "forged-evidence-receipt"
        runtime["reviews"][-1]["workspace_action"][
            "correlation_id"
        ] = "forged-review-receipt"
        mission.runtime_path.write_text(json.dumps(runtime), encoding="utf-8")

        summary = self.load_service().snapshot().missions[0].sessions[0]

        self.assertEqual(summary.launch_correlation_id, "")
        self.assertEqual(summary.evidence_correlation_id, "")
        self.assertEqual(summary.review_correlation_id, "")

    def test_mission_draft_records_selected_excluded_and_new_work_without_accepting_state(self) -> None:
        snapshots = self.load_service()
        mission = snapshots._primary_mission
        initial_issue_ids = tuple(mission.issues)
        initial_session_ids = tuple(mission.sessions)
        origin = AgentConsoleHistoryService(snapshots).append(
            role="user",
            content="Turn the useful ad hoc work into a proper Mission Draft.",
            outcome="proposed",
            source="mission-commander",
            expected_revision=1,
            expected_scope=snapshots.snapshot().conversation_scope,
        )
        queue = WorkspaceQueueService(snapshots)
        first = queue.propose_ad_hoc_delegation(
            correlation_id="mission-draft-ad-hoc-1",
            expected_revision=1,
            source="agent-console",
            scope=snapshots.snapshot().conversation_scope,
            acceptance_criteria=["Summarize useful command deck follow-up work."],
            allowed_paths=["docs/command-deck.md"],
            command_policy={},
            proposed_agent="qwen-coder-local-1",
            originating_message_id=origin.message_id,
        )
        queue.propose_ad_hoc_delegation(
            correlation_id="mission-draft-ad-hoc-2",
            expected_revision=1,
            source="agent-console",
            scope=snapshots.snapshot().conversation_scope,
            acceptance_criteria=["Investigate an unrelated packaging idea."],
            allowed_paths=["docs/packaging.md"],
            command_policy={},
            proposed_agent="qwen-coder-local-1",
            originating_message_id=origin.message_id,
        )

        acknowledgement = MissionDraftService(snapshots).create_draft(
            correlation_id="mission-draft-create-1",
            expected_revision=1,
            proposed_goal="Create a focused Command Deck follow-up mission.",
            selected_ad_hoc_ids=["ADHOC-000001"],
            excluded_ad_hoc_ids=["ADHOC-000002"],
            new_work_items=["Add a Mission Draft confirmation path."],
            dependencies=["Issue 10 ad hoc delegation approvals stay authoritative."],
            unresolved_decisions=["Confirm whether packaging belongs in a later mission."],
        )
        reloaded = MissionDraftService(self.load_service()).inspect()

        self.assertEqual(acknowledgement.outcome, "acknowledged")
        self.assertEqual(acknowledgement.draft_id, "mission-draft-command-deck-000001")
        self.assertEqual(acknowledgement.revision, 2)
        self.assertEqual(first.item_id, "ad-hoc-delegation-command-deck-000001")
        self.assertEqual(len(reloaded.drafts), 1)
        draft = reloaded.drafts[0]
        self.assertEqual(draft.draft_id, "mission-draft-command-deck-000001")
        self.assertEqual(draft.status, "draft")
        self.assertEqual(draft.mission_id, "command-deck")
        self.assertEqual(draft.proposed_goal, "Create a focused Command Deck follow-up mission.")
        self.assertEqual([item.work_id for item in draft.included_ad_hoc_work], ["ADHOC-000001"])
        self.assertEqual(
            draft.included_ad_hoc_work[0].acceptance_criteria,
            ("Summarize useful command deck follow-up work.",),
        )
        self.assertEqual(draft.excluded_ad_hoc_work_ids, ("ADHOC-000002",))
        self.assertEqual(draft.new_work_items, ("Add a Mission Draft confirmation path.",))
        self.assertEqual(
            draft.dependencies,
            ("Issue 10 ad hoc delegation approvals stay authoritative.",),
        )
        self.assertEqual(
            draft.unresolved_decisions,
            ("Confirm whether packaging belongs in a later mission.",),
        )
        self.assertEqual(tuple(mission.issues), initial_issue_ids)
        self.assertEqual(tuple(mission.sessions), initial_session_ids)
        self.assertNotIn("ADHOC-000001", mission.issues)
        self.assertEqual(snapshots.snapshot().revision, 1)

    def test_mission_draft_creation_replays_after_lost_audit_response(self) -> None:
        snapshots = self.load_service()
        request = {
            "correlation_id": "mission-draft-create-response-loss-1",
            "expected_revision": 1,
            "proposed_goal": "Recover one Mission Draft after audit response loss.",
            "selected_ad_hoc_ids": [],
            "excluded_ad_hoc_ids": [],
            "new_work_items": ["Persist one draft and one audit entry."],
            "dependencies": [],
            "unresolved_decisions": [],
        }

        with patch.object(
            ActivityJournalService,
            "record_mission_draft_created",
            side_effect=OSError("simulated Mission Draft audit response loss"),
        ):
            with self.assertRaisesRegex(OSError, "Mission Draft audit response loss"):
                MissionDraftService(snapshots).create_draft(**request)

        replayed = MissionDraftService(self.load_service()).create_draft(**request)
        replayed_again = MissionDraftService(self.load_service()).create_draft(**request)

        self.assertEqual(replayed, replayed_again)
        self.assertEqual(len(MissionDraftService(self.load_service()).inspect().drafts), 1)
        entries = ActivityJournalService(self.load_service()).inspect().entries
        self.assertEqual(
            [entry.correlation_id for entry in entries],
            ["mission-draft-create-response-loss-1"],
        )

    def test_mission_draft_replay_rejects_forged_request_effect_and_acknowledgement(
        self,
    ) -> None:
        snapshots = self.load_service()
        service = MissionDraftService(snapshots)
        request = {
            "correlation_id": "mission-draft-forged-create-1",
            "expected_revision": 1,
            "proposed_goal": "Keep every Mission Draft receipt semantically bound.",
            "selected_ad_hoc_ids": [],
            "excluded_ad_hoc_ids": [],
            "new_work_items": ["Reject a forged replay before any effect."],
            "dependencies": [],
            "unresolved_decisions": [],
        }
        service.create_draft(**request)
        original = json.loads(service.drafts_path.read_text(encoding="utf-8"))
        receipt = original["receipts"][0]

        cases = (
            (
                "request fields",
                {
                    **receipt,
                    "request": {
                        **receipt["request"],
                        "proposed_goal": "A forged request must not replay.",
                    },
                },
                {**request, "proposed_goal": "A forged request must not replay."},
            ),
            (
                "effect fields",
                {
                    **receipt,
                    "effect_draft": {
                        **receipt["effect_draft"],
                        "proposed_goal": "A forged effect must not replay.",
                    },
                },
                request,
            ),
            (
                "acknowledgement revision",
                {
                    **receipt,
                    "acknowledgement": {
                        **receipt["acknowledgement"],
                        "revision": original["revision"] + 10,
                    },
                },
                request,
            ),
            (
                "acknowledgement status",
                {
                    **receipt,
                    "acknowledgement": {
                        **receipt["acknowledgement"],
                        "draft_status": "abandoned",
                    },
                },
                request,
            ),
        )
        for label, forged_receipt, replay_request in cases:
            with self.subTest(label=label):
                payload = {
                    **original,
                    "receipts": [forged_receipt],
                }
                service.drafts_path.write_text(json.dumps(payload), encoding="utf-8")
                with self.assertRaisesRegex(
                    WorkspacePersistenceError,
                    "Mission Draft receipt",
                ):
                    MissionDraftService(self.load_service()).create_draft(
                        **replay_request
                    )
        service.drafts_path.write_text(json.dumps(original), encoding="utf-8")

    def test_mission_draft_replay_rejects_forged_update_and_abandon_receipts(
        self,
    ) -> None:
        service = MissionDraftService(self.load_service())
        created = service.create_draft(
            correlation_id="mission-draft-forged-lifecycle-create-1",
            expected_revision=1,
            proposed_goal="Validate every Mission Draft lifecycle receipt.",
            selected_ad_hoc_ids=[],
            excluded_ad_hoc_ids=[],
            new_work_items=["Create the initial proposal."],
            dependencies=[],
            unresolved_decisions=[],
        )
        update_request = {
            "correlation_id": "mission-draft-forged-lifecycle-update-1",
            "expected_revision": created.revision,
            "draft_id": created.draft_id,
            "proposed_goal": "Validate the updated Mission Draft receipt.",
            "selected_ad_hoc_ids": [],
            "excluded_ad_hoc_ids": [],
            "new_work_items": ["Persist one exact update."],
            "dependencies": [],
            "unresolved_decisions": [],
        }
        updated = service.update_draft(**update_request)
        abandon_request = {
            "correlation_id": "mission-draft-forged-lifecycle-abandon-1",
            "expected_revision": updated.revision,
            "draft_id": created.draft_id,
            "reason": "Conclude the exact lifecycle.",
        }
        service.abandon_draft(**abandon_request)
        original = json.loads(service.drafts_path.read_text(encoding="utf-8"))
        update_receipt = next(
            item
            for item in original["receipts"]
            if item["correlation_id"] == update_request["correlation_id"]
        )
        abandon_receipt = next(
            item
            for item in original["receipts"]
            if item["correlation_id"] == abandon_request["correlation_id"]
        )
        cases = (
            (
                "update request",
                update_receipt,
                {
                    **update_receipt,
                    "request": {
                        **update_receipt["request"],
                        "proposed_goal": "A forged update request.",
                    },
                },
                "update",
                {**update_request, "proposed_goal": "A forged update request."},
            ),
            (
                "update effect",
                update_receipt,
                {
                    **update_receipt,
                    "effect_draft": {
                        **update_receipt["effect_draft"],
                        "new_work_items": ["A forged update effect."],
                    },
                },
                "update",
                update_request,
            ),
            (
                "abandon request",
                abandon_receipt,
                {
                    **abandon_receipt,
                    "request": {
                        **abandon_receipt["request"],
                        "reason": "A forged abandonment reason.",
                    },
                },
                "abandon",
                {**abandon_request, "reason": "A forged abandonment reason."},
            ),
            (
                "abandon effect",
                abandon_receipt,
                {
                    **abandon_receipt,
                    "effect_draft": {
                        **abandon_receipt["effect_draft"],
                        "proposed_goal": "A forged abandoned effect.",
                    },
                },
                "abandon",
                abandon_request,
            ),
        )
        for label, original_receipt, forged_receipt, action, replay_request in cases:
            with self.subTest(label=label):
                payload = {
                    **original,
                    "receipts": [
                        forged_receipt if item is original_receipt else item
                        for item in original["receipts"]
                    ],
                }
                service.drafts_path.write_text(json.dumps(payload), encoding="utf-8")
                with self.assertRaisesRegex(
                    WorkspacePersistenceError,
                    "Mission Draft receipt",
                ):
                    replay_service = MissionDraftService(self.load_service())
                    if action == "update":
                        replay_service.update_draft(**replay_request)
                    else:
                        replay_service.abandon_draft(**replay_request)
        service.drafts_path.write_text(json.dumps(original), encoding="utf-8")

    def test_mission_draft_receipt_chain_rejects_coherent_lifecycle_substitution(
        self,
    ) -> None:
        service = MissionDraftService(self.load_service())
        created = service.create_draft(
            correlation_id="mission-draft-chain-create-1",
            expected_revision=1,
            proposed_goal="Bind the original receipt into one canonical chain.",
            selected_ad_hoc_ids=[],
            excluded_ad_hoc_ids=[],
            new_work_items=["Create the original chain state."],
            dependencies=[],
            unresolved_decisions=[],
        )
        service.update_draft(
            correlation_id="mission-draft-chain-update-1",
            expected_revision=created.revision,
            draft_id=created.draft_id,
            proposed_goal="Bind the updated receipt to canonical draft state.",
            selected_ad_hoc_ids=[],
            excluded_ad_hoc_ids=[],
            new_work_items=["Persist the canonical updated state."],
            dependencies=[],
            unresolved_decisions=[],
        )
        original = json.loads(service.drafts_path.read_text(encoding="utf-8"))
        create_receipt = next(
            item
            for item in original["receipts"]
            if item["correlation_id"] == "mission-draft-chain-create-1"
        )
        update_receipt = next(
            item
            for item in original["receipts"]
            if item["correlation_id"] == "mission-draft-chain-update-1"
        )

        forged_create = json.loads(json.dumps(create_receipt))
        forged_create["request"]["proposed_goal"] = "Coherently forged create state."
        forged_create["effect_draft"]["proposed_goal"] = (
            "Coherently forged create state."
        )
        forged_update = json.loads(json.dumps(update_receipt))
        forged_update["request"]["proposed_goal"] = "Coherently forged update state."
        forged_update["effect_draft"]["proposed_goal"] = (
            "Coherently forged update state."
        )

        for label, original_receipt, forged_receipt in (
            ("create", create_receipt, forged_create),
            ("update", update_receipt, forged_update),
        ):
            with self.subTest(label=label):
                payload = {
                    **original,
                    "receipts": [
                        forged_receipt if item is original_receipt else item
                        for item in original["receipts"]
                    ],
                }
                service.drafts_path.write_text(json.dumps(payload), encoding="utf-8")
                with self.assertRaisesRegex(
                    WorkspacePersistenceError,
                    "Mission Draft receipt chain",
                ):
                    MissionDraftService(self.load_service()).inspect()
        service.drafts_path.write_text(json.dumps(original), encoding="utf-8")

    def test_mission_draft_v2_receipt_cannot_be_downgraded_to_legacy(self) -> None:
        service = MissionDraftService(self.load_service())
        service.create_draft(
            correlation_id="mission-draft-nondowngrade-create-1",
            expected_revision=1,
            proposed_goal="Keep current receipt integrity non-downgradeable.",
            selected_ad_hoc_ids=[],
            excluded_ad_hoc_ids=[],
            new_work_items=["Reject removal of current receipt metadata."],
            dependencies=[],
            unresolved_decisions=[],
        )
        payload = json.loads(service.drafts_path.read_text(encoding="utf-8"))
        receipt = payload["receipts"][0]
        receipt.pop("receipt_version")
        receipt.pop("prior_draft")
        service.drafts_path.write_text(json.dumps(payload), encoding="utf-8")

        with self.assertRaisesRegex(
            WorkspacePersistenceError,
            "downgrade",
        ):
            MissionDraftService(self.load_service()).inspect()

    def test_mission_draft_v2_create_receipt_requires_explicit_null_prior(self) -> None:
        service = MissionDraftService(self.load_service())
        service.create_draft(
            correlation_id="mission-draft-prior-create-1",
            expected_revision=1,
            proposed_goal="Require an explicit root for the receipt chain.",
            selected_ad_hoc_ids=[],
            excluded_ad_hoc_ids=[],
            new_work_items=["Reject a missing prior-effect field."],
            dependencies=[],
            unresolved_decisions=[],
        )
        payload = json.loads(service.drafts_path.read_text(encoding="utf-8"))
        payload["receipts"][0].pop("prior_draft")
        service.drafts_path.write_text(json.dumps(payload), encoding="utf-8")

        with self.assertRaisesRegex(
            WorkspacePersistenceError,
            "prior effect",
        ):
            MissionDraftService(self.load_service()).inspect()

    def test_mission_draft_current_store_rejects_canonical_draft_without_receipt_chain(
        self,
    ) -> None:
        service = MissionDraftService(self.load_service())
        service.create_draft(
            correlation_id="mission-draft-unreceipted-create-1",
            expected_revision=1,
            proposed_goal="Keep every current canonical draft receipt-derived.",
            selected_ad_hoc_ids=[],
            excluded_ad_hoc_ids=[],
            new_work_items=["Reject fabricated canonical draft state."],
            dependencies=[],
            unresolved_decisions=[],
        )
        payload = json.loads(service.drafts_path.read_text(encoding="utf-8"))
        fabricated = json.loads(json.dumps(payload["drafts"][0]))
        fabricated["draft_id"] = "mission-draft-command-deck-999999"
        fabricated["proposed_goal"] = "Fabricated unreceipted draft state."
        payload["drafts"].append(fabricated)
        service.drafts_path.write_text(json.dumps(payload), encoding="utf-8")

        with self.assertRaisesRegex(
            WorkspacePersistenceError,
            "canonical coverage",
        ):
            MissionDraftService(self.load_service()).inspect()

    def test_mission_draft_legacy_receipts_migrate_as_fixed_compatibility_prefix(
        self,
    ) -> None:
        service = MissionDraftService(self.load_service())
        created = service.create_draft(
            correlation_id="mission-draft-legacy-create-1",
            expected_revision=1,
            proposed_goal="Preserve one valid legacy Mission Draft.",
            selected_ad_hoc_ids=[],
            excluded_ad_hoc_ids=[],
            new_work_items=["Migrate the legacy receipt prefix."],
            dependencies=[],
            unresolved_decisions=[],
        )
        updated = service.update_draft(
            correlation_id="mission-draft-legacy-update-1",
            expected_revision=created.revision,
            draft_id=created.draft_id,
            proposed_goal="Preserve the valid updated legacy draft.",
            selected_ad_hoc_ids=[],
            excluded_ad_hoc_ids=[],
            new_work_items=["Keep later current receipts outside the legacy prefix."],
            dependencies=[],
            unresolved_decisions=[],
        )
        legacy = json.loads(service.drafts_path.read_text(encoding="utf-8"))
        legacy.pop("receipt_protocol_version")
        legacy.pop("legacy_receipt_count")
        legacy.pop("legacy_draft_ids")
        for receipt in legacy["receipts"]:
            receipt.pop("receipt_version")
            receipt.pop("prior_draft")
        service.drafts_path.write_text(json.dumps(legacy), encoding="utf-8")

        projection = MissionDraftService(self.load_service()).inspect()
        current = MissionDraftService(self.load_service()).update_draft(
            correlation_id="mission-draft-current-after-legacy-1",
            expected_revision=updated.revision,
            draft_id=created.draft_id,
            proposed_goal="Append one current receipt after the compatibility prefix.",
            selected_ad_hoc_ids=[],
            excluded_ad_hoc_ids=[],
            new_work_items=["Retain non-downgradeable current integrity."],
            dependencies=[],
            unresolved_decisions=[],
        )
        persisted = json.loads(service.drafts_path.read_text(encoding="utf-8"))

        self.assertEqual(projection.drafts[0].proposed_goal, legacy["drafts"][0]["proposed_goal"])
        self.assertEqual(current.revision, updated.revision + 1)
        self.assertEqual(persisted["receipt_protocol_version"], 2)
        self.assertEqual(persisted["legacy_receipt_count"], 2)
        self.assertEqual(
            [receipt["receipt_version"] for receipt in persisted["receipts"]],
            [1, 1, 2],
        )

    def test_mission_draft_revision_keeps_unselected_ad_hoc_work_outside(self) -> None:
        snapshots = self.load_service()
        mission = snapshots._primary_mission
        origin = AgentConsoleHistoryService(snapshots).append(
            role="user",
            content="Revise the Mission Draft after more ad hoc work appears.",
            outcome="proposed",
            source="mission-commander",
            expected_revision=1,
            expected_scope=snapshots.snapshot().conversation_scope,
        )
        queue = WorkspaceQueueService(snapshots)
        for correlation_id, criteria, path in [
            ("mission-draft-revision-ad-hoc-1", "Keep this work in the draft.", "docs/included.md"),
            ("mission-draft-revision-ad-hoc-2", "Keep this work excluded.", "docs/excluded.md"),
            ("mission-draft-revision-ad-hoc-3", "This later work is unrelated.", "docs/unselected.md"),
        ]:
            queue.propose_ad_hoc_delegation(
                correlation_id=correlation_id,
                expected_revision=1,
                source="agent-console",
                scope=snapshots.snapshot().conversation_scope,
                acceptance_criteria=[criteria],
                allowed_paths=[path],
                command_policy={},
                proposed_agent="qwen-coder-local-1",
                originating_message_id=origin.message_id,
            )
        service = MissionDraftService(snapshots)
        created = service.create_draft(
            correlation_id="mission-draft-revision-create-1",
            expected_revision=1,
            proposed_goal="Create a focused Command Deck follow-up mission.",
            selected_ad_hoc_ids=["ADHOC-000001"],
            excluded_ad_hoc_ids=["ADHOC-000002"],
            new_work_items=["Add a confirmation path."],
            dependencies=[],
            unresolved_decisions=[],
        )

        acknowledgement = service.update_draft(
            correlation_id="mission-draft-revision-update-1",
            expected_revision=created.revision,
            draft_id=created.draft_id,
            proposed_goal="Create a focused Command Deck confirmation mission.",
            selected_ad_hoc_ids=["ADHOC-000001"],
            excluded_ad_hoc_ids=["ADHOC-000002"],
            new_work_items=["Add a confirmation path.", "Add rejection handling."],
            dependencies=["Issue 10 remains the source for ad hoc delegation records."],
            unresolved_decisions=[],
        )
        reloaded = MissionDraftService(self.load_service()).inspect().drafts[0]

        self.assertEqual(acknowledgement.outcome, "acknowledged")
        self.assertEqual(acknowledgement.revision, created.revision + 1)
        self.assertEqual(reloaded.proposed_goal, "Create a focused Command Deck confirmation mission.")
        self.assertEqual([item.work_id for item in reloaded.included_ad_hoc_work], ["ADHOC-000001"])
        self.assertEqual(reloaded.excluded_ad_hoc_work_ids, ("ADHOC-000002",))
        self.assertNotIn("ADHOC-000003", [item.work_id for item in reloaded.included_ad_hoc_work])
        self.assertNotIn("ADHOC-000003", reloaded.excluded_ad_hoc_work_ids)
        self.assertEqual(
            reloaded.new_work_items,
            ("Add a confirmation path.", "Add rejection handling."),
        )
        self.assertEqual(tuple(mission.sessions), ())
        self.assertNotIn("ADHOC-000003", mission.issues)

    def test_mission_draft_confirmation_creates_accepted_issue_slice(self) -> None:
        snapshots = self.load_service()
        mission = snapshots._primary_mission
        origin = AgentConsoleHistoryService(snapshots).append(
            role="user",
            content="Confirm the draft as accepted mission work.",
            outcome="proposed",
            source="mission-commander",
            expected_revision=1,
            expected_scope=snapshots.snapshot().conversation_scope,
        )
        queue = WorkspaceQueueService(snapshots)
        queue.propose_ad_hoc_delegation(
            correlation_id="mission-draft-confirm-ad-hoc-1",
            expected_revision=1,
            source="agent-console",
            scope=snapshots.snapshot().conversation_scope,
            acceptance_criteria=["Ad hoc evidence informs the confirmed mission."],
            allowed_paths=["docs/confirmed.md"],
            command_policy={},
            proposed_agent="qwen-coder-local-1",
            originating_message_id=origin.message_id,
        )
        service = MissionDraftService(snapshots)
        created = service.create_draft(
            correlation_id="mission-draft-confirm-create-1",
            expected_revision=1,
            proposed_goal="Create the confirmed Command Deck mission scope.",
            selected_ad_hoc_ids=["ADHOC-000001"],
            excluded_ad_hoc_ids=[],
            new_work_items=["Add a confirmation path."],
            dependencies=["Draft review is complete."],
            unresolved_decisions=["Choose the final UI placement."],
        )

        acknowledgement = service.confirm_draft(
            correlation_id="mission-draft-confirm-1",
            expected_revision=created.revision,
            draft_id=created.draft_id,
            reason="Mission Commander confirmed the draft.",
        )
        reloaded_service = self.load_service()
        reloaded_draft = MissionDraftService(reloaded_service).inspect().drafts[0]
        accepted_issue = reloaded_service._primary_mission.issues["ISS-02"]

        self.assertEqual(acknowledgement.outcome, "acknowledged")
        self.assertEqual(acknowledgement.draft_status, "confirmed")
        self.assertEqual(acknowledgement.accepted_issue_id, "ISS-02")
        self.assertEqual(reloaded_draft.status, "confirmed")
        self.assertEqual(accepted_issue.title, "Create The Confirmed Command Deck Mission Scope")
        self.assertEqual(
            accepted_issue.what_to_build,
            "Create the confirmed Command Deck mission scope.",
        )
        self.assertIn("Include Ad Hoc Delegation ADHOC-000001.", accepted_issue.acceptance_criteria)
        self.assertIn("New work: Add a confirmation path.", accepted_issue.acceptance_criteria)
        self.assertIn("Dependency: Draft review is complete.", accepted_issue.acceptance_criteria)
        self.assertIn("Resolve decision: Choose the final UI placement.", accepted_issue.acceptance_criteria)
        self.assertEqual(tuple(mission.sessions), ())
        self.assertNotIn("ADHOC-000001", mission.issues)

    def test_mission_draft_confirmation_replay_preserves_newer_unrelated_issue_runtime(
        self,
    ) -> None:
        stale_snapshots = self.load_service()
        stale_drafts = MissionDraftService(stale_snapshots)
        created = stale_drafts.create_draft(
            correlation_id="mission-draft-stale-replay-create-1",
            expected_revision=1,
            proposed_goal="Accept one replay-safe Mission Draft.",
            selected_ad_hoc_ids=[],
            excluded_ad_hoc_ids=[],
            new_work_items=["Preserve newer unrelated Issue Slice runtime."],
            dependencies=[],
            unresolved_decisions=[],
        )
        request = {
            "correlation_id": "mission-draft-stale-replay-confirm-1",
            "expected_revision": created.revision,
            "draft_id": created.draft_id,
            "reason": "Confirm this draft exactly once.",
        }
        acknowledged = stale_drafts.confirm_draft(**request)

        newer = self.load_service()
        newer._primary_mission.assign_issue(
            "ISS-01",
            "newer-unrelated-agent",
            notes="This newer unrelated runtime state must survive confirmation replay.",
        )

        replayed = stale_drafts.confirm_draft(**request)
        restored = self.load_service()._primary_mission

        self.assertEqual(replayed, acknowledged)
        self.assertEqual(
            restored.issue_detail("ISS-01")["assigned_agent"],
            "newer-unrelated-agent",
        )
        self.assertEqual(
            restored.issue_detail("ISS-01")["notes"],
            "This newer unrelated runtime state must survive confirmation replay.",
        )
        self.assertEqual(restored.issue_detail("ISS-02")["issue_id"], "ISS-02")

    def test_preloaded_mission_draft_services_allocate_distinct_confirmed_issue_slices(
        self,
    ) -> None:
        drafts = MissionDraftService(self.load_service())
        first_draft = drafts.create_draft(
            correlation_id="mission-draft-distinct-create-1",
            expected_revision=1,
            proposed_goal="Accept the first distinct Mission Draft.",
            selected_ad_hoc_ids=[],
            excluded_ad_hoc_ids=[],
            new_work_items=["Create the first accepted Issue Slice."],
            dependencies=[],
            unresolved_decisions=[],
        )
        second_draft = drafts.create_draft(
            correlation_id="mission-draft-distinct-create-2",
            expected_revision=1,
            proposed_goal="Accept the second distinct Mission Draft.",
            selected_ad_hoc_ids=[],
            excluded_ad_hoc_ids=[],
            new_work_items=["Create the second accepted Issue Slice."],
            dependencies=[],
            unresolved_decisions=[],
        )
        first_service = MissionDraftService(self.load_service())
        second_service = MissionDraftService(self.load_service())
        initial_revision = second_draft.revision

        first = first_service.confirm_draft(
            correlation_id="mission-draft-distinct-confirm-1",
            expected_revision=initial_revision,
            draft_id=first_draft.draft_id,
            reason="Confirm the first distinct draft.",
        )
        second = second_service.confirm_draft(
            correlation_id="mission-draft-distinct-confirm-2",
            expected_revision=first.revision,
            draft_id=second_draft.draft_id,
            reason="Confirm the second distinct draft.",
        )
        restored = self.load_service()._primary_mission

        self.assertEqual(first.accepted_issue_id, "ISS-02")
        self.assertEqual(second.accepted_issue_id, "ISS-03")
        self.assertEqual(sorted(restored.issues), ["ISS-01", "ISS-02", "ISS-03"])
        self.assertEqual(
            sorted(path.name for path in (self.tracker / "issues").glob("0[23]-*.md")),
            [
                "02-accept-the-first-distinct-mission-draft.md",
                "03-accept-the-second-distinct-mission-draft.md",
            ],
        )

    def test_mission_draft_confirmation_recovers_from_draft_store_failure_without_duplicates(
        self,
    ) -> None:
        snapshots = self.load_service()
        drafts = MissionDraftService(snapshots)
        created = drafts.create_draft(
            correlation_id="mission-draft-store-failure-create-1",
            expected_revision=1,
            proposed_goal="Recover one confirmed Issue Slice after draft-store failure.",
            selected_ad_hoc_ids=[],
            excluded_ad_hoc_ids=[],
            new_work_items=["Create exactly one accepted Issue Slice."],
            dependencies=[],
            unresolved_decisions=[],
        )
        request = {
            "correlation_id": "mission-draft-store-failure-confirm-1",
            "expected_revision": created.revision,
            "draft_id": created.draft_id,
            "reason": "Confirm this scope exactly once.",
        }
        original_write = WorkspaceSnapshotService._write_json_atomically
        failed = False

        def fail_first_confirmed_draft_write(path: Path, payload: dict[str, object]) -> None:
            nonlocal failed
            if path == drafts.drafts_path and not failed:
                failed = True
                raise OSError("simulated confirmed Mission Draft store failure")
            original_write(path, payload)

        with patch.object(
            WorkspaceSnapshotService,
            "_write_json_atomically",
            side_effect=fail_first_confirmed_draft_write,
        ):
            with self.assertRaisesRegex(OSError, "Mission Draft store failure"):
                drafts.confirm_draft(**request)

        replayed = drafts.confirm_draft(**request)
        replayed_again = MissionDraftService(self.load_service()).confirm_draft(**request)
        restored = self.load_service()

        self.assertEqual(replayed, replayed_again)
        self.assertEqual(replayed.accepted_issue_id, "ISS-02")
        self.assertEqual(
            sorted(issue_id for issue_id in restored._primary_mission.issues if issue_id != "ISS-01"),
            ["ISS-02"],
        )
        self.assertEqual(
            restored._primary_mission.timeline.count(
                "ISS-02 created from Mission Draft "
                f"{created.draft_id}: Confirm this scope exactly once."
            ),
            1,
        )
        self.assertEqual(
            len(list((self.tracker / "issues").glob("02-*.md"))),
            1,
        )
        with self.assertRaisesRegex(AlbertError, "different request"):
            MissionDraftService(self.load_service()).confirm_draft(
                **{**request, "reason": "A changed confirmation must not replay."}
            )

    def test_mission_draft_confirmation_replays_after_mission_persistence_failure(
        self,
    ) -> None:
        snapshots = self.load_service()
        mission = snapshots._primary_mission
        drafts = MissionDraftService(snapshots)
        created = drafts.create_draft(
            correlation_id="mission-draft-runtime-failure-create-1",
            expected_revision=1,
            proposed_goal="Recover accepted Mission state from a durable draft receipt.",
            selected_ad_hoc_ids=[],
            excluded_ad_hoc_ids=[],
            new_work_items=["Persist the accepted Issue Slice exactly once."],
            dependencies=[],
            unresolved_decisions=[],
        )
        request = {
            "correlation_id": "mission-draft-runtime-failure-confirm-1",
            "expected_revision": created.revision,
            "draft_id": created.draft_id,
            "reason": "Accept this recovered scope.",
        }

        with patch.object(
            mission,
            "_persist",
            side_effect=OSError("simulated Mission runtime persistence failure"),
        ):
            with self.assertRaisesRegex(OSError, "Mission runtime persistence failure"):
                drafts.confirm_draft(**request)

        replayed = drafts.confirm_draft(**request)
        replayed_again = MissionDraftService(self.load_service()).confirm_draft(**request)
        restored = self.load_service()

        self.assertEqual(replayed, replayed_again)
        self.assertEqual(replayed.accepted_issue_id, "ISS-02")
        self.assertEqual(
            sorted(issue_id for issue_id in restored._primary_mission.issues if issue_id != "ISS-01"),
            ["ISS-02"],
        )
        self.assertEqual(
            restored._primary_mission.timeline.count(
                "ISS-02 created from Mission Draft "
                f"{created.draft_id}: Accept this recovered scope."
            ),
            1,
        )
        confirmation_entries = [
            entry
            for entry in ActivityJournalService(restored).inspect().entries
            if entry.action_type == "mission-draft-confirmed"
        ]
        self.assertEqual(len(confirmation_entries), 1)

    def test_mission_draft_inspection_recovers_confirmed_issue_after_crash(
        self,
    ) -> None:
        snapshots = self.load_service()
        drafts = MissionDraftService(snapshots)
        created = drafts.create_draft(
            correlation_id="mission-draft-crash-recovery-create-1",
            expected_revision=1,
            proposed_goal="Recover the accepted Issue Slice after a process crash.",
            selected_ad_hoc_ids=[],
            excluded_ad_hoc_ids=[],
            new_work_items=["Create exactly one recoverable Issue Slice."],
            dependencies=[],
            unresolved_decisions=[],
        )
        request = {
            "correlation_id": "mission-draft-crash-recovery-confirm-1",
            "expected_revision": created.revision,
            "draft_id": created.draft_id,
            "reason": "Accept this draft despite a lost effect response.",
        }

        with patch.object(
            MissionDraftService,
            "_write_text_atomically",
            side_effect=OSError("simulated process crash before Issue persistence"),
        ):
            with self.assertRaisesRegex(OSError, "before Issue persistence"):
                drafts.confirm_draft(**request)

        before_recovery = self.load_service()
        self.assertEqual(sorted(before_recovery._primary_mission.issues), ["ISS-01"])

        recovered_service = self.load_service()
        projection = MissionDraftService(recovered_service).inspect()
        self.assertEqual(
            sorted(recovered_service._primary_mission.issues),
            ["ISS-01", "ISS-02"],
        )
        replayed = MissionDraftService(recovered_service).confirm_draft(**request)
        restored = self.load_service()

        self.assertEqual(projection.drafts[0].status, "confirmed")
        self.assertEqual(replayed.accepted_issue_id, "ISS-02")
        self.assertEqual(sorted(restored._primary_mission.issues), ["ISS-01", "ISS-02"])
        self.assertEqual(
            restored._primary_mission.timeline.count(
                "ISS-02 created from Mission Draft "
                f"{created.draft_id}: Accept this draft despite a lost effect response."
            ),
            1,
        )
        self.assertEqual(len(list((self.tracker / "issues").glob("02-*.md"))), 1)
        self.assertEqual(
            [
                entry.action_type
                for entry in ActivityJournalService(restored).inspect().entries
            ],
            ["mission-draft-created", "mission-draft-confirmed"],
        )

    def test_mission_draft_confirm_replay_rejects_forged_accepted_issue_before_mutation(
        self,
    ) -> None:
        snapshots = self.load_service()
        drafts = MissionDraftService(snapshots)
        created = drafts.create_draft(
            correlation_id="mission-draft-forged-confirm-create-1",
            expected_revision=1,
            proposed_goal="Keep the accepted Issue identity bound to confirmation.",
            selected_ad_hoc_ids=[],
            excluded_ad_hoc_ids=[],
            new_work_items=["Reject a substituted accepted Issue identity."],
            dependencies=[],
            unresolved_decisions=[],
        )
        request = {
            "correlation_id": "mission-draft-forged-confirm-1",
            "expected_revision": created.revision,
            "draft_id": created.draft_id,
            "reason": "Confirm exactly one accepted Issue Slice.",
        }
        drafts.confirm_draft(**request)
        payload = json.loads(drafts.drafts_path.read_text(encoding="utf-8"))
        receipt = next(
            item
            for item in payload["receipts"]
            if item["correlation_id"] == request["correlation_id"]
        )
        receipt["acknowledgement"]["accepted_issue_id"] = "ISS-99"
        receipt["acknowledgement"]["effect_summary"] = (
            f"Mission Draft {created.draft_id} confirmed as accepted Issue Slice ISS-99."
        )
        drafts.drafts_path.write_text(json.dumps(payload), encoding="utf-8")

        with self.assertRaisesRegex(
            WorkspacePersistenceError,
            "accepted Issue",
        ):
            MissionDraftService(self.load_service()).confirm_draft(**request)

        self.assertEqual(
            sorted(self.load_service()._primary_mission.issues),
            ["ISS-01", "ISS-02"],
        )
        self.assertEqual(len(list((self.tracker / "issues").glob("99-*.md"))), 0)

    def test_mission_draft_recovery_preserves_later_governed_issue_evolution(
        self,
    ) -> None:
        snapshots = self.load_service()
        drafts = MissionDraftService(snapshots)
        created = drafts.create_draft(
            correlation_id="mission-draft-governed-evolution-create-1",
            expected_revision=1,
            proposed_goal="Accept work that may evolve through governance.",
            selected_ad_hoc_ids=[],
            excluded_ad_hoc_ids=[],
            new_work_items=["Create the original accepted Issue Slice."],
            dependencies=[],
            unresolved_decisions=[],
        )
        confirmation_request = {
            "correlation_id": "mission-draft-governed-evolution-confirm-1",
            "expected_revision": created.revision,
            "draft_id": created.draft_id,
            "reason": "Accept the original draft boundary.",
        }
        acknowledged = drafts.confirm_draft(**confirmation_request)

        evolved = self.load_service()
        evolved._primary_mission.approve_issue(acknowledged.accepted_issue_id)
        queue = WorkspaceQueueService(evolved)
        proposal = queue.propose_issue_contract_change(
            correlation_id="mission-draft-governed-evolution-proposal-1",
            expected_revision=1,
            issue_id=acknowledged.accepted_issue_id,
            source="issue-slice-inspector",
            acceptance_criteria=["The later governed criterion remains authoritative."],
        )
        queue.decide(
            correlation_id="mission-draft-governed-evolution-decision-1",
            expected_revision=proposal.revision,
            item_id=proposal.item_id,
            decision="approve",
            reason="Apply a legitimate later governed evolution.",
        )

        reloaded = self.load_service()
        projection = MissionDraftService(reloaded).inspect()
        replayed = MissionDraftService(reloaded).confirm_draft(
            **confirmation_request
        )
        restored = self.load_service()

        self.assertEqual(projection.drafts[0].status, "confirmed")
        self.assertEqual(replayed, acknowledged)
        self.assertEqual(
            restored._primary_mission.issues[acknowledged.accepted_issue_id].acceptance_criteria,
            ["The later governed criterion remains authoritative."],
        )
        self.assertEqual(
            len(
                [
                    entry
                    for entry in restored._primary_mission.timeline
                    if f"created from Mission Draft {created.draft_id}" in entry
                ]
            ),
            1,
        )
        self.assertEqual(len(list((self.tracker / "issues").glob("02-*.md"))), 1)

    def test_mission_draft_confirm_replay_rejects_reason_substitution_without_duplicate_timeline(
        self,
    ) -> None:
        service = MissionDraftService(self.load_service())
        created = service.create_draft(
            correlation_id="mission-draft-reason-confirm-create-1",
            expected_revision=1,
            proposed_goal="Bind confirmation reason to one timeline marker.",
            selected_ad_hoc_ids=[],
            excluded_ad_hoc_ids=[],
            new_work_items=["Persist one exact confirmation reason."],
            dependencies=[],
            unresolved_decisions=[],
        )
        service.confirm_draft(
            correlation_id="mission-draft-reason-confirm-1",
            expected_revision=created.revision,
            draft_id=created.draft_id,
            reason="The original confirmation reason.",
        )
        payload = json.loads(service.drafts_path.read_text(encoding="utf-8"))
        receipt = next(
            item
            for item in payload["receipts"]
            if item["correlation_id"] == "mission-draft-reason-confirm-1"
        )
        receipt["request"]["reason"] = "A substituted confirmation reason."
        service.drafts_path.write_text(json.dumps(payload), encoding="utf-8")

        with self.assertRaisesRegex(
            WorkspacePersistenceError,
            "timeline marker",
        ):
            MissionDraftService(self.load_service()).inspect()

        self.assertEqual(
            [
                entry
                for entry in self.load_service()._primary_mission.timeline
                if f"created from Mission Draft {created.draft_id}" in entry
            ],
            [
                "ISS-02 created from Mission Draft "
                f"{created.draft_id}: The original confirmation reason."
            ],
        )

    def test_mission_draft_abandon_replay_rejects_reason_substitution(
        self,
    ) -> None:
        service = MissionDraftService(self.load_service())
        created = service.create_draft(
            correlation_id="mission-draft-reason-abandon-create-1",
            expected_revision=1,
            proposed_goal="Bind abandonment reason to its audit effect.",
            selected_ad_hoc_ids=[],
            excluded_ad_hoc_ids=[],
            new_work_items=["Persist one exact abandonment reason."],
            dependencies=[],
            unresolved_decisions=[],
        )
        service.abandon_draft(
            correlation_id="mission-draft-reason-abandon-1",
            expected_revision=created.revision,
            draft_id=created.draft_id,
            reason="The original abandonment reason.",
        )
        payload = json.loads(service.drafts_path.read_text(encoding="utf-8"))
        receipt = next(
            item
            for item in payload["receipts"]
            if item["correlation_id"] == "mission-draft-reason-abandon-1"
        )
        receipt["request"]["reason"] = "A substituted abandonment reason."
        service.drafts_path.write_text(json.dumps(payload), encoding="utf-8")

        with self.assertRaisesRegex(
            WorkspacePersistenceError,
            "abandonment reason",
        ):
            MissionDraftService(self.load_service()).inspect()

        abandon_entries = [
            entry
            for entry in ActivityJournalService(self.load_service()).inspect().entries
            if entry.action_type == "mission-draft-abandoned"
        ]
        self.assertEqual(len(abandon_entries), 1)
        self.assertIn("The original abandonment reason.", abandon_entries[0].summary)
        self.assertNotIn("substituted", abandon_entries[0].summary)

    def test_mission_draft_abandonment_preserves_existing_missions(self) -> None:
        snapshots = self.load_service()
        mission = snapshots._primary_mission
        initial_issue_ids = tuple(mission.issues)
        service = MissionDraftService(snapshots)
        created = service.create_draft(
            correlation_id="mission-draft-abandon-create-1",
            expected_revision=1,
            proposed_goal="Create a mission that should not be accepted.",
            selected_ad_hoc_ids=[],
            excluded_ad_hoc_ids=[],
            new_work_items=["Explore work that will be abandoned."],
            dependencies=[],
            unresolved_decisions=[],
        )

        acknowledgement = service.abandon_draft(
            correlation_id="mission-draft-abandon-1",
            expected_revision=created.revision,
            draft_id=created.draft_id,
            reason="Mission Commander rejected this draft.",
        )
        reloaded_service = self.load_service()
        reloaded_draft = MissionDraftService(reloaded_service).inspect().drafts[0]

        self.assertEqual(acknowledgement.outcome, "acknowledged")
        self.assertEqual(acknowledgement.draft_status, "abandoned")
        self.assertEqual(reloaded_draft.status, "abandoned")
        self.assertEqual(tuple(mission.issues), initial_issue_ids)
        self.assertEqual(tuple(reloaded_service._primary_mission.issues), initial_issue_ids)
        self.assertEqual(tuple(mission.sessions), ())

    def test_mission_draft_create_update_and_abandon_are_correlation_idempotent(self) -> None:
        service = MissionDraftService(self.load_service())
        create_request = {
            "correlation_id": "mission-draft-replay-create-1",
            "expected_revision": 1,
            "proposed_goal": "Exercise every non-confirming Mission Draft action.",
            "selected_ad_hoc_ids": [],
            "excluded_ad_hoc_ids": [],
            "new_work_items": ["Create the initial proposal."],
            "dependencies": [],
            "unresolved_decisions": [],
        }
        created = service.create_draft(**create_request)

        self.assertEqual(service.create_draft(**create_request), created)
        with self.assertRaisesRegex(AlbertError, "different request"):
            service.create_draft(
                **{
                    **create_request,
                    "new_work_items": ["A changed create must not replay."],
                }
            )

        update_request = {
            "correlation_id": "mission-draft-replay-update-1",
            "expected_revision": created.revision,
            "draft_id": created.draft_id,
            "proposed_goal": "Exercise the revised Mission Draft action.",
            "selected_ad_hoc_ids": [],
            "excluded_ad_hoc_ids": [],
            "new_work_items": ["Persist one revision."],
            "dependencies": [],
            "unresolved_decisions": [],
        }
        updated = service.update_draft(**update_request)

        self.assertEqual(service.update_draft(**update_request), updated)
        with self.assertRaisesRegex(AlbertError, "different request"):
            service.update_draft(
                **{**update_request, "proposed_goal": "A changed update must not replay."}
            )

        abandon_request = {
            "correlation_id": "mission-draft-replay-abandon-1",
            "expected_revision": updated.revision,
            "draft_id": created.draft_id,
            "reason": "Conclude the replay exercise.",
        }
        abandoned = service.abandon_draft(**abandon_request)

        self.assertEqual(service.abandon_draft(**abandon_request), abandoned)
        with self.assertRaisesRegex(AlbertError, "different request"):
            service.abandon_draft(
                **{**abandon_request, "reason": "A changed abandonment must not replay."}
            )
        restored = MissionDraftService(self.load_service()).inspect()
        self.assertEqual(len(restored.drafts), 1)
        self.assertEqual(restored.drafts[0].status, "abandoned")
        action_types = [
            entry.action_type for entry in ActivityJournalService(self.load_service()).inspect().entries
        ]
        self.assertEqual(
            action_types,
            ["mission-draft-created", "mission-draft-updated", "mission-draft-abandoned"],
        )

    def test_mission_draft_update_recovers_creation_audit_before_later_action(
        self,
    ) -> None:
        snapshots = self.load_service()
        drafts = MissionDraftService(snapshots)
        create_request = {
            "correlation_id": "mission-draft-causal-update-create-1",
            "expected_revision": 1,
            "proposed_goal": "Recover creation before recording a later update.",
            "selected_ad_hoc_ids": [],
            "excluded_ad_hoc_ids": [],
            "new_work_items": ["Create the initial causal proposal."],
            "dependencies": [],
            "unresolved_decisions": [],
        }
        with patch.object(
            ActivityJournalService,
            "record_mission_draft_created",
            side_effect=OSError("simulated lost creation audit"),
        ):
            with self.assertRaisesRegex(OSError, "lost creation audit"):
                drafts.create_draft(**create_request)

        MissionDraftService(self.load_service()).update_draft(
            correlation_id="mission-draft-causal-update-1",
            expected_revision=2,
            draft_id="mission-draft-command-deck-000001",
            proposed_goal="Record the update after its recovered creation.",
            selected_ad_hoc_ids=[],
            excluded_ad_hoc_ids=[],
            new_work_items=["Persist the later causal update."],
            dependencies=[],
            unresolved_decisions=[],
        )

        self.assertEqual(
            [
                (entry.correlation_id, entry.action_type)
                for entry in ActivityJournalService(self.load_service()).inspect().entries
            ],
            [
                (
                    "mission-draft-causal-update-create-1",
                    "mission-draft-created",
                ),
                ("mission-draft-causal-update-1", "mission-draft-updated"),
            ],
        )

    def test_mission_draft_confirmation_recovers_creation_audit_before_later_action(
        self,
    ) -> None:
        drafts = MissionDraftService(self.load_service())
        with patch.object(
            ActivityJournalService,
            "record_mission_draft_created",
            side_effect=OSError("simulated lost creation audit"),
        ):
            with self.assertRaisesRegex(OSError, "lost creation audit"):
                drafts.create_draft(
                    correlation_id="mission-draft-causal-confirm-create-1",
                    expected_revision=1,
                    proposed_goal="Recover creation before confirmation.",
                    selected_ad_hoc_ids=[],
                    excluded_ad_hoc_ids=[],
                    new_work_items=["Confirm only after causal recovery."],
                    dependencies=[],
                    unresolved_decisions=[],
                )

        MissionDraftService(self.load_service()).confirm_draft(
            correlation_id="mission-draft-causal-confirm-1",
            expected_revision=2,
            draft_id="mission-draft-command-deck-000001",
            reason="Record confirmation after its recovered creation.",
        )

        self.assertEqual(
            [
                (entry.correlation_id, entry.action_type)
                for entry in ActivityJournalService(self.load_service()).inspect().entries
            ],
            [
                (
                    "mission-draft-causal-confirm-create-1",
                    "mission-draft-created",
                ),
                ("mission-draft-causal-confirm-1", "mission-draft-confirmed"),
            ],
        )

    def test_mission_draft_abandonment_recovers_creation_audit_before_later_action(
        self,
    ) -> None:
        drafts = MissionDraftService(self.load_service())
        with patch.object(
            ActivityJournalService,
            "record_mission_draft_created",
            side_effect=OSError("simulated lost creation audit"),
        ):
            with self.assertRaisesRegex(OSError, "lost creation audit"):
                drafts.create_draft(
                    correlation_id="mission-draft-causal-abandon-create-1",
                    expected_revision=1,
                    proposed_goal="Recover creation before abandonment.",
                    selected_ad_hoc_ids=[],
                    excluded_ad_hoc_ids=[],
                    new_work_items=["Abandon only after causal recovery."],
                    dependencies=[],
                    unresolved_decisions=[],
                )

        MissionDraftService(self.load_service()).abandon_draft(
            correlation_id="mission-draft-causal-abandon-1",
            expected_revision=2,
            draft_id="mission-draft-command-deck-000001",
            reason="Record abandonment after its recovered creation.",
        )

        self.assertEqual(
            [
                (entry.correlation_id, entry.action_type)
                for entry in ActivityJournalService(self.load_service()).inspect().entries
            ],
            [
                (
                    "mission-draft-causal-abandon-create-1",
                    "mission-draft-created",
                ),
                ("mission-draft-causal-abandon-1", "mission-draft-abandoned"),
            ],
        )

    def test_mission_draft_recovery_fails_closed_on_prefixed_inverted_audit_history(
        self,
    ) -> None:
        snapshots = self.load_service()
        drafts = MissionDraftService(snapshots)
        created = drafts.create_draft(
            correlation_id="mission-draft-inverted-create-1",
            expected_revision=1,
            proposed_goal="Reject an inherited inverted lifecycle audit.",
            selected_ad_hoc_ids=[],
            excluded_ad_hoc_ids=[],
            new_work_items=["Keep creation before every later action."],
            dependencies=[],
            unresolved_decisions=[],
        )
        drafts.update_draft(
            correlation_id="mission-draft-inverted-update-1",
            expected_revision=created.revision,
            draft_id=created.draft_id,
            proposed_goal="Keep this update after its creation audit.",
            selected_ad_hoc_ids=[],
            excluded_ad_hoc_ids=[],
            new_work_items=["Preserve causal lifecycle order."],
            dependencies=[],
            unresolved_decisions=[],
        )
        journal = ActivityJournalService(snapshots)
        payload = json.loads(journal.journal_path.read_text(encoding="utf-8"))
        update_entry = next(
            item
            for item in payload["entries"]
            if item["action_type"] == "mission-draft-updated"
        )
        update_entry["sequence"] = 1
        update_entry["entry_id"] = "activity-000001"
        journal.journal_path.write_text(
            json.dumps(
                {
                    "schema_version": 1,
                    "revision": 1,
                    "entries": [update_entry],
                }
            ),
            encoding="utf-8",
        )

        with self.assertRaisesRegex(
            WorkspacePersistenceError,
            "causal order",
        ):
            MissionDraftService(self.load_service()).inspect()

        self.assertEqual(
            [
                entry.action_type
                for entry in ActivityJournalService(self.load_service()).inspect().entries
            ],
            ["mission-draft-updated"],
        )

    def test_stale_mission_draft_confirmation_preserves_mission_state(self) -> None:
        snapshots = self.load_service()
        mission = snapshots._primary_mission
        initial_issue_ids = tuple(mission.issues)
        service = MissionDraftService(snapshots)
        created = service.create_draft(
            correlation_id="mission-draft-stale-create-1",
            expected_revision=1,
            proposed_goal="Create a mission only from the fresh draft revision.",
            selected_ad_hoc_ids=[],
            excluded_ad_hoc_ids=[],
            new_work_items=["Original draft work."],
            dependencies=[],
            unresolved_decisions=[],
        )
        service.update_draft(
            correlation_id="mission-draft-stale-update-1",
            expected_revision=created.revision,
            draft_id=created.draft_id,
            proposed_goal="Create a mission only from the updated draft revision.",
            selected_ad_hoc_ids=[],
            excluded_ad_hoc_ids=[],
            new_work_items=["Updated draft work."],
            dependencies=[],
            unresolved_decisions=[],
        )

        with self.assertRaises(WorkspaceStaleActionError):
            service.confirm_draft(
                correlation_id="mission-draft-stale-confirm-1",
                expected_revision=created.revision,
                draft_id=created.draft_id,
                reason="This confirmation used a stale draft revision.",
            )
        reloaded_service = self.load_service()

        self.assertEqual(tuple(mission.issues), initial_issue_ids)
        self.assertEqual(tuple(reloaded_service._primary_mission.issues), initial_issue_ids)
        self.assertEqual(MissionDraftService(reloaded_service).inspect().drafts[0].status, "draft")

    def test_cli_creates_inspects_and_confirms_mission_draft_across_restart(self) -> None:
        snapshots = self.load_service()
        origin = AgentConsoleHistoryService(snapshots).append(
            role="user",
            content="Create a draft through the CLI.",
            outcome="proposed",
            source="mission-commander",
            expected_revision=1,
            expected_scope=snapshots.snapshot().conversation_scope,
        )
        WorkspaceQueueService(snapshots).propose_ad_hoc_delegation(
            correlation_id="mission-draft-cli-ad-hoc-1",
            expected_revision=1,
            source="agent-console",
            scope=snapshots.snapshot().conversation_scope,
            acceptance_criteria=["CLI-selected ad hoc work is represented."],
            allowed_paths=["docs/cli-draft.md"],
            command_policy={},
            proposed_agent="qwen-coder-local-1",
            originating_message_id=origin.message_id,
        )
        common = [
            "--target-repo",
            str(self.target_repo),
            "--tracker-dir",
            str(self.tracker),
            "--runtime-root",
            str(self.runtime),
            "--mission-id",
            "command-deck",
        ]
        create_output = io.StringIO()
        with redirect_stdout(create_output):
            create_exit = main(
                [
                    "mission-draft-create",
                    *common,
                    "--correlation-id",
                    "mission-draft-cli-create-1",
                    "--expected-revision",
                    "1",
                    "--proposed-goal",
                    "Create a CLI confirmed mission draft.",
                    "--selected-ad-hoc-id",
                    "ADHOC-000001",
                    "--new-work-item",
                    "Expose Mission Drafts through the CLI.",
                ]
            )
        created = json.loads(create_output.getvalue())
        inspect_output = io.StringIO()
        with redirect_stdout(inspect_output):
            inspect_exit = main(["mission-drafts", *common])
        inspected = json.loads(inspect_output.getvalue())
        confirm_output = io.StringIO()
        with redirect_stdout(confirm_output):
            confirm_exit = main(
                [
                    "mission-draft-confirm",
                    *common,
                    "--correlation-id",
                    "mission-draft-cli-confirm-1",
                    "--expected-draft-revision",
                    str(created["revision"]),
                    "--draft-id",
                    created["draft_id"],
                    "--reason",
                    "Confirmed through CLI.",
                ]
            )
        confirmed = json.loads(confirm_output.getvalue())
        reloaded = self.load_service()

        self.assertEqual(create_exit, 0)
        self.assertEqual(inspect_exit, 0)
        self.assertEqual(confirm_exit, 0)
        self.assertEqual(created["draft_status"], "draft")
        self.assertEqual(inspected["drafts"][0]["included_ad_hoc_work"][0]["work_id"], "ADHOC-000001")
        self.assertEqual(confirmed["draft_status"], "confirmed")
        self.assertEqual(confirmed["accepted_issue_id"], "ISS-02")
        self.assertIn("ISS-02", reloaded._primary_mission.issues)

    def test_workspace_snapshot_attention_links_to_pending_queue_items(self) -> None:
        snapshots = self.load_service()
        mission = snapshots._primary_mission
        mission.approve_issue("ISS-01")
        proposal = WorkspaceQueueService(snapshots).propose_issue_contract_change(
            correlation_id="proposal-attention-1",
            expected_revision=1,
            issue_id="ISS-01",
            source="issue-slice-inspector",
            acceptance_criteria=["Attention links to the queue item."],
        )

        summary = snapshots.snapshot().missions[0]

        self.assertEqual(summary.attention[0].attention_id, proposal.item_id)
        self.assertEqual(summary.attention[0].kind, "issue-change-proposal")
        self.assertEqual(summary.attention[0].label, "ISS-01 Issue Change Proposal pending")
        self.assertEqual(
            summary.attention[0].queue_link,
            f"workspace-queue#{proposal.item_id}",
        )

    def test_cli_returns_workspace_queue_projection_and_acknowledges_decision_as_json(self) -> None:
        snapshots = self.load_service()
        mission = snapshots._primary_mission
        mission.approve_issue("ISS-01")
        accepted_criteria = list(mission.issues["ISS-01"].acceptance_criteria)
        proposal = WorkspaceQueueService(snapshots).propose_issue_contract_change(
            correlation_id="proposal-cli-1",
            expected_revision=1,
            issue_id="ISS-01",
            source="issue-slice-inspector",
            acceptance_criteria=["CLI rejected proposal."],
        )
        common = [
            "--target-repo",
            str(self.target_repo),
            "--tracker-dir",
            str(self.tracker),
            "--runtime-root",
            str(self.runtime),
            "--mission-id",
            "command-deck",
        ]
        projection_output = io.StringIO()
        with redirect_stdout(projection_output):
            projection_exit = main(
                [
                    "workspace-queue",
                    *common,
                    "--item-type",
                    "issue-change-proposal",
                    "--queue-mission-id",
                    "command-deck",
                ]
            )
        decision_output = io.StringIO()
        with redirect_stdout(decision_output):
            decision_exit = main(
                [
                    "workspace-queue-decision",
                    *common,
                    "--correlation-id",
                    "proposal-cli-reject-1",
                    "--expected-queue-revision",
                    str(proposal.revision),
                    "--item-id",
                    proposal.item_id,
                    "--decision",
                    "reject",
                    "--reason",
                    "Rejected from CLI.",
                ]
            )

        projection = json.loads(projection_output.getvalue())
        acknowledgement = json.loads(decision_output.getvalue())
        self.assertEqual(projection_exit, 0)
        self.assertEqual(projection["items"][0]["item_id"], proposal.item_id)
        self.assertEqual(projection["groups"][0]["group_id"], "issue-change-proposal:command-deck")
        self.assertEqual(decision_exit, 0)
        self.assertEqual(acknowledgement["item_status"], "rejected")
        self.assertIsNone(acknowledgement["session_id"])
        self.assertEqual(mission.issues["ISS-01"].acceptance_criteria, accepted_criteria)

    def test_cli_proposes_ad_hoc_delegation_and_returns_queue_projection_as_json(self) -> None:
        snapshots = self.load_service()
        origin = AgentConsoleHistoryService(snapshots).append(
            role="user",
            content="Have a local agent refresh smoke-test notes.",
            outcome="proposed",
            source="mission-commander",
            expected_revision=1,
            expected_scope=snapshots.snapshot().conversation_scope,
        )
        common = [
            "--target-repo",
            str(self.target_repo),
            "--tracker-dir",
            str(self.tracker),
            "--runtime-root",
            str(self.runtime),
            "--mission-id",
            "command-deck",
        ]
        acknowledgement_output = io.StringIO()
        with redirect_stdout(acknowledgement_output):
            acknowledgement_exit = main(
                [
                    "ad-hoc-delegation-proposal",
                    *common,
                    "--correlation-id",
                    "ad-hoc-cli-proposal-1",
                    "--expected-revision",
                    "1",
                    "--source",
                    "agent-console",
                    "--scope-kind",
                    "working-directory",
                    "--scope-target",
                    str(self.target_repo),
                    "--scope-label",
                    "target",
                    "--acceptance-criterion",
                    "Smoke-test notes mention the focused unit command.",
                    "--allowed-path",
                    "docs/smoke-tests.md",
                    "--command-policy",
                    "python3 -m unittest tests.test_workspace_snapshot=auto-allowed",
                    "--proposed-agent",
                    "qwen-coder-local-1",
                    "--originating-message-id",
                    origin.message_id,
                ]
            )
        projection_output = io.StringIO()
        with redirect_stdout(projection_output):
            projection_exit = main(
                [
                    "workspace-queue",
                    *common,
                    "--item-type",
                    "ad-hoc-delegation",
                ]
            )

        acknowledgement = json.loads(acknowledgement_output.getvalue())
        projection = json.loads(projection_output.getvalue())
        self.assertEqual(acknowledgement_exit, 0)
        self.assertEqual(acknowledgement["item_status"], "pending")
        self.assertEqual(acknowledgement["item_id"], "ad-hoc-delegation-command-deck-000001")
        self.assertEqual(projection_exit, 0)
        self.assertEqual(projection["items"][0]["item_type"], "ad-hoc-delegation")
        self.assertEqual(projection["items"][0]["issue_id"], "ADHOC-000001")
        self.assertEqual(
            projection["items"][0]["proposed_changes"]["originating_message_id"],
            origin.message_id,
        )

    def test_review_workspace_accepts_valid_evidence_as_complete_pr_ready_not_merged(self) -> None:
        mission = self.load_service()._primary_mission
        mission.approve_issue("ISS-01")
        session = mission.launch_issue("ISS-01")
        mission.record_evidence(
            session.session_id,
            EvidencePackage(
                changed_files=["src/app.py"],
                diff_summary="Added the review workspace decision path.",
                commands_run=["python3 -m unittest tests.test_workspace_snapshot"],
                test_results="Focused workspace tests passed.",
                known_risks="None.",
                proposed_context_updates="No glossary changes.",
            ),
        )

        acknowledgement = ReviewWorkspaceService(WorkspaceSnapshotService(mission)).decide(
            correlation_id="review-accept-1",
            expected_revision=1,
            session_id=session.session_id,
            decision="accept",
            reason="Evidence satisfies the Issue Slice.",
        )

        self.assertEqual(acknowledgement.correlation_id, "review-accept-1")
        self.assertEqual(acknowledgement.outcome, "acknowledged")
        self.assertEqual(acknowledgement.review_outcome, "Approved")
        self.assertEqual(acknowledgement.next_action, "prepare-pr")
        self.assertEqual(acknowledgement.issue_lifecycle, "Complete")
        self.assertNotEqual(acknowledgement.issue_lifecycle, "Merged")
        self.assertEqual(mission.issues["ISS-01"].review_state, "pr-ready")

    def test_review_workspace_rejects_acceptance_when_required_evidence_is_missing(self) -> None:
        mission = self.load_service()._primary_mission
        mission.approve_issue("ISS-01")
        session = mission.launch_issue("ISS-01")
        session.status = "failed"
        session.runner_ended_at = "2026-07-11T10:00:00+00:00"
        mission._persist()

        with self.assertRaisesRegex(
            EvidenceValidationError,
            "changed_files, diff_summary, commands_run, test_results, known_risks, proposed_context_updates",
        ):
            ReviewWorkspaceService(WorkspaceSnapshotService(mission)).decide(
                correlation_id="review-accept-incomplete-1",
                expected_revision=1,
                session_id=session.session_id,
                decision="accept",
                reason="Looks fine.",
            )

        self.assertEqual(mission.issues["ISS-01"].review_state, "approved")
        self.assertEqual(mission.sessions[session.session_id].status, "failed")

    def test_review_workspace_routes_repair_human_escalation_and_stale_decisions(self) -> None:
        mission = self.load_service()._primary_mission
        mission.approve_issue("ISS-01")
        repair_session = mission.launch_issue("ISS-01")
        repair_session.status = "failed"
        repair_session.runner_ended_at = "2026-07-11T10:00:00+00:00"
        mission._persist()
        service = ReviewWorkspaceService(WorkspaceSnapshotService(mission))

        with self.assertRaisesRegex(AlbertError, "Repair review decisions require a reason"):
            service.decide(
                correlation_id="repair-without-reason",
                expected_revision=1,
                session_id=repair_session.session_id,
                decision="repair",
            )

        repair = service.decide(
            correlation_id="repair-with-reason",
            expected_revision=1,
            session_id=repair_session.session_id,
            decision="repair",
            reason="Acceptance criteria are not met.",
        )

        self.assertEqual(repair.review_outcome, "Needs repair")
        self.assertEqual(repair.next_action, "same-local-agent-repair")
        self.assertEqual(mission.issues["ISS-01"].review_state, "needs-repair")
        human_session = mission.launch_repair(repair_session.session_id)
        human_session.status = "failed"
        human_session.runner_ended_at = "2026-07-11T10:01:00+00:00"
        mission._persist()
        human = ReviewWorkspaceService(WorkspaceSnapshotService(mission)).decide(
            correlation_id="human-escalation",
            expected_revision=2,
            session_id=human_session.session_id,
            decision="escalate-human",
            reason="Human operator must inspect local-only evidence.",
        )

        self.assertEqual(human.review_outcome, "Needs human review")
        self.assertEqual(human.next_action, "user-review")
        self.assertEqual(mission.issues["ISS-01"].review_state, "needs-human-review")
        with self.assertRaises(WorkspaceStaleActionError):
            ReviewWorkspaceService(WorkspaceSnapshotService(mission)).decide(
                correlation_id="stale-review",
                expected_revision=1,
                session_id=human_session.session_id,
                decision="escalate-human",
                reason="Stale.",
            )

    def test_review_workspace_repair_reload_queues_one_inherited_issue_repair_and_dispatches(
        self,
    ) -> None:
        (self.target_repo / ".albert").mkdir()
        (self.target_repo / ".albert" / "agents.json").write_text(
            json.dumps(
                {
                    "agents": [
                        {
                            "id": "qwen-coder-local-1",
                            "role": "local-agent",
                            "provider": "test-harness",
                            "runner": "fake",
                            "model": "deterministic-fake",
                        }
                    ]
                }
            ),
            encoding="utf-8",
        )
        snapshots = self.load_service()
        mission = snapshots._primary_mission
        mission.approve_issue("ISS-01")
        prior = mission.launch_issue(
            "ISS-01",
            allowed_paths=["FAKE_AGENT_RESULT.md"],
            command_policy={"python3 -m unittest": "auto-allowed"},
        )
        mission.run_session(prior.session_id)
        prior.status = "failed"
        prior.runner_ended_at = "2026-07-11T10:00:00+00:00"
        mission._persist()

        decision = ReviewWorkspaceService(snapshots).decide(
            correlation_id="review-repair-reload-issue-1",
            expected_revision=1,
            session_id=prior.session_id,
            decision="repair",
            reason="The canonical snapshot output is incomplete.",
        )
        reloaded = self.load_service()
        canonical_prior = next(
            session
            for session in reloaded.snapshot().missions[0].sessions
            if session.session_id == prior.session_id
        )

        self.assertEqual(decision.revision, 2)
        self.assertEqual(canonical_prior.review_outcome, "Needs repair")
        self.assertEqual(canonical_prior.review_next_action, "same-local-agent-repair")
        self.assertTrue(canonical_prior.repair_action_available)

        request = {
            "correlation_id": "review-repair-launch-issue-1",
            "action_type": "issue-retry",
            "actor": "mission-commander",
            "expected_revision": decision.revision,
            "target_kind": "agent-session",
            "target_id": prior.session_id,
            "mission_id": "command-deck",
            "issue_id": "ISS-01",
            "session_id": prior.session_id,
        }
        launched = WorkstationActionService(reloaded).submit(**request)
        replayed = WorkstationActionService(self.load_service()).submit(**request)
        after_launch = self.load_service()
        repair = after_launch._primary_mission.sessions[launched.session_id]

        self.assertEqual(launched, replayed)
        self.assertEqual(list(after_launch._primary_mission.sessions), [prior.session_id, repair.session_id])
        self.assertEqual(repair.status, "queued")
        self.assertEqual(repair.task_packet["allowed_paths"], ["FAKE_AGENT_RESULT.md"])
        self.assertEqual(
            repair.task_packet["command_policy"],
            {"python3 -m unittest": "auto-allowed"},
        )
        refreshed_prior = next(
            session
            for session in after_launch.snapshot().missions[0].sessions
            if session.session_id == prior.session_id
        )
        self.assertFalse(refreshed_prior.repair_action_available)

        with self.assertRaisesRegex(
            AlbertError,
            f"Review Workspace repair was already launched for {prior.session_id}",
        ):
            WorkstationActionService(after_launch).submit(
                **{
                    **request,
                    "correlation_id": "review-repair-launch-issue-duplicate",
                    "expected_revision": after_launch.snapshot().revision,
                }
            )
        self.assertEqual(
            list(self.load_service()._primary_mission.sessions),
            [prior.session_id, repair.session_id],
        )

        completed = after_launch._primary_mission.run_session(repair.session_id)
        self.assertEqual(completed.status, "evidence-ready")

    def test_tui_and_legacy_needs_repair_review_exposes_one_click_workstation_repair(
        self,
    ) -> None:
        snapshots = self.load_service()
        mission = snapshots._primary_mission
        mission.approve_issue("ISS-01")
        prior = mission.launch_issue(
            "ISS-01",
            allowed_paths=["src/app.py"],
            command_policy={"python3 -m unittest": "auto-allowed"},
        )
        perform_tui_action(
            mission,
            "review",
            "ISS-01",
            session_id=prior.session_id,
            outcome="Needs repair",
            reason="The TUI review found an incomplete implementation.",
        )

        # Simulate a persisted review written before workspace-action metadata existed.
        runtime_payload = json.loads(mission.runtime_path.read_text(encoding="utf-8"))
        runtime_payload["reviews"][-1].pop("workspace_action", None)
        WorkspaceSnapshotService._write_json_atomically(
            mission.runtime_path,
            runtime_payload,
        )
        reloaded = self.load_service()
        canonical_prior = next(
            session
            for session in reloaded.snapshot().missions[0].sessions
            if session.session_id == prior.session_id
        )

        self.assertEqual(canonical_prior.review_outcome, "Needs repair")
        self.assertEqual(
            canonical_prior.review_next_action,
            "same-local-agent-repair",
        )
        self.assertTrue(canonical_prior.repair_action_available)

        request = {
            "correlation_id": "tui-legacy-repair-launch-1",
            "action_type": "issue-retry",
            "actor": "mission-commander",
            "expected_revision": reloaded.snapshot().revision,
            "target_kind": "agent-session",
            "target_id": prior.session_id,
            "mission_id": mission.mission_id,
            "issue_id": "ISS-01",
            "session_id": prior.session_id,
        }
        launched = WorkstationActionService(reloaded).submit(**request)
        replayed = WorkstationActionService(self.load_service()).submit(**request)
        after_launch = self.load_service()

        self.assertEqual(replayed, launched)
        self.assertEqual(
            after_launch._primary_mission.sessions[launched.session_id].task_packet[
                "allowed_paths"
            ],
            ["src/app.py"],
        )
        self.assertEqual(
            after_launch._primary_mission.sessions[launched.session_id].task_packet[
                "command_policy"
            ],
            {"python3 -m unittest": "auto-allowed"},
        )
        with self.assertRaisesRegex(AlbertError, "already launched"):
            WorkstationActionService(after_launch).submit(
                **{
                    **request,
                    "correlation_id": "tui-legacy-repair-launch-duplicate",
                    "expected_revision": after_launch.snapshot().revision,
                }
            )
        self.assertEqual(
            list(self.load_service()._primary_mission.sessions),
            [prior.session_id, launched.session_id],
        )

    def test_cli_needs_repair_review_projects_workstation_repair_action(self) -> None:
        mission = self.load_service()._primary_mission
        mission.approve_issue("ISS-01")
        prior = mission.launch_issue("ISS-01")
        output = io.StringIO()

        with redirect_stdout(output):
            exit_code = main(
                [
                    "review",
                    "--target-repo",
                    str(self.target_repo),
                    "--tracker-dir",
                    str(self.tracker),
                    "--runtime-root",
                    str(self.runtime),
                    "--mission-id",
                    "command-deck",
                    prior.session_id,
                    "--outcome",
                    "Needs repair",
                    "--reason",
                    "The CLI review found missing acceptance evidence.",
                ]
            )

        canonical_prior = next(
            session
            for session in self.load_service().snapshot().missions[0].sessions
            if session.session_id == prior.session_id
        )
        self.assertEqual(exit_code, 0)
        self.assertIn("next action same-local-agent-repair", output.getvalue())
        self.assertEqual(canonical_prior.review_outcome, "Needs repair")
        self.assertTrue(canonical_prior.repair_action_available)

    def test_review_workspace_repair_reload_queues_one_inherited_ad_hoc_repair_and_dispatches(
        self,
    ) -> None:
        (self.target_repo / ".albert").mkdir()
        (self.target_repo / ".albert" / "agents.json").write_text(
            json.dumps(
                {
                    "agents": [
                        {
                            "id": "qwen-coder-local-1",
                            "role": "local-agent",
                            "provider": "test-harness",
                            "runner": "fake",
                            "model": "deterministic-fake",
                        }
                    ]
                }
            ),
            encoding="utf-8",
        )
        snapshots = self.load_service()
        history = AgentConsoleHistoryService(snapshots)
        origin = history.append(
            role="user",
            content="Repair the bounded ad hoc documentation result.",
            outcome="proposed",
            source="mission-commander",
            expected_revision=1,
            expected_scope=snapshots.snapshot().conversation_scope,
        )
        queue = WorkspaceQueueService(snapshots)
        proposal = queue.propose_ad_hoc_delegation(
            correlation_id="review-repair-ad-hoc-proposal-1",
            expected_revision=1,
            source="agent-console",
            scope=snapshots.snapshot().conversation_scope,
            acceptance_criteria=["The repair retains the bounded ad hoc contract."],
            allowed_paths=["FAKE_AGENT_RESULT.md"],
            command_policy={"python3 -m unittest": "auto-allowed"},
            proposed_agent="qwen-coder-local-1",
            originating_message_id=origin.message_id,
        )
        approved = queue.decide(
            correlation_id="review-repair-ad-hoc-approve-1",
            expected_revision=proposal.revision,
            item_id=proposal.item_id,
            decision="approve",
            reason="Run the bounded ad hoc task.",
        )
        prior_id = approved.session_id or ""
        snapshots._primary_mission.run_session(prior_id)

        decision = ReviewWorkspaceService(self.load_service()).decide(
            correlation_id="review-repair-ad-hoc-decision-1",
            expected_revision=self.load_service().snapshot().revision,
            mission_id="command-deck",
            session_id=prior_id,
            decision="repair",
            reason="The bounded result needs one repair pass.",
        )
        reloaded = self.load_service()
        canonical_prior = next(
            session
            for session in reloaded.snapshot().missions[0].sessions
            if session.session_id == prior_id
        )

        self.assertEqual(canonical_prior.issue_id, "ADHOC-000001")
        self.assertEqual(canonical_prior.review_outcome, "Needs repair")
        self.assertTrue(canonical_prior.repair_action_available)

        request = {
            "correlation_id": "review-repair-ad-hoc-launch-1",
            "action_type": "issue-retry",
            "actor": "mission-commander",
            "expected_revision": decision.revision,
            "target_kind": "agent-session",
            "target_id": prior_id,
            "mission_id": "command-deck",
            "issue_id": "ADHOC-000001",
            "session_id": prior_id,
        }
        launched = WorkstationActionService(reloaded).submit(**request)
        replayed = WorkstationActionService(self.load_service()).submit(**request)
        after_launch = self.load_service()
        repair = after_launch._primary_mission.sessions[launched.session_id]

        self.assertEqual(launched, replayed)
        self.assertNotIn("ADHOC-000001", after_launch._primary_mission.issues)
        self.assertEqual(list(after_launch._primary_mission.sessions), [prior_id, repair.session_id])
        self.assertEqual(repair.status, "queued")
        self.assertEqual(repair.task_packet["work_kind"], "ad-hoc-delegation")
        self.assertEqual(repair.task_packet["originating_message_id"], origin.message_id)
        self.assertEqual(
            repair.task_packet["acceptance_criteria"],
            ["The repair retains the bounded ad hoc contract."],
        )
        self.assertEqual(repair.task_packet["allowed_paths"], ["FAKE_AGENT_RESULT.md"])
        self.assertEqual(
            repair.task_packet["command_policy"],
            {"python3 -m unittest": "auto-allowed"},
        )

        completed = after_launch._primary_mission.run_session(repair.session_id)
        self.assertEqual(completed.status, "evidence-ready")

    def test_cli_returns_review_workspace_projection_as_json(self) -> None:
        mission = self.load_service()._primary_mission
        mission.approve_issue("ISS-01")
        session = mission.launch_issue("ISS-01")
        mission.record_evidence(
            session.session_id,
            EvidencePackage(
                changed_files=["src/app.py"],
                diff_summary="Added JSON review projection.",
                commands_run=["python3 -m unittest tests.test_workspace_snapshot"],
                test_results="Focused workspace tests passed.",
                known_risks="None.",
                proposed_context_updates="No glossary changes.",
            ),
        )
        output = io.StringIO()

        with redirect_stdout(output):
            exit_code = main(
                [
                    "review-workspace",
                    "--target-repo",
                    str(self.target_repo),
                    "--tracker-dir",
                    str(self.tracker),
                    "--runtime-root",
                    str(self.runtime),
                    "--mission-id",
                    "command-deck",
                ]
            )

        projection = json.loads(output.getvalue())
        self.assertEqual(exit_code, 0)
        self.assertEqual(projection["schema_version"], 1)
        self.assertEqual(projection["items"][0]["session_id"], session.session_id)
        self.assertEqual(projection["items"][0]["evidence"]["diff_summary"], "Added JSON review projection.")

    def test_cli_acknowledges_review_workspace_decision_as_json(self) -> None:
        mission = self.load_service()._primary_mission
        mission.approve_issue("ISS-01")
        session = mission.launch_issue("ISS-01")
        mission.record_evidence(
            session.session_id,
            EvidencePackage(
                changed_files=["src/app.py"],
                diff_summary="Added JSON review decision.",
                commands_run=["python3 -m unittest tests.test_workspace_snapshot"],
                test_results="Focused workspace tests passed.",
                known_risks="None.",
                proposed_context_updates="No glossary changes.",
            ),
        )
        output = io.StringIO()

        with redirect_stdout(output):
            exit_code = main(
                [
                    "review-decision",
                    "--target-repo",
                    str(self.target_repo),
                    "--tracker-dir",
                    str(self.tracker),
                    "--runtime-root",
                    str(self.runtime),
                    "--mission-id",
                    "command-deck",
                    "--correlation-id",
                    "review-accept-cli-1",
                    "--expected-revision",
                    "1",
                    "--action-type",
                    "review-decision",
                    "--actor",
                    "mission-commander",
                    "--target-kind",
                    "agent-session",
                    "--target-id",
                    session.session_id,
                    "--session-id",
                    session.session_id,
                    "--decision",
                    "accept",
                    "--reason",
                    "Evidence satisfies the Issue Slice.",
                ]
            )

        acknowledgement = json.loads(output.getvalue())
        self.assertEqual(exit_code, 0)
        self.assertEqual(acknowledgement["correlation_id"], "review-accept-cli-1")
        self.assertEqual(acknowledgement["review_outcome"], "Approved")
        self.assertEqual(acknowledgement["next_action"], "prepare-pr")
        self.assertEqual(acknowledgement["issue_lifecycle"], "Complete")
        self.assertEqual(self.load_service()._primary_mission.issues["ISS-01"].review_state, "pr-ready")

    def test_cli_returns_filtered_activity_journal_as_json(self) -> None:
        mission = self.load_service()._primary_mission
        mission.approve_issue("ISS-01")
        session = mission.launch_issue("ISS-01")
        mission.record_evidence(
            session.session_id,
            EvidencePackage(
                changed_files=["src/app.py"],
                diff_summary="Added CLI Activity Journal projection.",
                commands_run=["python3 -m unittest tests.test_workspace_snapshot"],
                test_results="Focused workspace tests passed.",
                known_risks="None.",
                proposed_context_updates="Document Activity Journal CLI.",
                artifact_links=["app-local://evidence/session-ISS-01-1"],
            ),
        )
        common = [
            "--target-repo",
            str(self.target_repo),
            "--tracker-dir",
            str(self.tracker),
            "--runtime-root",
            str(self.runtime),
            "--mission-id",
            "command-deck",
        ]

        with redirect_stdout(io.StringIO()):
            decision_exit = main(
                [
                    "review-decision",
                    *common,
                    "--correlation-id",
                    "review-activity-cli-1",
                    "--expected-revision",
                    "1",
                    "--action-type",
                    "review-decision",
                    "--actor",
                    "mission-commander",
                    "--target-kind",
                    "agent-session",
                    "--target-id",
                    session.session_id,
                    "--session-id",
                    session.session_id,
                    "--decision",
                    "accept",
                    "--reason",
                    "Evidence is complete.",
                ]
            )
        output = io.StringIO()

        with redirect_stdout(output):
            journal_exit = main(
                [
                    "activity-journal",
                    *common,
                    "--activity-mission-id",
                    "command-deck",
                    "--actor",
                    "mission-commander",
                    "--action-type",
                    "review-decision",
                    "--search",
                    "evidence",
                    "--started-at",
                    "2026-01-01T00:00:00Z",
                ]
            )

        projection = json.loads(output.getvalue())
        entry = projection["entries"][0]
        self.assertEqual(decision_exit, 0)
        self.assertEqual(journal_exit, 0)
        self.assertEqual(projection["schema_version"], 1)
        self.assertEqual(projection["revision"], 2)
        self.assertEqual(len(projection["entries"]), 1)
        self.assertEqual(entry["correlation_id"], "review-activity-cli-1")
        self.assertEqual(entry["action_type"], "review-decision")
        self.assertEqual(entry["evidence_links"], ["app-local://evidence/session-ISS-01-1"])
        self.assertEqual(
            [entity["entity_type"] for entity in entry["affected_entities"]],
            ["mission", "issue-slice", "local-agent-session", "evidence-package"],
        )

    def test_cli_reports_incomplete_review_decision_as_structured_json(self) -> None:
        mission = self.load_service()._primary_mission
        mission.approve_issue("ISS-01")
        session = mission.launch_issue("ISS-01")
        session.status = "failed"
        mission._persist()
        output = io.StringIO()
        error = io.StringIO()

        with redirect_stdout(output), redirect_stderr(error):
            exit_code = main(
                [
                    "review-decision",
                    "--target-repo",
                    str(self.target_repo),
                    "--tracker-dir",
                    str(self.tracker),
                    "--runtime-root",
                    str(self.runtime),
                    "--mission-id",
                    "command-deck",
                    "--correlation-id",
                    "review-accept-incomplete-cli-1",
                    "--expected-revision",
                    "1",
                    "--action-type",
                    "review-decision",
                    "--actor",
                    "mission-commander",
                    "--target-kind",
                    "agent-session",
                    "--target-id",
                    session.session_id,
                    "--session-id",
                    session.session_id,
                    "--decision",
                    "accept",
                    "--reason",
                    "Looks acceptable.",
                ]
            )

        payload = json.loads(error.getvalue())
        self.assertEqual(exit_code, 1)
        self.assertEqual(output.getvalue(), "")
        self.assertEqual(payload["error"]["code"], "evidence-incomplete")
        self.assertEqual(payload["error"]["recoverable"], True)
        self.assertIn("changed_files", payload["error"]["message"])
        self.assertEqual(self.load_service()._primary_mission.issues["ISS-01"].review_state, "approved")

    def test_cli_rejects_mismatched_review_decision_metadata_before_mutation(self) -> None:
        mission = self.load_service()._primary_mission
        mission.approve_issue("ISS-01")
        session = mission.launch_issue("ISS-01")
        output = io.StringIO()
        error = io.StringIO()

        with redirect_stdout(output), redirect_stderr(error):
            exit_code = main(
                [
                    "review-decision",
                    "--target-repo",
                    str(self.target_repo),
                    "--tracker-dir",
                    str(self.tracker),
                    "--runtime-root",
                    str(self.runtime),
                    "--mission-id",
                    "command-deck",
                    "--correlation-id",
                    "review-target-mismatch-cli-1",
                    "--expected-revision",
                    "1",
                    "--action-type",
                    "review-decision",
                    "--actor",
                    "mission-commander",
                    "--target-kind",
                    "agent-session",
                    "--target-id",
                    "session-other",
                    "--session-id",
                    session.session_id,
                    "--decision",
                    "repair",
                    "--reason",
                    "Needs focused repair.",
                ]
            )

        self.assertEqual(exit_code, 1)
        self.assertEqual(output.getvalue(), "")
        self.assertIn(
            "Error: Review decision action target id must match session id",
            error.getvalue(),
        )
        self.assertEqual(
            self.load_service()._primary_mission.issues["ISS-01"].review_state,
            "approved",
        )

    def test_stale_semantic_action_is_rejected_without_changing_accepted_state(self) -> None:
        sync = WorkspaceSyncService(self.load_service())
        action = WorkspaceAction(
            correlation_id="view-stale-1",
            expected_revision=0,
            active_mission_id="command-deck",
            conversation_scope=ConversationScope(
                kind="working-directory",
                target_id=str(self.target_repo),
                label="target",
            ),
            operations_view="activity",
        )

        with self.assertRaises(WorkspaceStaleActionError) as raised:
            sync.submit_action(action)

        unchanged = self.load_service().snapshot()
        self.assertEqual(raised.exception.current_revision, 1)
        self.assertEqual(unchanged.revision, 1)
        self.assertEqual(unchanged.operations_view, "mission-board")

    def test_accepted_actions_produce_ordered_batch_after_known_revision(self) -> None:
        sync = WorkspaceSyncService(self.load_service())
        scope = ConversationScope(
            kind="working-directory",
            target_id=str(self.target_repo),
            label="target",
        )

        first = sync.submit_action(
            WorkspaceAction(
                correlation_id="view-review-1",
                expected_revision=1,
                active_mission_id="command-deck",
                conversation_scope=scope,
                operations_view="review-workspace",
            )
        )
        second = sync.submit_action(
            WorkspaceAction(
                correlation_id="view-activity-2",
                expected_revision=2,
                active_mission_id="command-deck",
                conversation_scope=scope,
                operations_view="activity",
            )
        )

        batch = sync.updates_after(1)

        self.assertEqual(first.revision, 2)
        self.assertEqual(second.revision, 3)
        self.assertEqual([event.revision for event in batch.events], [2, 3])
        self.assertEqual(
            [event.correlation_id for event in batch.events],
            ["view-review-1", "view-activity-2"],
        )
        self.assertEqual(len({event.event_id for event in batch.events}), 2)
        self.assertEqual(batch.current_revision, 3)
        self.assertEqual(self.load_service().snapshot().operations_view, "activity")

    def test_activity_journal_records_acknowledged_workspace_actions_only(self) -> None:
        snapshots = self.load_service()
        sync = WorkspaceSyncService(snapshots)
        scope = ConversationScope(
            kind="working-directory",
            target_id=str(self.target_repo),
            label="target",
        )

        acknowledgement = sync.submit_action(
            WorkspaceAction(
                correlation_id="view-activity-journal-1",
                expected_revision=1,
                active_mission_id="command-deck",
                conversation_scope=scope,
                operations_view="activity",
            )
        )
        with self.assertRaises(WorkspaceStaleActionError):
            sync.submit_action(
                WorkspaceAction(
                    correlation_id="view-stale-journal-2",
                    expected_revision=1,
                    active_mission_id="command-deck",
                    conversation_scope=scope,
                    operations_view="mission-board",
                )
            )

        reloaded = self.load_service()
        projection = ActivityJournalService(reloaded).inspect()
        session_id = reloaded.snapshot().workspace_session.id

        self.assertEqual(acknowledgement.revision, 2)
        self.assertEqual(projection.revision, 1)
        self.assertEqual(len(projection.entries), 1)
        entry = projection.entries[0]
        self.assertEqual(entry.entry_id, "activity-000001")
        self.assertEqual(entry.sequence, 1)
        self.assertTrue(entry.recorded_at.endswith("Z"))
        self.assertEqual(entry.actor, "mission-commander")
        self.assertEqual(entry.action_type, "operations-view-selected")
        self.assertEqual(entry.correlation_id, "view-activity-journal-1")
        self.assertIn("Activity", entry.summary)
        self.assertEqual(entry.evidence_links, ())
        self.assertEqual(
            [(entity.entity_type, entity.entity_id) for entity in entry.affected_entities],
            [("workspace-session", session_id), ("mission", "command-deck")],
        )

    def test_activity_journal_searches_and_filters_chronological_entries(self) -> None:
        snapshots = self.load_service()
        sync = WorkspaceSyncService(snapshots)
        scope = ConversationScope(
            kind="working-directory",
            target_id=str(self.target_repo),
            label="target",
        )
        sync.submit_action(
            WorkspaceAction(
                correlation_id="view-review-journal-1",
                expected_revision=1,
                active_mission_id="command-deck",
                conversation_scope=scope,
                operations_view="review-workspace",
            )
        )
        sync.submit_action(
            WorkspaceAction(
                correlation_id="view-queue-journal-2",
                expected_revision=2,
                active_mission_id="command-deck",
                conversation_scope=scope,
                operations_view="workspace-queue",
            )
        )

        journal = ActivityJournalService(self.load_service())

        self.assertEqual(
            [entry.correlation_id for entry in journal.inspect().entries],
            ["view-review-journal-1", "view-queue-journal-2"],
        )
        self.assertEqual(
            [entry.correlation_id for entry in journal.inspect(search="queue").entries],
            ["view-queue-journal-2"],
        )
        self.assertEqual(
            [
                entry.correlation_id
                for entry in journal.inspect(
                    mission_id="command-deck",
                    actor="mission-commander",
                    action_type="operations-view-selected",
                ).entries
            ],
            ["view-review-journal-1", "view-queue-journal-2"],
        )
        self.assertEqual(journal.inspect(mission_id="unknown-mission").entries, ())

    def test_activity_journal_filters_by_recorded_time_window(self) -> None:
        snapshots = self.load_service()
        sync = WorkspaceSyncService(snapshots)
        scope = ConversationScope(
            kind="working-directory",
            target_id=str(self.target_repo),
            label="target",
        )

        with patch("albert_mvp.workspace.datetime") as clock:
            clock.now.side_effect = [
                datetime(2026, 6, 26, 10, 0, tzinfo=timezone.utc),
                datetime(2026, 6, 26, 10, 10, tzinfo=timezone.utc),
            ]
            sync.submit_action(
                WorkspaceAction(
                    correlation_id="view-review-time-1",
                    expected_revision=1,
                    active_mission_id="command-deck",
                    conversation_scope=scope,
                    operations_view="review-workspace",
                )
            )
            sync.submit_action(
                WorkspaceAction(
                    correlation_id="view-activity-time-2",
                    expected_revision=2,
                    active_mission_id="command-deck",
                    conversation_scope=scope,
                    operations_view="activity",
                )
            )

        journal = ActivityJournalService(self.load_service())

        self.assertEqual(
            [
                entry.correlation_id
                for entry in journal.inspect(started_at="2026-06-26T10:05:00Z").entries
            ],
            ["view-activity-time-2"],
        )
        self.assertEqual(
            [
                entry.correlation_id
                for entry in journal.inspect(ended_at="2026-06-26T10:05:00Z").entries
            ],
            ["view-review-time-1"],
        )
        self.assertEqual(
            [
                entry.correlation_id
                for entry in journal.inspect(
                    started_at="2026-06-26T09:55:00Z",
                    ended_at="2026-06-26T10:10:00Z",
                ).entries
            ],
            ["view-review-time-1", "view-activity-time-2"],
        )

    def test_activity_journal_records_review_decision_with_links(self) -> None:
        snapshots = self.load_service()
        mission = snapshots._primary_mission
        mission.approve_issue("ISS-01")
        session = mission.launch_issue("ISS-01")
        mission.record_evidence(
            session.session_id,
            EvidencePackage(
                changed_files=["src/app.py"],
                diff_summary="Added the Activity Journal review decision link.",
                commands_run=["python3 -m unittest tests.test_workspace_snapshot"],
                test_results="Focused workspace tests passed.",
                known_risks="None.",
                proposed_context_updates="Document Activity Journal review links.",
                artifact_links=["app-local://evidence/session-ISS-01-1"],
            ),
        )

        acknowledgement = ReviewWorkspaceService(snapshots).decide(
            correlation_id="review-journal-accept-1",
            expected_revision=1,
            session_id=session.session_id,
            decision="accept",
            reason="Evidence is complete and linked.",
        )

        projection = ActivityJournalService(self.load_service()).inspect()
        self.assertEqual(acknowledgement.outcome, "acknowledged")
        self.assertEqual(
            [entry.action_type for entry in projection.entries],
            ["evidence-package-submitted", "review-decision"],
        )
        entry = projection.entries[1]
        self.assertEqual(entry.actor, "mission-commander")
        self.assertEqual(entry.action_type, "review-decision")
        self.assertEqual(entry.correlation_id, "review-journal-accept-1")
        self.assertIn("Approved", entry.summary)
        self.assertEqual(entry.evidence_links, ("app-local://evidence/session-ISS-01-1",))
        self.assertEqual(
            [
                (entity.entity_type, entity.entity_id, entity.href)
                for entity in entry.affected_entities
            ],
            [
                ("mission", "command-deck", "app-local://missions/command-deck"),
                (
                    "issue-slice",
                    "ISS-01",
                    "app-local://missions/command-deck/issues/ISS-01",
                ),
                (
                    "local-agent-session",
                    session.session_id,
                    f"app-local://missions/command-deck/sessions/{session.session_id}",
                ),
                (
                    "evidence-package",
                    session.session_id,
                    "app-local://evidence/session-ISS-01-1",
                ),
            ],
        )

    def test_activity_journal_records_acknowledged_workspace_queue_decision_with_links(
        self,
    ) -> None:
        snapshots = self.load_service()
        mission = snapshots._primary_mission
        mission.approve_issue("ISS-01")
        queue = WorkspaceQueueService(snapshots)
        proposal = queue.propose_issue_contract_change(
            correlation_id="queue-journal-proposal-1",
            expected_revision=1,
            issue_id="ISS-01",
            source="issue-slice-inspector",
            acceptance_criteria=["Record the acknowledged queue decision."],
        )

        acknowledgement = queue.decide(
            correlation_id="queue-journal-decision-1",
            expected_revision=proposal.revision,
            item_id=proposal.item_id,
            decision="approve",
            reason="Accepted by the Mission Commander.",
        )

        projection = ActivityJournalService(self.load_service()).inspect()
        self.assertEqual(acknowledgement.outcome, "acknowledged")
        self.assertEqual(len(projection.entries), 1)
        entry = projection.entries[0]
        self.assertTrue(entry.recorded_at.endswith("Z"))
        self.assertEqual(entry.actor, "mission-commander")
        self.assertEqual(entry.action_type, "workspace-queue-decision")
        self.assertEqual(entry.correlation_id, "queue-journal-decision-1")
        self.assertIn("Approved", entry.summary)
        self.assertIn(proposal.item_id, entry.summary)
        self.assertEqual(entry.evidence_links, ())
        self.assertEqual(
            [
                (entity.entity_type, entity.entity_id, entity.href)
                for entity in entry.affected_entities
            ],
            [
                ("mission", "command-deck", "app-local://missions/command-deck"),
                (
                    "issue-slice",
                    "ISS-01",
                    "app-local://missions/command-deck/issues/ISS-01",
                ),
                (
                    "workspace-queue-item",
                    proposal.item_id,
                    f"workspace-queue#{proposal.item_id}",
                ),
                (
                    "workspace-queue-decision",
                    "queue-journal-decision-1",
                    f"workspace-queue#{proposal.item_id}",
                ),
            ],
        )

    def test_activity_journal_records_acknowledged_mission_draft_creation_with_links(
        self,
    ) -> None:
        snapshots = self.load_service()

        acknowledgement = MissionDraftService(snapshots).create_draft(
            correlation_id="mission-draft-journal-create-1",
            expected_revision=1,
            proposed_goal="Create linked Activity Journal coverage.",
            selected_ad_hoc_ids=[],
            excluded_ad_hoc_ids=[],
            new_work_items=["Record Mission Draft creation."],
            dependencies=[],
            unresolved_decisions=[],
        )

        projection = ActivityJournalService(self.load_service()).inspect()
        self.assertEqual(acknowledgement.outcome, "acknowledged")
        self.assertEqual(len(projection.entries), 1)
        entry = projection.entries[0]
        self.assertTrue(entry.recorded_at.endswith("Z"))
        self.assertEqual(entry.actor, "mission-commander")
        self.assertEqual(entry.action_type, "mission-draft-created")
        self.assertEqual(entry.correlation_id, "mission-draft-journal-create-1")
        self.assertIn(acknowledgement.draft_id, entry.summary)
        self.assertEqual(entry.evidence_links, ())
        self.assertEqual(
            [
                (entity.entity_type, entity.entity_id, entity.href)
                for entity in entry.affected_entities
            ],
            [
                ("mission", "command-deck", "app-local://missions/command-deck"),
                (
                    "mission-draft",
                    acknowledgement.draft_id,
                    f"workspace-queue#{acknowledgement.draft_id}",
                ),
            ],
        )

    def test_activity_journal_records_confirmed_mission_draft_and_accepted_issue_link(
        self,
    ) -> None:
        snapshots = self.load_service()
        drafts = MissionDraftService(snapshots)
        created = drafts.create_draft(
            correlation_id="mission-draft-journal-confirm-create-1",
            expected_revision=1,
            proposed_goal="Confirm linked Activity Journal coverage.",
            selected_ad_hoc_ids=[],
            excluded_ad_hoc_ids=[],
            new_work_items=["Record Mission Draft confirmation."],
            dependencies=[],
            unresolved_decisions=[],
        )

        acknowledgement = drafts.confirm_draft(
            correlation_id="mission-draft-journal-confirm-1",
            expected_revision=created.revision,
            draft_id=created.draft_id,
            reason="The Mission Commander accepted this scope.",
        )

        entries = ActivityJournalService(self.load_service()).inspect().entries
        self.assertEqual(acknowledgement.outcome, "acknowledged")
        self.assertEqual(
            [entry.action_type for entry in entries],
            ["mission-draft-created", "mission-draft-confirmed"],
        )
        entry = entries[1]
        self.assertEqual(entry.actor, "mission-commander")
        self.assertEqual(entry.correlation_id, "mission-draft-journal-confirm-1")
        self.assertIn(acknowledgement.accepted_issue_id, entry.summary)
        self.assertEqual(
            [
                (entity.entity_type, entity.entity_id, entity.href)
                for entity in entry.affected_entities
            ],
            [
                ("mission", "command-deck", "app-local://missions/command-deck"),
                (
                    "mission-draft",
                    created.draft_id,
                    f"workspace-queue#{created.draft_id}",
                ),
                (
                    "issue-slice",
                    acknowledgement.accepted_issue_id,
                    (
                        "app-local://missions/command-deck/issues/"
                        f"{acknowledgement.accepted_issue_id}"
                    ),
                ),
            ],
        )

    def test_activity_journal_records_acknowledged_mission_draft_revision(self) -> None:
        snapshots = self.load_service()
        drafts = MissionDraftService(snapshots)
        created = drafts.create_draft(
            correlation_id="mission-draft-journal-update-create-1",
            expected_revision=1,
            proposed_goal="Create initial Activity Journal coverage.",
            selected_ad_hoc_ids=[],
            excluded_ad_hoc_ids=[],
            new_work_items=["Record the initial draft."],
            dependencies=[],
            unresolved_decisions=[],
        )

        acknowledgement = drafts.update_draft(
            correlation_id="mission-draft-journal-update-1",
            expected_revision=created.revision,
            draft_id=created.draft_id,
            proposed_goal="Create revised Activity Journal coverage.",
            selected_ad_hoc_ids=[],
            excluded_ad_hoc_ids=[],
            new_work_items=["Record the revised draft."],
            dependencies=[],
            unresolved_decisions=[],
        )

        entries = ActivityJournalService(self.load_service()).inspect().entries
        self.assertEqual(acknowledgement.outcome, "acknowledged")
        self.assertEqual(
            [entry.action_type for entry in entries],
            ["mission-draft-created", "mission-draft-updated"],
        )
        entry = entries[1]
        self.assertEqual(entry.correlation_id, "mission-draft-journal-update-1")
        self.assertIn(created.draft_id, entry.summary)
        self.assertEqual(entry.affected_entities[1].entity_type, "mission-draft")
        self.assertEqual(entry.affected_entities[1].label, "Create revised Activity Journal coverage.")

    def test_activity_journal_records_acknowledged_mission_draft_abandonment(self) -> None:
        snapshots = self.load_service()
        drafts = MissionDraftService(snapshots)
        created = drafts.create_draft(
            correlation_id="mission-draft-journal-abandon-create-1",
            expected_revision=1,
            proposed_goal="Do not accept this Activity Journal scope.",
            selected_ad_hoc_ids=[],
            excluded_ad_hoc_ids=[],
            new_work_items=["Discard this draft after review."],
            dependencies=[],
            unresolved_decisions=[],
        )

        acknowledgement = drafts.abandon_draft(
            correlation_id="mission-draft-journal-abandon-1",
            expected_revision=created.revision,
            draft_id=created.draft_id,
            reason="The Mission Commander rejected this scope.",
        )

        entries = ActivityJournalService(self.load_service()).inspect().entries
        self.assertEqual(acknowledgement.outcome, "acknowledged")
        self.assertEqual(
            [entry.action_type for entry in entries],
            ["mission-draft-created", "mission-draft-abandoned"],
        )
        entry = entries[1]
        self.assertEqual(entry.correlation_id, "mission-draft-journal-abandon-1")
        self.assertIn(created.draft_id, entry.summary)
        self.assertEqual(entry.affected_entities[1].entity_type, "mission-draft")
        self.assertEqual(entry.affected_entities[1].entity_id, created.draft_id)

    def test_activity_journal_records_frontier_confirmation_request_with_links(self) -> None:
        snapshots = self.load_service()
        snapshots._primary_mission.approve_issue("ISS-01")

        acknowledgement = WorkspaceQueueService(snapshots).request_frontier_confirmation(
            correlation_id="frontier-journal-confirmation-1",
            expected_revision=1,
            issue_id="ISS-01",
            source="frontier-router",
            requested_action="Expand the launch boundary",
            affected_boundary="allowed_paths",
            consequence="Approval permits an additional generated file.",
            payload={"allowed_paths": ["generated/report.md"]},
        )

        projection = ActivityJournalService(self.load_service()).inspect()
        self.assertEqual(acknowledgement.outcome, "acknowledged")
        self.assertEqual(len(projection.entries), 1)
        entry = projection.entries[0]
        self.assertEqual(entry.actor, "frontier-model")
        self.assertEqual(entry.action_type, "frontier-confirmation-requested")
        self.assertEqual(entry.correlation_id, "frontier-journal-confirmation-1")
        self.assertIn(acknowledgement.item_id, entry.summary)
        self.assertEqual(
            [
                (entity.entity_type, entity.entity_id, entity.href)
                for entity in entry.affected_entities
            ],
            [
                ("mission", "command-deck", "app-local://missions/command-deck"),
                (
                    "issue-slice",
                    "ISS-01",
                    "app-local://missions/command-deck/issues/ISS-01",
                ),
                (
                    "workspace-queue-item",
                    acknowledgement.item_id,
                    f"workspace-queue#{acknowledgement.item_id}",
                ),
            ],
        )

    def test_activity_journal_records_orchestrator_ad_hoc_session_launch_with_links(
        self,
    ) -> None:
        snapshots = self.load_service()
        origin = AgentConsoleHistoryService(snapshots).append(
            role="user",
            content="Launch a bounded local-agent documentation session.",
            outcome="proposed",
            source="mission-commander",
            expected_revision=1,
            expected_scope=snapshots.snapshot().conversation_scope,
        )
        queue = WorkspaceQueueService(snapshots)
        proposal = queue.propose_ad_hoc_delegation(
            correlation_id="orchestrator-journal-proposal-1",
            expected_revision=1,
            source="agent-console",
            scope=snapshots.snapshot().conversation_scope,
            acceptance_criteria=["Document the focused Activity Journal tests."],
            allowed_paths=["docs/activity-journal.md"],
            command_policy={},
            proposed_agent="qwen-coder-local-1",
            originating_message_id=origin.message_id,
        )

        acknowledgement = queue.decide(
            correlation_id="orchestrator-journal-decision-1",
            expected_revision=proposal.revision,
            item_id=proposal.item_id,
            decision="approve",
            reason="Approved within the proposed boundaries.",
        )

        entries = ActivityJournalService(self.load_service()).inspect().entries
        self.assertEqual(acknowledgement.outcome, "acknowledged")
        self.assertEqual(
            [(entry.actor, entry.action_type) for entry in entries],
            [
                ("mission-commander", "workspace-queue-decision"),
                ("orchestrator", "local-agent-session-launched"),
            ],
        )
        entry = entries[1]
        session_id = "session-ADHOC-000001-1"
        self.assertEqual(entry.correlation_id, "orchestrator-journal-decision-1")
        self.assertIn(session_id, entry.summary)
        self.assertEqual(
            [
                (entity.entity_type, entity.entity_id, entity.href)
                for entity in entry.affected_entities
            ],
            [
                ("mission", "command-deck", "app-local://missions/command-deck"),
                (
                    "ad-hoc-delegation",
                    "ADHOC-000001",
                    "app-local://missions/command-deck/issues/ADHOC-000001",
                ),
                (
                    "local-agent-session",
                    session_id,
                    f"app-local://missions/command-deck/sessions/{session_id}",
                ),
                (
                    "workspace-queue-item",
                    proposal.item_id,
                    f"workspace-queue#{proposal.item_id}",
                ),
            ],
        )

    def test_activity_journal_records_local_agent_validated_evidence_with_links(self) -> None:
        snapshots = self.load_service()
        mission = snapshots._primary_mission
        mission.approve_issue("ISS-01")
        session = mission.launch_issue("ISS-01")

        mission.record_evidence(
            session.session_id,
            EvidencePackage(
                changed_files=["src/activity.py"],
                diff_summary="Recorded a meaningful Local Agent completion.",
                commands_run=["python3 -m unittest tests.test_workspace_snapshot"],
                test_results="Workspace tests passed.",
                known_risks="None.",
                proposed_context_updates="Document Local Agent Activity entries.",
                artifact_links=["app-local://evidence/local-agent-session-1"],
            ),
        )

        projection = ActivityJournalService(self.load_service()).inspect()
        self.assertEqual(len(projection.entries), 1)
        entry = projection.entries[0]
        self.assertEqual(entry.actor, "local-agent")
        self.assertEqual(entry.action_type, "evidence-package-submitted")
        self.assertEqual(
            entry.correlation_id,
            f"evidence:{mission.mission_id}:{session.session_id}",
        )
        self.assertIn(session.session_id, entry.summary)
        self.assertEqual(
            entry.evidence_links,
            ("app-local://evidence/local-agent-session-1",),
        )
        self.assertEqual(
            [
                (entity.entity_type, entity.entity_id, entity.href)
                for entity in entry.affected_entities
            ],
            [
                ("mission", "command-deck", "app-local://missions/command-deck"),
                (
                    "issue-slice",
                    "ISS-01",
                    "app-local://missions/command-deck/issues/ISS-01",
                ),
                (
                    "local-agent-session",
                    session.session_id,
                    f"app-local://missions/command-deck/sessions/{session.session_id}",
                ),
                (
                    "evidence-package",
                    session.session_id,
                    "app-local://evidence/local-agent-session-1",
                ),
            ],
        )

    def test_activity_journal_write_failure_surfaces_after_canonical_queue_persistence(
        self,
    ) -> None:
        snapshots = self.load_service()
        mission = snapshots._primary_mission
        mission.approve_issue("ISS-01")
        queue = WorkspaceQueueService(snapshots)
        proposal = queue.propose_issue_contract_change(
            correlation_id="journal-write-failure-proposal-1",
            expected_revision=1,
            issue_id="ISS-01",
            source="issue-slice-inspector",
            acceptance_criteria=["Canonical state persists before journal failure."],
        )
        atomic_write = WorkspaceSnapshotService._write_json_atomically

        def fail_activity_write(path: Path, data: dict[str, object]) -> None:
            if path.name == "activity-journal.json":
                raise OSError("simulated full disk")
            atomic_write(path, data)

        with patch.object(
            WorkspaceSnapshotService,
            "_write_json_atomically",
            side_effect=fail_activity_write,
        ):
            with self.assertRaisesRegex(
                WorkspacePersistenceError,
                "Activity Journal persistence write failed",
            ):
                queue.decide(
                    correlation_id="journal-write-failure-decision-1",
                    expected_revision=proposal.revision,
                    item_id=proposal.item_id,
                    decision="approve",
                    reason="Exercise the explicit partial-write contract.",
                )

        reloaded = self.load_service()
        persisted_item = WorkspaceQueueService(reloaded).inspect().items[0]
        self.assertEqual(persisted_item.status, "approved")
        self.assertEqual(
            reloaded._primary_mission.issues["ISS-01"].acceptance_criteria,
            ["Canonical state persists before journal failure."],
        )
        self.assertEqual(ActivityJournalService(reloaded).inspect().entries, ())

        replayed = WorkspaceQueueService(reloaded).decide(
            correlation_id="journal-write-failure-decision-1",
            expected_revision=proposal.revision,
            item_id=proposal.item_id,
            decision="approve",
            reason="Exercise the explicit partial-write contract.",
        )
        self.assertEqual(replayed.item_status, "approved")
        entries = ActivityJournalService(self.load_service()).inspect().entries
        self.assertEqual(
            [(entry.correlation_id, entry.action_type) for entry in entries],
            [
                (
                    "journal-write-failure-decision-1",
                    "workspace-queue-decision",
                )
            ],
        )

        replayed_again = WorkspaceQueueService(self.load_service()).decide(
            correlation_id="journal-write-failure-decision-1",
            expected_revision=proposal.revision,
            item_id=proposal.item_id,
            decision="approve",
            reason="Exercise the explicit partial-write contract.",
        )
        self.assertEqual(replayed_again, replayed)
        self.assertEqual(
            ActivityJournalService(self.load_service()).inspect().entries,
            entries,
        )

    def test_queue_audit_replay_tolerates_mutable_display_label_changes(self) -> None:
        snapshots = self.load_service()
        snapshots._primary_mission.approve_issue("ISS-01")
        queue = WorkspaceQueueService(snapshots)
        proposal = queue.propose_issue_contract_change(
            correlation_id="mutable-label-proposal-1",
            expected_revision=1,
            issue_id="ISS-01",
            source="issue-slice-inspector",
            acceptance_criteria=["Keep audit identity stable across label changes."],
        )
        decision = {
            "correlation_id": "mutable-label-decision-1",
            "expected_revision": proposal.revision,
            "item_id": proposal.item_id,
            "decision": "reject",
            "reason": "Exercise immutable audit identity.",
        }
        acknowledged = queue.decide(**decision)
        before = ActivityJournalService(snapshots).inspect().entries

        snapshots._primary_mission.prd_title = "Renamed Mission Display Label"
        snapshots._primary_mission.issues["ISS-01"].title = (
            "Renamed Issue Display Label"
        )
        replayed = queue.decide(**decision)

        self.assertEqual(replayed, acknowledged)
        self.assertEqual(ActivityJournalService(snapshots).inspect().entries, before)

    def test_queue_replay_repairs_only_the_missing_orchestrator_audit_phase(
        self,
    ) -> None:
        snapshots = self.load_service()
        origin = AgentConsoleHistoryService(snapshots).append(
            role="user",
            content="Launch one crash-recoverable bounded local-agent session.",
            outcome="proposed",
            source="mission-commander",
            expected_revision=1,
            expected_scope=snapshots.snapshot().conversation_scope,
        )
        queue = WorkspaceQueueService(snapshots)
        proposal = queue.propose_ad_hoc_delegation(
            correlation_id="partial-audit-proposal-1",
            expected_revision=1,
            source="agent-console",
            scope=snapshots.snapshot().conversation_scope,
            acceptance_criteria=["Recover the second audit phase exactly once."],
            allowed_paths=["docs/activity-journal.md"],
            command_policy={},
            proposed_agent="qwen-coder-local-1",
            originating_message_id=origin.message_id,
        )

        with patch.object(
            ActivityJournalService,
            "record_orchestrator_session_launched",
            autospec=True,
            side_effect=WorkspacePersistenceError(
                "simulated orchestrator audit phase failure"
            ),
        ):
            with self.assertRaisesRegex(
                WorkspacePersistenceError,
                "simulated orchestrator audit phase failure",
            ):
                queue.decide(
                    correlation_id="partial-audit-decision-1",
                    expected_revision=proposal.revision,
                    item_id=proposal.item_id,
                    decision="approve",
                    reason="Approve the exact bounded task.",
                )

        first_entries = ActivityJournalService(self.load_service()).inspect().entries
        self.assertEqual(
            [(entry.correlation_id, entry.action_type) for entry in first_entries],
            [("partial-audit-decision-1", "workspace-queue-decision")],
        )

        reloaded = self.load_service()
        replayed = WorkspaceQueueService(reloaded).decide(
            correlation_id="partial-audit-decision-1",
            expected_revision=proposal.revision,
            item_id=proposal.item_id,
            decision="approve",
            reason="Approve the exact bounded task.",
        )
        replayed_again = WorkspaceQueueService(self.load_service()).decide(
            correlation_id="partial-audit-decision-1",
            expected_revision=proposal.revision,
            item_id=proposal.item_id,
            decision="approve",
            reason="Approve the exact bounded task.",
        )
        self.assertEqual(replayed_again, replayed)
        entries = ActivityJournalService(self.load_service()).inspect().entries
        self.assertEqual(
            [(entry.correlation_id, entry.action_type) for entry in entries],
            [
                ("partial-audit-decision-1", "workspace-queue-decision"),
                ("partial-audit-decision-1", "local-agent-session-launched"),
            ],
        )
        self.assertEqual(
            list(self.load_service()._primary_mission.sessions),
            [replayed.session_id],
        )

    def test_queue_replay_refreshes_a_long_lived_mission_before_audit_repair(
        self,
    ) -> None:
        stale_snapshots = self.load_service()
        writer_snapshots = self.load_service()
        origin = AgentConsoleHistoryService(writer_snapshots).append(
            role="user",
            content="Launch one session while another process keeps stale Mission state.",
            outcome="proposed",
            source="mission-commander",
            expected_revision=1,
            expected_scope=writer_snapshots.snapshot().conversation_scope,
        )
        writer_queue = WorkspaceQueueService(writer_snapshots)
        proposal = writer_queue.propose_ad_hoc_delegation(
            correlation_id="stale-mission-audit-proposal-1",
            expected_revision=1,
            source="agent-console",
            scope=writer_snapshots.snapshot().conversation_scope,
            acceptance_criteria=["Repair audit from a long-lived process."],
            allowed_paths=["docs/activity-journal.md"],
            command_policy={},
            proposed_agent="qwen-coder-local-1",
            originating_message_id=origin.message_id,
        )
        with patch.object(
            ActivityJournalService,
            "record_orchestrator_session_launched",
            autospec=True,
            side_effect=WorkspacePersistenceError(
                "simulated missing orchestrator launch audit"
            ),
        ):
            with self.assertRaisesRegex(
                WorkspacePersistenceError,
                "missing orchestrator launch audit",
            ):
                writer_queue.decide(
                    correlation_id="stale-mission-audit-decision-1",
                    expected_revision=proposal.revision,
                    item_id=proposal.item_id,
                    decision="approve",
                    reason="Approve one bounded session.",
                )

        self.assertEqual(stale_snapshots._primary_mission.sessions, {})
        replayed = WorkspaceQueueService(stale_snapshots).decide(
            correlation_id="stale-mission-audit-decision-1",
            expected_revision=proposal.revision,
            item_id=proposal.item_id,
            decision="approve",
            reason="Approve one bounded session.",
        )

        self.assertIn(replayed.session_id, stale_snapshots._primary_mission.sessions)
        self.assertEqual(
            [
                (entry.correlation_id, entry.action_type)
                for entry in ActivityJournalService(self.load_service()).inspect().entries
            ],
            [
                ("stale-mission-audit-decision-1", "workspace-queue-decision"),
                ("stale-mission-audit-decision-1", "local-agent-session-launched"),
            ],
        )

    def test_restart_reconstructs_canonical_snapshot_and_retains_separate_journal_order(
        self,
    ) -> None:
        snapshots = self.load_service()
        mission = snapshots._primary_mission
        mission.approve_issue("ISS-01")
        scope = snapshots.snapshot().conversation_scope
        WorkspaceSyncService(snapshots).submit_action(
            WorkspaceAction(
                correlation_id="restart-journal-view-1",
                expected_revision=1,
                active_mission_id="command-deck",
                conversation_scope=scope,
                operations_view="activity",
            )
        )
        WorkspaceQueueService(snapshots).request_frontier_confirmation(
            correlation_id="restart-journal-frontier-2",
            expected_revision=2,
            issue_id="ISS-01",
            source="frontier-router",
            requested_action="Confirm restart-safe launch scope",
            affected_boundary="allowed_paths",
            consequence="Approval would extend the launch boundary.",
            payload={"allowed_paths": ["docs/restart.md"]},
        )
        canonical_before_restart = snapshots.snapshot().to_dict()
        journal_before_restart = ActivityJournalService(snapshots).inspect()

        reloaded = self.load_service()
        canonical_after_restart = reloaded.snapshot().to_dict()
        journal_after_restart = ActivityJournalService(reloaded).inspect()

        self.assertEqual(canonical_after_restart, canonical_before_restart)
        self.assertEqual(journal_after_restart, journal_before_restart)
        self.assertEqual(
            [entry.correlation_id for entry in journal_after_restart.entries],
            ["restart-journal-view-1", "restart-journal-frontier-2"],
        )
        self.assertEqual(
            [entry.actor for entry in journal_after_restart.entries],
            ["mission-commander", "frontier-model"],
        )
        self.assertNotEqual(
            ActivityJournalService(reloaded).journal_path,
            reloaded._primary_mission.runtime_path,
        )

    def test_activity_journal_excludes_transient_output_and_failed_actions(self) -> None:
        snapshots = self.load_service()
        history = AgentConsoleHistoryService(snapshots)
        with self.assertRaisesRegex(AlbertError, "Transient stream telemetry"):
            history.append(
                role="assistant",
                content="partial model token fragment",
                outcome="pending",
                source="frontier-model-stream",
            )
        with self.assertRaisesRegex(AlbertError, "Transient stream telemetry"):
            history.append(
                role="system",
                content="raw terminal byte chunk",
                outcome="pending",
                source="shell-terminal-stream",
            )
        mission = snapshots._primary_mission
        mission.approve_issue("ISS-01")
        session = mission.launch_issue("ISS-01")

        with self.assertRaises(EvidenceValidationError):
            mission.record_evidence(
                session.session_id,
                EvidencePackage(changed_files=["src/incomplete.py"]),
            )
        with self.assertRaises(WorkspaceStaleActionError):
            WorkspaceSyncService(snapshots).submit_action(
                WorkspaceAction(
                    correlation_id="unacknowledged-stale-action-1",
                    expected_revision=0,
                    active_mission_id="command-deck",
                    conversation_scope=snapshots.snapshot().conversation_scope,
                    operations_view="activity",
                )
            )

        self.assertEqual(ActivityJournalService(self.load_service()).inspect().entries, ())

    def test_malformed_event_order_is_rejected_without_projecting_updates(self) -> None:
        snapshots = self.load_service()
        sync = WorkspaceSyncService(snapshots)
        sync.submit_action(
            WorkspaceAction(
                correlation_id="view-review-1",
                expected_revision=1,
                active_mission_id="command-deck",
                conversation_scope=ConversationScope(
                    kind="working-directory",
                    target_id=str(self.target_repo),
                    label="target",
                ),
                operations_view="review-workspace",
            )
        )
        persisted = json.loads(snapshots.preferences_path.read_text(encoding="utf-8"))
        persisted["events"][0]["revision"] = 3
        snapshots.preferences_path.write_text(json.dumps(persisted), encoding="utf-8")

        with self.assertRaises(WorkspacePersistenceError):
            sync.updates_after(1)

    def test_event_lag_requires_resynchronization_instead_of_partial_batch(self) -> None:
        snapshots = self.load_service()
        sync = WorkspaceSyncService(snapshots)
        scope = ConversationScope(
            kind="working-directory",
            target_id=str(self.target_repo),
            label="target",
        )
        sync.submit_action(
            WorkspaceAction("view-review-1", 1, "command-deck", scope, "review-workspace")
        )
        sync.submit_action(
            WorkspaceAction("view-activity-2", 2, "command-deck", scope, "activity")
        )
        persisted = json.loads(snapshots.preferences_path.read_text(encoding="utf-8"))
        persisted["events"] = persisted["events"][1:]
        snapshots.preferences_path.write_text(json.dumps(persisted), encoding="utf-8")

        with self.assertRaises(WorkspaceRevisionGapError) as raised:
            sync.updates_after(1)

        self.assertEqual(raised.exception.requested_revision, 1)
        self.assertEqual(raised.exception.current_revision, 3)

    def test_empty_tracker_returns_valid_empty_workspace_snapshot(self) -> None:
        (self.tracker / "issues" / "01-restore.md").unlink()

        snapshot = self.load_service().snapshot()

        self.assertEqual(snapshot.workspace_session.status, "empty")
        self.assertEqual(snapshot.active_mission.issue_count, 0)
        self.assertEqual(snapshot.mission_board["ordered_issue_ids"], [])

    def test_mission_board_projects_authoritative_issue_graph_and_inspector_payload(self) -> None:
        (self.tracker / "issues" / "02-sync.md").write_text(
            ISSUE.replace("Restore the workspace session.", "Synchronize live state.").replace(
                "None - can start immediately",
                "- `01-restore.md`",
            ),
            encoding="utf-8",
        )
        snapshots = self.load_service()
        mission = snapshots._mission
        mission.approve_issue("ISS-02")
        session = LocalAgentSession(
            session_id="session-ISS-02-1",
            issue_id="ISS-02",
            assigned_agent="qwen-coder-local",
            worktree_path=self.target_repo / ".albert-worktrees" / "ISS-02",
            task_packet={
                "agent_config": {
                    "role": "local-agent",
                    "provider": "ollama",
                    "model": "qwen2.5-coder:14b",
                }
            },
            status="launched",
            evidence=EvidencePackage(
                changed_files=["mission-control/src/App.tsx"],
                diff_summary="Added Mission Board inspector.",
                commands_run=["npm test -- --run App.test.tsx"],
                test_results="Frontend interaction test passed.",
                known_risks="Disconnected before review.",
                proposed_context_updates="Issue 06 progress.",
                artifact_links=["app-local://evidence/ISS-02"],
            ),
            evidence_valid=True,
            runner_started_at="2026-06-25T10:00:00Z",
        )
        mission.sessions[session.session_id] = session

        board = snapshots.snapshot().mission_board
        sync = next(issue for issue in board["issue_slices"] if issue["issue_id"] == "ISS-02")

        self.assertEqual(board["ready_issue_ids"], [])
        self.assertEqual(sync["lifecycle"], "Approved")
        self.assertEqual(sync["progress"], "Waiting on ISS-01")
        self.assertEqual(sync["blockers"][0]["issue_id"], "ISS-01")
        self.assertEqual(sync["blockers"][0]["satisfied"], False)
        self.assertEqual(sync["accepted_boundary"]["what_to_build"], "Synchronize live state.")
        self.assertEqual(sync["sessions"][0]["session_id"], "session-ISS-02-1")
        self.assertEqual(sync["sessions"][0]["provider"], "ollama")
        self.assertEqual(sync["sessions"][0]["model"], "qwen2.5-coder:14b")
        self.assertEqual(sync["evidence"]["test_results"], "Frontend interaction test passed.")
        self.assertEqual(sync["working_context_sources"][0]["kind"], "shared-context")

    def test_corrupt_preferences_return_structured_persistence_failure(self) -> None:
        service = self.load_service()
        preferences = service.preferences_path
        preferences.write_text("{broken", encoding="utf-8")
        output = io.StringIO()
        error = io.StringIO()

        with redirect_stdout(output), redirect_stderr(error):
            exit_code = main(
                [
                    "workspace-snapshot",
                    "--target-repo",
                    str(self.target_repo),
                    "--tracker-dir",
                    str(self.tracker),
                    "--runtime-root",
                    str(self.runtime),
                    "--mission-id",
                    "command-deck",
                ]
            )

        failure = json.loads(error.getvalue())
        self.assertEqual(exit_code, 1)
        self.assertEqual(output.getvalue(), "")
        self.assertEqual(failure["error"]["code"], "persistence-read-failure")
        self.assertEqual(failure["error"]["recoverable"], True)

    def test_cli_can_load_prd_and_issue_records_from_explicit_separate_locations(self) -> None:
        prd_tracker = self.root / "command-deck-tracker"
        prd_tracker.mkdir()
        (prd_tracker / "PRD.md").write_text("# Current Command Deck\n", encoding="utf-8")
        output = io.StringIO()

        with redirect_stdout(output):
            exit_code = main(
                [
                    "workspace-snapshot",
                    "--target-repo",
                    str(self.target_repo),
                    "--tracker-dir",
                    str(prd_tracker),
                    "--issues-dir",
                    str(self.tracker / "issues"),
                    "--runtime-root",
                    str(self.runtime),
                    "--mission-id",
                    "current-command-deck",
                ]
            )

        snapshot = json.loads(output.getvalue())
        self.assertEqual(exit_code, 0)
        self.assertEqual(snapshot["active_mission"]["title"], "Current Command Deck")
        self.assertEqual(snapshot["mission_board"]["ordered_issue_ids"], ["ISS-01"])

    def test_cli_acknowledges_correlated_workspace_action(self) -> None:
        output = io.StringIO()

        with redirect_stdout(output):
            exit_code = main(
                [
                    "workspace-action",
                    "--target-repo",
                    str(self.target_repo),
                    "--tracker-dir",
                    str(self.tracker),
                    "--runtime-root",
                    str(self.runtime),
                    "--mission-id",
                    "command-deck",
                    "--correlation-id",
                    "view-activity-1",
                    "--expected-revision",
                    "1",
                    "--operations-view",
                    "activity",
                ]
            )

        acknowledgement = json.loads(output.getvalue())
        self.assertEqual(exit_code, 0)
        self.assertEqual(acknowledgement["correlation_id"], "view-activity-1")
        self.assertEqual(acknowledgement["outcome"], "acknowledged")
        self.assertEqual(acknowledgement["revision"], 2)

    def test_cli_returns_ordered_workspace_updates_after_known_revision(self) -> None:
        sync = WorkspaceSyncService(self.load_service())
        current = self.load_service().snapshot()
        sync.submit_action(
            WorkspaceAction(
                "view-activity-1",
                1,
                "command-deck",
                current.conversation_scope,
                "activity",
            )
        )
        output = io.StringIO()

        with redirect_stdout(output):
            exit_code = main(
                [
                    "workspace-updates",
                    "--target-repo",
                    str(self.target_repo),
                    "--tracker-dir",
                    str(self.tracker),
                    "--runtime-root",
                    str(self.runtime),
                    "--mission-id",
                    "command-deck",
                    "--after-revision",
                    "1",
                ]
            )

        batch = json.loads(output.getvalue())
        self.assertEqual(exit_code, 0)
        self.assertEqual(batch["after_revision"], 1)
        self.assertEqual(batch["current_revision"], 2)
        self.assertEqual([event["revision"] for event in batch["events"]], [2])
        self.assertEqual(batch["events"][0]["correlation_id"], "view-activity-1")

    def test_cli_returns_structured_stale_action_without_success_output(self) -> None:
        current = self.load_service().snapshot()
        WorkspaceSyncService(self.load_service()).submit_action(
            WorkspaceAction(
                "view-activity-1",
                1,
                "command-deck",
                current.conversation_scope,
                "activity",
            )
        )
        output = io.StringIO()
        error = io.StringIO()

        with redirect_stdout(output), redirect_stderr(error):
            exit_code = main(
                [
                    "workspace-action",
                    "--target-repo",
                    str(self.target_repo),
                    "--tracker-dir",
                    str(self.tracker),
                    "--runtime-root",
                    str(self.runtime),
                    "--mission-id",
                    "command-deck",
                    "--correlation-id",
                    "view-stale-2",
                    "--expected-revision",
                    "1",
                    "--operations-view",
                    "mission-board",
                ]
            )

        failure = json.loads(error.getvalue())
        self.assertEqual(exit_code, 1)
        self.assertEqual(output.getvalue(), "")
        self.assertEqual(failure["error"]["code"], "stale-action")
        self.assertEqual(failure["error"]["current_revision"], 2)
        self.assertEqual(self.load_service().snapshot().operations_view, "activity")

    def test_cli_deliberately_changes_mission_scope_without_authorization_side_effects(self) -> None:
        issue_path = self.tracker / "issues" / "01-restore.md"
        original_issue = issue_path.read_text(encoding="utf-8")
        before = AlbertMission(
            target_repo=self.target_repo,
            tracker_dir=self.tracker,
            runtime_root=self.runtime,
            mission_id="command-deck",
        ).load()
        before_detail = before.issue_detail("ISS-01")
        output = io.StringIO()

        with redirect_stdout(output):
            exit_code = main(
                [
                    "workspace-scope",
                    "--target-repo",
                    str(self.target_repo),
                    "--tracker-dir",
                    str(self.tracker),
                    "--runtime-root",
                    str(self.runtime),
                    "--mission-id",
                    "command-deck",
                    "--correlation-id",
                    "scope-mission-1",
                    "--expected-revision",
                    "1",
                    "--scope-kind",
                    "mission",
                    "--scope-target",
                    "command-deck",
                    "--scope-label",
                    "Command Deck Mission",
                ]
            )

        acknowledgement = json.loads(output.getvalue())
        restored = self.load_service().snapshot()
        after = AlbertMission(
            target_repo=self.target_repo,
            tracker_dir=self.tracker,
            runtime_root=self.runtime,
            mission_id="command-deck",
        ).load()
        after_detail = after.issue_detail("ISS-01")
        self.assertEqual(exit_code, 0)
        self.assertEqual(acknowledgement["correlation_id"], "scope-mission-1")
        self.assertEqual(acknowledgement["revision"], 2)
        self.assertEqual(restored.conversation_scope.kind, "mission")
        self.assertEqual(restored.conversation_scope.target_id, "command-deck")
        self.assertEqual(restored.operations_view, "mission-board")
        self.assertEqual(issue_path.read_text(encoding="utf-8"), original_issue)
        self.assertEqual(after_detail["review_state"], before_detail["review_state"])
        self.assertEqual(after_detail["runtime_status"], before_detail["runtime_status"])
        self.assertEqual(after_detail["assigned_agent"], before_detail["assigned_agent"])
        self.assertEqual(len(after.sessions), len(before.sessions))

    def test_cli_rejects_mismatched_scope_action_target_identity(self) -> None:
        output = io.StringIO()
        error = io.StringIO()

        with redirect_stdout(output), redirect_stderr(error):
            exit_code = main(
                [
                    "workspace-scope",
                    "--target-repo",
                    str(self.target_repo),
                    "--tracker-dir",
                    str(self.tracker),
                    "--runtime-root",
                    str(self.runtime),
                    "--mission-id",
                    "command-deck",
                    "--correlation-id",
                    "scope-issue-mismatch",
                    "--expected-revision",
                    "1",
                    "--action-type",
                    "conversation-scope-change",
                    "--actor",
                    "mission-commander",
                    "--target-kind",
                    "conversation-scope",
                    "--target-id",
                    "ISS-02",
                    "--scope-kind",
                    "issue-slice",
                    "--scope-target",
                    "ISS-01",
                    "--scope-label",
                    "Restore workspace session",
                ]
            )

        self.assertEqual(exit_code, 1)
        self.assertEqual(output.getvalue(), "")
        self.assertIn(
            "Error: Conversation Scope action target id must match scope target",
            error.getvalue(),
        )
        self.assertEqual(self.load_service().snapshot().conversation_scope.kind, "working-directory")

    def test_agent_console_history_restores_proposed_message_with_original_scope(self) -> None:
        first = AgentConsoleHistoryService(self.load_service())

        appended = first.append(
            role="user",
            content="Review the restore boundary.",
            outcome="proposed",
            source="mission-commander",
        )
        restored = AgentConsoleHistoryService(self.load_service()).history()

        self.assertEqual(appended.message_id, "console-000001")
        self.assertEqual(appended.sequence, 1)
        self.assertEqual(restored, (appended,))
        self.assertEqual(restored[0].scope.kind, "working-directory")
        self.assertEqual(restored[0].scope.target_id, str(self.target_repo))

    def test_agent_console_history_rejects_malformed_scope_kind(self) -> None:
        history = AgentConsoleHistoryService(self.load_service())
        history.append(
            role="assistant",
            content="This is model analysis only.",
            outcome="model-commentary",
            source="frontier-model",
        )
        persisted = json.loads(history.history_path.read_text(encoding="utf-8"))
        persisted["messages"][0]["scope"]["kind"] = "selected-panel"
        history.history_path.write_text(json.dumps(persisted), encoding="utf-8")

        with self.assertRaises(WorkspacePersistenceError):
            AgentConsoleHistoryService(self.load_service()).history()

    def test_agent_console_history_rejects_receipt_identity_on_model_commentary(
        self,
    ) -> None:
        history = AgentConsoleHistoryService(self.load_service())

        with self.assertRaisesRegex(
            AlbertError,
            "Model commentary cannot carry an Orchestrator receipt identity",
        ):
            history.append(
                role="assistant",
                content="I completed the requested action.",
                outcome="model-commentary",
                source="frontier-model",
                correlation_id="forged-receipt-1",
                action_phase="accepted-completion",
                action_outcome="no-action",
                action_message="No action taken.",
            )

        history.append(
            role="assistant",
            content="This remains controller commentary.",
            outcome="model-commentary",
            source="frontier-model",
            action_outcome="no-action",
            action_message=(
                "No action taken. Controller prose is commentary and no correlated "
                "Orchestrator receipt exists."
            ),
        )
        persisted = json.loads(history.history_path.read_text(encoding="utf-8"))
        persisted["messages"][0]["correlation_id"] = "forged-receipt-2"
        persisted["messages"][0]["action_phase"] = "accepted-completion"
        history.history_path.write_text(json.dumps(persisted), encoding="utf-8")

        with self.assertRaisesRegex(
            WorkspacePersistenceError,
            "model commentary cannot carry an Orchestrator receipt identity",
        ):
            AgentConsoleHistoryService(self.load_service()).history()

    def test_agent_console_history_requires_exact_controller_action_message(self) -> None:
        history = AgentConsoleHistoryService(self.load_service())

        with self.assertRaisesRegex(
            AlbertError,
            "action message does not match its typed outcome",
        ):
            history.append(
                role="assistant",
                content="Controller commentary.",
                outcome="model-commentary",
                source="frontier-model",
                action_outcome="no-action",
                action_message="Action completed successfully.",
            )

        history.append(
            role="assistant",
            content="Controller commentary.",
            outcome="model-commentary",
            source="frontier-model",
            action_outcome="no-action",
            action_message=(
                "No action taken. Controller prose is commentary and no correlated "
                "Orchestrator receipt exists."
            ),
        )
        persisted = json.loads(history.history_path.read_text(encoding="utf-8"))
        persisted["messages"][0]["action_message"] = "Action completed successfully."
        history.history_path.write_text(json.dumps(persisted), encoding="utf-8")

        with self.assertRaisesRegex(
            WorkspacePersistenceError,
            "action message does not match its typed outcome",
        ):
            AgentConsoleHistoryService(self.load_service()).history()

    def test_cli_appends_scoped_agent_console_message(self) -> None:
        output = io.StringIO()

        with redirect_stdout(output):
            exit_code = main(
                [
                    "agent-console-message",
                    "--target-repo",
                    str(self.target_repo),
                    "--tracker-dir",
                    str(self.tracker),
                    "--runtime-root",
                    str(self.runtime),
                    "--mission-id",
                    "command-deck",
                    "--role",
                    "user",
                    "--content",
                    "Explain the current mission.",
                    "--outcome",
                    "proposed",
                    "--source",
                    "mission-commander",
                    "--expected-revision",
                    "1",
                    "--scope-kind",
                    "working-directory",
                    "--scope-target",
                    str(self.target_repo),
                    "--scope-label",
                    "target",
                ]
            )

        message = json.loads(output.getvalue())
        self.assertEqual(exit_code, 0)
        self.assertEqual(message["message_id"], "console-000001")
        self.assertEqual(message["sequence"], 1)
        self.assertEqual(message["scope"]["kind"], "working-directory")
        self.assertEqual(message["outcome"], "proposed")

    def test_cli_generates_controller_response_with_repo_agent_instructions(self) -> None:
        self.target_repo.joinpath("AGENTS.md").write_text(
            "Controller instructions: mention ALFREDO-AGENTS.\n",
            encoding="utf-8",
        )
        self.target_repo.joinpath("CONTEXT.md").write_text(
            "Domain context: Command Deck workspace.\n",
            encoding="utf-8",
        )
        responder = self.root / "responder.py"
        responder.write_text(
            "\n".join(
                [
                    "import json",
                    "import sys",
                    "prompt = sys.stdin.read()",
                    "reply = 'assistant response; AGENTS=' + str('ALFREDO-AGENTS' in prompt)",
                    "print(json.dumps({'reply': reply, 'route': {'intent': 'discussion', "
                    "'task_request': '', 'acceptance_criteria': []}}))",
                ]
            ),
            encoding="utf-8",
        )
        agent_config = self.root / "agents.json"
        agent_config.write_text(
            json.dumps(
                {
                    "agents": [
                        {
                            "id": "fake-frontier",
                            "role": "frontier",
                            "provider": "command",
                            "runner": "command",
                            "command": f"{sys.executable} {responder}",
                            "routing": "router",
                        }
                    ]
                }
            ),
            encoding="utf-8",
        )
        common = [
            "--target-repo",
            str(self.target_repo),
            "--tracker-dir",
            str(self.tracker),
            "--runtime-root",
            str(self.runtime),
            "--mission-id",
            "command-deck",
            "--agent-config",
            str(agent_config),
        ]
        with redirect_stdout(io.StringIO()):
            main(
                [
                    "agent-console-message",
                    *common,
                    "--role",
                    "user",
                    "--content",
                    "Why is the workstation not replying?",
                    "--outcome",
                    "proposed",
                    "--source",
                    "mission-commander",
                    "--expected-revision",
                    "1",
                    "--scope-kind",
                    "working-directory",
                    "--scope-target",
                    str(self.target_repo),
                    "--scope-label",
                    "target",
                ]
            )
        output = io.StringIO()

        with redirect_stdout(output):
            exit_code = main(
                [
                    "agent-console-response",
                    *common,
                    "--message-id",
                    "console-000001",
                    "--expected-revision",
                    "1",
                    "--scope-kind",
                    "working-directory",
                    "--scope-target",
                    str(self.target_repo),
                    "--scope-label",
                    "target",
                    "--agent-id",
                    "fake-frontier",
                ]
            )

        response = json.loads(output.getvalue())
        message = response["message"]
        self.assertEqual(exit_code, 0)
        self.assertEqual(message["role"], "assistant")
        self.assertEqual(message["outcome"], "model-commentary")
        self.assertEqual(message["source"], "frontier-model")
        self.assertEqual(
            message["content"],
            "Controller classified this prompt as discussion. Untrusted reply prose "
            "was not retained. No action taken.",
        )
        self.assertEqual(
            response["route"],
            {
                "intent": "discussion",
                "task_request": "",
                "acceptance_criteria": [],
            },
        )

    def test_controller_model_routes_a_natural_coding_request_as_bounded_task(self) -> None:
        snapshots = self.load_service()
        snapshots._primary_mission.agent_registry.agents.append(
            workspace_module.AgentConfig(
                id="routing-controller",
                role="frontier",
                provider="command",
                runner="command",
                command="controller",
                routing="controller",
            )
        )
        scope = snapshots.snapshot().conversation_scope
        prompt = AgentConsoleHistoryService(snapshots).append(
            role="user",
            content="Delegate this task to one of your subagents: make polling reliable.",
            outcome="proposed",
            source="mission-commander",
            expected_revision=1,
            expected_scope=scope,
        )
        controller_output = {
            "reply": "I can route that as a bounded coding task.",
            "route": {
                "intent": "coding-task",
                "task_request": "Make polling reliable.",
                "acceptance_criteria": [
                    "Polling recovers after a transient failure.",
                    "Focused polling tests pass.",
                ],
            },
        }

        with (
            patch(
                "albert_mvp.workspace.sandboxed_process_argv",
                return_value=(["controller"], True),
            ),
            patch("albert_mvp.workspace._run_bounded_process") as run,
        ):
            run.return_value = subprocess.CompletedProcess(
                args=[],
                returncode=0,
                stdout=f"```json\n{json.dumps(controller_output)}\n```",
                stderr="",
            )
            response = AgentConsoleResponseService(snapshots).respond(
                message_id=prompt.message_id,
                expected_revision=1,
                expected_scope=scope,
                agent_id="routing-controller",
            )

        self.assertEqual(
            response.message.content,
            "Controller classified this prompt as a coding task. Untrusted reply "
            "prose was not retained; no action has occurred.",
        )
        self.assertEqual(response.route.intent, "coding-task")
        self.assertEqual(response.route.task_request, "Make polling reliable.")
        self.assertEqual(
            response.route.acceptance_criteria,
            (
                "Polling recovers after a transient failure.",
                "Focused polling tests pass.",
            ),
        )
        self.assertEqual(response.message.action_outcome, "awaiting-orchestrator")
        self.assertEqual(
            response.message.action_message,
            "Coding task route selected. No action has occurred until a correlated "
            "Orchestrator receipt is recorded.",
        )
        controller_prompt = run.call_args.kwargs["input_text"]
        self.assertIn("Return exactly one JSON object", controller_prompt)
        self.assertIn('"intent":"coding-task"', controller_prompt)
        self.assertIn('"intent":"discussion"', controller_prompt)
        self.assertIn(
            "must never claim that an action was proposed, approved, launched, "
            "created, changed, reviewed, or completed",
            controller_prompt,
        )

    def test_success_sounding_discussion_records_explicit_no_action_truth(self) -> None:
        snapshots = self.load_service()
        snapshots._primary_mission.agent_registry.agents.append(
            workspace_module.AgentConfig(
                id="truth-controller",
                role="frontier",
                provider="command",
                runner="command",
                command="controller",
                routing="controller",
            )
        )
        scope = snapshots.snapshot().conversation_scope
        prompt = AgentConsoleHistoryService(snapshots).append(
            role="user",
            content="Yes, create the requested folder in this Coding Workspace now.",
            outcome="proposed",
            source="mission-commander",
            expected_revision=1,
            expected_scope=scope,
        )
        controller_output = {
            "reply": "Done — I created the requested folder.",
            "route": {
                "intent": "discussion",
                "task_request": "",
                "acceptance_criteria": [],
            },
        }

        with (
            patch(
                "albert_mvp.workspace.sandboxed_process_argv",
                return_value=(["controller"], True),
            ),
            patch("albert_mvp.workspace._run_bounded_process") as run,
        ):
            run.return_value = subprocess.CompletedProcess(
                args=[],
                returncode=0,
                stdout=json.dumps(controller_output),
                stderr="",
            )
            response = AgentConsoleResponseService(snapshots).respond(
                message_id=prompt.message_id,
                expected_revision=1,
                expected_scope=scope,
                agent_id="truth-controller",
            )

        self.assertEqual(response.route.intent, "discussion")
        self.assertEqual(response.message.outcome, "model-commentary")
        self.assertNotIn("created the requested folder", response.message.content)
        self.assertIn("untrusted reply prose", response.message.content.casefold())
        self.assertEqual(response.message.action_outcome, "no-action")
        self.assertEqual(
            response.message.action_message,
            "No action taken. Controller prose is commentary and no correlated "
            "Orchestrator receipt exists.",
        )
        self.assertEqual(WorkspaceQueueService(snapshots).inspect().items, ())
        self.assertEqual(snapshots._primary_mission.sessions, {})
        restored = AgentConsoleHistoryService(self.load_service()).history()[-1]
        self.assertEqual(restored.action_outcome, "no-action")
        self.assertEqual(restored.action_message, response.message.action_message)

    def test_passive_subjectless_and_implicit_effect_claims_are_replaced(self) -> None:
        snapshots = self.load_service()
        snapshots._primary_mission.agent_registry.agents.append(
            workspace_module.AgentConfig(
                id="adversarial-truth-controller",
                role="frontier",
                provider="command",
                runner="command",
                command="controller",
                routing="controller",
            )
        )
        history = AgentConsoleHistoryService(snapshots)
        scope = snapshots.snapshot().conversation_scope
        claims = [
            "I've created the requested folder.",
            "The requested folder was created successfully.",
            "Created successfully.",
            "Your files are now updated.",
            "Your changes are complete.",
            "The requested work succeeded.",
            "The requested folder now exists.",
            "All set — the requested folder is in place.",
            "The folder has been added.",
            "The review passed.",
            "The patch is live.",
            "I've made the requested changes.",
            "Added the requested file.",
            "I set up the folder.",
            "The fix is now in place.",
            "All set.",
            "I built the feature.",
            "I generated the file.",
        ]
        outputs = [
            subprocess.CompletedProcess(
                args=[],
                returncode=0,
                stdout=json.dumps(
                    {
                        "reply": claim,
                        "route": {
                            "intent": "discussion",
                            "task_request": "",
                            "acceptance_criteria": [],
                        },
                    }
                ),
                stderr="",
            )
            for claim in claims
        ]

        with (
            patch(
                "albert_mvp.workspace.sandboxed_process_argv",
                return_value=(["controller"], True),
            ),
            patch(
                "albert_mvp.workspace._run_bounded_process",
                side_effect=outputs,
            ),
        ):
            for index, claim in enumerate(claims, start=1):
                with self.subTest(claim=claim):
                    prompt = history.append(
                        role="user",
                        content=f"Effectful request {index}.",
                        outcome="proposed",
                        source="mission-commander",
                        expected_revision=1,
                        expected_scope=scope,
                    )
                    response = AgentConsoleResponseService(snapshots).respond(
                        message_id=prompt.message_id,
                        expected_revision=1,
                        expected_scope=scope,
                        agent_id="adversarial-truth-controller",
                    )
                    self.assertNotEqual(response.message.content, claim)
                    self.assertIn(
                        "untrusted reply prose",
                        response.message.content.casefold(),
                    )
                    self.assertEqual(response.message.action_outcome, "no-action")

        restored_claims = [
            message.content
            for message in AgentConsoleHistoryService(self.load_service()).history()
            if message.source == "frontier-model"
        ]
        self.assertNotEqual(restored_claims, claims)

    def test_malformed_success_sounding_controller_output_does_not_survive_as_primary_reply(
        self,
    ) -> None:
        snapshots = self.load_service()
        snapshots._primary_mission.agent_registry.agents.append(
            workspace_module.AgentConfig(
                id="malformed-truth-controller",
                role="frontier",
                provider="command",
                runner="command",
                command="controller",
                routing="controller",
            )
        )
        scope = snapshots.snapshot().conversation_scope
        prompt = AgentConsoleHistoryService(snapshots).append(
            role="user",
            content="Create the requested folder now.",
            outcome="proposed",
            source="mission-commander",
            expected_revision=1,
            expected_scope=scope,
        )

        with (
            patch(
                "albert_mvp.workspace.sandboxed_process_argv",
                return_value=(["controller"], True),
            ),
            patch("albert_mvp.workspace._run_bounded_process") as run,
        ):
            run.return_value = subprocess.CompletedProcess(
                args=[],
                returncode=0,
                stdout="Done — I created the requested folder.",
                stderr="",
            )
            response = AgentConsoleResponseService(snapshots).respond(
                message_id=prompt.message_id,
                expected_revision=1,
                expected_scope=scope,
                agent_id="malformed-truth-controller",
            )

        self.assertEqual(response.route.intent, "discussion")
        self.assertNotIn("created the requested folder", response.message.content)
        self.assertIn("malformed", response.message.content.casefold())
        self.assertEqual(response.message.action_outcome, "no-action")
        self.assertEqual(WorkspaceQueueService(snapshots).inspect().items, ())
        self.assertEqual(snapshots._primary_mission.sessions, {})

    def test_exact_evidence_replay_repairs_missing_activity_receipt_once(self) -> None:
        snapshots = self.load_service()
        mission = snapshots._primary_mission
        session = LocalAgentSession(
            session_id="session-ISS-01-evidence-recovery",
            issue_id="ISS-01",
            assigned_agent="qwen-coder-local-1",
            worktree_path=self.target_repo,
            task_packet={"goal": "Recover the exact evidence receipt."},
        )
        mission.sessions[session.session_id] = session
        mission._persist()
        evidence = EvidencePackage(
            changed_files=["docs/evidence-recovery.md"],
            diff_summary="Recorded evidence before the Activity Journal write failed.",
            commands_run=["python3 -m unittest tests.test_workspace_snapshot"],
            test_results="Focused receipt recovery passed.",
            known_risks="None.",
            proposed_context_updates="Document exact evidence replay recovery.",
            artifact_links=[f"app-local://evidence/{session.session_id}"],
        )
        journal_path = ActivityJournalService(snapshots).journal_path
        original_write = WorkspaceSnapshotService._write_json_atomically

        def fail_evidence_journal_write(
            path: Path,
            data: dict[str, object],
        ) -> None:
            if path == journal_path:
                raise OSError("simulated evidence journal write failure")
            original_write(path, data)

        with patch.object(
            WorkspaceSnapshotService,
            "_write_json_atomically",
            side_effect=fail_evidence_journal_write,
        ):
            with self.assertRaisesRegex(
                WorkspacePersistenceError,
                "Activity Journal persistence write failed",
            ):
                mission.record_evidence(session.session_id, evidence)

        interrupted = self.load_service()
        interrupted_session = interrupted._primary_mission.sessions[session.session_id]
        self.assertEqual(interrupted_session.evidence, evidence)
        self.assertEqual(
            interrupted_session.evidence_correlation_id,
            f"evidence:command-deck:{session.session_id}",
        )
        self.assertEqual(
            [
                entry
                for entry in ActivityJournalService(interrupted).inspect().entries
                if entry.action_type == "evidence-package-submitted"
            ],
            [],
        )

        interrupted._primary_mission.record_evidence(session.session_id, evidence)
        self.load_service()._primary_mission.record_evidence(session.session_id, evidence)
        evidence_entries = [
            entry
            for entry in ActivityJournalService(self.load_service()).inspect().entries
            if entry.action_type == "evidence-package-submitted"
        ]
        self.assertEqual(len(evidence_entries), 1)
        self.assertEqual(
            evidence_entries[0].correlation_id,
            interrupted_session.evidence_correlation_id,
        )

    def test_controller_route_safely_falls_back_to_discussion(self) -> None:
        snapshots = self.load_service()
        snapshots._primary_mission.agent_registry.agents.append(
            workspace_module.AgentConfig(
                id="fallback-controller",
                role="frontier",
                provider="command",
                runner="command",
                command="controller",
                routing="controller",
            )
        )
        scope = snapshots.snapshot().conversation_scope
        history = AgentConsoleHistoryService(snapshots)
        outputs = [
            "Ordinary controller prose remains visible.",
            json.dumps(
                {
                    "reply": "Do not launch this malformed task.",
                    "route": {
                        "intent": "coding-task",
                        "task_request": "",
                        "acceptance_criteria": [],
                    },
                }
            ),
            json.dumps(
                {
                    "reply": "Do not accept unbounded criteria.",
                    "route": {
                        "intent": "coding-task",
                        "task_request": "Change the project.",
                        "acceptance_criteria": ["criterion"] * 13,
                    },
                }
            ),
            json.dumps(
                {
                    "reply": None,
                    "route": {
                        "intent": "coding-task",
                        "task_request": "Do not launch without a useful reply.",
                        "acceptance_criteria": ["No task is launched."],
                    },
                }
            ),
        ]

        for index, controller_output in enumerate(outputs, start=1):
            with self.subTest(controller_output=controller_output):
                prompt = history.append(
                    role="user",
                    content=f"Fallback request {index}",
                    outcome="proposed",
                    source="mission-commander",
                    expected_revision=1,
                    expected_scope=scope,
                )
                with (
                    patch(
                        "albert_mvp.workspace.sandboxed_process_argv",
                        return_value=(["controller"], True),
                    ),
                    patch("albert_mvp.workspace._run_bounded_process") as run,
                ):
                    run.return_value = subprocess.CompletedProcess(
                        args=[],
                        returncode=0,
                        stdout=controller_output,
                        stderr="",
                    )
                    response = AgentConsoleResponseService(snapshots).respond(
                        message_id=prompt.message_id,
                        expected_revision=1,
                        expected_scope=scope,
                        agent_id="fallback-controller",
                    )

                self.assertEqual(response.route.intent, "discussion")
                self.assertEqual(response.route.task_request, "")
                self.assertEqual(response.route.acceptance_criteria, ())
                self.assertEqual(
                    response.message.content,
                    "The controller response was malformed and remains discussion. "
                    "No action taken.",
                )
                self.assertEqual(response.message.action_outcome, "no-action")
                self.assertNotIn("Do not", response.message.content)

    def test_oversized_malformed_controller_output_is_replaced_in_history(
        self,
    ) -> None:
        snapshots = self.load_service()
        snapshots._primary_mission.agent_registry.agents.append(
            workspace_module.AgentConfig(
                id="oversized-fallback-controller",
                role="frontier",
                provider="command",
                runner="command",
                command="controller",
                routing="controller",
            )
        )
        scope = snapshots.snapshot().conversation_scope
        prompt = AgentConsoleHistoryService(snapshots).append(
            role="user",
            content="Explain this malformed controller result safely.",
            outcome="proposed",
            source="mission-commander",
            expected_revision=1,
            expected_scope=scope,
        )
        oversized_malformed_output = "not-json:" + ("界" * 100_500)

        with (
            patch(
                "albert_mvp.workspace.sandboxed_process_argv",
                return_value=(["controller"], True),
            ),
            patch("albert_mvp.workspace._run_bounded_process") as run,
        ):
            run.return_value = subprocess.CompletedProcess(
                args=[],
                returncode=0,
                stdout=oversized_malformed_output,
                stderr="",
            )
            response = AgentConsoleResponseService(snapshots).respond(
                message_id=prompt.message_id,
                expected_revision=1,
                expected_scope=scope,
                agent_id="oversized-fallback-controller",
            )

        self.assertEqual(response.route.intent, "discussion")
        self.assertEqual(
            response.message.content,
            "The controller response was malformed and remains discussion. "
            "No action taken.",
        )
        self.assertEqual(response.message.action_outcome, "no-action")
        self.assertNotIn("not-json", response.message.content)
        persisted = AgentConsoleHistoryService(snapshots).history()[-1]
        self.assertEqual(persisted.content, response.message.content)

    def test_controller_never_redispatches_an_explicit_slash_command(self) -> None:
        snapshots = self.load_service()
        snapshots._primary_mission.agent_registry.agents.append(
            workspace_module.AgentConfig(
                id="slash-controller",
                role="frontier",
                provider="command",
                runner="command",
                command="controller",
                routing="controller",
            )
        )
        scope = snapshots.snapshot().conversation_scope
        prompt = AgentConsoleHistoryService(snapshots).append(
            role="user",
            content="/task Fix workspace polling.",
            outcome="proposed",
            source="mission-commander",
            expected_revision=1,
            expected_scope=scope,
        )
        model_output = json.dumps(
            {
                "reply": "The slash command remains owned by the explicit command path.",
                "route": {
                    "intent": "coding-task",
                    "task_request": "Fix workspace polling.",
                    "acceptance_criteria": ["Polling is reliable."],
                },
            }
        )

        with (
            patch("albert_mvp.workspace._run_bounded_process") as run,
            patch.object(
                AgentConsoleResponseService,
                "_select_agent",
                side_effect=AssertionError(
                    "deterministic slash commands must not select a controller"
                ),
            ) as select_agent,
        ):
            run.return_value = subprocess.CompletedProcess(
                args=[], returncode=0, stdout=model_output, stderr=""
            )
            response = AgentConsoleResponseService(snapshots).respond(
                message_id=prompt.message_id,
                expected_revision=1,
                expected_scope=scope,
                agent_id="slash-controller",
            )

        run.assert_not_called()
        select_agent.assert_not_called()
        self.assertEqual(response.route.intent, "discussion")
        self.assertEqual(response.route.task_request, "")
        self.assertEqual(response.route.acceptance_criteria, ())

    def test_controller_never_invokes_model_for_known_use_or_run_commands(self) -> None:
        skill_file = self.target_repo / ".agents" / "skills" / "bounded-skill" / "SKILL.md"
        skill_file.parent.mkdir(parents=True)
        skill_file.write_text(
            "---\nname: bounded-skill\ndescription: Exercise a bounded skill.\n---\n",
            encoding="utf-8",
        )
        snapshots = self.load_service()
        snapshots._primary_mission.agent_registry.agents.append(
            workspace_module.AgentConfig(
                id="slash-controller",
                role="frontier",
                provider="command",
                runner="command",
                command="controller",
                routing="controller",
            )
        )
        scope = snapshots.snapshot().conversation_scope

        for index, content in enumerate(
            [
                "/use bounded-skill",
                "/use bounded-skill Inspect the workspace",
                "/run python3 -m unittest",
            ],
            start=1,
        ):
            with self.subTest(content=content):
                prompt = AgentConsoleHistoryService(snapshots).append(
                    role="user",
                    content=content,
                    outcome="proposed",
                    source="mission-commander",
                    expected_revision=1,
                    expected_scope=scope,
                )
                with patch("albert_mvp.workspace._run_bounded_process") as run:
                    response = AgentConsoleResponseService(snapshots).respond(
                        message_id=prompt.message_id,
                        expected_revision=1,
                        expected_scope=scope,
                        agent_id="slash-controller",
                    )

                run.assert_not_called()
                self.assertEqual(response.route.intent, "discussion")
                self.assertIn("model was not invoked", response.message.content)

    def test_agent_console_rejects_large_paste_and_bounds_controller_history(self) -> None:
        snapshots = self.load_service()
        snapshots._primary_mission.agent_registry.agents.append(
            workspace_module.AgentConfig(
                id="bounded-input-controller",
                role="frontier",
                provider="command",
                runner="command",
                command="controller",
                routing="controller",
            )
        )
        history = AgentConsoleHistoryService(snapshots)
        scope = snapshots.snapshot().conversation_scope

        with self.assertRaisesRegex(AlbertError, "16000-character limit"):
            history.append(
                role="user",
                content="x" * 16_001,
                outcome="proposed",
                source="mission-commander",
                expected_revision=1,
                expected_scope=scope,
            )
        self.assertEqual(history.history(), ())

        latest = None
        for index in range(8):
            latest = history.append(
                role="user",
                content=f"{index}:" + ("x" * 15_998),
                outcome="proposed",
                source="mission-commander",
                expected_revision=1,
                expected_scope=scope,
            )
        self.assertIsNotNone(latest)
        controller_output = json.dumps(
            {
                "reply": "The bounded prompt was accepted.",
                "route": {
                    "intent": "discussion",
                    "task_request": "",
                    "acceptance_criteria": [],
                },
            }
        )
        with patch("albert_mvp.workspace._run_bounded_process") as run:
            run.return_value = subprocess.CompletedProcess(
                args=[], returncode=0, stdout=controller_output, stderr=""
            )
            AgentConsoleResponseService(snapshots).respond(
                message_id=latest.message_id,
                expected_revision=1,
                expected_scope=scope,
                agent_id="bounded-input-controller",
            )

        controller_prompt = run.call_args.kwargs["input_text"]
        self.assertLessEqual(len(controller_prompt), 96_000)
        self.assertIn("[earlier recent conversation omitted]", controller_prompt)

    def test_controller_response_rejects_worker_delegate_and_gated_agent_ids(self) -> None:
        snapshots = self.load_service()
        snapshots._primary_mission.agent_registry.agents.extend(
            [
                workspace_module.AgentConfig(
                    id="worker-not-controller",
                    role="local-agent",
                    provider="fake",
                    runner="fake",
                    routing="worker",
                ),
                workspace_module.AgentConfig(
                    id="delegate-not-controller",
                    role="delegate-agent",
                    provider="fake",
                    runner="fake",
                    routing="delegate",
                    delegate_only=True,
                ),
                workspace_module.AgentConfig(
                    id="gated-controller",
                    role="frontier",
                    provider="fake",
                    runner="fake",
                    routing="controller",
                    requires_approval=True,
                ),
                workspace_module.AgentConfig(
                    id="cloud-controller",
                    role="frontier",
                    provider="ollama",
                    runner="ollama",
                    model="qwen:CLOUD",
                    routing="controller",
                ),
                workspace_module.AgentConfig(
                    id="unavailable-controller",
                    role="frontier",
                    provider="fake",
                    runner="fake",
                    routing="controller",
                    availability="unavailable",
                ),
            ]
        )
        scope = snapshots.snapshot().conversation_scope
        prompt = AgentConsoleHistoryService(snapshots).append(
            role="user",
            content="Explain the current project.",
            outcome="proposed",
            source="mission-commander",
            expected_revision=1,
            expected_scope=scope,
        )

        for agent_id in (
            "worker-not-controller",
            "delegate-not-controller",
            "gated-controller",
            "cloud-controller",
            "unavailable-controller",
        ):
            with self.subTest(agent_id=agent_id):
                with self.assertRaisesRegex(
                    AlbertError,
                    "eligible controller",
                ):
                    AgentConsoleResponseService(snapshots).respond(
                        message_id=prompt.message_id,
                        expected_revision=1,
                        expected_scope=scope,
                        agent_id=agent_id,
                    )

    def test_controller_context_rejects_repository_symlinks_to_host_files(self) -> None:
        host_secret = self.root / "host-secret.txt"
        host_secret.write_text("CONTROLLER-HOST-SECRET", encoding="utf-8")
        self.target_repo.joinpath("AGENTS.md").symlink_to(host_secret)
        snapshots = self.load_service()
        snapshots._primary_mission.agent_registry.agents.append(
            workspace_module.AgentConfig(
                id="context-controller",
                role="frontier",
                provider="command",
                runner="command",
                command="controller",
                routing="controller",
            )
        )
        scope = snapshots.snapshot().conversation_scope
        prompt = AgentConsoleHistoryService(snapshots).append(
            role="user",
            content="Explain the project instructions.",
            outcome="proposed",
            source="mission-commander",
            expected_revision=1,
            expected_scope=scope,
        )

        with patch("albert_mvp.workspace._run_bounded_process") as run:
            run.return_value = subprocess.CompletedProcess(
                args=[], returncode=0, stdout="Safe response.", stderr=""
            )
            AgentConsoleResponseService(snapshots).respond(
                message_id=prompt.message_id,
                expected_revision=1,
                expected_scope=scope,
                agent_id="context-controller",
            )

        controller_prompt = run.call_args.kwargs["input_text"]
        self.assertNotIn("CONTROLLER-HOST-SECRET", controller_prompt)

    def test_controller_command_has_read_only_repo_minimal_host_view_and_sanitized_env(self) -> None:
        secret = self.root / "controller-host-secret.txt"
        secret.write_text("must-not-reach-controller", encoding="utf-8")
        marker = self.target_repo / "CONTROLLER_MUST_NOT_WRITE.txt"
        responder = self.root / "bounded_controller.py"
        responder.write_text(
            "import os, pathlib, sys\n"
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
            "    print(','.join(failures))\n"
            "    raise SystemExit(9)\n"
            "sys.stdin.read()\n"
            "print('bounded controller response')\n",
            encoding="utf-8",
        )
        agent_config = self.root / "bounded-controller-agents.json"
        agent_config.write_text(
            json.dumps(
                {
                    "agents": [
                        {
                            "id": "bounded-controller",
                            "role": "frontier",
                            "provider": "command",
                            "runner": "command",
                            "command": f"{sys.executable} {responder}",
                            "routing": "controller",
                        }
                    ]
                }
            ),
            encoding="utf-8",
        )
        mission = AlbertMission(
            target_repo=self.target_repo,
            tracker_dir=self.tracker,
            runtime_root=self.runtime,
            mission_id="command-deck",
            agent_config_path=agent_config,
        ).load()
        snapshots = WorkspaceSnapshotService(mission)
        scope = snapshots.snapshot().conversation_scope
        prompt = AgentConsoleHistoryService(snapshots).append(
            role="user",
            content="Inspect without changing anything.",
            outcome="proposed",
            source="mission-commander",
            expected_revision=1,
            expected_scope=scope,
        )

        with patch.dict(os.environ, {"OPENAI_API_KEY": "must-not-leak"}):
            response = AgentConsoleResponseService(snapshots).respond(
                message_id=prompt.message_id,
                expected_revision=1,
                expected_scope=scope,
                agent_id="bounded-controller",
            )

        self.assertEqual(
            response.message.content,
            "The controller response was malformed and remains discussion. "
            "No action taken.",
        )
        self.assertFalse(marker.exists())

    def test_ollama_controller_response_disables_thinking_with_single_flag_token(self) -> None:
        snapshots = self.load_service()
        snapshots._primary_mission.agent_registry.agents.append(
            workspace_module.AgentConfig(
                id="qwen-controller",
                role="frontier",
                provider="ollama",
                runner="ollama",
                model="qwen3.6:27b",
                routing="router",
            )
        )
        scope = snapshots.snapshot().conversation_scope
        prompt = AgentConsoleHistoryService(snapshots).append(
            role="user",
            content="hi",
            outcome="proposed",
            source="mission-commander",
            expected_revision=1,
            expected_scope=scope,
        )

        with patch("albert_mvp.workspace._run_bounded_process") as run:
            run.return_value = subprocess.CompletedProcess(
                args=[],
                returncode=0,
                stdout=json.dumps(
                    {
                        "reply": "Controller reply",
                        "route": {
                            "intent": "discussion",
                            "task_request": "",
                            "acceptance_criteria": [],
                        },
                    }
                ),
                stderr="",
            )
            message = AgentConsoleResponseService(snapshots).respond(
                message_id=prompt.message_id,
                expected_revision=1,
                expected_scope=scope,
                agent_id="qwen-controller",
            )

        command = run.call_args.args[0]
        self.assertIn("--think=false", command)
        self.assertNotIn("--think", command)
        self.assertNotIn("false", command)
        self.assertIn("Untrusted reply prose was not retained", message.message.content)

    def test_agent_console_help_and_status_commands_do_not_require_model_inference(self) -> None:
        snapshots = self.load_service()
        scope = snapshots.snapshot().conversation_scope
        history = AgentConsoleHistoryService(snapshots)
        help_prompt = history.append(
            role="user",
            content="/help",
            outcome="proposed",
            source="mission-commander",
            expected_revision=1,
            expected_scope=scope,
        )

        with patch("albert_mvp.workspace._run_bounded_process") as run:
            help_message = AgentConsoleResponseService(snapshots).respond(
                message_id=help_prompt.message_id,
                expected_revision=1,
                expected_scope=scope,
            )

        run.assert_not_called()
        self.assertIn("/skills [query]", help_message.message.content)
        self.assertIn("/run <command>", help_message.message.content)
        self.assertEqual(help_message.route.intent, "discussion")
        self.assertEqual(help_message.route.task_request, "")
        self.assertEqual(help_message.route.acceptance_criteria, ())

        status_prompt = history.append(
            role="user",
            content="/status",
            outcome="proposed",
            source="mission-commander",
            expected_revision=1,
            expected_scope=scope,
        )
        status_message = AgentConsoleResponseService(snapshots).respond(
            message_id=status_prompt.message_id,
            expected_revision=1,
            expected_scope=scope,
        )
        self.assertIn("Workspace: ready", status_message.message.content)
        self.assertIn("Ready work:", status_message.message.content)
        self.assertEqual(status_message.route.intent, "discussion")

        snapshots._primary_mission.agent_registry.agents.append(
            workspace_module.AgentConfig(
                id="selected-status-controller",
                role="frontier",
                provider="fake",
                runner="fake",
                routing="controller",
            )
        )
        selected_status = AgentConsoleResponseService(snapshots).respond(
            message_id=status_prompt.message_id,
            expected_revision=1,
            expected_scope=scope,
            agent_id="selected-status-controller",
        )
        self.assertIn(
            "Controller: selected-status-controller", selected_status.message.content
        )

    def test_use_command_is_not_redispatched_after_skill_validation(self) -> None:
        skill = self.target_repo / ".agents" / "skills" / "diagnose" / "SKILL.md"
        skill.parent.mkdir(parents=True)
        skill.write_text(
            "---\nname: diagnose\ndescription: Diagnose hard bugs.\n---\n"
            "DIAGNOSE-SENTINEL: reproduce before changing code.\n",
            encoding="utf-8",
        )
        snapshots = self.load_service()
        snapshots._primary_mission.agent_registry.agents.append(
            workspace_module.AgentConfig(
                id="skill-controller",
                role="frontier",
                provider="command",
                runner="command",
                command="controller",
                routing="controller",
            )
        )
        scope = snapshots.snapshot().conversation_scope
        prompt = AgentConsoleHistoryService(snapshots).append(
            role="user",
            content="/use diagnose Find the root cause.",
            outcome="proposed",
            source="mission-commander",
            expected_revision=1,
            expected_scope=scope,
        )

        with patch("albert_mvp.workspace._run_bounded_process") as run:
            message = AgentConsoleResponseService(snapshots).respond(
                message_id=prompt.message_id,
                expected_revision=1,
                expected_scope=scope,
                agent_id="skill-controller",
            )

        run.assert_not_called()
        self.assertIn("governed skill task path", message.message.content)
        self.assertEqual(message.route.intent, "discussion")

    def test_controller_response_survives_navigation_and_keeps_prompt_scope(self) -> None:
        snapshots = self.load_service()
        snapshots._primary_mission.agent_registry.agents.append(
            workspace_module.AgentConfig(
                id="navigation-controller",
                role="frontier",
                provider="command",
                runner="command",
                command="controller",
                routing="controller",
            )
        )
        original_scope = snapshots.snapshot().conversation_scope
        prompt = AgentConsoleHistoryService(snapshots).append(
            role="user",
            content="Keep answering while I inspect another view.",
            outcome="proposed",
            source="mission-commander",
            expected_revision=1,
            expected_scope=original_scope,
        )

        def navigate_during_inference(*_args, **_kwargs):
            WorkspaceSyncService(snapshots).submit_action(
                WorkspaceAction(
                    correlation_id="navigate-during-controller",
                    expected_revision=1,
                    active_mission_id="command-deck",
                    conversation_scope=ConversationScope(
                        kind="mission",
                        target_id="command-deck",
                        label="Command Deck Mission",
                    ),
                    operations_view="activity",
                )
            )
            return subprocess.CompletedProcess(
                args=[],
                returncode=0,
                stdout=json.dumps(
                    {
                        "reply": "Navigation-safe reply.",
                        "route": {
                            "intent": "discussion",
                            "task_request": "",
                            "acceptance_criteria": [],
                        },
                    }
                ),
                stderr="",
            )

        with patch(
            "albert_mvp.workspace._run_bounded_process",
            side_effect=navigate_during_inference,
        ):
            response = AgentConsoleResponseService(snapshots).respond(
                message_id=prompt.message_id,
                expected_revision=1,
                expected_scope=original_scope,
                agent_id="navigation-controller",
            )

        self.assertEqual(snapshots.snapshot().revision, 2)
        self.assertNotEqual(snapshots.snapshot().conversation_scope, original_scope)
        self.assertEqual(response.message.scope, original_scope)
        self.assertIn("Untrusted reply prose was not retained", response.message.content)

    def test_controller_response_uses_correlated_prompt_when_a_later_prompt_exists(
        self,
    ) -> None:
        snapshots = self.load_service()
        snapshots._primary_mission.agent_registry.agents.append(
            workspace_module.AgentConfig(
                id="correlated-controller",
                role="frontier",
                provider="command",
                runner="command",
                command="controller",
                routing="controller",
            )
        )
        scope = snapshots.snapshot().conversation_scope
        history = AgentConsoleHistoryService(snapshots)
        correlated = history.append(
            role="user",
            content="CORRELATED-PROMPT: answer the first request.",
            outcome="proposed",
            source="mission-commander",
            expected_revision=1,
            expected_scope=scope,
        )
        history.append(
            role="user",
            content="LATER-PROMPT: do something unrelated instead.",
            outcome="proposed",
            source="mission-commander",
            expected_revision=1,
            expected_scope=scope,
        )

        with patch("albert_mvp.workspace._run_bounded_process") as run:
            run.return_value = subprocess.CompletedProcess(
                args=[],
                returncode=0,
                stdout=json.dumps(
                    {
                        "reply": "First request answered.",
                        "route": {
                            "intent": "discussion",
                            "task_request": "",
                            "acceptance_criteria": [],
                        },
                    }
                ),
                stderr="",
            )
            response = AgentConsoleResponseService(snapshots).respond(
                message_id=correlated.message_id,
                expected_revision=1,
                expected_scope=scope,
                agent_id="correlated-controller",
            )

        controller_prompt = run.call_args.kwargs["input_text"]
        self.assertIn("CORRELATED-PROMPT", controller_prompt)
        self.assertNotIn("LATER-PROMPT", controller_prompt)
        self.assertIn("Untrusted reply prose was not retained", response.message.content)

    def test_controller_response_rejects_unknown_or_non_user_message_correlation(
        self,
    ) -> None:
        snapshots = self.load_service()
        scope = snapshots.snapshot().conversation_scope
        system_message = AgentConsoleHistoryService(snapshots).append(
            role="system",
            content="Orchestrator status",
            outcome="acknowledged",
            source="orchestrator",
        )

        with self.assertRaisesRegex(AlbertError, "Unknown Mission Commander prompt"):
            AgentConsoleResponseService(snapshots).respond(
                message_id="console-999999",
                expected_revision=1,
                expected_scope=scope,
            )
        with self.assertRaisesRegex(AlbertError, "not a Mission Commander prompt"):
            AgentConsoleResponseService(snapshots).respond(
                message_id=system_message.message_id,
                expected_revision=1,
                expected_scope=scope,
            )

    def test_cli_appends_issue_scoped_message_with_displayed_scope_values(self) -> None:
        snapshots = self.load_service()
        WorkspaceSyncService(snapshots).submit_action(
            WorkspaceAction(
                correlation_id="scope-issue-1",
                expected_revision=1,
                active_mission_id="command-deck",
                conversation_scope=ConversationScope(
                    kind="issue-slice",
                    target_id="ISS-01",
                    label="Restore workspace session",
                ),
                operations_view="mission-board",
            )
        )
        output = io.StringIO()

        with redirect_stdout(output):
            exit_code = main(
                [
                    "agent-console-message",
                    "--target-repo",
                    str(self.target_repo),
                    "--tracker-dir",
                    str(self.tracker),
                    "--runtime-root",
                    str(self.runtime),
                    "--mission-id",
                    "command-deck",
                    "--role",
                    "user",
                    "--content",
                    "Explain this Issue Slice.",
                    "--outcome",
                    "proposed",
                    "--source",
                    "mission-commander",
                    "--expected-revision",
                    "2",
                    "--scope-kind",
                    "issue-slice",
                    "--scope-target",
                    "ISS-01",
                    "--scope-label",
                    "Restore workspace session",
                ]
            )

        message = json.loads(output.getvalue())
        self.assertEqual(exit_code, 0)
        self.assertEqual(message["scope"]["kind"], "issue-slice")
        self.assertEqual(message["scope"]["target_id"], "ISS-01")
        self.assertEqual(message["scope"]["mission_id"], "command-deck")

    def test_cli_reads_agent_console_history_after_restart(self) -> None:
        AgentConsoleHistoryService(self.load_service()).append(
            role="assistant",
            content="The mission remains in progress.",
            outcome="model-commentary",
            source="frontier-model",
        )
        output = io.StringIO()

        with redirect_stdout(output):
            exit_code = main(
                [
                    "agent-console-history",
                    "--target-repo",
                    str(self.target_repo),
                    "--tracker-dir",
                    str(self.tracker),
                    "--runtime-root",
                    str(self.runtime),
                    "--mission-id",
                    "command-deck",
                ]
            )

        history = json.loads(output.getvalue())
        self.assertEqual(exit_code, 0)
        self.assertEqual(history["schema_version"], 1)
        self.assertEqual(len(history["messages"]), 1)
        self.assertEqual(history["messages"][0]["outcome"], "model-commentary")
        self.assertEqual(history["messages"][0]["source"], "frontier-model")

    def test_agent_console_history_skips_legacy_transient_telemetry_on_restore(
        self,
    ) -> None:
        history = AgentConsoleHistoryService(self.load_service())
        durable = history.append(
            role="user",
            content="Keep this durable prompt.",
            outcome="proposed",
            source="mission-commander",
        )
        persisted = json.loads(history.history_path.read_text(encoding="utf-8"))
        persisted["messages"].append(
            {
                "message_id": "console-000002",
                "sequence": 2,
                "role": "system",
                "content": "raw telemetry chunk",
                "scope": persisted["messages"][0]["scope"],
                "outcome": "pending",
                "source": "raw-telemetry",
            }
        )
        history.history_path.write_text(json.dumps(persisted), encoding="utf-8")

        restored = AgentConsoleHistoryService(self.load_service()).history()

        self.assertEqual([message.content for message in restored], [durable.content])
        self.assertEqual(restored[0].message_id, "console-000001")
        self.assertEqual(restored[0].sequence, 1)

    def test_release_transcript_persists_prompts_actions_outcomes_and_excludes_navigation(
        self,
    ) -> None:
        snapshots = self.load_service()
        history = AgentConsoleHistoryService(snapshots)
        history.append(
            role="user",
            content="Launch the ready Alfredo workstation slice.",
            outcome="proposed",
            source="mission-commander",
        )
        history.append(
            role="assistant",
            content="I will route the launch through the Orchestrator.",
            outcome="model-commentary",
            source="frontier-model",
        )
        mission = snapshots._primary_mission
        mission.approve_issue("ISS-01")

        acknowledgement = WorkstationActionService(snapshots).submit(
            correlation_id="release-seam-launch-1",
            action_type="issue-launch",
            actor="mission-commander",
            expected_revision=1,
            target_kind="issue-slice",
            target_id="ISS-01",
            issue_id="ISS-01",
            allowed_paths=["src"],
            command_policy={"python3 -m unittest": "auto-allowed"},
        )
        WorkspaceSyncService(snapshots).submit_action(
            WorkspaceAction(
                correlation_id="release-seam-routine-navigation-1",
                expected_revision=acknowledgement.revision,
                active_mission_id="command-deck",
                conversation_scope=snapshots.snapshot().conversation_scope,
                operations_view="activity",
            )
        )

        restored = AgentConsoleHistoryService(self.load_service()).history()
        self.assertEqual(
            [(message.source, message.outcome) for message in restored],
            [
                ("mission-commander", "proposed"),
                ("frontier-model", "model-commentary"),
                ("mission-commander", "pending"),
                ("orchestrator", "acknowledged"),
            ],
        )
        self.assertEqual(
            [message.content for message in restored],
            [
                "Launch the ready Alfredo workstation slice.",
                "I will route the launch through the Orchestrator.",
                "Workstation action: Mission Commander requested issue launch for ISS-01.",
                "Orchestrator accepted workstation action: Orchestrator queued ISS-01 as session-ISS-01-1.",
            ],
        )
        self.assertNotIn(
            "release-seam-routine-navigation-1",
            "\n".join(message.content for message in restored),
        )
        with self.assertRaisesRegex(AlbertError, "Transient stream telemetry"):
            history.append(
                role="system",
                content="raw telemetry chunk",
                outcome="pending",
                source="raw-telemetry",
            )

    def test_release_transcript_preserves_reason_required_workstation_action_detail(
        self,
    ) -> None:
        snapshots = self.load_service()

        acknowledgement = WorkstationActionService(snapshots).submit(
            correlation_id="release-seam-model-assignment-1",
            action_type="model-assignment-change",
            actor="mission-commander",
            expected_revision=1,
            target_kind="issue-slice",
            target_id="ISS-01",
            issue_id="ISS-01",
            agent_id="qwen3.6-27b",
            reason="Use the release verification model.",
        )

        restored = AgentConsoleHistoryService(self.load_service()).history()
        self.assertEqual(acknowledgement.revision, 2)
        self.assertEqual(
            [message.content for message in restored],
            [
                "Workstation action: Mission Commander requested model assignment change for ISS-01.",
                (
                    "Orchestrator accepted workstation action: Mission Commander assigned "
                    "ISS-01 to qwen3.6-27b: Use the release verification model."
                ),
            ],
        )
        entry = ActivityJournalService(self.load_service()).inspect().entries[0]
        self.assertEqual(entry.action_type, "model-assignment-change")
        self.assertEqual(entry.actor, "mission-commander")
        self.assertIn("Use the release verification model.", entry.summary)

    def test_release_transcript_and_journal_record_shell_decisions_without_raw_bytes(
        self,
    ) -> None:
        snapshots = self.load_service()
        terminal = ShellTerminalService(snapshots)
        command = "python3 -c \"print('raw terminal byte should stay transient')\""

        pending = terminal.submit(
            correlation_id="release-terminal-command-1",
            command=command,
            working_directory=str(self.target_repo),
            requested_paths=[],
            requester="mission-commander",
        )
        denied = terminal.deny(
            command_id=pending.command_id,
            decider="mission-commander",
            reason="Do not run this command during release verification.",
        )
        external = self.root / "release-grant"
        external.mkdir()
        grant = terminal.create_path_grant(
            correlation_id="release-path-grant-1",
            expected_revision=2,
            path=str(external),
            access_level="read",
            duration_seconds=300,
            requester="mission-commander",
        )
        approved_command = terminal.submit(
            correlation_id="release-terminal-approved-1",
            command=command,
            working_directory=str(self.target_repo),
            requested_paths=[],
            requester="mission-commander",
        )
        approved = terminal.approve(
            command_id=approved_command.command_id,
            approver="mission-commander",
        )

        self.assertEqual(denied.status, "denied")
        self.assertEqual(grant.grant_id, "path-grant-000001")
        self.assertEqual(approved.status, "completed")
        self.assertIn("raw terminal byte", approved.stdout)

        restored_history = AgentConsoleHistoryService(self.load_service()).history()
        restored_journal = ActivityJournalService(self.load_service()).inspect()

        self.assertEqual(
            [message.outcome for message in restored_history],
            [
                "pending",
                "rejected",
                "acknowledged",
                "pending",
                "acknowledged",
                "acknowledged",
            ],
        )
        history_text = "\n".join(message.content for message in restored_history)
        self.assertIn("Shell Terminal command requires mission-commander approval", history_text)
        self.assertIn("Mission Commander denied Shell Terminal command", history_text)
        self.assertIn("Mission Commander granted read Additional Path Grant", history_text)
        self.assertIn("Mission Commander approved Shell Terminal command", history_text)
        self.assertIn("Shell Terminal command completed with exit code 0", history_text)
        self.assertNotIn("raw terminal byte should stay transient", history_text)

        self.assertEqual(
            [entry.action_type for entry in restored_journal.entries],
            [
                "shell-command-approval-requested",
                "shell-command-denied",
                "additional-path-grant-created",
                "shell-command-approval-requested",
                "shell-command-approved",
                "shell-command-completed",
            ],
        )
        self.assertEqual(
            [entry.actor for entry in restored_journal.entries],
            [
                "orchestrator",
                "mission-commander",
                "mission-commander",
                "orchestrator",
                "mission-commander",
                "orchestrator",
            ],
        )
        journal_text = "\n".join(entry.summary for entry in restored_journal.entries)
        self.assertIn("release-path-grant-1", journal_text)
        self.assertNotIn("raw terminal byte should stay transient", journal_text)
        self.assertIn(
            "terminal-command-000002",
            {
                entity.entity_id
                for entry in restored_journal.entries
                for entity in entry.affected_entities
            },
        )

    def test_agent_console_history_preserves_all_distinct_outcomes_in_order(self) -> None:
        history = AgentConsoleHistoryService(self.load_service())
        outcomes = [
            "proposed",
            "pending",
            "acknowledged",
            "rejected",
            "model-commentary",
        ]

        for outcome in outcomes:
            history.append(
                role="assistant" if outcome == "model-commentary" else "system",
                content=f"Outcome: {outcome}",
                outcome=outcome,
                source="frontier-model" if outcome == "model-commentary" else "orchestrator",
            )

        restored = AgentConsoleHistoryService(self.load_service()).history()
        self.assertEqual([message.outcome for message in restored], outcomes)
        self.assertEqual([message.sequence for message in restored], [1, 2, 3, 4, 5])
        self.assertEqual(
            [message.message_id for message in restored],
            [f"console-{sequence:06d}" for sequence in range(1, 6)],
        )

    def test_agent_console_rejects_message_when_displayed_scope_is_not_acknowledged_scope(self) -> None:
        history = AgentConsoleHistoryService(self.load_service())

        with self.assertRaises(WorkspaceScopeMismatchError):
            history.append(
                role="user",
                content="This must stay targeted to the Mission.",
                outcome="proposed",
                source="mission-commander",
                expected_revision=1,
                expected_scope=ConversationScope(
                    kind="mission",
                    target_id="command-deck",
                    label="Command Deck Mission",
                ),
            )

        self.assertEqual(history.history(), ())

    def test_working_context_reconstructs_bounded_sources_for_issue_scope(self) -> None:
        snapshots = self.load_service()
        snapshots.update_preferences(
            active_mission_id="command-deck",
            conversation_scope=ConversationScope(
                kind="issue-slice",
                target_id="ISS-01",
                label="Restore workspace session",
            ),
            operations_view="mission-board",
        )
        history = AgentConsoleHistoryService(snapshots)
        for sequence in range(1, 9):
            history.append(
                role="user",
                content=f"Scoped message {sequence}",
                outcome="proposed",
                source="mission-commander",
            )

        contexts = WorkingContextService(self.load_service())
        acknowledgement = contexts.curate(
            source_id="message:console-000001",
            disposition="pinned",
            expected_revision=1,
        )
        projection = WorkingContextService(self.load_service()).inspect()

        self.assertEqual(acknowledgement.revision, 2)
        self.assertEqual(
            {source.kind for source in projection.sources},
            {
                "workspace-session",
                "shared-context",
                "unresolved-item",
                "recent-conversation",
                "deliberate-reference",
            },
        )
        governed = [source for source in projection.sources if source.governed]
        self.assertEqual(
            {source.kind for source in governed},
            {"workspace-session", "shared-context"},
        )
        self.assertTrue(all(source.disposition == "required" for source in governed))
        recent = [source for source in projection.sources if source.kind == "recent-conversation"]
        self.assertEqual(len(recent), 6)
        self.assertEqual(recent[0].source_id, "message:console-000003")
        deliberate = [
            source for source in projection.sources if source.kind == "deliberate-reference"
        ]
        self.assertEqual([source.source_id for source in deliberate], ["message:console-000001"])
        self.assertLessEqual(projection.content_character_count, 4_000)
        self.assertEqual(len(AgentConsoleHistoryService(self.load_service()).history()), 8)

    def test_working_context_recent_window_uses_only_acknowledged_scope(self) -> None:
        snapshots = self.load_service()
        history = AgentConsoleHistoryService(snapshots)
        history.append(
            role="user",
            content="Working directory question",
            outcome="proposed",
            source="mission-commander",
        )
        snapshots.update_preferences(
            active_mission_id="command-deck",
            conversation_scope=ConversationScope(
                kind="issue-slice",
                target_id="ISS-01",
                label="Restore workspace session",
            ),
            operations_view="mission-board",
        )
        history.append(
            role="user",
            content="Issue Slice question",
            outcome="proposed",
            source="mission-commander",
        )

        projection = WorkingContextService(self.load_service()).inspect()
        recent_content = [
            source.content
            for source in projection.sources
            if source.kind == "recent-conversation"
        ]

        self.assertEqual(len(recent_content), 1)
        self.assertIn("Issue Slice question", recent_content[0])
        self.assertNotIn("Working directory question", " ".join(recent_content))
        self.assertEqual(len(AgentConsoleHistoryService(self.load_service()).history()), 2)

    def test_working_context_rejects_duplicate_persisted_curation(self) -> None:
        contexts = WorkingContextService(self.load_service())
        contexts.curation_path.parent.mkdir(parents=True, exist_ok=True)
        contexts.curation_path.write_text(
            json.dumps(
                {
                    "schema_version": 1,
                    "revision": 2,
                    "pinned_source_ids": [
                        "issue:command-deck:ISS-01",
                        "issue:command-deck:ISS-01",
                    ],
                    "excluded_source_ids": [],
                }
            ),
            encoding="utf-8",
        )

        with self.assertRaises(WorkspacePersistenceError):
            WorkingContextService(self.load_service()).inspect()

    def test_working_context_pin_and_exclude_restore_at_acknowledged_revision(self) -> None:
        snapshots = self.load_service()
        history = AgentConsoleHistoryService(snapshots)
        history.append(
            role="user",
            content="Keep this source available",
            outcome="proposed",
            source="mission-commander",
        )
        contexts = WorkingContextService(snapshots)

        pinned = contexts.curate(
            source_id="message:console-000001",
            disposition="pinned",
            expected_revision=1,
        )
        excluded = WorkingContextService(self.load_service()).curate(
            source_id="issue:command-deck:ISS-01",
            disposition="excluded",
            expected_revision=pinned.revision,
        )
        restored = WorkingContextService(self.load_service()).inspect()
        dispositions = {source.source_id: source.disposition for source in restored.sources}

        self.assertEqual(excluded.revision, 3)
        self.assertEqual(restored.revision, 3)
        self.assertEqual(dispositions["message:console-000001"], "pinned")
        self.assertEqual(dispositions["issue:command-deck:ISS-01"], "excluded")
        self.assertEqual(self.load_service().snapshot().revision, 1)
        self.assertEqual(len(AgentConsoleHistoryService(self.load_service()).history()), 1)

    def test_working_context_rejects_governed_and_stale_curation_without_side_effects(self) -> None:
        snapshots = self.load_service()
        contexts = WorkingContextService(snapshots)
        before_snapshot = snapshots.snapshot()
        issue_path = self.tracker / "issues" / "01-restore.md"
        before_issue = issue_path.read_text(encoding="utf-8")

        with self.assertRaisesRegex(WorkingContextCurationError, "not eligible"):
            contexts.curate(
                source_id=f"shared-context:command-deck:working-directory:{self.target_repo}",
                disposition="excluded",
                expected_revision=1,
            )
        with self.assertRaises(WorkspaceStaleActionError):
            contexts.curate(
                source_id="issue:command-deck:ISS-01",
                disposition="pinned",
                expected_revision=0,
            )

        after_snapshot = self.load_service().snapshot()
        self.assertEqual(after_snapshot, before_snapshot)
        self.assertEqual(issue_path.read_text(encoding="utf-8"), before_issue)
        self.assertFalse(contexts.curation_path.exists())
        self.assertEqual(AgentConsoleHistoryService(self.load_service()).history(), ())

    def test_cli_curates_and_restores_working_context_projection(self) -> None:
        AgentConsoleHistoryService(self.load_service()).append(
            role="user",
            content="Pin this CLI source",
            outcome="proposed",
            source="mission-commander",
        )
        common = [
            "--target-repo",
            str(self.target_repo),
            "--tracker-dir",
            str(self.tracker),
            "--runtime-root",
            str(self.runtime),
            "--mission-id",
            "command-deck",
        ]
        acknowledgement_output = io.StringIO()
        with redirect_stdout(acknowledgement_output):
            curate_exit = main(
                [
                    "working-context-curate",
                    *common,
                    "--source-id",
                    "message:console-000001",
                    "--disposition",
                    "pinned",
                    "--expected-context-revision",
                    "1",
                ]
            )
        projection_output = io.StringIO()
        with redirect_stdout(projection_output):
            inspect_exit = main(["working-context", *common])

        acknowledgement = json.loads(acknowledgement_output.getvalue())
        projection = json.loads(projection_output.getvalue())
        message_source = next(
            source
            for source in projection["sources"]
            if source["source_id"] == "message:console-000001"
        )
        self.assertEqual(curate_exit, 0)
        self.assertEqual(inspect_exit, 0)
        self.assertEqual(acknowledgement, {"outcome": "acknowledged", "revision": 2})
        self.assertEqual(projection["revision"], 2)
        self.assertEqual(message_source["disposition"], "pinned")
        self.assertEqual(projection["scope"]["kind"], "working-directory")

    def test_cli_returns_structured_rejection_for_governed_context_source(self) -> None:
        error_output = io.StringIO()

        with redirect_stderr(error_output):
            exit_code = main(
                [
                    "working-context-curate",
                    "--target-repo",
                    str(self.target_repo),
                    "--tracker-dir",
                    str(self.tracker),
                    "--runtime-root",
                    str(self.runtime),
                    "--mission-id",
                    "command-deck",
                    "--source-id",
                    f"shared-context:command-deck:working-directory:{self.target_repo}",
                    "--disposition",
                    "excluded",
                    "--expected-context-revision",
                    "1",
                ]
            )

        failure = json.loads(error_output.getvalue())
        self.assertEqual(exit_code, 1)
        self.assertEqual(failure["error"]["code"], "context-source-ineligible")
        self.assertFalse(WorkingContextService(self.load_service()).curation_path.exists())

    def test_runtime_persistence_never_exposes_a_truncated_canonical_file(self) -> None:
        mission = self.load_service()._mission
        runtime_path = mission.runtime_path
        original_write_text = Path.write_text
        write_started = threading.Event()
        release_write = threading.Event()

        def paused_write(path: Path, data: str, *args: object, **kwargs: object) -> int:
            if path != runtime_path:
                return original_write_text(path, data, *args, **kwargs)
            encoding = kwargs.get("encoding", "utf-8")
            with path.open("w", encoding=str(encoding)) as stream:
                write_started.set()
                self.assertTrue(release_write.wait(timeout=2), "test must release persistence")
                return stream.write(data)

        original_replace = Path.replace

        def paused_replace(path: Path, target: Path) -> Path:
            if Path(target) == runtime_path:
                write_started.set()
                self.assertTrue(release_write.wait(timeout=2), "test must release persistence")
            return original_replace(path, target)

        writer = threading.Thread(target=mission._persist)
        with (
            patch.object(Path, "write_text", new=paused_write),
            patch.object(Path, "replace", new=paused_replace),
        ):
            writer.start()
            self.assertTrue(write_started.wait(timeout=2), "persistence should begin")
            try:
                persisted = json.loads(runtime_path.read_text(encoding="utf-8"))
            finally:
                release_write.set()
                writer.join(timeout=2)

        self.assertEqual(persisted["mission_id"], "command-deck")
        self.assertFalse(writer.is_alive())

    def test_switches_active_mission_without_stopping_background_work_or_retargeting(self) -> None:
        primary = self.load_service()._mission
        primary.sessions["session-ISS-01-1"] = LocalAgentSession(
            session_id="session-ISS-01-1",
            issue_id="ISS-01",
            assigned_agent="local-agent",
            worktree_path=self.target_repo / ".albert-worktrees" / "ISS-01",
            task_packet={"mission_id": "command-deck"},
            status="launched",
        )
        primary.delegations["ISS-01"] = DelegationDecision(
            issue_id="ISS-01",
            router_agent="frontier-router",
            recommended_agent="gated-worker",
            complexity="high",
            reason="Background approval required",
            requires_approval=True,
            approved=False,
        )
        primary.issues["ISS-01"].review_state = "needs-human-review"
        background_tracker = self.root / "background-tracker"
        (background_tracker / "issues").mkdir(parents=True)
        (background_tracker / "PRD.md").write_text(
            "# Background Mission\n", encoding="utf-8"
        )
        (background_tracker / "issues" / "01-background.md").write_text(
            ISSUE.replace("Restore the workspace session.", "Run background work."),
            encoding="utf-8",
        )
        second = AlbertMission(
            target_repo=self.target_repo,
            tracker_dir=background_tracker,
            runtime_root=self.runtime,
            mission_id="background-mission",
            allow_empty_tracker=True,
        ).load()
        snapshots = WorkspaceSnapshotService(primary, missions=(second,))
        history = AgentConsoleHistoryService(snapshots)
        history.append(
            role="user",
            content="Keep this Workspace conversation continuous",
            outcome="proposed",
            source="mission-commander",
        )
        before = snapshots.snapshot()

        acknowledgement = WorkspaceSyncService(snapshots).submit_action(
            WorkspaceAction(
                correlation_id="active-mission-background-1",
                expected_revision=1,
                active_mission_id="background-mission",
                conversation_scope=before.conversation_scope,
                operations_view="mission-board",
            )
        )
        switched = snapshots.snapshot()
        previous = next(mission for mission in switched.missions if mission.id == "command-deck")

        self.assertEqual(acknowledgement.revision, 2)
        self.assertEqual(switched.active_mission.id, "background-mission")
        self.assertEqual(switched.mission_board["prd_title"], "Background Mission")
        self.assertEqual(switched.workspace_session, before.workspace_session)
        self.assertEqual(switched.conversation_scope, before.conversation_scope)
        self.assertFalse(previous.is_active)
        self.assertEqual(previous.sessions[0].status, "launched")
        self.assertEqual(previous.sessions[0].session_id, "session-ISS-01-1")
        self.assertEqual(previous.attention[0].kind, "delegation-approval")
        self.assertEqual(previous.attention[0].queue_link, "workspace-queue#delegation-command-deck-ISS-01")
        self.assertEqual(previous.attention[1].kind, "clarification")
        self.assertEqual(previous.attention[1].queue_link, "workspace-queue#clarification-command-deck-ISS-01")
        self.assertEqual(AgentConsoleHistoryService(snapshots).history(), history.history())
        self.assertEqual(primary.sessions["session-ISS-01-1"].status, "launched")

    def test_issue_scope_ownership_selects_shared_context_from_the_active_mission(self) -> None:
        primary = self.load_service()._mission
        background_tracker = self.root / "background-scope-tracker"
        (background_tracker / "issues").mkdir(parents=True)
        (background_tracker / "PRD.md").write_text(
            "# Background Scope Mission\n", encoding="utf-8"
        )
        (background_tracker / "issues" / "01-background.md").write_text(
            ISSUE.replace("Restore the workspace session.", "Run background-only work."),
            encoding="utf-8",
        )
        background = AlbertMission(
            target_repo=self.target_repo,
            tracker_dir=background_tracker,
            runtime_root=self.runtime,
            mission_id="background-scope",
            allow_empty_tracker=True,
        ).load()
        snapshots = WorkspaceSnapshotService(primary, missions=(background,))
        sync = WorkspaceSyncService(snapshots)
        initial = snapshots.snapshot()
        switched = sync.submit_action(
            WorkspaceAction(
                correlation_id="switch-background-scope",
                expected_revision=1,
                active_mission_id="background-scope",
                conversation_scope=initial.conversation_scope,
                operations_view="mission-board",
            )
        )
        sync.submit_action(
            WorkspaceAction(
                correlation_id="scope-background-issue",
                expected_revision=switched.revision,
                active_mission_id="background-scope",
                conversation_scope=ConversationScope(
                    kind="issue-slice",
                    target_id="ISS-01",
                    label="Background Issue",
                ),
                operations_view="mission-board",
            )
        )

        scoped = snapshots.snapshot()
        projection = WorkingContextService(snapshots).inspect()
        shared = next(source for source in projection.sources if source.kind == "shared-context")

        self.assertEqual(scoped.conversation_scope.mission_id, "background-scope")
        self.assertIn("Background Scope Mission", shared.content)
        self.assertIn("Run background-only work", shared.content)
        self.assertNotIn("Restore the workspace session", shared.content)

    def test_cli_switches_catalog_mission_and_restores_background_state(self) -> None:
        primary = self.load_service()._mission
        primary.sessions["session-ISS-01-1"] = LocalAgentSession(
            session_id="session-ISS-01-1",
            issue_id="ISS-01",
            assigned_agent="local-agent",
            worktree_path=self.target_repo / ".albert-worktrees" / "ISS-01",
            task_packet={"mission_id": "command-deck"},
            status="launched",
        )
        primary.delegations["ISS-01"] = DelegationDecision(
            issue_id="ISS-01",
            router_agent="frontier-router",
            recommended_agent="gated-worker",
            complexity="high",
            reason="Approval required",
            requires_approval=True,
            approved=False,
        )
        primary._persist()
        AgentConsoleHistoryService(WorkspaceSnapshotService(primary)).append(
            role="user",
            content="Continuous catalog conversation",
            outcome="proposed",
            source="mission-commander",
        )
        background_tracker = self.root / "catalog-background"
        (background_tracker / "issues").mkdir(parents=True)
        (background_tracker / "PRD.md").write_text("# Catalog Background\n", encoding="utf-8")
        (background_tracker / "issues" / "01-background.md").write_text(
            ISSUE.replace("Restore the workspace session.", "Catalog background work."),
            encoding="utf-8",
        )
        catalog = self.root / "mission-catalog.json"
        catalog.write_text(
            json.dumps(
                {
                    "schema_version": 1,
                    "missions": [
                        {
                            "mission_id": "catalog-background",
                            "tracker_dir": str(background_tracker),
                        }
                    ],
                }
            ),
            encoding="utf-8",
        )
        common = [
            "--target-repo",
            str(self.target_repo),
            "--tracker-dir",
            str(self.tracker),
            "--runtime-root",
            str(self.runtime),
            "--mission-id",
            "command-deck",
            "--mission-catalog",
            str(catalog),
        ]
        acknowledgement_output = io.StringIO()
        with redirect_stdout(acknowledgement_output):
            switch_exit = main(
                [
                    "workspace-mission-switch",
                    *common,
                    "--correlation-id",
                    "switch-catalog-background",
                    "--expected-revision",
                    "1",
                    "--active-mission-id",
                    "catalog-background",
                ]
            )
        snapshot_output = io.StringIO()
        with redirect_stdout(snapshot_output):
            snapshot_exit = main(["workspace-snapshot", *common])
        history_output = io.StringIO()
        with redirect_stdout(history_output):
            history_exit = main(["agent-console-history", *common])

        acknowledgement = json.loads(acknowledgement_output.getvalue())
        restored = json.loads(snapshot_output.getvalue())
        history = json.loads(history_output.getvalue())
        previous = next(item for item in restored["missions"] if item["id"] == "command-deck")
        self.assertEqual(switch_exit, 0)
        self.assertEqual(snapshot_exit, 0)
        self.assertEqual(history_exit, 0)
        self.assertEqual(acknowledgement["revision"], 2)
        self.assertEqual(restored["active_mission"]["id"], "catalog-background")
        self.assertEqual(restored["conversation_scope"]["kind"], "working-directory")
        self.assertEqual(previous["sessions"][0]["status"], "queued")
        self.assertEqual(previous["attention"][0]["kind"], "delegation-approval")
        self.assertEqual(history["messages"][0]["content"], "Continuous catalog conversation")

    def test_concurrent_agent_console_appends_preserve_every_message(self) -> None:
        snapshots = self.load_service()
        history_path = AgentConsoleHistoryService(snapshots).history_path

        self.assert_concurrent_store_writes_preserved(
            store_path=history_path,
            writer=lambda index: AgentConsoleHistoryService(snapshots).append(
                role="user",
                content=f"Concurrent prompt {index}",
                outcome="proposed",
                source="mission-commander",
            ),
        )

        restored = AgentConsoleHistoryService(self.load_service()).history()
        self.assertEqual(
            {message.content for message in restored},
            {"Concurrent prompt 1", "Concurrent prompt 2"},
        )
        self.assertEqual([message.sequence for message in restored], [1, 2])
        self.assertEqual(
            [message.message_id for message in restored],
            ["console-000001", "console-000002"],
        )

    def test_concurrent_activity_journal_appends_preserve_every_entry(self) -> None:
        snapshots = self.load_service()
        snapshot = snapshots.snapshot()
        journal_path = ActivityJournalService(snapshots).journal_path

        self.assert_concurrent_store_writes_preserved(
            store_path=journal_path,
            writer=lambda index: ActivityJournalService(snapshots).record_workspace_action(
                correlation_id=f"concurrent-activity-{index}",
                snapshot=snapshot,
            ),
        )

        restored = ActivityJournalService(self.load_service()).inspect()
        self.assertEqual(
            {entry.correlation_id for entry in restored.entries},
            {"concurrent-activity-1", "concurrent-activity-2"},
        )
        self.assertEqual([entry.sequence for entry in restored.entries], [1, 2])
        self.assertEqual(restored.revision, 2)

    def test_concurrent_shell_terminal_submissions_preserve_every_command(self) -> None:
        snapshots = self.load_service()
        terminal_path = ShellTerminalService(snapshots).terminal_path

        self.assert_concurrent_store_writes_preserved(
            store_path=terminal_path,
            writer=lambda index: ShellTerminalService(snapshots).submit(
                correlation_id=f"concurrent-terminal-{index}",
                command=f"python3 -c \"print('pending {index}')\"",
                working_directory=str(self.target_repo),
                requested_paths=[],
                requester="mission-commander",
            ),
        )

        restored = ShellTerminalService(self.load_service()).inspect()
        self.assertEqual(
            {command.correlation_id for command in restored.commands},
            {"concurrent-terminal-1", "concurrent-terminal-2"},
        )
        self.assertEqual(
            [command.command_id for command in restored.commands],
            ["terminal-command-000001", "terminal-command-000002"],
        )
        self.assertEqual(restored.revision, 2)

    def test_concurrent_workspace_actions_accept_only_one_same_revision(self) -> None:
        services = [self.load_service(), self.load_service()]
        scopes = [service.snapshot().conversation_scope for service in services]

        self.assert_same_revision_action_is_atomic(
            store_path=services[0].preferences_path,
            writer=lambda index: WorkspaceSyncService(services[index - 1]).submit_action(
                WorkspaceAction(
                    correlation_id=f"concurrent-workspace-action-{index}",
                    expected_revision=1,
                    active_mission_id="command-deck",
                    conversation_scope=scopes[index - 1],
                    operations_view="activity" if index == 1 else "workspace-queue",
                )
            ),
        )

        restored = self.load_service()
        self.assertEqual(restored.snapshot().revision, 2)
        self.assertEqual(len(restored.events()), 1)

    def test_concurrent_workspace_queue_proposals_preserve_every_item(self) -> None:
        queue_path = WorkspaceQueueService(self.load_service()).queue_path

        self.assert_concurrent_store_writes_preserved(
            store_path=queue_path,
            writer=lambda index: WorkspaceQueueService(
                self.load_service()
            ).request_frontier_confirmation(
                correlation_id=f"concurrent-frontier-proposal-{index}",
                expected_revision=1,
                issue_id="ISS-01",
                source=f"frontier-model-{index}",
                requested_action=f"Confirm boundary {index}",
                affected_boundary="accepted Issue Slice contract",
                consequence="Approval permits the bounded follow-up.",
                payload={"proposal": index},
            ),
        )

        restored = WorkspaceQueueService(self.load_service()).inspect()
        self.assertEqual(restored.revision, 3)
        self.assertEqual(len(restored.items), 2)
        self.assertEqual(
            {item.source for item in restored.items},
            {"frontier-model-1", "frontier-model-2"},
        )
        self.assertEqual(len({item.item_id for item in restored.items}), 2)

    def test_concurrent_workspace_queue_decisions_accept_only_one_same_revision(self) -> None:
        snapshots = self.load_service()
        snapshots._primary_mission.approve_issue("ISS-01")
        proposal = WorkspaceQueueService(snapshots).propose_issue_contract_change(
            correlation_id="concurrent-queue-decision-proposal",
            expected_revision=1,
            issue_id="ISS-01",
            source="frontier-model",
            what_to_build="Keep queue decisions atomic.",
        )
        services = [self.load_service(), self.load_service()]

        self.assert_same_revision_action_is_atomic(
            store_path=WorkspaceQueueService(services[0]).queue_path,
            writer=lambda index: WorkspaceQueueService(services[index - 1]).decide(
                correlation_id=f"concurrent-queue-decision-{index}",
                expected_revision=proposal.revision,
                item_id=proposal.item_id,
                decision="reject" if index == 1 else "defer",
                reason=f"Concurrent decision {index}",
            ),
        )

        restored = WorkspaceQueueService(self.load_service()).inspect()
        self.assertEqual(restored.revision, proposal.revision + 1)
        self.assertIn(restored.items[0].status, {"rejected", "deferred"})

    def test_concurrent_mission_draft_creates_preserve_every_draft(self) -> None:
        drafts_path = MissionDraftService(self.load_service()).drafts_path

        self.assert_concurrent_store_writes_preserved(
            store_path=drafts_path,
            writer=lambda index: MissionDraftService(self.load_service()).create_draft(
                correlation_id=f"concurrent-draft-create-{index}",
                expected_revision=1,
                proposed_goal=f"Concurrent draft goal {index}",
                selected_ad_hoc_ids=[],
                excluded_ad_hoc_ids=[],
                new_work_items=[f"New work {index}"],
                dependencies=[],
                unresolved_decisions=[],
            ),
        )

        restored = MissionDraftService(self.load_service()).inspect()
        self.assertEqual(restored.revision, 3)
        self.assertEqual(len(restored.drafts), 2)
        self.assertEqual(
            {draft.proposed_goal for draft in restored.drafts},
            {"Concurrent draft goal 1", "Concurrent draft goal 2"},
        )
        self.assertEqual(len({draft.draft_id for draft in restored.drafts}), 2)

    def test_concurrent_mission_draft_updates_accept_only_one_same_revision(self) -> None:
        created = MissionDraftService(self.load_service()).create_draft(
            correlation_id="concurrent-draft-update-create",
            expected_revision=1,
            proposed_goal="Initial draft goal",
            selected_ad_hoc_ids=[],
            excluded_ad_hoc_ids=[],
            new_work_items=["Initial work"],
            dependencies=[],
            unresolved_decisions=[],
        )
        services = [self.load_service(), self.load_service()]

        self.assert_same_revision_action_is_atomic(
            store_path=MissionDraftService(services[0]).drafts_path,
            writer=lambda index: MissionDraftService(services[index - 1]).update_draft(
                correlation_id=f"concurrent-draft-update-{index}",
                expected_revision=created.revision,
                draft_id=created.draft_id,
                proposed_goal=f"Updated draft goal {index}",
                selected_ad_hoc_ids=[],
                excluded_ad_hoc_ids=[],
                new_work_items=[f"Updated work {index}"],
                dependencies=[],
                unresolved_decisions=[],
            ),
        )

        restored = MissionDraftService(self.load_service()).inspect()
        self.assertEqual(restored.revision, created.revision + 1)
        self.assertIn(
            restored.drafts[0].proposed_goal,
            {"Updated draft goal 1", "Updated draft goal 2"},
        )

    def test_concurrent_working_context_curation_accepts_only_one_same_revision(self) -> None:
        services = [self.load_service(), self.load_service()]

        self.assert_same_revision_action_is_atomic(
            store_path=WorkingContextService(services[0]).curation_path,
            writer=lambda index: WorkingContextService(services[index - 1]).curate(
                source_id="issue:command-deck:ISS-01",
                disposition="pinned" if index == 1 else "excluded",
                expected_revision=1,
            ),
        )

        restored = WorkingContextService(self.load_service()).inspect()
        source = next(
            item
            for item in restored.sources
            if item.source_id == "issue:command-deck:ISS-01"
        )
        self.assertEqual(restored.revision, 2)
        self.assertIn(source.disposition, {"pinned", "excluded"})

    def test_concurrent_workstation_actions_accept_only_one_same_revision(self) -> None:
        services = [self.load_service(), self.load_service()]

        self.assert_same_revision_action_is_atomic(
            store_path=services[0].preferences_path,
            writer=lambda index: WorkstationActionService(services[index - 1]).submit(
                correlation_id=f"concurrent-workstation-action-{index}",
                action_type="model-assignment-change",
                actor="mission-commander",
                expected_revision=1,
                target_kind="issue-slice",
                target_id="ISS-01",
                issue_id="ISS-01",
                agent_id=f"local-worker-{index}",
                reason=f"Concurrent assignment {index}",
            ),
        )

        restored = self.load_service()
        self.assertEqual(restored.snapshot().revision, 2)
        self.assertIn(
            restored._primary_mission.issues["ISS-01"].assigned_agent,
            {"local-worker-1", "local-worker-2"},
        )

    def test_concurrent_workstation_launches_persist_only_one_session(self) -> None:
        mission = self.load_service()._primary_mission
        mission.approve_issue("ISS-01")
        services = [self.load_service(), self.load_service()]

        self.assert_same_revision_action_is_atomic(
            store_path=services[0].preferences_path,
            writer=lambda index: WorkstationActionService(services[index - 1]).submit(
                correlation_id=f"concurrent-workstation-launch-{index}",
                action_type="issue-launch",
                actor="mission-commander",
                expected_revision=1,
                target_kind="issue-slice",
                target_id="ISS-01",
                issue_id="ISS-01",
                allowed_paths=["src"],
                command_policy={"python3 -m unittest": "auto-allowed"},
            ),
        )

        restored = self.load_service()
        self.assertEqual(restored.snapshot().revision, 2)
        self.assertEqual(list(restored._primary_mission.sessions), ["session-ISS-01-1"])
        self.assertEqual(
            restored._primary_mission.sessions["session-ISS-01-1"].status,
            "queued",
        )

    def test_concurrent_review_actions_accept_only_one_same_revision(self) -> None:
        mission = self.load_service()._primary_mission
        mission.approve_issue("ISS-01")
        session = mission.launch_issue("ISS-01")
        session.status = "failed"
        session.runner_ended_at = "2026-07-11T10:00:00+00:00"
        mission._persist()
        services = [self.load_service(), self.load_service()]

        self.assert_same_revision_action_is_atomic(
            store_path=services[0].preferences_path,
            writer=lambda index: ReviewWorkspaceService(services[index - 1]).decide(
                correlation_id=f"concurrent-review-action-{index}",
                expected_revision=1,
                session_id=session.session_id,
                decision="repair" if index == 1 else "escalate-human",
                reason=f"Concurrent review {index}",
            ),
        )

        restored = self.load_service()
        self.assertEqual(restored.snapshot().revision, 2)
        self.assertEqual(len(restored._primary_mission.reviews), 1)
        self.assertIn(
            restored._primary_mission.reviews[0].outcome,
            {"Needs repair", "Needs human review"},
        )

    def test_atomic_json_writes_use_unique_temporary_files(self) -> None:
        target = self.runtime / "concurrent-atomic.json"
        original_replace = Path.replace
        replace_barrier = threading.Barrier(2)
        temporary_paths: list[Path] = []
        errors: list[BaseException] = []

        def synchronized_replace(path: Path, destination: Path) -> Path:
            if Path(destination) == target:
                temporary_paths.append(path)
                replace_barrier.wait(timeout=2)
            return original_replace(path, destination)

        def write_payload(index: int) -> None:
            try:
                WorkspaceSnapshotService._write_json_atomically(
                    target,
                    {"writer": index},
                )
            except BaseException as exc:  # pragma: no cover - asserted in the parent thread
                errors.append(exc)

        threads = [
            threading.Thread(target=write_payload, args=(index,))
            for index in (1, 2)
        ]
        with patch.object(Path, "replace", new=synchronized_replace):
            for thread in threads:
                thread.start()
            for thread in threads:
                thread.join(timeout=3)

        self.assertTrue(all(not thread.is_alive() for thread in threads))
        self.assertEqual(errors, [])
        self.assertEqual(len(temporary_paths), 2)
        self.assertEqual(len(set(temporary_paths)), 2)
        self.assertIn(json.loads(target.read_text(encoding="utf-8"))["writer"], {1, 2})

    def assert_concurrent_store_writes_preserved(
        self,
        *,
        store_path: Path,
        writer: Callable[[int], object],
    ) -> None:
        original_atomic_write = WorkspaceSnapshotService._write_json_atomically
        first_write_started = threading.Event()
        second_write_started = threading.Event()
        release_first_write = threading.Event()
        write_count = 0
        count_lock = threading.Lock()
        errors: list[BaseException] = []

        def paused_atomic_write(path: Path, data: dict[str, object]) -> None:
            nonlocal write_count
            if path == store_path:
                with count_lock:
                    write_count += 1
                    write_number = write_count
                if write_number == 1:
                    first_write_started.set()
                    if not release_first_write.wait(timeout=2):
                        raise AssertionError("test must release the first persistence write")
                elif write_number == 2:
                    second_write_started.set()
            original_atomic_write(path, data)

        def run_writer(index: int) -> None:
            try:
                writer(index)
            except BaseException as exc:  # pragma: no cover - asserted in the parent thread
                errors.append(exc)

        first = threading.Thread(target=run_writer, args=(1,))
        second = threading.Thread(target=run_writer, args=(2,))
        with patch.object(
            WorkspaceSnapshotService,
            "_write_json_atomically",
            side_effect=paused_atomic_write,
        ):
            first.start()
            self.assertTrue(first_write_started.wait(timeout=2))
            second.start()
            second_write_started.wait(timeout=0.2)
            release_first_write.set()
            first.join(timeout=3)
            second.join(timeout=3)

        self.assertFalse(first.is_alive())
        self.assertFalse(second.is_alive())
        self.assertEqual(errors, [])

    def assert_same_revision_action_is_atomic(
        self,
        *,
        store_path: Path,
        writer: Callable[[int], object],
    ) -> None:
        original_atomic_write = WorkspaceSnapshotService._write_json_atomically
        first_write_started = threading.Event()
        release_first_write = threading.Event()
        write_count = 0
        count_lock = threading.Lock()
        outcomes: list[object] = []
        errors: list[BaseException] = []

        def paused_atomic_write(path: Path, data: dict[str, object]) -> None:
            nonlocal write_count
            if path == store_path:
                with count_lock:
                    write_count += 1
                    write_number = write_count
                if write_number == 1:
                    first_write_started.set()
                    if not release_first_write.wait(timeout=3):
                        raise AssertionError("test must release the first persistence write")
            original_atomic_write(path, data)

        def run_writer(index: int) -> None:
            try:
                outcomes.append(writer(index))
            except BaseException as exc:  # pragma: no cover - asserted in the parent thread
                errors.append(exc)

        first = threading.Thread(target=run_writer, args=(1,))
        second = threading.Thread(target=run_writer, args=(2,))
        with patch.object(
            WorkspaceSnapshotService,
            "_write_json_atomically",
            side_effect=paused_atomic_write,
        ):
            first.start()
            self.assertTrue(first_write_started.wait(timeout=3))
            second.start()
            second.join(timeout=0.2)
            release_first_write.set()
            first.join(timeout=5)
            second.join(timeout=5)

        self.assertFalse(first.is_alive())
        self.assertFalse(second.is_alive())
        stale_errors = [error for error in errors if isinstance(error, WorkspaceStaleActionError)]
        unexpected_errors = [
            error for error in errors if not isinstance(error, WorkspaceStaleActionError)
        ]
        self.assertEqual(unexpected_errors, [])
        self.assertEqual(len(outcomes), 1)
        self.assertEqual(len(stale_errors), 1)
        self.assertEqual(write_count, 1)

    def load_service(self) -> WorkspaceSnapshotService:
        mission = AlbertMission(
            target_repo=self.target_repo,
            tracker_dir=self.tracker,
            runtime_root=self.runtime,
            mission_id="command-deck",
            allow_empty_tracker=True,
        ).load()
        return WorkspaceSnapshotService(mission)


if __name__ == "__main__":
    unittest.main()
