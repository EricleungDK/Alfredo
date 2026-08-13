from __future__ import annotations

from contextlib import ExitStack
from pathlib import Path
import subprocess
from types import SimpleNamespace
import tempfile
import unittest
from unittest.mock import patch

from albert_mvp.core import AlbertMission, LocalAgentSession
from albert_mvp.execution import ExecutionJournal
from albert_mvp.workspace import ShellTerminalService, WorkspaceSnapshotService


class HostExecutionIntegrationTests(unittest.TestCase):
    def setUp(self) -> None:
        self.tmp = tempfile.TemporaryDirectory()
        self.root = Path(self.tmp.name)
        self.target = self.root / "target"
        self.target.mkdir()
        self.tracker = self.root / "tracker"
        (self.tracker / "issues").mkdir(parents=True)
        (self.tracker / "PRD.md").write_text("# Test Mission\n", encoding="utf-8")
        self.runtime = self.root / "runtime"

    def tearDown(self) -> None:
        self.tmp.cleanup()

    def _mission(self) -> AlbertMission:
        mission = AlbertMission(
            target_repo=self.target,
            tracker_dir=self.tracker,
            runtime_root=self.runtime,
            mission_id="test-mission",
            allow_empty_tracker=True,
        ).load()
        self.mission_runtime_dir = mission.runtime_dir
        return mission

    def _execution_journal(self) -> ExecutionJournal:
        return ExecutionJournal(self.mission_runtime_dir / "execution-receipts.json")

    def _terminal(self) -> ShellTerminalService:
        return ShellTerminalService(WorkspaceSnapshotService(self._mission()))

    def _sandboxed_terminal(self, terminal: ShellTerminalService) -> ExitStack:
        stack = ExitStack()
        stack.enter_context(
            patch(
                "albert_mvp.workspace._trusted_system_executable",
                return_value="/usr/bin/bwrap",
            )
        )
        stack.enter_context(
            patch.object(terminal, "_sandbox_argv", return_value=(["bwrap"], ""))
        )
        return stack

    def test_shell_reconciles_typed_receipt_and_exact_retry_without_provider_call(
        self,
    ) -> None:
        terminal = self._terminal()
        completed = subprocess.CompletedProcess(
            args=["bwrap"],
            returncode=0,
            stdout="transient shell output",
            stderr="",
        )
        with self._sandboxed_terminal(terminal) as stack:
            run = stack.enter_context(
                patch(
                    "albert_mvp.workspace._run_bounded_process", return_value=completed
                )
            )
            first = terminal.submit(
                correlation_id="shell-integration-1",
                command="python3 -m unittest --help",
                working_directory=str(self.target),
                requested_paths=[],
                requester="mission-commander",
            )

        second_terminal = self._terminal()
        with patch("albert_mvp.workspace._run_bounded_process") as replay_run:
            second = second_terminal.submit(
                correlation_id="shell-integration-1",
                command="python3 -m unittest --help",
                working_directory=str(self.target),
                requested_paths=[],
                requester="mission-commander",
            )

        self.assertEqual(first.status, "completed")
        self.assertEqual(second.status, "completed")
        self.assertEqual(first.stdout, "transient shell output")
        self.assertEqual(second.stdout, "")
        run.assert_called_once()
        replay_run.assert_not_called()
        receipt = self._execution_journal().inspect()[0]
        self.assertEqual(receipt.effect, "shell")
        self.assertEqual(receipt.status, "completed")
        self.assertEqual(receipt.stdout, "")
        self.assertEqual(receipt.stdout_bytes, len("transient shell output".encode()))

    def test_shell_provider_failure_becomes_outcome_unknown_without_retry(self) -> None:
        terminal = self._terminal()

        def effect_then_failure(
            _argv: list[str], **kwargs: object
        ) -> subprocess.CompletedProcess[str]:
            callback = kwargs["process_binding_started"]
            assert callable(callback)
            callback(SimpleNamespace(pid=999999), "shell-process-token")
            raise OSError("provider lost contact after launch")

        with self._sandboxed_terminal(terminal) as stack:
            run = stack.enter_context(
                patch(
                    "albert_mvp.workspace._run_bounded_process",
                    side_effect=effect_then_failure,
                )
            )
            with self.assertRaisesRegex(OSError, "provider lost contact"):
                terminal.submit(
                    correlation_id="shell-integration-crash-1",
                    command="python3 -m unittest --help",
                    working_directory=str(self.target),
                    requested_paths=[],
                    requester="mission-commander",
                )

        persisted = (
            ShellTerminalService(WorkspaceSnapshotService(self._mission()))
            .inspect()
            .commands[0]
        )
        self.assertEqual(persisted.status, "outcome-unknown")
        with patch("albert_mvp.workspace._run_bounded_process") as replay_run:
            replayed = ShellTerminalService(
                WorkspaceSnapshotService(self._mission())
            ).submit(
                correlation_id="shell-integration-crash-1",
                command="python3 -m unittest --help",
                working_directory=str(self.target),
                requested_paths=[],
                requester="mission-commander",
            )
        self.assertEqual(replayed.status, "outcome-unknown")
        run.assert_called_once()
        replay_run.assert_not_called()

    def test_local_agent_uses_the_same_provider_with_local_authority(self) -> None:
        mission = self._mission()
        session = LocalAgentSession(
            session_id="session-local-execution-1",
            issue_id="ISS-01",
            assigned_agent="local-test",
            worktree_path=self.target,
            task_packet={"allowed_paths": ["src"]},
            status="running",
            runner_operation_id="runner:test-mission:session-local-execution-1:1",
            worktree_identity="managed:test-mission:session-local-execution-1",
        )
        completed = subprocess.CompletedProcess(
            args=["bwrap"],
            returncode=0,
            stdout="transient local output",
            stderr="",
        )
        with (
            patch(
                "albert_mvp.core.sandboxed_process_argv",
                return_value=(["bwrap"], True),
            ),
            patch(
                "albert_mvp.core._run_bounded_process",
                return_value=completed,
            ) as run,
        ):
            result = mission._run_cancellable_process(
                session,
                ["python3", "-c", "pass"],
                effect_label="integration-local-command",
            )

        self.assertEqual(result.returncode, 0)
        self.assertEqual(result.stdout, "transient local output")
        run.assert_called_once()
        receipt = self._execution_journal().inspect()[0]
        self.assertEqual(receipt.effect, "local-agent")
        self.assertEqual(receipt.status, "completed")
        self.assertEqual(receipt.stdout, "")


if __name__ == "__main__":
    unittest.main()
