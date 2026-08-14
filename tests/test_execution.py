from __future__ import annotations

import json
import os
from pathlib import Path
from types import SimpleNamespace
import subprocess
import tempfile
import unittest

from albert_mvp.execution import (
    ExecutionCoordinator,
    ExecutionJournal,
    ExecutionLimits,
    ExecutionRequest,
    ExecutionSandbox,
    ExecutionReceipt,
    ExecutionReplayConflict,
    LocalAgentExecutionAuthority,
    PythonExecutionProvider,
    ShellExecutionAuthority,
)


class ExecutionContractTests(unittest.TestCase):
    def setUp(self) -> None:
        self.tmp = tempfile.TemporaryDirectory()
        self.root = Path(self.tmp.name)
        self.runtime = self.root / "runtime"
        self.worktree = self.root / "worktree"
        self.worktree.mkdir()

    def tearDown(self) -> None:
        self.tmp.cleanup()

    def _request(
        self,
        *,
        request_id: str = "shell:command-1",
        command: str = "python3 -c pass",
        authority: object | None = None,
        effect: str = "shell",
    ) -> ExecutionRequest:
        if authority is None:
            authority = ShellExecutionAuthority(
                mission_id="mission-1",
                command_id="command-1",
                correlation_id="command-correlation-1",
                command=command,
                classification="auto-allowed",
                requester="mission-commander",
                working_directory=str(self.worktree.resolve()),
                requested_paths=(),
                access_level="read",
                approval_actor="",
            )
        worktree = str(self.worktree.resolve())
        return ExecutionRequest(
            request_id=request_id,
            effect=effect,
            argv=(
                "/usr/bin/bwrap",
                "--die-with-parent",
                "--new-session",
                "--chdir",
                worktree,
                "--",
                "python3",
                "-c",
                "pass",
            ),
            working_directory=worktree,
            authority=authority,
            limits=ExecutionLimits(timeout_seconds=2, output_limit_bytes=1024),
            sandbox=ExecutionSandbox(
                mode="bubblewrap",
                readable_roots=(str(self.worktree.resolve()),),
                writable_roots=(str(self.worktree.resolve()),),
            ),
            environment=(("PATH", "/usr/bin:/bin"),),
        )

    def test_request_keeps_shell_and_local_agent_authority_inputs_distinct(
        self,
    ) -> None:
        shell = self._request()
        local = self._request(
            request_id="local-agent:session-1:operation-1:runner",
            effect="local-agent",
            authority=LocalAgentExecutionAuthority(
                mission_id="mission-1",
                session_id="session-1",
                session_revision=7,
                runner_operation_id="operation-1",
                worktree_identity="git:/repo/.git/worktrees/session-1",
                allowed_paths=("src",),
            ),
        )

        self.assertEqual(shell.effect, "shell")
        self.assertEqual(local.effect, "local-agent")
        self.assertNotEqual(shell.request_digest, local.request_digest)
        self.assertEqual(shell.to_dict()["authority"]["kind"], "shell")
        self.assertEqual(local.to_dict()["authority"]["kind"], "local-agent")

    def test_request_rejects_shell_execution_and_authority_mismatch(self) -> None:
        request = self._request()
        with self.assertRaisesRegex(ValueError, "shell execution is not allowed"):
            request.with_updates(shell=True)

        with self.assertRaisesRegex(ValueError, "prepared Bubblewrap"):
            PythonExecutionProvider(
                executor=lambda *_args, **_kwargs: subprocess.CompletedProcess(
                    [], 0, "", ""
                )
            ).execute(request.with_updates(argv=("python3", "-c", "pass")))

        with self.assertRaisesRegex(ValueError, "authority kind"):
            self._request(
                effect="local-agent",
                authority=ShellExecutionAuthority(
                    mission_id="mission-1",
                    command_id="command-1",
                    correlation_id="command-correlation-1",
                    command="python3 -c pass",
                    classification="auto-allowed",
                    requester="mission-commander",
                    working_directory=str(self.worktree.resolve()),
                    requested_paths=(),
                    access_level="read",
                    approval_actor="",
                ),
            )

    def test_exact_replay_returns_the_typed_receipt_without_rerunning_effect(
        self,
    ) -> None:
        request = self._request()
        journal = ExecutionJournal(self.runtime / "execution-receipts.json")
        calls: list[tuple[str, ...]] = []

        def executor(
            argv: tuple[str, ...], **_kwargs: object
        ) -> subprocess.CompletedProcess[str]:
            calls.append(tuple(argv))
            return subprocess.CompletedProcess(argv, 0, "first output", "")

        coordinator = ExecutionCoordinator(
            journal,
            PythonExecutionProvider(executor=executor),
        )
        first = coordinator.execute(request)
        second = coordinator.execute(request)

        self.assertEqual(first.status, "completed")
        self.assertEqual(first.stdout, "first output")
        self.assertEqual(second.status, "completed")
        self.assertEqual(second.stdout, "")
        self.assertEqual(first.request_digest, second.request_digest)
        self.assertEqual(len(calls), 1)
        persisted = journal.inspect()[0]
        self.assertEqual(persisted.status, "completed")
        self.assertEqual(persisted.stdout, "")
        self.assertNotIn(
            "first output", (self.runtime / "execution-receipts.json").read_text()
        )

    def test_changed_boundary_replay_is_rejected_before_provider_call(self) -> None:
        request = self._request()
        journal = ExecutionJournal(self.runtime / "execution-receipts.json")
        calls = 0

        def executor(
            argv: tuple[str, ...], **_kwargs: object
        ) -> subprocess.CompletedProcess[str]:
            nonlocal calls
            calls += 1
            return subprocess.CompletedProcess(argv, 0, "", "")

        coordinator = ExecutionCoordinator(
            journal,
            PythonExecutionProvider(executor=executor),
        )
        coordinator.execute(request)
        changed = self._request(command="python3 -c 'print(\"changed\")'")

        with self.assertRaises(ExecutionReplayConflict):
            coordinator.execute(changed)
        self.assertEqual(calls, 1)

    def test_effect_after_provider_crash_is_retained_as_uncertain_and_never_replayed(
        self,
    ) -> None:
        request = self._request(request_id="shell:crash-1")
        journal = ExecutionJournal(self.runtime / "execution-receipts.json")
        calls = 0

        def crashing_executor(
            argv: tuple[str, ...], **kwargs: object
        ) -> subprocess.CompletedProcess[str]:
            nonlocal calls
            calls += 1
            callback = kwargs["process_binding_started"]
            assert callable(callback)
            callback(SimpleNamespace(pid=12345), "test-process-token")
            raise OSError("provider connection lost after launch")

        coordinator = ExecutionCoordinator(
            journal,
            PythonExecutionProvider(executor=crashing_executor),
        )
        first = coordinator.execute(request)
        second = coordinator.execute(request)

        self.assertEqual(first.status, "outcome-unknown")
        self.assertTrue(first.reconciliation_required)
        self.assertEqual(second.status, "outcome-unknown")
        self.assertTrue(second.reconciliation_required)
        self.assertEqual(calls, 1)
        self.assertIn("provider connection lost", first.error_message)

    def test_post_start_file_not_found_is_uncertain(self) -> None:
        request = self._request(request_id="shell:post-start-not-found")

        def effect_then_file_not_found(
            _argv: tuple[str, ...], **kwargs: object
        ) -> subprocess.CompletedProcess[str]:
            callback = kwargs["process_binding_started"]
            assert callable(callback)
            callback(SimpleNamespace(pid=12345), "test-process-token")
            raise FileNotFoundError("provider lost the bound process")

        receipt = PythonExecutionProvider(
            executor=effect_then_file_not_found,
        ).execute(request)

        self.assertEqual(receipt.status, "outcome-unknown")
        self.assertTrue(receipt.reconciliation_required)
        self.assertTrue(receipt.effect_started)

    def test_owner_without_process_identity_is_not_claimed_live(self) -> None:
        request = self._request(request_id="shell:identity-unavailable")
        journal = ExecutionJournal(self.runtime / "execution-receipts.json")
        self.assertIsNone(journal.claim(request))
        payload = json.loads((self.runtime / "execution-receipts.json").read_text())
        payload["records"][request.request_id]["receipt"].update(
            {"owner_pid": os.getpid(), "owner_identity": ""}
        )
        (self.runtime / "execution-receipts.json").write_text(json.dumps(payload))

        receipt = journal.claim(request)

        self.assertIsNotNone(receipt)
        self.assertEqual(receipt.status, "outcome-unknown")
        self.assertTrue(receipt.reconciliation_required)

    def test_dead_intent_owner_is_reconciled_to_unknown_on_restart(self) -> None:
        request = self._request(request_id="shell:restart-cut")
        journal = ExecutionJournal(self.runtime / "execution-receipts.json")
        self.assertIsNone(journal.claim(request))
        payload = json.loads((self.runtime / "execution-receipts.json").read_text())
        payload["records"][request.request_id]["receipt"].update(
            {"owner_pid": 999999, "owner_identity": ""}
        )
        (self.runtime / "execution-receipts.json").write_text(json.dumps(payload))

        receipts = journal.reconcile()

        self.assertEqual(receipts[0].status, "outcome-unknown")
        self.assertTrue(receipts[0].reconciliation_required)
        self.assertEqual(journal.inspect()[0].status, "outcome-unknown")

    def test_provider_maps_timeout_and_output_limit_to_typed_receipts(self) -> None:
        for returncode, status in ((124, "timed-out"), (125, "output-limit")):
            with self.subTest(status=status):
                request = self._request(request_id=f"shell:{status}")
                observed: dict[str, object] = {}

                def executor(
                    argv: tuple[str, ...], **kwargs: object
                ) -> subprocess.CompletedProcess[str]:
                    observed.update(kwargs)
                    return subprocess.CompletedProcess(
                        argv,
                        returncode,
                        "bounded",
                        "bounded process result",
                    )

                provider = PythonExecutionProvider(
                    executor=executor,
                )
                receipt = provider.execute(request)
                self.assertEqual(receipt.status, status)
                self.assertEqual(receipt.exit_code, returncode)
                self.assertEqual(
                    observed["address_space_bytes"],
                    request.limits.address_space_bytes,
                )
                self.assertEqual(
                    observed["descendant_grace_seconds"],
                    request.limits.descendant_grace_seconds,
                )

    def test_provider_maps_start_failure_without_claiming_an_external_effect(
        self,
    ) -> None:
        request = self._request(request_id="shell:start-failure")
        receipt = PythonExecutionProvider(
            executor=lambda *_args, **_kwargs: (_ for _ in ()).throw(
                FileNotFoundError("missing executable")
            )
        ).execute(request)

        self.assertEqual(receipt.status, "start-failed")
        self.assertEqual(receipt.exit_code, 127)
        self.assertFalse(receipt.effect_started)
        self.assertFalse(receipt.reconciliation_required)

    def test_execution_receipt_round_trips_without_promoting_raw_output(self) -> None:
        request = self._request()
        receipt = ExecutionReceipt.completed(
            request,
            exit_code=0,
            stdout="secret terminal output",
            stderr="",
        )
        persisted = receipt.to_dict(include_output=False)
        restored = ExecutionReceipt.from_dict(persisted)

        self.assertEqual(restored.request_id, receipt.request_id)
        self.assertEqual(restored.status, "completed")
        self.assertEqual(restored.stdout, "")
        self.assertEqual(restored.stdout_sha256, receipt.stdout_sha256)
        self.assertNotIn("secret terminal output", str(persisted))

    def test_input_digest_supports_restart_replay_without_persisting_prompt_bytes(
        self,
    ) -> None:
        request = self._request(request_id="shell:input-replay").with_updates(
            input_text="private prompt bytes"
        )
        journal = ExecutionJournal(self.runtime / "execution-receipts.json")
        calls = 0

        def executor(
            argv: tuple[str, ...], **kwargs: object
        ) -> subprocess.CompletedProcess[str]:
            nonlocal calls
            calls += 1
            self.assertEqual(kwargs["input_text"], "private prompt bytes")
            return subprocess.CompletedProcess(argv, 0, "model output", "")

        first = ExecutionCoordinator(
            journal,
            PythonExecutionProvider(executor=executor),
        ).execute(request)
        second = ExecutionCoordinator(
            ExecutionJournal(self.runtime / "execution-receipts.json"),
            PythonExecutionProvider(
                executor=lambda *_args, **_kwargs: self.fail(
                    "replay re-executed input effect"
                )
            ),
        ).execute(request)

        self.assertEqual(first.status, "completed")
        self.assertEqual(second.status, "completed")
        self.assertEqual(calls, 1)
        journal_text = (self.runtime / "execution-receipts.json").read_text()
        self.assertNotIn("private prompt bytes", journal_text)
        self.assertIn(request.input_sha256 or "", journal_text)


if __name__ == "__main__":
    unittest.main()
