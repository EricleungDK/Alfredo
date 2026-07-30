from __future__ import annotations

import io
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest

from albert_mvp.server import serve


class CodingWorkspaceSelectionCliTests(unittest.TestCase):
    def test_acknowledges_an_exact_existing_repository_without_selecting_a_mission(
        self,
    ) -> None:
        with tempfile.TemporaryDirectory() as root_value:
            root = Path(root_value)
            starting_location = root / "projects"
            coding_workspace = starting_location / "alfredo-target"
            runtime_root = root / "runtime"
            starting_location.mkdir()
            coding_workspace.mkdir()
            subprocess.run(
                ["git", "init", "--quiet", str(coding_workspace)],
                check=True,
                capture_output=True,
                text=True,
            )

            result = subprocess.run(
                [
                    sys.executable,
                    "-m",
                    "albert_mvp",
                    "coding-workspace-select",
                    "--starting-location",
                    str(starting_location),
                    "--workspace-path",
                    str(coding_workspace),
                    "--selection-mode",
                    "existing",
                    "--runtime-root",
                    str(runtime_root),
                    "--correlation-id",
                    "workspace-select-1",
                    "--forbidden-root",
                    str(root / "install"),
                    "--forbidden-root",
                    str(root / "backend"),
                ],
                cwd=Path(__file__).resolve().parents[1],
                capture_output=True,
                text=True,
            )

            self.assertEqual(result.returncode, 0, result.stderr)
            acknowledgement = json.loads(result.stdout)
            self.assertEqual(acknowledgement["schema_version"], 1)
            self.assertEqual(acknowledgement["correlation_id"], "workspace-select-1")
            self.assertEqual(acknowledgement["outcome"], "acknowledged")
            self.assertEqual(
                acknowledgement["starting_location"], str(starting_location.resolve())
            )
            self.assertEqual(
                acknowledgement["coding_workspace"], str(coding_workspace.resolve())
            )
            self.assertEqual(acknowledgement["selection_mode"], "existing")
            self.assertIsNone(acknowledgement["active_mission"])
            self.assertFalse(acknowledgement["replayed"])
            self.assertIn("Coding Workspace acknowledged", acknowledgement["message"])

    def test_rejects_an_invalid_target_without_consuming_the_correlation(self) -> None:
        with tempfile.TemporaryDirectory() as root_value:
            root = Path(root_value)
            starting_location = root / "projects"
            candidate = starting_location / "not-yet-a-repository"
            runtime_root = root / "runtime"
            starting_location.mkdir()
            candidate.mkdir()
            common = [
                sys.executable,
                "-m",
                "albert_mvp",
                "coding-workspace-select",
                "--starting-location",
                str(starting_location),
                "--workspace-path",
                str(candidate),
                "--selection-mode",
                "existing",
                "--runtime-root",
                str(runtime_root),
                "--correlation-id",
                "workspace-select-retry",
            ]

            rejected = subprocess.run(
                common,
                cwd=Path(__file__).resolve().parents[1],
                capture_output=True,
                text=True,
            )

            self.assertNotEqual(rejected.returncode, 0)
            failure = json.loads(rejected.stderr)["error"]
            self.assertEqual(failure["code"], "workspace-invalid")
            self.assertTrue(failure["recoverable"])
            subprocess.run(
                ["git", "init", "--quiet", str(candidate)],
                check=True,
                capture_output=True,
                text=True,
            )

            acknowledged = subprocess.run(
                common,
                cwd=Path(__file__).resolve().parents[1],
                capture_output=True,
                text=True,
            )

            self.assertEqual(acknowledged.returncode, 0, acknowledged.stderr)
            self.assertEqual(json.loads(acknowledged.stdout)["outcome"], "acknowledged")

    def test_creates_and_acknowledges_a_new_repository_below_the_starting_location(
        self,
    ) -> None:
        with tempfile.TemporaryDirectory() as root_value:
            root = Path(root_value)
            starting_location = root / "projects"
            coding_workspace = starting_location / "new-project"
            starting_location.mkdir()

            result = subprocess.run(
                [
                    sys.executable,
                    "-m",
                    "albert_mvp",
                    "coding-workspace-select",
                    "--starting-location",
                    str(starting_location),
                    "--workspace-path",
                    str(coding_workspace),
                    "--selection-mode",
                    "create",
                    "--runtime-root",
                    str(root / "runtime"),
                    "--correlation-id",
                    "workspace-create-1",
                ],
                cwd=Path(__file__).resolve().parents[1],
                capture_output=True,
                text=True,
            )

            self.assertEqual(result.returncode, 0, result.stderr)
            acknowledgement = json.loads(result.stdout)
            self.assertEqual(acknowledgement["selection_mode"], "create")
            self.assertEqual(
                acknowledgement["coding_workspace"], str(coding_workspace.resolve())
            )
            repository_root = subprocess.run(
                ["git", "-C", str(coding_workspace), "rev-parse", "--show-toplevel"],
                check=True,
                capture_output=True,
                text=True,
            ).stdout.strip()
            self.assertEqual(Path(repository_root).resolve(), coding_workspace.resolve())
            self.assertFalse((coding_workspace / ".alfredo-create-pending").exists())

    def test_replays_a_create_request_without_repeating_the_effect(self) -> None:
        with tempfile.TemporaryDirectory() as root_value:
            root = Path(root_value)
            starting_location = root / "projects"
            coding_workspace = starting_location / "replayed-project"
            runtime_root = root / "runtime"
            starting_location.mkdir()
            common = [
                sys.executable,
                "-m",
                "albert_mvp",
                "coding-workspace-select",
                "--starting-location",
                str(starting_location),
                "--workspace-path",
                str(coding_workspace),
                "--selection-mode",
                "create",
                "--runtime-root",
                str(runtime_root),
                "--correlation-id",
                "workspace-create-replay-1",
            ]

            first = subprocess.run(
                common,
                cwd=Path(__file__).resolve().parents[1],
                capture_output=True,
                text=True,
            )
            replay = subprocess.run(
                common,
                cwd=Path(__file__).resolve().parents[1],
                capture_output=True,
                text=True,
            )

            self.assertEqual(first.returncode, 0, first.stderr)
            self.assertEqual(replay.returncode, 0, replay.stderr)
            self.assertFalse(json.loads(first.stdout)["replayed"])
            self.assertTrue(json.loads(replay.stdout)["replayed"])

            changed_request = common.copy()
            changed_request[changed_request.index("--workspace-path") + 1] = str(
                starting_location / "different-project"
            )
            changed = subprocess.run(
                changed_request,
                cwd=Path(__file__).resolve().parents[1],
                capture_output=True,
                text=True,
            )
            self.assertNotEqual(changed.returncode, 0)
            self.assertEqual(
                json.loads(changed.stderr)["error"]["code"], "correlation-conflict"
            )

    def test_git_validation_ignores_inherited_repository_environment(self) -> None:
        with tempfile.TemporaryDirectory() as root_value:
            root = Path(root_value)
            starting_location = root / "projects"
            coding_workspace = starting_location / "isolated-project"
            poisoned_git_dir = root / "poisoned.git"
            starting_location.mkdir()
            environment = os.environ.copy()
            environment["GIT_DIR"] = str(poisoned_git_dir)
            environment["GIT_WORK_TREE"] = str(root)

            result = subprocess.run(
                [
                    sys.executable,
                    "-m",
                    "albert_mvp",
                    "coding-workspace-select",
                    "--starting-location",
                    str(starting_location),
                    "--workspace-path",
                    str(coding_workspace),
                    "--selection-mode",
                    "create",
                    "--runtime-root",
                    str(root / "runtime"),
                    "--correlation-id",
                    "workspace-create-isolated-1",
                ],
                cwd=Path(__file__).resolve().parents[1],
                capture_output=True,
                text=True,
                env=environment,
            )

            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertTrue((coding_workspace / ".git").is_dir())
            self.assertFalse(poisoned_git_dir.exists())

    def test_concurrent_processes_cannot_reuse_a_correlation_for_different_boundaries(
        self,
    ) -> None:
        with tempfile.TemporaryDirectory() as root_value:
            root = Path(root_value)
            starting_location = root / "projects"
            runtime_root = root / "runtime"
            barrier = root / "barrier"
            candidates = [
                starting_location / "first",
                starting_location / "second",
            ]
            barrier.mkdir()
            for candidate in candidates:
                candidate.mkdir(parents=True)
                subprocess.run(
                    ["git", "init", "--quiet", str(candidate)],
                    check=True,
                    capture_output=True,
                    text=True,
                )
            barrier_script = """
from pathlib import Path
import os
import sys
import time
from albert_mvp.cli import main

barrier = Path(sys.argv[1])
(barrier / str(os.getpid())).touch()
deadline = time.monotonic() + 5
while len(list(barrier.iterdir())) < 2:
    if time.monotonic() >= deadline:
        raise SystemExit("concurrency barrier timed out")
    time.sleep(0.005)
raise SystemExit(main(sys.argv[2:]))
"""

            processes = [
                subprocess.Popen(
                    [
                        sys.executable,
                        "-c",
                        barrier_script,
                        str(barrier),
                        "coding-workspace-select",
                        "--starting-location",
                        str(starting_location),
                        "--workspace-path",
                        str(candidate),
                        "--selection-mode",
                        "existing",
                        "--runtime-root",
                        str(runtime_root),
                        "--correlation-id",
                        "workspace-concurrent-correlation-1",
                    ],
                    cwd=Path(__file__).resolve().parents[1],
                    stdout=subprocess.PIPE,
                    stderr=subprocess.PIPE,
                    text=True,
                )
                for candidate in candidates
            ]
            results = [
                (*process.communicate(timeout=10), process.returncode)
                for process in processes
            ]

            self.assertEqual(sorted(result[2] for result in results), [0, 1])
            acknowledged = next(result for result in results if result[2] == 0)
            rejected = next(result for result in results if result[2] != 0)
            self.assertEqual(json.loads(acknowledged[0])["outcome"], "acknowledged")
            self.assertEqual(
                json.loads(rejected[1])["error"]["code"], "correlation-conflict"
            )

    def test_concurrent_create_requests_do_not_delete_the_winning_repository(
        self,
    ) -> None:
        with tempfile.TemporaryDirectory() as root_value:
            root = Path(root_value)
            starting_location = root / "projects"
            coding_workspace = starting_location / "contended"
            runtime_root = root / "runtime"
            barrier = root / "barrier"
            starting_location.mkdir()
            barrier.mkdir()
            barrier_script = """
from pathlib import Path
import os
import sys
import time
from albert_mvp.cli import main

barrier = Path(sys.argv[1])
(barrier / str(os.getpid())).touch()
deadline = time.monotonic() + 5
while len(list(barrier.iterdir())) < 2:
    if time.monotonic() >= deadline:
        raise SystemExit("concurrency barrier timed out")
    time.sleep(0.005)
raise SystemExit(main(sys.argv[2:]))
"""
            processes = [
                subprocess.Popen(
                    [
                        sys.executable,
                        "-c",
                        barrier_script,
                        str(barrier),
                        "coding-workspace-select",
                        "--starting-location",
                        str(starting_location),
                        "--workspace-path",
                        str(coding_workspace),
                        "--selection-mode",
                        "create",
                        "--runtime-root",
                        str(runtime_root),
                        "--correlation-id",
                        f"workspace-concurrent-create-{index}",
                    ],
                    cwd=Path(__file__).resolve().parents[1],
                    stdout=subprocess.PIPE,
                    stderr=subprocess.PIPE,
                    text=True,
                )
                for index in range(2)
            ]
            results = [
                (*process.communicate(timeout=10), process.returncode)
                for process in processes
            ]

            self.assertEqual(sorted(result[2] for result in results), [0, 1])
            repository_root = subprocess.run(
                ["git", "-C", str(coding_workspace), "rev-parse", "--show-toplevel"],
                check=True,
                capture_output=True,
                text=True,
            ).stdout.strip()
            self.assertEqual(Path(repository_root).resolve(), coding_workspace.resolve())
            self.assertFalse((coding_workspace / ".alfredo-create-pending").exists())

    def test_completed_replay_revalidates_the_current_repository_boundary(self) -> None:
        with tempfile.TemporaryDirectory() as root_value:
            root = Path(root_value)
            starting_location = root / "projects"
            coding_workspace = starting_location / "revalidated"
            runtime_root = root / "runtime"
            starting_location.mkdir()
            common = [
                sys.executable,
                "-m",
                "albert_mvp",
                "coding-workspace-select",
                "--starting-location",
                str(starting_location),
                "--workspace-path",
                str(coding_workspace),
                "--selection-mode",
                "create",
                "--runtime-root",
                str(runtime_root),
                "--correlation-id",
                "workspace-revalidate-replay-1",
            ]
            first = subprocess.run(
                common,
                cwd=Path(__file__).resolve().parents[1],
                capture_output=True,
                text=True,
            )
            self.assertEqual(first.returncode, 0, first.stderr)
            shutil.rmtree(coding_workspace / ".git")

            replay = subprocess.run(
                common,
                cwd=Path(__file__).resolve().parents[1],
                capture_output=True,
                text=True,
            )

            self.assertNotEqual(replay.returncode, 0)
            self.assertEqual(json.loads(replay.stderr)["error"]["code"], "workspace-invalid")

    def test_completed_create_replay_reconciles_an_interrupted_marker_cleanup(
        self,
    ) -> None:
        with tempfile.TemporaryDirectory() as root_value:
            root = Path(root_value)
            starting_location = root / "projects"
            coding_workspace = starting_location / "cleanup-replay"
            runtime_root = root / "runtime"
            starting_location.mkdir()
            common = [
                sys.executable,
                "-m",
                "albert_mvp",
                "coding-workspace-select",
                "--starting-location",
                str(starting_location),
                "--workspace-path",
                str(coding_workspace),
                "--selection-mode",
                "create",
                "--runtime-root",
                str(runtime_root),
                "--correlation-id",
                "workspace-cleanup-replay-1",
            ]
            first = subprocess.run(
                common,
                cwd=Path(__file__).resolve().parents[1],
                capture_output=True,
                text=True,
            )
            self.assertEqual(first.returncode, 0, first.stderr)
            receipt_path = next(
                (runtime_root / "workspace-selection-receipts").glob("*.json")
            )
            receipt = json.loads(receipt_path.read_text(encoding="utf-8"))
            create_token = "a" * 64
            receipt["create_token"] = create_token
            receipt_path.write_text(json.dumps(receipt), encoding="utf-8")
            marker = coding_workspace / ".alfredo-create-pending"
            marker.write_text(create_token, encoding="utf-8")

            replay = subprocess.run(
                common,
                cwd=Path(__file__).resolve().parents[1],
                capture_output=True,
                text=True,
            )

            self.assertEqual(replay.returncode, 0, replay.stderr)
            self.assertTrue(json.loads(replay.stdout)["replayed"])
            self.assertFalse(marker.exists())
            finalized_receipt = json.loads(receipt_path.read_text(encoding="utf-8"))
            self.assertIsNone(finalized_receipt["create_token"])

    def test_persistent_transport_returns_the_same_versioned_acknowledgement(
        self,
    ) -> None:
        with tempfile.TemporaryDirectory() as root_value:
            root = Path(root_value)
            starting_location = root / "projects"
            coding_workspace = starting_location / "persistent"
            starting_location.mkdir()
            coding_workspace.mkdir()
            subprocess.run(
                ["git", "init", "--quiet", str(coding_workspace)],
                check=True,
                capture_output=True,
                text=True,
            )
            argv = [
                "coding-workspace-select",
                "--starting-location",
                str(starting_location),
                "--workspace-path",
                str(coding_workspace),
                "--selection-mode",
                "existing",
                "--runtime-root",
                str(root / "runtime"),
                "--correlation-id",
                "workspace-persistent-1",
            ]
            source = io.StringIO(
                json.dumps({"id": "selection-request", "argv": argv}) + "\n"
            )
            destination = io.StringIO()

            serve(source, destination)

            envelope = json.loads(destination.getvalue())
            self.assertEqual(envelope["id"], "selection-request")
            self.assertTrue(envelope["success"], envelope["stderr"])
            acknowledgement = json.loads(envelope["stdout"])
            self.assertEqual(acknowledgement["schema_version"], 1)
            self.assertEqual(acknowledgement["outcome"], "acknowledged")
            self.assertEqual(
                acknowledgement["coding_workspace"], str(coding_workspace.resolve())
            )

    def test_rejects_a_repository_that_contains_a_forbidden_product_root(self) -> None:
        with tempfile.TemporaryDirectory() as root_value:
            root = Path(root_value)
            starting_location = root / "projects"
            coding_workspace = starting_location / "unsafe"
            backend_root = coding_workspace / "alfredo-backend"
            backend_root.mkdir(parents=True)
            subprocess.run(
                ["git", "init", "--quiet", str(coding_workspace)],
                check=True,
                capture_output=True,
                text=True,
            )

            result = subprocess.run(
                [
                    sys.executable,
                    "-m",
                    "albert_mvp",
                    "coding-workspace-select",
                    "--starting-location",
                    str(starting_location),
                    "--workspace-path",
                    str(coding_workspace),
                    "--selection-mode",
                    "existing",
                    "--runtime-root",
                    str(root / "runtime"),
                    "--correlation-id",
                    "workspace-unsafe-1",
                    "--forbidden-root",
                    str(backend_root),
                ],
                cwd=Path(__file__).resolve().parents[1],
                capture_output=True,
                text=True,
            )

            self.assertNotEqual(result.returncode, 0)
            self.assertEqual(json.loads(result.stderr)["error"]["code"], "workspace-unsafe")


if __name__ == "__main__":
    unittest.main()
