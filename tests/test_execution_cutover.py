from __future__ import annotations

from contextlib import redirect_stdout
import hashlib
import io
import json
import os
from pathlib import Path
import subprocess
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

from albert_mvp.cli import main
from albert_mvp.core import AlbertError, AlbertMission
from albert_mvp.execution import (
    ExecutionCoordinator,
    ExecutionJournal,
    ExecutionLimits,
    ExecutionRequest,
    ExecutionSandbox,
    PythonExecutionProvider,
    ShellExecutionAuthority,
)
from albert_mvp.execution_cutover import (
    RustExecutionProvider,
    shell_execution_provider_from_environment,
)
from albert_mvp.server import serve
from albert_mvp.workspace import ShellTerminalService, WorkspaceSnapshotService


class _ReceiptLosingProvider:
    provider_id = "rust"

    def validate_request(self, request: ExecutionRequest) -> None:
        PythonExecutionProvider().validate_request(request)

    def execute(self, request: ExecutionRequest, **callbacks: object):
        process_binding_started = callbacks["process_binding_started"]
        process_binding_started(SimpleNamespace(pid=os.getpid()), "rust-provider")
        raise OSError("Rust provider receipt was lost after provider start")


class ExecutionCutoverTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        self.root = Path(self.temporary.name)
        self.worktree = self.root / "worktree"
        self.worktree.mkdir()
        self.tracker = self.root / "tracker"
        (self.tracker / "issues").mkdir(parents=True)
        (self.tracker / "PRD.md").write_text(
            "# Shell cutover fixture\n", encoding="utf-8"
        )
        self.runtime = self.root / "runtime"

    def tearDown(self) -> None:
        self.temporary.cleanup()

    def _shell_request(self, request_id: str = "shell:cutover-command-1") -> ExecutionRequest:
        worktree = str(self.worktree.resolve())
        system_mounts = tuple(
            argument
            for root in ("/usr", "/bin", "/sbin", "/lib", "/lib64", "/etc")
            if Path(root).exists()
            for argument in ("--ro-bind", root, root)
        )
        return ExecutionRequest(
            request_id=request_id,
            effect="shell",
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
            authority=ShellExecutionAuthority(
                mission_id="cutover-mission",
                command_id="cutover-command-1",
                correlation_id="cutover-correlation-1",
                command="python3 -c pass",
                classification="auto-allowed",
                requester="mission-commander",
                working_directory=worktree,
                requested_paths=(),
                access_level="read",
                approval_actor="",
            ),
            limits=ExecutionLimits(timeout_seconds=2, output_limit_bytes=1024),
            sandbox=ExecutionSandbox(
                mode="bubblewrap",
                readable_roots=(worktree,),
                writable_roots=(worktree,),
            ),
            environment=(("PATH", "/usr/bin:/bin"),),
        )

    def _rust_provider_fixture(self) -> tuple[Path, Path, str]:
        counter = self.root / "rust-provider-calls.txt"
        provider = self.root / "alfredo-execution-provider"
        provider.write_text(
            """#!/usr/bin/python3
import datetime
import hashlib
import json
from pathlib import Path
import sys

counter = Path(%r)
counter.write_text(counter.read_text() + "rust\\n" if counter.exists() else "rust\\n")
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
stdout = "rust shell output"
stderr = ""
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
    "stderr": stderr,
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
"""
            % str(counter),
            encoding="utf-8",
        )
        provider.chmod(0o755)
        digest = hashlib.sha256(provider.read_bytes()).hexdigest()
        return provider, counter, digest

    def _malformed_streaming_provider_fixture(self) -> tuple[Path, str]:
        provider = self.root / "malformed-alfredo-execution-provider"
        provider.write_text(
            """#!/usr/bin/python3
import json
import os
import subprocess
import sys

json.loads(sys.stdin.buffer.read().decode("utf-8"))
if sys.platform.startswith("linux"):
    remainder = open(f"/proc/{os.getpid()}/stat", encoding="utf-8").read().rsplit(")", 1)[1].split()
    identity = f"linux:{os.getpid()}:{remainder[19]}"
else:
    started_at = subprocess.run(
        ["/bin/ps", "-p", str(os.getpid()), "-o", "lstart="],
        check=True,
        capture_output=True,
        text=True,
    ).stdout.strip()
    identity = f"posix:{os.getpid()}:{started_at}"
event = {
    "event": "process-started",
    "process_pid": os.getpid(),
    "process_identity": identity,
}
sys.stdout.write(json.dumps(event) + "\\n")
sys.stdout.write(json.dumps(event) + "\\n")
sys.stdout.flush()
""",
            encoding="utf-8",
        )
        provider.chmod(0o755)
        return provider, hashlib.sha256(provider.read_bytes()).hexdigest()

    def _snapshots(self) -> WorkspaceSnapshotService:
        mission = AlbertMission(
            target_repo=self.worktree,
            tracker_dir=self.tracker,
            runtime_root=self.runtime,
            mission_id="cutover-mission",
            allow_empty_tracker=True,
        ).load()
        return WorkspaceSnapshotService(mission)

    def test_enabled_shell_cutover_selects_one_integrity_bound_rust_provider(self) -> None:
        provider_path, counter, provider_sha256 = self._rust_provider_fixture()
        python_effects = 0

        def forbidden_python(*_args: object, **_kwargs: object):
            nonlocal python_effects
            python_effects += 1
            raise AssertionError("Rust-selected Shell effect reached Python")

        provider = shell_execution_provider_from_environment(
            {
                "ALFREDO_RUST_CANDIDATE_ENABLED": "1",
                "ALFREDO_RUST_SHELL_ENABLED": "1",
                "ALFREDO_RUST_EXECUTION_PROVIDER": str(provider_path),
                "ALFREDO_RUST_EXECUTION_PROVIDER_SHA256": provider_sha256,
            },
            python_executor=forbidden_python,
        )
        receipt = ExecutionCoordinator(
            ExecutionJournal(self.root / "rust-execution-receipts.json"),
            provider,
        ).execute(self._shell_request("shell:rust-selected"))

        self.assertEqual(receipt.status, "completed")
        self.assertEqual(receipt.provider, "rust")
        self.assertEqual(receipt.stdout, "rust shell output")
        self.assertEqual(counter.read_text(encoding="utf-8").splitlines(), ["rust"])
        self.assertEqual(python_effects, 0)

    def test_shell_terminal_uses_rust_then_replays_without_python_fallback(self) -> None:
        provider_path, counter, provider_sha256 = self._rust_provider_fixture()
        request = {
            "correlation_id": "shell-cutover-public-1",
            "command": "python3 -m unittest --help",
            "working_directory": str(self.worktree),
            "requested_paths": [],
            "requester": "mission-commander",
        }
        enabled = {
            "ALFREDO_RUST_CANDIDATE_ENABLED": "1",
            "ALFREDO_RUST_SHELL_ENABLED": "1",
            "ALFREDO_RUST_EXECUTION_PROVIDER": str(provider_path),
            "ALFREDO_RUST_EXECUTION_PROVIDER_SHA256": provider_sha256,
        }

        snapshots = self._snapshots()
        with (
            patch.dict(os.environ, enabled, clear=False),
            patch(
                "albert_mvp.workspace._run_bounded_process",
                side_effect=AssertionError("Shell cutover invoked Python"),
            ) as python_run,
        ):
            completed = ShellTerminalService(snapshots).submit(**request)

        self.assertEqual(completed.status, "completed")
        self.assertEqual(completed.stdout, "rust shell output")
        python_run.assert_not_called()
        self.assertEqual(counter.read_text(encoding="utf-8").splitlines(), ["rust"])
        [receipt] = ExecutionJournal(
            snapshots._primary_mission.runtime_dir / "execution-receipts.json"
        ).inspect()
        self.assertEqual(receipt.provider, "rust")

        with (
            patch.dict(
                os.environ,
                {"ALFREDO_RUST_CANDIDATE_ENABLED": "0"},
                clear=False,
            ),
            patch(
                "albert_mvp.workspace._run_bounded_process",
                side_effect=AssertionError("replay invoked Python fallback"),
            ) as fallback_run,
        ):
            replayed = ShellTerminalService(self._snapshots()).submit(**request)

        self.assertEqual(replayed.status, "completed")
        self.assertEqual(replayed.stdout, "")
        fallback_run.assert_not_called()
        self.assertEqual(counter.read_text(encoding="utf-8").splitlines(), ["rust"])

        changed = {**request, "command": "python3 -m unittest --version"}
        with (
            patch.dict(os.environ, enabled, clear=False),
            patch(
                "albert_mvp.workspace._run_bounded_process",
                side_effect=AssertionError("changed replay invoked Python"),
            ) as changed_run,
            self.assertRaisesRegex(AlbertError, "correlation id"),
        ):
            ShellTerminalService(self._snapshots()).submit(**changed)

        changed_run.assert_not_called()
        self.assertEqual(counter.read_text(encoding="utf-8").splitlines(), ["rust"])

    def test_explicit_shell_fallback_executes_only_the_python_provider(self) -> None:
        for disabled_flag in (
            "ALFREDO_RUST_CANDIDATE_ENABLED",
            "ALFREDO_RUST_SHELL_ENABLED",
        ):
            with self.subTest(disabled_flag=disabled_flag):
                python_effects = 0

                def python_effect(argv: list[str], **_callbacks: object):
                    nonlocal python_effects
                    python_effects += 1
                    return subprocess.CompletedProcess(argv, 0, "python output", "")

                environment = {
                    "ALFREDO_RUST_CANDIDATE_ENABLED": "1",
                    "ALFREDO_RUST_SHELL_ENABLED": "1",
                    disabled_flag: "0",
                }
                provider = shell_execution_provider_from_environment(
                    environment,
                    python_executor=python_effect,
                )
                request = self._shell_request(
                    f"shell:python-fallback:{disabled_flag}"
                )
                receipt = ExecutionCoordinator(
                    ExecutionJournal(
                        self.root / f"python-fallback-{disabled_flag}.json"
                    ),
                    provider,
                ).execute(request)

                self.assertEqual(receipt.status, "completed")
                self.assertEqual(receipt.provider, "python")
                self.assertEqual(receipt.stdout, "python output")
                self.assertEqual(python_effects, 1)

    def test_shell_flag_cannot_bypass_the_global_candidate_gate(self) -> None:
        provider_path, _counter, provider_sha256 = self._rust_provider_fixture()

        provider = shell_execution_provider_from_environment(
            {
                "ALFREDO_RUST_SHELL_ENABLED": "1",
                "ALFREDO_RUST_EXECUTION_PROVIDER": str(provider_path),
                "ALFREDO_RUST_EXECUTION_PROVIDER_SHA256": provider_sha256,
            }
        )

        self.assertIsInstance(provider, PythonExecutionProvider)

    def test_cli_and_persistent_transport_share_rust_replay_and_conflict_truth(
        self,
    ) -> None:
        provider_path, counter, provider_sha256 = self._rust_provider_fixture()
        enabled = {
            "ALFREDO_RUST_CANDIDATE_ENABLED": "1",
            "ALFREDO_RUST_SHELL_ENABLED": "1",
            "ALFREDO_RUST_EXECUTION_PROVIDER": str(provider_path),
            "ALFREDO_RUST_EXECUTION_PROVIDER_SHA256": provider_sha256,
        }
        common = [
            "--target-repo",
            str(self.worktree),
            "--tracker-dir",
            str(self.tracker),
            "--runtime-root",
            str(self.runtime),
            "--mission-id",
            "cutover-mission",
        ]
        exact = [
            "shell-terminal-submit",
            *common,
            "--correlation-id",
            "shell-cutover-transport-1",
            "--command-text",
            "python3 -m unittest --help",
            "--working-directory",
            str(self.worktree),
            "--requester",
            "mission-commander",
            "--access-level",
            "read",
        ]
        cli_output = io.StringIO()
        with patch.dict(os.environ, enabled, clear=False), redirect_stdout(cli_output):
            cli_exit = main(exact)

        self.assertEqual(cli_exit, 0)
        self.assertEqual(json.loads(cli_output.getvalue())["status"], "completed")
        self.assertEqual(counter.read_text(encoding="utf-8").splitlines(), ["rust"])

        changed = list(exact)
        changed[changed.index("python3 -m unittest --help")] = (
            "python3 -m unittest --version"
        )
        requests = io.StringIO(
            "\n".join(
                json.dumps({"id": request_id, "argv": argv})
                for request_id, argv in (
                    ("persistent-replay", exact),
                    ("persistent-conflict", changed),
                )
            )
            + "\n"
        )
        responses = io.StringIO()
        with patch.dict(os.environ, enabled, clear=False):
            serve(requests, responses)

        replay, conflict = [
            json.loads(line) for line in responses.getvalue().splitlines()
        ]
        self.assertTrue(replay["success"])
        self.assertEqual(json.loads(replay["stdout"])["status"], "completed")
        self.assertFalse(conflict["success"])
        self.assertIn("correlation id", conflict["stderr"])
        self.assertEqual(counter.read_text(encoding="utf-8").splitlines(), ["rust"])

    def test_invalid_candidate_selection_is_a_no_effect_failure_not_fallback(self) -> None:
        python_effects = 0

        def forbidden_python(*_args: object, **_kwargs: object):
            nonlocal python_effects
            python_effects += 1
            raise AssertionError("invalid Rust selection fell back to Python")

        provider = shell_execution_provider_from_environment(
            {
                "ALFREDO_RUST_CANDIDATE_ENABLED": "sometimes",
                "ALFREDO_RUST_SHELL_ENABLED": "1",
            },
            python_executor=forbidden_python,
        )
        receipt = ExecutionCoordinator(
            ExecutionJournal(self.root / "invalid-selection-receipts.json"),
            provider,
        ).execute(self._shell_request("shell:invalid-provider-selection"))

        self.assertEqual(receipt.status, "start-failed")
        self.assertFalse(receipt.effect_started)
        self.assertEqual(receipt.provider, "rust")
        self.assertIn("must be 0 or 1", receipt.error_message)
        self.assertEqual(python_effects, 0)

    def test_selected_rust_integrity_failure_does_not_fall_back(self) -> None:
        provider_path, counter, _provider_sha256 = self._rust_provider_fixture()
        python_effects = 0

        def forbidden_python(*_args: object, **_kwargs: object):
            nonlocal python_effects
            python_effects += 1
            raise AssertionError("integrity failure fell back to Python")

        provider = shell_execution_provider_from_environment(
            {
                "ALFREDO_RUST_CANDIDATE_ENABLED": "1",
                "ALFREDO_RUST_SHELL_ENABLED": "1",
                "ALFREDO_RUST_EXECUTION_PROVIDER": str(provider_path),
                "ALFREDO_RUST_EXECUTION_PROVIDER_SHA256": "0" * 64,
            },
            python_executor=forbidden_python,
        )

        receipt = ExecutionCoordinator(
            ExecutionJournal(self.root / "rust-integrity-failure-receipts.json"),
            provider,
        ).execute(self._shell_request("shell:rust-integrity-failure"))

        self.assertEqual(receipt.status, "start-failed")
        self.assertFalse(receipt.effect_started)
        self.assertEqual(receipt.provider, "rust")
        self.assertIn("integrity", receipt.error_message)
        self.assertFalse(counter.exists())
        self.assertEqual(python_effects, 0)

    def test_protocol_failure_after_effect_binding_is_outcome_unknown(self) -> None:
        provider_path, provider_sha256 = self._malformed_streaming_provider_fixture()
        provider = shell_execution_provider_from_environment(
            {
                "ALFREDO_RUST_CANDIDATE_ENABLED": "1",
                "ALFREDO_RUST_SHELL_ENABLED": "1",
                "ALFREDO_RUST_EXECUTION_PROVIDER": str(provider_path),
                "ALFREDO_RUST_EXECUTION_PROVIDER_SHA256": provider_sha256,
            }
        )
        journal = ExecutionJournal(self.root / "malformed-stream-receipts.json")

        receipt = ExecutionCoordinator(journal, provider).execute(
            self._shell_request("shell:malformed-rust-stream")
        )

        self.assertEqual(receipt.status, "outcome-unknown")
        self.assertTrue(receipt.effect_started)
        self.assertEqual(receipt.provider, "rust")
        self.assertRegex(receipt.process_identity, r"^(?:linux|posix):")
        self.assertTrue(receipt.reconciliation_required)

    def test_real_rust_shell_streams_the_effect_binding_before_completion(self) -> None:
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
            ExecutionJournal(self.root / "real-rust-shell-receipts.json"),
            provider,
        ).execute(
            self._shell_request("shell:real-rust-stream"),
            process_binding_started=lambda process, _token: bindings.append(
                process.pid
            ),
        )

        self.assertEqual(receipt.status, "completed")
        self.assertEqual(receipt.provider, "rust")
        self.assertEqual(receipt.exit_code, 0)
        self.assertEqual(bindings, [receipt.process_pid])
        self.assertTrue(receipt.process_identity)

    def test_real_rust_shell_cancellation_returns_a_canonical_receipt(self) -> None:
        binary = Path(
            "mission-control/src-tauri/target/debug/alfredo-execution-provider"
        ).resolve()
        if not binary.is_file() or not Path("/usr/bin/bwrap").is_file():
            self.skipTest("built Rust provider and Bubblewrap are required")
        provider = RustExecutionProvider(
            binary,
            hashlib.sha256(binary.read_bytes()).hexdigest(),
        )
        request = self._shell_request("shell:real-rust-cancel").with_updates(
            argv=(
                *self._shell_request("shell:real-rust-cancel").argv[:-1],
                "import time; time.sleep(30)",
            ),
        )
        bindings: list[int] = []

        def poll() -> None:
            if bindings:
                raise RuntimeError("Mission Commander cancelled the Shell effect")

        receipt = ExecutionCoordinator(
            ExecutionJournal(self.root / "real-rust-shell-cancel-receipts.json"),
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

    def test_uncertain_effect_retains_the_selected_provider_across_replay(self) -> None:
        request = self._shell_request()
        journal = ExecutionJournal(self.root / "execution-receipts.json")

        with self.assertRaisesRegex(OSError, "receipt was lost"):
            ExecutionCoordinator(journal, _ReceiptLosingProvider()).execute(request)

        [uncertain] = journal.inspect()
        self.assertEqual(uncertain.status, "outcome-unknown")
        self.assertEqual(uncertain.provider, "rust")

        python_effects = 0

        def forbidden_fallback(*_args: object, **_kwargs: object):
            nonlocal python_effects
            python_effects += 1
            raise AssertionError("uncertain Rust effect reached Python fallback")

        replayed = ExecutionCoordinator(
            ExecutionJournal(journal.path),
            PythonExecutionProvider(executor=forbidden_fallback),
        ).execute(request)

        self.assertEqual(replayed.status, "outcome-unknown")
        self.assertEqual(replayed.provider, "rust")
        self.assertEqual(python_effects, 0)


if __name__ == "__main__":
    unittest.main()
