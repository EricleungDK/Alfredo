from __future__ import annotations

import io
import os
import json
from pathlib import Path
import statistics
import subprocess
import sys
import tempfile
import time
import unittest

from albert_mvp.server import serve


class PersistentOrchestratorServerTests(unittest.TestCase):
    def test_serves_correlated_requests_until_input_closes(self) -> None:
        parent = r"C:\tmp" if os.name == "nt" else None
        with tempfile.TemporaryDirectory(dir=parent) as root_value:
            root = Path(root_value)
            target = root / "target"
            tracker = root / "tracker"
            issues = tracker / "issues"
            target.mkdir()
            issues.mkdir(parents=True)
            (tracker / "PRD.md").write_text("# Persistent Mission\n", encoding="utf-8")
            (issues / "01-persistent.md").write_text(
                "# Persistent\n\nStatus: ready-for-agent\nType: AFK\n\n## What to build\n\nStay warm.\n\n## Acceptance criteria\n\n- [ ] Warm.\n\n## Blocked by\n\nNone - can start immediately.\n",
                encoding="utf-8",
            )
            common = ["--target-repo", str(target), "--tracker-dir", str(tracker), "--runtime-root", str(root / "runtime"), "--mission-id", "persistent"]
            requests = io.StringIO(
                json.dumps({"id": "snapshot", "argv": ["workspace-snapshot", *common]}) + "\n" +
                json.dumps({"id": "updates", "argv": ["workspace-updates", *common, "--after-revision", "1"]}) + "\n"
            )
            responses = io.StringIO()

            serve(requests, responses)

            payloads = [json.loads(line) for line in responses.getvalue().splitlines()]
            self.assertEqual([item["id"] for item in payloads], ["snapshot", "updates"])
            self.assertTrue(all(item["success"] for item in payloads))
            self.assertEqual(json.loads(payloads[0]["stdout"])["schema_version"], 1)
            self.assertEqual(json.loads(payloads[1]["stdout"])["after_revision"], 1)

    def test_rejects_a_malformed_request_and_continues_serving(self) -> None:
        source = io.StringIO(
            '{"id":"broken","argv":"not-a-list"}\n'
            '{"id":"also-broken"}\n'
        )
        responses = io.StringIO()

        serve(source, responses)

        payloads = [json.loads(line) for line in responses.getvalue().splitlines()]
        self.assertEqual([item["id"] for item in payloads], ["broken", "also-broken"])
        self.assertTrue(all(not item["success"] for item in payloads))
        self.assertTrue(all(json.loads(item["stderr"])["error"]["code"] == "contract-failure" for item in payloads))

    def test_warm_process_reuses_transport_within_latency_budget(self) -> None:
        parent = r"C:\tmp" if os.name == "nt" else None
        with tempfile.TemporaryDirectory(dir=parent) as root_value:
            root = Path(root_value)
            target = root / "target"
            tracker = root / "tracker"
            issues = tracker / "issues"
            target.mkdir()
            issues.mkdir(parents=True)
            (tracker / "PRD.md").write_text("# Warm Mission\n", encoding="utf-8")
            common = [
                "--target-repo", str(target),
                "--tracker-dir", str(tracker),
                "--runtime-root", str(root / "runtime"),
                "--mission-id", "warm",
            ]
            process = subprocess.Popen(
                [sys.executable, "-m", "albert_mvp.server"],
                cwd=Path(__file__).resolve().parents[1],
                stdin=subprocess.PIPE,
                stdout=subprocess.PIPE,
                text=True,
            )
            assert process.stdin is not None
            assert process.stdout is not None
            durations: list[float] = []
            try:
                for index in range(21):
                    started = time.perf_counter()
                    process.stdin.write(json.dumps({
                        "id": f"warm-{index}",
                        "argv": ["workspace-snapshot", *common],
                    }) + "\n")
                    process.stdin.flush()
                    response = json.loads(process.stdout.readline())
                    elapsed = time.perf_counter() - started
                    self.assertEqual(response["id"], f"warm-{index}")
                    self.assertTrue(response["success"], response["stderr"])
                    if index:
                        durations.append(elapsed)
            finally:
                process.stdin.close()
                process.wait(timeout=5)
                process.stdout.close()

            p95 = statistics.quantiles(durations, n=20)[18]
            self.assertLess(p95, 0.150, f"warm p95 was {p95:.3f}s")


if __name__ == "__main__":
    unittest.main()
