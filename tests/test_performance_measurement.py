from __future__ import annotations

import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

from albert_mvp.core import AlbertMission
from albert_mvp.workspace import WorkspaceQueueService, WorkspaceSnapshotService


ISSUE = """Status: ready-for-agent
Type: AFK

## Parent

PRD.md

## What to build

Measure the public workspace.

## Acceptance criteria

- [ ] Monotonic evidence is emitted.

## Blocked by

None - can start immediately
"""


class PerformanceMeasurementTests(unittest.TestCase):
    def setUp(self) -> None:
        self.tmp = tempfile.TemporaryDirectory()
        self.root = Path(self.tmp.name)
        self.target = self.root / "target"
        self.tracker = self.root / "tracker"
        self.runtime = self.root / "runtime"
        self.raw = self.root / "raw.jsonl"
        self.target.mkdir()
        (self.tracker / "issues").mkdir(parents=True)
        (self.tracker / "PRD.md").write_text("# Performance Mission\n", encoding="utf-8")
        (self.tracker / "issues" / "01-measure.md").write_text(ISSUE, encoding="utf-8")

    def tearDown(self) -> None:
        self.tmp.cleanup()

    def measurement_environment(self, *, workflow: str) -> dict[str, str]:
        return {
            "ALFREDO_MEASUREMENT_JSONL": str(self.raw),
            "ALFREDO_MEASUREMENT_RUN_ID": "python-run-001",
            "ALFREDO_MEASUREMENT_SAMPLE_ID": "python-sample-001",
            "ALFREDO_MEASUREMENT_COHORT_ID": f"{workflow}-process-warm",
            "ALFREDO_MEASUREMENT_CORRELATION_ID": f"{workflow}-001",
            "ALFREDO_MEASUREMENT_FIXTURE_ID": "minimal-ready-v1",
            "ALFREDO_MEASUREMENT_FIXTURE_SHA256": (
                "c4cef5ccc043bb6476e6e07195f979ea722e20abf3890c10d06de8ad1628839b"
            ),
            "ALFREDO_MEASUREMENT_SOURCE_SHA256": (
                "a57b5956d8222b2ba001365fbff13e74350051ff13d5ba9dbe7fbeede203e721"
            ),
            "ALFREDO_MEASUREMENT_ARTIFACT_SHA256": (
                "6e68b39f69f605a9218e74dd48f091460ccf15768b922d5b7dfda9ec1e84c2ac"
            ),
            "ALFREDO_MEASUREMENT_VARIANT": "python",
            "ALFREDO_MEASUREMENT_WORKFLOW": workflow,
            "ALFREDO_MEASUREMENT_MODE": "process-warm",
        }

    def load_service(self) -> WorkspaceSnapshotService:
        mission = AlbertMission(
            target_repo=self.target,
            tracker_dir=self.tracker,
            runtime_root=self.runtime,
            mission_id="performance",
        ).load()
        return WorkspaceSnapshotService(mission)

    def raw_records(self) -> list[dict[str, object]]:
        return [
            json.loads(line)
            for line in self.raw.read_text(encoding="utf-8").splitlines()
            if line.strip()
        ]

    def test_partial_environment_fails_closed_instead_of_disabling_measurement(self) -> None:
        from albert_mvp.performance import PerformanceIdentity

        with self.assertRaisesRegex(ValueError, "measurement environment is incomplete"):
            PerformanceIdentity.from_environment(
                {
                    "ALFREDO_MEASUREMENT_JSONL": str(self.raw),
                    "ALFREDO_MEASUREMENT_RUN_ID": "partial-run",
                }
            )

    def test_control_file_identity_is_reloaded_for_each_recorder(self) -> None:
        from albert_mvp.performance import PerformanceRecorder

        control = self.root / "measurement-control.json"
        environment_identity = self.measurement_environment(workflow="queue-defer")
        first = {
            "jsonl_path": environment_identity["ALFREDO_MEASUREMENT_JSONL"],
            **{
                key.removeprefix("ALFREDO_MEASUREMENT_").lower(): value
                for key, value in environment_identity.items()
                if key != "ALFREDO_MEASUREMENT_JSONL"
            },
            "desktop_pid": 4101,
            "desktop_session_id": "desktop-one",
        }
        control.write_text(json.dumps(first), encoding="utf-8")
        environment = {"ALFREDO_MEASUREMENT_CONTROL_PATH": str(control)}

        first_recorder = PerformanceRecorder.from_environment(
            source="python-authority", environment=environment
        )
        self.assertIsNotNone(first_recorder)
        assert first_recorder is not None
        first_recorder.mark("R2", "start")

        second = dict(first)
        second["sample_id"] = "python-sample-002"
        control.write_text(json.dumps(second), encoding="utf-8")
        second_recorder = PerformanceRecorder.from_environment(
            source="python-authority", environment=environment
        )
        self.assertIsNotNone(second_recorder)
        assert second_recorder is not None
        second_recorder.mark("R2", "start")

        records = self.raw_records()
        self.assertEqual([record["sample_id"] for record in records], [
            "python-sample-001",
            "python-sample-002",
        ])
        self.assertTrue(all(record["desktop_pid"] == 4101 for record in records))
        self.assertTrue(
            all(record["desktop_session_id"] == "desktop-one" for record in records)
        )

    def test_control_path_rejects_legacy_identity_and_symlinks(self) -> None:
        from albert_mvp.performance import PerformanceIdentity

        control = self.root / "measurement-control.json"
        control.write_text("{}", encoding="utf-8")
        with self.assertRaisesRegex(ValueError, "must not be combined"):
            PerformanceIdentity.from_environment(
                {
                    "ALFREDO_MEASUREMENT_CONTROL_PATH": str(control),
                    "ALFREDO_MEASUREMENT_RUN_ID": "ambiguous",
                }
            )
        link = self.root / "measurement-control-link.json"
        link.symlink_to(control)
        with self.assertRaisesRegex(ValueError, "regular non-symlink"):
            PerformanceIdentity.from_environment(
                {"ALFREDO_MEASUREMENT_CONTROL_PATH": str(link)}
            )

    def test_workspace_snapshot_records_s6_without_changing_projection(self) -> None:
        service = self.load_service()

        with patch.dict(
            os.environ,
            self.measurement_environment(workflow="startup"),
            clear=False,
        ):
            snapshot = service.snapshot()

        self.assertEqual(snapshot.schema_version, 1)
        self.assertEqual(snapshot.active_mission.id, "performance")
        self.assertEqual(
            [
                (record["stage"], record["boundary"], record["source"])
                for record in self.raw_records()
            ],
            [("S6", "start", "python-authority"), ("S6", "end", "python-authority")],
        )

    def test_queue_decision_records_r2_after_lock_and_before_return(self) -> None:
        service = self.load_service()
        queue = WorkspaceQueueService(service)
        proposal = queue.request_frontier_confirmation(
            correlation_id="proposal-001",
            expected_revision=service.snapshot().revision,
            issue_id="ISS-01",
            source="frontier-router",
            requested_action="Confirm the bounded performance action",
            affected_boundary="performance",
            consequence="Approval records the governed decision.",
            payload={"measurement": "queue-defer"},
        )

        with patch.dict(
            os.environ,
            self.measurement_environment(workflow="queue-defer"),
            clear=False,
        ):
            acknowledgement = queue.decide(
                correlation_id="decision-001",
                expected_revision=proposal.revision,
                item_id=proposal.item_id,
                decision="defer",
                reason="Keep runner work outside the governance sample.",
            )

        self.assertEqual(acknowledgement.item_status, "deferred")
        records = self.raw_records()
        self.assertEqual(
            [(record["stage"], record["boundary"]) for record in records],
            [("R2", "start"), ("R2", "end")],
        )
        self.assertTrue(all(record["detail"]["outcome"] == "pass" for record in records))


if __name__ == "__main__":
    unittest.main()
