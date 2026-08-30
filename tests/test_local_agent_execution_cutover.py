from __future__ import annotations

import hashlib
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

from albert_mvp.core import AlbertMission, LocalAgentSession
from albert_mvp.execution import (
    ExecutionCoordinator,
    ExecutionJournal,
    ExecutionLimits,
    ExecutionReceipt,
    ExecutionRequest,
    ExecutionSandbox,
    LocalAgentExecutionAuthority,
    PythonExecutionProvider,
)
from albert_mvp.execution_cutover import RustExecutionProvider
from albert_mvp.execution_shadow import RustProviderTransport
from albert_mvp.local_agent_execution_cutover import (
    local_agent_execution_provider_from_environment,
)


class LocalAgentExecutionCutoverTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        self.root = Path(self.temporary.name)
        self.worktree = self.root / "worktree"
        self.worktree.mkdir()
        self.tracker = self.root / "tracker"
        (self.tracker / "issues").mkdir(parents=True)
        (self.tracker / "PRD.md").write_text(
            "# Local Agent cutover fixture\n", encoding="utf-8"
        )
        self.runtime = self.root / "runtime"

    def tearDown(self) -> None:
        self.temporary.cleanup()

    def _request(self, request_id: str) -> ExecutionRequest:
        worktree = str(self.worktree.resolve())
        system_mounts = tuple(
            argument
            for root in ("/usr", "/bin", "/sbin", "/lib", "/lib64", "/etc")
            if Path(root).exists()
            for argument in ("--ro-bind", root, root)
        )
        return ExecutionRequest(
            request_id=request_id,
            effect="local-agent",
            argv=(
                "/usr/bin/bwrap",
                "--die-with-parent",
                "--new-session",
                "--unshare-user",
                "--unshare-pid",
                "--tmpfs",
                "/",
                "--dev",
                "/dev",
                "--proc",
                "/proc",
                "--tmpfs",
                "/tmp",
                *system_mounts,
                "--bind",
                worktree,
                worktree,
                "--chdir",
                worktree,
                "--",
                "/usr/bin/prlimit",
                "--as=8589934592",
                "--fsize=2147483648",
                "--nofile=1024",
                "--nproc=256",
                "--",
                "python3",
                "-c",
                "pass",
            ),
            working_directory=worktree,
            authority=LocalAgentExecutionAuthority(
                mission_id="local-agent-cutover",
                session_id="session-local-agent-cutover",
                session_revision=1,
                runner_operation_id="runner:local-agent-cutover:1",
                worktree_identity="managed:local-agent-cutover:1",
                allowed_paths=("src",),
            ),
            limits=ExecutionLimits(timeout_seconds=2, output_limit_bytes=1024),
            sandbox=ExecutionSandbox(
                mode="bubblewrap",
                writable_roots=(worktree,),
            ),
            environment=(("PATH", "/usr/bin:/bin"),),
        )

    def test_python_feature_flag_keeps_the_packaged_python_provider_operable(
        self,
    ) -> None:
        provider = local_agent_execution_provider_from_environment(
            {"ALFREDO_RUST_LOCAL_AGENT_ENABLED": "0"}
        )

        self.assertIsInstance(provider, PythonExecutionProvider)
        self.assertEqual(provider.provider_id, "python")

    def test_rust_feature_flag_selects_the_integrity_bound_rust_provider(self) -> None:
        provider_path = self.root / "alfredo-execution-provider"
        provider_path.write_text("#!/bin/sh\nexit 0\n", encoding="utf-8")
        provider_path.chmod(0o755)
        provider_sha256 = hashlib.sha256(provider_path.read_bytes()).hexdigest()

        provider = local_agent_execution_provider_from_environment(
            {
                "ALFREDO_RUST_LOCAL_AGENT_ENABLED": "1",
                "ALFREDO_RUST_CANDIDATE_ENABLED": "1",
                "ALFREDO_RUST_EXECUTION_PROVIDER": str(provider_path),
                "ALFREDO_RUST_EXECUTION_PROVIDER_SHA256": provider_sha256,
            }
        )

        self.assertIsInstance(provider, RustExecutionProvider)
        self.assertEqual(provider.provider_id, "rust")

    def test_rust_preflight_failure_rolls_back_before_any_effect_or_journal_claim(
        self,
    ) -> None:
        provider = local_agent_execution_provider_from_environment(
            {
                "ALFREDO_RUST_LOCAL_AGENT_ENABLED": "1",
                "ALFREDO_RUST_CANDIDATE_ENABLED": "1",
                "ALFREDO_RUST_EXECUTION_PROVIDER": str(
                    self.root / "missing-execution-provider"
                ),
                "ALFREDO_RUST_EXECUTION_PROVIDER_SHA256": "0" * 64,
            }
        )

        self.assertIsInstance(provider, PythonExecutionProvider)
        self.assertEqual(provider.provider_id, "python")
        self.assertFalse((self.runtime / "execution-receipts.json").exists())

    def test_unqualified_global_candidate_cannot_enable_local_agent_rust(self) -> None:
        provider_path = self.root / "unqualified-execution-provider"
        provider_path.write_text("#!/bin/sh\nexit 0\n", encoding="utf-8")
        provider_path.chmod(0o755)

        provider = local_agent_execution_provider_from_environment(
            {
                "ALFREDO_RUST_LOCAL_AGENT_ENABLED": "1",
                "ALFREDO_RUST_CANDIDATE_ENABLED": "0",
                "ALFREDO_RUST_EXECUTION_PROVIDER": str(provider_path),
                "ALFREDO_RUST_EXECUTION_PROVIDER_SHA256": hashlib.sha256(
                    provider_path.read_bytes()
                ).hexdigest(),
            }
        )

        self.assertIsInstance(provider, PythonExecutionProvider)

    def test_local_agent_session_routes_the_authorized_effect_through_selected_rust(
        self,
    ) -> None:
        mission = AlbertMission(
            target_repo=self.worktree,
            tracker_dir=self.tracker,
            runtime_root=self.runtime,
            mission_id="local-agent-cutover",
            allow_empty_tracker=True,
        ).load()
        session = LocalAgentSession(
            session_id="session-local-agent-cutover",
            issue_id="ISS-73",
            assigned_agent="local-test",
            worktree_path=self.worktree,
            task_packet={"allowed_paths": ["src"]},
            status="running",
            runner_operation_id="runner:local-agent-cutover:1",
            worktree_identity="managed:local-agent-cutover:1",
        )
        mission.sessions[session.session_id] = session
        mission._persist()
        rust_requests: list[str] = []

        class SelectedRustProvider:
            provider_id = "rust"

            def validate_request(self, request: ExecutionRequest) -> None:
                request.validate()

            def execute(
                self, request: ExecutionRequest, **_callbacks: object
            ) -> ExecutionReceipt:
                rust_requests.append(request.request_id)
                return ExecutionReceipt._make(
                    request,
                    status="completed",
                    exit_code=0,
                    stdout="rust local-agent output",
                    stderr="",
                    effect_started=True,
                    reconciliation_required=False,
                    provider="rust",
                )

        governed_argv = [
            "/usr/bin/bwrap",
            "--die-with-parent",
            "--new-session",
            "--unshare-user",
            "--unshare-pid",
            "--tmpfs",
            "/",
            "--dev",
            "/dev",
            "--proc",
            "/proc",
            "--tmpfs",
            "/tmp",
            "--bind",
            str(self.worktree.resolve()),
            str(self.worktree.resolve()),
            "--chdir",
            str(self.worktree.resolve()),
            "--",
            "/usr/bin/prlimit",
            "--as=8589934592",
            "--fsize=2147483648",
            "--nofile=1024",
            "--nproc=256",
            "--",
            "python3",
            "-c",
            "pass",
        ]
        with (
            patch(
                "albert_mvp.core.sandboxed_process_argv",
                return_value=(governed_argv, True),
            ),
            patch(
                "albert_mvp.core.local_agent_execution_provider_from_environment",
                return_value=SelectedRustProvider(),
            ),
            patch(
                "albert_mvp.core._run_bounded_process",
                side_effect=AssertionError("Rust-selected Local Agent reached Python"),
            ) as python_run,
        ):
            result = mission._run_cancellable_process(
                session,
                ["python3", "-c", "pass"],
                effect_label="local-agent-cutover",
            )

        self.assertEqual(result.returncode, 0)
        self.assertEqual(result.stdout, "rust local-agent output")
        self.assertEqual(len(rust_requests), 1)
        python_run.assert_not_called()
        [receipt] = ExecutionJournal(
            mission.runtime_dir / "execution-receipts.json"
        ).inspect()
        self.assertEqual(receipt.provider, "rust")
        persisted = mission._refresh_persisted_session(session.session_id)
        self.assertEqual(persisted.execution_receipts[0]["provider"], "rust")
        restarted = AlbertMission(
            target_repo=self.worktree,
            tracker_dir=self.tracker,
            runtime_root=self.runtime,
            mission_id="local-agent-cutover",
            allow_empty_tracker=True,
        ).load()
        self.assertEqual(
            restarted.sessions[session.session_id].execution_receipts[0]["provider"],
            "rust",
        )

    def test_rust_crash_preserves_the_effect_process_binding_not_the_adapter_pid(
        self,
    ) -> None:
        provider_path = self.root / "crashing-execution-provider"
        provider_path.write_text(
            """#!/usr/bin/python3
import json
import sys

sys.stdin.readline()
sys.stdout.write(json.dumps({
    "event": "process-started",
    "process_pid": 424242,
    "process_identity": "linux:424242:cutover",
}) + "\\n")
sys.stdout.flush()
raise SystemExit(9)
""",
            encoding="utf-8",
        )
        provider_path.chmod(0o755)
        provider = RustExecutionProvider(
            provider_path,
            hashlib.sha256(provider_path.read_bytes()).hexdigest(),
        )
        request = self._request("local-agent:rust-provider-crash")
        journal = ExecutionJournal(self.root / "crash-execution-receipts.json")

        with patch.object(provider, "validate_request", return_value=None):
            receipt = ExecutionCoordinator(
                journal,
                provider,
            ).execute(request)

        self.assertEqual(receipt.status, "outcome-unknown")
        self.assertTrue(receipt.reconciliation_required)
        self.assertEqual(receipt.provider, "rust")
        self.assertEqual(receipt.process_pid, 424242)
        self.assertEqual(receipt.process_identity, "linux:424242:cutover")

        python_effects = 0

        def forbidden_python(*_args: object, **_kwargs: object):
            nonlocal python_effects
            python_effects += 1
            raise AssertionError("uncertain Rust effect reran through Python")

        replayed = ExecutionCoordinator(
            journal,
            PythonExecutionProvider(executor=forbidden_python),
        ).execute(request)

        self.assertEqual(replayed.receipt_id, receipt.receipt_id)
        self.assertEqual(replayed.status, "outcome-unknown")
        self.assertEqual(replayed.provider, "rust")
        self.assertEqual(python_effects, 0)

    def test_malformed_post_launch_receipt_is_uncertain_and_never_python_retryable(
        self,
    ) -> None:
        provider_path = self.root / "malformed-receipt-provider"
        provider_path.write_text(
            """#!/usr/bin/python3
import json
import sys

sys.stdin.readline()
sys.stdout.write(json.dumps({"event": "receipt", "ok": True}) + "\\n")
sys.stdout.flush()
""",
            encoding="utf-8",
        )
        provider_path.chmod(0o755)
        request = self._request("local-agent:malformed-rust-receipt")
        journal = ExecutionJournal(self.root / "malformed-receipt-journal.json")
        provider = RustExecutionProvider(
            provider_path,
            hashlib.sha256(provider_path.read_bytes()).hexdigest(),
        )

        with patch.object(provider, "validate_request", return_value=None):
            receipt = ExecutionCoordinator(journal, provider).execute(request)

        self.assertEqual(receipt.status, "outcome-unknown")
        self.assertTrue(receipt.reconciliation_required)
        self.assertEqual(receipt.provider, "rust")

        with patch(
            "albert_mvp.execution.PythonExecutionProvider.execute",
            side_effect=AssertionError("uncertain effect automatically retried"),
        ) as python_execute:
            replayed = ExecutionCoordinator(
                journal,
                PythonExecutionProvider(),
            ).execute(request)

        self.assertEqual(replayed.receipt_id, receipt.receipt_id)
        python_execute.assert_not_called()

    def test_unbound_exception_after_claim_is_uncertain_and_never_retried(
        self,
    ) -> None:
        request = self._request("local-agent:unbound-provider-loss")
        journal = ExecutionJournal(self.root / "unbound-provider-loss.json")

        class AcceptedThenLostProvider:
            provider_id = "rust"

            def validate_request(self, candidate: ExecutionRequest) -> None:
                candidate.validate()

            def execute(
                self, _request: ExecutionRequest, **_callbacks: object
            ) -> ExecutionReceipt:
                raise OSError("provider response was lost after request acceptance")

        with self.assertRaisesRegex(OSError, "after request acceptance"):
            ExecutionCoordinator(journal, AcceptedThenLostProvider()).execute(request)

        [uncertain] = journal.inspect()
        self.assertEqual(uncertain.status, "outcome-unknown")
        self.assertEqual(uncertain.provider, "rust")
        self.assertTrue(uncertain.reconciliation_required)

        with patch(
            "albert_mvp.execution.PythonExecutionProvider.execute",
            side_effect=AssertionError("uncertain effect automatically retried"),
        ) as python_execute:
            replayed = ExecutionCoordinator(
                journal,
                PythonExecutionProvider(),
            ).execute(request)

        self.assertEqual(replayed.receipt_id, uncertain.receipt_id)
        python_execute.assert_not_called()

    def test_real_rust_transport_streams_effect_binding_before_terminal_receipt(
        self,
    ) -> None:
        binary = Path(
            "mission-control/src-tauri/target/debug/alfredo-execution-provider"
        ).resolve()
        if not binary.is_file() or not Path("/usr/bin/bwrap").is_file():
            self.skipTest("built Rust provider and Bubblewrap are required")
        provider = RustExecutionProvider(
            binary,
            hashlib.sha256(binary.read_bytes()).hexdigest(),
        )
        bindings: list[int] = []

        receipt = ExecutionCoordinator(
            ExecutionJournal(self.root / "real-rust-execution-receipts.json"),
            provider,
        ).execute(
            self._request("local-agent:real-rust-stream"),
            process_binding_started=lambda process, _token: bindings.append(
                process.pid
            ),
        )

        self.assertEqual(receipt.status, "completed")
        self.assertEqual(receipt.provider, "rust")
        self.assertEqual(bindings, [receipt.process_pid])
        self.assertTrue(receipt.process_identity)

    def test_real_rust_transport_cancels_the_bound_effect_without_python_rerun(
        self,
    ) -> None:
        binary = Path(
            "mission-control/src-tauri/target/debug/alfredo-execution-provider"
        ).resolve()
        if not binary.is_file() or not Path("/usr/bin/bwrap").is_file():
            self.skipTest("built Rust provider and Bubblewrap are required")
        provider = RustExecutionProvider(
            binary,
            hashlib.sha256(binary.read_bytes()).hexdigest(),
        )
        request = self._request("local-agent:real-rust-cancel")
        request = request.with_updates(
            argv=(*request.argv[:-1], "import time; time.sleep(30)"),
        )
        bindings: list[int] = []

        def poll() -> None:
            if bindings:
                raise RuntimeError("Mission Commander cancelled the Local Agent")

        receipt = ExecutionCoordinator(
            ExecutionJournal(self.root / "real-rust-cancel-receipts.json"),
            provider,
        ).execute(
            request,
            process_binding_started=lambda process, _token: bindings.append(
                process.pid
            ),
            poll_callback=poll,
        )

        self.assertEqual(receipt.status, "cancelled")
        self.assertEqual(receipt.provider, "rust")
        self.assertEqual(bindings, [receipt.process_pid])
        self.assertFalse(receipt.reconciliation_required)

    def test_real_rust_transport_cleans_up_when_binding_callback_cancels(
        self,
    ) -> None:
        binary = Path(
            "mission-control/src-tauri/target/debug/alfredo-execution-provider"
        ).resolve()
        if not binary.is_file() or not Path("/usr/bin/bwrap").is_file():
            self.skipTest("built Rust provider and Bubblewrap are required")
        provider = RustExecutionProvider(
            binary,
            hashlib.sha256(binary.read_bytes()).hexdigest(),
        )
        request = self._request("local-agent:rust-bind-cancel")
        request = request.with_updates(
            argv=(*request.argv[:-1], "import time; time.sleep(30)"),
        )
        journal = ExecutionJournal(self.root / "bind-cancel-receipts.json")

        def cancel_during_binding(_process: object, _token: str) -> None:
            raise RuntimeError("cancelled during binding")

        with self.assertRaisesRegex(RuntimeError, "cancelled during binding"):
            ExecutionCoordinator(journal, provider).execute(
                request,
                process_binding_started=cancel_during_binding,
                exception_status=lambda _error: "cancelled",
            )

        [receipt] = journal.inspect()
        self.assertEqual(receipt.status, "cancelled")
        self.assertEqual(receipt.provider, "rust")
        self.assertTrue(receipt.effect_started)
        self.assertFalse(receipt.reconciliation_required)

    def test_binding_callback_does_not_erase_rust_cleanup_uncertainty(self) -> None:
        provider_path = self.root / "uncertain-cleanup-provider"
        provider_path.write_text(
            """#!/usr/bin/python3
import hashlib
import json
import os
import signal
import sys
import time

cancelled = False
def cancel(_signal, _frame):
    global cancelled
    cancelled = True

signal.signal(signal.SIGUSR1, cancel)
envelope = json.loads(sys.stdin.buffer.read().decode("utf-8"))
request = envelope["request"]
digest_request = dict(request)
digest_request.pop("input_text", None)
request_digest = hashlib.sha256(json.dumps(
    digest_request,
    ensure_ascii=True,
    sort_keys=True,
    separators=(",", ":"),
).encode("utf-8")).hexdigest()
identity = f"test:{os.getpid()}:uncertain-cleanup"
sys.stdout.write(json.dumps({
    "event": "process-started",
    "process_pid": os.getpid(),
    "process_identity": identity,
}) + "\\n")
sys.stdout.flush()
deadline = time.monotonic() + 2
while not cancelled and time.monotonic() < deadline:
    time.sleep(0.005)
started_at = "2026-08-30T10:00:00.000Z"
ended_at = "2026-08-30T10:00:00.001Z"
status = "outcome-unknown"
receipt_id = "execution-receipt:" + hashlib.sha256("\\n".join((
    request["request_id"], request_digest, started_at, ended_at, status,
)).encode("utf-8")).hexdigest()
receipt = {
    "schema_version": 1,
    "request_id": request["request_id"],
    "request_digest": request_digest,
    "effect": request["effect"],
    "status": status,
    "started_at": started_at,
    "ended_at": ended_at,
    "exit_code": None,
    "stdout": "",
    "stderr": "",
    "stdout_bytes": 0,
    "stderr_bytes": 0,
    "stdout_sha256": hashlib.sha256(b"").hexdigest(),
    "stderr_sha256": hashlib.sha256(b"").hexdigest(),
    "effect_started": True,
    "reconciliation_required": True,
    "error_code": "outcome-unknown",
    "error_message": "cleanup could not be proven",
    "receipt_id": receipt_id,
    "owner_pid": None,
    "owner_identity": "",
    "process_pid": os.getpid(),
    "process_identity": identity,
    "provider": "rust-shadow",
}
sys.stdout.write(json.dumps({"event": "receipt", "ok": True, "receipt": receipt}) + "\\n")
sys.stdout.flush()
""",
            encoding="utf-8",
        )
        provider_path.chmod(0o755)
        callbacks = 0

        def cancel_during_binding(_pid: int, _identity: str) -> None:
            nonlocal callbacks
            callbacks += 1
            raise RuntimeError("cancelled during binding")

        receipt = RustProviderTransport((str(provider_path),)).execute(
            self._request("local-agent:uncertain-bind-cleanup"),
            effect_process_started=cancel_during_binding,
        )

        self.assertEqual(callbacks, 1)
        self.assertEqual(receipt.status, "outcome-unknown")
        self.assertTrue(receipt.reconciliation_required)

    def test_post_launch_transport_oserror_is_uncertain_not_start_failed(self) -> None:
        provider_path = self.root / "post-launch-oserror-provider"
        provider_path.write_text(
            """#!/usr/bin/python3
import sys
import time

sys.stdin.buffer.read()
time.sleep(0.1)
""",
            encoding="utf-8",
        )
        provider_path.chmod(0o755)

        def request_cancel() -> None:
            raise RuntimeError("cancel provider")

        with patch(
            "albert_mvp.execution_shadow.os.kill",
            side_effect=PermissionError("signal denied after launch"),
        ):
            receipt = RustProviderTransport((str(provider_path),)).execute(
                self._request("local-agent:post-launch-oserror"),
                control_poll_callback=request_cancel,
            )

        self.assertEqual(receipt.status, "outcome-unknown")
        self.assertTrue(receipt.effect_started)
        self.assertTrue(receipt.reconciliation_required)

    def test_immediately_previous_one_response_transport_remains_readable(self) -> None:
        provider_path = self.root / "previous-execution-provider"
        provider_path.write_text(
            """#!/usr/bin/python3
import hashlib
import json
import sys

envelope = json.loads(sys.stdin.buffer.read().decode("utf-8"))
request = envelope["request"]
digest_request = dict(request)
digest_request.pop("input_text", None)
request_digest = hashlib.sha256(json.dumps(
    digest_request,
    ensure_ascii=True,
    sort_keys=True,
    separators=(",", ":"),
).encode("utf-8")).hexdigest()
started_at = "2026-08-30T10:00:00.000Z"
ended_at = "2026-08-30T10:00:00.001Z"
status = "completed"
stdout = "previous transport output"
receipt_id = "execution-receipt:" + hashlib.sha256("\\n".join((
    request["request_id"], request_digest, started_at, ended_at, status,
)).encode("utf-8")).hexdigest()
receipt = {
    "schema_version": 1,
    "request_id": request["request_id"],
    "request_digest": request_digest,
    "effect": request["effect"],
    "status": status,
    "started_at": started_at,
    "ended_at": ended_at,
    "exit_code": 0,
    "stdout": stdout,
    "stderr": "",
    "stdout_bytes": len(stdout.encode("utf-8")),
    "stderr_bytes": 0,
    "stdout_sha256": hashlib.sha256(stdout.encode("utf-8")).hexdigest(),
    "stderr_sha256": hashlib.sha256(b"").hexdigest(),
    "effect_started": True,
    "reconciliation_required": False,
    "error_code": "",
    "error_message": "",
    "receipt_id": receipt_id,
    "owner_pid": None,
    "owner_identity": "",
    "process_pid": None,
    "process_identity": "",
    "provider": "rust-shadow",
}
sys.stdout.write(json.dumps({"ok": True, "receipt": receipt}))
""",
            encoding="utf-8",
        )
        provider_path.chmod(0o755)
        provider = RustExecutionProvider(
            provider_path,
            hashlib.sha256(provider_path.read_bytes()).hexdigest(),
        )

        receipt = ExecutionCoordinator(
            ExecutionJournal(self.root / "previous-transport-receipts.json"),
            provider,
        ).execute(self._request("local-agent:previous-transport"))

        self.assertEqual(receipt.status, "completed")
        self.assertEqual(receipt.stdout, "previous transport output")
        self.assertEqual(receipt.provider, "rust")

    def test_previous_packaged_provider_binary_remains_compatible(self) -> None:
        provider_path = Path(
            "mission-control/release/out/alfredo-agent-linux-x64-gnu/"
            "bin/alfredo-execution-provider"
        ).resolve()
        if not provider_path.is_file() or not Path("/usr/bin/bwrap").is_file():
            self.skipTest("previous packaged Rust provider and Bubblewrap are required")
        provider = RustExecutionProvider(
            provider_path,
            hashlib.sha256(provider_path.read_bytes()).hexdigest(),
        )

        receipt = ExecutionCoordinator(
            ExecutionJournal(self.root / "previous-binary-receipts.json"),
            provider,
        ).execute(self._request("local-agent:previous-binary"))

        self.assertEqual(receipt.status, "completed")
        self.assertEqual(receipt.provider, "rust")

    def test_previous_providerless_receipt_state_loads_without_conversion(self) -> None:
        request = self._request("local-agent:previous-providerless-state")
        journal = ExecutionJournal(self.root / "previous-state-receipts.json")
        self.assertIsNone(journal.claim(request))
        journal.complete(
            request,
            ExecutionReceipt._make(
                request,
                status="completed",
                exit_code=0,
                stdout="",
                stderr="",
                effect_started=True,
                reconciliation_required=False,
                provider="python",
            ),
        )
        payload = json.loads(journal.path.read_text(encoding="utf-8"))
        payload["records"][request.request_id]["receipt"].pop("provider")
        journal.path.write_text(
            json.dumps(payload, indent=2, sort_keys=True) + "\n",
            encoding="utf-8",
        )
        before = journal.path.read_bytes()

        [loaded] = ExecutionJournal(journal.path).inspect()

        self.assertEqual(loaded.provider, "python")
        self.assertEqual(journal.path.read_bytes(), before)


if __name__ == "__main__":
    unittest.main()
